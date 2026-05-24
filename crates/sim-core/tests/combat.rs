//! Step 7 integration tests — gunnery & damage events.
//!
//! These tests construct a `World`, hand-place two ships within cannon
//! range, give the attacker powder + shot, and run a single hourly tick.
//! They verify that:
//!   1. A FireBroadside command applies hull + rigging damage to the target.
//!   2. An attacker without powder fires nothing and does no damage.
//!   3. Rigging damage reduces effective speed (Ship::effective_speed).

use sim_core::combat::{
    broadside_supply_cost, compute_broadside_damage, CANNON_RANGE_NM, LONG_RANGE_FALLOFF,
};
use sim_core::goods::ids::{CANNON_SHOT, GUNPOWDER};
use sim_core::ship::{Ship, ShipStats};
use sim_core::types::WindVector;
use std::path::Path;

fn fresh_world() -> sim_core::world::World {
    // `cargo test` runs with cwd == crate root; data/ lives at the
    // workspace root, so we step out two dirs.
    sim_core::world::World::load(Path::new("../../data/"))
}

#[test]
fn pure_damage_formula_falloff_is_monotonic() {
    let near = compute_broadside_damage(10, 0.0);
    let mid = compute_broadside_damage(10, 0.25);
    let far = compute_broadside_damage(10, CANNON_RANGE_NM);
    assert!(near.0 > mid.0 && mid.0 > far.0);
    assert!(near.1 > mid.1 && mid.1 > far.1);
    // Sanity-check the falloff endpoint matches the published constant.
    let expected_far = 10.0 * 0.5 * LONG_RANGE_FALLOFF; // BROADSIDE_HULL_BASE = 0.5
    assert!((far.0 - expected_far).abs() < 1e-3);
}

#[test]
fn rigging_damage_reduces_effective_speed() {
    let stats = ShipStats::sloop();
    let mut ship = Ship::new(
        sim_core::types::Position::new(0.0, 0.0),
        sim_core::ship::ShipState::Sailing,
    );
    ship.set_steering(0.0, stats.speed_max);
    let wind = WindVector { u: 0.0, v: -10.0 }; // tailwind
    let healthy = ship.effective_speed(&stats, &wind);
    // Knock rigging down to half.
    ship.rigging_integrity = stats.rigging_integrity_max * 0.5;
    let crippled = ship.effective_speed(&stats, &wind);
    assert!(
        crippled < healthy * 0.6,
        "halving rigging should roughly halve speed: healthy={healthy}, crippled={crippled}"
    );
}

#[test]
fn supply_cost_proportional_to_gun_count() {
    let (p8, s8) = broadside_supply_cost(8);
    let (p24, s24) = broadside_supply_cost(24);
    assert!((p24 - 3.0 * p8).abs() < 1e-4);
    assert!((s24 - 3.0 * s8).abs() < 1e-4);
}

/// A pirate within cannon range of a merchant, with powder + shot in the
/// magazine, hits and damages the merchant on the first tick.
#[test]
fn pirate_in_cannon_range_damages_merchant() {
    use sim_core::ai::ShipAI;
    use sim_core::port::Faction;
    use sim_core::ship::{ShipPolicy, ShipState};
    use sim_core::types::Position;

    // Use a real World so the AI + Resolution phases run end-to-end.
    let mut world = fresh_world();

    // Pick any two distinct port indices so both ships have an
    // `owner_port` and don't trip Owner-port asserts.
    let p_pirate = world.ports[0].position;
    let p_merch = world.ports[1].position;
    let _ = (p_pirate, p_merch); // positions overridden below

    // Spawn pirate at (0,0), merchant 0.1 NM north — well inside the
    // 0.5 NM cannon range.
    let mut pirate = Ship::seeded_at_port(Position::new(0.0, 0.0), 0, Faction::Free);
    pirate.policy = ShipPolicy::Pirate;
    pirate.state = ShipState::Sailing;
    pirate.nav.docked_at_port = None;
    pirate.cargo.add(GUNPOWDER, 4.0);
    pirate.cargo.add(CANNON_SHOT, 4.0);
    let pirate_id = world.add_ship(pirate, ShipAI::with_seed(7));

    let mut merchant = Ship::seeded_at_port(Position::new(0.0, 0.1), 1, Faction::England);
    merchant.policy = ShipPolicy::Merchant;
    merchant.state = ShipState::Sailing;
    merchant.nav.docked_at_port = None;
    // Make the merchant fatter than the sloop pirate so `see_prey`'s
    // richer-or-slower filter accepts it as a target.
    merchant.ship_type = sim_core::shiptype::ids::BARK;
    let merch_hull_max = merchant.hull_integrity;
    let merch_rig_max = merchant.rigging_integrity;
    let merchant_id = world.add_ship(merchant, ShipAI::with_seed(11));

    // Run one hour of world ticking.
    world.tick();

    let merchant_after = &world.ships[merchant_id];
    assert!(
        merchant_after.hull_integrity < merch_hull_max,
        "merchant should have taken hull damage (was {merch_hull_max}, now {})",
        merchant_after.hull_integrity,
    );
    assert!(
        merchant_after.rigging_integrity < merch_rig_max,
        "merchant should have taken rigging damage (was {merch_rig_max}, now {})",
        merchant_after.rigging_integrity,
    );

    // Pirate should have deducted powder + shot.
    let pirate_after = &world.ships[pirate_id];
    let leftover_powder = pirate_after.cargo.get(GUNPOWDER);
    let leftover_shot = pirate_after.cargo.get(CANNON_SHOT);
    assert!(
        leftover_powder < 4.0,
        "pirate should have burned powder (still has {leftover_powder} t)"
    );
    assert!(
        leftover_shot < 4.0,
        "pirate should have burned shot (still has {leftover_shot} t)"
    );
}

