//! Shipbuilding: shipyard ports decide each month whether to commission
//! a new merchant vessel, based on whether the math pencils out.
//!
//! For each ship type the yard is equipped to build:
//!
//!   build_cost  =  type.build_silver
//!                + type.build_naval_stores  * naval_stores_buy_price
//!                + type.build_manufactures  * manufactures_buy_price
//!                + type.build_provisions    * provisions_buy_price
//!
//!   build iff
//!       type.expected_lifetime_months
//!         * avg_monthly_fleet_profit
//!         * (type.cargo_capacity_tons / REFERENCE_CARGO_TONS)
//!         >  HURDLE_MULTIPLIER * build_cost
//!     AND  port has all inputs in stock and silver to pay
//!
//! Each shipyard tries each of its allowed types and picks the one
//! with the highest return-on-cost (`(revenue − hurdle*cost) / cost`).
//! ROI, rather than absolute surplus, is what surfaces the genuine
//! cost-per-ton advantage of types like the Fluyt — otherwise the
//! largest hull always wins.
//! At most one ship is built per port per month — keeps the system
//! from carpet-bombing the seas.
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

use crate::goods::{ids, GoodsRegistry};
use crate::market::PortMarket;
use crate::port::Port;
use crate::ship::Ship;
use crate::shiptype::{ShipType, ShipTypeId, ShipTypeRegistry};

/// "Math pencils" hurdle: the shipyard insists the new ship's
/// expected lifetime revenue be at least this many times the build
/// cost. 2.0× ≈ a ~10%/yr return over the assumed lifetime, the
/// historically attractive merchant-investor target.
pub const HURDLE_MULTIPLIER: f32 = 2.0;

/// Reference cargo capacity (tons) used to scale expected revenue
/// from `avg_monthly_profit`. The fleet-wide profit number is a
/// per-ship average; without scaling, every type's revenue would be
/// `lifetime × that_one_number`, so longer-lived hulls always win
/// regardless of size. Scaling by `cargo / REFERENCE_CARGO_TONS`
/// preserves the existing calibration for the sloop (60 tons → 1.0×)
/// while crediting larger hulls with proportionally more expected
/// revenue, so Fluyts and Ships can also pencil out at the right
/// kind of yard.
pub const REFERENCE_CARGO_TONS: f32 = 60.0;

/// Lower bound on the starting silver handed to a freshly-built
/// ship. Even if the home port has nothing cheap to export, we don't
/// want a brand-new vessel to be unable to buy *any* cargo.
pub const STARTING_SILVER_FLOOR: f32 = 2000.0;

/// Soft cap on starting silver: at most this fraction of the port's
/// treasury, so a flagship Amsterdam ship can't bankrupt its own
/// home port at launch.
pub const STARTING_SILVER_PORT_FRACTION_CAP: f32 = 0.5;

/// Cost decomposition of a single build, in pesos.
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

/// Compute the build cost for a specific type at this port's current
/// market prices.
pub fn build_cost(ty: &ShipType, market: &PortMarket, goods: &GoodsRegistry) -> BuildCost {
    BuildCost {
        silver: ty.build_silver,
        naval_stores: ty.build_naval_stores * market.buy_price(ids::NAVAL_STORES, goods),
        manufactures: ty.build_manufactures * market.buy_price(ids::MANUFACTURES, goods),
        provisions: ty.build_provisions * market.buy_price(ids::PROVISIONS, goods),
    }
}

/// Starting silver for a freshly-built ship of this type at this
/// home port: enough to buy one full hold of the average local
/// export, floored at `STARTING_SILVER_FLOOR` and capped at
/// `STARTING_SILVER_PORT_FRACTION_CAP × port_silver`.
pub fn starting_silver(ty: &ShipType, market: &PortMarket, goods: &GoodsRegistry) -> f32 {
    let outputs = &market.recipe.monthly_outputs;
    let raw = if outputs.is_empty() {
        STARTING_SILVER_FLOOR
    } else {
        let avg_export_price: f32 = outputs
            .iter()
            .map(|(g, _)| market.buy_price(*g, goods))
            .sum::<f32>()
            / outputs.len() as f32;
        ty.stats.cargo_capacity_tons * avg_export_price
    };
    let cap = market.silver * STARTING_SILVER_PORT_FRACTION_CAP;
    raw.max(STARTING_SILVER_FLOOR)
        .min(cap.max(STARTING_SILVER_FLOOR))
}

/// Why one specific type was rejected (or accepted) at a yard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TypeEvaluation {
    Built,
    BelowHurdle,
    InsufficientPortSilver,
    InsufficientInputs,
}

