//! Tier-1 global geodesic mesh: a subdivided icosahedron used for
//! whole-planet land/sea topology, reachability, and long-haul routing.
//!
//! This is the coarse global substrate described in
//! `planning/development-log.md` (global map expansion) — it does *not*
//! replace the fine-grained flat-plane `LandMap`/`CoastlineGeom`/`TileMesh`
//! navmesh used for in-chart ship physics and coastal pathfinding. Those
//! stay exactly as they are, scoped per regional `Chart`. This mesh exists
//! only for: coarse land/sea classification, connectivity flood-fill,
//! river-edge bookkeeping, and anchoring ports/regions to a coarse
//! shipping-lane graph.
//!
//! Construction mirrors `garret-pirates/src/geodesic.js`: start from a
//! regular icosahedron (12 vertices, 20 faces), subdivide each triangular
//! face into 4 (the "4T" scheme) `subdivisions` times with a shared-edge
//! midpoint cache so adjacent faces don't duplicate vertices, then treat
//! the resulting **vertices** as tiles (the dual of the triangulation is a
//! hexagon/pentagon tiling — this is the standard geodesic-globe trick).
//! Tile count follows `10 * 4^subdivisions + 2`; exactly 12 tiles (the
//! original icosahedron corners) are pentagons (degree 5), all others are
//! hexagons (degree 6).

use std::collections::HashMap;

use arrayvec::ArrayVec;
use glam::Vec3;

/// Index into [`GeoMesh`]'s per-tile arrays.
pub type TileId = u32;

/// Canonical subdivision level for the Tier-1 mesh used by the data
/// pipeline (`tools/preprocess/preprocess_geo_tiles.py`) and by default in
/// `sim-globe-view`: 40,962 tiles, ~70 km edge length. Fine enough to
/// resolve major gulfs/rivers/basins for connectivity and rendering,
/// far cheaper than a per-hull-scale mesh (that precision instead lives in
/// the flat-plane regional `Chart`s — see module docs).
pub const DEFAULT_SUBDIVISIONS: u32 = 6;

/// A whole-planet geodesic tile graph. See module docs.
pub struct GeoMesh {
    pub subdivisions: u32,
    /// Unit-sphere direction of each tile's center.
    pub centers: Vec<Vec3>,
    pub lat_deg: Vec<f32>,
    pub lon_deg: Vec<f32>,
    /// Ordered (rotational) adjacency ring per tile. Length is 5 for the
    /// 12 pentagon tiles, 6 for all others. Position in this array is the
    /// tile's "edge index" toward that neighbor — used later to tag river
    /// edges (`edge index E of tile T carries a river`).
    pub neighbors: Vec<ArrayVec<TileId, 6>>,
}

/// Expected tile count for a given subdivision level: `10 * 4^n + 2`.
pub fn expected_tile_count(subdivisions: u32) -> usize {
    10 * 4usize.pow(subdivisions) + 2
}

/// Convert a (lat, lon) in degrees to a unit direction vector, using the
/// same convention as `garret-pirates`' `latLonToDirection`: `x =
/// cos(lat)cos(lon)`, `y = sin(lat)`, `z = -cos(lat)sin(lon)`. Kept
/// consistent so lat/lon fixtures and reasoning port directly.
pub fn latlon_to_dir(lat_deg: f32, lon_deg: f32) -> Vec3 {
    let lat = lat_deg.to_radians();
    let lon = lon_deg.to_radians();
    let (sin_lat, cos_lat) = lat.sin_cos();
    let (sin_lon, cos_lon) = lon.sin_cos();
    Vec3::new(cos_lat * cos_lon, sin_lat, -cos_lat * sin_lon)
}

/// Inverse of [`latlon_to_dir`]: unit direction vector to (lat, lon) in
/// degrees. `dir` need not be pre-normalized.
pub fn dir_to_latlon(dir: Vec3) -> (f32, f32) {
    let dir = dir.normalize();
    let lat = dir.y.asin().to_degrees();
    let lon = (-dir.z).atan2(dir.x).to_degrees();
    (lat, lon)
}

impl GeoMesh {
    pub fn tile_count(&self) -> usize {
        self.centers.len()
    }

    pub fn is_pentagon(&self, tile: TileId) -> bool {
        self.neighbors[tile as usize].len() == 5
    }

    /// Edge index of `tile`'s boundary that faces `neighbor`, if adjacent.
    pub fn edge_towards(&self, tile: TileId, neighbor: TileId) -> Option<u8> {
        self.neighbors[tile as usize]
            .iter()
            .position(|&n| n == neighbor)
            .map(|i| i as u8)
    }

