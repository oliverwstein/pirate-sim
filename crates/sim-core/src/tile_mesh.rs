//! Portal-aware convex-tile navigation mesh, loaded from
//! `data/grids/navmesh.bin`.
//!
//! This module replaces the programmatic `Navmesh::build(&LandMap)` for
//! the new navigation substrate. The mesh itself is produced offline
//! by `tools/preprocess/preprocess_navmesh.py` (Hertel–Mehlhorn convex
//! merge of a CDT of the sea polygon, with the land buffered outward
//! in EPSG:3857 to remove staircase wedging).
//!
//! Routing model:
//!
//! - Nodes are tile **centroids**. Each tile is convex by
//!   construction, so any point inside the tile has line-of-sight to
//!   the centroid.
//! - Edges between adjacent tiles carry a **portal**: the midpoint of
//!   the shared polygon edge. Routing as `centroid_A → portal_AB →
//!   centroid_B` therefore can never cross land, even when the
//!   convex-merge pass produced an L-shaped tile whose centroid sits
//!   in the inside corner.
//! - We do **not** ask "which tile is this position in" — that would
//!   require point-in-(possibly-sharp)-polygon. The only locality
//!   query we need is "give me the nearest few centroids that are
//!   line-of-sight visible from `pos`" (Phase C will wire that to
//!   `coastline_geom`).
//!
//! Binary format v2 (matches `preprocess_navmesh.py::write_navmesh`,
//! little-endian):
//!
//! ```text
//! u32  magic = 0x32564D4E ("NMV2")
//! u32  num_tiles
//! per tile:
//!     u32  num_vertices
//!     (f32 x, f32 y) × num_vertices     // CCW, NM-from-origin
//!     f32  centroid_x, f32 centroid_y
//!     f32  clearance_nm                 // distance to nearest land,
//!                                          capped at the preprocessor's
//!                                          CLEARANCE_REPORT_CAP_NM (30 NM).
//!     u32  num_neighbors
//!     (u32 tile_index, f32 portal_x, f32 portal_y) × num_neighbors
//! ```
//!
//! The preprocessor already filters to the largest connected
//! component, so the loader does not repeat that step.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::io;
use std::path::Path;

use crate::types::Position;

/// Centroid-bucket cell edge length, in NM. Sized so the average
/// bucket holds a handful of centroids across the Caribbean +
/// Atlantic basin (~40k tiles spread over ~6300 × 3900 NM).
pub const BUCKET_NM: f32 = 25.0;

/// One convex tile in the navmesh. The vertex list is owned inline so
/// runtime callers don't need to chase indirections; tiles average
/// ~5–6 vertices so the inline cost is small.
#[derive(Debug, Clone)]
pub struct Tile {
    pub vertices: Vec<Position>,
    pub centroid: Position,
    /// AABB of `vertices`. Convenient for cheap rejection in higher-
    /// level queries (e.g. "is this position even close to this tile").
    pub bbox_min: Position,
    pub bbox_max: Position,
}

/// One undirected adjacency, stored on both endpoints.
///
/// `portal` is the midpoint of the shared polygon edge; routing via
/// the portal is guaranteed to stay inside `A ∪ B`.
#[derive(Debug, Clone, Copy)]
pub struct TileEdge {
    pub to: u32,
    pub portal: Position,
    /// Centroid-to-centroid distance in NM. The A* cost function used
    /// by `route`. (Portal-aware path *length* is computed by the
    /// Phase C funnel pass, not by this graph.)
    pub dist_nm: f32,
}

/// Portal-aware convex-tile navmesh.
pub struct TileMesh {
    pub tiles: Vec<Tile>,
    pub neighbors: Vec<Vec<TileEdge>>,
    /// Optional long-range A* edges for deep-water tiles. Empty until
    /// [`TileMesh::build_deep_water_shortcuts`] is called. Each entry
    /// `i` lists up-to-8 octant-binned shortcuts from tile `i` to
    /// distant deep-water tiles whose centroid is LOS-clear from
    /// `tiles[i].centroid`. Routing layers (`route`,
    /// `dijkstra_from_sources`) walk both `neighbors` and these
    /// shortcuts during relaxation. The mesh geometry, portals, and
    /// per-ship `current_tile` tracking are untouched: shortcuts only
    /// densify the A* graph in open ocean, so deep-water paths
    /// approach the rhumb line instead of zig-zagging through the
    /// hex lattice's discrete bearing axes.
    pub shortcut_neighbors: Vec<Vec<TileEdge>>,
    /// Spatial hash of centroids in `BUCKET_NM`-sized cells. The key
    /// is `(floor(x / BUCKET_NM), floor(y / BUCKET_NM))`.
    centroid_buckets: HashMap<(i32, i32), Vec<u32>>,
    /// Per-tile distance (NM) from the centroid to the nearest
    /// coastline edge, capped at [`CLEARANCE_MAX_NM`]. Empty after
    /// [`TileMesh::load`]; filled by [`TileMesh::set_clearance`] once
    /// `CoastlineGeom` is available. All zeros means "no clearance
    /// information" (planner falls back to pure distance cost).
    clearance_nm: Vec<f32>,
}

