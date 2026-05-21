//! Per-port market: stockpiles, prices, production recipe.
//!
//! A `PortMarket` is the economic mirror of a `Port`. While the `Port`
//! describes physical/political identity (name, position, faction,
//! harbor radius), the `PortMarket` tracks the goods flowing through it.
//!
//! Phase 2 adds the data structure and pricing rule. Step 4 will wire
//! the monthly production tick. Step 5 will plumb buy/sell through the
//! market and expose silver. Until then, every market starts with a
//! large stockpile of every good and uses fixed (base-price) pricing.

use crate::cargo::Cargo;
use crate::goods::{GoodId, GoodsRegistry};

/// Initial stockpile per good when a market is first constructed. Big
/// enough that no port runs dry until production/consumption is wired
/// in step 4.
const INITIAL_STOCKPILE_TONS: f32 = 1000.0;

/// Starting silver in a port's treasury. Used to settle ship sales
/// against the port. Big enough that no port goes broke in the first
/// month of trading; production tick doesn't (yet) replenish it.
const INITIAL_PORT_SILVER_PESOS: f32 = 50_000.0;

/// Buy/sell spread (port "vig"). Buying costs base × (1 + SPREAD);
/// selling earns base × (1 - SPREAD). Stops infinite arbitrage of a
/// stationary stockpile.
const PRICE_SPREAD: f32 = 0.05;

/// Linearity of the supply-driven price modulation. price =
/// base × (1 + PRICE_K × (target - current)/target), clamped to
/// [PRICE_FLOOR_FRAC × base, PRICE_CEIL_FRAC × base].
pub const PRICE_K: f32 = 1.0;
pub const PRICE_FLOOR_FRAC: f32 = 0.25;
pub const PRICE_CEIL_FRAC: f32 = 4.0;

/// What a port produces and consumes each simulated month. Outputs are
/// added to stockpiles; inputs are deducted (clamped at zero — a port
/// that lacks inputs simply produces less, modeled in step 4).
#[derive(Clone, Debug, Default)]
pub struct ProductionRecipe {
    pub monthly_outputs: Vec<(GoodId, f32)>,
    pub monthly_inputs: Vec<(GoodId, f32)>,
    /// Multiplier on outputs. 1.0 = baseline historical estimate;
    /// >1.0 boom, <1.0 stagnation. Used by step 9 calibration.
    pub prosperity: f32,
}

impl ProductionRecipe {
    pub fn empty() -> Self {
        Self {
            monthly_outputs: Vec::new(),
            monthly_inputs: Vec::new(),
            prosperity: 1.0,
        }
    }
}

/// Economic state of a port.
pub struct PortMarket {
    pub stockpile: Cargo,
    /// Pesos in the port's treasury. Unused until step 5.
    pub silver: f32,
    pub recipe: ProductionRecipe,
}

impl PortMarket {
    /// Construct a market with a uniform initial stockpile of every
    /// good in the registry. Useful for tests; `World::load` instead
    /// uses `with_recipe` to seed each port to its target stock so
    /// prices start near base.
    pub fn with_initial_stockpile(registry: &GoodsRegistry) -> Self {
        let mut stockpile = Cargo::new();
        for good in registry.iter() {
            stockpile.add(good.id, INITIAL_STOCKPILE_TONS);
        }
        Self {
            stockpile,
            silver: INITIAL_PORT_SILVER_PESOS,
            recipe: ProductionRecipe::empty(),
        }
    }

