//! Step 7 — gunnery and damage events.
//!
//! This module is deliberately small and deterministic: a single damage
//! formula keyed off cannon count and range. Stochastic crit/miss/raking
//! mechanics are deferred to a later calibration pass (Step 10).
//!
//! ## Range and falloff
//!
//! 17th-century smooth-bore broadsides were most effective at point-blank
//! (~50 yd) and bled rapidly past 500 yd; the long shot at 1000 yd was a
//! hopeful affair. We collapse that into a linear falloff from 1.0× at 0
//! NM to `LONG_RANGE_FALLOFF` (0.3×) at `CANNON_RANGE_NM` (0.5 NM ≈ 1000
//! yd). 0.25 NM was considered for "the real fight" but 0.5 NM gives the
//! pursuer enough hours-of-tick window to score hits before the merchant
//! closes the gap (or breaks contact) at the next position update.
//!
//! ## Damage split
//!
//! Hull and rigging take parallel damage from the same broadside but at
//! different coefficients. Per Step 7's model both kinds of damage matter:
//! hull damage paves the way for sinking (Step 8) and lowers boarding
//! resistance; rigging damage immediately reduces the target's effective
//! speed, which is what lets a slower pursuer catch a faster prey.
//!
//! ## Consumables
//!
//! Each broadside costs `POWDER_TONS_PER_GUN × cannons` tons of gunpowder
//! and `SHOT_TONS_PER_GUN × cannons` tons of cannon shot from the
//! attacker's cargo. A 24-gun ship firing a broadside spends 0.24 t of
//! powder and 0.24 t of shot — meaningful but not enormous. If either
//! good is missing the command is silently dropped in the Resolution
//! Phase (no fire, no damage).

/// Maximum range at which a broadside is resolved at all (nautical
/// miles). Outside this range the AI may still pursue but won't fire.
pub const CANNON_RANGE_NM: f32 = 0.5;

/// Damage multiplier at `CANNON_RANGE_NM` (linear interpolation from
/// 1.0× at point-blank).
pub const LONG_RANGE_FALLOFF: f32 = 0.3;

/// Hull HP removed per gun at point-blank range, before falloff.
pub const BROADSIDE_HULL_BASE: f32 = 0.5;

/// Rigging HP removed per gun at point-blank range, before falloff.
pub const BROADSIDE_RIGGING_BASE: f32 = 0.3;

/// Tons of gunpowder consumed per gun per broadside.
pub const POWDER_TONS_PER_GUN: f32 = 0.01;

/// Tons of cannon shot consumed per gun per broadside.
pub const SHOT_TONS_PER_GUN: f32 = 0.01;

// ── Step 8: boarding & sinking constants ────────────────────────────────

/// Maximum closest-approach distance during a tick (NM) at which a
/// boarding action can resolve. ~0.05 NM ≈ 100 yd — grappling hooks
/// and the boarders' bowsprit can reach across that gap once the
/// pursuer's longboat has been put over the side. Smaller than
/// `CANNON_RANGE_NM` because boarding required the ships to physically
/// be alongside, not merely in pistol-shot.
pub const BOARDING_RANGE_NM: f32 = 0.05;

/// Target rigging integrity (as a fraction of `rigging_integrity_max`)
/// below which the prey is considered crippled enough to be boarded:
/// it can no longer outrun the boarders' grapples. 0.30 means "less
/// than 30% of original rig" — a half-dismasted hulk.
pub const BOARDING_RIGGING_THRESHOLD: f32 = 0.30;

/// Effective-force multiplier per unit morale. Fully-content crew
/// (`morale = 1.0`) fights at `(1 + this)` times their raw count;
/// shaken crew (`morale = 0`) fights at raw count. Models the
/// difference between an eager pirate boarding party and a press-
/// ganged merchantman's defense — see Step 9 morale plumbing.
pub const BOARDING_MORALE_BONUS: f32 = 0.5;

/// Fraction of the *loser*'s crew killed in a single-tick boarding.
/// Boarding fights were ferocious and short; survivors typically
/// surrendered rather than fought to the last man. Calibrated so a
/// well-matched fight (1:1 force) still leaves ~30% of the smaller
/// crew alive to be transferred or marooned.
pub const LOSER_CASUALTY_RATE: f32 = 0.65;

