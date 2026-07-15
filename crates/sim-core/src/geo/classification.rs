//! Land/sea classification for the Tier-1 [`GeoMesh`], baked offline by
//! `tools/preprocess/preprocess_geo_tiles.py` from Natural Earth land
//! polygons (point-in-polygon at each tile center — plenty precise at
//! ~40-70 km tile resolution; this is coarse global topology, not the
//! fine coastline used for in-chart ship physics).
//!
//! Binary format (`data/grids/geo_tiles.bin`), little-endian:
//! ```text
//! u32  subdivisions
//! u32  tile_count
//! u8 × tile_count   // 1 = land, 0 = sea, indexed by TileId
//! ```
//! `subdivisions`/`tile_count` are stored so a mismatched mesh (built with
//! a different `DEFAULT_SUBDIVISIONS`) fails loudly instead of silently
//! misaligning classification to the wrong tiles.

use std::path::Path;

use super::mesh::TileId;

pub struct LandSea {
    pub subdivisions: u32,
    pub is_land: Vec<bool>,
}

impl LandSea {
    pub fn load(path: &Path) -> Self {
        let bytes = std::fs::read(path).unwrap_or_else(|e| {
            panic!(
                "Failed to load land/sea classification from {}: {}",
                path.display(),
                e
            )
        });
        assert!(bytes.len() >= 8, "geo_tiles.bin too small");
        let subdivisions = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let tile_count = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(
            bytes.len() - 8,
            tile_count,
            "geo_tiles.bin data size mismatch: header says {} tiles, got {} bytes",
            tile_count,
            bytes.len() - 8
        );
        let is_land = bytes[8..].iter().map(|&b| b != 0).collect();
        Self {
            subdivisions,
            is_land,
        }
    }

    pub fn is_land(&self, tile: TileId) -> bool {
        self.is_land[tile as usize]
    }
}
