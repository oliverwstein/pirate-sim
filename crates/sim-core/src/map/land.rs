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
    /// Chebyshev (king-move) distance from each cell to the nearest land
    /// cell, saturating at 255. Land cells are 0. Computed once at load
    /// time so `has_cell_clearance` is an O(1) array lookup instead of a
    /// (2k+1)² nested scan — critical for A* performance at 1 NM/cell.
    pub dist_to_land: Vec<u8>,
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
        let dist_to_land = compute_distance_to_land(&data, width, height);

        Self {
            data,
            width,
            height,
            origin: Position::new(origin_x, origin_y),
            cell_size_nm: cell_size,
            dist_to_land,
        }
    }

    /// Convert world position to grid cell indices. Returns None if out of bounds.
    pub fn pos_to_cell(&self, pos: Position) -> Option<(u32, u32)> {
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

    /// World-space position of a cell center.
    pub fn cell_to_pos(&self, col: u32, row: u32) -> Position {
        Position::new(
            self.origin.x + (col as f32 + 0.5) * self.cell_size_nm,
            self.origin.y - (row as f32 + 0.5) * self.cell_size_nm,
        )
    }

    /// Returns true if the given cell is sea (in-bounds + data == 0).
    pub fn is_sea_cell(&self, col: u32, row: u32) -> bool {
        if col >= self.width || row >= self.height {
            return false;
        }
        let idx = (row * self.width + col) as usize;
        self.data[idx] != 255
    }

    /// Find the nearest sea cell to a starting cell using BFS. Useful when a
    /// destination position falls on a land cell (e.g., a coastal port).
    /// Returns None only if no sea cell is reachable within `max_radius` cells.
    pub fn nearest_sea_cell(&self, col: u32, row: u32, max_radius: u32) -> Option<(u32, u32)> {
        if self.is_sea_cell(col, row) {
            return Some((col, row));
        }
        use std::collections::{HashSet, VecDeque};
        let mut visited: HashSet<(u32, u32)> = HashSet::new();
        let mut queue: VecDeque<(u32, u32, u32)> = VecDeque::new();
        visited.insert((col, row));
        queue.push_back((col, row, 0));
        while let Some((c, r, d)) = queue.pop_front() {
            if d > max_radius {
                continue;
            }
            for (dc, dr) in &[
                (-1i32, 0i32),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (1, -1),
                (-1, 1),
                (1, 1),
            ] {
                let nc = c as i32 + dc;
                let nr = r as i32 + dr;
                if nc < 0 || nr < 0 || nc >= self.width as i32 || nr >= self.height as i32 {
                    continue;
                }
                let key = (nc as u32, nr as u32);
                if !visited.insert(key) {
                    continue;
                }
                if self.is_sea_cell(key.0, key.1) {
                    return Some(key);
                }
                queue.push_back((key.0, key.1, d + 1));
            }
        }
        None
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

    /// Check whether the straight line from `a` to `b` crosses any land,
    /// sampling at roughly cell-resolution along the way. Used for path
    /// smoothing: skip an intermediate waypoint if the line of sight is clear.
    pub fn line_is_clear(&self, a: Position, b: Position) -> bool {
        let delta = b - a;
        let dist = delta.length();
        if dist <= 0.0 {
            return true;
        }
        // Sample at half-cell intervals so we don't skip a thin spit.
        let step = (self.cell_size_nm * 0.5).max(0.1);
        let steps = (dist / step).ceil() as u32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let p = a + delta * t;
            if self.is_land(p) {
                return false;
            }
        }
        true
    }

    /// Stricter check: the rectangular corridor of half-width `margin_nm`
    /// centered on the segment a→b is entirely sea. Used during path
    /// smoothing so that smoothed segments retain clearance from coastlines.
    pub fn corridor_is_clear(&self, a: Position, b: Position, margin_nm: f32) -> bool {
        let delta = b - a;
        let dist = delta.length();
        if dist <= 0.0 {
            return self.has_clearance(a, margin_nm);
        }
        let dir = delta / dist;
        // Perpendicular (rotated +90°): (dx, dy) -> (-dy, dx). In our coords,
        // this is just the 2D left-normal.
        let perp = Position::new(-dir.y, dir.x);
        let step = (self.cell_size_nm * 0.5).max(0.1);
        let steps = (dist / step).ceil() as u32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let p = a + delta * t;
            if self.is_land(p) {
                return false;
            }
            if margin_nm > 0.0 {
                let off1 = p + perp * margin_nm;
                let off2 = p - perp * margin_nm;
                if self.is_land(off1) || self.is_land(off2) {
                    return false;
                }
            }
        }
        true
    }

    /// True if `pos` and a small disc of radius `margin_nm` around it are
    /// all sea (samples at cardinal + diagonal offsets).
    pub fn has_clearance(&self, pos: Position, margin_nm: f32) -> bool {
        if self.is_land(pos) {
            return false;
        }
        if margin_nm <= 0.0 {
            return true;
        }
        let d = margin_nm;
        let s = margin_nm * std::f32::consts::FRAC_1_SQRT_2;
        for off in [
            Position::new(d, 0.0),
            Position::new(-d, 0.0),
            Position::new(0.0, d),
            Position::new(0.0, -d),
            Position::new(s, s),
            Position::new(s, -s),
            Position::new(-s, s),
            Position::new(-s, -s),
        ] {
            if self.is_land(pos + off) {
                return false;
            }
        }
        true
    }

    /// Returns the farthest point along the segment a→b that is still in
    /// open sea. If the start is on land or the entire segment is clear,
    /// the result equals `b`. If the segment immediately hits land,
    /// returns `a`. Implemented via a small binary search after sample-stepping.
    pub fn farthest_clear_point(&self, a: Position, b: Position) -> Position {
        if a == b {
            return a;
        }
        if !self.is_land(b) && self.line_is_clear(a, b) {
            return b;
        }
        let delta = b - a;
        let dist = delta.length();
        if dist <= 0.0 {
            return a;
        }
        // Walk forward in cell-half steps until we hit land; binary search the
        // last segment for sub-cell precision.
        let step = (self.cell_size_nm * 0.5).max(0.1);
        let n = (dist / step).ceil().max(1.0) as u32;
        let mut last_safe_t = 0.0_f32;
        for i in 1..=n {
            let t = (i as f32 / n as f32).min(1.0);
            let p = a + delta * t;
            if self.is_land(p) {
                // Refine: binary search between last_safe_t and t.
                let mut lo = last_safe_t;
                let mut hi = t;
                for _ in 0..10 {
                    let mid = 0.5 * (lo + hi);
                    if self.is_land(a + delta * mid) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                return a + delta * lo;
            }
            last_safe_t = t;
        }
        b
    }

    /// Construct a LandMap from raw components (used by tests / synthetic maps).
    pub fn from_raw(
        data: Vec<u8>,
        width: u32,
        height: u32,
        origin: Position,
        cell_size_nm: f32,
    ) -> Self {
        assert_eq!(data.len(), (width * height) as usize, "data size mismatch");
        let dist_to_land = compute_distance_to_land(&data, width, height);
        Self {
            data,
            width,
            height,
            origin,
            cell_size_nm,
            dist_to_land,
        }
    }
}

