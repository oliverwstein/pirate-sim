//! Shipbuilding: shipyard ports decide each month whether to commission
//! a new merchant vessel, based on whether the math pencils out.
//!
//! Design (stage 1):
//!
//!   build_cost  =  BUILD_SILVER
//!                + BUILD_NAVAL_STORES_TONS  * naval_stores_buy_price
//!                + BUILD_MANUFACTURES_TONS  * manufactures_buy_price
//!                + BUILD_PROVISIONS_TONS    * provisions_buy_price
//!
//!   build iff
//!       expected_lifetime_months * avg_monthly_fleet_profit
//!         >  hurdle_multiplier * build_cost
//!     AND  port has all inputs in stock and silver to pay
//!
//! The cost/lifetime/hurdle constants are calibrated to late-17C
//! merchant economics (5–10 yr Caribbean sloop lifetime, 10–20%
//! annual hurdle rate, so paying back 2× cost over 7 years).
//!
//! When a build fires, the port debits its treasury (silver) and
//! consumes the inputs from its stockpile. Because the shadow-price
//! model permits borrowing against next month's production for goods
//! the port produces, a port that is short on (e.g.) its own naval
//! stores can still build by going into hinterland debt — which
//! naturally raises future prices for everyone.
//!
//! Stage 2 (deferred): home-port deposit/withdraw loop, ownership
//! profit remittance, and refusing to build when fleet is overcrowded.
//!
//! The starting silver for a freshly-built ship is sized so it can
//! buy roughly one hold of the cheapest local export at the home
//! port; the BUY_BEST tree at undock time then does the actual trade
//! planning.

use crate::cargo::Cargo;
use crate::goods::{ids, GoodsRegistry};
use crate::market::PortMarket;
use crate::port::Port;
use crate::ship::{Ship, ShipStats};

/// Silver cost of a sloop hull + rigging (1680s pesos).
pub const BUILD_SILVER: f32 = 4000.0;
/// Tons of naval stores (pitch, tar, cordage, sailcloth) for outfitting.
pub const BUILD_NAVAL_STORES_TONS: f32 = 8.0;
/// Tons of manufactured goods (iron fittings, tools, ship's stores).
pub const BUILD_MANUFACTURES_TONS: f32 = 5.0;
/// Tons of provisions to outfit the maiden voyage.
pub const BUILD_PROVISIONS_TONS: f32 = 6.0;

/// Months of expected revenue used to amortize the build cost. Median
/// historical Caribbean sloop lifetime is ~7 years (5–10 yr range);
/// 84 months is a reasonable median.
pub const LIFETIME_MONTHS: f32 = 84.0;

/// "Math pencils" hurdle: the shipyard insists the new ship's
/// expected lifetime revenue be at least this many times the build
/// cost. 2.0× ≈ a ~10%/yr return over the assumed lifetime, the
/// historically attractive merchant-investor target.
pub const HURDLE_MULTIPLIER: f32 = 2.0;

/// Lower bound on the starting silver handed to a freshly-built
/// ship. Even if the home port has nothing cheap to export, we don't
/// want a brand-new vessel to be unable to buy *any* cargo.
pub const STARTING_SILVER_FLOOR: f32 = 2000.0;

/// Cost decomposition of a single build, in pesos. Useful for tests
/// and for diagnostic logging.
#[derive(Debug, Clone, Copy)]
pub struct BuildCost {
    pub silver: f32,
    pub naval_stores: f32,
    pub manufactures: f32,
    pub provisions: f32,
}

impl BuildCost {
    pub fn total(&self) -> f32 {
        self.silver + self.naval_stores + self.manufactures + self.provisions
    }
}

/// Compute the per-unit build cost at this port's current market
/// prices. Naval stores / manufactures / provisions are valued at
/// the buy price (what the shipyard would have to pay if it
/// purchased them on the open wharf).
pub fn build_cost(market: &PortMarket, goods: &GoodsRegistry) -> BuildCost {
    BuildCost {
        silver: BUILD_SILVER,
        naval_stores: BUILD_NAVAL_STORES_TONS * market.buy_price(ids::NAVAL_STORES, goods),
        manufactures: BUILD_MANUFACTURES_TONS * market.buy_price(ids::MANUFACTURES, goods),
        provisions: BUILD_PROVISIONS_TONS * market.buy_price(ids::PROVISIONS, goods),
    }
}

