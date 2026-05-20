//! A* navigation on the land-mask grid.
//!
//! The planner is a fairly standard 8-connected A* with two modifications:
//!
//! 1. **Time-based edge cost**: instead of pure distance, each edge is weighted
//!    by `distance / effective_speed` for the average wind sampled at the edge
//!    midpoint. This makes the planner prefer downwind segments when both are
//!    feasible (e.g., it will swing slightly north when the trade winds make a
//!    westward leg much cheaper than a straight-line beat).
//!
//! 2. **Clearance**: search treats only cells whose `CLEARANCE_CELLS`-radius
//!    neighborhood is sea as safe. The start and goal cells themselves bypass
//!    this so ships can launch from / arrive at coastal points.
//!
//! After the grid path is recovered we run a corridor-aware line-of-sight
//! smoother to drop superfluous waypoints while preserving clearance from
//! coastlines.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::harbor::Harbor;
use crate::map::land::{CoarseLand, LandMap};
use crate::ship::{speed_at_heading, ShipStats};
use crate::types::{Position, WindVector};
use crate::weather::wind::WindGrid;

/// Bundle of references the planner needs.
pub struct PathfindContext<'a> {
    pub land: &'a LandMap,
    pub wind: &'a WindGrid,
    pub stats: &'a ShipStats,
    pub month: u8,
}

impl<'a> PathfindContext<'a> {
    pub fn new(land: &'a LandMap, wind: &'a WindGrid, stats: &'a ShipStats, month: u8) -> Self {
        Self { land, wind, stats, month }
    }
}

/// Maximum number of nodes the planner will expand before giving up. The
/// search runs on the coarse (5 NM/cell) grid, where a typical Caribbean
/// route is well under 10k expansions even with a weighted heuristic.
const MAX_EXPANSIONS: usize = 500_000;

/// Maximum BFS radius (in coarse cells) when snapping a land cell to the
/// nearest sea. 16 coarse cells = 80 NM, ample for any port we plausibly
/// start inside of.
const SNAP_RADIUS: u32 = 16;

/// Required clearance (in coarse cells) around a node for it to be
/// considered safe. Zero is correct here: coarse cells are conservatively
/// "land" if any of their 25 fine source cells is land, so a sea coarse
/// cell already implies all 25 fine source cells are clear sea — i.e., the
/// cell is at least one coarse-cell (5 NM) away from any actual coastline
/// in fine grid terms. Adding extra coarse clearance over-constrains tight
/// bays (e.g., Petit-Goâve in Bay of Gonâve) where every cell is within
/// one coarse-cell of land.
const CLEARANCE_CELLS: u32 = 0;

/// Smoother corridor margin (NM): smoothed segments must keep this much
/// distance from any land along their length. Kept modest because at 1 NM/cell
/// the planner already routes through narrow channels and we don't want the
/// smoother to invalidate too many of the planner's waypoints.
const SMOOTH_MARGIN_NM: f32 = 2.0;

/// Heuristic weight for weighted A*. Values >1 make the search greedier:
/// dramatically faster, paths a few percent longer than optimal. At 1 NM/cell
/// a strict (weight=1.0) search blows the expansion budget on long open-water
/// routes. Lower toward 1.0 if path quality starts to matter more than speed.
const HEURISTIC_WEIGHT: f32 = 2.0;