/// Two-pass Chebyshev distance transform. Land cells (and out-of-bounds,
/// implicitly via initial INF values not propagating through them) get 0;
/// sea cells get the king-move distance to the nearest land cell, saturated
/// at 255.
fn compute_distance_to_land(data: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    // Use u16 internally to avoid wraparound during accumulation; clamp to
    // u8 at the end. Initial sea cells get a large value, land = 0.
    let mut dist: Vec<u16> = data
        .iter()
        .map(|&v| if v == 255 { 0 } else { u16::MAX })
        .collect();
    // Forward pass: top-to-bottom, left-to-right. Borders are treated as
    // out-of-bounds (effectively land) so coastline cells at the edge get
    // dist=1, ensuring `has_cell_clearance` rejects them safely.
    for r in 0..h {
        for c in 0..w {
            let idx = r * w + c;
            if dist[idx] == 0 {
                continue;
            }
            let mut m: u16 = u16::MAX;
            // Border check: treat out-of-bounds as land (dist=0), so the
            // candidate neighbor distance is 0+1=1.
            if r == 0 || c == 0 || c + 1 == w {
                m = 1;
            } else {
                if dist[idx - w - 1] < m {
                    m = dist[idx - w - 1];
                }
                if dist[idx - w] < m {
                    m = dist[idx - w];
                }
                if dist[idx - w + 1] < m {
                    m = dist[idx - w + 1];
                }
                if dist[idx - 1] < m {
                    m = dist[idx - 1];
                }
            }
            let cand = m.saturating_add(1);
            if cand < dist[idx] {
                dist[idx] = cand;
            }
        }
    }
    // Backward pass: bottom-to-top, right-to-left.
    for r in (0..h).rev() {
        for c in (0..w).rev() {
            let idx = r * w + c;
            if dist[idx] == 0 {
                continue;
            }
            let mut m: u16 = u16::MAX;
            if r + 1 == h || c == 0 || c + 1 == w {
                m = 1;
            } else {
                if dist[idx + 1] < m {
                    m = dist[idx + 1];
                }
                if dist[idx + w - 1] < m {
                    m = dist[idx + w - 1];
                }
                if dist[idx + w] < m {
                    m = dist[idx + w];
                }
                if dist[idx + w + 1] < m {
                    m = dist[idx + w + 1];
                }
            }
            let cand = m.saturating_add(1);
            if cand < dist[idx] {
                dist[idx] = cand;
            }
        }
    }
    dist.into_iter().map(|d| d.min(255) as u8).collect()
}
