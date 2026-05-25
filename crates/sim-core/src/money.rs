//! Fixed-point currency for the simulation.
//!
//! `Pesos` is the canonical money type for every stored balance —
//! ship strongboxes, port treasuries, chandler debts, lifetime
//! dividends, wage arrears. Internally `i64` *centavos* (hundredths of
//! a peso), which matches the real specie of the period (the Spanish
//! peso de a ocho subdivided naturally into 8 reales, but for
//! accounting purposes the hundredth is granular enough to render any
//! per-hour wage or per-ton trade margin without rounding drift).
//!
//! ### Why fixed point
//!
//! The simulation runs ~17 500 hourly ticks per in-game year and
//! every ship may transact several times per dock visit. `f32`
//! accumulators leak fractional pesos over long runs and produce
//! comparisons like `cost > silver + 1e-4` to mask the drift. With
//! `Pesos`, every store/sub/cmp is exact integer arithmetic; the only
//! rounding happens when an `f32`-computed bill (price × tons,
//! per-HP repair charge, etc.) is converted at the boundary, and that
//! rounding is a single deterministic floor that never compounds.
//!
//! ### Boundary conversions
//!
//! Use `Pesos::from_pesos_f32` (or `from_centavos_f32`) when computing
//! a price/bill in floating point and assigning it into a balance.
//! Use `Pesos::as_pesos_f32` only for diagnostics, display, and the
//! equilibrium LP report (which is a mathematical baseline, not a
//! ledger). Never round-trip a stored balance through `f32`.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Pesos held to centavo precision. Negative values are permitted
/// (e.g. an overdrawn float on the way to repayment).
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Pesos(i64);

impl Pesos {
    pub const ZERO: Pesos = Pesos(0);

    /// Construct directly from a whole-peso amount.
    #[inline]
    pub const fn from_pesos(p: i64) -> Self {
        Pesos(p.saturating_mul(100))
    }

    /// Construct from raw centavos.
    #[inline]
    pub const fn from_centavos(c: i64) -> Self {
        Pesos(c)
    }

    /// Convert an `f32` peso amount to fixed-point. Rounds half-to-even
    /// (banker's rounding) on the centavo grain, which keeps long
    /// accumulating streams (e.g. hourly wage accrual) unbiased.
    #[inline]
    pub fn from_pesos_f32(p: f32) -> Self {
        if !p.is_finite() {
            return Pesos::ZERO;
        }
        let centavos = (p as f64 * 100.0).round() as i64;
        Pesos(centavos)
    }

    /// Raw centavo count.
    #[inline]
    pub const fn as_centavos(self) -> i64 {
        self.0
    }

    /// Lossy conversion back to floating point for display / analytics.
    /// Do not round-trip a stored balance through this.
    #[inline]
    pub fn as_pesos_f32(self) -> f32 {
        self.0 as f32 / 100.0
    }

    /// Lossy conversion back to floating point for display / analytics.
    #[inline]
    pub fn as_pesos_f64(self) -> f64 {
        self.0 as f64 / 100.0
    }

    /// Multiply by a non-negative scalar (tons, fraction, etc.) and
    /// round to the nearest centavo. Useful for `price × tons`-style
    /// bills where both factors are floats but the result must land
    /// in the integer ledger.
    #[inline]
    pub fn scale(self, factor: f32) -> Self {
        if !factor.is_finite() {
            return Pesos::ZERO;
        }
        let c = (self.0 as f64 * factor as f64).round() as i64;
        Pesos(c)
    }

    /// Clamp at zero (returns `ZERO` if negative).
    #[inline]
    pub fn max_zero(self) -> Self {
        if self.0 < 0 {
            Pesos::ZERO
        } else {
            self
        }
    }

    /// Saturating subtraction that never goes below zero — convenient
    /// for "spend at most what you have" semantics.
    #[inline]
    pub fn saturating_sub(self, rhs: Self) -> Self {
        Pesos((self.0 - rhs.0).max(0))
    }

