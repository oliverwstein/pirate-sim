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

/// Weight applied to the lookahead leg's profit when scoring a
/// circuit. Small because the onward leg is highly speculative:
/// prices and stockpiles will have shifted by the time the ship
/// actually arrives at B and re-plans, and bench calibration showed
/// that anything ≥0.5 causes ships to chase phantom long-distance
/// circuits (drained destination ports forecast huge sell prices
/// that never materialize). 0.15 is large enough to break ties
/// between equal first-leg destinations in favor of the one with
/// onward options, but small enough that the immediate margin still
/// dominates the score.
const ONWARD_LOOKAHEAD_WEIGHT: f32 = 0.15;

/// Penalty (pesos/ton) charged to a candidate first leg whose
/// destination has *no* profitable onward leg back toward another
/// port (after excluding the origin). This is the mechanism that
/// breaks the "Home Bias / Amsterdam Fluyt" pathology from
/// `planning/phase-3-postmortem.md §2`: a ship full of cash sitting at
/// Barbados used to score the Barbados→Martinique leg highly because
/// the leg itself was profitable, even though Martinique had nothing
/// to ship back. With the penalty, the score for "I'll be stranded
/// with empty holds at the dest" drops below the score for "I'll
/// reach a port with onward export options". Sized at ~3× the
/// minimum profit threshold so a marginally-profitable first leg into
/// a dead-end loses to a marginal first leg into a working circuit.
const DEAD_END_PENALTY_PESOS_PER_TON: f32 = 1.5;

/// Temperature (pesos/ton) for the softmax distribution over
/// candidate trades. When `find_best_trade` is called with an RNG,
/// it samples from `exp((score - max_score) / TEMPERATURE)` over all
/// candidates clearing the profit threshold, rather than picking the
/// strict argmax. This breaks ties that would otherwise stampede
/// every same-port ship to the same destination on the same tick.
///
/// At T = 10:
///   - A 0-peso lead over the runner-up gives the top option ~50%
///     mass (ties split evenly).
///   - A 10-peso lead → top option ~73%.
///   - A 30-peso lead → top option ~95%.
///   - A 50-peso lead → top option ~99%.
///
/// I.e., strong opportunities still dominate; weak / similar
/// alternatives draw real probability.
pub const TRADE_CHOICE_TEMPERATURE_PESOS_PER_TON: f32 = 10.0;

/// A planned trade leg: which good to buy, where to take it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TradePlan {
    pub good: GoodId,
    pub dest_port: usize,
    pub estimated_profit_per_ton: f32,
}

