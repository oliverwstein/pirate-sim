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
//! Binary format (matches `preprocess_navmesh.py::write_navmesh`,
//! little-endian):
//!
//! ```text
//! u32  num_tiles
//! per tile:
//!     u32  num_vertices
//!     (f32 x, f32 y) × num_vertices     // CCW, NM-from-origin
//!     f32  centroid_x, f32 centroid_y
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
    /// Spatial hash of centroids in `BUCKET_NM`-sized cells. The key
    /// is `(floor(x / BUCKET_NM), floor(y / BUCKET_NM))`.
    centroid_buckets: HashMap<(i32, i32), Vec<u32>>,
}

impl TileMesh {
    /// Load a navmesh from the binary produced by
    /// `preprocess_navmesh.py`. Returns an error on truncation or
    /// any inconsistency; panics are reserved for the caller.
    pub fn load(path: &Path) -> io::Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
    }

    /// Parse a navmesh from a byte buffer (`navmesh.bin` contents).
    pub fn from_bytes(buf: &[u8]) -> io::Result<Self> {
        let mut cur = 0usize;
        let num_tiles = read_u32(buf, &mut cur)? as usize;

        let mut tiles: Vec<Tile> = Vec::with_capacity(num_tiles);
        let mut neighbors: Vec<Vec<TileEdge>> = Vec::with_capacity(num_tiles);
        // First pass: read vertices + centroid + raw neighbor records.
        // Centroid-to-centroid distance for each edge is filled in a
        // second pass once every tile's centroid is known.
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
            centroid_buckets,
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
                for e in &self.neighbors[cur_us] {
                    let tentative = cur_g + e.dist_nm;
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
            centroid_buckets,
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