/// Starting silver for a freshly-built ship at this home port: enough
/// to buy one full hold of the average local export, floored at
/// `STARTING_SILVER_FLOOR`. If the port produces nothing locally
/// (rare; happens for some pure-import minor ports), fall back to
/// the floor.
pub fn starting_silver(market: &PortMarket, stats: &ShipStats, goods: &GoodsRegistry) -> f32 {
    let outputs = &market.recipe.monthly_outputs;
    if outputs.is_empty() {
        return STARTING_SILVER_FLOOR;
    }
    let avg_export_price: f32 = outputs
        .iter()
        .map(|(g, _)| market.buy_price(*g, goods))
        .sum::<f32>()
        / outputs.len() as f32;
    let one_hold = stats.cargo_capacity_tons * avg_export_price;
    one_hold.max(STARTING_SILVER_FLOOR)
}

/// Outcome of the monthly "try to build" check at one shipyard port.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BuildOutcome {
    /// Not a shipyard.
    NotAShipyard,
    /// Math didn't pencil (expected lifetime revenue < hurdle × cost).
    BelowHurdle { cost: f32, expected: f32 },
    /// Math penciled but inputs weren't available.
    InsufficientInputs,
    /// A ship was built. Caller should spawn the returned Ship.
    Built {
        cost: f32,
        starting_silver: f32,
    },
}

/// Decide whether `port` should build a ship this month, and if so,
/// debit the port's resources and return a freshly-built `Ship`.
///
/// `port_idx` indexes into the world's port list; it's stored on the
/// new ship as `owner_port` for future remittance.
///
/// `avg_monthly_profit` is the expected per-ship monthly profit (in
/// pesos) used to forecast the new ship's earnings. The world
/// computes this from rolling fleet P/L.
pub fn try_build(
    port: &Port,
    port_idx: usize,
    market: &mut PortMarket,
    goods: &GoodsRegistry,
    stats: &ShipStats,
    avg_monthly_profit: f32,
) -> (BuildOutcome, Option<Ship>) {
    if !port.is_shipyard {
        return (BuildOutcome::NotAShipyard, None);
    }

    let cost = build_cost(market, goods);
    let total_cost = cost.total();
    let expected_lifetime_revenue = LIFETIME_MONTHS * avg_monthly_profit;
    if expected_lifetime_revenue < HURDLE_MULTIPLIER * total_cost {
        return (
            BuildOutcome::BelowHurdle {
                cost: total_cost,
                expected: expected_lifetime_revenue,
            },
            None,
        );
    }

    // The port pays itself for the silver portion (it never leaves
    // the treasury), but it must still have it on the books. We don't
    // require port silver to cover the in-kind portion — that's paid
    // in goods directly from the stockpile.
    if market.silver < BUILD_SILVER {
        return (BuildOutcome::InsufficientInputs, None);
    }

    // Verify inputs are available. For goods the port produces, the
    // shadow-price model permits borrowing against next month — we
    // still allow that here (it raises future prices, which is the
    // intended feedback). For pure imports, the stockpile must cover.
    let need = [
        (ids::NAVAL_STORES, BUILD_NAVAL_STORES_TONS),
        (ids::MANUFACTURES, BUILD_MANUFACTURES_TONS),
        (ids::PROVISIONS, BUILD_PROVISIONS_TONS),
    ];
    for (gid, tons) in need.iter().copied() {
        let in_stock = market.stockpile.get(gid);
        let produces = market.recipe.monthly_outputs.iter().any(|(g, _)| *g == gid);
        if in_stock + 1e-4 < tons && !produces {
            return (BuildOutcome::InsufficientInputs, None);
        }
    }

    // Commit: debit silver, consume inputs (using same wharf-then-
    // hinterland-debt mechanism as buy()).
    market.silver -= BUILD_SILVER;
    for (gid, tons) in need.iter().copied() {
        let in_stock = market.stockpile.get(gid);
        let from_wharf = tons.min(in_stock);
        if from_wharf > 0.0 {
            market.stockpile.remove(gid, from_wharf);
        }
        let from_hinterland = (tons - from_wharf).max(0.0);
        if from_hinterland > 0.0 {
            market.debt.add(gid, from_hinterland);
        }
    }

    let start_silver = starting_silver(market, stats, goods);
    let ship = Ship::freshly_built(port.position, port_idx, start_silver);

    (
        BuildOutcome::Built {
            cost: total_cost,
            starting_silver: start_silver,
        },
        Some(ship),
    )
}