/// Cap on the clearance-from-land lookup. Beyond this, every tile is
/// considered "fully clear" and pays no penalty. 1 NM is the soft
/// preferred buffer (see `clearance_penalty`); the cap is set a bit
/// higher so the penalty curve has headroom and the bucket scan is
/// bounded.
/// Hard upper bound on the per-tile clearance value carried in
/// `clearance_nm`. The preprocessor caps at 30 NM (well past
/// [`DEEP_WATER_NM`]); the Rust side uses the value for two purposes:
/// (a) the [`clearance_penalty`] ramp (only the 0..1 NM range
/// matters there) and (b) the steering layer's "am I in deep water?"
/// gate (compares against `DEEP_WATER_NM`). Anything above the cap
/// is informationally equivalent for both.
pub const CLEARANCE_MAX_NM: f32 = 30.0;

/// Soft preferred clearance (NM). Tiles with centroid clearance ≥ this
/// value incur no routing penalty. Tiles closer to land are penalised
/// quadratically up to a maximum cost multiplier of
/// `1.0 + CLEARANCE_PENALTY_K`, applied at clearance = 0.
pub const PREFERRED_CLEARANCE_NM: f32 = 1.0;

/// Maximum extra cost multiplier when clearance = 0. A value of 9.0
/// means a hard-against-coast edge costs 10× its true length, so a
/// detour of up to ~10× the straight-line distance will be chosen if
/// it stays in open water — but if the only route hugs the coast, the
/// planner still takes it (cost stays finite).
pub const CLEARANCE_PENALTY_K: f32 = 9.0;

/// Open-water threshold (NM). Tiles whose centroid clearance reaches
/// this value are considered "in deep water". The Python preprocessor
/// places regular hex tiles wherever the sea (eroded by a small
/// buffer) fully contains a hex of pitch ≥ this value, so deep-water
/// tiles naturally satisfy `clearance ≥ DEEP_WATER_NM`.
pub const DEEP_WATER_NM: f32 = 12.0;

/// Magic header for navmesh.bin format v2 (little-endian "NMV2").
/// The Python preprocessor writes this as the first 4 bytes of the
/// file; the Rust loader rejects any file that doesn't start with it
/// so we fail loudly instead of misreading an older format.
pub const NAVMESH_MAGIC: u32 = 0x32564D4E;

/// Edge-cost penalty multiplier for two tiles with the given pairwise
/// clearance (the *minimum* of the two endpoint clearances). Returns
/// 1.0 when `clearance ≥ PREFERRED_CLEARANCE_NM`, ramps quadratically
/// up to `1.0 + CLEARANCE_PENALTY_K` at `clearance = 0`.
#[inline]
pub fn clearance_penalty(clearance: f32) -> f32 {
    let deficit = (PREFERRED_CLEARANCE_NM - clearance).max(0.0) / PREFERRED_CLEARANCE_NM;
    1.0 + CLEARANCE_PENALTY_K * deficit * deficit
}

impl TileMesh {
    /// An empty tile mesh — for tests that don't exercise the planner's
    /// tile-routing path.
    pub fn empty() -> Self {
        Self {
            tiles: Vec::new(),
            neighbors: Vec::new(),
            shortcut_neighbors: Vec::new(),
            centroid_buckets: HashMap::new(),
            clearance_nm: Vec::new(),
        }
    }

