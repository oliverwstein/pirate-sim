//! Per-port bounded-balance market state, prices, and production recipe.

use crate::goods::{GoodId, GoodsRegistry};
use crate::market_curve::{self, BalanceTable};
use crate::money::Pesos;

/// Starting silver in a port's treasury. Used to settle ship/port
/// transactions and local credit.
const INITIAL_PORT_SILVER_PESOS: Pesos = Pesos::from_pesos(50_000);

/// Fraction of `crown_silver` transferred into the port treasury each
/// month — historically, the crown's collected duties paid the
/// governor, garrison, and dockworkers, putting silver back into the
/// local economy.
const CROWN_SILVER_MONTHLY_BLEED: f32 = 1.0;

/// What a port produces and consumes each simulated month.
#[derive(Clone, Debug, Default)]
pub struct ProductionRecipe {
    pub monthly_outputs: Vec<(GoodId, f32)>,
    pub monthly_inputs: Vec<(GoodId, f32)>,
    /// Multiplier on outputs/inputs. 1.0 = baseline historical estimate;
    /// >1.0 boom, <1.0 stagnation.
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
    /// Pesos in the port's treasury (the merchants' / governor's
    /// working capital).
    pub silver: Pesos,
    /// Pesos collected as duties (tariffs) on behalf of the crown.
    pub crown_silver: Pesos,
    pub recipe: ProductionRecipe,
    /// Per-good base bound (max signed balance, prosperity = 1.0).
    pub base_bounds: BalanceTable,
    /// Per-good signed trade balance. Positive means surplus/glut;
    /// negative means shortage. Prices are derived from this value.
    pub balance: BalanceTable,
}

/// Six months of recipe gross throughput, used by Phase B.4 only to keep
/// the Phase B.1 heuristic seeding semantics until LP seeding replaces it.
fn recipe_target_stock(recipe: &ProductionRecipe, id: GoodId) -> f32 {
    let from_outputs = recipe
        .monthly_outputs
        .iter()
        .find(|(g, _)| *g == id)
        .map(|(_, t)| *t)
        .unwrap_or(0.0);
    let from_inputs = recipe
        .monthly_inputs
        .iter()
        .find(|(g, _)| *g == id)
        .map(|(_, t)| *t)
        .unwrap_or(0.0);
    from_outputs.max(from_inputs) * 6.0
}

impl PortMarket {
    /// Per-good base bound: twelve months of the recipe's gross throughput,
    /// floored at 1 ton.
    fn derive_base_bounds(recipe: &ProductionRecipe) -> BalanceTable {
        let mut bounds = BalanceTable::new();
        for (id, tons) in &recipe.monthly_outputs {
            let prior = bounds.get(*id);
            let candidate = (tons * 12.0).round() as i32;
            if candidate > prior {
                bounds.set(*id, candidate.max(1));
            }
        }
        for (id, tons) in &recipe.monthly_inputs {
            let prior = bounds.get(*id);
            let candidate = (tons * 12.0).round() as i32;
            if candidate > prior {
                bounds.set(*id, candidate.max(1));
            }
        }
        bounds
    }

    /// Initial signed balance for recipe goods. Mirrors the former stock
    /// seeding: output goods at 12× monthly throughput become full surplus;
    /// input-only goods at 3× monthly throughput become half shortage.
    fn derive_initial_balance(recipe: &ProductionRecipe, bounds: &BalanceTable) -> BalanceTable {
        let mut balance = BalanceTable::new();
        for (id, tons) in &recipe.monthly_outputs {
            let target = recipe_target_stock(recipe, *id);
            let bound = bounds.get(*id);
            if target > 0.0 && bound > 0 {
                let seeded = *tons * 12.0;
                let offset = ((seeded - target) / target) * bound as f32;
                balance.set(*id, (offset.round() as i32).clamp(-bound, bound));
            }
        }
        for (id, tons) in &recipe.monthly_inputs {
            if balance.get(*id) != 0 {
                continue;
            }
            let target = recipe_target_stock(recipe, *id);
            let bound = bounds.get(*id);
            if target > 0.0 && bound > 0 {
                let seeded = *tons * 3.0;
                let offset = ((seeded - target) / target) * bound as f32;
                balance.set(*id, (offset.round() as i32).clamp(-bound, bound));
            }
        }
        balance
    }

    /// Effective per-good bound after applying prosperity. Floored at 1.
    pub fn effective_bound(&self, id: GoodId) -> i32 {
        market_curve::effective_bound(self.base_bounds.get(id), self.recipe.prosperity)
    }