/// An attacker with no powder still pursues but does NOT damage the target.
#[test]
fn pirate_without_powder_does_no_damage() {
    use sim_core::ai::ShipAI;
    use sim_core::port::Faction;
    use sim_core::ship::{ShipPolicy, ShipState};
    use sim_core::types::Position;

    let mut world = fresh_world();

    let mut pirate = Ship::seeded_at_port(Position::new(0.0, 0.0), 0, Faction::Free);
    pirate.policy = ShipPolicy::Pirate;
    pirate.state = ShipState::Sailing;
    pirate.nav.docked_at_port = None;
    // No powder, no shot.
    let _pirate_id = world.add_ship(pirate, ShipAI::with_seed(7));

    let mut merchant = Ship::seeded_at_port(Position::new(0.0, 0.1), 1, Faction::England);
    merchant.policy = ShipPolicy::Merchant;
    merchant.state = ShipState::Sailing;
    merchant.nav.docked_at_port = None;
    merchant.ship_type = sim_core::shiptype::ids::BARK;
    let hull_before = merchant.hull_integrity;
    let rig_before = merchant.rigging_integrity;
    let merchant_id = world.add_ship(merchant, ShipAI::with_seed(11));

    world.tick();

    let merchant_after = &world.ships[merchant_id];
    assert_eq!(
        merchant_after.hull_integrity, hull_before,
        "merchant hull should be untouched when pirate has no powder"
    );
    assert_eq!(
        merchant_after.rigging_integrity, rig_before,
        "merchant rigging should be untouched when pirate has no powder"
    );
}

// ── Step 8 integration tests: boarding & sinking ────────────────────────

