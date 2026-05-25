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
//! ### Storage
//!
//! Internally a single `Vec<Entry>` sorted by cell key. Sorting once
//! per rebuild and binary-searching for each cell touched by a query
//! is significantly cache-friendlier than a `BTreeMap` of per-cell
//! `Vec`s — the entire dataset is contiguous, prefetchable, and free
//! of node-pointer chases. Sort uses `sort_by` keyed on cell + ShipId
//! so the order is total and fully deterministic across runs.

use crate::types::{Position, ShipId};

/// Edge length of one spatial-hash cell, in nautical miles. See module
/// docstring for the rationale.
pub const SPATIAL_CELL_NM: f32 = 10.0;

/// Packed (cell_x, cell_y) — sortable and cheap to compare.
type CellKey = (i32, i32);

#[derive(Debug, Clone, Copy)]
struct Entry {
    cell: CellKey,
    id: ShipId,
    pos: Position,
}

/// Uniform-grid spatial index. Build by clearing, inserting all
/// entries, and querying with `neighbors`; the first query after
/// inserts triggers a sort that puts entries from the same cell into
/// a contiguous slice.
#[derive(Debug, Default, Clone)]
pub struct SpatialHash {
    entries: Vec<Entry>,
    /// True iff `entries` is currently sorted by `cell`. Cleared by
    /// `insert`, set by the lazy sort inside `neighbors`. Avoids
    /// re-sorting on every query when no inserts have happened.
    sorted: bool,
}

#[inline]
fn cell_of(pos: Position) -> CellKey {
    (
        (pos.x / SPATIAL_CELL_NM).floor() as i32,
        (pos.y / SPATIAL_CELL_NM).floor() as i32,
    )
}

impl SpatialHash {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop all entries. Cheap; the backing Vec retains its capacity
    /// for reuse on the next rebuild.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.sorted = true; // empty is trivially sorted
    }

    /// Insert one ship at `pos`.
    pub fn insert(&mut self, id: ShipId, pos: Position) {
        self.entries.push(Entry {
            cell: cell_of(pos),
            id,
            pos,
        });
        self.sorted = false;
    }

    /// Total number of inserted ships (for diagnostics / tests).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Sort entries by cell key, breaking ties on ShipId so the
    /// ordering is total and deterministic. Idempotent. Call once
    /// after all inserts and before any queries; `neighbors` panics
    /// (in debug) if called on a dirty index, since silently sorting
    /// inside a read-only query would force `&mut self` to propagate
    /// through every caller.
    pub fn finalize(&mut self) {
        if self.sorted {
            return;
        }
        // sort_unstable is acceptable because the key (cell, id) is a
        // total order — no two entries can compare equal (a given
        // ShipId appears at most once per rebuild).
        self.entries
            .sort_unstable_by(|a, b| a.cell.cmp(&b.cell).then_with(|| a.id.cmp(&b.id)));
        self.sorted = true;
    }

    /// Return the contiguous slice of entries whose cell key equals
    /// `cell`, or an empty slice if no such cell exists. Caller must
    /// have already ensured the entries are sorted.
    #[inline]
    fn cell_slice(&self, cell: CellKey) -> &[Entry] {
        // partition_point gives the first index >= cell; we then scan
        // forward while the cell key matches. With sort by cell, all
        // matching entries are contiguous.
        let start = self.entries.partition_point(|e| e.cell < cell);
        // Find end by another partition_point on the strict-greater
        // predicate, but it's usually faster to scan since cells are
        // small (typically <10 ships per 10-NM cell).
        let mut end = start;
        while end < self.entries.len() && self.entries[end].cell == cell {
            end += 1;
        }
        &self.entries[start..end]
    }

    /// All ships within `range_nm` of `pos` (true Euclidean distance)
    /// for which `filter(id)` returns true. The filter is invoked only
    /// for entries that pass the distance check. Order of returned ids
    /// is deterministic.
    pub fn neighbors<F>(&self, pos: Position, range_nm: f32, mut filter: F) -> Vec<ShipId>
    where
        F: FnMut(ShipId) -> bool,
    {
        debug_assert!(
            self.sorted,
            "SpatialHash::neighbors called before finalize(); call finalize() after inserts"
        );
        let mut out = Vec::new();
        let r2 = range_nm * range_nm;
        let span = (range_nm / SPATIAL_CELL_NM).ceil() as i32;
        let (cx, cy) = cell_of(pos);
        for dx in -span..=span {
            for dy in -span..=span {
                for entry in self.cell_slice((cx + dx, cy + dy)) {
                    if entry.pos.distance_squared(pos) <= r2 && filter(entry.id) {
                        out.push(entry.id);
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
        let mut sh = SpatialHash::new();
        sh.finalize();
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
        sh.finalize();

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
        sh.finalize();

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
        sh.finalize();

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
        let ids = mk_ids(2);
        let mut sh = SpatialHash::new();
        sh.insert(ids[0], Position::new(-1.0, -1.0));
        sh.insert(ids[1], Position::new(1.0, 1.0));
        sh.finalize();
        let near = sh.neighbors(Position::new(1.0, 1.0), 0.5, |_| true);
        assert_eq!(near, vec![ids[1]]);
    }

    #[test]
    fn deterministic_neighbor_order_across_rebuilds() {
        let ids = mk_ids(5);
        let positions = [
            Position::new(0.0, 0.0),
            Position::new(2.0, 0.0),
            Position::new(0.0, 2.0),
            Position::new(2.0, 2.0),
            Position::new(50.0, 50.0),
        ];
        let mut a = SpatialHash::new();
        let mut b = SpatialHash::new();
        for i in 0..ids.len() {
            a.insert(ids[i], positions[i]);
            b.insert(ids[i], positions[i]);
        }
        a.finalize();
        b.finalize();
        let na = a.neighbors(Position::new(0.0, 0.0), 5.0, |_| true);
        let nb = b.neighbors(Position::new(0.0, 0.0), 5.0, |_| true);
        assert_eq!(na, nb);
    }
}
