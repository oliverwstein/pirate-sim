use std::path::Path;

use crate::types::Position;

/// Binary land/sea mask loaded from preprocessed GEBCO data.
///
/// File format:
///   width: u32 (little-endian)
///   height: u32
///   origin_x: f32 (NW corner, nautical miles)
///   origin_y: f32
///   cell_size: f32 (nautical miles per cell)
///   data: [u8; width * height] (row-major, top-to-bottom = north-to-south)
///     0 = sea, 255 = land
pub struct LandMap {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub origin: Position, // NW corner in NM
    pub cell_size_nm: f32,
}

impl LandMap {
    pub fn load(path: &Path) -> Self {
        let bytes = std::fs::read(path)
            .unwrap_or_else(|e| panic!("Failed to load land mask from {}: {}", path.display(), e));

        assert!(bytes.len() >= 20, "Land mask file too small");

        let width = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let height = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let origin_x = f32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let origin_y = f32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let cell_size = f32::from_le_bytes(bytes[16..20].try_into().unwrap());

        let expected_data_len = (width * height) as usize;
        assert_eq!(
            bytes.len() - 20,
            expected_data_len,
            "Land mask data size mismatch: expected {} bytes, got {}",
            expected_data_len,
            bytes.len() - 20
        );

        let data = bytes[20..].to_vec();

        Self {
            data,
            width,
            height,
            origin: Position::new(origin_x, origin_y),
            cell_size_nm: cell_size,
        }
    }

    /// Convert world position to grid cell indices. Returns None if out of bounds.
    fn pos_to_cell(&self, pos: Position) -> Option<(u32, u32)> {
        let dx = pos.x - self.origin.x;
        let dy = self.origin.y - pos.y; // Y flipped: origin is NW, Y increases southward in grid

        let col = (dx / self.cell_size_nm) as i32;
        let row = (dy / self.cell_size_nm) as i32;

        if col < 0 || row < 0 || col >= self.width as i32 || row >= self.height as i32 {
            None
        } else {
            Some((col as u32, row as u32))
        }
    }

    /// Returns true if the position is on land (or out of bounds = treated as land).
    pub fn is_land(&self, pos: Position) -> bool {
        match self.pos_to_cell(pos) {
            Some((col, row)) => {
                let idx = (row * self.width + col) as usize;
                self.data[idx] == 255
            }
            None => true, // out of bounds = impassable
        }
    }

    /// Grid dimensions in world space.
    pub fn world_width(&self) -> f32 {
        self.width as f32 * self.cell_size_nm
    }

    pub fn world_height(&self) -> f32 {
        self.height as f32 * self.cell_size_nm
    }
}
