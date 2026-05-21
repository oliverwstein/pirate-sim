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

/// What Europe pays for export goods (per ton, at base price). Set so
/// a ship hauling 60 t of sugar Caribbean→England-gateway makes a
/// solid profit even after the Atlantic-leg distance cost the planner
/// will model in later phases. Tuned coarsely; revisit in step 9.
pub const WORLD_EXPORT_PREMIUM: f32 = 1.30;

/// What Europe sells imported goods for (per ton, at base price).
pub const WORLD_IMPORT_DISCOUNT: f32 = 0.90;

/// Goods Europe is an infinite *buyer* of (gateway ports always sell
/// these at a stable world price, regardless of local stockpile).
fn is_europe_export(id: GoodId) -> bool {
    use crate::goods::ids::*;
    id == SUGAR || id == MOLASSES || id == RUM || id == TOBACCO
        || id == NAVAL_STORES || id == SILVER
}

/// Goods Europe is an infinite *seller* of (gateway ports always
/// supply these at a stable world price).
fn is_europe_import(id: GoodId) -> bool {
    use crate::goods::ids::*;
    id == MANUFACTURES || id == ENSLAVED_PERSONS
}

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
    /// True for Atlantic-facing ports (Boston, Philadelphia, Charles
    /// Town, Bridgetown). Step 7 will route exports here at world price.
    pub is_europe_gateway: bool,
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
            is_europe_gateway: false,
        }
    }

    /// Construct a market initialized to the recipe's *target* stock
    /// for every good it produces or consumes. This makes the opening
    /// price for each recipe-good equal to `base_price`, so the
    /// economy starts in equilibrium and drifts as production and
    /// trade happen.
    pub fn with_recipe(
        _registry: &GoodsRegistry,
        recipe: ProductionRecipe,
        is_europe_gateway: bool,
    ) -> Self {
        let mut stockpile = Cargo::new();
        for (id, tons) in &recipe.monthly_outputs {
            stockpile.add(*id, *tons * 6.0);
        }
        for (id, tons) in &recipe.monthly_inputs {
            // If a good is both produced and consumed (rare), the
            // input pass would double-add — guard with current value.
            let already = stockpile.get(*id);
            if already <= 0.0 {
                stockpile.add(*id, *tons * 6.0);
            }
        }
        Self {
            stockpile,
            silver: INITIAL_PORT_SILVER_PESOS,
            recipe,
            is_europe_gateway,
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
    /// At a Europe-gateway port, import goods are capped at the world
    /// price — Europe is an infinite supplier, so the local market
    /// can't gouge above that ceiling.
    pub fn buy_price(&self, id: GoodId, registry: &GoodsRegistry) -> f32 {
        let local = self.price(id, registry) * (1.0 + PRICE_SPREAD);
        if self.is_europe_gateway && is_europe_import(id) {
            let world = registry.get(id).base_price_pesos * WORLD_IMPORT_DISCOUNT;
            local.min(world)
        } else {
            local
        }
    }

    /// Price a ship receives per ton selling `id` (below the local mid).
    /// At a Europe-gateway port, export goods are floored at the world
    /// price — Europe is an infinite buyer, so a local sugar glut can't
    /// crush the price below the Europe-bound ship's reservation.
    pub fn sell_price(&self, id: GoodId, registry: &GoodsRegistry) -> f32 {
        let local = self.price(id, registry) * (1.0 - PRICE_SPREAD);
        if self.is_europe_gateway && is_europe_export(id) {
            let world = registry.get(id).base_price_pesos * WORLD_EXPORT_PREMIUM;
            local.max(world)
        } else {
            local
        }
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
    ///
    /// At a Europe-gateway port, *import* goods are sourced from Europe
    /// at the world price — the local stockpile is bypassed (Europe is
    /// an infinite supplier) and the port's treasury is unaffected (the
    /// silver leaves the simulation, off-map).
    ///
    /// Fails when the ship lacks silver or the cargo hold lacks room.
    /// Fails for non-portal flows when the local market lacks stockpile.
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
        let via_europe = self.is_europe_gateway && is_europe_import(id);
        if !via_europe && requested_tons > self.stockpile.get(id) + 1e-4 {
            return Err(TradeError::InsufficientStockpile);
        }
        ship.silver -= cost;
        if !via_europe {
            self.silver += cost;
            self.stockpile.remove(id, requested_tons);
        }
        ship.cargo.add(id, requested_tons);
        Ok(cost)
    }

    /// Sell `requested_tons` of `id` from `ship` into this market.
    /// Atomic. Returns proceeds in pesos on success.
    ///
    /// At a Europe-gateway port, *export* goods are bought by Europe at
    /// the world price — the local stockpile is unaffected (the goods
    /// leave the simulation) and the silver paid to the ship is
    /// likewise sourced off-map (the local treasury is unchanged, and
    /// the port-silver check is bypassed).
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
        let via_europe = self.is_europe_gateway && is_europe_export(id);
        if !via_europe && proceeds > self.silver + 1e-4 {
            return Err(TradeError::InsufficientPortSilver);
        }
        ship.cargo.remove(id, requested_tons);
        if !via_europe {
            self.stockpile.add(id, requested_tons);
            self.silver -= proceeds;
        }
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
                &[(SILVER, 5.0)],
                &[(MANUFACTURES, 8.0), (PROVISIONS, 6.0), (ENSLAVED_PERSONS, 2.0)],
            ),
            PortArchetype::SpanishEntrepot => (
                &[(SUGAR, 20.0), (TOBACCO, 15.0)],
                &[(MANUFACTURES, 5.0), (PROVISIONS, 5.0)],
            ),
            PortArchetype::PirateHaven => (
                &[],
                &[(PROVISIONS, 4.0), (RUM, 3.0), (MANUFACTURES, 2.0)],
            ),
            PortArchetype::Minor => (
                &[],
                &[(PROVISIONS, 2.0), (MANUFACTURES, 1.0)],
            ),
        };
        ProductionRecipe {
            monthly_outputs: outputs.to_vec(),
            monthly_inputs: inputs.to_vec(),
            prosperity: 1.0,
        }
    }
}

