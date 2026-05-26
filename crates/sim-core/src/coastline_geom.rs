//! Polygon-aware coastline geometry queries.
//!
//! This is the **Phase 1** primitive for the navigation overhaul (see
//! `planning/development-log.md` entry "Coastal navigation overhaul —
//! design agreed"). It exposes three queries that the rest of the
//! nav stack will migrate to in later phases:
//!
//! - [`CoastlineGeom::is_land`]
//! - [`CoastlineGeom::line_is_clear`]
//! - [`CoastlineGeom::first_land_hit`]
//!
//! All three are **bilevel**: the [`LandMap`] raster acts as a fast
//! positive filter, and the polygon mesh is consulted only at coastal
//! cells where the 1 NM raster staircases the true coastline. This
//! keeps open-water queries at raster cost while giving full polygon
//! resolution exactly where the player sees the difference.
//!
//! The coastline mesh is held in two uniform-grid spatial indices:
//!
//! 1. [`CoastlineEdgeIndex`] — buckets of coastline polyline segments,
//!    used by `line_is_clear` and `first_land_hit` for segment-vs-edge
//!    intersection tests.
//! 2. [`LandTriangleIndex`] — buckets of land triangles (from the
//!    pre-triangulated [`LandMesh`]), used by `is_land` for
//!    point-in-triangle tests.
//!
//! Both indices use the same bucket size and origin so a position →
//! bucket key conversion is shared. Buckets are sized at
//! [`BUCKET_NM`] = 10 NM, chosen so the average bucket holds a few
//! dozen primitives across the Caribbean + Atlantic basin (segment
//! count ~50k, triangle count ~few × 10k).
//!
//! This module has **no production callers yet**. It is exercised by
//! unit tests and the `diag_polygon_los` example (Phase 1 deliverable);
//! Phase 3 will thread it through `PathfindContext`, `compute_steering`,
//! `deflect_for_land`, and the `world.rs` motion sweep.

use crate::coastline::{CoastlineMap, LandMesh};
use crate::map::land::LandMap;
use crate::types::Position;

/// Bucket edge length (NM) for both spatial indices. Larger buckets
/// reduce build time and memory but increase per-query primitive
/// scan length. 10 NM is a sweet spot for our edge density:
/// ~50k coastline segments / (map-area / 100 NM²) ≈ a few dozen
/// edges per coastal bucket, with most open-water buckets empty.
pub const BUCKET_NM: f32 = 10.0;

/// One coastline polyline segment as the spatial index sees it:
/// the segment's two endpoints in world (NM) coordinates. We store
/// the geometry directly rather than indices into the source
/// `CoastlineMap` because polylines are short and de-referencing
/// during the inner loop of LOS tests would cost more than the
/// 16 bytes of duplication per edge.
#[derive(Debug, Clone, Copy)]
pub struct CoastEdge {
    pub a: Position,
    pub b: Position,
}

/// Uniform-grid spatial index over coastline polyline segments. Each
/// cell of the grid stores the list of edges whose bounding box
/// touches that cell — an edge that crosses a bucket boundary appears
/// in every bucket it touches, so a segment LOS query is "look up the
/// buckets the query segment passes through and union their edges".
pub struct CoastlineEdgeIndex {
    /// NW corner of bucket (0, 0) in world coords.
    origin: Position,
    /// Grid dims in buckets.
    cols: u32,
    rows: u32,
    /// `cols × rows` length. Each entry is the edges in that bucket.
    buckets: Vec<Vec<CoastEdge>>,
}

/// One triangle from the [`LandMesh`] as the spatial index sees it.
#[derive(Debug, Clone, Copy)]
pub struct LandTriangle {
    pub v0: Position,
    pub v1: Position,
    pub v2: Position,
}

/// Uniform-grid spatial index over land triangles. The grid has the
/// same origin + bucket size as [`CoastlineEdgeIndex`] so a single
/// `(col, row)` lookup serves both queries.
pub struct LandTriangleIndex {
    origin: Position,
    cols: u32,
    rows: u32,
    buckets: Vec<Vec<LandTriangle>>,
}

