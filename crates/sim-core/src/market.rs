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
/// Below this fraction of base, the price floor kicks in (deep glut).
pub const PRICE_FLOOR_FRAC: f32 = 0.25;
/// Above this fraction of base, the price ceiling kicks in (deep
/// scarcity). Set high enough that goods produced locally can keep
/// rising to genuinely choke off marginal demand when ships have
/// drained the wharf and started borrowing against next month's
/// production.
pub const PRICE_CEIL_FRAC: f32 = 8.0;

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
    /// Tons borrowed against future production for goods the port
    /// produces locally. When the wharf is empty but a ship still
    /// wants to buy a locally-produced good, the deficit is recorded
    /// here as a debt against next month's harvest. `tick_month` (and
    /// any sell that adds the good back to the wharf) pays this debt
    /// down before adding to the visible stockpile. Effective stock
    /// for pricing = `stockpile − debt`, so each successive draw on
    /// the hinterland raises the price for the next buyer.
    pub debt: Cargo,
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
            debt: Cargo::new(),
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
    pub fn with_recipe(_registry: &GoodsRegistry, recipe: ProductionRecipe) -> Self {
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
            debt: Cargo::new(),
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
    ///
    /// After applying flow, settle any outstanding hinterland debt by
    /// having new production pay down what was borrowed last month.
    pub fn tick_month(&mut self) {
        let prosperity = self.recipe.prosperity.max(0.0);
        for (id, tons) in self.recipe.monthly_outputs.clone() {
            self.stockpile.add(id, tons * prosperity);
        }
        for (id, tons) in self.recipe.monthly_inputs.clone() {
            self.stockpile.remove(id, tons * prosperity);
        }
        self.settle_debt();
    }

    /// Whenever stockpile goes positive, use it to pay down any
    /// outstanding hinterland debt for that good. Called after monthly
    /// production and after any sell that adds inventory back to the
    /// wharf.
    fn settle_debt(&mut self) {
        let owed: Vec<(GoodId, f32)> = self.debt.iter().collect();
        for (id, debt_tons) in owed {
            let in_stock = self.stockpile.get(id);
            let pay = debt_tons.min(in_stock);
            if pay > 0.0 {
                self.stockpile.remove(id, pay);
                self.debt.remove(id, pay);
            }
        }
    }

    /// True iff the port produces this good (it appears in
    /// `monthly_outputs`). Used to decide whether `buy` may borrow
    /// against next month's production when the wharf is empty.
    fn produces(&self, id: GoodId) -> bool {
        self.recipe.monthly_outputs.iter().any(|(g, _)| *g == id)
    }

    /// Pesos-per-ton local price for `id`, factoring effective stock
    /// (visible stockpile minus any outstanding hinterland debt).
    /// Stockpile-driven modulation kicks in only once a `target` is
    /// declared via the recipe; until then the base price is returned.
    pub fn price(&self, id: GoodId, registry: &GoodsRegistry) -> f32 {
        let good = registry.get(id);
        let target = self.target_stock(id);
        if target <= 0.0 {
            return good.base_price_pesos;
        }
        // Effective stock can go negative when ships have borrowed
        // against next month's production via `buy`; that pushes the
        // factor above 2× and toward the ceiling, exactly what we
        // want — the Nth ship of a dry month pays much more than the
        // first.
        let current = self.stockpile.get(id) - self.debt.get(id);
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

    /// Implicit "target" stockpile: 6 months of the recipe's gross
    /// throughput (max of output and input rates). For dual-flow goods
    /// like provisions on a sugar island (5 t/mo locally produced,
    /// 12 t/mo consumed) the consumption rate dominates the inventory
    /// target, since the wharf needs to cover the *demand*, not just
    /// the local supply. Returns 0.0 when neither flow is set, which
    /// gives flat base pricing.
    fn target_stock(&self, id: GoodId) -> f32 {
        let from_outputs = self
            .recipe
            .monthly_outputs
            .iter()
            .find(|(g, _)| *g == id)
            .map(|(_, t)| *t)
            .unwrap_or(0.0);
        let from_inputs = self
            .recipe
            .monthly_inputs
            .iter()
            .find(|(g, _)| *g == id)
            .map(|(_, t)| *t)
            .unwrap_or(0.0);
        from_outputs.max(from_inputs) * 6.0
    }

    /// Buy `requested_tons` of `id` from this market on behalf of `ship`.
    /// Atomic: either the full requested amount transacts or nothing
    /// changes. Returns the cost in pesos on success.
    ///
    /// Locally-produced goods (anything in `monthly_outputs`) can be
    /// bought even when the wharf stockpile is depleted: the deficit
    /// is recorded as `debt` (borrowed against next month's harvest)
    /// and prices rise accordingly via the effective-stock formula.
    /// Goods the port doesn't produce — only imports — hard-fail when
    /// stockpile runs out.
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
        let in_stock = self.stockpile.get(id);
        if requested_tons > in_stock + 1e-4 {
            // Wharf empty: only sustainable if the port produces this
            // good locally (then we borrow against next month). For a
            // pure import-good the wharf running dry is a hard stop.
            if !self.produces(id) {
                return Err(TradeError::InsufficientStockpile);
            }
        }
        ship.silver -= cost;
        self.silver += cost;
        let from_wharf = requested_tons.min(in_stock);
        if from_wharf > 0.0 {
            self.stockpile.remove(id, from_wharf);
        }
        let from_hinterland = (requested_tons - from_wharf).max(0.0);
        if from_hinterland > 0.0 {
            self.debt.add(id, from_hinterland);
        }
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
        // Resold inventory pays down any outstanding hinterland debt
        // for this good before remaining as visible stockpile.
        self.settle_debt();
        Ok(proceeds)
    }

    /// Home-port settlement. Called when a ship docks at its owner
    /// port: any silver above `float` is paid to the port treasury
    /// (the "owners"), and the ship is left with exactly `float`
    /// silver for incidental running costs.
    ///
    /// Returns the deposited amount (always ≥ 0). If the ship's
    /// silver is already at or below `float`, this is a no-op.
    ///
    /// Models 17th-century practice: the supercargo books proceeds
    /// with the owners on return; dividends go to the share-holding
    /// merchants of the home port. The ship's "treasury" only fills
    /// up again when capital is drawn for the next outbound cargo
    /// via [`Self::draw_for_outfit`].
    pub fn deposit_owner_profit(&mut self, ship: &mut crate::ship::Ship, float: f32) -> f32 {
        let surplus = ship.silver - float;
        if surplus <= 0.0 {
            return 0.0;
        }
        ship.silver -= surplus;
        self.silver += surplus;
        surplus
    }

    /// Outbound-cargo capital draw. Called when a ship is at its
    /// owner port and about to load cargo: tops up `ship.silver` to
    /// `target` by withdrawing from the port treasury. The withdrawal
    /// is capped at `port_fraction_cap × self.silver` so one ship
    /// can't drain a port's working capital.
    ///
    /// Returns the actual amount drawn. If the port has insufficient
    /// silver (or the cap binds), the ship sails with what it could
    /// get; the buy logic will then load a partial cargo.
    pub fn draw_for_outfit(
        &mut self,
        ship: &mut crate::ship::Ship,
        target: f32,
        port_fraction_cap: f32,
    ) -> f32 {
        let needed = target - ship.silver;
        if needed <= 0.0 {
            return 0.0;
        }
        let cap = (self.silver * port_fraction_cap).max(0.0);
        let drawn = needed.min(cap).max(0.0);
        if drawn <= 0.0 {
            return 0.0;
        }
        self.silver -= drawn;
        ship.silver += drawn;
        drawn
    }

    /// Extend chandler/factor credit to a docked ship: advance silver
    /// from the port's treasury into the ship's strongbox and add the
    /// same amount to `ship.debt`. The advance is capped by the
    /// remaining headroom under `max_total_debt`, the port's available
    /// liquidity, and `port_fraction_cap × self.silver`.
    ///
    /// Returns the actual silver advanced (always ≥ 0). Used for two
    /// historically distinct purposes:
    ///   * **Chandler credit** — provisions taken on tick when broke
    ///     (`target` ≈ cost of topping up the larder).
    ///   * **Tramping / freight** — cargo advanced on consignment when
    ///     no profitable arbitrage is in reach (`target` ≈ cost of
    ///     filling the hold with a local export).
    ///
    /// Settles fungibly: although the loan is granted by this port,
    /// repayment goes to whichever port the ship next docks at —
    /// historically arranged via bills of exchange between merchant
    /// correspondents.
    pub fn extend_credit(
        &mut self,
        ship: &mut crate::ship::Ship,
        target: f32,
        port_fraction_cap: f32,
        max_total_debt: f32,
    ) -> f32 {
        let needed = target.max(0.0);
        let debt_headroom = (max_total_debt - ship.debt).max(0.0);
        let liquidity_cap = (self.silver * port_fraction_cap).max(0.0);
        let advance = needed.min(debt_headroom).min(liquidity_cap);
        if advance <= 0.0 {
            return 0.0;
        }
        self.silver -= advance;
        ship.silver += advance;
        ship.debt += advance;
        advance
    }

    /// Settle outstanding ship debt against the current port's treasury,
    /// out of any silver the ship holds above `float`. Returns the
    /// amount repaid. Called at every dock arrival before dividends
    /// or other port settlement, so creditors are paid first.
    pub fn collect_debt(&mut self, ship: &mut crate::ship::Ship, float: f32) -> f32 {
        if ship.debt <= 0.0 {
            return 0.0;
        }
        let available = (ship.silver - float).max(0.0);
        let payment = available.min(ship.debt);
        if payment <= 0.0 {
            return 0.0;
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
                &[(TOBACCO, 60.0), (PROVISIONS, 4.0)],
                &[
                    (PROVISIONS, 6.0),
                    (MANUFACTURES, 4.0),
                    (ENSLAVED_PERSONS, 2.0),
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
                &[(MANUFACTURES, 80.0), (RUM, 20.0), (PROVISIONS, 70.0)],
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
        assert!(
            p < base,
            "Surplus should depress price (got {} >= {})",
            p,
            base
        );
        assert!(p >= base * PRICE_FLOOR_FRAC - 1e-3);
    }

    #[test]
    fn shortage_raises_price_under_recipe() {
        let registry = GoodsRegistry::starter();
        let mut market = PortMarket::with_initial_stockpile(&registry);
        // Plantation that *consumes* manufactures: target ≈ inputs × 6.
        market
            .recipe
            .monthly_inputs
            .push((ids::MANUFACTURES, 200.0));
        // Target = 1200 > stockpile 1000 → mild shortage premium.
        let base = registry.get(ids::MANUFACTURES).base_price_pesos;
        let p = market.price(ids::MANUFACTURES, &registry);
        assert!(
            p > base,
            "Shortage should raise price (got {} <= {})",
            p,
            base
        );
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
        // Inputs sit at 3× monthly throughput (3 months' buffer) —
        // unless the good is *also* an output (e.g. PROVISIONS on a
        // sugar island), in which case the output seeding wins.
        for (id, tons) in &recipe.monthly_inputs {
            if recipe.monthly_outputs.iter().any(|(g, _)| g == id) {
                continue;
            }
            assert_eq!(market.stockpile.get(*id), *tons * 3.0);
        }
        // Output prices should be cheap (surplus); input prices expensive.
        let sugar_base = registry.get(ids::SUGAR).base_price_pesos;
        let manuf_base = registry.get(ids::MANUFACTURES).base_price_pesos;
        assert!(
            market.price(ids::SUGAR, &registry) < sugar_base,
            "producer port should price its output below base"
        );
        assert!(
            market.price(ids::MANUFACTURES, &registry) > manuf_base,
            "consumer port should price its input above base"
        );
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
        let sugar_out = recipe
            .monthly_outputs
            .iter()
            .find(|(g, _)| *g == ids::SUGAR)
            .unwrap()
            .1;
        assert!((market.stockpile.get(ids::SUGAR) - (sugar_before + sugar_out)).abs() < 1e-3);
        // Provisions appear on both sides (sugar islands grow some
        // food locally and import the rest); net change = output − input.
        let prov_out = recipe
            .monthly_outputs
            .iter()
            .find(|(g, _)| *g == ids::PROVISIONS)
            .map(|(_, t)| *t)
            .unwrap_or(0.0);
        let prov_in = recipe
            .monthly_inputs
            .iter()
            .find(|(g, _)| *g == ids::PROVISIONS)
            .map(|(_, t)| *t)
            .unwrap_or(0.0);
        let expected = provisions_before + prov_out - prov_in;
        assert!((market.stockpile.get(ids::PROVISIONS) - expected).abs() < 1e-3);
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
        assert!(matches!(
            archetype_for("Bridgetown"),
            PortArchetype::SugarIsland
        ));
        assert!(matches!(
            archetype_for("Boston"),
            PortArchetype::NorthAmericanFarming
        ));
        assert!(matches!(
            archetype_for("Cartagena"),
            PortArchetype::SpanishTreasure
        ));
        assert!(matches!(
            archetype_for("Tortuga"),
            PortArchetype::PirateHaven
        ));
        assert!(matches!(
            archetype_for("London"),
            PortArchetype::EuropeanLondon
        ));
        assert!(matches!(
            archetype_for("Cadiz"),
            PortArchetype::EuropeanCadiz
        ));
        assert!(matches!(
            archetype_for("Elmina"),
            PortArchetype::AfricanElmina
        ));
        assert!(matches!(
            archetype_for("Ouidah"),
            PortArchetype::AfricanOuidah
        ));
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
        let cost = market
            .buy(&mut ship, &stats, ids::SUGAR, 5.0, &registry)
            .unwrap();

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
        market
            .buy(&mut ship, &stats, ids::SUGAR, 1.0, &registry)
            .unwrap();
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
        assert!(
            p > base * 1.5,
            "expected drained London sugar sell price well above base; got {} (base {})",
            p,
            base
        );
    }

    #[test]
    fn african_slave_port_produces_enslaved_persons() {
        let registry = GoodsRegistry::starter();
        let elmina = PortMarket::with_recipe(&registry, PortArchetype::AfricanElmina.recipe());
        // Elmina's monthly_outputs include ENSLAVED_PERSONS, so the
        // stockpile is seeded at 6× monthly output.
        assert!(elmina.stockpile.get(ids::ENSLAVED_PERSONS) > 0.0);
    }

    #[test]
    fn producer_port_can_be_bought_into_negative_effective_stock() {
        // A producer port: makes SUGAR, no inputs. Drain its wharf,
        // then a further buy should still succeed by borrowing against
        // next month's harvest — but at a higher unit price than the
        // first.
        use crate::ship::{Ship, ShipState, ShipStats};
        use crate::types::Position;
        let registry = GoodsRegistry::starter();
        let mut recipe = ProductionRecipe::empty();
        recipe.monthly_outputs.push((ids::SUGAR, 4.0)); // 4 t/mo → 48 t starting wharf, fits in sloop
        let mut market = PortMarket::with_recipe(&registry, recipe);
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.silver = 1_000_000.0;

        // Drain the visible wharf.
        let wharf = market.stockpile.get(ids::SUGAR);
        market
            .buy(&mut ship, &stats, ids::SUGAR, wharf, &registry)
            .unwrap();
        let price_at_zero = market.buy_price(ids::SUGAR, &registry);

        // Now buy more — must succeed via the hinterland-debt path.
        let further = 5.0;
        let cost = market
            .buy(&mut ship, &stats, ids::SUGAR, further, &registry)
            .unwrap();
        let unit_paid = cost / further;
        assert!(
            market.debt.get(ids::SUGAR) >= further - 1e-3,
            "deficit should be recorded as hinterland debt"
        );
        assert!(
            unit_paid > price_at_zero - 1e-3,
            "borrowing against next month should price above the empty-wharf price"
        );

        // And the next monthly tick should pay debt down before
        // adding to visible stockpile.
        let debt_before = market.debt.get(ids::SUGAR);
        market.tick_month();
        assert!(
            market.debt.get(ids::SUGAR) < debt_before,
            "tick_month should settle outstanding hinterland debt"
        );
    }

    #[test]
    fn import_only_good_still_hard_fails_when_stockpile_empty() {
        // Sugar island does not produce manufactures. Once its
        // import stock is exhausted, further buys must fail.
        use crate::ship::{Ship, ShipState, ShipStats};
        use crate::types::Position;
        let registry = GoodsRegistry::starter();
        let recipe = PortArchetype::SugarIsland.recipe();
        let mut market = PortMarket::with_recipe(&registry, recipe);
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.silver = 10_000_000.0;

        let wharf = market.stockpile.get(ids::MANUFACTURES);
        market
            .buy(&mut ship, &stats, ids::MANUFACTURES, wharf, &registry)
            .unwrap();
        let err = market.buy(&mut ship, &stats, ids::MANUFACTURES, 1.0, &registry);
        assert!(matches!(err, Err(TradeError::InsufficientStockpile)));
    }
}