/// Find a navigable path of waypoints from `start` to a goal *point*. The
/// returned list does NOT include `start` but ends with the snapped sea-cell
/// anchor near `goal`. Returns `None` if no path can be found.
///
/// This single-point variant is mostly for tests, the diagnostic example,
/// and emergency cases where there's no harbor zone (e.g., open-water
/// rendezvous). Production code should use [`find_path_to_harbor`] so that
/// the planner can terminate at any cell in the destination's harbor zone.
pub fn find_path(
    ctx: &PathfindContext<'_>,
    start: Position,
    goal: Position,
) -> Option<Vec<Position>> {
    let coarse = &ctx.land.coarse;

    let raw_start = coarse.pos_to_cell(start)?;
    let raw_goal = coarse.pos_to_cell(goal)?;
    let start_cell = coarse.nearest_sea_cell(raw_start.0, raw_start.1, SNAP_RADIUS)?;
    let goal_cell = coarse.nearest_sea_cell(raw_goal.0, raw_goal.1, SNAP_RADIUS)?;
    let goal_anchor = coarse.cell_to_pos(goal_cell.0, goal_cell.1);

    if start_cell == goal_cell {
        return Some(vec![goal_anchor]);
    }

    if ctx.land.corridor_is_clear(start, goal_anchor, SMOOTH_MARGIN_NM) {
        return Some(vec![goal_anchor]);
    }

    let mut goal_set = HashSet::with_capacity(1);
    goal_set.insert(goal_cell);
    let cells = a_star(ctx, start_cell, &goal_set, goal_cell)?;

    finalize_path(ctx.land, coarse, start, &cells, goal_anchor)
}

/// Plan a path from `start` to any cell of `harbor`'s zone. The returned
/// waypoint list ends with whichever harbor cell A* terminated on (smoothed
/// back to fewer points). Returns `None` if no path exists or if start and
/// the harbor are in disconnected sea components.
pub fn find_path_to_harbor(
    ctx: &PathfindContext<'_>,
    start: Position,
    harbor: &Harbor,
) -> Option<Vec<Position>> {
    let land = ctx.land;
    let coarse = &land.coarse;

    if harbor.cells.is_empty() {
        return None;
    }

    // If we're already inside the harbor zone, no movement needed.
    if harbor.contains_pos(land, start) {
        return Some(vec![start]);
    }

    // Fast path: clear corridor straight to the harbor anchor.
    if land.corridor_is_clear(start, harbor.anchor, SMOOTH_MARGIN_NM) {
        return Some(vec![harbor.anchor]);
    }

    let raw_start = coarse.pos_to_cell(start)?;
    let start_cell = coarse.nearest_sea_cell(raw_start.0, raw_start.1, SNAP_RADIUS)?;

    // Lift the harbor zone (fine cells) into the coarse grid so A* can
    // terminate at any coarse cell that contains a harbor fine cell.
    let stride = coarse.stride;
    let mut goal_set: HashSet<(u32, u32)> = HashSet::with_capacity(harbor.cells.len() / 16 + 1);
    for &(c, r) in &harbor.cells {
        goal_set.insert((c / stride, r / stride));
    }
    if goal_set.is_empty() {
        return None;
    }

    // Heuristic target: the coarse cell of the harbor anchor.
    let anchor_coarse = coarse.pos_to_cell(harbor.anchor)?;

    if start_cell == anchor_coarse || goal_set.contains(&start_cell) {
        return Some(vec![harbor.anchor]);
    }

    let cells = a_star(ctx, start_cell, &goal_set, anchor_coarse)?;

    // Use the harbor anchor as the terminal world position so the ship has
    // a definite point to aim at inside the zone.
    finalize_path(land, coarse, start, &cells, harbor.anchor)
}

/// Common path-finalization: coarse cells → world positions → smoothed waypoints.
/// Smoothing uses the FINE land grid so corridor checks are accurate.
fn finalize_path(
    land: &LandMap,
    coarse: &CoarseLand,
    start: Position,
    cells: &[(u32, u32)],
    terminal: Position,
) -> Option<Vec<Position>> {
    let mut points: Vec<Position> = Vec::with_capacity(cells.len() + 1);
    points.push(start);
    for &(c, r) in cells {
        points.push(coarse.cell_to_pos(c, r));
    }
    if let Some(last) = points.last_mut() {
        *last = terminal;
    } else {
        points.push(terminal);
    }

    let smoothed = smooth_path(land, &points);
    Some(smoothed.into_iter().skip(1).collect())
}

/// 8-connected neighbor offsets.
const NEIGHBORS: [(i32, i32); 8] = [
    (1, 0), (-1, 0), (0, 1), (0, -1),
    (1, 1), (1, -1), (-1, 1), (-1, -1),
];

#[derive(Copy, Clone, PartialEq)]
struct FNode {
    f: f32,
    cell: (u32, u32),
}

impl Eq for FNode {}