/// Bundle: coastline edge index + land triangle index. The primary
/// entry point for the polygon-aware nav stack.
///
/// **Ownership.** This struct is fully owned (no borrows) so it can
/// live on `World` for the duration of a run. The bilevel queries
/// take a `&LandMap` parameter for the raster pre-filter — keeping
/// the raster fast-path without forcing a self-referential lifetime.
pub struct CoastlineGeom {
    coast: CoastlineEdgeIndex,
    polys: LandTriangleIndex,
    /// True iff the coastline polyline source provided any edges.
    /// When false, `line_is_clear` / `first_land_hit` degrade to the
    /// raster verdict (test scaffolding without `coastline.bin`).
    has_polylines: bool,
    /// True iff the land triangle mesh provided any triangles.
    /// When false, `is_land` degrades to the raster verdict.
    has_triangles: bool,
}

impl CoastlineGeom {
    /// Build the geometry bundle. `coastline` provides the polyline
    /// edges for LOS tests; `mesh` provides the triangles for
    /// point-in-polygon tests. Either may be empty independently;
    /// each query degrades to raster-only when *its* polygon source
    /// is missing.
    ///
    /// `land` is used at build time to fix the index origin/extent
    /// and is not retained. Queries take `&LandMap` directly.
    pub fn build(land: &LandMap, coastline: &CoastlineMap, mesh: &LandMesh) -> Self {
        let coast = CoastlineEdgeIndex::build(land, coastline);
        let polys = LandTriangleIndex::build(land, mesh);
        let has_polylines = !coastline.lines.is_empty();
        let has_triangles = !mesh.vertices.is_empty() && !mesh.indices.is_empty();
        Self {
            coast,
            polys,
            has_polylines,
            has_triangles,
        }
    }

    /// Empty geometry — convenience for tests that want a no-op
    /// `CoastlineGeom`. All polygon queries degrade to raster-only.
    pub fn empty(land: &LandMap) -> Self {
        Self::build(land, &CoastlineMap::default(), &LandMesh::default())
    }

    /// True iff `pos` is land. Bilevel:
    /// - Raster says sea → return `false` (open water; the polygons
    ///   never *add* land to a known sea cell, only refine known land
    ///   cells).
    /// - Raster says land + triangles available → consult the polygon
    ///   mesh. If `pos` is inside any land triangle → true; otherwise
    ///   false (the raster mis-classified a sea cell as land due to
    ///   coastline staircasing).
    /// - Raster says land + no triangles loaded → fall through to the
    ///   raster verdict.
    pub fn is_land(&self, land: &LandMap, pos: Position) -> bool {
        if !land.is_land(pos) {
            return false;
        }
        if !self.has_triangles {
            return true;
        }
        self.polys.contains(pos)
    }

    /// True iff the straight segment from `a` to `b` does not cross any
    /// land. Bilevel:
    /// - Walk the raster cells the segment crosses; if every cell is
    ///   sea, return `true` immediately (open-water fast path).
    /// - Otherwise, if polylines are available, run a
    ///   segment-vs-coastline-edges test over just the buckets the
    ///   segment passes through. No polylines → mirror the raster
    ///   verdict.
    pub fn line_is_clear(&self, land: &LandMap, a: Position, b: Position) -> bool {
        if !raster_corridor_touches_land(land, a, b) {
            return true;
        }
        if !self.has_polylines {
            return false;
        }
        let mut blocked = false;
        self.coast.for_each_bucket_along(a, b, |edges| {
            for e in edges {
                if segments_intersect(a, b, e.a, e.b) {
                    blocked = true;
                    return false;
                }
            }
            true
        });
        !blocked
    }

    /// True iff the rectangular corridor of half-width `margin_nm`
    /// centered on the segment `a → b` does not cross any land. Three
    /// parallel polygon-LOS rays (center plus ±perpendicular offsets)
    /// — exact for any land polygon whose footprint reaches at least
    /// one of the three rays, which covers every coastline feature in
    /// the bundled mesh at the planner's 2 NM margin. Falls back to
    /// the raster `LandMap::corridor_is_clear` when no polylines are
    /// loaded (test fixtures, etc.).
    pub fn corridor_is_clear(
        &self,
        land: &LandMap,
        a: Position,
        b: Position,
        margin_nm: f32,
    ) -> bool {
        if !self.has_polylines {
            return land.corridor_is_clear(a, b, margin_nm);
        }
        if !self.line_is_clear(land, a, b) {
            return false;
        }
        if margin_nm <= 0.0 {
            return true;
        }
        let delta = b - a;
        let dist = delta.length();
        if dist <= 0.0 {
            // Degenerate segment: probe a small disc around the point.
            for off in [
                Position::new(margin_nm, 0.0),
                Position::new(-margin_nm, 0.0),
                Position::new(0.0, margin_nm),
                Position::new(0.0, -margin_nm),
            ] {
                if self.is_land(land, a + off) {
                    return false;
                }
            }
            return true;
        }
        let dir = delta / dist;
        let perp = Position::new(-dir.y, dir.x) * margin_nm;
        self.line_is_clear(land, a + perp, b + perp) && self.line_is_clear(land, a - perp, b - perp)
    }