    /// Construct a market initialized to the recipe's *target* stock
    /// Construct a market keyed off a `ProductionRecipe`. Seeds an
    /// asymmetric opening stockpile that reflects each port's role:
    ///
    /// * Output goods start at **12× monthly output** (~2× the price
    ///   target), so a producer port enters with surplus and
    ///   therefore *cheap* prices for what it makes.
    /// * Input goods start at **3× monthly input** (half the price
    ///   target), so a consumer port enters with shortage and
    ///   therefore *expensive* prices for what it consumes — but it
    ///   does still hold a real stockpile, e.g. so a sugar island
    ///   has provisions to sell to visiting ships at high prices
    ///   rather than going completely dry.
    ///
    /// This puts arbitrage on the table from tick 0 instead of waiting
    /// for the first monthly production+consumption pass to skew the
    /// stockpiles. Without it, every recipe-good would start at
    /// exactly its target stockpile and prices would be flat (= base
    /// price) everywhere on day 0, so ships would dock and undock
    /// empty until the economy warmed up.
    pub fn with_recipe(
        _registry: &GoodsRegistry,
        recipe: ProductionRecipe,
    ) -> Self {
        let mut stockpile = Cargo::new();
        for (id, tons) in &recipe.monthly_outputs {
            stockpile.add(*id, *tons * 12.0);
        }
        for (id, tons) in &recipe.monthly_inputs {
            // If a good is both produced and consumed (rare), the
            // output pass already gave it the surplus seeding — don't
            // overwrite that with a shortage value.
            if stockpile.get(*id) <= 0.0 {
                stockpile.add(*id, *tons * 3.0);
            }
        }
        Self {
            stockpile,
            silver: INITIAL_PORT_SILVER_PESOS,
            recipe,
        }
    }

    /// Apply one month of the recipe: outputs are produced and added
    /// to stockpiles (scaled by `prosperity`); inputs are consumed
    /// from stockpiles (clamped at zero — a port that lacks an input
    /// simply loses the value and produces nothing extra). Step 4 v1
    /// applies output and input independently; coupling production to
    /// input availability is a refinement deferred to Phase 3.
    pub fn tick_month(&mut self) {
        let prosperity = self.recipe.prosperity.max(0.0);
        for (id, tons) in self.recipe.monthly_outputs.clone() {
            self.stockpile.add(id, tons * prosperity);
        }
        for (id, tons) in self.recipe.monthly_inputs.clone() {
            self.stockpile.remove(id, tons * prosperity);
        }
    }

    /// Pesos-per-ton local price for `id`, factoring stockpile.
    /// Stockpile-driven modulation kicks in only once a `target` is
    /// declared via the recipe; until then the base price is returned.
    pub fn price(&self, id: GoodId, registry: &GoodsRegistry) -> f32 {
        let good = registry.get(id);
        let target = self.target_stock(id);
        if target <= 0.0 {
            return good.base_price_pesos;
        }
        let current = self.stockpile.get(id);
        let factor = 1.0 + PRICE_K * (target - current) / target;
        let clamped = factor.clamp(PRICE_FLOOR_FRAC, PRICE_CEIL_FRAC);
        good.base_price_pesos * clamped
    }

    /// Price a ship pays per ton to buy `id` (above the local mid).
    /// Pure local price + spread — no gateway magic. Europe is on the
    /// map as four real ports (London, Amsterdam, Cadiz, Nantes); to
    /// trade with Europe a ship sails there.
    pub fn buy_price(&self, id: GoodId, registry: &GoodsRegistry) -> f32 {
        self.price(id, registry) * (1.0 + PRICE_SPREAD)
    }

    /// Price a ship receives per ton selling `id` (below the local mid).
    pub fn sell_price(&self, id: GoodId, registry: &GoodsRegistry) -> f32 {
        self.price(id, registry) * (1.0 - PRICE_SPREAD)
    }

    /// Implicit "target" stockpile: 6 months of the recipe's output
    /// (or input). When neither is set, returns 0.0 → flat pricing.
    fn target_stock(&self, id: GoodId) -> f32 {
        let from_outputs = self.recipe.monthly_outputs
            .iter()
            .find(|(g, _)| *g == id)
            .map(|(_, t)| *t * 6.0);
        let from_inputs = self.recipe.monthly_inputs
            .iter()
            .find(|(g, _)| *g == id)
            .map(|(_, t)| *t * 6.0);
        from_outputs.or(from_inputs).unwrap_or(0.0)
    }

