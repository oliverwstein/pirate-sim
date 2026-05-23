//! Trade-good registry.
//!
//! A `Good` is a single tradeable commodity in the simulation — sugar,
//! rum, provisions, etc. Each good has a stable identifier (`GoodId`),
//! a category, a reference Caribbean price, an optional Europe price
//! (for goods that flow off-map), and perishability metadata.
//!
//! The catalog is loaded from `data/registries/goods.ron`. Position in
//! that file determines the `GoodId` — re-ordering breaks the `ids::*`
//! constants and the `ids_resolve_to_expected_goods` test will catch it.

use serde::Deserialize;

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct GoodId(pub u8);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize)]
pub enum GoodCategory {
    /// Bulk agricultural commodity (sugar, tobacco, molasses).
    Staple,
    /// Higher-value processed export (rum).
    Cash,
    /// European-finished imports (textiles, metalware, tools).
    Manufactured,
    /// Specie / bullion.
    Currency,
    /// Naval stores (tar, pitch, masts, cordage).
    Naval,
    /// Food rations.
    Provision,
    /// Enslaved persons. Tracked here as a normal good for economic
    /// fidelity; the simulation does not depict narrative content.
    Person,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize)]
pub enum Perishability {
    /// Effectively unlimited shelf life (rum, silver, manufactures).
    Indefinite,
    /// Months of storage before quality loss (sugar, tobacco).
    Months(u8),
    /// Days of storage before spoilage (fresh provisions). Phase 2
    /// records the field but does not simulate spoilage; Phase 3 may.
    Days(u16),
}

/// On-disk shape of one good. The runtime `Good` is built from this by
/// stamping in the index-derived `GoodId`.
#[derive(Clone, Debug, Deserialize)]
struct GoodRecord {
    name: String,
    category: GoodCategory,
    tons_per_unit: f32,
    base_price_pesos: f32,
    europe_price_pesos: f32,
    perishability: Perishability,
}

#[derive(Clone, Debug)]
pub struct Good {
    pub id: GoodId,
    pub name: String,
    pub category: GoodCategory,
    /// Tons per nominal trade unit (hogshead, barrel, etc.). Tons are
    /// the canonical accounting unit on ships and in port stockpiles.
    pub tons_per_unit: f32,
    /// Reference Caribbean price in pesos per ton.
    pub base_price_pesos: f32,
    /// Per-ton London / European price, or 0.0 if the good is not
    /// Europe-bound. Used by Atlantic-gateway ports as an off-map
    /// demand sink.
    pub europe_price_pesos: f32,
    pub perishability: Perishability,
}

pub struct GoodsRegistry {
    goods: Vec<Good>,
}

/// The bundled RON catalog, compiled into the binary. Editing this file
/// requires a rebuild for now; a runtime path-loader can be added later.
const GOODS_RON: &str = include_str!("../../../data/registries/goods.ron");

impl GoodsRegistry {
    /// The starter goods, loaded from the bundled `goods.ron`. Panics
    /// if the file fails to parse — that would be a build-time bug.
    pub fn starter() -> Self {
        Self::from_ron_str(GOODS_RON).expect("bundled goods.ron must parse")
    }

    /// Parse a goods catalog from a RON string. Records are tagged with
    /// `GoodId` equal to their position in the list.
    pub fn from_ron_str(s: &str) -> Result<Self, ron::error::SpannedError> {
        let records: Vec<GoodRecord> = ron::from_str(s)?;
        let goods = records
            .into_iter()
            .enumerate()
            .map(|(i, r)| Good {
                id: GoodId(i as u8),
                name: r.name,
                category: r.category,
                tons_per_unit: r.tons_per_unit,
                base_price_pesos: r.base_price_pesos,
                europe_price_pesos: r.europe_price_pesos,
                perishability: r.perishability,
            })
            .collect();
        Ok(Self { goods })
    }