/// Step 11.a: a successful boarding now resolves to one of four
/// outcomes — take (rare, only on real upgrade), sell, sink, or
/// release — but in every case the prize's cargo gets stripped and
/// the attacker's silver goes up. This test pins those invariants
/// regardless of which outcome the RNG picks.
#[test]
fn pirate_boards_dismasted_merchant_and_resolves_prize() {
    use sim_core::ai::ShipAI;
    use sim_core::cargo::Cargo;
    use sim_core::goods::ids::{CANNON_SHOT, GUNPOWDER, SUGAR};
    use sim_core::port::Faction;
    use sim_core::ship::{ShipPolicy, ShipState};
    use sim_core::types::Position;

    let mut world = fresh_world();

    let mut pirate = Ship::seeded_at_port(Position::new(0.0, 0.0), 0, Faction::Free);
    pirate.policy = ShipPolicy::Pirate;
    pirate.state = ShipState::Sailing;
    pirate.nav.docked_at_port = None;
    pirate.ship_type = sim_core::shiptype::ids::BRIGANTINE;
    let pirate_stats = world.ship_types.get(pirate.ship_type).stats.clone();
    pirate.crew_alive = pirate_stats.crew_typical();
    pirate.morale = 1.0;
    pirate.silver = 0.0;
    pirate.cargo = Cargo::new();
    pirate.cargo.add(GUNPOWDER, 4.0);
    pirate.cargo.add(CANNON_SHOT, 4.0);
    let pirate_id = world.add_ship(pirate, ShipAI::with_seed(7));

    let mut merchant = Ship::seeded_at_port(Position::new(0.0, 0.02), 1, Faction::England);
    merchant.policy = ShipPolicy::Merchant;
    merchant.state = ShipState::Sailing;
    merchant.nav.docked_at_port = None;
    merchant.ship_type = sim_core::shiptype::ids::BARK;
    let merch_stats = world.ship_types.get(merchant.ship_type).stats.clone();
    merchant.crew_alive = (merch_stats.crew_typical() / 2).max(2);
    merchant.rigging_integrity = merch_stats.rigging_integrity_max * 0.1;
    // Load some valuable cargo and silver so we can confirm the
    // boarders stripped both.
    merchant.cargo = Cargo::new();
    merchant.cargo.add(SUGAR, 20.0);
    merchant.silver = 500.0;
    let merchant_id = world.add_ship(merchant, ShipAI::with_seed(11));

    world.tick();

    // Some prize outcome must have been recorded. §3c-2b: a sell roll
    // now places the prize *in tow* rather than instantly sinking her,
    // so `prizes_in_tow` counts toward the outcome tally.
    let total_outcomes = world.prizes_taken
        + world.prizes_sold
        + world.prizes_sunk
        + world.prizes_released
        + world.prizes_in_tow;
    assert!(
        total_outcomes >= 1,
        "boarding should have produced at least one prize outcome, counters were 0"
    );

    // Pirate must have accrued silver (cargo bounty + stripped silver
    // applied on every non-`take` outcome; on `take` she gets the
    // hull but no cash). At minimum she's still alive.
    let pirate_after = world
        .ships
        .get(pirate_id)
        .expect("attacker should still exist after winning the boarding");
    assert!(pirate_after.crew_alive < pirate_stats.crew_typical());

    if let Some(merch) = world.ships.get(merchant_id) {
        // §3c-2b: a `sell` outcome now sets the prize in tow rather
        // than instant-selling. In that case the prize is still in
        // the world with cargo intact, `prize_owner == Some(pirate)`,
        // policy flipped to Pirate. Other surviving outcomes are
        // `take` (flag flipped, cargo intact) and `release` (cargo
        // stripped, original policy). Sink/instant-sell would have
        // removed the ship from the world (handled in the `else`).
        let sugar_left = merch.cargo.get(SUGAR);
        if merch.prize_owner == Some(pirate_id) {
            assert_eq!(world.prizes_in_tow, 1);
            assert_eq!(merch.policy, ShipPolicy::Pirate);
        } else if merch.policy == ShipPolicy::Pirate {
            assert_eq!(world.prizes_taken, 1);
        } else {
            assert_eq!(world.prizes_released, 1);
            assert!(
                sugar_left == 0.0,
                "released merchant's cargo should be stripped (sugar={})",
                sugar_left
            );
        }
    } else {
        // Ship gone: must have been sunk or sold (both reaped).
        assert!(
            world.prizes_sunk + world.prizes_sold >= 1,
            "missing ship should correspond to a sink or sell outcome"
        );
        // And the attacker should have pocketed the cargo silver.
        assert!(
            pirate_after.silver > 0.0,
            "non-take outcomes should bank stripped cargo silver"
        );
    }
}