/// Map a port name to its archetype + Europe-gateway flag. Atlantic
/// gateway ports both have local recipes *and* expose the off-map
/// world price (step 7).
pub fn archetype_for(port_name: &str) -> (PortArchetype, bool) {
    use PortArchetype::*;
    match port_name {
        // Sugar islands
        "Bridgetown" => (SugarIsland, true),  // Barbados — England gateway
        "Port Royal" | "Kingston" => (SugarIsland, false),
        "Basseterre" | "English Harbour" => (SugarIsland, false),
        "Fort-Royal" | "Basse-Terre" => (SugarIsland, false),
        "Cap-Français" | "Petit-Goâve" => (SugarIsland, false),
        "Paramaribo" | "Cayenne" => (SugarIsland, false),
        "Willemstad" | "St. Eustatius" => (SugarIsland, false),

        // Tobacco
        "Charleston" => (TobaccoColony, true),

        // North American farming
        "Boston" | "Philadelphia" | "New York" => (NorthAmericanFarming, true),

        // Spanish silver
        "Cartagena" | "Portobelo" => (SpanishTreasure, false),

        // Spanish entrepôt
        "Havana" | "Santo Domingo" | "Santiago de Cuba" | "San Juan" => (SpanishEntrepot, false),

        // Pirate
        "Tortuga" | "Nassau" | "Tobago" => (PirateHaven, false),

        // Bermuda — naval stores transshipment
        "Bermuda" => (NorthAmericanFarming, false),

        // Minor / other
        _ => (Minor, false),
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
    fn with_recipe_starts_at_target() {
        let registry = GoodsRegistry::starter();
        let recipe = PortArchetype::SugarIsland.recipe();
        let market = PortMarket::with_recipe(&registry, recipe.clone(), true);
        // Each output and input should sit at 6× monthly throughput.
        for (id, tons) in &recipe.monthly_outputs {
            assert_eq!(market.stockpile.get(*id), *tons * 6.0);
        }
        for (id, tons) in &recipe.monthly_inputs {
            assert_eq!(market.stockpile.get(*id), *tons * 6.0);
        }
        // Prices should equal base because stockpile == target.
        let base = registry.get(ids::SUGAR).base_price_pesos;
        let p = market.price(ids::SUGAR, &registry);
        assert!((p - base).abs() < 1e-3);
    }

    #[test]
    fn tick_month_produces_outputs_and_consumes_inputs() {
        let registry = GoodsRegistry::starter();
        let recipe = PortArchetype::SugarIsland.recipe();
        let mut market = PortMarket::with_recipe(&registry, recipe.clone(), false);
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
        let mut market = PortMarket::with_recipe(&registry, recipe, false);
        // Stockpile = 6000 (target), tick_month consumes 1000 → 5000.
        market.tick_month();
        assert_eq!(market.stockpile.get(ids::PROVISIONS), 5000.0);
        // Run more ticks than there is stockpile.
        for _ in 0..10 {
            market.tick_month();
        }
        // Should bottom out at zero, not go negative.
        assert_eq!(market.stockpile.get(ids::PROVISIONS), 0.0);
    }

    #[test]
    fn archetype_for_known_ports() {
        let (a, gw) = archetype_for("Bridgetown");
        assert!(matches!(a, PortArchetype::SugarIsland));
        assert!(gw);
        assert!(matches!(archetype_for("Boston").0, PortArchetype::NorthAmericanFarming));
        assert!(matches!(archetype_for("Cartagena").0, PortArchetype::SpanishTreasure));
        assert!(matches!(archetype_for("Tortuga").0, PortArchetype::PirateHaven));
        // Unknown port falls back to Minor.
        assert!(matches!(archetype_for("Atlantis").0, PortArchetype::Minor));
    }

    #[test]
    fn buy_transfers_goods_silver_and_stockpile() {
        use crate::ship::{Ship, ShipState, ShipStats};
        use crate::types::Position;

        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(
            &registry,
            PortArchetype::SugarIsland.recipe(),
            false,
        );
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
        let mut market = PortMarket::with_recipe(
            &registry,
            PortArchetype::SugarIsland.recipe(),
            false,
        );
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
        let mut market = PortMarket::with_recipe(
            &registry,
            PortArchetype::SugarIsland.recipe(),
            false,
        );
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
        let mut market = PortMarket::with_recipe(
            &registry,
            PortArchetype::SugarIsland.recipe(),
            false,
        );
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
        let mut market = PortMarket::with_recipe(
            &registry,
            PortArchetype::SugarIsland.recipe(),
            false,
        );
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        let result = market.sell(&mut ship, ids::SUGAR, 1.0, &registry);
        assert_eq!(result, Err(TradeError::InsufficientShipCargo));
    }

    #[test]
    fn gateway_floors_export_sell_price_at_world_rate() {
        let registry = GoodsRegistry::starter();
        // Sugar island gateway with a giant local sugar surplus → local
        // sell price would crash to floor (0.25× base). The world
        // premium (1.30× base) should kick in.
        let mut gateway = PortMarket::with_recipe(
            &registry,
            PortArchetype::SugarIsland.recipe(),
            true,
        );
        gateway.stockpile.add(ids::SUGAR, 100_000.0);
        let base = registry.get(ids::SUGAR).base_price_pesos;
        let p = gateway.sell_price(ids::SUGAR, &registry);
        assert!((p - base * WORLD_EXPORT_PREMIUM).abs() < 1e-3,
            "expected world export floor {} got {}", base * WORLD_EXPORT_PREMIUM, p);
    }

    #[test]
    fn gateway_caps_import_buy_price_at_world_rate() {
        let registry = GoodsRegistry::starter();
        // Gateway port with a starved stockpile of manufactures →
        // local buy price would soar; world price (0.90× base) caps it.
        let mut gateway = PortMarket::with_recipe(
            &registry,
            PortArchetype::Minor.recipe(),
            true,
        );
        let stk = gateway.stockpile.get(ids::MANUFACTURES);
        gateway.stockpile.remove(ids::MANUFACTURES, stk);
        let base = registry.get(ids::MANUFACTURES).base_price_pesos;
        let p = gateway.buy_price(ids::MANUFACTURES, &registry);
        assert!((p - base * WORLD_IMPORT_DISCOUNT).abs() < 1e-3,
            "expected world import cap {} got {}", base * WORLD_IMPORT_DISCOUNT, p);
    }

    #[test]
    fn gateway_export_sell_bypasses_treasury_and_stockpile() {
        use crate::ship::{Ship, ShipState};
        use crate::types::Position;

        let registry = GoodsRegistry::starter();
        let mut gateway = PortMarket::with_recipe(
            &registry,
            PortArchetype::SugarIsland.recipe(),
            true,
        );
        // Drain port silver to (almost) zero — would normally fail a sale.
        gateway.silver = 1.0;
        let stockpile_before = gateway.stockpile.get(ids::SUGAR);
        let port_silver_before = gateway.silver;

        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.cargo.add(ids::SUGAR, 10.0);
        let ship_silver_before = ship.silver;

        let proceeds = gateway.sell(&mut ship, ids::SUGAR, 10.0, &registry).unwrap();

        // Ship got world-price proceeds.
        let base = registry.get(ids::SUGAR).base_price_pesos;
        assert!((proceeds - 10.0 * base * WORLD_EXPORT_PREMIUM).abs() < 1e-2);
        assert!((ship.silver - (ship_silver_before + proceeds)).abs() < 1e-2);
        // Stockpile and port silver UNCHANGED — goods went to Europe,
        // silver came from off-map.
        assert!((gateway.stockpile.get(ids::SUGAR) - stockpile_before).abs() < 1e-3);
        assert!((gateway.silver - port_silver_before).abs() < 1e-3);
    }

    #[test]
    fn gateway_import_buy_bypasses_stockpile() {
        use crate::ship::{Ship, ShipState, ShipStats};
        use crate::types::Position;

        let registry = GoodsRegistry::starter();
        let mut gateway = PortMarket::with_recipe(
            &registry,
            PortArchetype::Minor.recipe(),
            true,
        );
        // Empty manufactures stockpile — local would refuse to sell.
        let stk = gateway.stockpile.get(ids::MANUFACTURES);
        gateway.stockpile.remove(ids::MANUFACTURES, stk);
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        let port_silver_before = gateway.silver;

        let cost = gateway.buy(&mut ship, &stats, ids::MANUFACTURES, 5.0, &registry).unwrap();

        let base = registry.get(ids::MANUFACTURES).base_price_pesos;
        assert!((cost - 5.0 * base * WORLD_IMPORT_DISCOUNT).abs() < 1e-2);
        assert!((ship.cargo.get(ids::MANUFACTURES) - 5.0).abs() < 1e-3);
        // Stockpile stays empty (came from Europe), port silver unchanged
        // (silver went off-map).
        assert!(gateway.stockpile.get(ids::MANUFACTURES) <= 1e-3);
        assert!((gateway.silver - port_silver_before).abs() < 1e-3);
    }

    #[test]
    fn non_gateway_still_uses_local_prices() {
        let registry = GoodsRegistry::starter();
        let mut local = PortMarket::with_recipe(
            &registry,
            PortArchetype::SugarIsland.recipe(),
            false,
        );
        local.stockpile.add(ids::SUGAR, 100_000.0);
        let base = registry.get(ids::SUGAR).base_price_pesos;
        // Non-gateway: heavy surplus crushes price to floor.
        let p = local.sell_price(ids::SUGAR, &registry);
        assert!(p < base * WORLD_EXPORT_PREMIUM,
            "non-gateway should crash below world floor; got {}", p);
    }
}
