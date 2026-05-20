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
}

/// All ports from historical research (coordinates in NM from origin 17.5°N, 72.5°W).
pub fn all_ports() -> Vec<Port> {
    PORTS
        .iter()
        .map(|(name, x, y, faction)| Port {
            name,
            position: Position::new(*x, *y),
            faction: *faction,
        })
        .collect()
}

const PORTS: &[(&str, f32, f32, Faction)] = &[
    // === SPANISH ===
    ("Havana", -592.8, 337.8, Faction::Spain),
    ("Portobelo", -429.0, -477.0, Faction::Spain),
    ("Cartagena", -181.8, -426.0, Faction::Spain),
    ("Santo Domingo", 157.2, 58.2, Faction::Spain),
    ("Santiago de Cuba", -199.2, 151.2, Faction::Spain),
    ("San Juan", 382.8, 58.2, Faction::Spain),
    ("Maracaibo", 52.2, -412.2, Faction::Spain),
    ("La Guaira", 334.2, -414.0, Faction::Spain),
    ("Trinidad", 658.8, -411.0, Faction::Spain),
    ("Margarita", 517.8, -390.0, Faction::Spain),
    // === ENGLISH ===
    ("Port Royal", -260.4, 26.4, Faction::England),
    ("Kingston", -257.4, 28.2, Faction::England),
    ("Bridgetown", 772.8, -264.0, Faction::England),
    ("Basseterre", 586.8, -12.0, Faction::England),
    ("English Harbour", 644.4, -30.0, Faction::England),
    ("Charleston", -445.8, 916.8, Faction::England),
    ("Boston", 86.4, 1491.6, Faction::England),
    ("New York", -90.6, 1392.6, Faction::England),
    ("Philadelphia", -160.2, 1347.0, Faction::England),
    ("Bermuda", 469.2, 892.8, Faction::England),
    ("Belize", -942.0, 0.0, Faction::England),
    // === FRENCH ===
    ("Fort-Royal", 685.8, -174.0, Faction::France),
    ("Basse-Terre", 646.2, -90.0, Faction::France),
    ("Cap-Français", 18.0, 135.6, Faction::France),
    ("Petit-Goâve", -22.2, 55.8, Faction::France),
    ("Cayenne", 1210.2, -753.6, Faction::France),
    // === DUTCH ===
    ("Willemstad", 214.2, -323.4, Faction::Holland),
    ("St. Eustatius", 571.8, -1.2, Faction::Holland),
    ("Paramaribo", 1039.8, -700.2, Faction::Holland),
    // === PIRATE HAVENS ===
    ("Tortuga", -16.8, 152.4, Faction::Pirate),
    ("Nassau", -291.0, 453.0, Faction::Pirate),
    ("Tobago", 709.8, -375.0, Faction::Pirate),
];