    pub fn with_neutral_balance(_registry: &GoodsRegistry) -> Self {
        let recipe = ProductionRecipe::empty();
        Self {
            silver: INITIAL_PORT_SILVER_PESOS,
            crown_silver: Pesos::ZERO,
            recipe,
            base_bounds: BalanceTable::new(),
            balance: BalanceTable::new(),
        }
    }

    pub fn with_recipe(_registry: &GoodsRegistry, recipe: ProductionRecipe) -> Self {
        let base_bounds = Self::derive_base_bounds(&recipe);
        let balance = Self::derive_initial_balance(&recipe, &base_bounds);
        Self {
            silver: INITIAL_PORT_SILVER_PESOS,
            crown_silver: Pesos::ZERO,
            recipe,
            base_bounds,
            balance,
        }
    }

    /// Apply one month of recipe flow to the signed balance and bleed crown
    /// duties into the local treasury.
    pub fn tick_month(&mut self) {
        let prosperity = self.recipe.prosperity.max(0.0);
        for (id, tons) in self.recipe.monthly_outputs.clone() {
            self.add_balance_tons(id, tons * prosperity);
        }
        for (id, tons) in self.recipe.monthly_inputs.clone() {
            self.add_balance_tons(id, -tons * prosperity);
        }
        let bleed = self.crown_silver.scale(CROWN_SILVER_MONTHLY_BLEED);
        self.silver += bleed;
        self.crown_silver -= bleed;
    }

    fn set_balance_clamped(&mut self, id: GoodId, value: f32) {
        let bound = self.effective_bound(id).max(1);
        self.balance
            .set(id, (value.round() as i32).clamp(-bound, bound));
    }

    fn add_balance_tons(&mut self, id: GoodId, delta_tons: f32) {
        let next = self.balance.get(id) as f32 + delta_tons;
        self.set_balance_clamped(id, next);
    }

    /// Tons the port can sell before this good reaches its shortage bound.
    pub fn available_to_buy(&self, id: GoodId) -> f32 {
        if self.base_bounds.get(id) <= 0 && self.balance.get(id) == 0 {
            return 0.0;
        }
        (self.balance.get(id) + self.effective_bound(id)).max(0) as f32
    }

    fn multiplier_at_ratio(x: f32) -> f32 {
        let x = x.clamp(-1.0, 1.0);
        if x < 0.0 {
            1.0 + market_curve::ALPHA_SHORTAGE * (-x).powf(market_curve::P_SHORTAGE)
        } else {
            1.0 / (1.0 + market_curve::BETA_GLUT * x.powf(market_curve::P_GLUT))
        }
    }

    /// Current local mid-price for `good` in pesos per ton, derived from
    /// the port's signed balance and the asymmetric bounded curve.
    pub fn price_at(&self, good: GoodId, goods: &GoodsRegistry) -> f32 {
        let base = goods.get(good).base_price_pesos;
        let bound = self.effective_bound(good).max(1);
        let bal = self.balance.get(good);
        base * market_curve::price_multiplier(bal, bound)
    }

    /// Local mid-price after a hypothetical trade. Positive `delta_tons`
    /// means a ship buys that many tons from the port (port balance goes
    /// down); negative means a ship sells to the port (balance goes up).
    pub fn price_after_trade(&self, good: GoodId, delta_tons: f32, goods: &GoodsRegistry) -> f32 {
        let base = goods.get(good).base_price_pesos;
        let bound = self.effective_bound(good).max(1);
        let bal = self.balance.get(good) as f32 - delta_tons;
        base * Self::multiplier_at_ratio(bal / bound as f32)
    }

    /// Buy `requested_tons` of `id` from this market on behalf of `ship`.
    /// This legacy direct path is bounded by `balance`; the main world loop
    /// uses the fixed-point auction instead.
    pub fn buy(
        &mut self,
        ship: &mut crate::ship::Ship,
        ship_stats: &crate::ship::ShipStats,
        id: GoodId,
        requested_tons: f32,
        duty: f32,
        registry: &GoodsRegistry,
    ) -> Result<Pesos, TradeError> {
        if requested_tons <= 0.0 {
            return Err(TradeError::NonPositiveAmount);
        }
        let unit = self.price_after_trade(id, requested_tons, registry);
        let base = Pesos::from_pesos_f32(unit * requested_tons);
        let duty_amount = base.scale(duty.max(0.0));
        let cost = base + duty_amount;
        if cost > ship.silver {
            return Err(TradeError::InsufficientSilver);
        }
        let cargo_room = ship_stats.cargo_capacity_tons - ship.cargo.total_tons();
        if requested_tons > cargo_room + 1e-4 {
            return Err(TradeError::InsufficientCargoSpace);
        }
        if requested_tons > self.available_to_buy(id) + 1e-4 {
            return Err(TradeError::InsufficientStockpile);
        }
        ship.silver -= cost;
        self.silver += base;
        self.crown_silver += duty_amount;
        self.add_balance_tons(id, -requested_tons);
        ship.cargo.add(id, requested_tons);
        Ok(cost)
    }

