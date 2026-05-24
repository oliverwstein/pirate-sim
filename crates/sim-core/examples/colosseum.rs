//! Combat colosseum — hand-place attackers and targets and tick the
//! world forward hour-by-hour, printing what the sub-tick combat
//! resolver does. Phase 4 §3c-1 dropped the previous anchor hack:
//! each scenario now runs to a *terminal outcome* (Sunk, Escaped, or
//! the MAX_HOURS time cap) thanks to the engagement-lock added in
//! §3c-1. Once any broadside lands, both ships flip into
//! `engaged_with = Some(...)` and the BT's Engaged branch keeps the
//! Attacker pursuing and the Defender fleeing until the engagement
//! clears via sink, escape (range > ESCAPE_THRESHOLD_NM AND defender
//! outruns attacker for K_ESCAPE_HOURS) or — in later §3c sub-commits
//! — surrender / boarding.
//!
//! Usage:
//!   cargo run --release --example colosseum

use sim_core::ai::ShipAI;
use sim_core::combat::{broadside_supply_cost, CANNON_RANGE_NM, ESCAPE_THRESHOLD_NM};
use sim_core::goods::ids::{CANNON_SHOT, GUNPOWDER};
use sim_core::port::Faction;
use sim_core::ship::{Ship, ShipPolicy, ShipState};
use sim_core::shiptype::{ids as shiptype_ids, ShipTypeId};
use sim_core::types::{Position, ShipId};
use sim_core::world::World;
use std::path::Path;

const MAX_HOURS: u32 = 96;

fn fresh_world() -> World {
    World::load(Path::new("data/"))
}

/// Drop a freshly-built combat-ready ship into the world at `pos` with
/// the given policy + ship type. `seasoned_frac` controls
/// `crew_seasoned / crew_alive` so the colosseum can compare reload
/// rates between veteran and pressed crews. Powder + shot are sized
/// for `broadsides` full discharges so we can also watch a magazine
/// run dry.
#[allow(clippy::too_many_arguments)]
fn spawn(
    world: &mut World,
    pos: Position,
    owner_port: usize,
    policy: ShipPolicy,
    ship_type: ShipTypeId,
    seasoned_frac: f32,
    broadsides: u32,
    ai_seed: u64,
) -> ShipId {
    let stats = world.ship_types.get(ship_type).stats.clone();
    let mut ship = Ship::seeded_at_port_typed(
        pos,
        owner_port,
        if matches!(policy, ShipPolicy::Pirate) {
            Faction::Free
        } else {
            Faction::England
        },
        ship_type,
        &stats,
        100.0,
    );
    ship.policy = policy;
    ship.state = ShipState::Sailing;
    ship.nav.docked_at_port = None;
    let total = ship.crew_alive as f32;
    ship.crew_seasoned = (total * seasoned_frac.clamp(0.0, 1.0)).round() as u16;
    let (per_p, per_s) = broadside_supply_cost(stats.cannons);
    ship.cargo.add(GUNPOWDER, per_p * broadsides as f32);
    ship.cargo.add(CANNON_SHOT, per_s * broadsides as f32);
    world.add_ship(ship, ShipAI::with_seed(ai_seed))
}

fn snapshot(world: &World, id: ShipId, label: &str) -> String {
    let s = match world.ships.get(id) {
        Some(s) => s,
        None => return format!("{label}: <reaped>"),
    };
    let stats = &world.ship_types.get(s.ship_type).stats;
    let hull_pct = 100.0 * s.hull_integrity / stats.hull_integrity_max;
    let rig_pct = 100.0 * s.rigging_integrity / stats.rigging_integrity_max;
    let powder = s.cargo.get(GUNPOWDER);
    let shot = s.cargo.get(CANNON_SHOT);
    let (per_p, _) = broadside_supply_cost(stats.cannons);
    let shots_left = if per_p > 0.0 { powder / per_p } else { 0.0 };
    let eng = match s.engaged_with {
        None => "free".to_string(),
        Some(_) => "engaged".to_string(),
    };
    let cooldown = s.disengaged_until_minute.saturating_sub(world.sim_minute);
    format!(
        "{label} hull={hull_pct:5.1}% rig={rig_pct:5.1}% pow={powder:4.1}t shot={shot:4.1}t \
         (~{shots_left:4.1} broadsides) crew={}/{} state={:?} {eng}(cd={cooldown}m)",
        s.crew_seasoned, s.crew_alive, s.state
    )
}

