//! Harbor zones — the set of sea cells around each port where a ship is
//! considered "in port" for arrival/docking purposes.
//!
//! Pathfinding targets a harbor zone (a goal *region*, not a goal point), and
//! arrival triggers as soon as the ship enters the zone. This decouples three
//! formerly-tangled concerns:
//!
//! 1. Routing clearance (how much sea margin A* requires).
//! 2. Reachability (can a ship physically reach this port?).
//! 3. Docking (when does Sailing → Docked transition fire?).
//!
//! It also makes ports up rivers (Philadelphia, New York, Paramaribo) work
//! without forcing the planner to thread the river itself.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::map::land::LandMap;
use crate::port::Port;
use crate::types::Position;

/// One port's harbor zone: a connected set of sea cells reachable from the
/// port within `harbor_radius_nm`, plus a deep-water anchor cell suitable as
/// a heuristic target.
pub struct Harbor {
    pub port_index: usize,
    pub anchor: Position,
    pub cells: HashSet<(u32, u32)>,
    /// Bounding box of the zone in world space (min, max). Useful for cheap
    /// "is this position even close?" pre-checks.
    pub bbox: (Position, Position),
}

impl Harbor {
    /// True if a world position lies inside this harbor zone.
    pub fn contains_pos(&self, land: &LandMap, pos: Position) -> bool {
        // Cheap bbox reject first.
        let (mn, mx) = self.bbox;
        if pos.x < mn.x || pos.x > mx.x || pos.y < mn.y || pos.y > mx.y {
            return false;
        }
        match land.pos_to_cell(pos) {
            Some(cell) => self.cells.contains(&cell),
            None => false,
        }
    }
}

/// All harbors plus a reverse lookup from cell → port index.
pub struct HarborMap {
    pub harbors: Vec<Harbor>,
    pub cell_to_port: HashMap<(u32, u32), usize>,
}

impl HarborMap {
    /// An empty harbor map — useful for tests where no harbor zones are
    /// needed (the AI falls back to geometric arrival).
    pub fn empty() -> Self {
        HarborMap {
            harbors: Vec::new(),
            cell_to_port: HashMap::new(),
        }
    }

    /// Build a harbor zone for each port by BFS through sea cells from the
    /// port's nearest sea cell, stopping when the world-space distance from
    /// the port position exceeds its `harbor_radius_nm`.
    pub fn build(land: &LandMap, ports: &[Port]) -> Self {
        let mut harbors = Vec::with_capacity(ports.len());
        let mut cell_to_port: HashMap<(u32, u32), usize> = HashMap::new();

        for (idx, port) in ports.iter().enumerate() {
            let raw = match land.pos_to_cell(port.position) {
                Some(c) => c,
                None => continue, // off-map port — skip
            };
            // Snap onto a sea cell. Use a generous radius proportional to
            // the harbor radius so river-mouth ports (port position on land)
            // can still find an anchor.
            let snap_radius = ((port.harbor_radius_nm / land.cell_size_nm).ceil() as u32).max(8);
            let anchor_cell = match land.nearest_sea_cell(raw.0, raw.1, snap_radius) {
                Some(c) => c,
                None => continue, // no sea within reach — port unreachable
            };
            let anchor = land.cell_to_pos(anchor_cell.0, anchor_cell.1);

            let cells = bfs_zone(land, anchor_cell, port.position, port.harbor_radius_nm);

            // Compute bounding box and reverse-index the cells.
            let mut min_x = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            let mut min_y = f32::INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            let half = land.cell_size_nm * 0.5;
            for &cell in &cells {
                let p = land.cell_to_pos(cell.0, cell.1);
                min_x = min_x.min(p.x - half);
                max_x = max_x.max(p.x + half);
                min_y = min_y.min(p.y - half);
                max_y = max_y.max(p.y + half);
                cell_to_port.insert(cell, idx);
            }

            let bbox = if cells.is_empty() {
                (anchor, anchor)
            } else {
                (Position::new(min_x, min_y), Position::new(max_x, max_y))
            };

            harbors.push(Harbor {
                port_index: idx,
                anchor,
                cells,
                bbox,
            });
        }

        HarborMap {
            harbors,
            cell_to_port,
        }
    }

