//! Programmatic navigation mesh built from the land mask.
//!
//! The navmesh replaces grid-based A* with graph-based routing. We sample
//! a few thousand waypoints at strategic positions and connect them by
//! line-of-sight on the fine land grid. Routing is then ordinary A* on a
//! small graph (sub-millisecond) instead of expanding hundreds of
//! thousands of grid cells.
//!
//! ## Waypoint placement
//!
//! Waypoints are placed in two passes, both fully programmatic:
//!
//! 1. **Open-water grid**: the fine land grid is sampled on a coarse stride
//!    (`OPEN_STRIDE_NM`); cells that are sea with `dist_to_land >= MIN_OFFSHORE_NM`
//!    become nodes. This gives a uniform "highway" of waypoints in deep
//!    water — the bulk of Atlantic / Caribbean transit.
//!
//! 2. **Channel detection**: every `CHANNEL_STRIDE × CHANNEL_STRIDE` block
//!    of fine cells is examined; if it contains both land and sea, we drop
//!    a waypoint at the deepest fine sea cell within. This auto-discovers
//!    the Bocas of Trinidad, the Maracaibo Strait, the Florida Strait, the
//!    Yucatán Channel, and similar narrow passages — exactly the places
//!    the open-water grid is too sparse for.
//!
//! ## Edges
//!
//! Each node pair within `MAX_EDGE_NM` is tested with `corridor_is_clear`
//! on the fine grid (margin `EDGE_MARGIN_NM`). If clear, an edge is added
//! weighted by Great-Circle distance. After construction we flood-fill and
//! keep only the largest connected component, guaranteeing the routing
//! graph is one piece.
//!
//! ## Routing
//!
//! Given a starting `Position` and a destination port (with its
//! pre-computed entry-node set), we:
//!   - Find candidate "entries" visible from the start (corridor-clear).
//!   - Run A* on the navmesh graph.
//!   - Return waypoint positions plus the harbor anchor at the end.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::map::land::LandMap;
use crate::types::Position;

/// Spacing of the open-water sampling grid.
const OPEN_STRIDE_NM: f32 = 25.0;

/// Minimum distance-from-land for an open-water waypoint.
const MIN_OFFSHORE_NM: u32 = 1;

/// Minimum distance-from-land for a channel-detection waypoint. Channels
/// are by definition narrower than open water, so this is more permissive.
const MIN_CHANNEL_NM: u32 = 0;

/// Coarse-cell stride (in fine cells) used by the channel-detection pass.
/// Each `CHANNEL_STRIDE × CHANNEL_STRIDE` block of fine cells is examined
/// for a mixed land/sea state; if mixed, the deepest fine sea cell in the
/// block becomes a channel waypoint.
const CHANNEL_STRIDE: u32 = 5;

/// Maximum length of any single navmesh edge. Long edges are slower to
/// validate (more corridor samples) and rarely useful — open-water sampling
/// already provides a thicket of medium-length connections.
const MAX_EDGE_NM: f32 = 80.0;

/// Half-width of the corridor used for edge clearance checks. Set to 0
/// (line-of-sight only) so the densest channels (Maracaibo strait, Hudson
/// approach, Delaware River) admit edges; ships have their own land-rescue
/// deflection so a strict corridor margin is not necessary at the graph level.
const EDGE_MARGIN_NM: f32 = 0.0;

/// A node in the navmesh.
#[derive(Debug, Clone, Copy)]
pub struct Node {
    pub pos: Position,
}

/// A directed (here: undirected, stored both ways) edge in the navmesh.
#[derive(Debug, Clone, Copy)]
pub struct Edge {
    pub to: u32,
    pub dist_nm: f32,
}

/// The navigation mesh: nodes plus an adjacency list.
pub struct Navmesh {
    pub nodes: Vec<Node>,
    pub adj: Vec<Vec<Edge>>,
    /// Spatial hash for fast nearest-node queries (key = (cell_col,cell_row)
    /// in BUCKET_NM buckets, value = node indices).
    pub buckets: HashMap<(i32, i32), Vec<u32>>,
    pub bucket_size_nm: f32,
}