    /// Load a navmesh from the binary produced by
    /// `preprocess_navmesh.py`. Returns an error on truncation or
    /// any inconsistency; panics are reserved for the caller.
    pub fn load(path: &Path) -> io::Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
    }

    /// Parse a navmesh from a byte buffer (`navmesh.bin` contents).
    /// Expects the v2 format documented at the top of this module.
    pub fn from_bytes(buf: &[u8]) -> io::Result<Self> {
        let mut cur = 0usize;
        let magic = read_u32(buf, &mut cur)?;
        if magic != NAVMESH_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "navmesh magic mismatch: got 0x{magic:08X}, expected 0x{NAVMESH_MAGIC:08X} \
                     (regenerate with tools/preprocess/preprocess_navmesh.py)"
                ),
            ));
        }
        let num_tiles = read_u32(buf, &mut cur)? as usize;

        let mut tiles: Vec<Tile> = Vec::with_capacity(num_tiles);
        let mut neighbors: Vec<Vec<TileEdge>> = Vec::with_capacity(num_tiles);
        let mut clearance_nm: Vec<f32> = Vec::with_capacity(num_tiles);
        let mut raw_neighbors: Vec<Vec<(u32, Position)>> = Vec::with_capacity(num_tiles);

        for tile_idx in 0..num_tiles {
            let n_verts = read_u32(buf, &mut cur)? as usize;
            let mut vertices = Vec::with_capacity(n_verts);
            let mut bbox_min = Position::new(f32::INFINITY, f32::INFINITY);
            let mut bbox_max = Position::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
            for _ in 0..n_verts {
                let x = read_f32(buf, &mut cur)?;
                let y = read_f32(buf, &mut cur)?;
                let p = Position::new(x, y);
                vertices.push(p);
                bbox_min.x = bbox_min.x.min(x);
                bbox_min.y = bbox_min.y.min(y);
                bbox_max.x = bbox_max.x.max(x);
                bbox_max.y = bbox_max.y.max(y);
            }
            let cx = read_f32(buf, &mut cur)?;
            let cy = read_f32(buf, &mut cur)?;
            let centroid = Position::new(cx, cy);
            let clr = read_f32(buf, &mut cur)?;

            let n_nbrs = read_u32(buf, &mut cur)? as usize;
            let mut nbrs = Vec::with_capacity(n_nbrs);
            for _ in 0..n_nbrs {
                let nb = read_u32(buf, &mut cur)?;
                if (nb as usize) >= num_tiles {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "tile {} references out-of-range neighbor {} (num_tiles={})",
                            tile_idx, nb, num_tiles
                        ),
                    ));
                }
                let px = read_f32(buf, &mut cur)?;
                let py = read_f32(buf, &mut cur)?;
                nbrs.push((nb, Position::new(px, py)));
            }

            tiles.push(Tile {
                vertices,
                centroid,
                bbox_min,
                bbox_max,
            });
            clearance_nm.push(clr);
            raw_neighbors.push(nbrs);
        }

        if cur != buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "trailing {} bytes after navmesh tile records",
                    buf.len() - cur
                ),
            ));
        }

        // Second pass: fill in centroid-to-centroid edge distances.
        for nbrs in &raw_neighbors {
            let edges: Vec<TileEdge> = nbrs
                .iter()
                .map(|&(to, portal)| {
                    let from_pos = tiles[neighbors.len()].centroid;
                    let to_pos = tiles[to as usize].centroid;
                    TileEdge {
                        to,
                        portal,
                        dist_nm: from_pos.distance(to_pos),
                    }
                })
                .collect();
            neighbors.push(edges);
        }

        let centroid_buckets = build_centroid_buckets(&tiles, BUCKET_NM);

        Ok(Self {
            tiles,
            neighbors,
            shortcut_neighbors: Vec::new(),
            centroid_buckets,
            clearance_nm,
        })
    }

    /// Number of tiles in the mesh.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// Empty-mesh predicate. Test-only fixtures may produce empty
    /// meshes; the loader rejects empty real files.
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Install per-tile clearance values. `clearance.len()` must equal
    /// `self.tiles.len()`. Each entry is the distance (NM, capped at
    /// [`CLEARANCE_MAX_NM`]) from the tile centroid to the nearest
    /// coastline edge. Called by `World::load` after both the mesh
    /// and the `CoastlineGeom` have been built.
    pub fn set_clearance(&mut self, clearance: Vec<f32>) {
        debug_assert_eq!(clearance.len(), self.tiles.len());
        self.clearance_nm = clearance;
    }

    /// Build long-range A* shortcut edges for deep-water tiles.
    ///
    /// On a flat-top hex lattice the only neighbour directions are
    /// `30°, 90°, 150°, …`; an A* path between two points whose true
    /// bearing falls between those axes can only approximate it by
    /// zig-zagging or by following one axis fully then turning. Both
    /// shapes have the same total length, so the heap's tie-break
    /// picks whichever expansion order it sees first — typically a
    /// long monotone leg along a single axis that overshoots toward
    /// a map edge before turning back (the "Africa → Caribbean
    /// route hugs the equator" symptom).
    ///
    /// This method densifies the A* graph in open ocean so the
    /// planner can take a single straight edge between distant
    /// deep-water tiles instead of stepping hex-by-hex along an
    /// axis. The mesh geometry, portals, neighbour adjacency for
    /// `walk_to_tile`, and `current_tile` tracking are all
    /// untouched — only the routing graph gets extra edges.
    ///
    /// Algorithm: for every tile whose `clearance_nm ≥ DEEP_WATER_NM`,
    /// gather all other deep-water tiles within `radius_nm`, bin
    /// them into `n_octants` angular sectors around the centroid,
    /// and for each sector keep the **farthest** candidate whose
    /// centroid-to-centroid segment passes the polygon-LOS test.
    /// Result: ≤ `n_octants` shortcut edges per deep-water tile,
    /// each spanning up to `radius_nm` of open ocean.
    ///
    /// LOS checks use `geom.line_is_clear(land, …)`; the raster
    /// fast-path inside `CoastlineGeom` keeps them cheap (a few µs
    /// each), and the farthest-first ordering per sector means we
    /// usually need only 1 LOS check per sector. The whole build
    /// runs in parallel over tiles via Rayon.
    ///
    /// Requires clearance data — calls `has_clearance()` first and
    /// becomes a no-op if none is installed (test fixtures).
    pub fn build_deep_water_shortcuts(
        &mut self,
        geom: &crate::coastline_geom::CoastlineGeom,
        land: &crate::map::land::LandMap,
        radius_nm: f32,
        n_octants: usize,
    ) {
        use rayon::prelude::*;

        if !self.has_clearance() || self.tiles.is_empty() {
            self.shortcut_neighbors = vec![Vec::new(); self.tiles.len()];
            return;
        }
        let n_octants = n_octants.max(1);
        let sector_rad = std::f32::consts::TAU / n_octants as f32;

        let n = self.tiles.len();
        let centroids: Vec<Position> = self.tiles.iter().map(|t| t.centroid).collect();
        let clearance = &self.clearance_nm;

        self.shortcut_neighbors = (0..n)
            .into_par_iter()
            .map(|i| {
                if clearance[i] < DEEP_WATER_NM {
                    return Vec::new();
                }
                let pi = centroids[i];
                // Gather deep-water candidates within radius, with
                // distance. `nearest_centroids` already sorts ascending;
                // we'll bucket then sort per-octant descending.
                let mut by_sector: Vec<Vec<(f32, u32)>> = vec![Vec::new(); n_octants];
                for (j, dist) in self.nearest_centroids(pi, radius_nm) {
                    if j as usize == i {
                        continue;
                    }
                    if clearance[j as usize] < DEEP_WATER_NM {
                        continue;
                    }
                    let pj = centroids[j as usize];
                    let bearing = (pj.x - pi.x).atan2(pj.y - pi.y);
                    let normalized = (bearing + std::f32::consts::TAU) % std::f32::consts::TAU;
                    let sector = ((normalized / sector_rad) as usize) % n_octants;
                    by_sector[sector].push((dist, j));
                }
                let mut edges: Vec<TileEdge> = Vec::with_capacity(n_octants);
                for bucket in &mut by_sector {
                    bucket.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                    for &(d, j) in bucket.iter() {
                        let pj = centroids[j as usize];
                        if geom.line_is_clear(land, pi, pj) {
                            edges.push(TileEdge {
                                to: j,
                                portal: (pi + pj) * 0.5,
                                dist_nm: d,
                            });
                            break;
                        }
                    }
                }
                edges
            })
            .collect();
    }

    /// True iff clearance data has been installed. When false, the
    /// router falls back to pure distance cost (penalty multiplier 1).
    #[inline]
    pub fn has_clearance(&self) -> bool {
        !self.clearance_nm.is_empty()
    }

    /// Effective routing cost for the directed edge `from → e.to`,
    /// blending raw distance with the clearance penalty. Returns
    /// `e.dist_nm` unchanged when no clearance data is installed.
    #[inline]
    pub fn edge_cost(&self, from: u32, e: &TileEdge) -> f32 {
        if self.clearance_nm.is_empty() {
            return e.dist_nm;
        }
        let ca = self.clearance_nm[from as usize];
        let cb = self.clearance_nm[e.to as usize];
        e.dist_nm * clearance_penalty(ca.min(cb))
    }

    /// Indices of tiles whose centroid is within `radius_nm` of `pos`,
    /// sorted by ascending centroid distance.
    ///
    /// **Pure locality, no LOS filter.** Phase C / Phase E callers
    /// will compose this with `coastline_geom::line_is_clear` to
    /// pick visible entries.
    pub fn nearest_centroids(&self, pos: Position, radius_nm: f32) -> Vec<(u32, f32)> {
        let (bc, br) = pos_to_bucket(pos, BUCKET_NM);
        let r_buckets = (radius_nm / BUCKET_NM).ceil() as i32;
        let r2 = radius_nm * radius_nm;
        let mut out: Vec<(u32, f32)> = Vec::new();
        for dbr in -r_buckets..=r_buckets {
            for dbc in -r_buckets..=r_buckets {
                if let Some(list) = self.centroid_buckets.get(&(bc + dbc, br + dbr)) {
                    for &i in list {
                        let c = self.tiles[i as usize].centroid;
                        let dx = c.x - pos.x;
                        let dy = c.y - pos.y;
                        let d2 = dx * dx + dy * dy;
                        if d2 <= r2 {
                            out.push((i, d2.sqrt()));
                        }
                    }
                }
            }
        }
        out.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
        out
    }

    /// A* over the tile-centroid graph from any of `start_tiles` to
    /// any of `goal_tiles`. Returns the tile-id sequence
    /// (`start_tile`, …, `goal_tile`), inclusive on both ends.
    ///
    /// Cost is the sum of centroid-to-centroid `dist_nm` of traversed
    /// edges — a slight under-estimate of the funnel-smoothed real
    /// path, but a consistent heuristic for A*. The Phase C pipeline
    /// uses this sequence to derive the portal corridor and runs the
    /// funnel pass on top.
    ///
    /// Uses thread-local scratch storage (`ROUTE_SCRATCH`) so the
    /// per-call cost is proportional to expanded tiles, not to
    /// `tiles.len()`.
    pub fn route(&self, start_tiles: &[u32], goal_tiles: &[u32]) -> Option<Vec<u32>> {
        if start_tiles.is_empty() || goal_tiles.is_empty() || self.tiles.is_empty() {
            return None;
        }
        let n_tiles = self.tiles.len();

        let goal_centroids: Vec<Position> = goal_tiles
            .iter()
            .map(|&g| self.tiles[g as usize].centroid)
            .collect();
        let h = |i: u32| -> f32 {
            let p = self.tiles[i as usize].centroid;
            goal_centroids
                .iter()
                .map(|gc| p.distance(*gc))
                .fold(f32::INFINITY, f32::min)
        };

        ROUTE_SCRATCH.with(|cell| {
            let mut scratch = cell.borrow_mut();
            scratch.reset_and_size(n_tiles);

            for &g in goal_tiles {
                let idx = g as usize;
                if !scratch.in_goal[idx] {
                    scratch.in_goal[idx] = true;
                    scratch.touched_goals.push(g);
                }
            }

            for &s in start_tiles {
                let idx = s as usize;
                if scratch.g_score[idx] == f32::INFINITY {
                    scratch.touched.push(s);
                }
                scratch.g_score[idx] = 0.0;
                scratch.open.push(HeapEntry { f: h(s), idx: s });
            }

            while let Some(HeapEntry { idx: cur, .. }) = scratch.open.pop() {
                let cur_us = cur as usize;
                if scratch.in_goal[cur_us] {
                    let mut path = vec![cur];
                    let mut c = cur;
                    loop {
                        let p = scratch.came_from[c as usize];
                        if p == u32::MAX {
                            break;
                        }
                        path.push(p);
                        c = p;
                    }
                    path.reverse();
                    return Some(path);
                }
                let cur_g = scratch.g_score[cur_us];
                let relax = |e: &TileEdge, scratch: &mut RouteScratch| {
                    let tentative = cur_g + self.edge_cost(cur, e);
                    let to_us = e.to as usize;
                    let prev = scratch.g_score[to_us];
                    if tentative < prev {
                        if prev == f32::INFINITY {
                            scratch.touched.push(e.to);
                        }
                        scratch.g_score[to_us] = tentative;
                        scratch.came_from[to_us] = cur;
                        scratch.open.push(HeapEntry {
                            f: tentative + h(e.to),
                            idx: e.to,
                        });
                    }
                };
                for e in &self.neighbors[cur_us] {
                    relax(e, &mut scratch);
                }
                if let Some(shortcuts) = self.shortcut_neighbors.get(cur_us) {
                    for e in shortcuts {
                        relax(e, &mut scratch);
                    }
                }
            }
            None
        })
    }

    /// Look up the portal `Position` for the directed edge `a → b`.
    /// Returns `None` if the tiles are not adjacent. Used by the
    /// Phase C funnel pass.
    pub fn portal_between(&self, a: u32, b: u32) -> Option<Position> {
        self.neighbors.get(a as usize)?.iter().find_map(|e| {
            if e.to == b {
                Some(e.portal)
            } else {
                None
            }
        })
    }

    /// The two shared-edge endpoints between adjacent tiles `a` and `b`,
    /// returned as `(left, right)` from the perspective of someone
    /// **standing in tile `a` looking into tile `b`**. "Left" is the
    /// endpoint that lies on the port (left) side of the directed
    /// `centroid_a → centroid_b` ray; "right" is on the starboard side.
    ///
    /// Returns `None` if the tiles aren't adjacent or don't share two
    /// vertices within the merge tolerance (which would indicate a
    /// malformed mesh).
    ///
    /// Used by the SSFA funnel pass in `pathfind` to smooth the
    /// centroid chain into a near-shortest polyline.
    pub fn shared_edge(&self, a: u32, b: u32) -> Option<(Position, Position)> {
        let ta = self.tiles.get(a as usize)?;
        let tb = self.tiles.get(b as usize)?;
        // Tiles share an edge iff two of `ta.vertices` appear in
        // `tb.vertices` within ε. The preprocessor emits vertex
        // coordinates rounded to f32 at the same precision in both
        // tiles, so an exact match works for the bundled mesh; a
        // small ε keeps us safe against any future preprocessor
        // changes that introduce sub-NM jitter.
        const EPS_NM: f32 = 0.05;
        let near = |p: Position, q: Position| -> bool {
            (p.x - q.x).abs() <= EPS_NM && (p.y - q.y).abs() <= EPS_NM
        };

        let mut shared: [Option<Position>; 2] = [None, None];
        let mut n = 0usize;
        for &va in &ta.vertices {
            if tb.vertices.iter().any(|&vb| near(va, vb)) {
                if n < 2 {
                    shared[n] = Some(va);
                }
                n += 1;
            }
        }
        if n != 2 {
            return None;
        }
        let p0 = shared[0]?;
        let p1 = shared[1]?;

        // Orient (p0, p1) so that p0 is on the **left** of the
        // directed ray `centroid_a → centroid_b`. The 2D cross
        // product `(b - a) × (p - a)` is positive when `p` is on the
        // left, negative on the right.
        let ca = ta.centroid;
        let cb = tb.centroid;
        let dx = cb.x - ca.x;
        let dy = cb.y - ca.y;
        let cross_p0 = dx * (p0.y - ca.y) - dy * (p0.x - ca.x);
        if cross_p0 >= 0.0 {
            Some((p0, p1))
        } else {
            Some((p1, p0))
        }
    }

    /// Test whether `pos` lies inside the convex polygon of tile `id`.
    /// Uses the half-plane sign test for each ordered edge; returns
    /// true if `pos` is on the same side of every edge (CCW polygon).
    /// Tolerates degenerate edges by ignoring them.
    pub fn tile_contains(&self, id: u32, pos: Position) -> bool {
        let Some(tile) = self.tiles.get(id as usize) else {
            return false;
        };
        if pos.x < tile.bbox_min.x
            || pos.x > tile.bbox_max.x
            || pos.y < tile.bbox_min.y
            || pos.y > tile.bbox_max.y
        {
            return false;
        }
        let v = &tile.vertices;
        if v.len() < 3 {
            return false;
        }
        let mut sign: f32 = 0.0;
        for i in 0..v.len() {
            let a = v[i];
            let b = v[(i + 1) % v.len()];
            let cross = (b.x - a.x) * (pos.y - a.y) - (b.y - a.y) * (pos.x - a.x);
            if cross.abs() < 1e-6 {
                continue;
            }
            if sign == 0.0 {
                sign = cross;
            } else if sign.signum() != cross.signum() {
                return false;
            }
        }
        true
    }

    /// Locate the tile containing `pos` by searching the nearest
    /// centroids (cheap broad-phase) and applying the half-plane test.
    /// Returns `None` if `pos` is not inside any tile within `radius_nm`.
    pub fn find_tile_containing(&self, pos: Position, radius_nm: f32) -> Option<u32> {
        self.nearest_centroids(pos, radius_nm)
            .into_iter()
            .map(|(id, _)| id)
            .find(|&id| self.tile_contains(id, pos))
    }

    /// Update the ship's current tile after a move `old_pos → new_pos`,
    /// starting from `current`. Walks portal-by-portal up to `MAX_HOPS`
    /// times: if the move crossed any shared edge with a neighbor, the
    /// neighbor becomes the new current and we test from there. Falls
    /// back to a fresh [`find_tile_containing`] when the walk leaves
    /// the known neighborhood (rare — typically a teleport or replan).
    pub fn walk_to_tile(&self, current: u32, old_pos: Position, new_pos: Position) -> Option<u32> {
        const MAX_HOPS: u32 = 4;
        let mut tile = current;
        for _ in 0..MAX_HOPS {
            let mut crossed: Option<u32> = None;
            if let Some(edges) = self.neighbors.get(tile as usize) {
                for e in edges {
                    if let Some((l, r)) = self.shared_edge(tile, e.to) {
                        if segments_cross(old_pos, new_pos, l, r) {
                            crossed = Some(e.to);
                            break;
                        }
                    }
                }
            }
            match crossed {
                Some(next) => tile = next,
                None => {
                    return if self.tile_contains(tile, new_pos) {
                        Some(tile)
                    } else {
                        self.find_tile_containing(new_pos, 50.0)
                    };
                }
            }
        }
        Some(tile)
    }
}