fn range_between(world: &World, a: ShipId, b: ShipId) -> Option<f32> {
    let a = world.ships.get(a)?;
    let b = world.ships.get(b)?;
    let dx = a.position.x - b.position.x;
    let dy = a.position.y - b.position.y;
    Some((dx * dx + dy * dy).sqrt())
}

#[derive(Debug, Clone, Copy)]
enum Outcome {
    AttackerSunk,
    TargetSunk,
    Escaped,
    Surrendered,
    TimeCap,
}

struct Scenario {
    title: &'static str,
    blurb: &'static str,
    setup: fn(&mut World) -> (ShipId, ShipId),
}

fn scen_seasoned_brawl(world: &mut World) -> (ShipId, ShipId) {
    let pirate = spawn(
        world,
        Position::new(0.0, 0.0),
        0,
        ShipPolicy::Pirate,
        shiptype_ids::SLOOP,
        1.0,
        40,
        7,
    );
    let merchant = spawn(
        world,
        Position::new(0.0, 0.1),
        1,
        ShipPolicy::Merchant,
        shiptype_ids::BARK,
        1.0,
        20,
        11,
    );
    (pirate, merchant)
}

fn scen_green_pirate(world: &mut World) -> (ShipId, ShipId) {
    let pirate = spawn(
        world,
        Position::new(0.0, 0.0),
        0,
        ShipPolicy::Pirate,
        shiptype_ids::SLOOP,
        0.0,
        40,
        7,
    );
    let merchant = spawn(
        world,
        Position::new(0.0, 0.1),
        1,
        ShipPolicy::Merchant,
        shiptype_ids::BARK,
        1.0,
        20,
        11,
    );
    (pirate, merchant)
}

fn scen_capital_duel(world: &mut World) -> (ShipId, ShipId) {
    let pirate = spawn(
        world,
        Position::new(0.0, 0.0),
        0,
        ShipPolicy::Pirate,
        shiptype_ids::SHIP,
        1.0,
        40,
        7,
    );
    let merchant = spawn(
        world,
        Position::new(0.0, 0.15),
        1,
        ShipPolicy::Merchant,
        shiptype_ids::BARK,
        1.0,
        20,
        11,
    );
    (pirate, merchant)
}

fn scen_stern_chase(world: &mut World) -> (ShipId, ShipId) {
    let pirate = spawn(
        world,
        Position::new(0.0, 0.0),
        0,
        ShipPolicy::Pirate,
        shiptype_ids::SLOOP,
        1.0,
        40,
        7,
    );
    let merchant = spawn(
        world,
        Position::new(0.0, 0.6),
        1,
        ShipPolicy::Merchant,
        shiptype_ids::FLUYT,
        1.0,
        20,
        11,
    );
    (pirate, merchant)
}

fn scen_fleet_fluyt(world: &mut World) -> (ShipId, ShipId) {
    // Slower merchant: tests whether the sloop can run it down.
    let pirate = spawn(
        world,
        Position::new(0.0, 0.0),
        0,
        ShipPolicy::Pirate,
        shiptype_ids::SLOOP,
        1.0,
        40,
        7,
    );
    let merchant = spawn(
        world,
        Position::new(0.0, 0.4),
        1,
        ShipPolicy::Merchant,
        shiptype_ids::FLUYT,
        1.0,
        20,
        11,
    );
    (pirate, merchant)
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        title: "1. Seasoned pirate sloop vs merchant bark (point-blank start)",
        blurb: "Both ships in cannon range at h0. Engagement-lock fires on the first\n  \
                landed broadside. Expect a sustained duel until somebody sinks.",
        setup: scen_seasoned_brawl,
    },
    Scenario {
        title: "2. Green pirate sloop vs seasoned merchant bark (point-blank start)",
        blurb: "Same geometry as #1 but the pirate's reload is 6 min vs 3 min for the\n  \
                merchant's back-fire — watch the firepower asymmetry favour the merchant.",
        setup: scen_green_pirate,
    },
    Scenario {
        title: "3. Seasoned 24-gun pirate ship vs merchant bark",
        blurb: "Heavier broadside weight — bark crumbles faster per fire.",
        setup: scen_capital_duel,
    },
    Scenario {
        title: "4. Stern chase: seasoned sloop vs fluyt at 0.6 NM",
        blurb: "Out-of-range start; pirate must close before any fires land. Engagement\n  \
                begins on the first landed broadside, then it's a question of speed.",
        setup: scen_stern_chase,
    },
    Scenario {
        title: "5. Slow fluyt at 0.4 NM ahead",
        blurb: "Stern chase but starting closer. Fluyt is slow; expect a sloop catch.",
        setup: scen_fleet_fluyt,
    },
];