    /// Sell `requested_tons` of `id` from `ship` into this market.
    pub fn sell(
        &mut self,
        ship: &mut crate::ship::Ship,
        id: GoodId,
        requested_tons: f32,
        duty: f32,
        registry: &GoodsRegistry,
    ) -> Result<Pesos, TradeError> {
        if requested_tons <= 0.0 {
            return Err(TradeError::NonPositiveAmount);
        }
        if requested_tons > ship.cargo.get(id) + 1e-4 {
            return Err(TradeError::InsufficientShipCargo);
        }
        let unit = self.price_after_trade(id, -requested_tons, registry);
        let base = Pesos::from_pesos_f32(unit * requested_tons);
        if base > self.silver {
            return Err(TradeError::InsufficientPortSilver);
        }
        let duty_amount = base.scale(duty.max(0.0));
        let ship_proceeds = base - duty_amount;
        ship.cargo.remove(id, requested_tons);
        self.add_balance_tons(id, requested_tons);
        self.silver -= base;
        self.crown_silver += duty_amount;
        ship.silver += ship_proceeds;
        Ok(ship_proceeds)
    }

    /// Home-port settlement. Called when a ship docks at its owner port.
    pub fn deposit_owner_profit(&mut self, ship: &mut crate::ship::Ship, float: Pesos) -> Pesos {
        let surplus = ship.silver - float;
        if !surplus.is_positive() {
            return Pesos::ZERO;
        }
        ship.silver -= surplus;
        self.silver += surplus;
        surplus
    }

    /// Outbound-cargo capital draw.
    pub fn draw_for_outfit(
        &mut self,
        ship: &mut crate::ship::Ship,
        target: Pesos,
        port_fraction_cap: f32,
    ) -> Pesos {
        let needed = target - ship.silver;
        if !needed.is_positive() {
            return Pesos::ZERO;
        }
        let cap = self.silver.scale(port_fraction_cap).max_zero();
        let drawn = needed.min(cap).max_zero();
        if !drawn.is_positive() {
            return Pesos::ZERO;
        }
        self.silver -= drawn;
        ship.silver += drawn;
        drawn
    }

    /// Extend chandler/factor credit to a docked ship.
    pub fn extend_credit(
        &mut self,
        ship: &mut crate::ship::Ship,
        target: Pesos,
        port_fraction_cap: f32,
        max_total_debt: Pesos,
    ) -> Pesos {
        let needed = target.max_zero();
        let debt_headroom = (max_total_debt - ship.debt).max_zero();
        let liquidity_cap = self.silver.scale(port_fraction_cap).max_zero();
        let advance = needed.min(debt_headroom).min(liquidity_cap);
        if !advance.is_positive() {
            return Pesos::ZERO;
        }
        self.silver -= advance;
        ship.silver += advance;
        ship.debt += advance;
        advance
    }

