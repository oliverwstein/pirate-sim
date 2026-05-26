//! Navmesh-based ship path planning.
//!
//! **Phase C migration in progress.** Production callers now use the
//! [`TileMesh`] portal-aware convex-tile mesh (loaded from
//! `data/grids/navmesh.bin`) plus a Simple Stupid Funnel pass through
//! the corridor of shared edges. The legacy raster-derived [`Navmesh`]
//! is still loaded and retained on [`PathfindContext`] for tests and
//! as a backstop, but [`find_path`] and [`find_path_to_harbor`] no
//! longer consult it when a tile mesh is attached.
//!
//! Pipeline:
//!
//! 1. Trivial line-of-sight check from start to goal — if the raster
//!    corridor is clear, return a single waypoint. (Phase E replaces
//!    raster LOS with polygon-truth `coastline_geom`.)
//! 2. Locate nearest tile-mesh centroids around start and goal.
//! 3. A* on tile centroids → tile-id sequence.
//! 4. Reconstruct `(left, right)` shared-edge endpoints between
//!    consecutive tiles via [`TileMesh::shared_edge`].
//! 5. Run SSFA over the (left, right) chain → smoothed waypoint
//!    polyline.
//! 6. Validate each output segment with raster corridor LOS; on any
//!    failure, fall back to the unsmoothed centroid sequence
//!    (guaranteed safe because each centroid lies inside a convex
//!    tile).

use std::collections::HashSet;

use crate::coastline_geom::CoastlineGeom;
use crate::harbor::Harbor;
use crate::map::land::LandMap;
use crate::navmesh::Navmesh;
use crate::portroutes::PortRouteCache;
use crate::ship::ShipStats;
use crate::tile_mesh::{self, TileMesh};
use crate::types::Position;
use crate::weather::wind::WindGrid;

/// Bundle of references the planner needs.
pub struct PathfindContext<'a> {
    pub land: &'a LandMap,
    pub wind: &'a WindGrid,
    pub stats: &'a ShipStats,
    pub month: u8,
    pub navmesh: &'a Navmesh,
    /// Phase C: portal-aware convex-tile mesh. When `Some`, the planner
    /// uses it; when `None` (tests that haven't yet been migrated) it
    /// falls back to the legacy [`Navmesh`] path.
    pub tile_mesh: Option<&'a TileMesh>,
    /// Optional per-port SSSP cache. When present, `find_path_to_harbor`
    /// uses it instead of running A* every call. Tests and small
    /// fixtures may pass `None` to skip the precomputation step.
    pub port_routes: Option<&'a PortRouteCache>,
    /// Phase E: polygon-truth coastline geometry. Used downstream by
    /// the AI to construct a `NavTerrain` for `compute_steering`'s
    /// reactive deflection. The planner itself doesn't currently
    /// consult it (the raster `LandMap` is still the corridor oracle
    /// inside `tile_mesh_path`), but plumbing it through here keeps
    /// the AI's terrain bundle in one place.
    pub coastline_geom: Option<&'a CoastlineGeom>,
}

impl<'a> PathfindContext<'a> {
    pub fn new(
        land: &'a LandMap,
        wind: &'a WindGrid,
        stats: &'a ShipStats,
        month: u8,
        navmesh: &'a Navmesh,
    ) -> Self {
        Self {
            land,
            wind,
            stats,
            month,
            navmesh,
            tile_mesh: None,
            port_routes: None,
            coastline_geom: None,
        }
    }

    /// Attach a [`PortRouteCache`] to enable cached harbor pathing.
    pub fn with_port_routes(mut self, cache: &'a PortRouteCache) -> Self {
        self.port_routes = Some(cache);
        self
    }

    /// Attach a [`TileMesh`] to enable Phase C tile-based planning.
    pub fn with_tile_mesh(mut self, tile_mesh: &'a TileMesh) -> Self {
        self.tile_mesh = Some(tile_mesh);
        self
    }

    /// Attach a [`CoastlineGeom`] so the AI can build a `NavTerrain`
    /// for reactive deflection (Phase E).
    pub fn with_coastline_geom(mut self, geom: &'a CoastlineGeom) -> Self {
        self.coastline_geom = Some(geom);
        self
    }
}