/// Optional bias toward the ship's home (owner) port. When `Some`,
/// the planner adds `bias_pesos_per_ton` to the apparent profit of
/// any candidate *circuit* that terminates at home — pulling
/// cash-laden ships back to settle with their owners even when a
/// marginally better foreign opportunity exists. With `bias = 0` or
/// `home_port = None` the planner behaves as a pure profit-maximizer.
///
/// "Terminates at home" means either the first-hop destination *is*
/// home (one-leg trip ending at home) or the best onward leg from
/// the first-hop destination would carry the ship to home. This is
/// the post-cleanup behavior; before the multi-hop refactor the
/// bonus was applied only to the immediate destination, which
/// produced the Barbados↔Martinique oscillation discussed in
/// `planning/phase-3-postmortem.md §2`.
#[derive(Debug, Clone, Copy)]
pub struct HomeBias {
    pub home_port: usize,
    pub bias_pesos_per_ton: f32,
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
///
/// `home_bias` (optional) lets the AI bias the choice toward the
/// ship's owner port; see [`HomeBias`].
///
/// **Two-hop scoring (postmortem §2 cleanup).** Each candidate first
/// leg `A → B` is scored against the best speculative onward leg
/// `B → C` (with `A` excluded as a degenerate immediate-return).
/// The returned plan still covers only the first hop — the captain
/// re-plans at every port (rolling horizon), so we don't commit to
/// a hypothetical C two ports out. The onward profit enters the
/// scoring with `ONWARD_LOOKAHEAD_WEIGHT`, and destinations with no
/// profitable onward leg are penalized by `DEAD_END_PENALTY_PESOS_PER_TON`.
/// **Stochastic choice (postmortem #N).** If `rng` is `Some`, the
/// planner softmax-samples over all candidates clearing
/// `MIN_PROFIT_THRESHOLD_PESOS_PER_TON`, weighted by
/// `exp((score - max_score) / TRADE_CHOICE_TEMPERATURE_PESOS_PER_TON)`.
/// This decorrelates same-port same-tick decisions across a fleet
/// (ships no longer all stampede to the single best destination).
/// If `rng` is `None`, returns the strict argmax — the legacy
/// behavior, used by tests that want deterministic single-ship
/// outcomes.
#[allow(clippy::too_many_arguments)]
pub fn find_best_trade(
    origin_idx: usize,
    ports: &[Port],
    markets: &[PortMarket],
    goods: &GoodsRegistry,
    stats: &ShipStats,
    provision_days_budget: f32,
    home_bias: Option<HomeBias>,
    rng: Option<&mut crate::sim_rng::SimRng>,
) -> Option<TradePlan> {
    if origin_idx >= ports.len() || origin_idx >= markets.len() {
        return None;
    }
    let origin = &ports[origin_idx];
    let origin_market = &markets[origin_idx];

    // Collect every viable candidate. `best` (argmax over score) is
    // tracked alongside so the `rng = None` path returns exactly the
    // same plan as before the stochastic refactor.
    let mut best: Option<(TradePlan, f32)> = None;
    let mut candidates: Vec<(TradePlan, f32)> = Vec::new();
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
            let voyage_days = stats.estimated_voyage_days(dist);
            if voyage_days + REACHABILITY_BUFFER_DAYS > provision_days_budget {
                continue;
            }
            let sell_p = markets[dest_idx].sell_price(good.id, goods);
            let cost = dist * TRADE_COST_PER_TON_NM;
            let profit = sell_p - buy_p - cost;

            // Speculative onward leg: where could the ship go *next*
            // from `dest_idx`, excluding the immediate-return path
            // back to `origin_idx`? `None` means "dead end" — no
            // profitable onward leg above threshold. Same provision
            // budget assumption (the ship will resupply at B before
            // committing); we apply the same reachability filter
            // there.
            let onward = best_single_leg_excluding(
                dest_idx,
                Some(origin_idx),
                ports,
                markets,
                goods,
                stats,
                provision_days_budget,
            );
            let (onward_profit, onward_terminus) = match onward {
                Some((_g, c, p)) => {
                    // Cap onward profit at the first-leg profit: the
                    // lookahead is speculative, so it can't out-vote
                    // the leg we're actually about to commit to.
                    // Without this clamp, a drained downstream port
                    // can forecast a 300+ pesos/ton sell price that
                    // never materializes, and ships chase the phantom
                    // circuit and go bankrupt (bench_trade 730 went
                    // from 90 → 162 bankrupt before this clamp).
                    (p.min(profit), c)
                }
                // Dead-end: penalize. The ship would be stuck at B
                // with empty holds and nowhere worthwhile to go.
                None => (-DEAD_END_PENALTY_PESOS_PER_TON, dest_idx),
            };

            // Apply home bias to the *circuit terminus*, not the
            // first-hop destination. A first-hop dest that *is* home
            // is its own terminus; otherwise the onward leg's
            // destination is the terminus.
            let bonus = match home_bias {
                Some(hb) if hb.home_port == dest_idx || hb.home_port == onward_terminus => {
                    hb.bias_pesos_per_ton
                }
                _ => 0.0,
            };

            let score = profit + ONWARD_LOOKAHEAD_WEIGHT * onward_profit + bonus;

            if profit > MIN_PROFIT_THRESHOLD_PESOS_PER_TON {
                let plan = TradePlan {
                    good: good.id,
                    dest_port: dest_idx,
                    // Report the unbiased first-leg margin — the
                    // score is internal to the planner; analytics
                    // and ROI math want the raw per-ton profit.
                    estimated_profit_per_ton: profit,
                };
                if best.as_ref().is_none_or(|(_, s)| score > *s) {
                    best = Some((plan, score));
                }
                candidates.push((plan, score));
            }
        }
    }

    let (best_plan, max_score) = best?;

    let rng = match rng {
        Some(r) => r,
        // Legacy deterministic path: argmax wins.
        None => return Some(best_plan),
    };

    // Softmax sample over candidates. Subtracting `max_score` keeps
    // the exponentials in [0, 1] for numerical stability. With T =
    // 10 pesos/ton (the default), the top option still wins ~half
    // the time on a flat tie and ~95% on a 30-peso lead, but the
    // mass on near-equivalent alternatives is enough to decorrelate
    // same-port stampedes.
    let temp = TRADE_CHOICE_TEMPERATURE_PESOS_PER_TON.max(0.01);
    let weights: Vec<f32> = candidates
        .iter()
        .map(|(_, s)| ((s - max_score) / temp).exp())
        .collect();
    let total: f32 = weights.iter().sum();
    if !(total.is_finite() && total > 0.0) {
        // Degenerate / NaN — fall back to argmax.
        return Some(best_plan);
    }
    let mut threshold = rng.uniform_f32() * total;
    for ((plan, _), w) in candidates.iter().zip(weights.iter()) {
        threshold -= *w;
        if threshold <= 0.0 {
            return Some(*plan);
        }
    }
    // Floating-point rounding — fall back to the last candidate
    // (mathematically guaranteed to have had nonzero mass).
    Some(candidates.last().map(|(p, _)| *p).unwrap_or(best_plan))
}

