//! Tile-mesh ship path planning.
//!
//! Production path planning runs on the [`TileMesh`] portal-aware
//! convex-tile mesh (loaded from `data/grids/navmesh.bin`) plus a
//! Simple Stupid Funnel pass through the corridor of shared edges.
//!
//! Pipeline:
//!
//! 1. Trivial line-of-sight check from start to goal — if the raster
//!    corridor is clear, return a single waypoint. (A future pass can
//!    replace raster LOS with polygon-truth `coastline_geom`.)
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

use crate::coastline_geom::CoastlineGeom;
use crate::harbor::Harbor;
use crate::map::land::LandMap;
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
    /// Portal-aware convex-tile mesh — the sole planning substrate.
    pub tile_mesh: &'a TileMesh,
    /// Polygon-truth coastline geometry — the sole LOS oracle for the
    /// planner. Also used downstream by the AI to construct a
    /// `NavTerrain` for `compute_steering`'s reactive deflection.
    pub coastline_geom: &'a CoastlineGeom,
    /// Optional per-port SSSP cache. When present, `find_path_to_harbor`
    /// uses it instead of running A* every call. Tests and small
    /// fixtures may pass `None` to skip the precomputation step.
    pub port_routes: Option<&'a PortRouteCache>,
}

impl<'a> PathfindContext<'a> {
    pub fn new(
        land: &'a LandMap,
        wind: &'a WindGrid,
        stats: &'a ShipStats,
        month: u8,
        tile_mesh: &'a TileMesh,
        coastline_geom: &'a CoastlineGeom,
    ) -> Self {
        Self {
            land,
            wind,
            stats,
            month,
            tile_mesh,
            coastline_geom,
            port_routes: None,
        }
    }

    /// Attach a [`PortRouteCache`] to enable cached harbor pathing.
    pub fn with_port_routes(mut self, cache: &'a PortRouteCache) -> Self {
        self.port_routes = Some(cache);
        self
    }
}

/// Smoother corridor margin (NM): the line-of-sight short-circuit and the
/// post-route smoother both require this much clearance from any land
/// along the segment. At 1 NM/cell this is a 2-cell buffer.
const SMOOTH_MARGIN_NM: f32 = 2.0;

/// Search radius (NM) for finding the tile-mesh entry tiles from a
/// non-mesh start/goal point. Tile centroids are dense (~43k tiles),
/// so we only need a handful of nearby candidates.
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
    if ctx
        .coastline_geom
        .corridor_is_clear(ctx.land, start, goal, SMOOTH_MARGIN_NM)
    {
        return Some(vec![goal]);
    }
    tile_mesh_path(
        ctx.land,
        ctx.coastline_geom,
        ctx.tile_mesh,
        start,
        goal,
        None,
    )
}

/// Plan a path from `start` to any cell of `harbor`'s zone, ending at the
/// harbor anchor. Returns `None` if no route exists between any tile
/// visible from `start` and any tile visible from the anchor.
///
/// When `ctx.port_routes` is set (the production world configuration),
/// the per-port SSSP cache is consulted first — for any voyage to a
/// well-connected harbor, this turns A* into a constant-time lookup
/// plus a predecessor walk. Cache misses (start has no path to the
/// port's entry set) silently fall back to live A* via `tile_mesh_path`.
pub fn find_path_to_harbor(
    ctx: &PathfindContext<'_>,
    start: Position,
    harbor: &Harbor,
) -> Option<Vec<Position>> {
    let land = ctx.land;
    let mesh = ctx.tile_mesh;
    let geom = ctx.coastline_geom;

    // If we're already inside the harbor zone, no movement needed.
    if harbor.contains_pos(land, start) {
        return Some(vec![start]);
    }

    // Line-of-sight to the harbor anchor.
    if geom.corridor_is_clear(land, start, harbor.anchor, SMOOTH_MARGIN_NM) {
        return Some(vec![harbor.anchor]);
    }

    // Prefer the SSSP cache when available. Cache miss (port has no
    // entry or no reachable start) silently falls through to live A*.
    if let Some(cache) = ctx.port_routes {
        if let Some(path) = tile_mesh_cached_path(land, geom, mesh, cache, start, harbor) {
            return Some(path);
        }
    }

    // Live A* on the tile mesh. Seed the goal tile set with the anchor
    // tile and its immediate neighbours so the funnel has room to
    // smooth into the harbor's mouth.
    let anchor_tile = harbor.anchor_tile?;
    let mut goal_tiles: Vec<u32> = Vec::with_capacity(8);
    goal_tiles.push(anchor_tile);
    for e in &mesh.neighbors[anchor_tile as usize] {
        goal_tiles.push(e.to);
    }
    tile_mesh_path(land, geom, mesh, start, harbor.anchor, Some(&goal_tiles))
}

/// Cached harbor path: look up a precomputed tile-id route from the
/// per-port SSSP cache, then stitch it into a polyline using the same
/// shared-edge → SSFA pipeline as the live planner. Returns `None` on
/// cache miss so the caller can fall back to live A*.
fn tile_mesh_cached_path(
    land: &LandMap,
    geom: &CoastlineGeom,
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
    Some(stitch_tile_route(
        land,
        geom,
        mesh,
        start,
        harbor.anchor,
        &route,
    ))
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
/// that fails polygon LOS (rare; only when the smoothed path tries to
/// cut across a sliver outside the convex tile chain), we fall back to
/// the raw centroid sequence, which is provably safe because each
/// centroid lies inside a convex tile and each consecutive pair shares
/// an edge.
fn tile_mesh_path(
    land: &LandMap,
    geom: &CoastlineGeom,
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
    Some(stitch_tile_route(land, geom, mesh, start, terminal, &route))
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
/// Shared between the live planner ([`tile_mesh_path`]) and the cached
/// planner ([`tile_mesh_cached_path`]); both produce identical waypoint
/// streams given identical tile routes.
fn stitch_tile_route(
    land: &LandMap,
    geom: &CoastlineGeom,
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

    // Validate every smoothed segment against polygon truth. Any
    // failure → fall back to the centroid chain (guaranteed safe).
    let mut prev = start;
    for &p in &smoothed {
        if !geom.corridor_is_clear(land, prev, p, SMOOTH_MARGIN_NM) {
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

    fn empty_tile_mesh() -> TileMesh {
        TileMesh::empty()
    }

    #[test]
    fn open_sea_returns_direct_path() {
        let land = open_sea_land(20, 20);
        let wind = flat_wind_grid(20, 20, land.origin, land.cell_size_nm);
        let mesh = empty_tile_mesh();
        let geom = CoastlineGeom::empty(&land);
        let stats = ShipStats::sloop();
        let ctx = PathfindContext::new(&land, &wind, &stats, 0, &mesh, &geom);

        let start = Position::new(20.0, 20.0);
        let goal = Position::new(180.0, 180.0);
        let path = find_path(&ctx, start, goal).expect("path");
        // LOS short-circuit: single waypoint at the goal.
        assert_eq!(path.len(), 1);
        assert_eq!(path[0], goal);
    }
}
