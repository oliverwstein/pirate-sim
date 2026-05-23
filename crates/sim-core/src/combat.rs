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
}
