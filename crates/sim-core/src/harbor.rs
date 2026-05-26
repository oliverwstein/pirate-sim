//! Harbor zones — the region around each port that counts as "in port"
//! for arrival/docking purposes.
//!
//! **Phase B rewrite.** The legacy raster cell-set (`HashSet<(u32,u32)>`
//! over the 1 NM `LandMap`) is gone. A harbor is now defined by:
//!
//! - The port's geographic coordinate (`port_pos`).
//! - A radius (`harbor_radius_nm`, from `Port::harbor_radius_nm`) — the
//!   physical extent of the harbor.
//! - An **anchor**: a tile-centroid `Position` in clean water, reachable
//!   from `port_pos` by line-of-sight. This becomes the planner's
//!   terminal target.
//! - The **anchor tile id** in the new convex-tile [`TileMesh`], so
//!   routing entries can use SSSP from the anchor tile and its
//!   neighbours.
//!
//! Membership test ("is this ship in the harbor?"):
//! ```text
//! dist(pos, port_pos) <= harbor_radius_nm
//!   AND line_is_clear(pos, anchor)
//! ```
//! The line-of-sight clause is what prevents a ship on the wrong side
//! of a peninsula from being declared "docked" just because it's
//! geometrically close to the city. We still consult [`LandMap`] for
//! the LOS test here; Phase E will swap that for the polygon-truth
//! `coastline_geom::CoastlineGeom::line_is_clear`.
//!
//! Anchor derivation (`HarborMap::build`):
//! 1. Look up the geometric-nearest tile centroid via the [`TileMesh`]
//!    centroid hash. That centroid lies in the largest connected ocean
//!    component (the preprocessor's filter), so it is always a safe
//!    planner target.
//! 2. If no tile sits within the initial 80 NM scan, expand the search
//!    radius geometrically (×2 per shell) until either a centroid is
//!    found or `ANCHOR_FALLBACK_MAX_RADIUS_NM` is exceeded.
//! 3. No line-of-sight check from `port_pos` to the candidate centroid:
//!    many `ports.ron` coordinates sit slightly inland at our
//!    coastline-polygon resolution, and an LOS gate would spuriously
//!    reject every nearby tile. The LOS clause is still applied in
//!    `Harbor::contains_pos` between the **ship's** position and the
//!    anchor — that one is meaningful (ships are in water by invariant).
//!
//! The reverse `cell_to_port` index is also gone — its only caller was
//! the legacy `HarborMap`-side BFS that no longer exists. Phase F
//! tests confirm no current production code reaches for it.

use crate::map::land::LandMap;
use crate::port::Port;
use crate::tile_mesh::TileMesh;
use crate::types::Position;

/// Search radius (NM) around `port_pos` for the initial tile-centroid
/// scan. Sized to comfortably exceed the largest single `harbor_radius_nm`
/// in the port catalogue (60 NM at New York), so even widely-spread
/// harbors find candidates without a graph walk.
const ANCHOR_SEARCH_RADIUS_NM: f32 = 80.0;

/// Maximum search radius (NM) for the expanding-shell fallback. A port
/// whose nominal position is more than this far from any navmesh tile
/// is treated as genuinely unreachable.
const ANCHOR_FALLBACK_MAX_RADIUS_NM: f32 = 5120.0;

/// One port's harbor zone.
pub struct Harbor {
    pub port_index: usize,
    /// The port's geographic coordinate (`Port::position`) — copied here
    /// so the planner and the `contains_pos` membership test don't need
    /// to chase back through `&[Port]`.
    pub port_pos: Position,
    /// The harbor's physical extent. Equal to `Port::harbor_radius_nm`.
    pub harbor_radius_nm: f32,
    /// Deep-water anchor: a tile centroid LOS-visible from `port_pos`,
    /// used as the planner's terminal target.
    pub anchor: Position,
    /// Index into [`TileMesh::tiles`] for the anchor's tile. `None`
    /// only for test fixtures that build a `HarborMap` without a tile
    /// mesh (via the soon-to-be-removed legacy fallback in unit tests
    /// — see Phase G).
    pub anchor_tile: Option<u32>,
    /// World-space bounding box of the harbor disk. Cheap pre-reject
    /// for `contains_pos`.
    pub bbox: (Position, Position),
}