/// Standard 2-D segment-segment crossing test (open intervals).
fn segments_cross(p1: Position, p2: Position, p3: Position, p4: Position) -> bool {
    let o = |a: Position, b: Position, c: Position| -> f32 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    };
    let d1 = o(p3, p4, p1);
    let d2 = o(p3, p4, p2);
    let d3 = o(p1, p2, p3);
    let d4 = o(p1, p2, p4);
    (d1 > 0.0 && d2 < 0.0 || d1 < 0.0 && d2 > 0.0) && (d3 > 0.0 && d4 < 0.0 || d3 < 0.0 && d4 > 0.0)
}

/// Simple Stupid Funnel Algorithm.
///
/// Given a `start` point, an `end` point, and a chain of `(left, right)`
/// portal endpoints between them — each pair oriented so `left` lies
/// on the left of the directed corridor — returns a near-shortest
/// polyline through the corridor as a sequence of waypoints **not
/// including `start`** (the caller already knows where the ship is)
/// but **including `end`** (so the caller can re-anchor to a precise
/// target like a harbor anchor).
///
/// The corridor is sealed by treating `end` as the final `(left, right)`
/// pair `(end, end)` — a degenerate portal at the goal. This lets the
/// algorithm terminate cleanly without special-casing the last segment.
///
/// Empty `portals` (no shared edges; `start` and `end` are inside the
/// same tile) → `vec![end]`.
pub fn funnel(start: Position, end: Position, portals: &[(Position, Position)]) -> Vec<Position> {
    // Seal the corridor with a degenerate final portal at `end`.
    let mut seq: Vec<(Position, Position)> = Vec::with_capacity(portals.len() + 1);
    seq.extend_from_slice(portals);
    seq.push((end, end));

    let mut out: Vec<Position> = Vec::with_capacity(seq.len());
    let mut apex = start;
    let mut left = start;
    let mut right = start;
    let mut left_idx: usize = 0;
    let mut right_idx: usize = 0;

    let mut i = 0;
    while i < seq.len() {
        let (l, r) = seq[i];

        // Right side: tighten when the new right point is on the
        // **left** (or on) the current right ray — i.e. it shrinks
        // the funnel from the right. Mononen's reference convention.
        if triangle_area2(apex, right, r) >= 0.0 {
            if apex == right || triangle_area2(apex, left, r) < 0.0 {
                right = r;
                right_idx = i;
            } else {
                // Right crossed over left: emit left as new apex.
                out.push(left);
                apex = left;
                right = apex;
                left = apex;
                i = left_idx + 1;
                left_idx = i;
                right_idx = i;
                continue;
            }
        }

        // Left side: tighten when the new left point is on the
        // **right** (or on) the current left ray — i.e. it shrinks
        // the funnel from the left.
        if triangle_area2(apex, left, l) <= 0.0 {
            if apex == left || triangle_area2(apex, right, l) > 0.0 {
                left = l;
                left_idx = i;
            } else {
                // Left crossed over right: emit right as new apex.
                out.push(right);
                apex = right;
                left = apex;
                right = apex;
                i = right_idx + 1;
                left_idx = i;
                right_idx = i;
                continue;
            }
        }
        i += 1;
    }

    // Final segment from the last emitted apex to `end`.
    if out.last() != Some(&end) {
        out.push(end);
    }
    out
}

