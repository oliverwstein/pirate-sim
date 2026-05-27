use crate::pop::PortCategory;
use crate::shiptype::{ShipTypeId, ShipTypeRegistry};
use crate::types::Position;
use serde::Deserialize;

/// Faction allegiance. Phase 3 keeps 5 factions: the four colonial powers
/// plus `Free` (independents, including pirates — actual piracy is a
/// per-ship `ShipPolicy`, not a faction).
///
/// `#[repr(u8)]` so a future `FactionSet` can be a compact bitset (used
/// e.g. for `ShipPolicy::Privateer { against: FactionSet }` in Step 6
/// and dynamic war-declaration sets in Phase 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[repr(u8)]
pub enum Faction {
    Spain = 0,
    England = 1,
    France = 2,
    Netherlands = 3,
    Free = 4,
}

impl Faction {
    pub fn color_rgb(&self) -> (u8, u8, u8) {
        match self {
            Faction::Spain => (255, 215, 0),       // gold
            Faction::England => (200, 50, 50),     // red
            Faction::France => (70, 130, 255),     // blue
            Faction::Netherlands => (255, 140, 0), // orange
            Faction::Free => (180, 180, 180),      // gray
        }
    }
}

/// A port/settlement on the map.
#[derive(Clone, Debug)]
pub struct Port {
    pub name: String,
    pub position: Position,
    pub faction: Faction,
    /// Radius (NM) around `position` defining the harbor zone — the set of
    /// connected sea cells where a ship is considered "in port" for
    /// arrival/docking purposes. Larger values are useful for ports that sit
    /// up rivers or estuaries (Philadelphia, New York, New Orleans, etc.).
    pub harbor_radius_nm: f32,
    /// If this port has a shipyard, the list of ship types it is
    /// equipped to build. `None` means no yard. Historically yards
    /// specialized: Bermuda's cedar suited only sloops; Amsterdam
    /// pioneered the fluyt; Cadiz's royal yards built large ships.
    pub shipyard: Option<Vec<ShipTypeId>>,
    /// Demographic category — drives sailor pool growth/mortality.
    /// See `pop::PortCategory` and `planning/crewing-plan.md`.
    pub category: PortCategory,
}

/// Default harbor radius (NM) used when a port doesn't specify one.
pub const DEFAULT_HARBOR_RADIUS_NM: f32 = 8.0;

/// Map-projection origin shared with the preprocessor pipeline
/// (`tools/preprocess/*`). Real-world `(lat_deg, lon_deg)` get
/// projected to the simulator's NM grid via
/// `x = (lon - ORIGIN_LON) * 60`, `y = (lat - ORIGIN_LAT) * 60`.
const ORIGIN_LAT_DEG: f32 = 17.5;
const ORIGIN_LON_DEG: f32 = -72.5;
const NM_PER_DEG: f32 = 60.0;

/// On-disk shape of one port. Coordinates are real-world decimal
/// degrees `(lat, lon)`; they are projected to the NM grid at load.
/// Shipyard ship-type names are resolved to `ShipTypeId` against the
/// live registry.
#[derive(Clone, Debug, Deserialize)]
struct PortRecord {
    name: String,
    coord: (f32, f32),
    faction: Faction,
    harbor_radius_nm: f32,
    shipyard: Option<Vec<String>>,
    category: PortCategory,
}

/// The bundled RON catalog of ports, compiled into the binary.
const PORTS_RON: &str = include_str!("../../../data/registries/ports.ron");

/// All ports from historical research, loaded from the bundled
/// `ports.ron`. Shipyard ship-type names are resolved against
/// `ship_types`; an unknown name is a build-time bug and panics.
pub fn all_ports(ship_types: &ShipTypeRegistry) -> Vec<Port> {
    from_ron_str(PORTS_RON, ship_types).expect("bundled ports.ron must parse")
}

/// Parse a port catalog from a RON string. Each `shipyard` ship-type
/// name is looked up in `ship_types`; an unknown name returns an error.
pub fn from_ron_str(s: &str, ship_types: &ShipTypeRegistry) -> Result<Vec<Port>, PortLoadError> {
    let records: Vec<PortRecord> = ron::from_str(s).map_err(PortLoadError::Ron)?;
    let mut ports = Vec::with_capacity(records.len());
    for r in records {
        let shipyard = match r.shipyard {
            None => None,
            Some(names) => {
                let mut ids = Vec::with_capacity(names.len());
                for n in &names {
                    let id = ship_types
                        .iter()
                        .find(|t| t.name == *n)
                        .map(|t| t.id)
                        .ok_or_else(|| PortLoadError::UnknownShipType {
                            port: r.name.clone(),
                            name: n.clone(),
                        })?;
                    ids.push(id);
                }
                Some(ids)
            }
        };
        let (lat_deg, lon_deg) = r.coord;
        let x_nm = (lon_deg - ORIGIN_LON_DEG) * NM_PER_DEG;
        let y_nm = (lat_deg - ORIGIN_LAT_DEG) * NM_PER_DEG;
        ports.push(Port {
            name: r.name,
            position: Position::new(x_nm, y_nm),
            faction: r.faction,
            harbor_radius_nm: r.harbor_radius_nm,
            shipyard,
            category: r.category,
        });
    }
    Ok(ports)
}

#[derive(Debug)]
pub enum PortLoadError {
    Ron(ron::error::SpannedError),
    UnknownShipType { port: String, name: String },
}

impl std::fmt::Display for PortLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortLoadError::Ron(e) => write!(f, "RON parse error: {e}"),
            PortLoadError::UnknownShipType { port, name } => {
                write!(f, "port {port:?} references unknown ship type {name:?}")
            }
        }
    }
}

impl std::error::Error for PortLoadError {}