/// Smoother corridor margin (NM): the line-of-sight short-circuit and the
/// post-route smoother both require this much clearance from any land
/// along the segment. At 1 NM/cell this is a 2-cell buffer.
const SMOOTH_MARGIN_NM: f32 = 2.0;

/// Search radius (NM) for finding visible navmesh entry/exit nodes from a
/// non-mesh start/goal point (a harbor anchor, a ship's current position).
const ENTRY_RADIUS_NM: f32 = 200.0;

/// Cap on the number of mesh entry/exit nodes considered. The mesh has very
/// high local connectivity (avg degree 30+), so a handful of nearby nodes
/// already covers all reasonable graph entries.
const ENTRY_CANDIDATES: usize = 16;

/// Margin (NM) used when probing line-of-sight from a coastal start/anchor
/// point to a mesh node. Zero (line-of-sight only) is correct here: harbor
/// anchors are placed adjacent to coastlines so any non-zero margin would
/// reject every candidate.
const ENTRY_MARGIN_NM: f32 = 0.0;

/// Search radius (NM) for finding the tile-mesh entry tiles from a
/// non-mesh start/goal point. Smaller than the legacy `ENTRY_RADIUS_NM`
/// because tile centroids are denser (~43k tiles vs ~thousands of
/// navmesh nodes) and we only need a handful of nearby candidates.
const TILE_ENTRY_RADIUS_NM: f32 = 50.0;

/// Hard cap on tile-entry candidates the planner considers. Pure-A*
/// over the centroid graph is cheap, so this exists only to bound
/// extreme cases.
const TILE_ENTRY_MAX: usize = 8;

/// Find a navigable path of waypoints from `start` to a goal *point*. The
/// returned list does NOT include `start` but ends at `goal`. Returns
/// `None` if no path can be found.
///
/// This single-point variant is for tests and emergency cases (open-water
/// rendezvous). Production code should use [`find_path_to_harbor`] so the
/// planner can terminate at any cell in the destination's harbor zone.
pub fn find_path(
    ctx: &PathfindContext<'_>,
    start: Position,
    goal: Position,
) -> Option<Vec<Position>> {
    if ctx.land.corridor_is_clear(start, goal, SMOOTH_MARGIN_NM) {
        return Some(vec![goal]);
    }
    if let Some(mesh) = ctx.tile_mesh {
        if let Some(path) = tile_mesh_path(ctx.land, mesh, start, goal, None) {
            return Some(path);
        }
    }
    navmesh_path(ctx.land, ctx.navmesh, start, &[goal], goal)
}

/// Plan a path from `start` to any cell of `harbor`'s zone, ending at the
/// harbor anchor. Returns `None` if the navmesh has no route between any
/// node visible from `start` and any node visible from the anchor.
///
/// When `ctx.port_routes` is set (the production world configuration),
/// the per-port SSSP cache is consulted first — for any voyage to a
/// well-connected harbor, this turns A* into a constant-time lookup
/// plus a predecessor walk. Cache misses (start has no path to the
/// port's entry set) silently fall back to live A* via `navmesh_path`.
pub fn find_path_to_harbor(
    ctx: &PathfindContext<'_>,
    start: Position,
    harbor: &Harbor,
) -> Option<Vec<Position>> {
    let land = ctx.land;

    // If we're already inside the harbor zone, no movement needed.
    if harbor.contains_pos(land, start) {
        return Some(vec![start]);
    }

    // Line-of-sight to the harbor anchor.
    if land.corridor_is_clear(start, harbor.anchor, SMOOTH_MARGIN_NM) {
        return Some(vec![harbor.anchor]);
    }

    // Phase D: prefer the SSSP cache when both the tile mesh and the
    // cache are attached. Cache miss (port has no entry or no reachable
    // start) silently falls through to live A* via `tile_mesh_path`.
    if let (Some(mesh), Some(cache)) = (ctx.tile_mesh, ctx.port_routes) {
        if let Some(path) = tile_mesh_cached_path(land, mesh, cache, start, harbor) {
            return Some(path);
        }
    }

    // Phase C: live A* on the tile mesh. Seed the goal tile set with
    // the anchor tile and its immediate neighbours so the funnel has
    // room to smooth into the harbor's mouth.
    if let Some(mesh) = ctx.tile_mesh {
        if let Some(anchor_tile) = harbor.anchor_tile {
            let mut goal_tiles: Vec<u32> = Vec::with_capacity(8);
            goal_tiles.push(anchor_tile);
            for e in &mesh.neighbors[anchor_tile as usize] {
                goal_tiles.push(e.to);
            }
            if let Some(path) = tile_mesh_path(land, mesh, start, harbor.anchor, Some(&goal_tiles))
            {
                return Some(path);
            }
        }
    }

    navmesh_path(land, ctx.navmesh, start, &[harbor.anchor], harbor.anchor)
}

