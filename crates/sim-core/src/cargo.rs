//! Generic per-good cargo container.
//!
//! A `Cargo` is a small map from `GoodId` to tons. Used by ships for
//! their trade hold and by ports for their stockpiles. Insertion order
//! is not preserved; lookups are linear (good count is tiny — ~9 in
//! Phase 2 — so Vec<(GoodId, f32)> is faster than HashMap and avoids
//! a hashing dep).

use crate::goods::GoodId;

#[derive(Clone, Debug, Default)]
pub struct Cargo {
    items: Vec<(GoodId, f32)>,
}

impl Cargo {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Total tons across all goods.
    pub fn total_tons(&self) -> f32 {
        self.items.iter().map(|(_, t)| *t).sum()
    }

    /// Tons of `id` currently held (0.0 if absent).
    pub fn get(&self, id: GoodId) -> f32 {
        self.items
            .iter()
            .find(|(g, _)| *g == id)
            .map(|(_, t)| *t)
            .unwrap_or(0.0)
    }

    /// Add `tons` of `id`. Tons must be non-negative.
    pub fn add(&mut self, id: GoodId, tons: f32) {
        debug_assert!(tons >= 0.0, "Cargo::add expects non-negative tons");
        if tons <= 0.0 {
            return;
        }
        if let Some(slot) = self.items.iter_mut().find(|(g, _)| *g == id) {
            slot.1 += tons;
        } else {
            self.items.push((id, tons));
        }
    }

    /// Remove up to `tons` of `id`. Returns the amount actually
    /// removed (clamped to what was available).
    pub fn remove(&mut self, id: GoodId, tons: f32) -> f32 {
        debug_assert!(tons >= 0.0, "Cargo::remove expects non-negative tons");
        let pos = match self.items.iter().position(|(g, _)| *g == id) {
            Some(p) => p,
            None => return 0.0,
        };
        let available = self.items[pos].1;
        let removed = tons.min(available);
        self.items[pos].1 -= removed;
        if self.items[pos].1 <= 0.0 {
            self.items.swap_remove(pos);
        }
        removed
    }

    /// Iterate `(GoodId, tons)` pairs for goods with positive stock.
    pub fn iter(&self) -> impl Iterator<Item = (GoodId, f32)> + '_ {
        self.items.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn clear(&mut self) {
        self.items.clear();
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
        assert!(c.is_empty(), "empty slot should be reaped");
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
    fn iter_yields_present_goods() {
        let mut c = Cargo::new();
        c.add(ids::SUGAR, 5.0);
        c.add(ids::RUM, 2.0);
        let collected: Vec<(GoodId, f32)> = c.iter().collect();
        assert_eq!(collected.len(), 2);
        let total: f32 = collected.iter().map(|(_, t)| t).sum();
        assert_eq!(total, 7.0);
    }
}
