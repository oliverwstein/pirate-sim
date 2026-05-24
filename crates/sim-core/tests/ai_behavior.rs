//! Integration tests for the AI behavior tree (dock sequence, sailing, etc.)

use rstest::rstest;
use sim_core::ai::{ShipAI, ShipSnapshot, ShipTickInputs};
use sim_core::command::ShipCommand;
use sim_core::goods::GoodsRegistry;
use sim_core::harbor::HarborMap;
use sim_core::market::PortMarket;
use sim_core::port::{Port, DEFAULT_HARBOR_RADIUS_NM};
use sim_core::ship::{DockAction, Ship, ShipState, ShipStats};
use sim_core::spatial::SpatialHash;
use sim_core::types::{Position, ShipId, WindVector};
use slotmap::SecondaryMap;

/// Helper: create a calm wind (so ship speed is predictable).
fn calm_wind() -> WindVector {
    WindVector { u: 0.0, v: -5.0 } // light southerly
}

/// Apply a Steer command back to the ship — the test-side equivalent of
/// the world's Resolution Phase (Step 5.c). Other command variants are
/// ignored: these tests don't exercise combat yet.
fn apply_commands(ship: &mut Ship, commands: &[(ShipId, ShipCommand)]) {
    for (_id, cmd) in commands {
        match cmd {
            ShipCommand::Steer { heading, speed } => ship.set_steering(*heading, *speed),
            ShipCommand::FireBroadside { .. } => {
                // Step 7: combat resolution is the world's job; these
                // single-ship tests just verify the command was emitted.
            }
            ShipCommand::AttemptBoard { .. } => {
                // Step 8: same — boarding resolution lives in the
                // world. Single-ship tests just confirm the intent.
            }
            ShipCommand::Disengage { .. } => {
                // Phase 4 §3c-1: engagement-flag mutation lives in
                // the world. Single-ship tests just confirm the
                // intent.
            }
            ShipCommand::Strike { .. } => {
                // Phase 4 §3c-2: surrender + prize handling lives in
                // the world. Single-ship tests just confirm the
                // intent.
            }
        }
    }
}

/// A throwaway `ShipId`. The AI stamps it onto every emitted command;
/// `apply_commands` ignores the id (single-ship tests).
fn dummy_id() -> ShipId {
    // `slotmap::KeyData` exposes a `from_ffi(u64)`-style constructor via
    // `KeyData::from_ffi(1)`, but ShipId is constructed via SlotMap. The
    // simplest portable option: insert into an empty SlotMap, take the
    // key, throw the map away.
    use slotmap::SlotMap;
    let mut sm: SlotMap<ShipId, ()> = SlotMap::with_key();
    sm.insert(())
}

/// Helper: tick the AI with no markets / no goods / no pathfind. Most
/// integration tests don't model an economy — the act_resupply /
/// act_sell_all / act_buy_best paths fall through harmlessly on an
/// out-of-range `docked_at_port` index.
fn tick_ai(ai: &mut ShipAI, ship: &mut Ship, stats: &ShipStats, wind: &WindVector, ports: &[Port]) {
    let harbors = HarborMap::empty();
    let mut markets: Vec<PortMarket> = Vec::new();
    let goods = GoodsRegistry::starter();
    let mut commands: Vec<(ShipId, ShipCommand)> = Vec::new();
    let snapshots: SecondaryMap<ShipId, ShipSnapshot> = SecondaryMap::new();
    let spatial = SpatialHash::new();
    let me = dummy_id();
    {
        let mut inputs = ShipTickInputs {
            me,
            ship,
            stats,
            wind,
            ports,
            harbors: &harbors,
            pathfind: None,
            markets: &mut markets,
            goods: &goods,
            commands: &mut commands,
            day_of_year: 0,
            snapshots: &snapshots,
            spatial: &spatial,
        };
        ai.tick(&mut inputs);
    }
    apply_commands(ship, &commands);
}