    /// Settle outstanding ship debt against the current port's treasury.
    pub fn collect_debt(&mut self, ship: &mut crate::ship::Ship, float: Pesos) -> Pesos {
        if !ship.debt.is_positive() {
            return Pesos::ZERO;
        }
        let available = (ship.silver - float).max_zero();
        let payment = available.min(ship.debt);
        if !payment.is_positive() {
            return Pesos::ZERO;
        }
        ship.silver -= payment;
        ship.debt -= payment;
        self.silver += payment;
        payment
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
        #[allow(clippy::type_complexity)]
        let (outputs, inputs): (&[(GoodId, f32)], &[(GoodId, f32)]) = match self {
            PortArchetype::SugarIsland => (
                // Small local provisions output (yams, cassava, fish)
                // on top of the staple sugar/molasses/rum complex.
                // Net food importer because consumption (12 t/mo)
                // exceeds local supply (5 t/mo) — but a visiting ship
                // can borrow against the next harvest at rising
                // prices when the wharf is dry.
                &[
                    (SUGAR, 80.0),
                    (MOLASSES, 30.0),
                    (RUM, 15.0),
                    (PROVISIONS, 5.0),
                ],
                &[
                    (PROVISIONS, 12.0),
                    (MANUFACTURES, 6.0),
                    (ENSLAVED_PERSONS, 3.0),
                    (NAVAL_STORES, 2.0),
                ],
            ),
            PortArchetype::TobaccoColony => (
                // Charleston represents the whole Chesapeake/Carolina
                // tobacco region in this map (no separate Virginia
                // port). Historical 1680-1720 Chesapeake exports ran
                // 15-25k tons/yr by the early 18th century; 100 t/mo
                // here is a deliberate compression but balances the
                // European re-export demand of ~165 t/mo. Scaled up
                // inputs to match the bigger plantation operation.
                &[(TOBACCO, 100.0), (PROVISIONS, 6.0)],
                &[
                    (PROVISIONS, 10.0),
                    (MANUFACTURES, 6.0),
                    (ENSLAVED_PERSONS, 3.0),
                ],
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
                &[(PROVISIONS, 3.0)],
                &[(PROVISIONS, 4.0), (RUM, 3.0), (MANUFACTURES, 2.0)],
            ),
            PortArchetype::Minor => (
                &[(PROVISIONS, 2.0)],
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
                &[
                    (MANUFACTURES, 200.0),
                    (PROVISIONS, 100.0),
                    // Step 7: Royal powder mills (Faversham, Waltham
                    // Abbey) and the Tower Foundry. London is the
                    // dominant English arsenal of the era.
                    (GUNPOWDER, 8.0),
                    (CANNON_SHOT, 12.0),
                ],
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
                &[
                    (MANUFACTURES, 120.0),
                    (NAVAL_STORES, 30.0),
                    (PROVISIONS, 80.0),
                    // Dutch powder works (Muiden) + Swedish iron-shot
                    // re-exported through the entrepôt of Europe.
                    (GUNPOWDER, 6.0),
                    (CANNON_SHOT, 10.0),
                ],
                &[
                    (SUGAR, 120.0),
                    (TOBACCO, 30.0),
                    (RUM, 15.0),
                    (MOLASSES, 15.0),
                    (SILVER, 0.2),
                ],
            ),
            PortArchetype::EuropeanCadiz => (
                &[
                    (MANUFACTURES, 30.0),
                    (PROVISIONS, 60.0),
                    // Royal Spanish arsenal at La Carraca. Smaller
                    // output than London/Amsterdam but enough to
                    // victual the treasure fleets with shot.
                    (GUNPOWDER, 3.0),
                    (CANNON_SHOT, 5.0),
                ],
                &[(SILVER, 1.0), (SUGAR, 40.0), (TOBACCO, 20.0)],
            ),
            PortArchetype::EuropeanNantes => (
                &[
                    (MANUFACTURES, 80.0),
                    (RUM, 20.0),
                    (PROVISIONS, 70.0),
                    // Phase 4 §1.2: French royal powder works (Essonnes,
                    // est. 1664 by Colbert) routed exports through the
                    // Atlantic ports. Modest output relative to London
                    // and Amsterdam, and no foundry shot of note —
                    // French shot largely went into army artillery.
                    (GUNPOWDER, 4.0),
                ],
                &[(SUGAR, 80.0), (TOBACCO, 30.0), (MOLASSES, 20.0)],
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
                &[(ENSLAVED_PERSONS, 30.0), (PROVISIONS, 8.0)],
                &[(MANUFACTURES, 12.0), (RUM, 4.0), (TOBACCO, 3.0)],
            ),
            PortArchetype::AfricanOuidah => (
                &[(ENSLAVED_PERSONS, 40.0), (PROVISIONS, 10.0)],
                &[(MANUFACTURES, 8.0), (RUM, 8.0), (TOBACCO, 4.0)],
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

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn neutral_market_prices_at_base() {
        let registry = GoodsRegistry::starter();
        let market = PortMarket::with_neutral_balance(&registry);
        let base = registry.get(ids::SUGAR).base_price_pesos;
        assert_eq!(market.price_at(ids::SUGAR, &registry), base);
    }

    #[test]
    fn phase_b1_seeds_bounds_and_balance_for_recipe_goods() {
        let registry = GoodsRegistry::starter();
        let market = PortMarket::with_recipe(&registry, PortArchetype::SugarIsland.recipe());
        let sugar_bound = market.base_bounds.get(ids::SUGAR);
        assert_eq!(sugar_bound, 960);
        assert_eq!(market.balance.get(ids::SUGAR), sugar_bound);
        let mfg_bound = market.base_bounds.get(ids::MANUFACTURES);
        assert_eq!(mfg_bound, 72);
        assert_eq!(market.balance.get(ids::MANUFACTURES), -36);
    }

    #[test]
    fn price_at_uses_asymmetric_curve_edges() {
        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&registry, PortArchetype::SugarIsland.recipe());
        let good = ids::SUGAR;
        let base = registry.get(good).base_price_pesos;
        let bound = market.effective_bound(good);
        market.balance.set(good, bound);
        assert!(approx_eq(
            market.price_at(good, &registry),
            base * 0.5,
            1e-4
        ));
        market.balance.set(good, -bound);
        assert!(approx_eq(
            market.price_at(good, &registry),
            base * 5.0,
            1e-4
        ));
        market.balance.set(good, 0);
        assert!(approx_eq(market.price_at(good, &registry), base, 1e-4));
    }

    #[test]
    fn price_after_trade_documents_buy_sell_sign() {
        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&registry, PortArchetype::SugarIsland.recipe());
        let good = ids::SUGAR;
        market.balance.set(good, 10);
        let current = market.price_at(good, &registry);
        let after_ship_buys = market.price_after_trade(good, 20.0, &registry);
        let after_ship_sells = market.price_after_trade(good, -20.0, &registry);
        assert!(
            after_ship_buys > current,
            "buying from port should raise price"
        );
        assert!(
            after_ship_sells < current,
            "selling to port should lower price"
        );
    }

    #[test]
    fn price_after_trade_clamps_at_edges() {
        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&registry, PortArchetype::SugarIsland.recipe());
        let good = ids::SUGAR;
        let base = registry.get(good).base_price_pesos;
        let bound = market.effective_bound(good);
        market.balance.set(good, 0);
        assert!(approx_eq(
            market.price_after_trade(good, bound as f32 * 2.0, &registry),
            base * 5.0,
            1e-4
        ));
        assert!(approx_eq(
            market.price_after_trade(good, -(bound as f32 * 2.0), &registry),
            base * 0.5,
            1e-4
        ));
    }