/// Signed 2× area of triangle `(a, b, c)`. Positive when `c` is on the
/// left of the directed segment `a → b`, negative when on the right,
/// zero when collinear.
fn triangle_area2(a: Position, b: Position, c: Position) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn pos_to_bucket(pos: Position, bucket_nm: f32) -> (i32, i32) {
    (
        (pos.x / bucket_nm).floor() as i32,
        (pos.y / bucket_nm).floor() as i32,
    )
}

fn build_centroid_buckets(tiles: &[Tile], bucket_nm: f32) -> HashMap<(i32, i32), Vec<u32>> {
    let mut m: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (i, t) in tiles.iter().enumerate() {
        m.entry(pos_to_bucket(t.centroid, bucket_nm))
            .or_default()
            .push(i as u32);
    }
    m
}

fn read_u32(buf: &[u8], cur: &mut usize) -> io::Result<u32> {
    let end = *cur + 4;
    if end > buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated navmesh.bin: expected u32",
        ));
    }
    let v = u32::from_le_bytes(buf[*cur..end].try_into().unwrap());
    *cur = end;
    Ok(v)
}

fn read_f32(buf: &[u8], cur: &mut usize) -> io::Result<f32> {
    let end = *cur + 4;
    if end > buf.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated navmesh.bin: expected f32",
        ));
    }
    let v = f32::from_le_bytes(buf[*cur..end].try_into().unwrap());
    *cur = end;
    Ok(v)
}