/// Fraction of the *winner*'s crew killed in a single-tick boarding.
/// Boarders take real losses even when they win — climbing aboard
/// under fire is the worst place to be on a ship.
pub const WINNER_CASUALTY_RATE: f32 = 0.20;

/// Fraction of the winning attacker's surviving crew transferred to
/// the prize as the prize crew (sailing the prize home). The remainder
/// stays on the attacker's hull. 0.5 keeps the attacker viable while
/// giving the prize enough hands to make port.
pub const PRIZE_CREW_SPLIT: f32 = 0.5;

/// Minimum closest approach (NM) between two line segments over a unit
/// tick. Both ships are modeled as travelling at constant velocity for
/// the duration of the tick — a good approximation for the 1-hour
/// hourly tick. Returns the minimum |r(t)| for t ∈ [0, 1], where
/// `r(t) = (b_pos - a_pos) + (b_vel - a_vel) * t`. Used by the
/// Resolution Phase to decide whether a broadside or boarding action
/// actually had a chance to land — end-of-tick positions alone are
/// too coarse at this granularity (ships moving 5–8 kt can pass
/// through each other's combat envelope in a single tick).
pub fn min_distance_over_tick(
    a_pos: (f32, f32),
    a_vel: (f32, f32),
    b_pos: (f32, f32),
    b_vel: (f32, f32),
) -> f32 {
    let rx0 = b_pos.0 - a_pos.0;
    let ry0 = b_pos.1 - a_pos.1;
    let dvx = b_vel.0 - a_vel.0;
    let dvy = b_vel.1 - a_vel.1;
    let dv2 = dvx * dvx + dvy * dvy;
    let t_star = if dv2 < 1e-8 {
        // Parallel motion (or both stationary): distance is constant.
        0.0
    } else {
        let t = -(rx0 * dvx + ry0 * dvy) / dv2;
        t.clamp(0.0, 1.0)
    };
    let rx = rx0 + dvx * t_star;
    let ry = ry0 + dvy * t_star;
    (rx * rx + ry * ry).sqrt()
}

/// Outcome of a deterministic single-tick boarding action.
///
/// `attacker_wins` is true iff the attacker's effective force strictly
/// exceeds the defender's. Ties go to the defender (the home-side
/// advantage of fighting from your own deck). `attacker_losses` and
/// `defender_losses` are the number of crew killed on each side and
/// have been clamped to each side's current `crew_alive` count by the
/// caller before they're applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardingOutcome {
    pub attacker_wins: bool,
    pub attacker_losses: u16,
    pub defender_losses: u16,
}

/// Resolve a boarding fight in a single tick. Force = `crew * (1 +
/// BOARDING_MORALE_BONUS * morale)`; the larger force wins. Casualties
/// are flat fractions of each side's starting crew.
///
/// Pure function: takes only the relevant per-ship scalars, returns
/// the outcome. Callers are responsible for applying the losses, the
/// prize transfer, and any state-flip side effects.
pub fn resolve_boarding(
    attacker_crew: u16,
    attacker_morale: f32,
    defender_crew: u16,
    defender_morale: f32,
) -> BoardingOutcome {
    let am = attacker_morale.clamp(0.0, 1.0);
    let dm = defender_morale.clamp(0.0, 1.0);
    let a_force = (attacker_crew as f32) * (1.0 + BOARDING_MORALE_BONUS * am);
    let d_force = (defender_crew as f32) * (1.0 + BOARDING_MORALE_BONUS * dm);
    let attacker_wins = a_force > d_force;
    let (a_rate, d_rate) = if attacker_wins {
        (WINNER_CASUALTY_RATE, LOSER_CASUALTY_RATE)
    } else {
        (LOSER_CASUALTY_RATE, WINNER_CASUALTY_RATE)
    };
    let attacker_losses = ((attacker_crew as f32) * a_rate).round() as u16;
    let defender_losses = ((defender_crew as f32) * d_rate).round() as u16;
    BoardingOutcome {
        attacker_wins,
        attacker_losses: attacker_losses.min(attacker_crew),
        defender_losses: defender_losses.min(defender_crew),
    }
}

