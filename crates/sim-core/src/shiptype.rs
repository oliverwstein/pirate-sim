//! Ship type catalog. Each type has its own stats, build cost, and
//! expected lifetime. A `Ship` carries a `ShipTypeId` and the world
//! looks up the matching `ShipType` for per-tick stats access.
//!
//! The catalog is loaded from `data/registries/ship_types.ron`. Position
//! in that file determines the `ShipTypeId` — the `ids_match_indices`
//! test will catch any re-ordering.
//!
//! v1 catalog (merchant rigs relevant to the 1680s economy):
//!
//!   SLOOP        small, fast, fore-and-aft — Bermuda / Jamaica
//!                cedar build, the quintessential Caribbean trader
//!   BRIGANTINE   two-masted mixed rig, popular colonial trader
//!   BARK         small-medium three-masted square-rigger, bulk
//!   FLUYT        Dutch dedicated cargo hull, small crew, half the
//!                cost-per-ton of an English ship
//!   SHIP         full-rigged three-masted Atlantic workhorse
//!
//! Numbers are mid-range values from `planning/research/ship-types.md`.
//! Warships (frigates, SOL, galleon, slave ship, East Indiaman) are
//! out of scope for v1 — they'll arrive with combat/piracy systems.

use crate::money::Pesos;
use crate::ship::ShipStats;
use serde::Deserialize;

/// Stable opaque identifier for a ship type. Stored on every `Ship`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct ShipTypeId(pub u8);

/// Canonical handles for the built-in ship types.
pub mod ids {
    use super::ShipTypeId;
    pub const SLOOP: ShipTypeId = ShipTypeId(0);
    pub const BRIGANTINE: ShipTypeId = ShipTypeId(1);
    pub const BARK: ShipTypeId = ShipTypeId(2);
    pub const FLUYT: ShipTypeId = ShipTypeId(3);
    pub const SHIP: ShipTypeId = ShipTypeId(4);
}

/// On-disk shape of one ship type. The runtime `ShipType` is built from
/// this by stamping in the index-derived `ShipTypeId`.
#[derive(Clone, Debug, Deserialize)]
struct ShipTypeRecord {
    name: String,
    stats: ShipStats,
    build_silver: f32,
    build_naval_stores: f32,
    build_manufactures: f32,
    build_provisions: f32,
    expected_lifetime_months: f32,
}

/// Per-type design data.
#[derive(Clone, Debug)]
pub struct ShipType {
    pub id: ShipTypeId,
    pub name: String,
    pub stats: ShipStats,
    /// Silver portion of the build cost (pesos).
    pub build_silver: Pesos,
    /// Naval stores (pitch, tar, cordage, sailcloth) in tons.
    pub build_naval_stores: f32,
    /// Manufactured goods (iron, tools, fittings) in tons.
    pub build_manufactures: f32,
    /// Provisions to outfit the maiden voyage, in tons.
    pub build_provisions: f32,
    /// Amortization horizon in months for the "math pencils" check.
    /// Calibrated to the type's historical service life.
    pub expected_lifetime_months: f32,
}

/// Registry of all known ship types. Lookup by `ShipTypeId`.
pub struct ShipTypeRegistry {
    types: Vec<ShipType>,
}

/// The bundled RON catalog, compiled into the binary.
const SHIP_TYPES_RON: &str = include_str!("../../../data/registries/ship_types.ron");

impl ShipTypeRegistry {
    /// Construct the v1 catalog by parsing the bundled `ship_types.ron`.
    /// Panics if the file fails to parse — that would be a build-time bug.
    pub fn starter() -> Self {
        Self::from_ron_str(SHIP_TYPES_RON).expect("bundled ship_types.ron must parse")
    }

    /// Parse a ship-type catalog from a RON string. Records are tagged
    /// with `ShipTypeId` equal to their position in the list.
    pub fn from_ron_str(s: &str) -> Result<Self, ron::error::SpannedError> {
        let records: Vec<ShipTypeRecord> = ron::from_str(s)?;
        let types = records
            .into_iter()
            .enumerate()
            .map(|(i, r)| ShipType {
                id: ShipTypeId(i as u8),
                name: r.name,
                stats: r.stats,
                build_silver: Pesos::from_pesos_f32(r.build_silver),
                build_naval_stores: r.build_naval_stores,
                build_manufactures: r.build_manufactures,
                build_provisions: r.build_provisions,
                expected_lifetime_months: r.expected_lifetime_months,
            })
            .collect();
        Ok(Self { types })
    }

    pub fn get(&self, id: ShipTypeId) -> &ShipType {
        &self.types[id.0 as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = &ShipType> {
        self.types.iter()
    }

    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_has_five_types() {
        let r = ShipTypeRegistry::starter();
        assert_eq!(r.len(), 5);
    }

    #[test]
    fn ids_match_indices() {
        let r = ShipTypeRegistry::starter();
        assert_eq!(r.get(ids::SLOOP).name, "sloop");
        assert_eq!(r.get(ids::BRIGANTINE).name, "brigantine");
        assert_eq!(r.get(ids::BARK).name, "bark");
        assert_eq!(r.get(ids::FLUYT).name, "fluyt");
        assert_eq!(r.get(ids::SHIP).name, "ship");
    }

    #[test]
    fn all_types_have_positive_costs_and_stats() {
        for t in ShipTypeRegistry::starter().iter() {
            assert!(t.build_silver > Pesos::ZERO, "{}: silver", t.name);
            assert!(t.build_naval_stores > 0.0, "{}: naval", t.name);
            assert!(t.build_manufactures > 0.0, "{}: manufactures", t.name);
            assert!(t.build_provisions > 0.0, "{}: provisions", t.name);
            assert!(t.expected_lifetime_months > 0.0, "{}: lifetime", t.name);
            assert!(t.stats.cargo_capacity_tons > 0.0, "{}: cargo", t.name);
            assert!(t.stats.crew > 0, "{}: crew", t.name);
        }
    }

    #[test]
    fn fluyt_has_more_cargo_per_silver_than_ship() {
        // Historical: the fluyt's competitive edge was cheap cargo
        // capacity — Davis (1962) puts Dutch shipping ~30% below
        // English in unit costs.
        let r = ShipTypeRegistry::starter();
        let fluyt = r.get(ids::FLUYT);
        let ship = r.get(ids::SHIP);
        let fluyt_ton_per_peso =
            fluyt.stats.cargo_capacity_tons / fluyt.build_silver.as_pesos_f32();
        let ship_ton_per_peso = ship.stats.cargo_capacity_tons / ship.build_silver.as_pesos_f32();
        assert!(
            fluyt_ton_per_peso > ship_ton_per_peso,
            "fluyt {:.4} vs ship {:.4}",
            fluyt_ton_per_peso,
            ship_ton_per_peso
        );
    }
}