const BUCKET_NM: f32 = 50.0;

impl Navmesh {
    /// Construct the navmesh from a land map.
    pub fn build(land: &LandMap) -> Self {
        let mut node_positions: Vec<Position> = Vec::new();

        // --- Pass 1: open-water sampling ---
        let stride_cells = (OPEN_STRIDE_NM / land.cell_size_nm).max(1.0).round() as u32;
        let w = land.width;
        let h = land.height;
        let mut r = 0u32;
        while r < h {
            let mut c = 0u32;
            while c < w {
                if cell_clearance_nm(land, c, r) >= MIN_OFFSHORE_NM {
                    node_positions.push(land.cell_to_pos(c, r));
                }
                c += stride_cells;
            }
            r += stride_cells;
        }
        let open_count = node_positions.len();

        // --- Pass 2: channel detection (fine-vs-coarse comparison) ---
        // For each `CHANNEL_STRIDE × CHANNEL_STRIDE` block of fine cells
        // that contains BOTH land and sea (i.e., a coastal block), drop a
        // waypoint at the deepest fine sea cell inside it. To avoid
        // duplicates we de-dup by block — at most one channel waypoint per
        // block. This auto-discovers narrow straits, river mouths and
        // similar passages that the open-water grid is too sparse for.
        let stride = CHANNEL_STRIDE;
        let cw = w.div_ceil(stride);
        let ch = h.div_ceil(stride);
        for cr in 0..ch {
            for cc in 0..cw {
                let r0 = cr * stride;
                let c0 = cc * stride;
                let r1 = (r0 + stride).min(h);
                let c1 = (c0 + stride).min(w);
                let mut has_land = false;
                let mut has_sea = false;
                'cells: for rr in r0..r1 {
                    for col in c0..c1 {
                        if land.is_sea_cell(col, rr) {
                            has_sea = true;
                        } else {
                            has_land = true;
                        }
                        if has_land && has_sea {
                            break 'cells;
                        }
                    }
                }
                // Coastal blocks only: pure-sea blocks are already covered
                // by pass 1.
                if !(has_land && has_sea) {
                    continue;
                }
                // Find the deepest (highest dist_to_land) fine sea cell.
                let mut best: Option<(u32, u32, u32)> = None;
                for rr in r0..r1 {
                    for col in c0..c1 {
                        if !land.is_sea_cell(col, rr) {
                            continue;
                        }
                        let d = cell_clearance_nm(land, col, rr);
                        // MIN_CHANNEL_NM is currently 0 (a tunable); the
                        // comparison is intentionally a no-op until a
                        // non-zero floor is set.
                        #[allow(clippy::absurd_extreme_comparisons)]
                        let too_narrow = d < MIN_CHANNEL_NM;
                        if too_narrow {
                            continue;
                        }
                        if best.is_none_or(|(_, _, bd)| d > bd) {
                            best = Some((col, rr, d));
                        }
                    }
                }
                if let Some((col, rr, _)) = best {
                    node_positions.push(land.cell_to_pos(col, rr));
                }
            }
        }
        let channel_count = node_positions.len() - open_count;

        // Build node list.
        let nodes: Vec<Node> = node_positions.iter().map(|&pos| Node { pos }).collect();

        // --- Spatial hash for edge candidate lookup ---
        let buckets = build_buckets(&nodes, BUCKET_NM);

        // --- Build edges ---
        let mut adj: Vec<Vec<Edge>> = vec![Vec::new(); nodes.len()];
        let r_buckets = (MAX_EDGE_NM / BUCKET_NM).ceil() as i32;
        for (i, n) in nodes.iter().enumerate() {
            let (bc, br) = pos_to_bucket(n.pos, BUCKET_NM);
            for dbr in -r_buckets..=r_buckets {
                for dbc in -r_buckets..=r_buckets {
                    let key = (bc + dbc, br + dbr);
                    if let Some(list) = buckets.get(&key) {
                        for &j in list {
                            if (j as usize) <= i {
                                continue; // each pair once
                            }
                            let m = &nodes[j as usize];
                            let d = n.pos.distance(m.pos);
                            if d > MAX_EDGE_NM || d <= 0.0 {
                                continue;
                            }
                            if land.corridor_is_clear(n.pos, m.pos, EDGE_MARGIN_NM) {
                                adj[i].push(Edge { to: j, dist_nm: d });
                                adj[j as usize].push(Edge {
                                    to: i as u32,
                                    dist_nm: d,
                                });
                            }
                        }
                    }
                }
            }
        }