/// Helper: clone a Cargo for snapshotting. Centralized so future
/// refactors of Cargo's internal storage don't break callers.
pub fn snapshot_cargo(c: &Cargo) -> Cargo {
    c.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goods::GoodsRegistry;
    use crate::market::{PortArchetype, PortMarket};
    use crate::port::{Faction, Port};
    use crate::types::Position;

    fn shipyard_port() -> Port {
        Port {
            name: "Boston",
            position: Position::new(0.0, 0.0),
            faction: Faction::England,
            harbor_radius_nm: 20.0,
            is_shipyard: true,
        }
    }

    fn nonshipyard_port() -> Port {
        Port {
            name: "Bridgetown",
            position: Position::new(0.0, 0.0),
            faction: Faction::England,
            harbor_radius_nm: 8.0,
            is_shipyard: false,
        }
    }

    #[test]
    fn non_shipyards_never_build() {
        let goods = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        let stats = ShipStats::sloop();
        let (outcome, ship) = try_build(&nonshipyard_port(), 0, &mut market, &goods, &stats, 1_000_000.0);
        assert_eq!(outcome, BuildOutcome::NotAShipyard);
        assert!(ship.is_none());
    }

    #[test]
    fn shipyard_does_not_build_when_profit_zero() {
        let goods = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        let stats = ShipStats::sloop();
        let (outcome, ship) = try_build(&shipyard_port(), 0, &mut market, &goods, &stats, 0.0);
        assert!(matches!(outcome, BuildOutcome::BelowHurdle { .. }));
        assert!(ship.is_none());
    }

    #[test]
    fn shipyard_builds_when_math_pencils_and_inputs_available() {
        let goods = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        // Seed huge stockpiles for required inputs so wharf isn't the bottleneck.
        market.stockpile.add(ids::NAVAL_STORES, 50.0);
        market.stockpile.add(ids::MANUFACTURES, 50.0);
        market.stockpile.add(ids::PROVISIONS, 50.0);
        let stats = ShipStats::sloop();
        // Very high projected profit so hurdle clears easily.
        let (outcome, ship) = try_build(&shipyard_port(), 0, &mut market, &goods, &stats, 500.0);
        match outcome {
            BuildOutcome::Built { cost, starting_silver } => {
                assert!(cost > BUILD_SILVER);
                assert!(starting_silver >= STARTING_SILVER_FLOOR);
            }
            other => panic!("expected Built, got {:?}", other),
        }
        let ship = ship.expect("ship should be returned");
        assert_eq!(ship.owner_port, Some(0));
    }

    #[test]
    fn build_debits_port_silver_and_consumes_inputs() {
        let goods = GoodsRegistry::starter();
        let mut market = PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        market.stockpile.add(ids::NAVAL_STORES, 50.0);
        market.stockpile.add(ids::MANUFACTURES, 50.0);
        market.stockpile.add(ids::PROVISIONS, 50.0);
        let silver_before = market.silver;
        let ns_before = market.stockpile.get(ids::NAVAL_STORES);
        let stats = ShipStats::sloop();
        let (outcome, _ship) = try_build(&shipyard_port(), 0, &mut market, &goods, &stats, 500.0);
        assert!(matches!(outcome, BuildOutcome::Built { .. }));
        assert!((silver_before - market.silver - BUILD_SILVER).abs() < 1e-3);
        let ns_after = market.stockpile.get(ids::NAVAL_STORES);
        assert!(ns_before - ns_after >= BUILD_NAVAL_STORES_TONS - 1e-3);
    }
}
