//! Coarse lon/lat bucket spatial index over a [`GeoMesh`], for
//! `nearest_tile` lookups (port anchoring, chart-boundary checks).
//!
//! Mirrors `garret-pirates/src/geodesic.js`'s `createDirectionIndex` /
//! `findNearestTileId`: a uniform lon/lat grid holds a bucket list of tile
//! ids whose center falls in that cell, and lookups expand outward ring by
//! ring from the query's cell until a candidate is found, then confirm it
//! by max dot-product (closest angular distance) among everything in the
//! searched rings. This is purely an acceleration structure — the
//! [`GeoMesh`] adjacency graph remains the source of truth for topology.

use super::mesh::{GeoMesh, TileId};
use glam::Vec3;

/// Default bucket size in degrees (matches Garrett's 2.5° choice).
pub const DEFAULT_LON_CELLS: u32 = 144; // 360 / 2.5
pub const DEFAULT_LAT_CELLS: u32 = 72; // 180 / 2.5

pub struct DirectionIndex {
    lon_cells: u32,
    lat_cells: u32,
    buckets: Vec<Vec<TileId>>,
}

impl DirectionIndex {
    pub fn build(mesh: &GeoMesh, lon_cells: u32, lat_cells: u32) -> Self {
        let mut buckets = vec![Vec::new(); (lon_cells * lat_cells) as usize];
        for (tile, (&lat, &lon)) in mesh.lat_deg.iter().zip(mesh.lon_deg.iter()).enumerate() {
            let (cx, cy) = Self::cell_coords(lat, lon, lon_cells, lat_cells);
            buckets[(cy * lon_cells + cx) as usize].push(tile as TileId);
        }
        Self {
            lon_cells,
            lat_cells,
            buckets,
        }
    }

    fn cell_coords(lat_deg: f32, lon_deg: f32, lon_cells: u32, lat_cells: u32) -> (u32, u32) {
        // lon in [-180, 180) -> [0, lon_cells); lat in [-90, 90] -> [0, lat_cells)
        let lon_norm = ((lon_deg + 180.0).rem_euclid(360.0)) / 360.0;
        let lat_norm = ((lat_deg + 90.0).clamp(0.0, 180.0)) / 180.0;
        let cx = ((lon_norm * lon_cells as f32) as u32).min(lon_cells - 1);
        let cy = ((lat_norm * lat_cells as f32) as u32).min(lat_cells - 1);
        (cx, cy)
    }

    /// Nearest tile to a (not necessarily normalized) direction, by
    /// expanding-ring bucket search then max-dot-product confirmation
    /// among all candidates found within the searched rings.
    pub fn nearest_tile(&self, mesh: &GeoMesh, dir: Vec3) -> TileId {
        let dir = dir.normalize();
        let (lat, lon) = super::mesh::dir_to_latlon(dir);
        let (cx, cy) = Self::cell_coords(lat, lon, self.lon_cells, self.lat_cells);

        for radius in 0..=self.lon_cells.max(self.lat_cells) {
            let mut best: Option<(TileId, f32)> = None;
            for dy in -(radius as i32)..=(radius as i32) {
                let ry = cy as i32 + dy;
                if ry < 0 || ry >= self.lat_cells as i32 {
                    continue;
                }
                for dx in -(radius as i32)..=(radius as i32) {
                    // only the outer ring at this radius (interior already
                    // scanned at smaller radii)
                    if dx.abs() != radius as i32 && dy.abs() != radius as i32 {
                        continue;
                    }
                    let rx = (cx as i32 + dx).rem_euclid(self.lon_cells as i32);
                    for &tile in &self.buckets[(ry as u32 * self.lon_cells + rx as u32) as usize] {
                        let dot = mesh.centers[tile as usize].dot(dir);
                        if best.is_none_or(|(_, best_dot)| dot > best_dot) {
                            best = Some((tile, dot));
                        }
                    }
                }
            }
            if let Some((tile, _)) = best {
                return tile;
            }
        }

        // Fallback: full linear scan (should be unreachable for a
        // populated mesh, but keeps this function total).
        (0..mesh.centers.len())
            .max_by(|&a, &b| {
                mesh.centers[a]
                    .dot(dir)
                    .partial_cmp(&mesh.centers[b].dot(dir))
                    .unwrap()
            })
            .expect("mesh has at least one tile") as TileId
    }
}

impl Default for DirectionIndex {
    fn default() -> Self {
        Self {
            lon_cells: DEFAULT_LON_CELLS,
            lat_cells: DEFAULT_LAT_CELLS,
            buckets: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::mesh::latlon_to_dir;

    #[test]
    fn nearest_tile_finds_exact_center_for_its_own_tile() {
        let mesh = GeoMesh::build(5);
        let index = DirectionIndex::build(&mesh, DEFAULT_LON_CELLS, DEFAULT_LAT_CELLS);
        for tile in 0..mesh.tile_count() as TileId {
            let dir = mesh.centers[tile as usize];
            assert_eq!(index.nearest_tile(&mesh, dir), tile);
        }
    }

    #[test]
    fn nearest_tile_is_close_for_arbitrary_points() {
        let mesh = GeoMesh::build(6);
        let index = DirectionIndex::build(&mesh, DEFAULT_LON_CELLS, DEFAULT_LAT_CELLS);
        // A handful of real port coordinates (lat, lon) from ports.ron.
        let ports = [
            (23.1450, -82.3600), // Havana
            (51.5074, -0.1278),  // London (approx)
            (52.3676, 4.9041),   // Amsterdam (approx)
        ];
        for (lat, lon) in ports {
            let dir = latlon_to_dir(lat, lon);
            let tile = index.nearest_tile(&mesh, dir);
            let (tlat, tlon) = (mesh.lat_deg[tile as usize], mesh.lon_deg[tile as usize]);
            // At n=6 tile edge length is ~70km => resolved tile center
            // should be within a few degrees of the query.
            assert!((tlat - lat).abs() < 3.0, "lat {lat} -> tile lat {tlat}");
            assert!(
                (tlon - lon).abs() < 3.0 || (tlon - lon).abs() > 357.0,
                "lon {lon} -> tile lon {tlon}"
            );
        }
    }
}