        // --- Largest connected component only ---
        let comp = label_components(&adj);
        let main_id = largest_component(&comp);
        let keep: Vec<bool> = comp.iter().map(|&c| c == main_id).collect();
        // Re-index to drop pruned nodes.
        let mut remap: Vec<i32> = vec![-1; nodes.len()];
        let mut new_nodes: Vec<Node> = Vec::with_capacity(nodes.len());
        for (i, n) in nodes.iter().enumerate() {
            if keep[i] {
                remap[i] = new_nodes.len() as i32;
                new_nodes.push(*n);
            }
        }
        let mut new_adj: Vec<Vec<Edge>> = vec![Vec::new(); new_nodes.len()];
        for (i, edges) in adj.iter().enumerate() {
            let ni = remap[i];
            if ni < 0 {
                continue;
            }
            for e in edges {
                let nj = remap[e.to as usize];
                if nj < 0 {
                    continue;
                }
                new_adj[ni as usize].push(Edge {
                    to: nj as u32,
                    dist_nm: e.dist_nm,
                });
            }
        }

        let pruned = nodes.len() - new_nodes.len();
        let buckets = build_buckets(&new_nodes, BUCKET_NM);

        eprintln!(
            "[navmesh] {} open-water + {} channel = {} candidates; {} pruned to {} (1 component); avg deg {:.1}",
            open_count,
            channel_count,
            nodes.len(),
            pruned,
            new_nodes.len(),
            new_adj.iter().map(|v| v.len()).sum::<usize>() as f32 / new_nodes.len().max(1) as f32,
        );

        // unused now but keep for future caller reuse
        let _ = keep;