    /// If the segment `a → b` crosses land, return the polygon-precise
    /// hit point closest to `a`. Returns `None` if the segment is
    /// entirely clear (i.e. `line_is_clear(a, b) == true`).
    pub fn first_land_hit(&self, land: &LandMap, a: Position, b: Position) -> Option<Position> {
        if !raster_corridor_touches_land(land, a, b) {
            return None;
        }
        if !self.has_polylines {
            return raster_first_land_hit(land, a, b);
        }
        let mut best: Option<(f32, Position)> = None;
        self.coast.for_each_bucket_along(a, b, |edges| {
            for e in edges {
                if let Some((t, p)) = segment_intersection_point(a, b, e.a, e.b) {
                    if best.is_none_or(|(bt, _)| t < bt) {
                        best = Some((t, p));
                    }
                }
            }
            true
        });
        best.map(|(_, p)| p)
    }

    /// Polygon-truth replacement for `LandMap::farthest_clear_point`:
    /// the farthest point along `a → b` that stays in water. If the
    /// segment is fully clear, returns `b`. Otherwise returns the
    /// first land hit, pulled back by `PULLBACK_NM` along the
    /// segment direction so the result sits a few metres clear of the
    /// polygon edge (avoids floating-point edge-grazing on the next
    /// tick).
    pub fn farthest_clear_point(&self, land: &LandMap, a: Position, b: Position) -> Position {
        const PULLBACK_NM: f32 = 0.02;
        let Some(hit) = self.first_land_hit(land, a, b) else {
            return b;
        };
        let delta = b - a;
        let len = delta.length();
        if len <= 1e-6 {
            return a;
        }
        let along = delta * (1.0 / len);
        let hit_t = (hit - a).length();
        let t = (hit_t - PULLBACK_NM).max(0.0);
        a + along * t
    }

    // Test accessors.
    #[doc(hidden)]
    pub fn coast_edge_count(&self) -> usize {
        self.coast.buckets.iter().map(|b| b.len()).sum()
    }
    #[doc(hidden)]
    pub fn land_triangle_count(&self) -> usize {
        self.polys.buckets.iter().map(|b| b.len()).sum()
    }
    #[doc(hidden)]
    pub fn has_polylines(&self) -> bool {
        self.has_polylines
    }
    #[doc(hidden)]
    pub fn has_triangles(&self) -> bool {
        self.has_triangles
    }
}

/// Free-function version of the raster pre-filter (was a method on
/// the borrowed-LandMap variant). Walks the segment at half-cell
/// intervals and returns `true` the moment a land cell is touched.
fn raster_corridor_touches_land(land: &LandMap, a: Position, b: Position) -> bool {
    let delta = b - a;
    let dist = delta.length();
    if dist <= 0.0 {
        return land.is_land(a);
    }
    let step = (land.cell_size_nm * 0.5).max(0.1);
    let steps = (dist / step).ceil() as u32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let p = a + delta * t;
        if land.is_land(p) {
            return true;
        }
    }
    false
}

// ──────────────────────── CoastlineEdgeIndex ────────────────────────

