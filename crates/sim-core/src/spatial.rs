//! Dynamic spatial hash for inter-ship interaction queries at sea.
//!
//! Indexes ships by their world position into a uniform grid of
//! `SPATIAL_CELL_NM`-sized cells. Rebuilt every hourly tick (over
//! `Sailing` ships) inside `tick_hourly_ai_and_physics` — see
//! `world.rs`. This is the read substrate for "what other ships are
//! near me" queries: viz sight-lines (Step 4.d), and in Step 6+ the
//! `SeePrey` / `Pursue` / `Flee` BT conditions.
//!
//! ### API design notes
//!
//! `neighbors` takes a filter closure so callers can express
//! faction-aware queries (e.g. "all ships within visual range that are
//! not of my faction") without doing a second pass. The closure
//! operates on `ShipId` only; callers that need ship fields look them
//! up in `World::ships` from inside the closure. The closure is run
//! AFTER the actual Euclidean distance check, so it is invoked only
//! on true neighbors rather than on every entry in the touched cells.
//!
//! Cell size of 10 NM matches typical 17C sea visibility (~10 NM from
//! a quarterdeck on a clear day; the British naval signal range was
//! similarly bounded). A range query of `r` NM touches at most a
//! `ceil(2r / cell) + 1` × `ceil(2r / cell) + 1` block of cells.
//!
//! Storage uses `BTreeMap` rather than `HashMap` so iteration order
//! is deterministic across runs — important for reproducible benches.

use std::collections::BTreeMap;

use crate::types::{Position, ShipId};

/// Edge length of one spatial-hash cell, in nautical miles. See module
/// docstring for the rationale.
pub const SPATIAL_CELL_NM: f32 = 10.0;

/// Uniform-grid spatial index keyed by `(cell_x, cell_y)` integer
/// coordinates. Values are `(ShipId, exact_position)` tuples so range
/// queries can do precise distance checks without external lookups.
#[derive(Debug, Default, Clone)]
pub struct SpatialHash {
    cells: BTreeMap<(i32, i32), Vec<(ShipId, Position)>>,
}

#[inline]
fn cell_of(pos: Position) -> (i32, i32) {
    (
        (pos.x / SPATIAL_CELL_NM).floor() as i32,
        (pos.y / SPATIAL_CELL_NM).floor() as i32,
    )
}

impl SpatialHash {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop all entries. Cheap; the BTreeMap retains its allocated
    /// node arena for reuse on the next rebuild.
    pub fn clear(&mut self) {
        self.cells.clear();
    }

    /// Insert one ship at `pos` into the cell it occupies.
    pub fn insert(&mut self, id: ShipId, pos: Position) {
        self.cells.entry(cell_of(pos)).or_default().push((id, pos));
    }

    /// Total number of inserted ships (for diagnostics / tests).
    pub fn len(&self) -> usize {
        self.cells.values().map(|v| v.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// All ships within `range_nm` of `pos` (true Euclidean distance)
    /// for which `filter(id)` returns true. The filter is invoked only
    /// for entries that pass the distance check. Order of returned ids
    /// is deterministic (BTreeMap iteration over cells, then insertion
    /// order within each cell).
    pub fn neighbors<F>(&self, pos: Position, range_nm: f32, mut filter: F) -> Vec<ShipId>
    where
        F: FnMut(ShipId) -> bool,
    {
        let mut out = Vec::new();
        let r2 = range_nm * range_nm;
        // Bounding box of cells that could contain a ship within range.
        let span = (range_nm / SPATIAL_CELL_NM).ceil() as i32;
        let (cx, cy) = cell_of(pos);
        for dx in -span..=span {
            for dy in -span..=span {
                if let Some(bucket) = self.cells.get(&(cx + dx, cy + dy)) {
                    for (id, p) in bucket {
                        if p.distance_squared(pos) <= r2 && filter(*id) {
                            out.push(*id);
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slotmap::SlotMap;

    fn mk_ids(n: usize) -> Vec<ShipId> {
        let mut sm: SlotMap<ShipId, ()> = SlotMap::with_key();
        (0..n).map(|_| sm.insert(())).collect()
    }

    #[test]
    fn empty_hash_returns_nothing() {
        let sh = SpatialHash::new();
        let v = sh.neighbors(Position::new(0.0, 0.0), 100.0, |_| true);
        assert!(v.is_empty());
        assert!(sh.is_empty());
    }

    #[test]
    fn insert_and_query_returns_only_in_range() {
        let ids = mk_ids(4);
        let mut sh = SpatialHash::new();
        sh.insert(ids[0], Position::new(0.0, 0.0));
        sh.insert(ids[1], Position::new(5.0, 0.0)); // 5 NM east
        sh.insert(ids[2], Position::new(50.0, 0.0)); // 50 NM east
        sh.insert(ids[3], Position::new(0.0, 9.0)); // 9 NM north

        // Range 10 NM from origin: should catch 0, 1, 3 but not 2.
        let mut got = sh.neighbors(Position::new(0.0, 0.0), 10.0, |_| true);
        got.sort();
        let mut want = vec![ids[0], ids[1], ids[3]];
        want.sort();
        assert_eq!(got, want);

        assert_eq!(sh.len(), 4);
    }

    #[test]
    fn distance_uses_euclidean_not_cell_membership() {
        // Two ships both in the *same* 10NM cell, but 9NM apart along
        // the diagonal — should be returned for r=10 but not r=8.
        let ids = mk_ids(2);
        let mut sh = SpatialHash::new();
        sh.insert(ids[0], Position::new(0.5, 0.5));
        sh.insert(ids[1], Position::new(7.0, 7.0)); // ~9.19 NM away

        let near = sh.neighbors(Position::new(0.5, 0.5), 10.0, |_| true);
        assert!(near.contains(&ids[0]) && near.contains(&ids[1]));

        let close = sh.neighbors(Position::new(0.5, 0.5), 8.0, |_| true);
        assert!(close.contains(&ids[0]) && !close.contains(&ids[1]));
    }

    #[test]
    fn filter_closure_excludes_matching_ids() {
        let ids = mk_ids(3);
        let mut sh = SpatialHash::new();
        sh.insert(ids[0], Position::new(0.0, 0.0));
        sh.insert(ids[1], Position::new(2.0, 0.0));
        sh.insert(ids[2], Position::new(3.0, 0.0));

        let me = ids[0];
        let others = sh.neighbors(Position::new(0.0, 0.0), 100.0, |id| id != me);
        assert!(!others.contains(&me));
        assert!(others.contains(&ids[1]) && others.contains(&ids[2]));
    }

    #[test]
    fn clear_empties_hash() {
        let ids = mk_ids(2);
        let mut sh = SpatialHash::new();
        sh.insert(ids[0], Position::new(0.0, 0.0));
        sh.insert(ids[1], Position::new(1.0, 0.0));
        assert_eq!(sh.len(), 2);
        sh.clear();
        assert_eq!(sh.len(), 0);
        assert!(sh.is_empty());
    }

    #[test]
    fn cells_partition_negative_coords() {
        // Verifies cell_of handles negative positions correctly (floor,
        // not truncate). Ships at (-1, -1) and (1, 1) live in cells
        // (-1, -1) and (0, 0) respectively.
        let ids = mk_ids(2);
        let mut sh = SpatialHash::new();
        sh.insert(ids[0], Position::new(-1.0, -1.0));
        sh.insert(ids[1], Position::new(1.0, 1.0));
        assert_eq!(sh.cells.len(), 2);
    }
}