fn run_scenario(scen: &Scenario) {
    println!("\n══════════════════════════════════════════════════════════════════");
    println!("{}", scen.title);
    println!("  {}", scen.blurb);
    println!("──────────────────────────────────────────────────────────────────");

    let mut world = fresh_world();
    let (atk, tgt) = (scen.setup)(&mut world);

    println!("setup:");
    println!("  ATK  {}", snapshot(&world, atk, "pirate "));
    println!("  TGT  {}", snapshot(&world, tgt, "target "));
    if let Some(r) = range_between(&world, atk, tgt) {
        println!(
            "  range: {r:.3} NM  (cannon range: {CANNON_RANGE_NM} NM; \
             escape threshold: {ESCAPE_THRESHOLD_NM} NM)"
        );
    }
    println!();

    let stats = world
        .ship_types
        .get(world.ships[atk].ship_type)
        .stats
        .clone();
    let (per_p, _) = broadside_supply_cost(stats.cannons);

    let mut atk_powder_prev = world.ships[atk].cargo.get(GUNPOWDER);
    let mut tgt_powder_prev = world.ships[tgt].cargo.get(GUNPOWDER);
    let mut atk_hull_prev = world.ships[atk].hull_integrity;
    let mut tgt_hull_prev = world.ships[tgt].hull_integrity;
    let mut total_atk_fires = 0u32;
    let mut total_tgt_fires = 0u32;
    let mut hours_in_range = 0u32;

    let tgt_stats = world
        .ship_types
        .get(world.ships[tgt].ship_type)
        .stats
        .clone();
    let (tgt_per_p, _) = broadside_supply_cost(tgt_stats.cannons);

    let mut outcome: Option<(Outcome, u32)> = None;
    let prize_baseline =
        world.prizes_taken + world.prizes_sold + world.prizes_sunk + world.prizes_released;

    for hour in 1..=MAX_HOURS {
        let range_pre = range_between(&world, atk, tgt).unwrap_or(f32::INFINITY);

        world.tick();

        let range_post = range_between(&world, atk, tgt).unwrap_or(f32::INFINITY);

        let atk_alive = world.ships.contains_key(atk);
        let tgt_alive = world.ships.contains_key(tgt);

        let atk_pow_now = world
            .ships
            .get(atk)
            .map(|s| s.cargo.get(GUNPOWDER))
            .unwrap_or(0.0);
        let tgt_pow_now = world
            .ships
            .get(tgt)
            .map(|s| s.cargo.get(GUNPOWDER))
            .unwrap_or(0.0);
        let atk_fires = if per_p > 0.0 {
            ((atk_powder_prev - atk_pow_now) / per_p).max(0.0).round() as u32
        } else {
            0
        };
        let tgt_fires = if tgt_per_p > 0.0 {
            ((tgt_powder_prev - tgt_pow_now) / tgt_per_p)
                .max(0.0)
                .round() as u32
        } else {
            0
        };
        atk_powder_prev = atk_pow_now;
        tgt_powder_prev = tgt_pow_now;
        total_atk_fires += atk_fires;
        total_tgt_fires += tgt_fires;

        let atk_hull_now = world
            .ships
            .get(atk)
            .map(|s| s.hull_integrity)
            .unwrap_or(0.0);
        let tgt_hull_now = world
            .ships
            .get(tgt)
            .map(|s| s.hull_integrity)
            .unwrap_or(0.0);
        let atk_dmg = atk_hull_prev - atk_hull_now;
        let tgt_dmg = tgt_hull_prev - tgt_hull_now;
        atk_hull_prev = atk_hull_now;
        tgt_hull_prev = tgt_hull_now;

        if range_pre <= CANNON_RANGE_NM || range_post <= CANNON_RANGE_NM {
            hours_in_range += 1;
        }

        println!(
            "h{hour:02} range {range_pre:5.3}→{range_post:5.3}NM  \
             atk_fires={atk_fires:2} tgt_fires={tgt_fires:2}  \
             atk-hp-Δ={atk_dmg:5.1}  tgt-hp-Δ={tgt_dmg:5.1}"
        );
        println!("    ATK {}", snapshot(&world, atk, "pirate "));
        println!("    TGT {}", snapshot(&world, tgt, "target "));

        // Terminal detection.
        if !atk_alive {
            outcome = Some((Outcome::AttackerSunk, hour));
            break;
        }
        if !tgt_alive {
            outcome = Some((Outcome::TargetSunk, hour));
            break;
        }
        // Engagement-cleared (escape) is the case where both ships
        // were engaged at some point and now both have
        // engaged_with == None.
        let atk_eng = world.ships.get(atk).and_then(|s| s.engaged_with).is_some();
        let tgt_eng = world.ships.get(tgt).and_then(|s| s.engaged_with).is_some();
        if hour >= 2 && !atk_eng && !tgt_eng && hours_in_range > 0 {
            // Saw combat earlier this run, now mutually disengaged →
            // escape *or* surrender (Phase 4 §3c-2: Strike triggers
            // the prize resolver, which increments the prize ledger).
            // Sniff the ledger delta to disambiguate.
            let prize_now =
                world.prizes_taken + world.prizes_sold + world.prizes_sunk + world.prizes_released;
            let outcome_kind = if prize_now > prize_baseline {
                Outcome::Surrendered
            } else {
                Outcome::Escaped
            };
            outcome = Some((outcome_kind, hour));
            break;
        }
    }

    let (outcome, end_hour) = outcome.unwrap_or((Outcome::TimeCap, MAX_HOURS));

    let verdict = match outcome {
        Outcome::AttackerSunk => "ATTACKER SUNK",
        Outcome::TargetSunk => "TARGET SUNK",
        Outcome::Escaped => "DEFENDER ESCAPED",
        Outcome::Surrendered => "PRIZE SURRENDERED",
        Outcome::TimeCap => "TIME CAP",
    };
    println!();
    println!("──────────────────────── verdict ─────────────────────────────────");
    println!("  outcome: {verdict}  at h{end_hour:02}");
    println!(
        "  fires: attacker {total_atk_fires}, target {total_tgt_fires} \
         (over {hours_in_range}h in cannon range)"
    );
    if let Some(s) = world.ships.get(atk) {
        let st = &world.ship_types.get(s.ship_type).stats;
        println!(
            "  final ATK: hull {:.1}% rig {:.1}% crew {}/{}",
            100.0 * s.hull_integrity / st.hull_integrity_max,
            100.0 * s.rigging_integrity / st.rigging_integrity_max,
            s.crew_seasoned,
            s.crew_alive
        );
    }
    if let Some(s) = world.ships.get(tgt) {
        let st = &world.ship_types.get(s.ship_type).stats;
        println!(
            "  final TGT: hull {:.1}% rig {:.1}% crew {}/{}",
            100.0 * s.hull_integrity / st.hull_integrity_max,
            100.0 * s.rigging_integrity / st.rigging_integrity_max,
            s.crew_seasoned,
            s.crew_alive
        );
    }
}

fn main() {
    println!("Combat Colosseum — Phase 4 §3c-1 symmetric engagement");
    println!("Cannon range: {CANNON_RANGE_NM} NM   Max hours/scenario: {MAX_HOURS}");
    println!(
        "Disengage: either party may tactically break off (out of ordnance, badly outclassed, \
         outnumbered, lost contact at >{ESCAPE_THRESHOLD_NM} NM)."
    );
    for scen in SCENARIOS {
        run_scenario(scen);
    }
}