    /// Look up a harbor by its port index. O(n) over harbors; n is small.
    pub fn for_port(&self, port_index: usize) -> Option<&Harbor> {
        self.harbors.iter().find(|h| h.port_index == port_index)
    }
}

/// 8-connected BFS through sea cells starting at `seed`, stopping cells
/// whose center is more than `radius_nm` from `center`.
fn bfs_zone(
    land: &LandMap,
    seed: (u32, u32),
    center: Position,
    radius_nm: f32,
) -> HashSet<(u32, u32)> {
    let mut zone: HashSet<(u32, u32)> = HashSet::new();
    let mut q: VecDeque<(u32, u32)> = VecDeque::new();
    if !land.is_sea_cell(seed.0, seed.1) {
        return zone;
    }
    zone.insert(seed);
    q.push_back(seed);

    let r2 = radius_nm * radius_nm;

    const NEIGHBORS: [(i32, i32); 8] = [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (1, -1),
        (-1, 1),
        (-1, -1),
    ];

    while let Some(cell) = q.pop_front() {
        for &(dc, dr) in &NEIGHBORS {
            let nc = cell.0 as i32 + dc;
            let nr = cell.1 as i32 + dr;
            if nc < 0 || nr < 0 || nc >= land.width as i32 || nr >= land.height as i32 {
                continue;
            }
            let neighbor = (nc as u32, nr as u32);
            if zone.contains(&neighbor) {
                continue;
            }
            if !land.is_sea_cell(neighbor.0, neighbor.1) {
                continue;
            }
            // Distance gate against the *center* (port position), not the
            // BFS seed. This keeps the zone roughly disk-shaped around the
            // port itself even when the anchor cell is offset.
            let p = land.cell_to_pos(neighbor.0, neighbor.1);
            let dx = p.x - center.x;
            let dy = p.y - center.y;
            if dx * dx + dy * dy > r2 {
                continue;
            }
            zone.insert(neighbor);
            q.push_back(neighbor);
        }
    }

    zone
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::Faction;

    fn open_sea_land(width: u32, height: u32, cell: f32) -> LandMap {
        let data = vec![0u8; (width * height) as usize];
        LandMap::from_raw(
            data,
            width,
            height,
            Position::new(0.0, height as f32 * cell),
            cell,
        )
    }

    #[test]
    fn harbor_zone_is_disk_in_open_sea() {
        let land = open_sea_land(40, 40, 1.0);
        let port = Port {
            name: "Test".to_string(),
            position: Position::new(20.0, 20.0),
            faction: Faction::Pirate,
            harbor_radius_nm: 5.0,
            shipyard: None,
        };
        let map = HarborMap::build(&land, std::slice::from_ref(&port));
        assert_eq!(map.harbors.len(), 1);
        let h = &map.harbors[0];
        // ~π·5² ≈ 78 cells (1 NM/cell)
        assert!(
            h.cells.len() > 60 && h.cells.len() < 100,
            "got {}",
            h.cells.len()
        );
        assert!(h.contains_pos(&land, port.position));
        assert!(!h.contains_pos(&land, Position::new(20.0, 30.0))); // 10 NM away
    }

    #[test]
    fn harbor_zone_respects_isthmus() {
        // 20x20 grid, cell=1 NM. Wall of land at col=10 with no gap → BFS
        // from west of the wall must NOT cross to the east half even if it's
        // within `radius_nm`.
        let w = 20u32;
        let h = 20u32;
        let mut data = vec![0u8; (w * h) as usize];
        for r in 0..h {
            data[(r * w + 10) as usize] = 255;
        }
        let land = LandMap::from_raw(data, w, h, Position::new(0.0, h as f32), 1.0);
        let port = Port {
            name: "WestSide".to_string(),
            position: Position::new(8.0, 10.0), // west of wall
            faction: Faction::Pirate,
            harbor_radius_nm: 8.0, // would cross wall if straight-line
            shipyard: None,
        };
        let map = HarborMap::build(&land, std::slice::from_ref(&port));
        let h0 = &map.harbors[0];
        // Every cell in the zone must have col < 10 (west of the wall).
        for &(c, _r) in &h0.cells {
            assert!(c < 10, "zone leaked east of wall to col {}", c);
        }
    }
}