// ----- A* scratch (mirrors navmesh.rs / portroutes.rs pattern) -----

struct RouteScratch {
    g_score: Vec<f32>,
    came_from: Vec<u32>,
    in_goal: Vec<bool>,
    open: BinaryHeap<HeapEntry>,
    touched: Vec<u32>,
    touched_goals: Vec<u32>,
}

impl RouteScratch {
    fn new() -> Self {
        Self {
            g_score: Vec::new(),
            came_from: Vec::new(),
            in_goal: Vec::new(),
            open: BinaryHeap::new(),
            touched: Vec::new(),
            touched_goals: Vec::new(),
        }
    }

    fn reset_and_size(&mut self, n_tiles: usize) {
        if self.g_score.len() < n_tiles {
            self.g_score.resize(n_tiles, f32::INFINITY);
            self.came_from.resize(n_tiles, u32::MAX);
            self.in_goal.resize(n_tiles, false);
        }
        for &i in &self.touched {
            let idx = i as usize;
            self.g_score[idx] = f32::INFINITY;
            self.came_from[idx] = u32::MAX;
        }
        self.touched.clear();
        for &i in &self.touched_goals {
            self.in_goal[i as usize] = false;
        }
        self.touched_goals.clear();
        self.open.clear();
    }
}

thread_local! {
    static ROUTE_SCRATCH: RefCell<RouteScratch> = RefCell::new(RouteScratch::new());
}