impl Ord for FNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // min-heap on f: reverse compare. NaN-safe via total_cmp.
        other.f.total_cmp(&self.f)
    }
}

impl PartialOrd for FNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn a_star(
    ctx: &PathfindContext<'_>,
    start: (u32, u32),
    goal_cells: &HashSet<(u32, u32)>,
    heuristic_target: (u32, u32),
) -> Option<Vec<(u32, u32)>> {
    let coarse = &ctx.land.coarse;
    let goal_pos = coarse.cell_to_pos(heuristic_target.0, heuristic_target.1);

    // Start-window: a small disk around the start cell bypasses the
    // clearance requirement so the ship can leave a coastal port. Goal-side
    // exemptions are handled by the harbor zone itself.
    let start_window: i32 = (CLEARANCE_CELLS as i32) + 1;

    let mut open: BinaryHeap<FNode> = BinaryHeap::new();
    let mut g_score: HashMap<(u32, u32), f32> = HashMap::new();
    let mut came_from: HashMap<(u32, u32), (u32, u32)> = HashMap::new();

    g_score.insert(start, 0.0);
    open.push(FNode { f: heuristic(ctx, coarse.cell_to_pos(start.0, start.1), goal_pos), cell: start });

    let mut expansions = 0usize;
    while let Some(FNode { cell: current, .. }) = open.pop() {
        if goal_cells.contains(&current) {
            return Some(reconstruct(came_from, current));
        }
        expansions += 1;
        if expansions > MAX_EXPANSIONS {
            return None;
        }

        let current_pos = coarse.cell_to_pos(current.0, current.1);
        let current_g = *g_score.get(&current).unwrap_or(&f32::INFINITY);

        for &(dc, dr) in &NEIGHBORS {
            let nc = current.0 as i32 + dc;
            let nr = current.1 as i32 + dr;
            if nc < 0 || nr < 0 || nc >= coarse.width as i32 || nr >= coarse.height as i32 {
                continue;
            }
            let neighbor = (nc as u32, nr as u32);

            let near_endpoint = cell_chebyshev(neighbor, start) <= start_window
                || goal_cells.contains(&neighbor);
            let passable = if near_endpoint {
                coarse.is_sea_cell(neighbor.0, neighbor.1)
            } else {
                coarse.has_cell_clearance(neighbor.0, neighbor.1, CLEARANCE_CELLS)
            };
            if !passable {
                continue;
            }

            // Disallow diagonal squeezes between two land cells.
            if dc != 0 && dr != 0 {
                let side_a = (current.0 as i32 + dc, current.1 as i32);
                let side_b = (current.0 as i32, current.1 as i32 + dr);
                if !coarse.is_sea_cell(side_a.0 as u32, side_a.1 as u32)
                    || !coarse.is_sea_cell(side_b.0 as u32, side_b.1 as u32)
                {
                    continue;
                }
            }

            let neighbor_pos = coarse.cell_to_pos(neighbor.0, neighbor.1);
            let edge = edge_cost(ctx, current_pos, neighbor_pos);
            let tentative = current_g + edge;

            let prev = g_score.get(&neighbor).copied().unwrap_or(f32::INFINITY);
            if tentative < prev {
                came_from.insert(neighbor, current);
                g_score.insert(neighbor, tentative);
                let f = tentative + heuristic(ctx, neighbor_pos, goal_pos);
                open.push(FNode { f, cell: neighbor });
            }
        }
    }

    None
}

fn reconstruct(came_from: HashMap<(u32, u32), (u32, u32)>, end: (u32, u32)) -> Vec<(u32, u32)> {
    let mut path = vec![end];
    let mut current = end;
    while let Some(&prev) = came_from.get(&current) {
        path.push(prev);
        current = prev;
    }
    path.reverse();
    path
}

/// Edge cost in hours: distance divided by the effective speed of a ship
/// sailing the bearing from `a` to `b` under wind sampled at the midpoint.
fn edge_cost(ctx: &PathfindContext<'_>, a: Position, b: Position) -> f32 {
    let delta = b - a;
    let dist = delta.length();
    if dist <= 0.0 {
        return 0.0;
    }
    let heading = delta.x.atan2(delta.y).to_degrees();
    let mid = a + delta * 0.5;
    let wind = sample_wind(ctx, mid);
    let speed = speed_at_heading(heading, ctx.stats, &wind).max(0.5);
    dist / speed
}