    pub fn len(&self) -> usize {
        self.goods.len()
    }
    pub fn is_empty(&self) -> bool {
        self.goods.is_empty()
    }

    pub fn get(&self, id: GoodId) -> &Good {
        &self.goods[id.0 as usize]
    }

    pub fn try_get(&self, id: GoodId) -> Option<&Good> {
        self.goods.get(id.0 as usize)
    }

    pub fn by_name(&self, name: &str) -> Option<GoodId> {
        self.goods
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(name))
            .map(|g| g.id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Good> {
        self.goods.iter()
    }
}

/// Stable handles for the starter goods. Code that needs to refer to a
/// specific commodity (the provisions resupply path, AI heuristics for
/// sugar, etc.) should use these instead of literal indices.
pub mod ids {
    use super::GoodId;
    pub const PROVISIONS: GoodId = GoodId(0);
    pub const SUGAR: GoodId = GoodId(1);
    pub const MOLASSES: GoodId = GoodId(2);
    pub const RUM: GoodId = GoodId(3);
    pub const TOBACCO: GoodId = GoodId(4);
    pub const MANUFACTURES: GoodId = GoodId(5);
    pub const NAVAL_STORES: GoodId = GoodId(6);
    pub const SILVER: GoodId = GoodId(7);
    pub const ENSLAVED_PERSONS: GoodId = GoodId(8);
    /// Step 7: gunpowder, consumed by ships per broadside. Produced
    /// at European powder mills (London, Amsterdam, Cadiz).
    pub const GUNPOWDER: GoodId = GoodId(9);
    /// Step 7: cast-iron cannon shot, consumed alongside gunpowder.
    /// Produced at the same European arsenals.
    pub const CANNON_SHOT: GoodId = GoodId(10);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_has_eleven_goods() {
        let registry = GoodsRegistry::starter();
        assert_eq!(registry.len(), 11);
    }

    #[test]
    fn goodids_match_indices() {
        let registry = GoodsRegistry::starter();
        for (i, good) in registry.iter().enumerate() {
            assert_eq!(good.id.0 as usize, i);
        }
    }

    #[test]
    fn by_name_is_case_insensitive() {
        let registry = GoodsRegistry::starter();
        assert_eq!(registry.by_name("sugar"), None); // full name only
        assert_eq!(registry.by_name("Muscovado Sugar"), Some(ids::SUGAR));
        assert_eq!(registry.by_name("muscovado sugar"), Some(ids::SUGAR));
        assert_eq!(registry.by_name("RUM"), Some(ids::RUM));
        assert_eq!(registry.by_name("nonexistent"), None);
    }

    #[test]
    fn ids_resolve_to_expected_goods() {
        let registry = GoodsRegistry::starter();
        assert_eq!(registry.get(ids::PROVISIONS).name, "Provisions");
        assert_eq!(registry.get(ids::SUGAR).name, "Muscovado Sugar");
        assert_eq!(
            registry.get(ids::MANUFACTURES).category,
            GoodCategory::Manufactured
        );
        assert_eq!(registry.get(ids::SILVER).category, GoodCategory::Currency);
    }

    #[test]
    fn europe_prices_reflect_export_economy() {
        let registry = GoodsRegistry::starter();
        // Caribbean exports fetch a premium in Europe.
        assert!(
            registry.get(ids::SUGAR).europe_price_pesos > registry.get(ids::SUGAR).base_price_pesos
        );
        // Manufactures are cheaper in Europe (the source).
        assert!(
            registry.get(ids::MANUFACTURES).europe_price_pesos
                < registry.get(ids::MANUFACTURES).base_price_pesos
        );
        // Provisions don't go to Europe.
        assert_eq!(registry.get(ids::PROVISIONS).europe_price_pesos, 0.0);
    }

    #[test]
    fn try_get_handles_out_of_range() {
        let registry = GoodsRegistry::starter();
        assert!(registry.try_get(GoodId(255)).is_none());
        assert!(registry.try_get(ids::SUGAR).is_some());
    }
}
