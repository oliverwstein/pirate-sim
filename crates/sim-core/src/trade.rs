//! Trade planning: pick a profitable (good, destination) pair from a
//! port. The evaluator is intentionally simple — it loops every good
//! and every port, computes per-ton arbitrage minus a linear distance
//! cost, and returns the highest-margin option above a small threshold.
//!
//! It does not consider available silver, hold size, or stockpile when
//! ranking — those are enforced by the actual buy/sell call. A future
//! refinement can switch to "expected total profit for this trip" by
//! multiplying margin by a feasibility-aware tonnage.

use crate::goods::{GoodId, GoodsRegistry};
use crate::market::PortMarket;
use crate::port::Port;
use crate::ship::ShipStats;

/// Approximate per-ton-mile cost charged against arbitrage profits.
/// Captures wages, victualling, hull wear, and convoy fees in a single
/// coarse number. Tuned so a 1500 NM run costs ~1.5 pesos/t — long
/// hauls remain viable so non-gateway ports can profitably feed
/// gateways. (Piracy/wreck risk is intentionally NOT priced in here;
/// in the absence of those threats, successful voyages should be
/// reliably profitable.)
pub const TRADE_COST_PER_TON_NM: f32 = 0.001;

/// Below this margin (pesos / ton) we consider a trade not worth doing
/// and the ship sails ballast to the next port the AI picks.
pub const MIN_PROFIT_THRESHOLD_PESOS_PER_TON: f32 = 0.5;

/// Extra days of provisions the AI insists on having beyond the
/// estimated voyage time before committing to a destination. Covers
/// weather slow-downs, off-track detours, and resupply queue time
/// at the destination port.
pub const REACHABILITY_BUFFER_DAYS: f32 = 7.0;

/// A planned trade leg: which good to buy, where to take it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TradePlan {
    pub good: GoodId,
    pub dest_port: usize,
    pub estimated_profit_per_ton: f32,
}

