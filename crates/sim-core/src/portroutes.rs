//! Per-port pre-computed shortest paths over the [`TileMesh`].
//!
//! Phase D of the navmesh migration: the cache is now keyed by tile id
//! (the convex-tile portal-aware mesh loaded from `data/grids/navmesh.bin`)
//! rather than the legacy raster-derived [`Navmesh`] nodes. Each port's
//! entry-tile set is `{ anchor_tile } ∪ neighbours(anchor_tile)`; a
//! multi-source Dijkstra from that set yields, for every tile in the
//! mesh, the shortest centroid-graph distance and predecessor toward
//! the port. Per-tick voyages then resolve to a constant-time lookup
//! plus a predecessor walk — no live A* at all.
//!
//! The output is a tile-id sequence; the caller (`pathfind.rs`) stitches
//! it into a polyline by reconstructing each shared-edge `(left, right)`
//! portal and running SSFA, identical to the live planner.
//!
//! Memory: ~8 bytes × `tiles.len()` × `ports.len()` ≈ ~13 MB for 38
//! ports × 43k tiles. Build cost: ~few ms per port (one Dijkstra over
//! ~43k tiles with avg degree ~3), well under 100 ms total.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::coastline_geom::CoastlineGeom;
use crate::harbor::HarborMap;
use crate::map::land::LandMap;
use crate::tile_mesh::TileMesh;

/// SSSP table for one port destination: the shortest distance and
/// predecessor (tile id) for every tile toward that port's entry-tile
/// set. `pred == u32::MAX` marks an entry tile (a Dijkstra source).
struct PortRoutes {
    dist: Vec<f32>,
    pred: Vec<u32>,
}

/// Static cache holding a [`PortRoutes`] for every port that has at
/// least one tile-mesh entry tile. Indexed by `port_index` (matches
/// `Harbor::port_index`). Ports without an `anchor_tile` get `None`.
pub struct PortRouteCache {
    entries: Vec<Option<PortRoutes>>,
}

impl PortRouteCache {
    /// Build SSSP tables for every harbor in `harbors`. The result is
    /// indexed by `port_index`; ports without an `anchor_tile` yield
    /// `None`.
    ///
    /// The Dijkstra source set per harbor is `anchor_tile` plus any
    /// immediate neighbour whose centroid has clear polygon LOS to
    /// the anchor. The LOS filter matters in tight harbors (Port
    /// Royal sits at the tip of the Palisadoes spit — N/E/W neighbours
    /// are obstructed by land even though they share a mesh edge with
    /// the anchor tile). Without it, `route_from` can pick a neighbour
    /// source whose centroid → anchor segment crosses land, producing
    /// a path whose last leg sails through the spit.
    pub fn build(
        tile_mesh: &TileMesh,
        harbors: &HarborMap,
        land: &LandMap,
        geom: &CoastlineGeom,
    ) -> Self {
        let max_port_idx = harbors
            .harbors
            .iter()
            .map(|h| h.port_index)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        let mut entries: Vec<Option<PortRoutes>> = (0..max_port_idx).map(|_| None).collect();
        let n_tiles = tile_mesh.tiles.len();

        for harbor in &harbors.harbors {
            let Some(anchor_tile) = harbor.anchor_tile else {
                continue;
            };
            // Sources = anchor tile + neighbours whose centroid is
            // visible to the anchor. Matches the live planner's
            // harbor goal set (`pathfind::find_path_to_harbor`).
            let mut sources: Vec<u32> = Vec::with_capacity(8);
            sources.push(anchor_tile);
            for e in &tile_mesh.neighbors[anchor_tile as usize] {
                let nc = tile_mesh.tiles[e.to as usize].centroid;
                if geom.line_is_clear(land, nc, harbor.anchor) {
                    sources.push(e.to);
                }
            }
            let routes = dijkstra_from_sources(tile_mesh, n_tiles, &sources);
            entries[harbor.port_index] = Some(routes);
        }

        Self { entries }
    }

    /// Walk the cached predecessor chain to produce a tile-id sequence
    /// from one of `starts` to the entry-tile set for `port_idx`.
    /// Returns `None` when the port has no cache entry or no start is
    /// reachable.
    ///
    /// The returned sequence starts with the chosen start tile and
    /// ends at an entry tile — matches `TileMesh::route`'s output
    /// orientation, so callers can substitute one for the other.
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

/// Multi-source Dijkstra over the tile-mesh adjacency: every tile in
/// `sources` starts at distance 0 with `pred = u32::MAX` (sentinel for
/// "I am a source"). Standard relaxation otherwise; edge cost is the
/// `dist_nm` field already carried on `TileMesh::neighbors`.
fn dijkstra_from_sources(mesh: &TileMesh, n_tiles: usize, sources: &[u32]) -> PortRoutes {
    let mut dist = vec![f32::INFINITY; n_tiles];
    let mut pred = vec![u32::MAX; n_tiles];
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
        let relax = |e: &crate::tile_mesh::TileEdge,
                     dist: &mut Vec<f32>,
                     pred: &mut Vec<u32>,
                     heap: &mut BinaryHeap<HeapEntry>| {
            let nd = d + mesh.edge_cost(cur, e);
            let to_us = e.to as usize;
            if nd < dist[to_us] {
                dist[to_us] = nd;
                pred[to_us] = cur;
                heap.push(HeapEntry { d: nd, idx: e.to });
            }
        };
        for e in &mesh.neighbors[cur_us] {
            relax(e, &mut dist, &mut pred, &mut heap);
        }
        if let Some(shortcuts) = mesh.shortcut_neighbors.get(cur_us) {
            for e in shortcuts {
                relax(e, &mut dist, &mut pred, &mut heap);
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