    #[test]
    fn tick_month_moves_balance_and_clamps() {
        let registry = GoodsRegistry::starter();
        let mut recipe = ProductionRecipe::empty();
        recipe.monthly_outputs.push((ids::SUGAR, 10.0));
        recipe.monthly_inputs.push((ids::MANUFACTURES, 5.0));
        let mut market = PortMarket::with_recipe(&registry, recipe);
        let sugar_before = market.balance.get(ids::SUGAR);
        let mfg_before = market.balance.get(ids::MANUFACTURES);
        market.tick_month();
        assert!(market.balance.get(ids::SUGAR) >= sugar_before);
        assert!(market.balance.get(ids::MANUFACTURES) < mfg_before);
    }

    #[test]
    fn buy_and_sell_update_balance() {
        use crate::ship::{Ship, ShipState, ShipStats};
        use crate::types::Position;
        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&registry, PortArchetype::SugarIsland.recipe());
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        let before = market.balance.get(ids::SUGAR);
        market
            .buy(&mut ship, &stats, ids::SUGAR, 5.0, 0.0, &registry)
            .unwrap();
        assert!(market.balance.get(ids::SUGAR) < before);
        market
            .sell(&mut ship, ids::SUGAR, 5.0, 0.0, &registry)
            .unwrap();
        assert!(market.balance.get(ids::SUGAR) >= before);
    }

    #[test]
    fn buy_rejects_beyond_shortage_bound() {
        use crate::ship::{Ship, ShipState, ShipStats};
        use crate::types::Position;
        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&registry, PortArchetype::SugarIsland.recipe());
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.silver = Pesos::from_pesos(10_000_000);
        let bound = market.effective_bound(ids::MANUFACTURES);
        market.balance.set(ids::MANUFACTURES, -bound);
        let err = market.buy(&mut ship, &stats, ids::MANUFACTURES, 1.0, 0.0, &registry);
        assert!(matches!(err, Err(TradeError::InsufficientStockpile)));
    }

    #[test]
    fn archetype_for_known_ports() {
        assert!(matches!(
            archetype_for("Bridgetown"),
            PortArchetype::SugarIsland
        ));
        assert!(matches!(
            archetype_for("Boston"),
            PortArchetype::NorthAmericanFarming
        ));
        assert!(matches!(
            archetype_for("London"),
            PortArchetype::EuropeanLondon
        ));
        assert!(matches!(archetype_for("Atlantis"), PortArchetype::Minor));
    }

    #[test]
    fn every_european_hub_produces_gunpowder() {
        for archetype in [
            PortArchetype::EuropeanLondon,
            PortArchetype::EuropeanAmsterdam,
            PortArchetype::EuropeanCadiz,
            PortArchetype::EuropeanNantes,
        ] {
            let recipe = archetype.recipe();
            assert!(recipe
                .monthly_outputs
                .iter()
                .any(|(g, _)| *g == ids::GUNPOWDER));
        }
    }
}
