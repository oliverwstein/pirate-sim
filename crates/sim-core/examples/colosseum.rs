//! Combat colosseum — hand-place attackers and targets and tick the
//! world forward hour-by-hour, printing a per-tick log of what the
//! sub-tick combat resolver does to each ship. Used during Phase 4 §3b
//! to eyeball the new reload-driven fire loop: is the seasoned/green
//! reload differential observable in real engagements? Does a magazine
//! actually run dry? Does an engagement end cleanly when a ship sinks?
//!
//! Important: this is a *qualitative* tool, not calibration. Several
//! scenarios cheat by clamping ship positions each tick (the AI's
//! pursue/flee dance would otherwise end most engagements in 1–2 hrs,
//! pre-§3c engagement-lock). Watch the per-hour `fires=` column to see
//! the sub-tick fire loop's actual cadence, which is gated by:
//!   * `combat::reload_minutes(seasoned_ratio)` — 3 min seasoned / 6 min green
//!   * Closest-approach over each 5-min sub-tick step (`CANNON_RANGE_NM = 0.5`)
//!   * Magazine: powder + shot per `broadside_supply_cost(cannons)`
//!
//! Even with positions clamped, the AI still picks a flee/pursue
//! velocity, which the sub-tick uses for linear interpolation —
//! so fires/hour is bounded by how quickly the closest-approach
//! line carries them past 0.5 NM. That's an honest preview of what
//! §3c's engagement-lock will fix.
//!
//! Usage:
//!   cargo run --release --example colosseum

use sim_core::ai::ShipAI;
use sim_core::combat::{broadside_supply_cost, CANNON_RANGE_NM};
use sim_core::goods::ids::{CANNON_SHOT, GUNPOWDER};
use sim_core::port::Faction;
use sim_core::ship::{Ship, ShipPolicy, ShipState};
use sim_core::shiptype::{ids as shiptype_ids, ShipTypeId};
use sim_core::types::{Position, ShipId};
use sim_core::world::World;
use std::path::Path;

const MAX_HOURS: u32 = 36;

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
    // Override seasoning.
    let total = ship.crew_alive as f32;
    ship.crew_seasoned = (total * seasoned_frac.clamp(0.0, 1.0)).round() as u16;
    // Magazine: enough powder + shot for `broadsides` discharges.
    let (per_p, per_s) = broadside_supply_cost(stats.cannons);
    ship.cargo.add(GUNPOWDER, per_p * broadsides as f32);
    ship.cargo.add(CANNON_SHOT, per_s * broadsides as f32);
    world.add_ship(ship, ShipAI::with_seed(ai_seed))
}