#[derive(Copy, Clone, PartialEq)]
struct HeapEntry {
    f: f32,
    idx: u32,
}
impl Eq for HeapEntry {}
impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny 3-tile mesh by hand (no file I/O) to exercise the
    /// graph + bucket + A* without depending on `navmesh.bin`. Layout:
    ///
    /// ```text
    ///   T0 ── portal_01 ── T1 ── portal_12 ── T2
    /// ```
    fn linear_three_tile_mesh() -> TileMesh {
        let tiles = vec![
            Tile {
                vertices: vec![
                    Position::new(0.0, 0.0),
                    Position::new(10.0, 0.0),
                    Position::new(10.0, 10.0),
                    Position::new(0.0, 10.0),
                ],
                centroid: Position::new(5.0, 5.0),
                bbox_min: Position::new(0.0, 0.0),
                bbox_max: Position::new(10.0, 10.0),
            },
            Tile {
                vertices: vec![
                    Position::new(10.0, 0.0),
                    Position::new(20.0, 0.0),
                    Position::new(20.0, 10.0),
                    Position::new(10.0, 10.0),
                ],
                centroid: Position::new(15.0, 5.0),
                bbox_min: Position::new(10.0, 0.0),
                bbox_max: Position::new(20.0, 10.0),
            },
            Tile {
                vertices: vec![
                    Position::new(20.0, 0.0),
                    Position::new(30.0, 0.0),
                    Position::new(30.0, 10.0),
                    Position::new(20.0, 10.0),
                ],
                centroid: Position::new(25.0, 5.0),
                bbox_min: Position::new(20.0, 0.0),
                bbox_max: Position::new(30.0, 10.0),
            },
        ];
        let portal_01 = Position::new(10.0, 5.0);
        let portal_12 = Position::new(20.0, 5.0);
        let neighbors = vec![
            vec![TileEdge {
                to: 1,
                portal: portal_01,
                dist_nm: tiles[0].centroid.distance(tiles[1].centroid),
            }],
            vec![
                TileEdge {
                    to: 0,
                    portal: portal_01,
                    dist_nm: tiles[1].centroid.distance(tiles[0].centroid),
                },
                TileEdge {
                    to: 2,
                    portal: portal_12,
                    dist_nm: tiles[1].centroid.distance(tiles[2].centroid),
                },
            ],
            vec![TileEdge {
                to: 1,
                portal: portal_12,
                dist_nm: tiles[2].centroid.distance(tiles[1].centroid),
            }],
        ];
        let centroid_buckets = build_centroid_buckets(&tiles, BUCKET_NM);
        TileMesh {
            tiles,
            neighbors,
            shortcut_neighbors: Vec::new(),
            centroid_buckets,
            clearance_nm: Vec::new(),
        }
    }

    #[test]
    fn nearest_centroids_orders_by_distance() {
        let m = linear_three_tile_mesh();
        let out = m.nearest_centroids(Position::new(8.0, 5.0), 30.0);
        assert!(!out.is_empty());
        // Closest centroid to (8,5) is T0 (5,5) at d=3.
        assert_eq!(out[0].0, 0);
        // Next closest is T1 (15,5) at d=7.
        assert_eq!(out[1].0, 1);
    }

    #[test]
    fn route_finds_three_tile_chain() {
        let m = linear_three_tile_mesh();
        let path = m.route(&[0], &[2]).expect("path exists");
        assert_eq!(path, vec![0, 1, 2]);
    }

    #[test]
    fn route_same_tile_returns_singleton() {
        let m = linear_three_tile_mesh();
        let path = m.route(&[1], &[1]).expect("trivial path");
        assert_eq!(path, vec![1]);
    }

    #[test]
    fn portal_between_returns_midpoint() {
        let m = linear_three_tile_mesh();
        let p = m.portal_between(0, 1).expect("adjacent");
        assert_eq!(p, Position::new(10.0, 5.0));
        assert!(m.portal_between(0, 2).is_none(), "not adjacent");
    }

    #[test]
    fn shared_edge_orients_left_right_by_ray() {
        // Tiles 0 and 1 share the vertical edge at x=10, from (10,0) to
        // (10,10). Looking from T0 (centroid 5,5) into T1 (centroid
        // 15,5), the ray points +x; "left" is +y, "right" is -y. So
        // left = (10,10), right = (10,0).
        let m = linear_three_tile_mesh();
        let (l, r) = m.shared_edge(0, 1).expect("adjacent");
        assert_eq!(l, Position::new(10.0, 10.0));
        assert_eq!(r, Position::new(10.0, 0.0));
        // Reverse direction flips left/right.
        let (l, r) = m.shared_edge(1, 0).expect("adjacent");
        assert_eq!(l, Position::new(10.0, 0.0));
        assert_eq!(r, Position::new(10.0, 10.0));
    }

    #[test]
    fn shared_edge_none_for_non_adjacent() {
        let m = linear_three_tile_mesh();
        assert!(m.shared_edge(0, 2).is_none());
    }

    #[test]
    fn funnel_no_portals_is_direct_to_end() {
        let out = funnel(Position::new(0.0, 0.0), Position::new(10.0, 0.0), &[]);
        assert_eq!(out, vec![Position::new(10.0, 0.0)]);
    }

    #[test]
    fn funnel_straight_corridor_passes_through() {
        // Three-tile straight corridor as in `linear_three_tile_mesh`.
        // Start at (2,5) in T0, end at (28,5) in T2. The corridor is
        // open all the way, so the smoothed path is a single segment
        // straight to the end.
        let portals = vec![
            (Position::new(10.0, 10.0), Position::new(10.0, 0.0)),
            (Position::new(20.0, 10.0), Position::new(20.0, 0.0)),
        ];
        let out = funnel(Position::new(2.0, 5.0), Position::new(28.0, 5.0), &portals);
        assert_eq!(out, vec![Position::new(28.0, 5.0)]);
    }

    #[test]
    fn funnel_bend_emits_apex_at_inside_corner() {
        // Two-portal L-shape: corridor goes east, then turns north.
        // Start (0,5), goes through portal1 at edge x=10 (right corner
        // (10,0), left corner (10,10)), then portal2 at edge y=10
        // through a north tile with corners (10,10) and (20,10).
        // End at (15,20). The inside corner is (10,10); funnel should
        // emit it as a waypoint.
        let portals = vec![
            (Position::new(10.0, 10.0), Position::new(10.0, 0.0)),
            (Position::new(20.0, 10.0), Position::new(10.0, 10.0)),
        ];
        let out = funnel(Position::new(0.0, 5.0), Position::new(15.0, 20.0), &portals);
        // Expect the inside-corner waypoint then the end.
        assert!(out.contains(&Position::new(10.0, 10.0)));
        assert_eq!(*out.last().unwrap(), Position::new(15.0, 20.0));
    }

    #[test]
    fn load_real_navmesh_bin() {
        // Skip silently if the bundled mesh is not present (CI fork
        // checkouts without LFS or preprocessed grids).
        let path = Path::new("../../data/grids/navmesh.bin");
        if !path.exists() {
            eprintln!("skipping load_real_navmesh_bin: {path:?} missing");
            return;
        }
        let mesh = TileMesh::load(path).expect("load navmesh.bin");
        assert!(
            mesh.len() > 1000,
            "expected thousands of tiles, got {}",
            mesh.len()
        );
        // Sanity: every neighbor reference is in range, every portal
        // is finite, every tile has at least one neighbor (largest
        // component already filtered offline).
        for (i, nbrs) in mesh.neighbors.iter().enumerate() {
            assert!(
                !nbrs.is_empty(),
                "tile {i} has no neighbors in largest component"
            );
            for e in nbrs {
                assert!((e.to as usize) < mesh.len());
                assert!(e.portal.x.is_finite() && e.portal.y.is_finite());
                assert!(e.dist_nm.is_finite() && e.dist_nm > 0.0);
            }
        }
    }
}