    /// Buy `requested_tons` of `id` from this market on behalf of `ship`.
    /// Atomic: either the full requested amount transacts or nothing
    /// changes. Returns the cost in pesos on success.
    pub fn buy(
        &mut self,
        ship: &mut crate::ship::Ship,
        ship_stats: &crate::ship::ShipStats,
        id: GoodId,
        requested_tons: f32,
        registry: &GoodsRegistry,
    ) -> Result<f32, TradeError> {
        if requested_tons <= 0.0 {
            return Err(TradeError::NonPositiveAmount);
        }
        let unit = self.buy_price(id, registry);
        let cost = unit * requested_tons;
        if cost > ship.silver + 1e-4 {
            return Err(TradeError::InsufficientSilver);
        }
        let cargo_room = ship_stats.cargo_capacity_tons - ship.cargo.total_tons();
        if requested_tons > cargo_room + 1e-4 {
            return Err(TradeError::InsufficientCargoSpace);
        }
        if requested_tons > self.stockpile.get(id) + 1e-4 {
            return Err(TradeError::InsufficientStockpile);
        }
        ship.silver -= cost;
        self.silver += cost;
        self.stockpile.remove(id, requested_tons);
        ship.cargo.add(id, requested_tons);
        Ok(cost)
    }

    /// Sell `requested_tons` of `id` from `ship` into this market.
    /// Atomic. Returns proceeds in pesos on success.
    pub fn sell(
        &mut self,
        ship: &mut crate::ship::Ship,
        id: GoodId,
        requested_tons: f32,
        registry: &GoodsRegistry,
    ) -> Result<f32, TradeError> {
        if requested_tons <= 0.0 {
            return Err(TradeError::NonPositiveAmount);
        }
        if requested_tons > ship.cargo.get(id) + 1e-4 {
            return Err(TradeError::InsufficientShipCargo);
        }
        let unit = self.sell_price(id, registry);
        let proceeds = unit * requested_tons;
        if proceeds > self.silver + 1e-4 {
            return Err(TradeError::InsufficientPortSilver);
        }
        ship.cargo.remove(id, requested_tons);
        self.stockpile.add(id, requested_tons);
        self.silver -= proceeds;
        ship.silver += proceeds;
        Ok(proceeds)
    }
}

/// Why a buy/sell transaction was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeError {
    NonPositiveAmount,
    InsufficientSilver,
    InsufficientCargoSpace,
    InsufficientStockpile,
    InsufficientShipCargo,
    InsufficientPortSilver,
}

/// Historical archetype for a port, used to assign a default recipe.
#[derive(Clone, Copy, Debug)]
pub enum PortArchetype {
    /// Sugar/molasses/rum producer; imports provisions, manufactures,
    /// enslaved labor (Bridgetown, Port Royal, Martinique, ...).
    SugarIsland,
    /// Tobacco / cacao producer; imports provisions and manufactures.
    TobaccoColony,
    /// North Atlantic provisioner; produces salt-meat / flour / naval
    /// stores; imports sugar, rum, manufactures (Boston, Philadelphia).
    NorthAmericanFarming,
    /// Spanish silver/treasure port (Cartagena, Portobelo, Veracruz);
    /// produces silver; imports manufactures, provisions, enslaved.
    SpanishTreasure,
    /// Spanish secondary entrepôt — modest sugar/tobacco, treasure
    /// transshipment (Havana, Santo Domingo, San Juan).
    SpanishEntrepot,
    /// Pirate haven: consumes provisions/rum/manufactures, produces
    /// nothing (the goods come in by other means).
    PirateHaven,
    /// Generic minor port: imports provisions and manufactures.
    Minor,
    /// London — the largest European entrepôt for the Atlantic trade.
    /// Big sugar refiner, biggest tobacco re-exporter, finance and
    /// shipping center; outputs manufactures (cloth/iron/firearms).
    EuropeanLondon,
    /// Amsterdam — Dutch entrepôt; sugar refining, banking,
    /// re-exports Asian textiles. Outputs manufactures + naval stores
    /// (Baltic timber/tar via Dutch shipping).
    EuropeanAmsterdam,
    /// Cadiz — Spanish silver entry point. Modest manufactures output;
    /// dominant feature is the silver sink (silver flows in from the
    /// Spanish Main and is consumed locally / passed on outside our
    /// model to mint/Asia).
    EuropeanCadiz,
    /// Nantes — leading French slave-trade and sugar-refining port.
    /// Outputs manufactures (textiles, brandy → modeled as
    /// manufactures + rum).
    EuropeanNantes,
    /// Elmina — Gold Coast slave-trade fort. Demands manufactures,
    /// rum, tobacco; produces enslaved persons.
    AfricanElmina,
    /// Ouidah — Bight of Benin slave-trade port. Same trade goods but
    /// regional differences (more rum, less iron).
    AfricanOuidah,
}

