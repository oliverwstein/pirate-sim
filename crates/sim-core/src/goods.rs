//! Trade-good registry.
//!
//! A `Good` is a single tradeable commodity in the simulation — sugar,
//! rum, provisions, etc. Each good has a stable identifier (`GoodId`),
//! a category, a reference Caribbean price, an optional Europe price
//! (for goods that flow off-map), and perishability metadata.
//!
//! Phase 2 starts with 9 hardcoded goods (see `GoodsRegistry::starter`).
//! A RON loader will replace the hardcoded list in a follow-up step;
//! the public API stays the same.

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct GoodId(pub u8);

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
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

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Perishability {
    /// Effectively unlimited shelf life (rum, silver, manufactures).
    Indefinite,
    /// Months of storage before quality loss (sugar, tobacco).
    Months(u8),
    /// Days of storage before spoilage (fresh provisions). Phase 2
    /// records the field but does not simulate spoilage; Phase 3 may.
    Days(u16),
}

#[derive(Clone, Debug)]
pub struct Good {
    pub id: GoodId,
    pub name: &'static str,
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

impl GoodsRegistry {
    /// The 9 starter goods for Phase 2. Order is stable: each
    /// good's `GoodId` equals its index in this list.
    pub fn starter() -> Self {
        use GoodCategory::*;
        use Perishability::*;
        let entries = [
            ("Provisions", Provision, 1.0, 18.0, 0.0, Days(180)),
            ("Muscovado Sugar", Staple, 1.0, 70.0, 130.0, Months(12)),
            ("Molasses", Staple, 1.0, 25.0, 35.0, Indefinite),
            ("Rum", Cash, 1.0, 200.0, 280.0, Indefinite),
            ("Tobacco", Staple, 1.0, 40.0, 90.0, Months(24)),
            ("Manufactures", Manufactured, 1.0, 250.0, 200.0, Indefinite),
            ("Naval Stores", Naval, 1.0, 80.0, 110.0, Months(24)),
            ("Spanish Silver", Currency, 0.5, 1000.0, 1000.0, Indefinite),
            ("Enslaved Persons", Person, 0.5, 600.0, 0.0, Days(60)),
        ];
        let goods = entries
            .into_iter()
            .enumerate()
            .map(|(i, (name, category, tpu, base, eur, perish))| Good {
                id: GoodId(i as u8),
                name,
                category,
                tons_per_unit: tpu,
                base_price_pesos: base,
                europe_price_pesos: eur,
                perishability: perish,
            })
            .collect();
        Self { goods }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_has_nine_goods() {
        let registry = GoodsRegistry::starter();
        assert_eq!(registry.len(), 9);
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