    /// Min/max helpers.
    #[inline]
    pub fn min(self, rhs: Self) -> Self {
        Pesos(self.0.min(rhs.0))
    }
    #[inline]
    pub fn max(self, rhs: Self) -> Self {
        Pesos(self.0.max(rhs.0))
    }

    #[inline]
    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    #[inline]
    pub fn is_positive(self) -> bool {
        self.0 > 0
    }

    #[inline]
    pub fn is_negative(self) -> bool {
        self.0 < 0
    }
}

impl Add for Pesos {
    type Output = Pesos;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Pesos(self.0 + rhs.0)
    }
}

impl Sub for Pesos {
    type Output = Pesos;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Pesos(self.0 - rhs.0)
    }
}

impl AddAssign for Pesos {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl SubAssign for Pesos {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl Neg for Pesos {
    type Output = Pesos;
    #[inline]
    fn neg(self) -> Self::Output {
        Pesos(-self.0)
    }
}

impl Sum for Pesos {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Pesos::ZERO, |a, b| a + b)
    }
}

impl<'a> Sum<&'a Pesos> for Pesos {
    fn sum<I: Iterator<Item = &'a Pesos>>(iter: I) -> Self {
        iter.copied().fold(Pesos::ZERO, |a, b| a + b)
    }
}

impl fmt::Display for Pesos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let whole = self.0 / 100;
        let frac = (self.0 % 100).abs();
        write!(f, "{}.{:02} pesos", whole, frac)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_peso_construction_and_arithmetic() {
        let a = Pesos::from_pesos(50);
        let b = Pesos::from_pesos(30);
        assert_eq!((a + b).as_centavos(), 8000);
        assert_eq!((a - b).as_centavos(), 2000);
    }

    #[test]
    fn from_f32_rounds_to_centavo() {
        assert_eq!(Pesos::from_pesos_f32(12.345).as_centavos(), 1235);
        assert_eq!(Pesos::from_pesos_f32(12.344).as_centavos(), 1234);
        assert_eq!(Pesos::from_pesos_f32(-0.014).as_centavos(), -1);
    }

    #[test]
    fn scale_applies_float_and_rounds() {
        let unit = Pesos::from_pesos(10); // 1000 centavos
        let bill = unit.scale(3.5); // 3500 centavos
        assert_eq!(bill.as_centavos(), 3500);
        let bill = unit.scale(0.001); // 1 centavo (0.01 * 100 = 1)
        assert_eq!(bill.as_centavos(), 1);
    }

    #[test]
    fn saturating_sub_never_goes_negative() {
        let a = Pesos::from_pesos(5);
        let b = Pesos::from_pesos(7);
        assert_eq!(a.saturating_sub(b), Pesos::ZERO);
        assert_eq!(b.saturating_sub(a).as_centavos(), 200);
    }

    #[test]
    fn no_drift_under_repeated_addition() {
        // Adding the same f32-derived bill many times must not drift,
        // because we round once at construction and then do exact i64
        // arithmetic forever after. Contrast with f32 accumulation of
        // 0.1, which famously drifts.
        let bill = Pesos::from_pesos_f32(0.10);
        let mut total = Pesos::ZERO;
        for _ in 0..10_000 {
            total += bill;
        }
        assert_eq!(total.as_centavos(), 100_000);
    }

    #[test]
    #[allow(clippy::inconsistent_digit_grouping)]
    fn display_renders_two_decimals() {
        assert_eq!(format!("{}", Pesos::from_centavos(123_45)), "123.45 pesos");
        assert_eq!(format!("{}", Pesos::from_centavos(-7_05)), "-7.05 pesos");
    }

    #[test]
    fn sum_iterator() {
        let xs = [
            Pesos::from_pesos(10),
            Pesos::from_pesos(20),
            Pesos::from_pesos(3),
        ];
        let s: Pesos = xs.iter().copied().sum();
        assert_eq!(s.as_centavos(), 3300);
    }
}