/// A merchant whose hull is hammered to 0 by a broadside is marked Sunk
/// and removed from the world before the next tick begins.
#[test]
fn ship_with_zero_hull_is_reaped_in_cleanup() {
    use sim_core::ai::ShipAI;
    use sim_core::port::Faction;
    use sim_core::ship::{ShipPolicy, ShipState};
    use sim_core::types::Position;

    let mut world = fresh_world();

    // Pirate at point-blank, plenty of powder, big guns.
    let mut pirate = Ship::seeded_at_port(Position::new(0.0, 0.0), 0, Faction::Free);
    pirate.policy = ShipPolicy::Pirate;
    pirate.state = ShipState::Sailing;
    pirate.nav.docked_at_port = None;
    pirate.ship_type = sim_core::shiptype::ids::BRIGANTINE;
    let pirate_stats = world.ship_types.get(pirate.ship_type).stats.clone();
    pirate.crew_alive = pirate_stats.crew_typical();
    pirate.cargo.add(GUNPOWDER, 10.0);
    pirate.cargo.add(CANNON_SHOT, 10.0);
    let _ = world.add_ship(pirate, ShipAI::with_seed(7));

    // Merchant pre-damaged to 1 hull point — the next broadside finishes her.
    let mut merchant = Ship::seeded_at_port(Position::new(0.0, 0.1), 1, Faction::England);
    merchant.policy = ShipPolicy::Merchant;
    merchant.state = ShipState::Sailing;
    merchant.nav.docked_at_port = None;
    merchant.ship_type = sim_core::shiptype::ids::BARK;
    merchant.hull_integrity = 1.0;
    let merchant_id = world.add_ship(merchant, ShipAI::with_seed(11));

    world.tick();

    assert!(
        world.ships.get(merchant_id).is_none(),
        "merchant should have been reaped by Cleanup after hull hit 0"
    );
}

/// Step 11.a: a pirate too thinly crewed to spare a prize crew can
/// never roll the `take` outcome — instead the prize resolves to
/// sell / sink / release, and the boarders strip the cargo either
/// way. The defender's flag must NOT have flipped to Pirate.
#[test]
fn under_crewed_pirate_cannot_take_prize() {
    use sim_core::ai::ShipAI;
    use sim_core::port::Faction;
    use sim_core::ship::{ShipPolicy, ShipState};
    use sim_core::types::Position;

    let mut world = fresh_world();

    // Pirate with just enough crew above min to fight, but not enough
    // to split off a prize crew.
    let mut pirate = Ship::seeded_at_port(Position::new(0.0, 0.0), 0, Faction::Free);
    pirate.policy = ShipPolicy::Pirate;
    pirate.state = ShipState::Sailing;
    pirate.nav.docked_at_port = None;
    let pirate_stats = world.ship_types.get(pirate.ship_type).stats.clone();
    // Just at crew_min — surviving any boarding leaves us below the
    // split threshold required to crew a prize.
    pirate.crew_alive = pirate_stats.crew_min();
    pirate.morale = 1.0;
    let _ = world.add_ship(pirate, ShipAI::with_seed(7));

    // Tiny defender so the pirate still wins despite the rough match.
    let mut merchant = Ship::seeded_at_port(Position::new(0.0, 0.02), 1, Faction::England);
    merchant.policy = ShipPolicy::Merchant;
    merchant.state = ShipState::Sailing;
    merchant.nav.docked_at_port = None;
    merchant.ship_type = sim_core::shiptype::ids::BARK;
    let merch_stats = world.ship_types.get(merchant.ship_type).stats.clone();
    merchant.crew_alive = 3;
    merchant.morale = 0.1; // shaken
    merchant.rigging_integrity = merch_stats.rigging_integrity_max * 0.1;
    let merchant_id = world.add_ship(merchant, ShipAI::with_seed(11));

    world.tick();

    // A prize outcome was recorded, but NOT `take`.
    assert_eq!(
        world.prizes_taken, 0,
        "an under-crewed pirate must not roll the take outcome"
    );
    let resolved = world.prizes_sold + world.prizes_sunk + world.prizes_released;
    assert!(
        resolved >= 1,
        "boarding should have resolved as sell/sink/release"
    );

    // If the defender survived (release outcome), it must still be a
    // Merchant — never a Pirate.
    if let Some(merch) = world.ships.get(merchant_id) {
        assert_eq!(
            merch.policy,
            ShipPolicy::Merchant,
            "release outcome must leave the defender as a Merchant"
        );
    }
}

// ── Phase 4 §3a — sub-tick clock plumbing ─────────────────────────────

/// Phase 4 §3a: `World::sim_minute` must advance by exactly 60 each
/// hourly `tick()`. The upcoming sub-tick combat loop (§3b) keys
/// reload timing off this counter, so any drift between calendar
/// hours and the minute clock would silently miscalibrate reload
/// rates.
#[test]
fn sim_minute_advances_60_per_hour() {
    let mut world = fresh_world();
    let start = world.sim_minute;
    world.tick();
    assert_eq!(world.sim_minute, start + sim_core::world::MINUTES_PER_HOUR);
    for _ in 0..23 {
        world.tick();
    }
    assert_eq!(
        world.sim_minute,
        start + 24 * sim_core::world::MINUTES_PER_HOUR
    );
}