/// Outcome of the monthly build attempt at one yard.
#[derive(Debug, Clone)]
pub enum BuildOutcome {
    NotAShipyard,
    NoTypePencils, // all allowed types failed the hurdle or input checks
    Built {
        ship_type: ShipTypeId,
        cost: f32,
        starting_silver: f32,
    },
}

/// Decide whether `port` should build a ship this month, and if so,
/// debit the port's resources and return a freshly-built `Ship`.
///
/// Iterates each of the port's allowed types and picks the one with
/// the largest `expected_lifetime_revenue − HURDLE_MULTIPLIER * cost`
/// margin (only types passing the hurdle and with inputs available
/// are considered). Returns `(NoTypePencils, None)` if no type works.
pub fn try_build(
    port: &Port,
    port_idx: usize,
    market: &mut PortMarket,
    goods: &GoodsRegistry,
    types: &ShipTypeRegistry,
    avg_monthly_profit: f32,
) -> (BuildOutcome, Option<Ship>) {
    let allowed = match &port.shipyard {
        Some(list) if !list.is_empty() => list,
        _ => return (BuildOutcome::NotAShipyard, None),
    };

    // Evaluate every allowed type; keep the best surplus-per-silver-of-cost
    // (i.e. ROI) among those that clear all gates. Absolute surplus alone
    // always favors the largest type (more cargo × longer lifetime), which
    // hides the genuine cost-per-ton advantage of types like the Fluyt;
    // ROI compares like with like across hull sizes.
    let mut best: Option<(ShipTypeId, f32, f32)> = None; // (id, total_cost, roi)
    for &tid in allowed {
        let ty = types.get(tid);
        let cost = build_cost(ty, market, goods).total();
        let cargo_scale = ty.stats.cargo_capacity_tons / REFERENCE_CARGO_TONS;
        let revenue = ty.expected_lifetime_months * avg_monthly_profit * cargo_scale;
        let surplus = revenue - HURDLE_MULTIPLIER * cost;
        if surplus <= 0.0 {
            continue;
        }
        if market.silver < ty.build_silver {
            continue;
        }
        // Verify input availability. For goods the port produces, the
        // shadow-price model allows borrowing against next month — so
        // we still permit those. For pure imports, the wharf must cover.
        let need = [
            (ids::NAVAL_STORES, ty.build_naval_stores),
            (ids::MANUFACTURES, ty.build_manufactures),
            (ids::PROVISIONS, ty.build_provisions),
        ];
        let mut inputs_ok = true;
        for (gid, tons) in need.iter().copied() {
            let in_stock = market.stockpile.get(gid);
            let produces = market.recipe.monthly_outputs.iter().any(|(g, _)| *g == gid);
            if in_stock + 1e-4 < tons && !produces {
                inputs_ok = false;
                break;
            }
        }
        if !inputs_ok {
            continue;
        }
        let roi = surplus / cost.max(1.0);
        match best {
            Some((_, _, prev_roi)) if prev_roi >= roi => {}
            _ => best = Some((tid, cost, roi)),
        }
    }

    let (chosen, total_cost, _roi) = match best {
        Some(b) => b,
        None => return (BuildOutcome::NoTypePencils, None),
    };
    let ty = types.get(chosen);

    // Commit: debit silver and inputs.
    market.silver -= ty.build_silver;
    let need = [
        (ids::NAVAL_STORES, ty.build_naval_stores),
        (ids::MANUFACTURES, ty.build_manufactures),
        (ids::PROVISIONS, ty.build_provisions),
    ];
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

    let start_silver = starting_silver(ty, market, goods);
    let ship = Ship::freshly_built(
        port.position,
        port_idx,
        start_silver,
        chosen,
        &ty.stats,
        port.faction,
    );

    (
        BuildOutcome::Built {
            ship_type: chosen,
            cost: total_cost,
            starting_silver: start_silver,
        },
        Some(ship),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goods::GoodsRegistry;
    use crate::market::{PortArchetype, PortMarket};
    use crate::port::{Faction, Port};
    use crate::shiptype::{ids as st_ids, ShipTypeRegistry};
    use crate::types::Position;

    fn yard_port(name: &str, allowed: &[ShipTypeId]) -> Port {
        Port {
            name: name.to_string(),
            position: Position::new(0.0, 0.0),
            faction: Faction::England,
            harbor_radius_nm: 20.0,
            shipyard: Some(allowed.to_vec()),
            category: crate::pop::PortCategory::SmallColonial,
        }
    }

    fn nonyard() -> Port {
        Port {
            name: "Bridgetown".to_string(),
            position: Position::new(0.0, 0.0),
            faction: Faction::England,
            harbor_radius_nm: 8.0,
            shipyard: None,
            category: crate::pop::PortCategory::SmallColonial,
        }
    }

    fn well_stocked_market() -> PortMarket {
        let goods = GoodsRegistry::starter();
        let mut market =
            PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        market.stockpile.add(ids::NAVAL_STORES, 200.0);
        market.stockpile.add(ids::MANUFACTURES, 200.0);
        market.stockpile.add(ids::PROVISIONS, 200.0);
        market.silver = 100_000.0;
        market
    }

    #[test]
    fn non_shipyards_never_build() {
        let goods = GoodsRegistry::starter();
        let mut market = well_stocked_market();
        let types = ShipTypeRegistry::starter();
        let (outcome, ship) = try_build(&nonyard(), 0, &mut market, &goods, &types, 1_000_000.0);
        assert!(matches!(outcome, BuildOutcome::NotAShipyard));
        assert!(ship.is_none());
    }

    #[test]
    fn zero_profit_yields_no_build() {
        let goods = GoodsRegistry::starter();
        let mut market = well_stocked_market();
        let types = ShipTypeRegistry::starter();
        let port = yard_port("Boston", &[st_ids::SLOOP, st_ids::BRIGANTINE, st_ids::BARK]);
        let (outcome, ship) = try_build(&port, 0, &mut market, &goods, &types, 0.0);
        assert!(matches!(outcome, BuildOutcome::NoTypePencils));
        assert!(ship.is_none());
    }

    #[test]
    fn bermuda_only_builds_sloops() {
        let goods = GoodsRegistry::starter();
        let mut market = well_stocked_market();
        let types = ShipTypeRegistry::starter();
        let port = yard_port("Bermuda", &[st_ids::SLOOP]);
        let (outcome, ship) = try_build(&port, 0, &mut market, &goods, &types, 2000.0);
        match outcome {
            BuildOutcome::Built { ship_type, .. } => assert_eq!(ship_type, st_ids::SLOOP),
            o => panic!("expected Built(sloop), got {:?}", o),
        }
        assert_eq!(ship.unwrap().ship_type, st_ids::SLOOP);
    }

    #[test]
    fn amsterdam_with_high_profit_prefers_ship_over_fluyt() {
        // Both clear the hurdle but Ship has larger cargo × better
        // amortization-windowed revenue, so surplus is higher.
        let goods = GoodsRegistry::starter();
        let mut market = well_stocked_market();
        let types = ShipTypeRegistry::starter();
        let port = yard_port("Amsterdam", &[st_ids::FLUYT, st_ids::SHIP]);
        let (outcome, ship) = try_build(&port, 0, &mut market, &goods, &types, 5000.0);
        match outcome {
            BuildOutcome::Built { ship_type, .. } => {
                assert!(ship_type == st_ids::SHIP || ship_type == st_ids::FLUYT);
            }
            o => panic!("expected Built, got {:?}", o),
        }
        assert!(ship.unwrap().owner_port == Some(0));
    }

    #[test]
    fn build_debits_port_silver_and_consumes_inputs() {
        let goods = GoodsRegistry::starter();
        let mut market = well_stocked_market();
        let types = ShipTypeRegistry::starter();
        let port = yard_port("Boston", &[st_ids::SLOOP]);
        let silver_before = market.silver;
        let ns_before = market.stockpile.get(ids::NAVAL_STORES);
        let (outcome, _ship) = try_build(&port, 0, &mut market, &goods, &types, 500.0);
        assert!(matches!(outcome, BuildOutcome::Built { .. }));
        let sloop = types.get(st_ids::SLOOP);
        assert!((silver_before - market.silver - sloop.build_silver).abs() < 1e-2);
        assert!(
            ns_before - market.stockpile.get(ids::NAVAL_STORES) >= sloop.build_naval_stores - 1e-3
        );
    }

    #[test]
    fn freshly_built_ship_starts_hiring_with_no_crew() {
        let goods = GoodsRegistry::starter();
        let mut market = well_stocked_market();
        let types = ShipTypeRegistry::starter();
        let port = yard_port("Boston", &[st_ids::SLOOP]);
        let (outcome, ship) = try_build(&port, 0, &mut market, &goods, &types, 2000.0);
        assert!(matches!(outcome, BuildOutcome::Built { .. }));
        let ship = ship.expect("ship");
        assert_eq!(ship.state, crate::ship::ShipState::Hiring);
        assert_eq!(ship.crew_alive, 0);
    }
}
