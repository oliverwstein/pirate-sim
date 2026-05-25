//! Per-port pre-computed shortest paths over the [`Navmesh`].
//!
//! Perf phase 7: every port destination in the world has a fixed set of
//! navmesh entry nodes (the mesh nodes visible from the harbor anchor).
//! Because the navmesh graph is static and edge costs (`dist_nm`) are
//! invariant, the all-pairs shortest path *to* a given port can be
//! precomputed once at world load via multi-source Dijkstra from those
//! entry nodes. Per-tick voyages then become a constant-time lookup
//! plus an O(path-length) predecessor walk — no A* at all.
//!
//! Algorithmic note: a cached Dijkstra returns an *optimal-cost* path
//! that may differ from live A* at tie-break points (Dijkstra has no
//! heuristic-induced ordering bias). Total cost is preserved, but the
//! exact node sequence on equal-cost ties may shift. This is acceptable
//! for our use case (waypoints stitched through `smooth_path` later).
//!
//! Memory: ~8 bytes × `nodes.len()` × `ports.len()` ≈ ~11 MB for 38
//! ports × 37,588 nodes. Build cost: ~30 ms per port (one Dijkstra),
//! ~1 s added to `World::load`.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::harbor::HarborMap;
use crate::map::land::LandMap;
use crate::navmesh::Navmesh;

/// SSSP table for one port destination: the shortest distance and
/// predecessor for every navmesh node toward that port's entry-node
/// set. `pred == u32::MAX` marks an entry node (a Dijkstra source).
struct PortRoutes {
    dist: Vec<f32>,
    pred: Vec<u32>,
}

/// Static cache holding a [`PortRoutes`] for every port that has at
/// least one navmesh entry node. Indexed by `port_index` (matches
/// `Harbor::port_index`). Ports without a reachable mesh entry (e.g.
/// totally enclosed test setups) get `None`.
pub struct PortRouteCache {
    entries: Vec<Option<PortRoutes>>,
}

/// Same constants as `pathfind::find_path_to_harbor` for the
/// harbor-anchor → mesh visibility probe. Duplicated here to avoid a
/// circular dependency with `pathfind`.
const ENTRY_RADIUS_NM: f32 = 200.0;
const ENTRY_CANDIDATES: usize = 16;
const ENTRY_MARGIN_NM: f32 = 0.0;

impl PortRouteCache {
    /// Build SSSP tables for every harbor in `harbors`. The result is
    /// indexed by `port_index`; ports not present in the harbor map
    /// (or with no reachable entry node) yield `None`.
    pub fn build(land: &LandMap, nm: &Navmesh, harbors: &HarborMap) -> Self {
        let max_port_idx = harbors
            .harbors
            .iter()
            .map(|h| h.port_index)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        let mut entries: Vec<Option<PortRoutes>> = (0..max_port_idx).map(|_| None).collect();
        let n_nodes = nm.nodes.len();

        for harbor in &harbors.harbors {
            let entry_nodes = nm.visible_from(
                land,
                harbor.anchor,
                ENTRY_RADIUS_NM,
                ENTRY_CANDIDATES,
                ENTRY_MARGIN_NM,
            );
            if entry_nodes.is_empty() {
                continue;
            }
            let routes = dijkstra_from_sources(nm, n_nodes, &entry_nodes);
            entries[harbor.port_index] = Some(routes);
        }

        Self { entries }
    }

    /// Walk the cached predecessor chain to produce a path from one of
    /// `starts` to the entry-node set for `port_idx`. Returns `None`
    /// when the port has no cache entry or no start is reachable.
    ///
    /// The returned sequence starts with the chosen start node and
    /// ends at an entry node — same orientation as `Navmesh::route`'s
    /// output, so callers can substitute one for the other.
    pub fn route_from(&self, starts: &[u32], port_idx: usize) -> Option<Vec<u32>> {
        if starts.is_empty() {
            return None;
        }
        let routes = self.entries.get(port_idx)?.as_ref()?;

        let mut best_start = u32::MAX;
        let mut best_dist = f32::INFINITY;
        for &s in starts {
            let d = routes.dist[s as usize];
            if d < best_dist {
                best_dist = d;
                best_start = s;
            }
        }
        if best_start == u32::MAX || best_dist == f32::INFINITY {
            return None;
        }

        let mut path = Vec::with_capacity(32);
        let mut cur = best_start;
        path.push(cur);
        loop {
            let p = routes.pred[cur as usize];
            if p == u32::MAX {
                break;
            }
            path.push(p);
            cur = p;
        }
        Some(path)
    }
}

/// Multi-source Dijkstra over the navmesh adjacency: every node in
/// `sources` starts at distance 0 with `pred = u32::MAX` (sentinel for
/// "I am a source"). Standard relaxation otherwise.
fn dijkstra_from_sources(nm: &Navmesh, n_nodes: usize, sources: &[u32]) -> PortRoutes {
    let mut dist = vec![f32::INFINITY; n_nodes];
    let mut pred = vec![u32::MAX; n_nodes];
    let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();

    for &s in sources {
        let idx = s as usize;
        if dist[idx] > 0.0 {
            dist[idx] = 0.0;
            heap.push(HeapEntry { d: 0.0, idx: s });
        }
    }

    while let Some(HeapEntry { d, idx: cur }) = heap.pop() {
        let cur_us = cur as usize;
        if d > dist[cur_us] {
            continue;
        }
        for e in &nm.adj[cur_us] {
            let nd = d + e.dist_nm;
            let to_us = e.to as usize;
            if nd < dist[to_us] {
                dist[to_us] = nd;
                pred[to_us] = cur;
                heap.push(HeapEntry { d: nd, idx: e.to });
            }
        }
    }

    PortRoutes { dist, pred }
}

#[derive(Copy, Clone, PartialEq)]
struct HeapEntry {
    d: f32,
    idx: u32,
}
impl Eq for HeapEntry {}
impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.d.partial_cmp(&self.d).unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
