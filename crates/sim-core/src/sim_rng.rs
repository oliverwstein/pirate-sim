//! Project-wide deterministic PRNG. Wraps `rand_pcg::Pcg64Mcg` (a
//! statistically robust, multiplicatively-mixed PCG variant) so the rest
//! of the codebase can use one consistent type without taking a `rand_core`
//! dependency at every call site.
//!
//! Determinism contract: same seed -> same sequence on all platforms and
//! across Rust versions. `Pcg64Mcg` is specified by its multiplicative
//! constant and a 128-bit state; the `rand_pcg` crate documents the
//! algorithm as stable.
//!
//! The old hand-rolled `xorshift64` plus a multiplicative mixer worked,
//! but xorshift's low bits are known to be weak and the mixer was added
//! ad-hoc. PCG has uniform output across all bits without manual
//! whitening.

use rand_core::{RngCore, SeedableRng};
use rand_pcg::Pcg64Mcg;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SimRng(Pcg64Mcg);

impl SimRng {
    /// Seed from any `u64`. Internally widened to the 128-bit state PCG
    /// expects via `SeedableRng::seed_from_u64`, which uses splitmix64 to
    /// avoid the all-zero degenerate state.
    #[inline]
    pub fn new(seed: u64) -> Self {
        SimRng(Pcg64Mcg::seed_from_u64(seed))
    }

    /// Uniform sample in [0, 1). Uses the high 53 bits of a 64-bit draw,
    /// matching the standard double-precision construction (cast to f32).
    #[inline]
    pub fn uniform_f32(&mut self) -> f32 {
        let bits = self.0.next_u64() >> 11;
        bits as f32 / (1u64 << 53) as f32
    }

    /// Uniform sample in (0, 1] — guarantees the result is strictly above
    /// zero so callers that take `ln(u)` (Box-Muller) don't blow up.
    #[inline]
    pub fn uniform_f32_positive(&mut self) -> f32 {
        self.uniform_f32().max(1e-7)
    }

    /// One sample of N(0, 1) via Box-Muller. Two uniforms, one ln, one cos.
    #[inline]
    pub fn gaussian(&mut self) -> f32 {
        let u1 = self.uniform_f32_positive();
        let u2 = self.uniform_f32_positive();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos()
    }

    /// Raw 64-bit draw — for the rare site that wants integer modulo
    /// (e.g. `(rng.next_u64() as usize) % n`).
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0.next_u64()
    }
}

impl Default for SimRng {
    fn default() -> Self {
        // Arbitrary but non-zero default; equivalent to the pre-PCG
        // hand-rolled state's most-common initialiser.
        Self::new(0x9E37_79B9_7F4A_7C15)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determinism_same_seed_same_sequence() {
        let mut a = SimRng::new(42);
        let mut b = SimRng::new(42);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn uniform_in_range() {
        let mut r = SimRng::new(7);
        for _ in 0..1024 {
            let v = r.uniform_f32();
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn gaussian_mean_near_zero() {
        let mut r = SimRng::new(1);
        let mut s = 0.0;
        let n = 4096;
        for _ in 0..n {
            s += r.gaussian();
        }
        let mean = s / n as f32;
        assert!(mean.abs() < 0.1, "mean = {mean}");
    }
}