/// Single-leg search used by both the public `find_best_trade` (when
/// it needs a lookahead from a candidate destination) and as the
/// future hook for additional callers (e.g., post-prize replan).
/// Excludes `exclude` from candidate destinations if provided — used
/// during lookahead to prevent the planner from "rewarding" an
/// immediate Barbados → Martinique → Barbados bounce as if it were
/// a genuine onward leg. Returns `(good, dest, profit_per_ton)`.
fn best_single_leg_excluding(
    origin_idx: usize,
    exclude: Option<usize>,
    ports: &[Port],
    markets: &[PortMarket],
    goods: &GoodsRegistry,
    stats: &ShipStats,
    provision_days_budget: f32,
) -> Option<(GoodId, usize, f32)> {
    if origin_idx >= ports.len() || origin_idx >= markets.len() {
        return None;
    }
    let origin = &ports[origin_idx];
    let origin_market = &markets[origin_idx];
    let mut best: Option<(GoodId, usize, f32)> = None;
    for good in goods.iter() {
        let buy_p = origin_market.buy_price(good.id, goods);
        if origin_market.stockpile.get(good.id) <= 0.0 {
            continue;
        }
        for (dest_idx, dest) in ports.iter().enumerate() {
            if dest_idx == origin_idx {
                continue;
            }
            if exclude == Some(dest_idx) {
                continue;
            }
            let dist = origin.position.distance(dest.position);
            let voyage_days = stats.estimated_voyage_days(dist);
            if voyage_days + REACHABILITY_BUFFER_DAYS > provision_days_budget {
                continue;
            }
            let sell_p = markets[dest_idx].sell_price(good.id, goods);
            let cost = dist * TRADE_COST_PER_TON_NM;
            let profit = sell_p - buy_p - cost;
            if profit > MIN_PROFIT_THRESHOLD_PESOS_PER_TON
                && best.as_ref().is_none_or(|(_, _, p)| profit > *p)
            {
                best = Some((good.id, dest_idx, profit));
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goods::{ids, GoodsRegistry};
    use crate::market::{PortArchetype, PortMarket};
    use crate::port::Port;
    use crate::ship::ShipStats;
    use crate::types::Position;

    fn synth_port(name: &str, x: f32, y: f32) -> Port {
        Port {
            name: name.to_string(),
            position: Position::new(x, y),
            faction: crate::port::Faction::England,
            harbor_radius_nm: 5.0,
            shipyard: None,
            category: crate::pop::PortCategory::SmallColonial,
        }
    }

    fn full_budget(stats: &ShipStats) -> f32 {
        stats.provision_capacity / stats.daily_provision_consumption()
    }

    #[test]
    fn picks_arbitrage_with_largest_margin() {
        let goods = GoodsRegistry::starter();
        let stats = ShipStats::sloop();
        let ports = vec![synth_port("A", 0.0, 0.0), synth_port("B", 100.0, 0.0)];

        let mut market_a = PortMarket::with_recipe(&goods, PortArchetype::SugarIsland.recipe());
        let mut market_b =
            PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        // Bias A's sugar price low (huge surplus) and B's high (drained).
        market_a.stockpile.add(ids::SUGAR, 10_000.0);
        market_b
            .stockpile
            .remove(ids::SUGAR, market_b.stockpile.get(ids::SUGAR));

        let markets = vec![market_a, market_b];
        let plan = find_best_trade(
            0,
            &ports,
            &markets,
            &goods,
            &stats,
            full_budget(&stats),
            None,
            None,
        )
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
        let ports = vec![synth_port("A", 0.0, 0.0), synth_port("B", 1000.0, 0.0)];
        let markets = vec![
            PortMarket::with_recipe(&goods, PortArchetype::Minor.recipe()),
            PortMarket::with_recipe(&goods, PortArchetype::Minor.recipe()),
        ];
        assert!(find_best_trade(
            0,
            &ports,
            &markets,
            &goods,
            &stats,
            full_budget(&stats),
            None,
            None
        )
        .is_none());
    }

    #[test]
    fn skips_goods_origin_lacks() {
        let goods = GoodsRegistry::starter();
        let stats = ShipStats::sloop();
        let ports = vec![synth_port("A", 0.0, 0.0), synth_port("B", 100.0, 0.0)];
        let mut market_a = PortMarket::with_recipe(&goods, PortArchetype::SugarIsland.recipe());
        // Drain everything in A. Then nothing can be bought.
        let snapshot: Vec<_> = market_a.stockpile.iter().collect();
        for (id, t) in snapshot {
            market_a.stockpile.remove(id, t);
        }
        let market_b =
            PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        let markets = vec![market_a, market_b];
        assert!(find_best_trade(
            0,
            &ports,
            &markets,
            &goods,
            &stats,
            full_budget(&stats),
            None,
            None
        )
        .is_none());
    }

    #[test]
    fn skips_unreachable_destinations() {
        let goods = GoodsRegistry::starter();
        let stats = ShipStats::sloop();
        // B is 50,000 NM away — far beyond what any provision budget
        // could ever cover. Even with a profitable arbitrage we should
        // refuse to commit.
        let ports = vec![synth_port("A", 0.0, 0.0), synth_port("B", 50_000.0, 0.0)];
        let mut market_a = PortMarket::with_recipe(&goods, PortArchetype::SugarIsland.recipe());
        let mut market_b =
            PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        market_a.stockpile.add(ids::SUGAR, 10_000.0);
        market_b
            .stockpile
            .remove(ids::SUGAR, market_b.stockpile.get(ids::SUGAR));
        let markets = vec![market_a, market_b];
        assert!(find_best_trade(
            0,
            &ports,
            &markets,
            &goods,
            &stats,
            full_budget(&stats),
            None,
            None
        )
        .is_none());
    }

    /// Postmortem §2 cleanup: with two equally-profitable first-leg
    /// candidates B (working circuit, has goods to ship onward) and
    /// B' (dead end, no profitable onward leg), the planner must
    /// prefer the working-circuit destination. This is what breaks
    /// the Barbados↔Martinique oscillation: a short dead-end hop
    /// loses to a longer hop that opens a productive next leg.
    #[test]
    fn prefers_working_circuit_over_dead_end() {
        use crate::market::ProductionRecipe;
        let goods = GoodsRegistry::starter();
        let stats = ShipStats::sloop();
        // A = sugar exporter (and ONLY sugar).
        // B-deadend and B-working both consume sugar (identical
        // first-leg attractiveness as buyers). B-working additionally
        // produces tobacco. D consumes tobacco.
        let ports = vec![
            synth_port("A", 0.0, 0.0),
            synth_port("B-deadend", 80.0, 0.0),
            synth_port("B-working", 0.0, 80.0),
            synth_port("D", 0.0, 160.0),
        ];

        let sugar_exporter = ProductionRecipe {
            monthly_outputs: vec![(ids::SUGAR, 100.0)],
            monthly_inputs: vec![],
            prosperity: 1.0,
        };
        let sugar_consumer = ProductionRecipe {
            monthly_outputs: vec![],
            monthly_inputs: vec![(ids::SUGAR, 50.0)],
            prosperity: 1.0,
        };
        let sugar_consumer_and_tobacco_exporter = ProductionRecipe {
            monthly_outputs: vec![(ids::TOBACCO, 100.0)],
            monthly_inputs: vec![(ids::SUGAR, 50.0)],
            prosperity: 1.0,
        };
        let tobacco_consumer = ProductionRecipe {
            monthly_outputs: vec![],
            monthly_inputs: vec![(ids::TOBACCO, 50.0)],
            prosperity: 1.0,
        };

        let market_a = PortMarket::with_recipe(&goods, sugar_exporter);
        // Drain sugar buyers so they pay premium for incoming sugar.
        let mut market_b_dead = PortMarket::with_recipe(&goods, sugar_consumer);
        market_b_dead
            .stockpile
            .remove(ids::SUGAR, market_b_dead.stockpile.get(ids::SUGAR));
        let market_b_work = PortMarket::with_recipe(&goods, sugar_consumer_and_tobacco_exporter);
        // Drain B-working sugar too so both B's pay the same premium
        // for incoming sugar (recipe seeds 3 months of input buffer
        // = 150 t; we want both buyers to start at identical 0-stock
        // demand state).
        let mut market_b_work = market_b_work;
        market_b_work
            .stockpile
            .remove(ids::SUGAR, market_b_work.stockpile.get(ids::SUGAR));
        let mut market_d = PortMarket::with_recipe(&goods, tobacco_consumer);
        market_d
            .stockpile
            .remove(ids::TOBACCO, market_d.stockpile.get(ids::TOBACCO));

        let markets = vec![market_a, market_b_dead, market_b_work, market_d];
        let plan = find_best_trade(
            0,
            &ports,
            &markets,
            &goods,
            &stats,
            full_budget(&stats),
            None,
            None,
        )
        .expect("at least one profitable first leg exists");
        // Planner must pick the working circuit (B-working = idx 2),
        // not the dead-end (idx 1).
        assert_eq!(
            plan.dest_port, 2,
            "dead-end destination should lose to working circuit"
        );
    }
}