/// Phase D cached harbor path: look up a precomputed tile-id route from
/// the per-port SSSP cache, then stitch it into a polyline using the
/// same shared-edge → SSFA pipeline as the live planner. Returns
/// `None` on cache miss so the caller can fall back to live A*.
fn tile_mesh_cached_path(
    land: &LandMap,
    mesh: &TileMesh,
    cache: &PortRouteCache,
    start: Position,
    harbor: &Harbor,
) -> Option<Vec<Position>> {
    let start_candidates = mesh.nearest_centroids(start, TILE_ENTRY_RADIUS_NM);
    if start_candidates.is_empty() {
        return None;
    }
    let starts: Vec<u32> = start_candidates
        .iter()
        .take(TILE_ENTRY_MAX)
        .map(|&(i, _)| i)
        .collect();

    let route = cache.route_from(&starts, harbor.port_index)?;
    Some(stitch_tile_route(land, mesh, start, harbor.anchor, &route))
}

/// Plan a path through the navmesh from `start` to any of the goal anchors,
/// returning waypoint positions ending at `terminal`. The boundary legs
/// (start → first mesh node, last mesh node → terminal) bypass the mesh
/// and rely on line-of-sight checks.
fn navmesh_path(
    land: &LandMap,
    nm: &Navmesh,
    start: Position,
    goal_anchors: &[Position],
    terminal: Position,
) -> Option<Vec<Position>> {
    let starts = nm.visible_from(
        land,
        start,
        ENTRY_RADIUS_NM,
        ENTRY_CANDIDATES,
        ENTRY_MARGIN_NM,
    );
    if starts.is_empty() {
        return None;
    }

    let mut goal_seen: HashSet<u32> = HashSet::new();
    let mut goals: Vec<u32> = Vec::new();
    for &ga in goal_anchors {
        for n in nm.visible_from(land, ga, ENTRY_RADIUS_NM, ENTRY_CANDIDATES, ENTRY_MARGIN_NM) {
            if goal_seen.insert(n) {
                goals.push(n);
            }
        }
    }
    if goals.is_empty() {
        return None;
    }

    let route = nm.route(&starts, &goals)?;

    let mut points: Vec<Position> = Vec::with_capacity(route.len() + 2);
    points.push(start);
    for idx in &route {
        points.push(nm.nodes[*idx as usize].pos);
    }
    points.push(terminal);

    let smoothed = smooth_boundaries(land, &points);
    Some(smoothed.into_iter().skip(1).collect())
}

/// Plan a path through the [`TileMesh`] from `start` to `terminal`.
///
/// If `goal_tiles` is `Some`, those tiles are the goal set for A*; the
/// terminal is appended to the smoothed polyline so harbor anchors and
/// rendezvous points are hit exactly. If `goal_tiles` is `None`, the
/// nearest centroids to `terminal` are used (general point-to-point).
///
/// Pipeline: nearest-centroids → A* → shared-edge `(left, right)`
/// reconstruction → SSFA → segment-LOS validation. On any SSFA segment
/// that fails raster LOS (rare; only when the smoothed path tries to
/// cut across a sliver outside the convex tile chain), we fall back to
/// the raw centroid sequence, which is provably safe because each
/// centroid lies inside a convex tile and each consecutive pair shares
/// an edge.
fn tile_mesh_path(
    land: &LandMap,
    mesh: &TileMesh,
    start: Position,
    terminal: Position,
    goal_tiles: Option<&[u32]>,
) -> Option<Vec<Position>> {
    let start_candidates = mesh.nearest_centroids(start, TILE_ENTRY_RADIUS_NM);
    if start_candidates.is_empty() {
        return None;
    }
    let starts: Vec<u32> = start_candidates
        .iter()
        .take(TILE_ENTRY_MAX)
        .map(|&(i, _)| i)
        .collect();

    let owned_goals: Vec<u32>;
    let goals: &[u32] = if let Some(g) = goal_tiles {
        g
    } else {
        let goal_candidates = mesh.nearest_centroids(terminal, TILE_ENTRY_RADIUS_NM);
        if goal_candidates.is_empty() {
            return None;
        }
        owned_goals = goal_candidates
            .iter()
            .take(TILE_ENTRY_MAX)
            .map(|&(i, _)| i)
            .collect();
        &owned_goals
    };

    let route = mesh.route(&starts, goals)?;
    Some(stitch_tile_route(land, mesh, start, terminal, &route))
}