impl CoastlineEdgeIndex {
    fn build(land: &LandMap, coastline: &CoastlineMap) -> Self {
        // Bucket grid aligned with LandMap origin so position → bucket
        // is a simple subtraction + divide. We round up the dims to
        // cover the full LandMap extent.
        let origin = land.origin;
        let extent_w = land.width as f32 * land.cell_size_nm;
        let extent_h = land.height as f32 * land.cell_size_nm;
        let cols = (extent_w / BUCKET_NM).ceil() as u32;
        let rows = (extent_h / BUCKET_NM).ceil() as u32;
        let mut buckets: Vec<Vec<CoastEdge>> = vec![Vec::new(); (cols * rows) as usize];

        for line in &coastline.lines {
            for window in line.windows(2) {
                let a = window[0];
                let b = window[1];
                let edge = CoastEdge { a, b };
                // Stamp the edge into every bucket whose bbox the
                // edge's bbox overlaps. A single edge is at most a
                // few buckets so the overhead is bounded.
                let (c0, r0, c1, r1) = bbox_buckets(a, b, origin, cols, rows);
                for r in r0..=r1 {
                    for c in c0..=c1 {
                        buckets[(r * cols + c) as usize].push(edge);
                    }
                }
            }
        }

        Self {
            origin,
            cols,
            rows,
            buckets,
        }
    }

    /// Visit every bucket the segment `a → b` passes through, in order
    /// of increasing `t` along the segment. The closure receives the
    /// bucket's edge slice; return `false` to stop iteration early
    /// (e.g. once a hit is found). Buckets are visited at most once.
    fn for_each_bucket_along(
        &self,
        a: Position,
        b: Position,
        mut visit: impl FnMut(&[CoastEdge]) -> bool,
    ) {
        // Simple approach: enumerate the bucket bbox of the segment
        // (small for short segments, modest for long ones) rather than
        // a true DDA traversal. The polygon-vs-segment test inside
        // each bucket is cheap and we de-dup buckets implicitly because
        // each (col, row) is visited once.
        let (c0, r0, c1, r1) = bbox_buckets(a, b, self.origin, self.cols, self.rows);
        for r in r0..=r1 {
            for c in c0..=c1 {
                let idx = (r * self.cols + c) as usize;
                if !visit(&self.buckets[idx]) {
                    return;
                }
            }
        }
    }
}

// ──────────────────────── LandTriangleIndex ────────────────────────

impl LandTriangleIndex {
    fn build(land: &LandMap, mesh: &LandMesh) -> Self {
        let origin = land.origin;
        let extent_w = land.width as f32 * land.cell_size_nm;
        let extent_h = land.height as f32 * land.cell_size_nm;
        let cols = (extent_w / BUCKET_NM).ceil() as u32;
        let rows = (extent_h / BUCKET_NM).ceil() as u32;
        let mut buckets: Vec<Vec<LandTriangle>> = vec![Vec::new(); (cols * rows) as usize];

        let mut i = 0;
        while i + 2 < mesh.indices.len() {
            let v0 = mesh.vertices[mesh.indices[i] as usize];
            let v1 = mesh.vertices[mesh.indices[i + 1] as usize];
            let v2 = mesh.vertices[mesh.indices[i + 2] as usize];
            let tri = LandTriangle { v0, v1, v2 };
            // Stamp into the buckets covered by the triangle's bbox.
            let (c0, r0, c1, r1) = tri_bbox_buckets(v0, v1, v2, origin, cols, rows);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    buckets[(r * cols + c) as usize].push(tri);
                }
            }
            i += 3;
        }

        Self {
            origin,
            cols,
            rows,
            buckets,
        }
    }

    /// True iff `pos` lies inside any land triangle in its bucket.
    fn contains(&self, pos: Position) -> bool {
        let (col, row) = match pos_to_bucket_in_bounds(pos, self.origin, self.cols, self.rows) {
            Some(rc) => rc,
            None => return false,
        };
        let idx = (row * self.cols + col) as usize;
        for tri in &self.buckets[idx] {
            if point_in_triangle(pos, tri.v0, tri.v1, tri.v2) {
                return true;
            }
        }
        false
    }
}

// ──────────────────────── helpers ────────────────────────

/// Convert a world position to its `(col, row)` bucket key. Returns
/// `None` if the position falls outside the indexed bounds (in which
/// case land queries default to whatever the caller treats as the
/// out-of-bounds verdict).
fn pos_to_bucket_in_bounds(
    pos: Position,
    origin: Position,
    cols: u32,
    rows: u32,
) -> Option<(u32, u32)> {
    let dx = pos.x - origin.x;
    let dy = origin.y - pos.y;
    if dx < 0.0 || dy < 0.0 {
        return None;
    }
    let col = (dx / BUCKET_NM) as u32;
    let row = (dy / BUCKET_NM) as u32;
    if col >= cols || row >= rows {
        return None;
    }
    Some((col, row))
}