/// Helper: tick the AI with explicit markets + goods. Used by the
/// economy-aware tests (act_sell_all / act_buy_best / act_resupply
/// happy paths).
fn tick_ai_with_markets(
    ai: &mut ShipAI,
    ship: &mut Ship,
    stats: &ShipStats,
    wind: &WindVector,
    ports: &[Port],
    markets: &mut [PortMarket],
    goods: &GoodsRegistry,
) {
    let harbors = HarborMap::empty();
    let mut commands: Vec<(ShipId, ShipCommand)> = Vec::new();
    let snapshots: SecondaryMap<ShipId, ShipSnapshot> = SecondaryMap::new();
    let spatial = SpatialHash::new();
    let me = dummy_id();
    {
        let mut inputs = ShipTickInputs {
            me,
            ship,
            stats,
            wind,
            ports,
            harbors: &harbors,
            pathfind: None,
            markets,
            goods,
            commands: &mut commands,
            day_of_year: 0,
            snapshots: &snapshots,
            spatial: &spatial,
        };
        ai.tick(&mut inputs);
    }
    apply_commands(ship, &commands);
}

/// Helper: some test ports for the AI to use.
fn test_ports() -> Vec<Port> {
    vec![
        Port {
            name: "PortA".to_string(),
            position: Position { x: 100.0, y: 0.0 },
            faction: sim_core::port::Faction::England,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
        Port {
            name: "PortB".to_string(),
            position: Position { x: -100.0, y: 0.0 },
            faction: sim_core::port::Faction::Spain,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
        Port {
            name: "PortC".to_string(),
            position: Position { x: 0.0, y: 100.0 },
            faction: sim_core::port::Faction::France,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
    ]
}

/// Helper: create a docked ship at origin with given provisions/fouling.
fn docked_ship(provisions: f32, fouling: f32) -> Ship {
    let mut ship = Ship::new(Position { x: 0.0, y: 0.0 }, ShipState::Docked);
    ship.provisions = provisions;
    ship.hull_fouling = fouling;
    ship
}

/// Helper: tick AI repeatedly until predicate matches or max ticks exceeded.
fn tick_until(
    ai: &mut ShipAI,
    ship: &mut Ship,
    stats: &ShipStats,
    wind: &WindVector,
    ports: &[Port],
    max_ticks: usize,
    predicate: impl Fn(&Ship, &ShipAI) -> bool,
) -> usize {
    for t in 0..max_ticks {
        if predicate(ship, ai) {
            return t;
        }
        tick_ai(ai, ship, stats, wind, ports);
        ship.tick_resources(stats);
    }
    max_ticks
}

// ============================================================
// Dock sequence tests
// ============================================================

#[rstest]
#[case::empty_provisions(0.5, 0.0, DockAction::Resupplying)]
#[case::dirty_hull(6.0, 50.0, DockAction::Careening)]
#[case::both_depleted(1.0, 40.0, DockAction::Resupplying)] // resupply first
fn dock_sequence_starts_correct_action(
    #[case] provisions: f32,
    #[case] fouling: f32,
    #[case] expected_first_action: DockAction,
) {
    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let mut ship = docked_ship(provisions, fouling);
    let mut ai = ShipAI::new(); // no destination

    // One tick should start the appropriate action
    tick_ai(&mut ai, &mut ship, &stats, &wind, &test_ports());

    assert_eq!(ship.dock_action, expected_first_action);
}

#[test]
fn dock_sequence_resupply_then_careen() {
    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let mut ship = docked_ship(1.0, 40.0); // needs both
    let mut ai = ShipAI::new();

    // Should resupply first
    tick_ai(&mut ai, &mut ship, &stats, &wind, &test_ports());
    assert_eq!(ship.dock_action, DockAction::Resupplying);

    // Tick until resupply completes (action transitions away from Resupplying)
    let ticks = tick_until(
        &mut ai,
        &mut ship,
        &stats,
        &wind,
        &test_ports(),
        100,
        |s, _| s.dock_action != DockAction::Resupplying,
    );
    assert!(ticks < 100, "resupply should complete within 100 ticks");

    // Should now be careening (provisions were filled, fouling > 0)
    assert_eq!(ship.dock_action, DockAction::Careening);
}

#[test]
fn dock_sequence_careen_completes_to_zero() {
    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let mut ship = docked_ship(6.0, 30.0); // full provisions, dirty hull
    let mut ai = ShipAI::new();

    // Tick until careening completes (action transitions away from Careening)
    // First tick starts careening, subsequent ticks reduce fouling
    tick_ai(&mut ai, &mut ship, &stats, &wind, &test_ports());
    ship.tick_resources(&stats);
    assert_eq!(ship.dock_action, DockAction::Careening);

    let ticks = tick_until(
        &mut ai,
        &mut ship,
        &stats,
        &wind,
        &test_ports(),
        200,
        |s, _| s.dock_action != DockAction::Careening,
    );

    // 30 fouling / ~2.99 net per tick ≈ 10-11 ticks
    assert!(
        ticks < 15,
        "careening 30pts should complete in ~11 ticks, got {}",
        ticks
    );
    // Fouling should be negligible (tick_resources adds tiny amount after careen zeroes it)
    assert!(
        ship.hull_fouling < 0.1,
        "fouling should be near zero: {}",
        ship.hull_fouling
    );
}

#[test]
fn dock_sequence_no_ping_pong() {
    // The old bug: resupply → careen → resupply → careen forever.
    // With BT sequence, once resupply succeeds, we never go back.
    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let mut ship = docked_ship(1.0, 40.0); // needs both
    let mut ai = ShipAI::new();

    // Run for many ticks — should eventually reach Idle, not oscillate
    let mut resupply_phases = 0;
    let mut last_action = DockAction::Idle;

    for _ in 0..500 {
        tick_ai(&mut ai, &mut ship, &stats, &wind, &test_ports());
        ship.tick_resources(&stats);

        if ship.dock_action == DockAction::Resupplying && last_action != DockAction::Resupplying {
            resupply_phases += 1;
        }
        last_action = ship.dock_action;
    }

    // Should only enter resupply phase once (the initial one)
    // The sequence guarantees: resupply → careen → done, no going back
    assert_eq!(
        resupply_phases, 1,
        "should only resupply once, not ping-pong"
    );
}

#[test]
fn dock_sequence_chooses_destination_after_servicing() {
    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let ports = test_ports();
    let mut ship = docked_ship(6.0, 0.0); // fully supplied, clean hull
    let mut ai = ShipAI::new(); // NO destination initially

    // With ports available, AI should: resupply(instant) → careen(instant) → undock(needs dest)
    // Undock fails (no dest), dock_tree fails, so selector falls through to ChooseDestination
    // Next tick: HasDestination → Sail → undock
    for _ in 0..5 {
        tick_ai(&mut ai, &mut ship, &stats, &wind, &ports);
        ship.tick_resources(&stats);
    }

    // Should have picked a destination and started sailing
    assert_eq!(
        ship.state,
        ShipState::Sailing,
        "ship should undock after choosing destination"
    );
    assert!(ai.goal.destination.is_some(), "should have a destination");
}

#[test]
fn dock_sequence_undocks_when_destination_set() {
    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let mut ship = docked_ship(6.0, 0.0); // fully supplied, clean
    let mut ai = ShipAI::new();

    // Set a destination
    ai.set_destination(Position { x: 100.0, y: 0.0 }, &mut ship);

    // Should undock after processing the dock sequence (resupply=instant, careen=instant, undock)
    tick_ai(&mut ai, &mut ship, &stats, &wind, &test_ports());

    assert_eq!(ship.state, ShipState::Sailing);
}

// ============================================================
// Sailing → docking transition
// ============================================================

#[test]
fn ship_docks_on_arrival() {
    let stats = ShipStats::sloop();
    let wind = calm_wind();

    // Ship very close to destination
    let dest = Position { x: 2.0, y: 0.0 };
    let mut ship = Ship::new(Position { x: 1.5, y: 0.0 }, ShipState::Sailing);
    let mut ai = ShipAI::with_destination(dest);

    // Tick until docked (should be nearly immediate — within arrival threshold)
    let ticks = tick_until(
        &mut ai,
        &mut ship,
        &stats,
        &wind,
        &test_ports(),
        50,
        |s, _| s.state == ShipState::Docked,
    );

    assert!(ticks < 50, "ship should dock on arrival");
    assert_eq!(ship.state, ShipState::Docked);
}

// ============================================================
// Resource consumption during dock actions
// ============================================================

#[rstest]
#[case::during_resupply(1.0, 0.0)]
#[case::during_careening(3.0, 30.0)]
fn crew_eats_during_dock_actions(#[case] provisions: f32, #[case] fouling: f32) {
    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let mut ship = docked_ship(provisions, fouling);
    let mut ai = ShipAI::new();

    // Record starting provisions (after first tick which may add resupply)
    tick_ai(&mut ai, &mut ship, &stats, &wind, &test_ports());
    ship.tick_resources(&stats);
    let after_first_tick = ship.provisions;

    // If careening, provisions should be decreasing each tick
    if fouling > 0.0 && provisions >= stats.provision_capacity {
        tick_ai(&mut ai, &mut ship, &stats, &wind, &test_ports());
        ship.tick_resources(&stats);
        // Provisions decrease because crew eats, and we're careening (not resupplying)
        assert!(
            ship.provisions < after_first_tick,
            "crew should consume provisions during careening"
        );
    }
}

#[test]
fn fouling_accumulates_while_resupplying() {
    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let mut ship = docked_ship(0.5, 10.0); // needs resupply
    let mut ai = ShipAI::new();

    let initial_fouling = ship.hull_fouling;

    // Tick once (starts resupplying)
    tick_ai(&mut ai, &mut ship, &stats, &wind, &test_ports());
    ship.tick_resources(&stats);

    // Fouling should increase slightly (tick_resources adds 0.0083/hr)
    assert!(
        ship.hull_fouling > initial_fouling,
        "fouling accumulates even while docked"
    );
}

#[test]
fn full_scenario_depleted_ship_docks_and_services() {
    // Simulates the demo: ship arrives docked with low provisions and high fouling
    let stats = ShipStats::sloop();
    let wind = calm_wind();

    // Simulate arriving at port already docked (as would happen after sailing)
    let mut ship = docked_ship(0.78, 41.0); // realistic arrival state
    let mut ai = ShipAI::new(); // no destination (it was consumed on arrival)

    // Track action sequence
    let mut actions_seen: Vec<DockAction> = vec![];

    for _ in 0..200 {
        tick_ai(&mut ai, &mut ship, &stats, &wind, &test_ports());
        ship.tick_resources(&stats);

        if actions_seen.last() != Some(&ship.dock_action) {
            actions_seen.push(ship.dock_action);
        }
    }

    eprintln!("Actions sequence: {:?}", actions_seen);
    eprintln!(
        "Final: prov={:.3} foul={:.3}",
        ship.provisions, ship.hull_fouling
    );

    // Should see: Resupplying → Careening → Idle
    assert!(
        actions_seen.contains(&DockAction::Resupplying),
        "should resupply"
    );
    assert!(
        actions_seen.contains(&DockAction::Careening),
        "should careen"
    );
    assert_eq!(
        actions_seen[0],
        DockAction::Resupplying,
        "resupply should come first"
    );

    // Find index of each
    let resupply_idx = actions_seen
        .iter()
        .position(|a| *a == DockAction::Resupplying)
        .unwrap();
    let careen_idx = actions_seen
        .iter()
        .position(|a| *a == DockAction::Careening)
        .unwrap();
    assert!(resupply_idx < careen_idx, "resupply before careen");
}

#[test]
fn trace_sailing_to_port_royal() {
    let stats = ShipStats::sloop();

    let barbados = Position {
        x: 772.8,
        y: -264.0,
    };
    let port_royal = Position { x: -260.4, y: 26.4 };

    // Include Port Royal in the port list so diversion works sensibly
    let ports = vec![
        Port {
            name: "Port Royal".to_string(),
            position: port_royal,
            faction: sim_core::port::Faction::England,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
        Port {
            name: "Bridgetown".to_string(),
            position: barbados,
            faction: sim_core::port::Faction::England,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
    ];

    let mut ship = Ship::new(barbados, ShipState::Sailing);
    ship.provisions = 1.0;
    ship.hull_fouling = 40.0;
    let mut ai = ShipAI::with_destination(port_royal);

    // Simulate for 30 days (720 hours/ticks)
    let mut docked_at: Option<usize> = None;

    for t in 0..720 {
        let wind = WindVector { u: -4.0, v: -2.0 }; // trade wind (ENE)
        tick_ai(&mut ai, &mut ship, &stats, &wind, &ports);
        ship.tick_resources(&stats);

        // Physics (only if sailing)
        if ship.state == ShipState::Sailing {
            let new_pos = ship.compute_next_position(&stats, &wind, 1.0);
            ship.position = new_pos;
            ship.speed = ship.effective_speed(&stats, &wind);
        }

        if ship.state == ShipState::Docked && docked_at.is_none() {
            docked_at = Some(t);
            let dist = ship.position.distance(port_royal);
            eprintln!(
                "DOCKED at t={} (day {:.1}) dist={:.1} prov={:.3} foul={:.1}",
                t,
                t as f32 / 24.0,
                dist,
                ship.provisions,
                ship.hull_fouling
            );
        }
    }

    assert!(
        docked_at.is_some(),
        "ship should have docked within 30 days"
    );
}

// ============================================================
// Low provisions diversion
// ============================================================

#[test]
fn low_provisions_diverts_to_nearest_port() {
    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let ports = vec![
        Port {
            name: "NearPort".to_string(),
            position: Position { x: 50.0, y: 0.0 },
            faction: sim_core::port::Faction::England,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
        Port {
            name: "FarPort".to_string(),
            position: Position { x: 500.0, y: 0.0 },
            faction: sim_core::port::Faction::Spain,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
    ];

    // Ship at origin, heading to far port, but very low on provisions (< 4 days)
    let mut ship = Ship::new(Position { x: 0.0, y: 0.0 }, ShipState::Sailing);
    ship.provisions = 0.15; // ~3.3 days at 25 crew — below 4-day threshold
    let mut ai = ShipAI::with_destination(Position { x: 500.0, y: 0.0 });

    // Seed NearPort with provisions so the divert-stockpile filter
    // accepts it; FarPort stays dry so it can't be the chosen divert.
    let goods = GoodsRegistry::starter();
    let mut markets: Vec<PortMarket> = vec![
        PortMarket::with_initial_stockpile(&goods),
        PortMarket::with_initial_stockpile(&goods),
    ];
    markets[1]
        .stockpile
        .remove(sim_core::goods::ids::PROVISIONS, 1_000_000.0);
    tick_ai_with_markets(
        &mut ai,
        &mut ship,
        &stats,
        &wind,
        &ports,
        &mut markets,
        &goods,
    );

    // Should have diverted to nearest port (NearPort at 50,0)
    assert_eq!(
        ai.goal.destination,
        Some(Position { x: 50.0, y: 0.0 }),
        "should divert to nearest port when provisions are low"
    );
}

// ============================================================
// Random destination selection
// ============================================================

#[test]
fn chooses_random_destination_when_idle() {
    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let ports = test_ports();

    // Sailing ship with no destination
    let mut ship = Ship::new(Position { x: 0.0, y: 0.0 }, ShipState::Sailing);
    ship.provisions = 3.0; // plenty of food
    let mut ai = ShipAI::new(); // no destination

    tick_ai(&mut ai, &mut ship, &stats, &wind, &ports);

    // Should have chosen a destination from available ports
    assert!(
        ai.goal.destination.is_some(),
        "should choose a random destination"
    );
}

#[test]
fn continuous_sailing_with_port_visits() {
    // Ship should sail, dock, service, pick new destination, repeat
    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let ports = vec![
        Port {
            name: "Home".to_string(),
            position: Position { x: 0.0, y: 0.0 },
            faction: sim_core::port::Faction::England,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
        Port {
            name: "Dest".to_string(),
            position: Position { x: 30.0, y: 0.0 },
            faction: sim_core::port::Faction::Spain,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
    ];

    let mut ship = Ship::new(Position { x: 0.0, y: 0.0 }, ShipState::Sailing);
    let mut ai = ShipAI::with_seed(42);

    let mut dock_count = 0;

    for _ in 0..500 {
        tick_ai(&mut ai, &mut ship, &stats, &wind, &ports);
        ship.tick_resources(&stats);

        // Simplified physics
        if ship.state == ShipState::Sailing {
            let new_pos = ship.compute_next_position(&stats, &wind, 1.0);
            ship.position = new_pos;
        }

        if ship.state == ShipState::Docked {
            dock_count += 1;
        }
    }

    // Should have docked at least once in 500 ticks
    assert!(
        dock_count > 0,
        "ship should dock at least once during continuous operation"
    );
    // Should have picked destinations and be sailing or docked (not stuck)
    assert!(
        ai.goal.destination.is_some() || ship.state == ShipState::Docked,
        "ship should always have a goal"
    );
}

// ============================================================
// Trade cycle integration tests (with markets wired in)
// ============================================================

#[test]
fn dock_cycle_sells_arriving_cargo_and_buys_outgoing() {
    use sim_core::goods::{ids, GoodsRegistry};
    use sim_core::market::{PortArchetype, PortMarket};

    let goods = GoodsRegistry::starter();
    let stats = ShipStats::sloop();
    let wind = calm_wind();

    // Two ports far enough apart that arbitrage clears the distance
    // cost: a sugar surplus at Home, a sugar deficit at Dest.
    let ports = vec![
        Port {
            name: "Home".to_string(),
            position: Position { x: 0.0, y: 0.0 },
            faction: sim_core::port::Faction::England,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
        Port {
            name: "Dest".to_string(),
            position: Position { x: 30.0, y: 0.0 },
            faction: sim_core::port::Faction::Spain,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
    ];
    let mut markets = vec![
        PortMarket::with_recipe(&goods, PortArchetype::SugarIsland.recipe()),
        PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe()),
    ];
    // Bias: surplus of sugar at Home, drain Dest's sugar to zero.
    markets[0].stockpile.add(ids::SUGAR, 5_000.0);
    let dest_sugar = markets[1].stockpile.get(ids::SUGAR);
    markets[1].stockpile.remove(ids::SUGAR, dest_sugar);

    // Ship starts docked at Home with fresh provisions (skip resupply
    // dwell time) and a small dirty hull so careen passes quickly.
    let mut ship = Ship::new(Position { x: 0.0, y: 0.0 }, ShipState::Docked);
    ship.provisions = stats.provision_capacity;
    ship.hull_fouling = 0.0;
    let mut ai = ShipAI::with_seed(42);
    ship.nav.docked_at_port = Some(0); // simulate arrival at Home

    let silver_before = ship.silver;

    // One tick should: SELL_ALL (no-op, empty cargo) → RESUPPLY (no-op, full)
    // → BUY_BEST (loads sugar, sets destination=Dest) → CAREEN (no-op)
    // → UNDOCK (success, transitions to Sailing).
    tick_ai_with_markets(
        &mut ai,
        &mut ship,
        &stats,
        &wind,
        &ports,
        &mut markets,
        &goods,
    );

    assert_eq!(ship.state, ShipState::Sailing, "should have undocked");
    assert_eq!(ai.goal.dest_port, Some(1), "destination should be Dest");
    assert!(ship.cargo.get(ids::SUGAR) > 0.0, "should have bought sugar");
    assert!(
        ship.silver < silver_before,
        "should have spent silver buying cargo"
    );
}

#[test]
fn ship_with_no_profitable_trade_still_undocks() {
    use sim_core::goods::GoodsRegistry;
    use sim_core::market::{PortArchetype, PortMarket};

    let goods = GoodsRegistry::starter();
    let stats = ShipStats::sloop();
    let wind = calm_wind();

    // Two identical Minor ports — find_best_trade returns None.
    let ports = vec![
        Port {
            name: "A".to_string(),
            position: Position { x: 0.0, y: 0.0 },
            faction: sim_core::port::Faction::England,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
        Port {
            name: "B".to_string(),
            position: Position { x: 30.0, y: 0.0 },
            faction: sim_core::port::Faction::England,
            harbor_radius_nm: DEFAULT_HARBOR_RADIUS_NM,
            shipyard: None,
            category: sim_core::pop::PortCategory::SmallColonial,
        },
    ];
    let mut markets = vec![
        PortMarket::with_recipe(&goods, PortArchetype::Minor.recipe()),
        PortMarket::with_recipe(&goods, PortArchetype::Minor.recipe()),
    ];

    let mut ship = Ship::new(Position { x: 0.0, y: 0.0 }, ShipState::Docked);
    ship.provisions = stats.provision_capacity;
    ship.hull_fouling = 0.0;
    let mut ai = ShipAI::with_seed(42);
    ship.nav.docked_at_port = Some(0);

    // Up to a few ticks: BUY_BEST returns Success without setting a
    // destination, so UNDOCK fails, falls through to ACT_CHOOSE_DESTINATION,
    // then on a subsequent tick UNDOCK succeeds.
    let mut undocked = false;
    for _ in 0..5 {
        tick_ai_with_markets(
            &mut ai,
            &mut ship,
            &stats,
            &wind,
            &ports,
            &mut markets,
            &goods,
        );
        if ship.state == ShipState::Sailing {
            undocked = true;
            break;
        }
    }
    assert!(undocked, "ship should still undock via random fallback");
    assert!(ship.cargo.is_empty(), "no cargo should have been loaded");
}

// ============================================================
// Step 6: Pursue / Flee
// ============================================================

/// Helper: tick the AI with an explicit snapshot + spatial-hash
/// pair. Used by the Step 6 pursue/flee tests where the AI under
/// test needs to see another ship's position.
#[allow(clippy::too_many_arguments)]
fn tick_ai_with_snapshots(
    ai: &mut ShipAI,
    ship: &mut Ship,
    stats: &ShipStats,
    wind: &WindVector,
    ports: &[Port],
    snapshots: &SecondaryMap<ShipId, ShipSnapshot>,
    spatial: &SpatialHash,
    me: ShipId,
) -> Vec<(ShipId, ShipCommand)> {
    let harbors = HarborMap::empty();
    let mut markets: Vec<PortMarket> = Vec::new();
    let goods = GoodsRegistry::starter();
    let mut commands: Vec<(ShipId, ShipCommand)> = Vec::new();
    {
        let mut inputs = ShipTickInputs {
            me,
            ship,
            stats,
            wind,
            ports,
            harbors: &harbors,
            pathfind: None,
            markets: &mut markets,
            goods: &goods,
            commands: &mut commands,
            day_of_year: 0,
            snapshots,
            spatial,
        };
        ai.tick(&mut inputs);
    }
    apply_commands(ship, &commands);
    commands
}

#[test]
fn pirate_sees_and_pursues_merchant_in_range() {
    use sim_core::ai::VISUAL_RANGE_NM;
    use sim_core::ship::ShipPolicy;
    use slotmap::SlotMap;

    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let ports = test_ports();

    // Two real ShipIds from a temp SlotMap so the SecondaryMap
    // keying works correctly.
    let mut sm: SlotMap<ShipId, ()> = SlotMap::with_key();
    let pirate_id = sm.insert(());
    let merchant_id = sm.insert(());

    // Pirate at origin; merchant 8 NM north (within 12 NM visual).
    let mut pirate = Ship::new(Position { x: 0.0, y: 0.0 }, ShipState::Sailing);
    pirate.policy = ShipPolicy::Pirate;
    let merchant_pos = Position { x: 0.0, y: 8.0 };

    // Snapshot has the merchant (with a *larger* cargo capacity so
    // the prey filter fires — we fake it via a brig-sized number).
    let mut snapshots: SecondaryMap<ShipId, ShipSnapshot> = SecondaryMap::new();
    snapshots.insert(
        merchant_id,
        ShipSnapshot {
            position: merchant_pos,
            policy: ShipPolicy::Merchant,
            faction: sim_core::port::Faction::England,
            max_speed: stats.speed_max,
            // Bigger than a sloop's hold to trip the "richer" branch.
            cargo_capacity_tons: stats.cargo_capacity_tons + 50.0,
            velocity: (0.0, 0.0),
            rigging_frac: 1.0,
            hull_frac: 1.0,
            cannons: 0,
        },
    );
    // Pirate also in the snapshot (matching the per-tick map shape).
    snapshots.insert(
        pirate_id,
        ShipSnapshot {
            position: pirate.position,
            policy: ShipPolicy::Pirate,
            faction: sim_core::port::Faction::Free,
            max_speed: stats.speed_max,
            cargo_capacity_tons: stats.cargo_capacity_tons,
            velocity: (0.0, 0.0),
            rigging_frac: 1.0,
            hull_frac: 1.0,
            cannons: 0,
        },
    );
    let mut spatial = SpatialHash::new();
    spatial.insert(pirate_id, pirate.position);
    spatial.insert(merchant_id, merchant_pos);

    let mut ai = ShipAI::with_seed(42);
    let cmds = tick_ai_with_snapshots(
        &mut ai,
        &mut pirate,
        &stats,
        &wind,
        &ports,
        &snapshots,
        &spatial,
        pirate_id,
    );

    assert_eq!(
        ai.goal.pursue_target,
        Some(merchant_id),
        "pirate should lock onto the merchant within {VISUAL_RANGE_NM} NM"
    );
    assert!(
        !cmds.is_empty(),
        "act_pursue should have emitted a Steer command"
    );
    // Heading roughly north (0°) since merchant is due north.
    let steer = cmds
        .iter()
        .find_map(|(_id, c)| match c {
            ShipCommand::Steer { heading, .. } => Some(*heading),
            _ => None,
        })
        .expect("act_pursue should emit a Steer command");
    assert!(
        (steer.abs() < 5.0) || ((steer - 360.0).abs() < 5.0),
        "pirate should steer ~north toward merchant, got {steer}"
    );
}

#[test]
fn merchant_flees_when_pirate_in_range() {
    use sim_core::ship::ShipPolicy;
    use slotmap::SlotMap;

    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let ports = test_ports();

    let mut sm: SlotMap<ShipId, ()> = SlotMap::with_key();
    let merchant_id = sm.insert(());
    let pirate_id = sm.insert(());

    let mut merchant = Ship::new(Position { x: 50.0, y: 50.0 }, ShipState::Sailing);
    let pirate_pos = Position { x: 50.0, y: 58.0 }; // 8 NM north

    let mut snapshots: SecondaryMap<ShipId, ShipSnapshot> = SecondaryMap::new();
    snapshots.insert(
        pirate_id,
        ShipSnapshot {
            position: pirate_pos,
            policy: ShipPolicy::Pirate,
            faction: sim_core::port::Faction::Free,
            max_speed: stats.speed_max,
            cargo_capacity_tons: stats.cargo_capacity_tons,
            velocity: (0.0, 0.0),
            rigging_frac: 1.0,
            hull_frac: 1.0,
            cannons: 0,
        },
    );
    snapshots.insert(
        merchant_id,
        ShipSnapshot {
            position: merchant.position,
            policy: ShipPolicy::Merchant,
            faction: sim_core::port::Faction::England,
            max_speed: stats.speed_max,
            cargo_capacity_tons: stats.cargo_capacity_tons,
            velocity: (0.0, 0.0),
            rigging_frac: 1.0,
            hull_frac: 1.0,
            cannons: 0,
        },
    );
    let mut spatial = SpatialHash::new();
    spatial.insert(merchant_id, merchant.position);
    spatial.insert(pirate_id, pirate_pos);

    let mut ai = ShipAI::with_seed(7);
    let cmds = tick_ai_with_snapshots(
        &mut ai,
        &mut merchant,
        &stats,
        &wind,
        &ports,
        &snapshots,
        &spatial,
        merchant_id,
    );

    assert_eq!(
        ai.goal.flee_from,
        Some(pirate_id),
        "merchant should mark the pirate as a threat"
    );
    assert!(!cmds.is_empty(), "act_flee should emit a Steer command");
}

#[test]
fn pirate_ignores_other_pirate() {
    use sim_core::ship::ShipPolicy;
    use slotmap::SlotMap;

    let stats = ShipStats::sloop();
    let wind = calm_wind();
    let ports = test_ports();

    let mut sm: SlotMap<ShipId, ()> = SlotMap::with_key();
    let p1 = sm.insert(());
    let p2 = sm.insert(());

    let mut me = Ship::new(Position { x: 0.0, y: 0.0 }, ShipState::Sailing);
    me.policy = ShipPolicy::Pirate;
    let other_pos = Position { x: 0.0, y: 5.0 };

    let mut snapshots: SecondaryMap<ShipId, ShipSnapshot> = SecondaryMap::new();
    snapshots.insert(
        p2,
        ShipSnapshot {
            position: other_pos,
            policy: ShipPolicy::Pirate,
            faction: sim_core::port::Faction::Free,
            max_speed: stats.speed_max,
            cargo_capacity_tons: stats.cargo_capacity_tons + 100.0,
            velocity: (0.0, 0.0),
            rigging_frac: 1.0,
            hull_frac: 1.0,
            cannons: 0,
        },
    );
    snapshots.insert(
        p1,
        ShipSnapshot {
            position: me.position,
            policy: ShipPolicy::Pirate,
            faction: sim_core::port::Faction::Free,
            max_speed: stats.speed_max,
            cargo_capacity_tons: stats.cargo_capacity_tons,
            velocity: (0.0, 0.0),
            rigging_frac: 1.0,
            hull_frac: 1.0,
            cannons: 0,
        },
    );
    let mut spatial = SpatialHash::new();
    spatial.insert(p1, me.position);
    spatial.insert(p2, other_pos);

    let mut ai = ShipAI::with_seed(13);
    let _ = tick_ai_with_snapshots(
        &mut ai, &mut me, &stats, &wind, &ports, &snapshots, &spatial, p1,
    );

    assert_eq!(
        ai.goal.pursue_target, None,
        "pirate should not pursue another pirate"
    );
}