/// Stitch a tile-id `route` into a polyline of waypoints from `start`
/// (excluded) to `terminal` (included). Reconstructs the `(left, right)`
/// shared-edge chain between consecutive tiles, runs SSFA, and validates
/// every output segment against raster LOS. On any validation failure
/// (rare; only when the smoothed path tries to cut across a sliver
/// outside the convex tile chain), falls back to the raw centroid
/// sequence with the terminal appended — provably safe because each
/// centroid lies inside a convex tile and each consecutive pair shares
/// an edge.
///
/// Shared between the Phase C live planner ([`tile_mesh_path`]) and the
/// Phase D cached planner ([`tile_mesh_cached_path`]); both produce
/// identical waypoint streams given identical tile routes.
fn stitch_tile_route(
    land: &LandMap,
    mesh: &TileMesh,
    start: Position,
    terminal: Position,
    route: &[u32],
) -> Vec<Position> {
    let mut portals: Vec<(Position, Position)> = Vec::with_capacity(route.len().saturating_sub(1));
    for w in route.windows(2) {
        let Some((l, r)) = mesh.shared_edge(w[0], w[1]) else {
            // Should not happen on the bundled mesh, but guard anyway.
            return centroid_chain_with_terminal(mesh, route, terminal);
        };
        portals.push((l, r));
    }

    let smoothed = tile_mesh::funnel(start, terminal, &portals);

    // Validate every smoothed segment against the raster. Any failure
    // → fall back to the centroid chain (guaranteed safe).
    let mut prev = start;
    for &p in &smoothed {
        if !land.corridor_is_clear(prev, p, SMOOTH_MARGIN_NM) {
            return centroid_chain_with_terminal(mesh, route, terminal);
        }
        prev = p;
    }
    smoothed
}

/// Fallback: emit centroids in route order, then the terminal. Each
/// consecutive pair of centroids is provably inside `tile_a ∪ tile_b`
/// (both convex), so no segment can cross land. The terminal is
/// appended unconditionally; callers must ensure it's reachable
/// from the final centroid (true for harbor anchors by construction,
/// since the anchor *is* a tile centroid).
fn centroid_chain_with_terminal(
    mesh: &TileMesh,
    route: &[u32],
    terminal: Position,
) -> Vec<Position> {
    let mut out: Vec<Position> = Vec::with_capacity(route.len() + 1);
    for &t in route {
        out.push(mesh.tiles[t as usize].centroid);
    }
    if out.last() != Some(&terminal) {
        out.push(terminal);
    }
    out
}

/// Corridor-aware line-of-sight smoothing restricted to the two boundary
/// legs (`start → first mesh node` and `last mesh node → terminal`).
///
/// Interior mesh-to-mesh segments are kept verbatim. Consecutive nodes in
/// a navmesh route are valid by construction (the mesh is built so its
/// edges have line-of-sight with the mesh margin), so the only smoothing
/// of practical value is collapsing leading and trailing nodes that the
/// non-mesh endpoints can see past. This bounds LOS work to
/// `prefix_len + suffix_len` calls (each bounded by `ENTRY_RADIUS_NM`)
/// instead of `~N` calls across the full route.
fn smooth_boundaries(land: &LandMap, points: &[Position]) -> Vec<Position> {
    if points.len() <= 3 {
        return smooth_path(land, points);
    }
    let n = points.len();

    // Prefix: advance over leading interior nodes that `start` can see past.
    // `prefix_end` is the index of the first interior waypoint we keep.
    let mut prefix_end = 1usize;
    while prefix_end + 1 < n - 1
        && land.corridor_is_clear(points[0], points[prefix_end + 1], SMOOTH_MARGIN_NM)
    {
        prefix_end += 1;
    }

    // Suffix: walk `terminal` back over trailing interior nodes.
    // `suffix_start` is the index of the last interior waypoint we keep.
    let mut suffix_start = n - 2;
    while suffix_start > prefix_end
        && land.corridor_is_clear(points[suffix_start - 1], points[n - 1], SMOOTH_MARGIN_NM)
    {
        suffix_start -= 1;
    }

    let mut out = Vec::with_capacity(suffix_start - prefix_end + 3);
    out.push(points[0]);
    out.extend_from_slice(&points[prefix_end..=suffix_start]);
    out.push(points[n - 1]);
    out
}