/// Search every (good, destination) pair from `origin_idx` and return
/// the highest-margin option, or `None` if nothing clears the
/// threshold. Distance cost is computed against the great-circle
/// distance between port positions (good enough for ranking).
///
/// `provision_days_budget` is the number of days of provisions the
/// ship will sail with after resupply (typically `provision_capacity /
/// daily_consumption`). Destinations whose estimated voyage time plus
/// `REACHABILITY_BUFFER_DAYS` exceeds the budget are skipped — the AI
/// won't commit to a leg it can't physically reach.
pub fn find_best_trade(
    origin_idx: usize,
    ports: &[Port],
    markets: &[PortMarket],
    goods: &GoodsRegistry,
    stats: &ShipStats,
    provision_days_budget: f32,
) -> Option<TradePlan> {
    if origin_idx >= ports.len() || origin_idx >= markets.len() {
        return None;
    }
    let origin = &ports[origin_idx];
    let origin_market = &markets[origin_idx];

    let mut best: Option<TradePlan> = None;
    for good in goods.iter() {
        let buy_p = origin_market.buy_price(good.id, goods);
        // Refuse to even consider goods the origin is dry on — saves
        // a bunch of pointless candidates.
        if origin_market.stockpile.get(good.id) <= 0.0 {
            continue;
        }
        for (dest_idx, dest) in ports.iter().enumerate() {
            if dest_idx == origin_idx {
                continue;
            }
            let dist = origin.position.distance(dest.position);
            // Reachability gate: skip any destination we can't make
            // even after fully resupplying. The AI may still divert to
            // unreachable ports as an emergency, but it won't *commit*
            // to one as a profitable trade leg.
            let voyage_days = stats.estimated_voyage_days(dist);
            if voyage_days + REACHABILITY_BUFFER_DAYS > provision_days_budget {
                continue;
            }
            let sell_p = markets[dest_idx].sell_price(good.id, goods);
            let cost = dist * TRADE_COST_PER_TON_NM;
            let profit = sell_p - buy_p - cost;
            if profit > MIN_PROFIT_THRESHOLD_PESOS_PER_TON
                && best.as_ref().map_or(true, |b| profit > b.estimated_profit_per_ton)
            {
                best = Some(TradePlan {
                    good: good.id,
                    dest_port: dest_idx,
                    estimated_profit_per_ton: profit,
                });
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goods::{GoodsRegistry, ids};
    use crate::market::{PortArchetype, PortMarket};
    use crate::port::Port;
    use crate::ship::ShipStats;
    use crate::types::Position;

    fn synth_port(name: &'static str, x: f32, y: f32) -> Port {
        Port {
            name,
            position: Position::new(x, y),
            faction: crate::port::Faction::England,
            harbor_radius_nm: 5.0,
        }
    }

    fn full_budget(stats: &ShipStats) -> f32 {
        stats.provision_capacity / stats.daily_provision_consumption()
    }

    #[test]
    fn picks_arbitrage_with_largest_margin() {
        let goods = GoodsRegistry::starter();
        let stats = ShipStats::sloop();
        let ports = vec![
            synth_port("A", 0.0, 0.0),
            synth_port("B", 100.0, 0.0),
        ];

        let mut market_a = PortMarket::with_recipe(&goods, PortArchetype::SugarIsland.recipe());
        let mut market_b = PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        // Bias A's sugar price low (huge surplus) and B's high (drained).
        market_a.stockpile.add(ids::SUGAR, 10_000.0);
        market_b.stockpile.remove(ids::SUGAR, market_b.stockpile.get(ids::SUGAR));

        let markets = vec![market_a, market_b];
        let plan = find_best_trade(0, &ports, &markets, &goods, &stats, full_budget(&stats))
            .expect("arbitrage should exist");
        assert_eq!(plan.dest_port, 1);
        assert_eq!(plan.good, ids::SUGAR);
        assert!(plan.estimated_profit_per_ton > 0.0);
    }

    #[test]
    fn returns_none_when_no_profit() {
        let goods = GoodsRegistry::starter();
        let stats = ShipStats::sloop();
        // Two identical "Minor" ports with the same recipe → every
        // good's price is identical at both ends, so the round trip
        // strictly loses (spread + distance cost).
        let ports = vec![
            synth_port("A", 0.0, 0.0),
            synth_port("B", 1000.0, 0.0),
        ];
        let markets = vec![
            PortMarket::with_recipe(&goods, PortArchetype::Minor.recipe()),
            PortMarket::with_recipe(&goods, PortArchetype::Minor.recipe()),
        ];
        assert!(find_best_trade(0, &ports, &markets, &goods, &stats, full_budget(&stats)).is_none());
    }

    #[test]
    fn skips_goods_origin_lacks() {
        let goods = GoodsRegistry::starter();
        let stats = ShipStats::sloop();
        let ports = vec![
            synth_port("A", 0.0, 0.0),
            synth_port("B", 100.0, 0.0),
        ];
        let mut market_a = PortMarket::with_recipe(&goods, PortArchetype::SugarIsland.recipe());
        // Drain everything in A. Then nothing can be bought.
        let snapshot: Vec<_> = market_a.stockpile.iter()
            .map(|(id, t)| (id, t)).collect();
        for (id, t) in snapshot {
            market_a.stockpile.remove(id, t);
        }
        let market_b = PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        let markets = vec![market_a, market_b];
        assert!(find_best_trade(0, &ports, &markets, &goods, &stats, full_budget(&stats)).is_none());
    }

    #[test]
    fn skips_unreachable_destinations() {
        let goods = GoodsRegistry::starter();
        let stats = ShipStats::sloop();
        // B is 50,000 NM away — far beyond what any provision budget
        // could ever cover. Even with a profitable arbitrage we should
        // refuse to commit.
        let ports = vec![
            synth_port("A", 0.0, 0.0),
            synth_port("B", 50_000.0, 0.0),
        ];
        let mut market_a = PortMarket::with_recipe(&goods, PortArchetype::SugarIsland.recipe());
        let mut market_b = PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        market_a.stockpile.add(ids::SUGAR, 10_000.0);
        market_b.stockpile.remove(ids::SUGAR, market_b.stockpile.get(ids::SUGAR));
        let markets = vec![market_a, market_b];
        assert!(find_best_trade(0, &ports, &markets, &goods, &stats, full_budget(&stats)).is_none());
    }
}