/// Bounding-box bucket range for a segment, clamped to grid bounds.
fn bbox_buckets(
    a: Position,
    b: Position,
    origin: Position,
    cols: u32,
    rows: u32,
) -> (u32, u32, u32, u32) {
    let min_x = a.x.min(b.x);
    let max_x = a.x.max(b.x);
    let min_y = a.y.min(b.y);
    let max_y = a.y.max(b.y);
    let c0 = (((min_x - origin.x) / BUCKET_NM).floor() as i32).clamp(0, cols as i32 - 1) as u32;
    let c1 = (((max_x - origin.x) / BUCKET_NM).floor() as i32).clamp(0, cols as i32 - 1) as u32;
    // Y is flipped: large world-Y = small row.
    let r0 = (((origin.y - max_y) / BUCKET_NM).floor() as i32).clamp(0, rows as i32 - 1) as u32;
    let r1 = (((origin.y - min_y) / BUCKET_NM).floor() as i32).clamp(0, rows as i32 - 1) as u32;
    (c0, r0, c1, r1)
}

/// Bounding-box bucket range for a triangle.
fn tri_bbox_buckets(
    v0: Position,
    v1: Position,
    v2: Position,
    origin: Position,
    cols: u32,
    rows: u32,
) -> (u32, u32, u32, u32) {
    let min_x = v0.x.min(v1.x).min(v2.x);
    let max_x = v0.x.max(v1.x).max(v2.x);
    let min_y = v0.y.min(v1.y).min(v2.y);
    let max_y = v0.y.max(v1.y).max(v2.y);
    let c0 = (((min_x - origin.x) / BUCKET_NM).floor() as i32).clamp(0, cols as i32 - 1) as u32;
    let c1 = (((max_x - origin.x) / BUCKET_NM).floor() as i32).clamp(0, cols as i32 - 1) as u32;
    let r0 = (((origin.y - max_y) / BUCKET_NM).floor() as i32).clamp(0, rows as i32 - 1) as u32;
    let r1 = (((origin.y - min_y) / BUCKET_NM).floor() as i32).clamp(0, rows as i32 - 1) as u32;
    (c0, r0, c1, r1)
}

/// True iff segments `(p1, p2)` and `(p3, p4)` cross properly. Uses
/// the standard CCW orientation test. Collinear-overlap is not
/// counted as a crossing — degenerate cases that arise from polylines
/// sharing endpoints with their neighbors are handled by the strict
/// inequality below.
fn segments_intersect(p1: Position, p2: Position, p3: Position, p4: Position) -> bool {
    let d1 = orient(p3, p4, p1);
    let d2 = orient(p3, p4, p2);
    let d3 = orient(p1, p2, p3);
    let d4 = orient(p1, p2, p4);
    (d1 > 0.0 && d2 < 0.0 || d1 < 0.0 && d2 > 0.0) && (d3 > 0.0 && d4 < 0.0 || d3 < 0.0 && d4 > 0.0)
}

/// CCW orientation: positive ⇒ `c` is left of `a→b`, negative ⇒ right,
/// zero ⇒ collinear.
fn orient(a: Position, b: Position, c: Position) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// If segments `(a, b)` and `(c, d)` cross, return `(t, point)` where
/// `t` ∈ (0, 1) parametrizes the intersection along `a → b`. Returns
/// `None` for non-crossing or collinear cases.
fn segment_intersection_point(
    a: Position,
    b: Position,
    c: Position,
    d: Position,
) -> Option<(f32, Position)> {
    let r = b - a;
    let s = d - c;
    let denom = r.x * s.y - r.y * s.x;
    if denom.abs() < 1e-9 {
        return None;
    }
    let qp = c - a;
    let t = (qp.x * s.y - qp.y * s.x) / denom;
    let u = (qp.x * r.y - qp.y * r.x) / denom;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some((t, a + r * t))
    } else {
        None
    }
}

