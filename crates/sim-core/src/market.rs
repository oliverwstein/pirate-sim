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
    /// good in the registry. Used during world load before per-port
    /// recipes are dialed in.
    pub fn with_initial_stockpile(registry: &GoodsRegistry) -> Self {
        let mut stockpile = Cargo::new();
        for good in registry.iter() {
            stockpile.add(good.id, INITIAL_STOCKPILE_TONS);
        }
        Self {
            stockpile,
            silver: 0.0,
            recipe: ProductionRecipe::empty(),
            is_europe_gateway: false,
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
}
