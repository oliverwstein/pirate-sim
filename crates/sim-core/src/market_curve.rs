//! Bounded signed trade-balance pricing curve.
//!
//! Part of the Phase-A scaffolding for the market redesign described in
//! `planning/development-log.md`. This module is *pure*: it defines the
//! price function and bound helpers, with no dependency on `PortMarket`
//! or `Stockpile`. Phase B will wire it into the auction and replace
//! the existing stockpile-based pricing.
//!
//! # Model
//!
//! Each port × good has a signed integer `balance ∈ [-bound, +bound]`.
//! `bound` is RON-declared per port × good, scaled by prosperity. Let
//! `x = balance / bound ∈ [-1, +1]`. The price is asymmetric and
//! monotone in `x`:
//!
//! ```text
//!   x < 0  (shortage):  P(x) = base · (1 + α · (-x)^p_shortage)
//!   x ≥ 0  (glut):      P(x) = base / (1 + β · x^p_glut)
//! ```
//!
//! With the default constants (`α=4 p_shortage=2 β=1 p_glut=1.5`):
//!
//! - At `x = -1` (port empty):   price ≈ 5.0 × base
//! - At `x =  0` (neutral):      price = base
//! - At `x = +1` (port stuffed): price ≈ 0.5 × base
//!
//! The curve is **continuous**, **strictly monotone**, and **bounded**
//! on both sides — guaranteeing a unique clearing price in the
//! Phase-B fixed-point auction.

/// Shortage steepness coefficient. Larger → steeper shortage price.
pub const ALPHA_SHORTAGE: f32 = 4.0;
/// Shortage exponent. Larger → flatter near 0, steeper near -bound.
pub const P_SHORTAGE: f32 = 2.0;
/// Glut depth coefficient. Larger → deeper glut price floor.
pub const BETA_GLUT: f32 = 1.0;
/// Glut exponent. Larger → flatter near 0, steeper near +bound.
pub const P_GLUT: f32 = 1.5;

/// Compute the effective bound for a port × good, given the RON-declared
/// base bound and the port's current prosperity scalar.
///
/// Saturates at 1 to avoid divide-by-zero in `price_multiplier` when a
/// port has collapsed to zero prosperity but still has a non-zero
/// recipe entry.
pub fn effective_bound(base_bound: i32, prosperity: f32) -> i32 {
    let scaled = (base_bound as f32 * prosperity.max(0.0)).round() as i32;
    scaled.max(1)
}

/// Compute the price multiplier `P(x) / base_price` for a given signed
/// `balance` against `effective_bound`. Returns 1.0 when `balance == 0`,
/// rises monotonically as balance falls below 0, falls monotonically
/// as balance rises above 0.
///
/// Clamps `x` to `[-1, +1]` (callers should keep balance within bounds,
/// but defensive clamping protects against migration bugs and seed
/// values from the LP solver that round just outside).
pub fn price_multiplier(balance: i32, effective_bound: i32) -> f32 {
    debug_assert!(effective_bound > 0, "effective_bound must be positive");
    let x = (balance as f32 / effective_bound as f32).clamp(-1.0, 1.0);
    if x < 0.0 {
        1.0 + ALPHA_SHORTAGE * (-x).powf(P_SHORTAGE)
    } else {
        1.0 / (1.0 + BETA_GLUT * x.powf(P_GLUT))
    }
}