/// Plan a path through the navmesh from `start` to any of the goal anchors,
/// it would force a path through (or too close to) land. Walks forward
/// greedily, jumping as far as the corridor with `SMOOTH_MARGIN_NM` clearance
/// allows.
fn smooth_path(land: &LandMap, points: &[Position]) -> Vec<Position> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity(points.len());
    out.push(points[0]);
    let mut anchor = 0usize;
    let mut probe = 1usize;
    while probe < points.len() {
        let next = probe + 1;
        let ok = if next < points.len() {
            land.corridor_is_clear(points[anchor], points[next], SMOOTH_MARGIN_NM)
        } else {
            false
        };
        if ok {
            probe = next;
        } else {
            out.push(points[probe]);
            anchor = probe;
            probe += 1;
        }
    }
    if *out.last().unwrap() != *points.last().unwrap() {
        out.push(*points.last().unwrap());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ship::ShipStats;

    fn flat_wind_grid(width: u32, height: u32, origin: Position, cell: f32) -> WindGrid {
        let n = (width * height) as usize;
        let u_data = vec![10.0_f32; n * 12];
        let v_data = vec![0.0_f32; n * 12];
        WindGrid::from_raw(u_data, v_data, width, height, origin, cell, 12)
    }

    fn open_sea_land(width: u32, height: u32) -> LandMap {
        let data = vec![0u8; (width * height) as usize];
        LandMap::from_raw(
            data,
            width,
            height,
            Position::new(0.0, height as f32 * 10.0),
            10.0,
        )
    }

    #[test]
    fn open_sea_returns_direct_path() {
        let land = open_sea_land(20, 20);
        let wind = flat_wind_grid(20, 20, land.origin, land.cell_size_nm);
        let nm = Navmesh::build(&land);
        let stats = ShipStats::sloop();
        let ctx = PathfindContext::new(&land, &wind, &stats, 0, &nm);

        let start = Position::new(20.0, 20.0);
        let goal = Position::new(180.0, 180.0);
        let path = find_path(&ctx, start, goal).expect("path");
        // LOS short-circuit: single waypoint at the goal.
        assert_eq!(path.len(), 1);
        assert_eq!(path[0], goal);
    }

    #[test]
    fn path_avoids_land_obstacle() {
        // 20x20 grid, vertical wall at column 10 with a 2-row gap so a
        // route exists.
        let w = 20u32;
        let h = 20u32;
        let mut data = vec![0u8; (w * h) as usize];
        for r in 0..h {
            if r == 5 || r == 6 {
                continue;
            }
            data[(r * w + 10) as usize] = 255;
        }
        let land = LandMap::from_raw(data, w, h, Position::new(0.0, h as f32 * 10.0), 10.0);
        let wind = flat_wind_grid(w, h, land.origin, land.cell_size_nm);
        let nm = Navmesh::build(&land);
        let stats = ShipStats::sloop();
        let ctx = PathfindContext::new(&land, &wind, &stats, 0, &nm);

        let start = Position::new(20.0, land.origin.y - 55.0);
        let goal = Position::new(180.0, land.origin.y - 55.0);
        let path = find_path(&ctx, start, goal).expect("path around wall");

        let mut prev = start;
        for p in &path {
            assert!(land.line_is_clear(prev, *p), "segment crosses land");
            prev = *p;
        }
        assert_eq!(*path.last().unwrap(), goal);
    }
}