        Self {
            nodes: new_nodes,
            adj: new_adj,
            buckets,
            bucket_size_nm: BUCKET_NM,
        }
    }

    /// Indices of nodes within `radius_nm` of `pos`, regardless of visibility.
    pub fn nodes_within(&self, pos: Position, radius_nm: f32) -> Vec<u32> {
        let (bc, br) = pos_to_bucket(pos, self.bucket_size_nm);
        let r = (radius_nm / self.bucket_size_nm).ceil() as i32;
        let mut out = Vec::new();
        for dbr in -r..=r {
            for dbc in -r..=r {
                if let Some(list) = self.buckets.get(&(bc + dbc, br + dbr)) {
                    for &i in list {
                        if self.nodes[i as usize].pos.distance(pos) <= radius_nm {
                            out.push(i);
                        }
                    }
                }
            }
        }
        out
    }

    /// Indices of nodes visible from `pos` (corridor-clear), sorted by
    /// distance ascending. Used at the boundary of a route plan.
    /// `margin_nm` should be small (e.g. 0.5) for harbor-to-mesh hops since
    /// port anchors sit right at the coast.
    pub fn visible_from(
        &self,
        land: &LandMap,
        pos: Position,
        max_radius_nm: f32,
        max_count: usize,
        margin_nm: f32,
    ) -> Vec<u32> {
        let mut candidates: Vec<(u32, f32)> = self
            .nodes_within(pos, max_radius_nm)
            .into_iter()
            .map(|i| (i, self.nodes[i as usize].pos.distance(pos)))
            .collect();
        candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
        let mut out = Vec::new();
        for (i, _) in candidates {
            if land.corridor_is_clear(pos, self.nodes[i as usize].pos, margin_nm) {
                out.push(i);
                if out.len() >= max_count {
                    break;
                }
            }
        }
        out
    }

    /// A* over the graph from `start_set` to `goal_set`, returning the
    /// node index sequence (inclusive). Cost = sum of edge `dist_nm`.
    pub fn route(&self, start_set: &[u32], goal_set: &[u32]) -> Option<Vec<u32>> {
        if start_set.is_empty() || goal_set.is_empty() {
            return None;
        }
        let goal_pos: Vec<Position> = goal_set
            .iter()
            .map(|&g| self.nodes[g as usize].pos)
            .collect();
        let h = |i: u32| -> f32 {
            let p = self.nodes[i as usize].pos;
            goal_pos
                .iter()
                .map(|gp| p.distance(*gp))
                .fold(f32::INFINITY, f32::min)
        };
        let goal_lookup: std::collections::HashSet<u32> = goal_set.iter().copied().collect();

        #[derive(Copy, Clone, PartialEq)]
        struct N {
            f: f32,
            idx: u32,
        }
        impl Eq for N {}
        impl Ord for N {
            fn cmp(&self, other: &Self) -> Ordering {
                other.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal)
            }
        }
        impl PartialOrd for N {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }

        let mut g_score: HashMap<u32, f32> = HashMap::new();
        let mut came_from: HashMap<u32, u32> = HashMap::new();
        let mut open: BinaryHeap<N> = BinaryHeap::new();
        for &s in start_set {
            g_score.insert(s, 0.0);
            open.push(N { f: h(s), idx: s });
        }

        while let Some(N { idx: cur, .. }) = open.pop() {
            if goal_lookup.contains(&cur) {
                let mut path = vec![cur];
                let mut c = cur;
                while let Some(&p) = came_from.get(&c) {
                    path.push(p);
                    c = p;
                }
                path.reverse();
                return Some(path);
            }
            let cur_g = *g_score.get(&cur).unwrap_or(&f32::INFINITY);
            for e in &self.adj[cur as usize] {
                let tentative = cur_g + e.dist_nm;
                let prev = g_score.get(&e.to).copied().unwrap_or(f32::INFINITY);
                if tentative < prev {
                    g_score.insert(e.to, tentative);
                    came_from.insert(e.to, cur);
                    open.push(N {
                        f: tentative + h(e.to),
                        idx: e.to,
                    });
                }
            }
        }
        None
    }
}

/// Distance-from-land for a fine grid cell, in NM (Chebyshev).
fn cell_clearance_nm(land: &LandMap, col: u32, row: u32) -> u32 {
    if !land.is_sea_cell(col, row) {
        return 0;
    }

    land.dist_to_land[(row * land.width + col) as usize] as u32 // already in cells; for cell_size_nm = 1 NM, cells == NM.
}

fn pos_to_bucket(pos: Position, bucket_nm: f32) -> (i32, i32) {
    (
        (pos.x / bucket_nm).floor() as i32,
        (pos.y / bucket_nm).floor() as i32,
    )
}

fn build_buckets(nodes: &[Node], bucket_nm: f32) -> HashMap<(i32, i32), Vec<u32>> {
    let mut m: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        m.entry(pos_to_bucket(n.pos, bucket_nm))
            .or_default()
            .push(i as u32);
    }
    m
}

/// Label connected components via flood-fill over the adjacency list.
fn label_components(adj: &[Vec<Edge>]) -> Vec<i32> {
    let n = adj.len();
    let mut comp = vec![-1i32; n];
    let mut next_id = 0i32;
    for s in 0..n {
        if comp[s] != -1 {
            continue;
        }
        comp[s] = next_id;
        let mut stack = vec![s];
        while let Some(u) = stack.pop() {
            for e in &adj[u] {
                let v = e.to as usize;
                if comp[v] == -1 {
                    comp[v] = next_id;
                    stack.push(v);
                }
            }
        }
        next_id += 1;
    }
    comp
}

fn largest_component(comp: &[i32]) -> i32 {
    let mut counts: HashMap<i32, u32> = HashMap::new();
    for &c in comp {
        *counts.entry(c).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(_, n)| n)
        .map(|(id, _)| id)
        .unwrap_or(0)
}