/// Invert the price curve: given a target price multiplier `ratio`
/// (= `target_price / base_price`), return the `x ∈ [-1, +1]` such that
/// `price_multiplier_at(x) == ratio`. Used at world-load time to seed
/// each port's balance from the Kantorovich LP's shadow prices.
///
/// Returns `1.0` (deep glut, balance = +bound) for ratios below the
/// achievable glut floor; returns `-1.0` (deep shortage) for ratios
/// above the shortage ceiling.
pub fn invert_multiplier(ratio: f32) -> f32 {
    if ratio >= 1.0 {
        // Shortage branch: ratio = 1 + α (-x)^p   ⇒   -x = ((ratio-1)/α)^(1/p)
        let max_ratio = 1.0 + ALPHA_SHORTAGE; // at x = -1
        if ratio >= max_ratio {
            return -1.0;
        }
        let neg_x = ((ratio - 1.0) / ALPHA_SHORTAGE).powf(1.0 / P_SHORTAGE);
        -neg_x.clamp(0.0, 1.0)
    } else {
        // Glut branch: ratio = 1 / (1 + β x^p)   ⇒   x = ((1/ratio - 1)/β)^(1/p)
        let min_ratio = 1.0 / (1.0 + BETA_GLUT); // at x = +1
        if ratio <= min_ratio {
            return 1.0;
        }
        let x = ((1.0 / ratio - 1.0) / BETA_GLUT).powf(1.0 / P_GLUT);
        x.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn neutral_balance_yields_base_price() {
        for bound in [1, 10, 100, 1_000, 100_000] {
            assert_eq!(price_multiplier(0, bound), 1.0);
        }
    }

    #[test]
    fn shortage_raises_price_glut_lowers_price() {
        let bound = 100;
        let neutral = price_multiplier(0, bound);
        let shortage = price_multiplier(-50, bound);
        let glut = price_multiplier(50, bound);
        assert!(shortage > neutral, "shortage should raise price");
        assert!(glut < neutral, "glut should lower price");
    }

    #[test]
    fn deep_shortage_hits_5x_default() {
        // Default constants: P(-1) = 1 + 4 = 5.0
        assert!(approx_eq(price_multiplier(-100, 100), 5.0, 1e-5));
    }

    #[test]
    fn deep_glut_hits_05x_default() {
        // Default constants: P(+1) = 1 / (1 + 1) = 0.5
        assert!(approx_eq(price_multiplier(100, 100), 0.5, 1e-5));
    }

    #[test]
    fn curve_is_strictly_monotone() {
        let bound = 1000;
        let mut prev = price_multiplier(-bound, bound);
        for b in (-bound + 1)..=bound {
            let p = price_multiplier(b, bound);
            assert!(
                p <= prev + 1e-7,
                "non-monotone at b={b}: prev={prev}, p={p}"
            );
            prev = p;
        }
    }

    #[test]
    fn clamps_balance_outside_bounds() {
        // Defensive: even if balance slips outside, price stays at
        // the cap rather than blowing up.
        let bound = 100;
        let p_below = price_multiplier(-200, bound);
        let p_at = price_multiplier(-100, bound);
        let p_above = price_multiplier(200, bound);
        let p_at_pos = price_multiplier(100, bound);
        assert!(approx_eq(p_below, p_at, 1e-5));
        assert!(approx_eq(p_above, p_at_pos, 1e-5));
    }

    #[test]
    fn invert_round_trip_within_range() {
        let bound = 1000;
        for b in [-900, -500, -100, -1, 0, 1, 100, 500, 900] {
            let ratio = price_multiplier(b, bound);
            let x = invert_multiplier(ratio);
            let b_back = (x * bound as f32).round() as i32;
            assert!(
                (b - b_back).abs() <= 1,
                "round-trip failed at b={b}: ratio={ratio}, x={x}, back={b_back}"
            );
        }
    }

    #[test]
    fn invert_clamps_extreme_ratios() {
        assert_eq!(invert_multiplier(0.001), 1.0);
        assert_eq!(invert_multiplier(1_000.0), -1.0);
        assert_eq!(invert_multiplier(1.0), 0.0);
    }

    #[test]
    fn effective_bound_scales_with_prosperity() {
        assert_eq!(effective_bound(100, 1.0), 100);
        assert_eq!(effective_bound(100, 2.0), 200);
        assert_eq!(effective_bound(100, 0.5), 50);
    }

    #[test]
    fn effective_bound_floors_at_one() {
        assert_eq!(effective_bound(100, 0.0), 1);
        assert_eq!(effective_bound(100, -1.0), 1);
        assert_eq!(effective_bound(0, 1.0), 1);
    }
}