/// Compute (hull, rigging) damage for a broadside.
///
/// `range_nm` is the slant range from attacker to target. Returns
/// `(0.0, 0.0)` for out-of-range shots so callers don't need a guard.
pub fn compute_broadside_damage(cannons: u16, range_nm: f32) -> (f32, f32) {
    if cannons == 0 || range_nm > CANNON_RANGE_NM {
        return (0.0, 0.0);
    }
    let t = (range_nm / CANNON_RANGE_NM).clamp(0.0, 1.0);
    let falloff = 1.0 + t * (LONG_RANGE_FALLOFF - 1.0);
    let guns = cannons as f32;
    (
        BROADSIDE_HULL_BASE * guns * falloff,
        BROADSIDE_RIGGING_BASE * guns * falloff,
    )
}

/// Powder and shot tonnage required to fire a single broadside of `cannons`
/// guns. Returned as `(powder, shot)`.
pub fn broadside_supply_cost(cannons: u16) -> (f32, f32) {
    let guns = cannons as f32;
    (POWDER_TONS_PER_GUN * guns, SHOT_TONS_PER_GUN * guns)
}

// ── Phase 4 §3b: sub-tick reload model ──────────────────────────────────

/// Minutes for a fully-seasoned crew (`seasoned_ratio == 1.0`) to reload
/// the great-gun battery. Three minutes ≈ the published RN ideal for
/// well-drilled 17C ships; faster crews (Nelson-era 90-second drills)
/// are out of period for this sim.
pub const RELOAD_MINUTES_SEASONED: f32 = 3.0;

/// Minutes for a fully-green crew (`seasoned_ratio == 0.0`) to reload.
/// Six minutes matches contemporary complaints about scratch crews on
/// freshly-pressed Spanish galleons and merchantmen converted to
/// privateers, where the time between effective broadsides was twice
/// or more that of veteran ships. The 2× spread between seasoned and
/// green is the single biggest payoff of the `crew_seasoned` machinery
/// (Phase 3 / A2).
pub const RELOAD_MINUTES_GREEN: f32 = 6.0;

/// Phase 4 §3b: minutes from `now` before this crew can next fire a
/// broadside. Linear interpolation in the seasoned ratio between
/// `RELOAD_MINUTES_GREEN` (ratio = 0.0) and `RELOAD_MINUTES_SEASONED`
/// (ratio = 1.0). Floored at 1 minute so the u64 sub-tick clock can
/// always make forward progress.
pub fn reload_minutes(seasoned_ratio: f32) -> u64 {
    let r = seasoned_ratio.clamp(0.0, 1.0);
    let mins = RELOAD_MINUTES_GREEN + r * (RELOAD_MINUTES_SEASONED - RELOAD_MINUTES_GREEN);
    mins.round().max(1.0) as u64
}

/// Phase 4 §3b: minutes per sub-tick step inside an engagement-locked
/// hour. 12 sub-ticks fit in one hourly tick (12 × 5 = 60).
pub const MINUTES_PER_SUB_TICK: u64 = 5;

/// Phase 4 §3b: number of sub-tick steps per hourly tick.
pub const SUB_TICKS_PER_HOUR: u64 = 12;

// ── Phase 4 §3c-1: tactical-judgment threshold ──────────────────────────