    /// The tile's boundary polygon (5 or 6 corners, unit-sphere, in
    /// rotational order matching `neighbors`) — the Goldberg-polyhedron
    /// dual of the underlying triangulation. Corner `i` sits between
    /// `neighbors[i]` and `neighbors[i+1]`, at the (normalized) centroid
    /// of the triangular face `(tile, neighbors[i], neighbors[i+1])`.
    /// Because that face is shared by exactly those three tiles, each
    /// corner point is bit-for-bit identical across all three tiles'
    /// polygons — the resulting tiling of the sphere is watertight, no
    /// separate stitching pass required. Used by `sim-globe-view` to
    /// render an actual 3D globe rather than point/edge wireframe.
    pub fn tile_polygon(&self, tile: TileId) -> ArrayVec<Vec3, 6> {
        let ring = &self.neighbors[tile as usize];
        let center = self.centers[tile as usize];
        let deg = ring.len();
        let mut corners = ArrayVec::new();
        for i in 0..deg {
            let a = self.centers[ring[i] as usize];
            let b = self.centers[ring[(i + 1) % deg] as usize];
            corners.push(((center + a + b) / 3.0).normalize());
        }
        corners
    }

    pub fn build(subdivisions: u32) -> Self {
        let (verts, faces) = build_subdivided_icosahedron(subdivisions);
        let neighbors = build_ordered_adjacency(&verts, &faces);

        let mut lat_deg = Vec::with_capacity(verts.len());
        let mut lon_deg = Vec::with_capacity(verts.len());
        for v in &verts {
            let (lat, lon) = dir_to_latlon(*v);
            lat_deg.push(lat);
            lon_deg.push(lon);
        }

        Self {
            subdivisions,
            centers: verts,
            lat_deg,
            lon_deg,
            neighbors,
        }
    }
}

const T_CONST: f32 = 1.618_034; // golden ratio, (1 + sqrt(5)) / 2

const ICOSA_FACES: [[u32; 3]; 20] = [
    [0, 11, 5],
    [0, 5, 1],
    [0, 1, 7],
    [0, 7, 10],
    [0, 10, 11],
    [1, 5, 9],
    [5, 11, 4],
    [11, 10, 2],
    [10, 7, 6],
    [7, 1, 8],
    [3, 9, 4],
    [3, 4, 2],
    [3, 2, 6],
    [3, 6, 8],
    [3, 8, 9],
    [4, 9, 5],
    [2, 4, 11],
    [6, 2, 10],
    [8, 6, 7],
    [9, 8, 1],
];

fn icosahedron_vertices() -> Vec<Vec3> {
    let raw: [[f32; 3]; 12] = [
        [-1.0, T_CONST, 0.0],
        [1.0, T_CONST, 0.0],
        [-1.0, -T_CONST, 0.0],
        [1.0, -T_CONST, 0.0],
        [0.0, -1.0, T_CONST],
        [0.0, 1.0, T_CONST],
        [0.0, -1.0, -T_CONST],
        [0.0, 1.0, -T_CONST],
        [T_CONST, 0.0, -1.0],
        [T_CONST, 0.0, 1.0],
        [-T_CONST, 0.0, -1.0],
        [-T_CONST, 0.0, 1.0],
    ];
    raw.iter()
        .map(|c| Vec3::new(c[0], c[1], c[2]).normalize())
        .collect()
}

/// Builds the subdivided icosahedron: returns (vertices, faces). Faces
/// remain CCW-wound (as viewed from outside the sphere) at every
/// subdivision level, since each subdivision round preserves winding.
fn build_subdivided_icosahedron(subdivisions: u32) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let mut verts = icosahedron_vertices();
    let mut faces: Vec<[u32; 3]> = ICOSA_FACES.to_vec();

    for _ in 0..subdivisions {
        let mut mid_cache: HashMap<(u32, u32), u32> = HashMap::new();
        let mut next_faces = Vec::with_capacity(faces.len() * 4);

        let mut get_mid = |verts: &mut Vec<Vec3>, i: u32, j: u32| -> u32 {
            let key = if i < j { (i, j) } else { (j, i) };
            if let Some(&id) = mid_cache.get(&key) {
                return id;
            }
            let mid = ((verts[i as usize] + verts[j as usize]) * 0.5).normalize();
            let id = verts.len() as u32;
            verts.push(mid);
            mid_cache.insert(key, id);
            id
        };

        for face in &faces {
            let [a, b, c] = *face;
            let ab = get_mid(&mut verts, a, b);
            let bc = get_mid(&mut verts, b, c);
            let ca = get_mid(&mut verts, c, a);
            next_faces.push([a, ab, ca]);
            next_faces.push([ab, b, bc]);
            next_faces.push([ca, bc, c]);
            next_faces.push([ab, bc, ca]);
        }

        faces = next_faces;
    }

    (verts, faces)
}

