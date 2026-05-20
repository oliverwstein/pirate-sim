use std::path::Path;

use crate::types::{Position, WindVector};

/// Monthly wind climatology loaded from preprocessed ERA5 data.
///
/// File format:
///   width: u32 (little-endian)
///   height: u32
///   origin_x: f32 (NW corner, nautical miles)
///   origin_y: f32
///   cell_size: f32 (nautical miles per cell)
///   months: u8
///   padding: [u8; 3]
///   u_data: [f32; months * height * width] (knots, row-major, month-major)
///   v_data: [f32; months * height * width]
pub struct WindGrid {
    u_data: Vec<f32>,
    v_data: Vec<f32>,
    pub width: u32,
    pub height: u32,
    pub origin: Position,
    pub cell_size_nm: f32,
    pub months: u8,
}

impl WindGrid {
    pub fn load(path: &Path) -> Self {
        let bytes = std::fs::read(path)
            .unwrap_or_else(|e| panic!("Failed to load wind grid from {}: {}", path.display(), e));

        assert!(bytes.len() >= 24, "Wind grid file too small");

        let width = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let height = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let origin_x = f32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let origin_y = f32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let cell_size = f32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let months = bytes[20];
        // bytes[21..24] = padding

        let grid_size = (months as usize) * (height as usize) * (width as usize);
        let header_size = 24;
        let expected_size = header_size + grid_size * 4 * 2; // u + v, each f32
        assert_eq!(
            bytes.len(),
            expected_size,
            "Wind grid size mismatch: expected {}, got {}",
            expected_size,
            bytes.len()
        );

        let float_data = &bytes[header_size..];
        let u_bytes = &float_data[..grid_size * 4];
        let v_bytes = &float_data[grid_size * 4..];

        let u_data: Vec<f32> = u_bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let v_data: Vec<f32> = v_bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();

        Self {
            u_data,
            v_data,
            width,
            height,
            origin: Position::new(origin_x, origin_y),
            cell_size_nm: cell_size,
            months,
        }
    }

    /// Bilinear interpolation of wind at a position for a given month.
    pub fn wind_at(&self, pos: Position, month: u8) -> WindVector {
        let month = month.min(self.months - 1) as usize;

        // Convert position to fractional grid coordinates
        let fx = (pos.x - self.origin.x) / self.cell_size_nm;
        let fy = (self.origin.y - pos.y) / self.cell_size_nm;

        // Clamp to grid bounds
        let fx = fx.clamp(0.0, (self.width - 1) as f32);
        let fy = fy.clamp(0.0, (self.height - 1) as f32);

        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = (x0 + 1).min((self.width - 1) as usize);
        let y1 = (y0 + 1).min((self.height - 1) as usize);

        let tx = fx - fx.floor();
        let ty = fy - fy.floor();

        let layer_offset = month * (self.height as usize) * (self.width as usize);
        let w = self.width as usize;

        // Sample 4 corners
        let u00 = self.u_data[layer_offset + y0 * w + x0];
        let u10 = self.u_data[layer_offset + y0 * w + x1];
        let u01 = self.u_data[layer_offset + y1 * w + x0];
        let u11 = self.u_data[layer_offset + y1 * w + x1];

        let v00 = self.v_data[layer_offset + y0 * w + x0];
        let v10 = self.v_data[layer_offset + y0 * w + x1];
        let v01 = self.v_data[layer_offset + y1 * w + x0];
        let v11 = self.v_data[layer_offset + y1 * w + x1];

        // Bilinear interpolation
        let u = u00 * (1.0 - tx) * (1.0 - ty)
            + u10 * tx * (1.0 - ty)
            + u01 * (1.0 - tx) * ty
            + u11 * tx * ty;
        let v = v00 * (1.0 - tx) * (1.0 - ty)
            + v10 * tx * (1.0 - ty)
            + v01 * (1.0 - tx) * ty
            + v11 * tx * ty;

        WindVector { u, v }
    }

    /// Construct a WindGrid from raw components (used by tests / synthetic maps).
    pub fn from_raw(
        u_data: Vec<f32>,
        v_data: Vec<f32>,
        width: u32,
        height: u32,
        origin: Position,
        cell_size_nm: f32,
        months: u8,
    ) -> Self {
        let n = (months as usize) * (height as usize) * (width as usize);
        assert_eq!(u_data.len(), n, "u_data size mismatch");
        assert_eq!(v_data.len(), n, "v_data size mismatch");
        Self { u_data, v_data, width, height, origin, cell_size_nm, months }
    }
}