/// Point-in-triangle via three CCW orientation signs. Robust against
/// CW vs CCW triangle winding because we accept either "all positive"
/// or "all negative" — a point is inside iff it is on the same side
/// of every edge.
fn point_in_triangle(p: Position, a: Position, b: Position, c: Position) -> bool {
    let d1 = orient(a, b, p);
    let d2 = orient(b, c, p);
    let d3 = orient(c, a, p);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

/// Raster-only first-hit walk; used when polygons are unavailable.
fn raster_first_land_hit(land: &LandMap, a: Position, b: Position) -> Option<Position> {
    let delta = b - a;
    let dist = delta.length();
    if dist <= 0.0 {
        return if land.is_land(a) { Some(a) } else { None };
    }
    let step = (land.cell_size_nm * 0.5).max(0.1);
    let steps = (dist / step).ceil() as u32;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let p = a + delta * t;
        if land.is_land(p) {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_sea_land() -> LandMap {
        // 20×20 grid, 1 NM/cell. Bottom-right 5×5 block is land.
        let w = 20u32;
        let h = 20u32;
        let mut data = vec![0u8; (w * h) as usize];
        for r in 15..20 {
            for c in 15..20 {
                data[(r * w + c) as usize] = 255;
            }
        }
        LandMap::from_raw(data, w, h, Position::new(0.0, h as f32 * 1.0), 1.0)
    }

    #[test]
    fn raster_only_mode_matches_landmap() {
        // No polygons → degrade to raster verdict.
        let land = tiny_sea_land();
        let geom = CoastlineGeom::build(&land, &CoastlineMap::default(), &LandMesh::default());
        assert!(!geom.has_triangles());

        // Sea point.
        let sea = Position::new(2.0, 18.0);
        assert!(!geom.is_land(&land, sea));
        // Land point (in the 15..20, 15..20 block; world y of row 15
        // is origin.y - 15.5 = 20 - 15.5 = 4.5).
        let land_pt = Position::new(17.5, 4.5);
        assert!(geom.is_land(&land, land_pt));
    }

    #[test]
    fn raster_clear_segment_short_circuits() {
        let land = tiny_sea_land();
        let geom = CoastlineGeom::build(&land, &CoastlineMap::default(), &LandMesh::default());
        let a = Position::new(2.0, 18.0);
        let b = Position::new(8.0, 18.0);
        assert!(geom.line_is_clear(&land, a, b));
        assert!(geom.first_land_hit(&land, a, b).is_none());
    }

    #[test]
    fn raster_blocked_segment_reports_blocked() {
        let land = tiny_sea_land();
        let geom = CoastlineGeom::build(&land, &CoastlineMap::default(), &LandMesh::default());
        // Segment from sea into the land block.
        let a = Position::new(2.0, 4.5);
        let b = Position::new(18.0, 4.5);
        assert!(!geom.line_is_clear(&land, a, b));
        let hit = geom.first_land_hit(&land, a, b).expect("hit");
        // First land cell is x≈15; hit should be near there.
        assert!(hit.x > 12.0 && hit.x < 18.0, "hit.x = {}", hit.x);
    }

    #[test]
    fn polygon_refines_misclassified_land() {
        // 5×5 raster, single cell at (2,2) marked land (1 NM/cell, so
        // a 1×1 NM land patch centered at world (2.5, 2.5)).
        let w = 5u32;
        let h = 5u32;
        let mut data = vec![0u8; (w * h) as usize];
        data[(2 * w + 2) as usize] = 255;
        let land = LandMap::from_raw(data, w, h, Position::new(0.0, 5.0), 1.0);

        // A polygon that says "actually this cell is sea" — empty
        // triangle list ⇒ point-in-polygon is always false, so the
        // bilevel `is_land` will report sea even though the raster
        // says land. This mirrors how the production polygon mesh
        // refines staircase-misclassified cells.
        let coastline = CoastlineMap::default();
        let mesh = LandMesh {
            vertices: vec![
                // A degenerate "land" triangle FAR from our test point.
                Position::new(100.0, 100.0),
                Position::new(101.0, 100.0),
                Position::new(100.0, 101.0),
            ],
            indices: vec![0, 1, 2],
        };
        let geom = CoastlineGeom::build(&land, &coastline, &mesh);
        assert!(geom.has_triangles());

        let raster_says_land = Position::new(2.5, 2.5); // y=2.5 → row 2
        assert!(land.is_land(raster_says_land));
        // Polygon contains no triangle covering this point → bilevel
        // is_land says SEA, refining the raster verdict.
        assert!(!geom.is_land(&land, raster_says_land));
    }
}