/// Builds the ordered (rotational) adjacency ring for every vertex, using
/// the standard "next around vertex" trick: for a CCW face `(p0, p1,
/// p2)`, going around `p0` from `p1` leads next to `p2` (and cyclically
/// for `p1`, `p2`). Storing this as a `next[(vertex, from)] = to` map lets
/// each vertex's full rotational ring be walked in O(degree) without any
/// search, and guarantees the result is symmetric and duplicate-free for
/// a valid closed triangulation.
fn build_ordered_adjacency(verts: &[Vec3], faces: &[[u32; 3]]) -> Vec<ArrayVec<TileId, 6>> {
    let mut next: HashMap<(u32, u32), u32> = HashMap::with_capacity(faces.len() * 3);
    for face in faces {
        let [p0, p1, p2] = *face;
        next.insert((p0, p1), p2);
        next.insert((p1, p2), p0);
        next.insert((p2, p0), p1);
    }

    let mut start_neighbor: Vec<Option<u32>> = vec![None; verts.len()];
    for face in faces {
        for &v in face {
            start_neighbor[v as usize]
                .get_or_insert_with(|| face.iter().copied().find(|&x| x != v).unwrap());
        }
    }

    let mut neighbors = Vec::with_capacity(verts.len());
    for v in 0..verts.len() as u32 {
        let mut ring: ArrayVec<TileId, 6> = ArrayVec::new();
        let start = start_neighbor[v as usize].expect("every vertex belongs to a face");
        let mut cur = start;
        loop {
            ring.push(cur);
            let nxt = *next
                .get(&(v, cur))
                .expect("closed triangulation: every (vertex, neighbor) has a next");
            if nxt == start {
                break;
            }
            cur = nxt;
        }
        neighbors.push(ring);
    }
    neighbors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_count_matches_formula() {
        for n in 0..5u32 {
            let mesh = GeoMesh::build(n);
            assert_eq!(mesh.tile_count(), expected_tile_count(n), "n={n}");
        }
    }

    #[test]
    fn adjacency_is_symmetric_and_duplicate_free() {
        for n in 0..4u32 {
            let mesh = GeoMesh::build(n);
            for (tile, ring) in mesh.neighbors.iter().enumerate() {
                let tile = tile as TileId;
                // no duplicates
                let mut sorted = ring.clone().to_vec();
                sorted.sort_unstable();
                sorted.dedup();
                assert_eq!(
                    sorted.len(),
                    ring.len(),
                    "tile {tile} has duplicate neighbors"
                );

                for &neighbor in ring {
                    assert!(
                        mesh.neighbors[neighbor as usize].contains(&tile),
                        "tile {tile} lists {neighbor} but not vice versa (n={n})"
                    );
                }
            }
        }
    }

    #[test]
    fn degree_is_five_or_six_with_exactly_twelve_pentagons() {
        for n in 0..5u32 {
            let mesh = GeoMesh::build(n);
            let mut pentagons = 0;
            for ring in &mesh.neighbors {
                assert!(
                    ring.len() == 5 || ring.len() == 6,
                    "unexpected degree {} (n={n})",
                    ring.len()
                );
                if ring.len() == 5 {
                    pentagons += 1;
                }
            }
            assert_eq!(pentagons, 12, "n={n}");
        }
    }

    #[test]
    fn latlon_direction_round_trips() {
        let cases = [(0.0, 0.0), (45.0, -30.0), (-60.0, 170.0), (89.0, 0.0)];
        for (lat, lon) in cases {
            let dir = latlon_to_dir(lat, lon);
            let (lat2, lon2) = dir_to_latlon(dir);
            assert!((lat - lat2).abs() < 1e-3, "lat {lat} vs {lat2}");
            assert!((lon - lon2).abs() < 1e-3, "lon {lon} vs {lon2}");
        }
    }

    #[test]
    fn centers_stay_on_unit_sphere() {
        let mesh = GeoMesh::build(4);
        for c in &mesh.centers {
            assert!((c.length() - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn tile_polygons_are_watertight() {
        let mesh = GeoMesh::build(3);
        for tile in 0..mesh.tile_count() as TileId {
            let corners = mesh.tile_polygon(tile);
            let deg = mesh.neighbors[tile as usize].len();
            assert_eq!(corners.len(), deg);
            for c in &corners {
                assert!((c.length() - 1.0).abs() < 1e-4);
            }
            // Corner `i` sits between the edges facing neighbors[i-1] and
            // neighbors[i]; both must also be corners of that neighbor's
            // own polygon, or the tiling would have cracks.
            for &neighbor in &mesh.neighbors[tile as usize] {
                let edge = mesh.edge_towards(tile, neighbor).unwrap() as usize;
                let neighbor_corners = mesh.tile_polygon(neighbor);
                for idx in [edge, (edge + deg - 1) % deg] {
                    let c = corners[idx];
                    assert!(
                        neighbor_corners.iter().any(|nc| nc.distance(c) < 1e-4),
                        "tile {tile} corner {idx} not found in neighbor {neighbor}'s polygon"
                    );
                }
            }
        }
    }
}