impl PortArchetype {
    /// Build a recipe for this archetype. Numbers are tons/month at
    /// prosperity 1.0 — calibrated coarsely against `production-model.md`.
    pub fn recipe(self) -> ProductionRecipe {
        use crate::goods::ids::*;
        let (outputs, inputs): (&[(GoodId, f32)], &[(GoodId, f32)]) = match self {
            PortArchetype::SugarIsland => (
                &[(SUGAR, 80.0), (MOLASSES, 30.0), (RUM, 15.0)],
                &[(PROVISIONS, 12.0), (MANUFACTURES, 6.0), (ENSLAVED_PERSONS, 3.0), (NAVAL_STORES, 2.0)],
            ),
            PortArchetype::TobaccoColony => (
                &[(TOBACCO, 60.0)],
                &[(PROVISIONS, 6.0), (MANUFACTURES, 4.0), (ENSLAVED_PERSONS, 2.0)],
            ),
            PortArchetype::NorthAmericanFarming => (
                &[(PROVISIONS, 60.0), (NAVAL_STORES, 30.0), (RUM, 10.0)],
                &[(SUGAR, 15.0), (MOLASSES, 25.0), (MANUFACTURES, 10.0)],
            ),
            PortArchetype::SpanishTreasure => (
                // Spanish Main mainland (Cartagena, Portobelo) had
                // significant cattle ranching and maize agriculture
                // in the hinterland — historically a *net* provisions
                // exporter, victualling fleets and treasure convoys.
                &[(SILVER, 5.0), (PROVISIONS, 30.0)],
                &[(MANUFACTURES, 8.0), (ENSLAVED_PERSONS, 2.0)],
            ),
            PortArchetype::SpanishEntrepot => (
                // Cuba/Hispaniola: cattle, cassava, plantains. Modest
                // provisions surplus on top of sugar/tobacco re-export.
                &[(SUGAR, 20.0), (TOBACCO, 15.0), (PROVISIONS, 20.0)],
                &[(MANUFACTURES, 5.0)],
            ),
            PortArchetype::PirateHaven => (
                &[],
                &[(PROVISIONS, 4.0), (RUM, 3.0), (MANUFACTURES, 2.0)],
            ),
            PortArchetype::Minor => (
                &[],
                &[(PROVISIONS, 2.0), (MANUFACTURES, 1.0)],
            ),
            // === EUROPE ===
            // Numbers reflect each port's relative economic gravity
            // (London ~3× Amsterdam ~Nantes >> Cadiz). Sketched from
            // general knowledge; revisit once
            // `planning/research/european-markets.md` lands.
            //
            // All European ports are net food producers — England,
            // Holland, France, and Spain all victualled their own
            // fleets and exported grain/salt-meat to colonies. So
            // every European archetype produces provisions in addition
            // to manufactures.
            PortArchetype::EuropeanLondon => (
                &[(MANUFACTURES, 200.0), (PROVISIONS, 100.0)],
                &[
                    (SUGAR, 200.0),
                    (TOBACCO, 80.0),
                    (RUM, 30.0),
                    (MOLASSES, 30.0),
                    (NAVAL_STORES, 20.0),
                    (SILVER, 0.3),
                ],
            ),
            PortArchetype::EuropeanAmsterdam => (
                &[(MANUFACTURES, 120.0), (NAVAL_STORES, 30.0), (PROVISIONS, 80.0)],
                &[
                    (SUGAR, 120.0),
                    (TOBACCO, 30.0),
                    (RUM, 15.0),
                    (MOLASSES, 15.0),
                    (SILVER, 0.2),
                ],
            ),
            PortArchetype::EuropeanCadiz => (
                &[(MANUFACTURES, 30.0), (PROVISIONS, 60.0)],
                &[
                    (SILVER, 1.0),
                    (SUGAR, 40.0),
                    (TOBACCO, 20.0),
                ],
            ),
            PortArchetype::EuropeanNantes => (
                &[(MANUFACTURES, 80.0), (RUM, 20.0), (PROVISIONS, 70.0)],
                &[
                    (SUGAR, 80.0),
                    (TOBACCO, 30.0),
                    (MOLASSES, 20.0),
                ],
            ),
            // === WEST AFRICA ===
            // Slave-trade ports. Captives sourced from inland trade
            // networks (modeled as monthly output of ENSLAVED_PERSONS).
            // Inputs are the trade-goods baskets European/Caribbean
            // ships paid in: textiles + ironware + firearms collapsed
            // into MANUFACTURES, plus alcohol (rum) and tobacco. No
            // silver — slave-coast traders generally took goods, not
            // coin. Differentiated regionally: Ouidah took relatively
            // more rum, Elmina relatively more iron/manufactures.
            PortArchetype::AfricanElmina => (
                &[(ENSLAVED_PERSONS, 8.0)],
                &[
                    (MANUFACTURES, 12.0),
                    (RUM, 4.0),
                    (TOBACCO, 3.0),
                ],
            ),
            PortArchetype::AfricanOuidah => (
                &[(ENSLAVED_PERSONS, 10.0)],
                &[
                    (MANUFACTURES, 8.0),
                    (RUM, 8.0),
                    (TOBACCO, 4.0),
                ],
            ),
        };
        ProductionRecipe {
            monthly_outputs: outputs.to_vec(),
            monthly_inputs: inputs.to_vec(),
            prosperity: 1.0,
        }
    }
}