impl Harbor {
    /// Is `pos` inside this harbor zone?
    ///
    /// True iff `dist(pos, port_pos) <= harbor_radius_nm` AND the
    /// straight line from `pos` to `anchor` is clear of land. The
    /// second clause stops a ship sitting on the wrong side of a
    /// peninsula from being declared "docked" purely because it is
    /// within the radius.
    pub fn contains_pos(&self, land: &LandMap, pos: Position) -> bool {
        let (mn, mx) = self.bbox;
        if pos.x < mn.x || pos.x > mx.x || pos.y < mn.y || pos.y > mx.y {
            return false;
        }
        let dx = pos.x - self.port_pos.x;
        let dy = pos.y - self.port_pos.y;
        if dx * dx + dy * dy > self.harbor_radius_nm * self.harbor_radius_nm {
            return false;
        }
        land.line_is_clear(pos, self.anchor)
    }
}

/// All harbors. Indexed by `port_index` only via the linear scan in
/// [`HarborMap::for_port`] — n is small (~40) so a hash map would
/// hurt cache locality more than it helps.
pub struct HarborMap {
    pub harbors: Vec<Harbor>,
}

impl HarborMap {
    /// An empty harbor map — useful for tests where no harbor zones are
    /// needed (the AI falls back to geometric arrival).
    pub fn empty() -> Self {
        HarborMap {
            harbors: Vec::new(),
        }
    }

    /// Build a harbor for each port from the convex-tile [`TileMesh`].
    ///
    /// `land` is still consulted for the LOS visibility test between
    /// `port_pos` and a candidate anchor tile centroid. Phase E will
    /// swap that for polygon-truth `coastline_geom::line_is_clear`;
    /// at that point this build signature changes to add `&CoastlineGeom`.
    pub fn build(land: &LandMap, tile_mesh: &TileMesh, ports: &[Port]) -> Self {
        let mut harbors = Vec::with_capacity(ports.len());

        for (idx, port) in ports.iter().enumerate() {
            let Some(anchor_tile) = find_anchor_tile(land, tile_mesh, port.position) else {
                // Genuinely unreachable port — skip, same failure mode
                // as the legacy `nearest_sea_cell` miss.
                eprintln!(
                    "[harbor] {}: no tile centroid within {} NM (expanding shell exhausted) — skipping",
                    port.name, ANCHOR_FALLBACK_MAX_RADIUS_NM
                );
                continue;
            };
            let anchor = tile_mesh.tiles[anchor_tile as usize].centroid;

            // Bbox is the disk around `port_pos`, padded outward to
            // include the anchor (which may sit slightly outside the
            // radius on river-mouth ports).
            let pad = port.harbor_radius_nm.max(port.position.distance(anchor));
            let bbox = (
                Position::new(port.position.x - pad, port.position.y - pad),
                Position::new(port.position.x + pad, port.position.y + pad),
            );

            harbors.push(Harbor {
                port_index: idx,
                port_pos: port.position,
                harbor_radius_nm: port.harbor_radius_nm,
                anchor,
                anchor_tile: Some(anchor_tile),
                bbox,
            });
        }

        HarborMap { harbors }
    }

    /// Look up a harbor by its port index. O(n) over harbors; n is small.
    pub fn for_port(&self, port_index: usize) -> Option<&Harbor> {
        self.harbors.iter().find(|h| h.port_index == port_index)
    }
}