/// One-line snapshot of a ship for the per-hour log.
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
    format!(
        "{label} hull={hull_pct:5.1}% rig={rig_pct:5.1}% pow={powder:4.1}t shot={shot:4.1}t (~{shots_left:4.1} broadsides) crew={}/{} state={:?}",
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

struct Scenario {
    title: &'static str,
    blurb: &'static str,
    setup: fn(&mut World) -> (ShipId, ShipId),
    /// If true, every tick the target's velocity is clamped to zero so
    /// the pirate can stay alongside. This bypasses the AI's
    /// flee/pursue dance and lets the sub-tick fire loop be observed
    /// over a sustained engagement. Useful for verifying broadsides-
    /// per-hour rates that would otherwise be cut short by §3c-pending
    /// disengagement.
    anchor_target: bool,
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
    // 24-gun pirate "ship" vs 12-gun bark — heavier exchange, expect
    // the bark to crumble fast under the heavier broadside weight.
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
    // Pirate starts just out of range (0.6 NM > 0.5 NM cannon range)
    // behind a slower fluyt — tests the convergence path before the
    // first broadside lands.
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

const SCENARIOS: &[Scenario] = &[
    Scenario {
        title: "1. Seasoned pirate sloop vs ANCHORED merchant bark, point-blank",
        blurb: "Positions clamped each tick; AI still picks flee/pursue velocities,\n  so closest-approach math limits sub-tick fires (preview of §3c need).\n  Watch the steady ~3 fires/hr by the seasoned crew vs ~2 in scenario #2.",
        setup: scen_seasoned_brawl,
        anchor_target: true,
    },
    Scenario {
        title: "2. Green pirate sloop vs ANCHORED merchant bark, point-blank",
        blurb: "Same as #1 but pirate fully green (6-min reload). Expect ~2 fires/hr\n  vs ~3 for seasoned — the reload differential survives even with AI flee.",
        setup: scen_green_pirate,
        anchor_target: true,
    },
    Scenario {
        title: "3. Seasoned 24-gun pirate ship vs ANCHORED merchant bark",
        blurb: "Heavy broadside weight — bark crumbles faster per fire.",
        setup: scen_capital_duel,
        anchor_target: true,
    },
    Scenario {
        title: "4. Free engagement: seasoned pirate sloop vs free-AI merchant bark",
        blurb: "No anchor — AI flees, pirate chases. Shows how short an engagement\n  really is without §3c disengagement / engagement-lock.",
        setup: scen_seasoned_brawl,
        anchor_target: false,
    },
    Scenario {
        title: "5. Stern chase: seasoned pirate closes from 0.6 NM behind an ANCHORED fluyt",
        blurb: "Out-of-range start; pirate must close before any fires register.",
        setup: scen_stern_chase,
        anchor_target: true,
    },
];

fn run_scenario(scen: &Scenario) {
    println!("\n══════════════════════════════════════════════════════════════════");
    println!("{}", scen.title);
    println!("  {}", scen.blurb);
    println!("──────────────────────────────────────────────────────────────────");

    let mut world = fresh_world();
    let (atk, tgt) = (scen.setup)(&mut world);

    // Initial snapshot before tick 0.
    println!("setup:");
    println!("  ATK  {}", snapshot(&world, atk, "pirate "));
    println!("  TGT  {}", snapshot(&world, tgt, "target "));
    if let Some(r) = range_between(&world, atk, tgt) {
        println!("  range: {r:.3} NM  (cannon range: {CANNON_RANGE_NM} NM)");
    }
    println!();

    // Track per-hour broadside count via powder delta.
    let stats = world
        .ship_types
        .get(world.ships[atk].ship_type)
        .stats
        .clone();
    let (per_p, _) = broadside_supply_cost(stats.cannons);

    let mut atk_powder_prev = world.ships[atk].cargo.get(GUNPOWDER);
    let mut atk_hull_prev = world.ships[atk].hull_integrity;
    let mut tgt_hull_prev = world.ships[tgt].hull_integrity;
    let mut out_of_range_streak = 0u32;
    let mut total_fires = 0u32;
    let mut hours_in_range = 0u32;
    let mut hours_fired = 0u32;

    // Cache both ships' starting positions for the anchor hack — after
    // each tick we forcibly restore them so the AI's pursue/flee dance
    // can't carry either ship out of cannon range. Anchoring BOTH
    // ships gives the colosseum a clean view of the sub-tick fire
    // loop without nav noise.
    let tgt_anchor_pos = world.ships[tgt].position;
    let atk_anchor_pos = world.ships[atk].position;

    for hour in 1..=MAX_HOURS {
        // Capture range at start-of-tick (before AI moves anyone).
        let range_pre = range_between(&world, atk, tgt).unwrap_or(f32::INFINITY);

        world.tick();

        // Anchor: forcibly reset positions + velocity so the AI dance
        // can't carry either ship out of cannon range. This is a
        // *display* hack for the colosseum, not how the real sim
        // behaves — see scenario #4 for the un-anchored case.
        if scen.anchor_target {
            if let Some(t) = world.ships.get_mut(tgt) {
                t.position = tgt_anchor_pos;
                t.speed = 0.0;
            }
            if let Some(a) = world.ships.get_mut(atk) {
                a.position = atk_anchor_pos;
                a.speed = 0.0;
            }
        }

        // Compute deltas before reading new snapshots.
        let (range, fires, atk_dmg, tgt_dmg, atk_alive, tgt_alive) = {
            let r = range_between(&world, atk, tgt).unwrap_or(f32::INFINITY);
            let atk_pow_now = world
                .ships
                .get(atk)
                .map(|s| s.cargo.get(GUNPOWDER))
                .unwrap_or(0.0);
            let fires = if per_p > 0.0 {
                ((atk_powder_prev - atk_pow_now) / per_p).max(0.0).round() as u32
            } else {
                0
            };
            atk_powder_prev = atk_pow_now;
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
            (
                r,
                fires,
                atk_dmg,
                tgt_dmg,
                world.ships.contains_key(atk),
                world.ships.contains_key(tgt),
            )
        };

        total_fires += fires;
        if fires > 0 {
            hours_fired += 1;
        }
        if range <= CANNON_RANGE_NM || range_pre <= CANNON_RANGE_NM {
            hours_in_range += 1;
        }

        let event = if !tgt_alive {
            "TARGET REAPED"
        } else if !atk_alive {
            "ATTACKER REAPED"
        } else if matches!(world.ships.get(tgt).map(|s| s.state), Some(ShipState::Sunk)) {
            "target SUNK"
        } else if matches!(world.ships.get(atk).map(|s| s.state), Some(ShipState::Sunk)) {
            "attacker SUNK"
        } else if range > CANNON_RANGE_NM {
            out_of_range_streak += 1;
            "out of range"
        } else {
            out_of_range_streak = 0;
            ""
        };
        if range <= CANNON_RANGE_NM {
            out_of_range_streak = 0;
        }

        println!(
            "h{hour:02} range pre={range_pre:5.3}NM→post={range:5.3}NM  fires={fires:2}  atk-hp-Δ={atk_dmg:5.1}  tgt-hp-Δ={tgt_dmg:5.1}  {event}"
        );
        println!("    ATK {}", snapshot(&world, atk, "pirate "));
        println!("    TGT {}", snapshot(&world, tgt, "target "));

        // Stop conditions.
        if !atk_alive || !tgt_alive {
            println!("→ engagement ends at h{hour:02}: a ship was reaped");
            print_tally(total_fires, hours_in_range, hours_fired, hour);
            return;
        }
        if matches!(world.ships.get(tgt).map(|s| s.state), Some(ShipState::Sunk))
            || matches!(world.ships.get(atk).map(|s| s.state), Some(ShipState::Sunk))
        {
            println!("→ engagement ends at h{hour:02}: a ship sunk (awaits cleanup)");
            print_tally(total_fires, hours_in_range, hours_fired, hour);
            return;
        }
        if out_of_range_streak >= 6 {
            println!("→ engagement broken at h{hour:02}: 6 consecutive hours out of range");
            print_tally(total_fires, hours_in_range, hours_fired, hour);
            return;
        }
    }
    println!("→ time cap ({MAX_HOURS}h) reached with both ships afloat");
    print_tally(total_fires, hours_in_range, hours_fired, MAX_HOURS);
}

fn print_tally(total_fires: u32, hours_in_range: u32, hours_fired: u32, hours_elapsed: u32) {
    let per_in_range = if hours_in_range > 0 {
        total_fires as f32 / hours_in_range as f32
    } else {
        0.0
    };
    println!(
        "   tally: {total_fires} attacker broadsides over {hours_elapsed}h \
         ({hours_fired}h with any fire, {hours_in_range}h with start- or end-of-tick \
         range ≤ {CANNON_RANGE_NM} NM; avg {per_in_range:.1} fires/in-range-hour)"
    );
}

fn main() {
    println!("Combat Colosseum — Phase 4 §3b sub-tick fire loop");
    println!("Cannon range: {CANNON_RANGE_NM} NM   Max hours/scenario: {MAX_HOURS}");
    for scen in SCENARIOS {
        run_scenario(scen);
    }
}
