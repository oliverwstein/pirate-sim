//! Generic per-good cargo container.
//!
//! A `Cargo` is a flat fixed-size array of tons-per-good. Used by ships
//! for their trade hold and by ports for their stockpiles. Holding the
//! data inline (no heap allocation, no pointer chase) keeps `Ship`
//! cache-friendly: `Cargo` is `[f32; CARGO_SLOTS]` (64 bytes — exactly
//! one cache line on common x86_64 / aarch64), so `Ship::cargo.get(id)`
//! is a single indexed load with no branch.
//!
//! `CARGO_SLOTS` is padded above the live good count so the goods
//! catalog can grow without resizing this type. The starter catalog has
//! 11 goods (see `goods::GoodsRegistry`), and 16 slots leaves clean
//! headroom while keeping the inline footprint to one cache line.
//!
//! `GoodId(0..CARGO_SLOTS)` is implicit; out-of-range ids will panic in
//! debug builds and silently no-op in release. Callers should only
//! address goods declared in `data/registries/goods.ron`.

use crate::goods::GoodId;

/// Maximum number of goods addressable by a `Cargo`. Sized for
/// near-term growth of `goods.ron` (starter = 11) and tuned to keep
/// the type at one cache line (16 × f32 = 64 bytes).
pub const CARGO_SLOTS: usize = 16;

#[derive(Clone, Debug)]
pub struct Cargo {
    tons: [f32; CARGO_SLOTS],
}

impl Default for Cargo {
    fn default() -> Self {
        Self::new()
    }
}

impl Cargo {
    pub const fn new() -> Self {
        Self {
            tons: [0.0; CARGO_SLOTS],
        }
    }

    #[inline]
    fn idx(id: GoodId) -> usize {
        debug_assert!(
            (id.0 as usize) < CARGO_SLOTS,
            "GoodId {} >= CARGO_SLOTS ({}); enlarge cargo::CARGO_SLOTS",
            id.0,
            CARGO_SLOTS,
        );
        id.0 as usize
    }

    /// Total tons across all goods.
    pub fn total_tons(&self) -> f32 {
        self.tons.iter().copied().sum()
    }

    /// Tons of `id` currently held (0.0 if absent or out-of-range).
    #[inline]
    pub fn get(&self, id: GoodId) -> f32 {
        let i = id.0 as usize;
        if i < CARGO_SLOTS {
            self.tons[i]
        } else {
            debug_assert!(false, "GoodId {} out of range", id.0);
            0.0
        }
    }

    /// Add `tons` of `id`. Tons must be non-negative.
    #[inline]
    pub fn add(&mut self, id: GoodId, tons: f32) {
        debug_assert!(tons >= 0.0, "Cargo::add expects non-negative tons");
        if tons <= 0.0 {
            return;
        }
        self.tons[Self::idx(id)] += tons;
    }

    /// Remove up to `tons` of `id`. Returns the amount actually removed
    /// (clamped to what was available).
    pub fn remove(&mut self, id: GoodId, tons: f32) -> f32 {
        debug_assert!(tons >= 0.0, "Cargo::remove expects non-negative tons");
        let i = Self::idx(id);
        let available = self.tons[i];
        let removed = tons.min(available);
        self.tons[i] = (available - removed).max(0.0);
        removed
    }

    /// Iterate `(GoodId, tons)` pairs for goods with positive stock.
    /// Order is by `GoodId` (ascending), which is deterministic and
    /// stable across runs.
    pub fn iter(&self) -> impl Iterator<Item = (GoodId, f32)> + '_ {
        self.tons
            .iter()
            .enumerate()
            .filter_map(|(i, &t)| (t > 0.0).then_some((GoodId(i as u8), t)))
    }

    pub fn is_empty(&self) -> bool {
        self.tons.iter().all(|&t| t <= 0.0)
    }

    /// Number of slots with positive stock.
    pub fn len(&self) -> usize {
        self.tons.iter().filter(|&&t| t > 0.0).count()
    }

    pub fn clear(&mut self) {
        self.tons = [0.0; CARGO_SLOTS];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goods::ids;

    #[test]
    fn new_cargo_is_empty() {
        let c = Cargo::new();
        assert!(c.is_empty());
        assert_eq!(c.total_tons(), 0.0);
        assert_eq!(c.get(ids::SUGAR), 0.0);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn add_accumulates() {
        let mut c = Cargo::new();
        c.add(ids::SUGAR, 5.0);
        c.add(ids::SUGAR, 3.0);
        c.add(ids::RUM, 2.0);
        assert_eq!(c.get(ids::SUGAR), 8.0);
        assert_eq!(c.get(ids::RUM), 2.0);
        assert_eq!(c.total_tons(), 10.0);
    }

    #[test]
    fn remove_clamps_to_available() {
        let mut c = Cargo::new();
        c.add(ids::SUGAR, 5.0);
        let removed = c.remove(ids::SUGAR, 10.0);
        assert_eq!(removed, 5.0);
        assert_eq!(c.get(ids::SUGAR), 0.0);
        assert!(c.is_empty(), "drained slot should report empty");
    }

    #[test]
    fn remove_partial_keeps_slot() {
        let mut c = Cargo::new();
        c.add(ids::SUGAR, 5.0);
        let removed = c.remove(ids::SUGAR, 2.0);
        assert_eq!(removed, 2.0);
        assert_eq!(c.get(ids::SUGAR), 3.0);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn remove_missing_returns_zero() {
        let mut c = Cargo::new();
        c.add(ids::SUGAR, 5.0);
        let removed = c.remove(ids::RUM, 1.0);
        assert_eq!(removed, 0.0);
        assert_eq!(c.get(ids::SUGAR), 5.0);
    }

    #[test]
    fn add_zero_is_noop() {
        let mut c = Cargo::new();
        c.add(ids::SUGAR, 0.0);
        assert!(c.is_empty());
    }

    #[test]
    fn iter_yields_positive_goods_in_id_order() {
        let mut c = Cargo::new();
        // Insert out of GoodId order to confirm iteration is by id, not insertion.
        c.add(ids::RUM, 2.0);
        c.add(ids::SUGAR, 5.0);
        c.add(ids::MANUFACTURES, 1.0);
        let collected: Vec<(GoodId, f32)> = c.iter().collect();
        assert_eq!(collected.len(), 3);
        // GoodId order: SUGAR(1) < RUM(3) < MANUFACTURES(5)
        assert_eq!(collected[0].0, ids::SUGAR);
        assert_eq!(collected[1].0, ids::RUM);
        assert_eq!(collected[2].0, ids::MANUFACTURES);
        let total: f32 = collected.iter().map(|(_, t)| t).sum();
        assert_eq!(total, 8.0);
    }

    #[test]
    fn slots_fit_one_cache_line() {
        // Compile-time guard against accidentally enlarging Cargo
        // beyond a 64-byte cache line.
        assert_eq!(std::mem::size_of::<Cargo>(), 64);
    }
}