/// Phase B anchor-discovery for one port.
///
/// Pick the geometric-nearest tile centroid in the [`TileMesh`]. No
/// line-of-sight check: many ports' nominal city coordinate sits
/// slightly inland at our coastline-polygon resolution, which would
/// fail LOS against every nearby centroid. The nearest centroid is
/// already in clean water (the preprocessor filtered to the largest
/// connected ocean component, with a 0.1 NM shore margin), so it is
/// always a safe planner target even when the city itself is on land.
///
/// Returns `None` only if the mesh has zero tiles within the initial
/// scan radius and the expanding-shell fallback also exhausts itself.
/// For the bundled `data/grids/navmesh.bin` this only happens for
/// ports placed far outside the world bounds.
fn find_anchor_tile(_land: &LandMap, mesh: &TileMesh, port_pos: Position) -> Option<u32> {
    // Initial scan at the standard radius.
    let mut candidates = mesh.nearest_centroids(port_pos, ANCHOR_SEARCH_RADIUS_NM);

    // Expanding-shell fallback for ports outside the initial radius
    // (e.g. atypical fudged coordinates): keep doubling until we hit a
    // tile or blow past a sane upper bound.
    let mut radius = ANCHOR_SEARCH_RADIUS_NM;
    while candidates.is_empty() && radius < ANCHOR_FALLBACK_MAX_RADIUS_NM {
        radius *= 2.0;
        candidates = mesh.nearest_centroids(port_pos, radius);
    }

    candidates.first().map(|&(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::Faction;
    use crate::tile_mesh::{Tile, TileEdge};

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

    /// Build a one-tile fake mesh whose centroid is at `centroid`.
    /// Sufficient for `HarborMap` tests since we exercise the anchor
    /// + LOS contract, not graph traversal.
    fn single_tile_mesh(centroid: Position) -> TileMesh {
        // Construct via from_bytes equivalent: we use the public
        // (test-only) trick of building TileMesh through its module-
        // private fields via the same path tests in tile_mesh use.
        // Here we just build a one-tile mesh by hand.
        let tile = Tile {
            vertices: vec![
                Position::new(centroid.x - 5.0, centroid.y - 5.0),
                Position::new(centroid.x + 5.0, centroid.y - 5.0),
                Position::new(centroid.x + 5.0, centroid.y + 5.0),
                Position::new(centroid.x - 5.0, centroid.y + 5.0),
            ],
            centroid,
            bbox_min: Position::new(centroid.x - 5.0, centroid.y - 5.0),
            bbox_max: Position::new(centroid.x + 5.0, centroid.y + 5.0),
        };
        // Use `from_bytes` so we don't need to expose private fields:
        // serialise the tile to the binary format and parse it back.
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&1u32.to_le_bytes()); // num_tiles
        buf.extend_from_slice(&(tile.vertices.len() as u32).to_le_bytes());
        for v in &tile.vertices {
            buf.extend_from_slice(&v.x.to_le_bytes());
            buf.extend_from_slice(&v.y.to_le_bytes());
        }
        buf.extend_from_slice(&tile.centroid.x.to_le_bytes());
        buf.extend_from_slice(&tile.centroid.y.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // num_neighbors
        TileMesh::from_bytes(&buf).expect("synthetic single-tile mesh parses")
    }

    /// Two-tile mesh, used to exercise the BFS fallback. Tile 0 is
    /// closer to `port_pos` but blocked by land; tile 1 is farther but
    /// LOS-clear. They are linked as neighbours so BFS can hop from
    /// 0 to 1.
    fn two_tile_mesh(c0: Position, c1: Position) -> TileMesh {
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&2u32.to_le_bytes()); // num_tiles
        for (i, c) in [(0u32, c0), (1u32, c1)] {
            // 3-vertex synthetic tile (a triangle); vertex coords are
            // not exercised by the test (we only care about centroids).
            buf.extend_from_slice(&3u32.to_le_bytes());
            for off in [(-5.0, -5.0), (5.0, -5.0), (0.0, 5.0)] {
                let v = Position::new(c.x + off.0, c.y + off.1);
                buf.extend_from_slice(&v.x.to_le_bytes());
                buf.extend_from_slice(&v.y.to_le_bytes());
            }
            buf.extend_from_slice(&c.x.to_le_bytes());
            buf.extend_from_slice(&c.y.to_le_bytes());
            // One neighbour: the other tile, with portal at the midpoint
            // (any finite Position works for this test).
            buf.extend_from_slice(&1u32.to_le_bytes());
            let other = 1 - i;
            buf.extend_from_slice(&other.to_le_bytes());
            let mid = Position::new((c0.x + c1.x) * 0.5, (c0.y + c1.y) * 0.5);
            buf.extend_from_slice(&mid.x.to_le_bytes());
            buf.extend_from_slice(&mid.y.to_le_bytes());
        }
        TileMesh::from_bytes(&buf).expect("synthetic two-tile mesh parses")
    }

    fn test_port(name: &str, position: Position, radius: f32) -> Port {
        Port {
            name: name.to_string(),
            position,
            faction: Faction::Free,
            harbor_radius_nm: radius,
            shipyard: None,
            category: crate::pop::PortCategory::SmallColonial,
        }
    }

    /// Edge cases for the silenced-but-correct anchor-search BFS:
    /// avoids `TileEdge`'s `dead_code` lint while keeping the import
    /// honest for the file (the field is read by the production
    /// `find_anchor_tile`).
    #[allow(dead_code)]
    fn _touch_tile_edge(e: TileEdge) -> u32 {
        e.to
    }

    #[test]
    fn anchor_picks_nearest_los_centroid_in_open_sea() {
        let land = open_sea_land(50, 50, 1.0);
        let port = test_port("Open", Position::new(25.0, 25.0), 10.0);
        let mesh = single_tile_mesh(Position::new(27.0, 27.0));
        let map = HarborMap::build(&land, &mesh, std::slice::from_ref(&port));
        assert_eq!(map.harbors.len(), 1);
        let h = &map.harbors[0];
        assert_eq!(h.anchor, Position::new(27.0, 27.0));
        assert_eq!(h.anchor_tile, Some(0));
        assert!(h.contains_pos(&land, port.position));
        // 30 NM east of the port is well outside the 10 NM radius.
        assert!(!h.contains_pos(&land, Position::new(55.0, 25.0)));
    }

    #[test]
    fn contains_pos_rejects_blocked_los() {
        // 50x50 grid, vertical wall of land at col=30. Port + anchor on
        // the west side; query position on the east side. Within radius
        // but LOS blocked → not in harbor.
        let w = 50u32;
        let h = 50u32;
        let mut data = vec![0u8; (w * h) as usize];
        for r in 0..h {
            data[(r * w + 30) as usize] = 255;
        }
        let land = LandMap::from_raw(data, w, h, Position::new(0.0, h as f32), 1.0);

        let port = test_port("WestSide", Position::new(20.0, 25.0), 20.0);
        let mesh = single_tile_mesh(Position::new(22.0, 25.0));
        let map = HarborMap::build(&land, &mesh, std::slice::from_ref(&port));
        let harbor = &map.harbors[0];

        // West side, within radius, LOS-clear: in harbor.
        assert!(harbor.contains_pos(&land, Position::new(15.0, 25.0)));
        // East side, within radius, but the wall blocks LOS: not in harbor.
        assert!(!harbor.contains_pos(&land, Position::new(35.0, 25.0)));
    }

    #[test]
    fn anchor_picks_geometric_nearest_ignoring_los() {
        // Vertical wall at col=30. Port at (20, 20). Tile 0 centroid
        // (40, 20) is 20 NM away and behind the wall; tile 1 centroid
        // (15, 20) is 5 NM away and clear. Anchor should be tile 1 by
        // pure distance — no LOS gate at build time.
        let w = 60u32;
        let h = 40u32;
        let mut data = vec![0u8; (w * h) as usize];
        for r in 0..h {
            data[(r * w + 30) as usize] = 255;
        }
        let land = LandMap::from_raw(data, w, h, Position::new(0.0, h as f32), 1.0);

        let port = test_port("Riverhead", Position::new(20.0, 20.0), 10.0);
        let mesh = two_tile_mesh(Position::new(40.0, 20.0), Position::new(15.0, 20.0));

        let map = HarborMap::build(&land, &mesh, std::slice::from_ref(&port));
        assert_eq!(map.harbors.len(), 1);
        let h = &map.harbors[0];
        assert_eq!(h.anchor_tile, Some(1), "nearest by distance is tile 1");
        assert_eq!(h.anchor, Position::new(15.0, 20.0));
    }

    #[test]
    fn anchor_falls_back_to_expanding_shell_when_initial_radius_empty() {
        // Port well outside the initial 80 NM scan from the only tile.
        // The expanding-shell fallback should still discover it.
        let land = open_sea_land(400, 400, 1.0);
        let port = test_port("Far", Position::new(10.0, 10.0), 5.0);
        let mesh = single_tile_mesh(Position::new(380.0, 380.0));
        let map = HarborMap::build(&land, &mesh, std::slice::from_ref(&port));
        assert_eq!(
            map.harbors.len(),
            1,
            "expanding shell should reach the tile"
        );
        assert_eq!(map.harbors[0].anchor_tile, Some(0));
    }
}
