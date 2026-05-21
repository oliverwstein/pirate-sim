//! Ship type catalog. Each type has its own stats, build cost, and
//! expected lifetime. A `Ship` carries a `ShipTypeId` and the world
//! looks up the matching `ShipType` for per-tick stats access.
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

use crate::ship::ShipStats;

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

/// Per-type design data.
#[derive(Clone, Debug)]
pub struct ShipType {
    pub id: ShipTypeId,
    pub name: &'static str,
    pub stats: ShipStats,
    /// Silver portion of the build cost (pesos).
    pub build_silver: f32,
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

impl ShipTypeRegistry {
    /// Construct the v1 hardcoded catalog. The order matches the
    /// `ids::*` constants — never reorder without updating them.
    pub fn starter() -> Self {
        let types = vec![
            ShipType {
                id: ids::SLOOP,
                name: "sloop",
                stats: ShipStats {
                    speed_typical: 9.0,
                    speed_max: 12.0,
                    windward_ability: 0.8,
                    no_go_half_angle: 40.0,
                    crew: 25,
                    provision_capacity: 6.0,
                    cargo_capacity_tons: 60.0,
                },
                build_silver: 4_000.0,
                build_naval_stores: 8.0,
                build_manufactures: 5.0,
                build_provisions: 6.0,
                expected_lifetime_months: 84.0, // 7 yr median (Caribbean tropical service)
            },
            ShipType {
                id: ids::BRIGANTINE,
                name: "brigantine",
                stats: ShipStats {
                    speed_typical: 8.0,
                    speed_max: 11.0,
                    windward_ability: 0.7,
                    no_go_half_angle: 45.0,
                    crew: 40,
                    provision_capacity: 10.0,
                    cargo_capacity_tons: 100.0,
                },
                build_silver: 7_000.0,
                build_naval_stores: 14.0,
                build_manufactures: 9.0,
                build_provisions: 10.0,
                expected_lifetime_months: 144.0, // 12 yr
            },
            ShipType {
                id: ids::BARK,
                name: "bark",
                stats: ShipStats {
                    speed_typical: 6.0,
                    speed_max: 9.0,
                    windward_ability: 0.5,
                    no_go_half_angle: 55.0,
                    crew: 35,
                    provision_capacity: 12.0,
                    cargo_capacity_tons: 160.0,
                },
                build_silver: 10_000.0,
                build_naval_stores: 20.0,
                build_manufactures: 14.0,
                build_provisions: 14.0,
                expected_lifetime_months: 180.0, // 15 yr
            },
            ShipType {
                id: ids::FLUYT,
                name: "fluyt",
                stats: ShipStats {
                    speed_typical: 5.0,
                    speed_max: 8.0,
                    windward_ability: 0.4,
                    no_go_half_angle: 60.0,
                    crew: 25, // characteristic small crew
                    provision_capacity: 16.0,
                    cargo_capacity_tons: 250.0,
                },
                build_silver: 9_000.0,        // cheaper than Ship despite larger hold
                build_naval_stores: 24.0,
                build_manufactures: 14.0,
                build_provisions: 16.0,
                expected_lifetime_months: 200.0, // ~17 yr (good Dutch maintenance)
            },
            ShipType {
                id: ids::SHIP,
                name: "ship",
                stats: ShipStats {
                    speed_typical: 6.0,
                    speed_max: 10.0,
                    windward_ability: 0.4,
                    no_go_half_angle: 60.0,
                    crew: 60,
                    provision_capacity: 20.0,
                    cargo_capacity_tons: 300.0,
                },
                build_silver: 18_000.0,
                build_naval_stores: 32.0,
                build_manufactures: 22.0,
                build_provisions: 22.0,
                expected_lifetime_months: 240.0, // 20 yr (well-maintained)
            },
        ];
        Self { types }
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
            assert!(t.build_silver > 0.0, "{}: silver", t.name);
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
        let fluyt_ton_per_peso = fluyt.stats.cargo_capacity_tons / fluyt.build_silver;
        let ship_ton_per_peso = ship.stats.cargo_capacity_tons / ship.build_silver;
        assert!(
            fluyt_ton_per_peso > ship_ton_per_peso,
            "fluyt {:.4} vs ship {:.4}",
            fluyt_ton_per_peso,
            ship_ton_per_peso
        );
    }
}