/// Range (NM) beyond which a ship may consider the engaged counterpart
/// to be "opening" rather than "closing". Used by tactical-judgment
/// conditions in `ai.rs` (e.g., a slower defender that has already
/// pulled past this threshold and is faster may safely break off).
/// Tuned to roughly two cannon ranges so a single tack outside the
/// fight envelope does not count as a real opening.
pub const ESCAPE_THRESHOLD_NM: f32 = 4.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_blank_is_full_damage() {
        let (hull, rig) = compute_broadside_damage(10, 0.0);
        assert!((hull - 10.0 * BROADSIDE_HULL_BASE).abs() < 1e-4);
        assert!((rig - 10.0 * BROADSIDE_RIGGING_BASE).abs() < 1e-4);
    }

    #[test]
    fn max_range_uses_long_range_falloff() {
        let (hull, _) = compute_broadside_damage(10, CANNON_RANGE_NM);
        let expected = 10.0 * BROADSIDE_HULL_BASE * LONG_RANGE_FALLOFF;
        assert!((hull - expected).abs() < 1e-4);
    }

    #[test]
    fn out_of_range_does_no_damage() {
        let (h, r) = compute_broadside_damage(20, CANNON_RANGE_NM + 0.01);
        assert_eq!(h, 0.0);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn damage_decreases_with_range() {
        let close = compute_broadside_damage(10, 0.1).0;
        let far = compute_broadside_damage(10, 0.4).0;
        assert!(close > far);
    }

    #[test]
    fn zero_cannons_does_nothing() {
        assert_eq!(compute_broadside_damage(0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn supply_cost_scales_with_guns() {
        let (p, s) = broadside_supply_cost(24);
        assert!((p - 0.24).abs() < 1e-4);
        assert!((s - 0.24).abs() < 1e-4);
    }

    #[test]
    fn min_distance_stationary() {
        // Both ships still at origin and (3,4) — distance is 5, constant.
        let d = min_distance_over_tick((0.0, 0.0), (0.0, 0.0), (3.0, 4.0), (0.0, 0.0));
        assert!((d - 5.0).abs() < 1e-4);
    }

    #[test]
    fn min_distance_crossing_paths() {
        // A at (0,0) heading east at 6 kt; B at (3,1) heading west at 6 kt.
        // They cross at t=0.25h, B at (1.5, 1), A at (1.5, 0) → 1.0 NM.
        let d = min_distance_over_tick((0.0, 0.0), (6.0, 0.0), (3.0, 1.0), (-6.0, 0.0));
        assert!((d - 1.0).abs() < 1e-3, "got {}", d);
    }

    #[test]
    fn min_distance_passes_close_during_tick() {
        // Ships 5 NM apart at start; one tick later, A has run past B.
        // End-of-tick distance is large, but at closest approach they're
        // alongside — the whole reason we use this helper.
        // A at (0,0) east at 10 kt; B at (5,0) east at 0 kt.
        // Relative vel = (-10,0); r0 = (5,0); t* = 0.5; closest = 0.
        let d = min_distance_over_tick((0.0, 0.0), (10.0, 0.0), (5.0, 0.0), (0.0, 0.0));
        assert!(d < 1e-3, "expected ~0, got {}", d);
    }

    #[test]
    fn min_distance_clamps_to_tick_end() {
        // Closing trajectories that won't actually meet until t > 1.
        // A at (0,0) east at 1 kt; B at (10,0) east at 0 kt.
        // Closest in unconstrained time would be at t=10, beyond tick.
        // Within [0,1] the minimum is at t=1: distance = 9.
        let d = min_distance_over_tick((0.0, 0.0), (1.0, 0.0), (10.0, 0.0), (0.0, 0.0));
        assert!((d - 9.0).abs() < 1e-3, "got {}", d);
    }

    #[test]
    fn boarding_larger_force_wins() {
        let out = resolve_boarding(40, 0.8, 20, 0.5);
        assert!(out.attacker_wins);
        // Attacker pays winner rate; defender pays loser rate.
        assert!(out.attacker_losses < out.defender_losses);
        // Defender mostly wiped.
        assert!(out.defender_losses as f32 >= 20.0 * LOSER_CASUALTY_RATE - 0.5);
    }

    #[test]
    fn boarding_defender_wins_on_tie() {
        // Exactly equal force → home-side advantage = defender wins.
        let out = resolve_boarding(20, 0.5, 20, 0.5);
        assert!(!out.attacker_wins);
    }

    #[test]
    fn boarding_morale_can_flip_outcome() {
        // Equal crew, but high-morale defender beats low-morale attacker.
        let out = resolve_boarding(25, 0.1, 25, 1.0);
        assert!(!out.attacker_wins);
        // And a high-morale attacker beats a demoralized defender.
        let out2 = resolve_boarding(25, 1.0, 25, 0.1);
        assert!(out2.attacker_wins);
    }

    #[test]
    fn boarding_losses_clamped_to_crew() {
        let out = resolve_boarding(2, 0.5, 100, 1.0);
        assert!(!out.attacker_wins);
        // Attacker has only 2 crew — losses can't exceed that even at
        // the loser casualty rate.
        assert!(out.attacker_losses <= 2);
    }
}
