use crate::types::Position;

/// Faction allegiance (simplified for Phase 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Faction {
    Spain,
    England,
    France,
    Holland,
    Pirate,
}

impl Faction {
    pub fn color_rgb(&self) -> (u8, u8, u8) {
        match self {
            Faction::Spain => (255, 215, 0),   // gold
            Faction::England => (200, 50, 50), // red
            Faction::France => (70, 130, 255), // blue
            Faction::Holland => (255, 140, 0), // orange
            Faction::Pirate => (180, 180, 180), // gray
        }
    }
}

/// A port/settlement on the map.
pub struct Port {
    pub name: &'static str,
    pub position: Position,
    pub faction: Faction,
    /// Radius (NM) around `position` defining the harbor zone — the set of
    /// connected sea cells where a ship is considered "in port" for
    /// arrival/docking purposes. Larger values are useful for ports that sit
    /// up rivers or estuaries (Philadelphia, New York, New Orleans, etc.).
    pub harbor_radius_nm: f32,
}

/// Default harbor radius (NM) used when a port doesn't specify one.
pub const DEFAULT_HARBOR_RADIUS_NM: f32 = 8.0;

/// All ports from historical research (coordinates in NM from origin 17.5°N, 72.5°W).
pub fn all_ports() -> Vec<Port> {
    PORTS
        .iter()
        .map(|(name, x, y, faction, radius)| Port {
            name,
            position: Position::new(*x, *y),
            faction: *faction,
            harbor_radius_nm: *radius,
        })
        .collect()
}

const D: f32 = DEFAULT_HARBOR_RADIUS_NM;

const PORTS: &[(&str, f32, f32, Faction, f32)] = &[
    // === SPANISH ===
    ("Havana", -592.8, 337.8, Faction::Spain, D),
    ("Portobelo", -429.0, -477.0, Faction::Spain, D),
    ("Cartagena", -181.8, -426.0, Faction::Spain, D),
    ("Santo Domingo", 157.2, 58.2, Faction::Spain, D),
    ("Santiago de Cuba", -199.2, 151.2, Faction::Spain, D),
    ("San Juan", 382.8, 58.2, Faction::Spain, D),
    ("Maracaibo", 52.2, -412.2, Faction::Spain, 30.0), // up Lake Maracaibo
    ("La Guaira", 334.2, -414.0, Faction::Spain, D),
    ("Trinidad", 658.8, -411.0, Faction::Spain, D),
    ("Margarita", 517.8, -390.0, Faction::Spain, D),
    // === ENGLISH ===
    ("Port Royal", -260.4, 26.4, Faction::England, D),
    ("Kingston", -257.4, 28.2, Faction::England, D),
    ("Bridgetown", 772.8, -264.0, Faction::England, D),
    ("Basseterre", 586.8, -12.0, Faction::England, D),
    ("English Harbour", 644.4, -30.0, Faction::England, D),
    ("Charleston", -445.8, 916.8, Faction::England, 25.0),
    ("Boston", 86.4, 1491.6, Faction::England, 20.0),
    ("New York", -90.6, 1392.6, Faction::England, 60.0),     // NY harbor + lower Hudson + reach to Atlantic
    ("Philadelphia", -177.8, 1312.6, Faction::England, 30.0), // City at 39.953°N -75.164°W; anchor here at the Delaware Bay mouth (~30 NM SE) since 1-NM grid can't open the river.
    ("Bermuda", 469.2, 892.8, Faction::England, D),
    ("Belize", -942.0, 0.0, Faction::England, 30.0),
    // === FRENCH ===
    ("Fort-Royal", 685.8, -174.0, Faction::France, D),
    ("Basse-Terre", 646.2, -90.0, Faction::France, D),
    ("Cap-Français", 18.0, 135.6, Faction::France, D),
    ("Petit-Goâve", -22.2, 55.8, Faction::France, D),
    ("Cayenne", 1210.2, -753.6, Faction::France, 15.0),
    // === DUTCH ===
    ("Willemstad", 214.2, -323.4, Faction::Holland, D),
    ("St. Eustatius", 571.8, -1.2, Faction::Holland, D),
    ("Paramaribo", 1039.8, -685.0, Faction::Holland, 15.0), // moved to mouth of Suriname river
    // === PIRATE HAVENS ===
    ("Tortuga", -16.8, 152.4, Faction::Pirate, D),
    ("Nassau", -291.0, 453.0, Faction::Pirate, D),
    ("Tobago", 709.8, -375.0, Faction::Pirate, D),
];