/// Weighted heuristic for greedy A*. With `HEURISTIC_WEIGHT > 1.0` paths are
/// no longer guaranteed optimal but search time drops by orders of magnitude
/// on a 1 NM/cell grid.
fn heuristic(ctx: &PathfindContext<'_>, a: Position, goal: Position) -> f32 {
    HEURISTIC_WEIGHT * a.distance(goal) / ctx.stats.speed_max.max(0.5)
}

fn sample_wind(ctx: &PathfindContext<'_>, pos: Position) -> WindVector {
    ctx.wind.wind_at(pos, ctx.month)
}

/// Chebyshev (king-move) distance between two grid cells.
fn cell_chebyshev(a: (u32, u32), b: (u32, u32)) -> i32 {
    let dx = (a.0 as i32 - b.0 as i32).abs();
    let dy = (a.1 as i32 - b.1 as i32).abs();
    dx.max(dy)
}

/// Corridor-aware line-of-sight smoothing: keep a waypoint only if removing
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

    fn flat_wind_grid(width: u32, height: u32, origin: Position, cell: f32) -> WindGrid {
        // Build a constant 10kt easterly (u=10, v=0) over 12 months.
        let n = (width * height) as usize;
        let u_data = vec![10.0_f32; n * 12];
        let v_data = vec![0.0_f32; n * 12];
        WindGrid::from_raw(u_data, v_data, width, height, origin, cell, 12)
    }

    fn open_sea_land(width: u32, height: u32) -> LandMap {
        let data = vec![0u8; (width * height) as usize];
        LandMap::from_raw(data, width, height, Position::new(0.0, height as f32 * 10.0), 10.0)
    }

    #[test]
    fn open_sea_returns_direct_path() {
        let land = open_sea_land(20, 20);
        let wind = flat_wind_grid(20, 20, land.origin, land.cell_size_nm);
        let stats = ShipStats::sloop();
        let ctx = PathfindContext::new(&land, &wind, &stats, 0);

        let start = Position::new(20.0, 20.0);
        let goal = Position::new(180.0, 180.0);
        let path = find_path(&ctx, start, goal).expect("path");
        // Final waypoint is the snapped sea-cell anchor near the goal.
        let last = path.last().copied().unwrap();
        assert!(last.distance(goal) < land.coarse.cell_size_nm * 1.5);
        // Line of sight is clear: should be a single waypoint (the goal anchor).
        assert_eq!(path.len(), 1);
    }

    #[test]
    fn path_avoids_land_obstacle() {
        // Build a 20x20 grid with a vertical wall of land in column 10
        // (except a narrow gap so a route exists).
        let w = 20u32;
        let h = 20u32;
        let mut data = vec![0u8; (w * h) as usize];
        for r in 0..h {
            if r == 5 || r == 6 {
                continue; // gap so a path exists with clearance
            }
            data[(r * w + 10) as usize] = 255;
        }
        let land = LandMap::from_raw(data, w, h, Position::new(0.0, h as f32 * 10.0), 10.0);
        let wind = flat_wind_grid(w, h, land.origin, land.cell_size_nm);
        let stats = ShipStats::sloop();
        let ctx = PathfindContext::new(&land, &wind, &stats, 0);

        let start = Position::new(20.0, land.origin.y - 55.0); // west side
        let goal = Position::new(180.0, land.origin.y - 55.0); // east side
        let path = find_path(&ctx, start, goal).expect("path around wall");

        // Path should not cross any land segment.
        let mut prev = start;
        for p in &path {
            assert!(land.line_is_clear(prev, *p), "segment crosses land");
            prev = *p;
        }
        // Final waypoint is the snapped sea-cell anchor near the goal.
        // (Coarse-grid planning can snap up to ~stride cells away.)
        let last = *path.last().unwrap();
        assert!(last.distance(goal) < land.coarse.cell_size_nm * 1.5);
    }
}