/// Map a port name to its archetype. Europe and West Africa are now
/// real nodes on the map (not "gateway" magic on Caribbean ports);
/// transatlantic trade is just the longest arbitrage in the system.
pub fn archetype_for(port_name: &str) -> PortArchetype {
    use PortArchetype::*;
    match port_name {
        // Sugar islands
        "Bridgetown" => SugarIsland,
        "Port Royal" | "Kingston" => SugarIsland,
        "Basseterre" | "English Harbour" => SugarIsland,
        "Fort-Royal" | "Basse-Terre" => SugarIsland,
        "Cap-Français" | "Petit-Goâve" => SugarIsland,
        "Paramaribo" | "Cayenne" => SugarIsland,
        "Willemstad" | "St. Eustatius" => SugarIsland,

        // Tobacco
        "Charleston" => TobaccoColony,

        // North American farming
        "Boston" | "Philadelphia" | "New York" => NorthAmericanFarming,

        // Spanish silver
        "Cartagena" | "Portobelo" => SpanishTreasure,

        // Spanish entrepôt
        "Havana" | "Santo Domingo" | "Santiago de Cuba" | "San Juan" => SpanishEntrepot,

        // Pirate
        "Tortuga" | "Nassau" | "Tobago" => PirateHaven,

        // Bermuda — naval stores transshipment
        "Bermuda" => NorthAmericanFarming,

        // Europe
        "London" => EuropeanLondon,
        "Amsterdam" => EuropeanAmsterdam,
        "Cadiz" => EuropeanCadiz,
        "Nantes" => EuropeanNantes,

        // West Africa
        "Elmina" => AfricanElmina,
        "Ouidah" => AfricanOuidah,

        // Minor / other
        _ => Minor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goods::ids;

    #[test]
    fn fresh_market_holds_every_good() {
        let registry = GoodsRegistry::starter();
        let market = PortMarket::with_initial_stockpile(&registry);
        for good in registry.iter() {
            assert_eq!(market.stockpile.get(good.id), INITIAL_STOCKPILE_TONS);
        }
    }

    #[test]
    fn flat_price_without_recipe() {
        let registry = GoodsRegistry::starter();
        let market = PortMarket::with_initial_stockpile(&registry);
        let sugar_base = registry.get(ids::SUGAR).base_price_pesos;
        assert_eq!(market.price(ids::SUGAR, &registry), sugar_base);
    }

    #[test]
    fn buy_and_sell_have_spread() {
        let registry = GoodsRegistry::starter();
        let market = PortMarket::with_initial_stockpile(&registry);
        let mid = market.price(ids::RUM, &registry);
        let buy = market.buy_price(ids::RUM, &registry);
        let sell = market.sell_price(ids::RUM, &registry);
        assert!(buy > mid);
        assert!(sell < mid);
        // Spread is symmetric.
        assert!(((buy - mid) - (mid - sell)).abs() < 1e-3);
    }

    #[test]
    fn surplus_lowers_price_under_recipe() {
        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_initial_stockpile(&registry);
        // Sugar plantation: produces 50 t/month; target = 6×50 = 300.
        market.recipe.monthly_outputs.push((ids::SUGAR, 50.0));
        // Initial 1000 t >> 300 target → prices should crash to floor.
        let base = registry.get(ids::SUGAR).base_price_pesos;
        let p = market.price(ids::SUGAR, &registry);
        assert!(p < base, "Surplus should depress price (got {} >= {})", p, base);
        assert!(p >= base * PRICE_FLOOR_FRAC - 1e-3);
    }

    #[test]
    fn shortage_raises_price_under_recipe() {
        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_initial_stockpile(&registry);
        // Plantation that *consumes* manufactures: target ≈ inputs × 6.
        market.recipe.monthly_inputs.push((ids::MANUFACTURES, 200.0));
        // Target = 1200 > stockpile 1000 → mild shortage premium.
        let base = registry.get(ids::MANUFACTURES).base_price_pesos;
        let p = market.price(ids::MANUFACTURES, &registry);
        assert!(p > base, "Shortage should raise price (got {} <= {})", p, base);
        assert!(p <= base * PRICE_CEIL_FRAC + 1e-3);
    }

    #[test]
    fn price_clamps_to_floor_when_oversupplied() {
        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_initial_stockpile(&registry);
        market.recipe.monthly_outputs.push((ids::SUGAR, 1.0));
        // Stockpile (1000) wildly exceeds tiny target (6) → clamp to floor.
        let base = registry.get(ids::SUGAR).base_price_pesos;
        let p = market.price(ids::SUGAR, &registry);
        assert!((p - base * PRICE_FLOOR_FRAC).abs() < 1e-3);
    }

    #[test]
    fn price_rises_when_starved() {
        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_initial_stockpile(&registry);
        market.recipe.monthly_inputs.push((ids::SUGAR, 1000.0));
        market.stockpile.remove(ids::SUGAR, INITIAL_STOCKPILE_TONS);
        let base = registry.get(ids::SUGAR).base_price_pesos;
        // Empty stockpile, large demand → factor ≈ 2.0 (K=1 caps natural
        // scarcity at 2× base; the 4× ceiling is a runaway-safety rail).
        let p = market.price(ids::SUGAR, &registry);
        assert!((p - base * 2.0).abs() < 1e-3);
        assert!(p <= base * PRICE_CEIL_FRAC + 1e-3);
    }

    #[test]
    fn with_recipe_starts_with_asymmetric_stockpile() {
        let registry = GoodsRegistry::starter();
        let recipe = PortArchetype::SugarIsland.recipe();
        let market = PortMarket::with_recipe(&registry, recipe.clone());
        // Outputs sit at 12× monthly throughput (year of surplus).
        for (id, tons) in &recipe.monthly_outputs {
            assert_eq!(market.stockpile.get(*id), *tons * 12.0);
        }
        // Inputs sit at 3× monthly throughput (3 months' buffer).
        for (id, tons) in &recipe.monthly_inputs {
            assert_eq!(market.stockpile.get(*id), *tons * 3.0);
        }
        // Output prices should be cheap (surplus); input prices expensive.
        let sugar_base = registry.get(ids::SUGAR).base_price_pesos;
        let manuf_base = registry.get(ids::MANUFACTURES).base_price_pesos;
        assert!(market.price(ids::SUGAR, &registry) < sugar_base,
            "producer port should price its output below base");
        assert!(market.price(ids::MANUFACTURES, &registry) > manuf_base,
            "consumer port should price its input above base");
    }

    #[test]
    fn tick_month_produces_outputs_and_consumes_inputs() {
        let registry = GoodsRegistry::starter();
        let recipe = PortArchetype::SugarIsland.recipe();
        let mut market = PortMarket::with_recipe(&registry, recipe.clone());
        let sugar_before = market.stockpile.get(ids::SUGAR);
        let provisions_before = market.stockpile.get(ids::PROVISIONS);

        market.tick_month();

        // Sugar (output) increased by its monthly figure.
        let sugar_out = recipe.monthly_outputs.iter()
            .find(|(g, _)| *g == ids::SUGAR).unwrap().1;
        assert!((market.stockpile.get(ids::SUGAR) - (sugar_before + sugar_out)).abs() < 1e-3);
        // Provisions (input) decreased by its monthly figure.
        let prov_in = recipe.monthly_inputs.iter()
            .find(|(g, _)| *g == ids::PROVISIONS).unwrap().1;
        assert!((market.stockpile.get(ids::PROVISIONS) - (provisions_before - prov_in)).abs() < 1e-3);
    }

    #[test]
    fn tick_month_clamps_inputs_at_zero() {
        let registry = GoodsRegistry::starter();
        let mut recipe = ProductionRecipe::empty();
        recipe.monthly_inputs.push((ids::PROVISIONS, 1000.0));
        let mut market = PortMarket::with_recipe(&registry, recipe);
        // Input stockpile starts at 3× monthly = 3000. Three ticks
        // drain it to zero exactly.
        market.tick_month();
        assert_eq!(market.stockpile.get(ids::PROVISIONS), 2000.0);
        market.tick_month();
        market.tick_month();
        assert_eq!(market.stockpile.get(ids::PROVISIONS), 0.0);
        // Run more ticks; stays at zero (clamped, no negatives).
        for _ in 0..10 {
            market.tick_month();
        }
        assert_eq!(market.stockpile.get(ids::PROVISIONS), 0.0);
    }

    #[test]
    fn archetype_for_known_ports() {
        assert!(matches!(archetype_for("Bridgetown"), PortArchetype::SugarIsland));
        assert!(matches!(archetype_for("Boston"), PortArchetype::NorthAmericanFarming));
        assert!(matches!(archetype_for("Cartagena"), PortArchetype::SpanishTreasure));
        assert!(matches!(archetype_for("Tortuga"), PortArchetype::PirateHaven));
        assert!(matches!(archetype_for("London"), PortArchetype::EuropeanLondon));
        assert!(matches!(archetype_for("Cadiz"), PortArchetype::EuropeanCadiz));
        assert!(matches!(archetype_for("Elmina"), PortArchetype::AfricanElmina));
        assert!(matches!(archetype_for("Ouidah"), PortArchetype::AfricanOuidah));
        // Unknown port falls back to Minor.
        assert!(matches!(archetype_for("Atlantis"), PortArchetype::Minor));
    }

    #[test]
    fn buy_transfers_goods_silver_and_stockpile() {
        use crate::ship::{Ship, ShipState, ShipStats};
        use crate::types::Position;

        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&registry, PortArchetype::SugarIsland.recipe());
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);

        let ship_silver_before = ship.silver;
        let port_silver_before = market.silver;
        let stockpile_before = market.stockpile.get(ids::SUGAR);
        let cost = market.buy(&mut ship, &stats, ids::SUGAR, 5.0, &registry).unwrap();

        // Ship paid; port received; stockpile decreased; cargo grew.
        assert!((ship.silver - (ship_silver_before - cost)).abs() < 1e-3);
        assert!((market.silver - (port_silver_before + cost)).abs() < 1e-3);
        assert!((market.stockpile.get(ids::SUGAR) - (stockpile_before - 5.0)).abs() < 1e-3);
        assert!((ship.cargo.get(ids::SUGAR) - 5.0).abs() < 1e-3);
    }

    #[test]
    fn buy_rejects_when_broke() {
        use crate::ship::{Ship, ShipState, ShipStats};
        use crate::types::Position;

        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&registry, PortArchetype::SugarIsland.recipe());
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.silver = 1.0;
        let result = market.buy(&mut ship, &stats, ids::SUGAR, 50.0, &registry);
        assert_eq!(result, Err(TradeError::InsufficientSilver));
        // Ship still has its silver; cargo still empty.
        assert_eq!(ship.silver, 1.0);
        assert!(ship.cargo.is_empty());
    }

    #[test]
    fn buy_rejects_when_hold_full() {
        use crate::ship::{Ship, ShipState, ShipStats};
        use crate::types::Position;

        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&registry, PortArchetype::SugarIsland.recipe());
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        // Pre-load cargo to capacity.
        ship.cargo.add(ids::TOBACCO, stats.cargo_capacity_tons);
        let result = market.buy(&mut ship, &stats, ids::SUGAR, 1.0, &registry);
        assert_eq!(result, Err(TradeError::InsufficientCargoSpace));
    }

    #[test]
    fn sell_round_trip_loses_money_to_spread() {
        use crate::ship::{Ship, ShipState, ShipStats};
        use crate::types::Position;

        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&registry, PortArchetype::SugarIsland.recipe());
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        let initial_silver = ship.silver;

        // Tiny round trip: buy 1t and immediately sell it back.
        market.buy(&mut ship, &stats, ids::SUGAR, 1.0, &registry).unwrap();
        market.sell(&mut ship, ids::SUGAR, 1.0, &registry).unwrap();

        // Spread must take a bite — ship strictly poorer.
        assert!(ship.silver < initial_silver);
        assert!(ship.cargo.is_empty());
    }

    #[test]
    fn sell_rejects_more_than_ship_carries() {
        use crate::ship::{Ship, ShipState};
        use crate::types::Position;

        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&registry, PortArchetype::SugarIsland.recipe());
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        let result = market.sell(&mut ship, ids::SUGAR, 1.0, &registry);
        assert_eq!(result, Err(TradeError::InsufficientShipCargo));
    }

    #[test]
    fn european_port_drained_of_imports_carries_high_sugar_premium() {
        // London's recipe consumes 200 t/mo sugar (target 1200 t).
        // Drain its stockpile fully → factor → 2.0 → ceiling-driven
        // local price ≈ 2× base. That high price is what makes the
        // transatlantic route profitable; no special "world price"
        // mechanism needed.
        let registry = GoodsRegistry::starter();
        let mut london = PortMarket::with_recipe(&registry, PortArchetype::EuropeanLondon.recipe());
        let stk = london.stockpile.get(ids::SUGAR);
        london.stockpile.remove(ids::SUGAR, stk);
        let base = registry.get(ids::SUGAR).base_price_pesos;
        let p = london.sell_price(ids::SUGAR, &registry);
        // Mid was 2× base, sell = 0.95 × that = 1.9× base.
        assert!(p > base * 1.5,
            "expected drained London sugar sell price well above base; got {} (base {})", p, base);
    }

    #[test]
    fn african_slave_port_produces_enslaved_persons() {
        let registry = GoodsRegistry::starter();
        let elmina = PortMarket::with_recipe(&registry, PortArchetype::AfricanElmina.recipe());
        // Elmina's monthly_outputs include ENSLAVED_PERSONS, so the
        // stockpile is seeded at 6× monthly output.
        assert!(elmina.stockpile.get(ids::ENSLAVED_PERSONS) > 0.0);
    }
}
