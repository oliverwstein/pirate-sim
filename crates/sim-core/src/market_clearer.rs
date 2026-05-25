//! Pure fixed-point auction clearer for the bounded signed-balance market.
//!
//! This module intentionally has no dependency on `World` or `PortMarket`.
//! Phase B.3 will adapt collected market intents into these simple order
//! types and apply the returned fills back to ships and ports.

use crate::market_curve::{invert_multiplier, price_multiplier};
use std::collections::BTreeMap;

const MAX_BISECTION_ITERS: usize = 32;
const PRICE_EPS: f32 = 0.001;
const TON_EPS: f32 = 0.000_001;

/// A ship's willingness to BUY up to `tons` of a good at no more than
/// `max_price_pesos_per_ton`. Identical to existing `MarketBid` data
/// (price ceiling, tonnage, payer identity). Order-independent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClearBid {
    pub ship_id: u32,
    pub tons: f32,
    pub max_price_pesos_per_ton: f32,
}

/// A ship's willingness to SELL up to `tons` at no less than
/// `min_price_pesos_per_ton`. Mirror of ClearBid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClearAsk {
    pub ship_id: u32,
    pub tons: f32,
    pub min_price_pesos_per_ton: f32,
}

/// What the clearer returns per ship: tons traded (positive = ship
/// bought, negative = ship sold), pesos paid (positive = ship paid,
/// negative = ship received). For Phase B.3 the caller mutates ship +
/// port state from this list.
#[derive(Clone, Debug, PartialEq)]
pub struct ClearFill {
    pub ship_id: u32,
    pub tons_signed: f32,
    pub pesos_signed: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClearResult {
    pub clearing_price_pesos_per_ton: f32,
    pub new_balance: i32,
    pub fills: Vec<ClearFill>,
}

#[derive(Clone, Copy, Debug)]
struct WorkFill {
    ship_id: u32,
    tons_signed: f32,
    marginal: bool,
}

/// Fixed-point auction clearer for one port × one good.
///
/// Inputs:
/// - `base_price_pesos_per_ton` — the good's flat base price (from `GoodsRegistry`).
/// - `current_balance` — port's current signed balance for this good.
/// - `effective_bound` — port's effective bound for this good (positive int).
/// - `bids` — ship buy orders (collected from `MarketBid` intents this tick).
/// - `asks` — ship sell orders (collected from `MarketAsk` intents this tick).
///
/// Returns: clearing price + resulting balance + per-ship fills. Empty
/// `fills` if no trade clears (e.g. all bids below min ask, all asks
/// above max bid AT the post-trade balance).
///
/// Algorithm: find clearing price P* such that
///     P* = base_price × price_multiplier(current_balance + ΔQ(P*), effective_bound)
/// where ΔQ(P*) = sum of asks satisfied at P* − sum of bids satisfied at P*)
/// (asks accept any p ≥ min_ask, bids accept any p ≤ max_bid). Uses
/// binary search over price; monotonicity guarantees uniqueness.
///
/// Determinism: bids/asks are sorted by ship_id before marginal matching
/// so the function is independent of input order.
pub fn clear(
    base_price_pesos_per_ton: f32,
    current_balance: i32,
    effective_bound: i32,
    bids: &[ClearBid],
    asks: &[ClearAsk],
) -> ClearResult {
    let bound = effective_bound.max(1);
    let current_balance = current_balance.clamp(-bound, bound);

    // Guard against malformed base price: a non-finite or non-positive
    // base would produce nonsense prices and break the monotonicity
    // assumptions of the bisection. Treat it as a no-trade tick.
    if !base_price_pesos_per_ton.is_finite() || base_price_pesos_per_ton <= 0.0 {
        return ClearResult {
            clearing_price_pesos_per_ton: 0.0,
            new_balance: current_balance,
            fills: Vec::new(),
        };
    }

    let current_price = price_at_balance(base_price_pesos_per_ton, current_balance, bound);

    let mut bids = normalized_bids(bids);
    let mut asks = normalized_asks(asks);
    bids.sort_by_key(|b| b.ship_id);
    asks.sort_by_key(|a| a.ship_id);

    if bids.is_empty() && asks.is_empty() {
        return ClearResult {
            clearing_price_pesos_per_ton: current_price,
            new_balance: current_balance,
            fills: Vec::new(),
        };
    }

    let total_bid_tons: f32 = bids.iter().map(|b| b.tons).sum();
    let total_ask_tons: f32 = asks.iter().map(|a| a.tons).sum();
    let low_balance = rounded_balance(current_balance as f32 + total_ask_tons, bound);
    let high_balance = rounded_balance(current_balance as f32 - total_bid_tons, bound);
    let mut lo = price_at_balance(base_price_pesos_per_ton, low_balance, bound);
    let mut hi = price_at_balance(base_price_pesos_per_ton, high_balance, bound);
    if lo > hi {
        std::mem::swap(&mut lo, &mut hi);
    }

    let clearing_price = snap_to_nearby_limit(
        bisect_price(
            base_price_pesos_per_ton,
            current_balance,
            bound,
            &bids,
            &asks,
            lo,
            hi,
        ),
        &bids,
        &asks,
    );
    let (fills, new_balance) = build_fills(
        base_price_pesos_per_ton,
        current_balance,
        bound,
        &bids,
        &asks,
        clearing_price,
    );

    if fills.is_empty() {
        ClearResult {
            clearing_price_pesos_per_ton: current_price,
            new_balance: current_balance,
            fills,
        }
    } else {
        ClearResult {
            clearing_price_pesos_per_ton: clearing_price,
            new_balance,
            fills,
        }
    }
}

fn normalized_bids(bids: &[ClearBid]) -> Vec<ClearBid> {
    bids.iter()
        .copied()
        .filter(|b| b.tons.is_finite() && b.tons > 0.0 && b.max_price_pesos_per_ton.is_finite())
        .collect()
}

fn normalized_asks(asks: &[ClearAsk]) -> Vec<ClearAsk> {
    asks.iter()
        .copied()
        .filter(|a| a.tons.is_finite() && a.tons > 0.0 && a.min_price_pesos_per_ton.is_finite())
        .collect()
}

fn bisect_price(
    base_price: f32,
    current_balance: i32,
    bound: i32,
    bids: &[ClearBid],
    asks: &[ClearAsk],
    mut lo: f32,
    mut hi: f32,
) -> f32 {
    for _ in 0..MAX_BISECTION_ITERS {
        if hi - lo < PRICE_EPS {
            break;
        }
        let mid = (lo + hi) * 0.5;
        let delta = accepted_delta_at_price(mid, bids, asks);
        let balance = rounded_balance(current_balance as f32 + delta, bound);
        let curve_price = price_at_balance(base_price, balance, bound);
        if curve_price > mid {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) * 0.5
}

fn snap_to_nearby_limit(price: f32, bids: &[ClearBid], asks: &[ClearAsk]) -> f32 {
    let mut best = price;
    let mut best_distance = PRICE_EPS;
    for limit in bids
        .iter()
        .map(|b| b.max_price_pesos_per_ton)
        .chain(asks.iter().map(|a| a.min_price_pesos_per_ton))
    {
        let distance = (limit - price).abs();
        if distance <= best_distance {
            best = limit;
            best_distance = distance;
        }
    }
    best
}

fn accepted_delta_at_price(price: f32, bids: &[ClearBid], asks: &[ClearAsk]) -> f32 {
    let ask_tons: f32 = asks
        .iter()
        .filter(|a| price + PRICE_EPS >= a.min_price_pesos_per_ton)
        .map(|a| a.tons)
        .sum();
    let bid_tons: f32 = bids
        .iter()
        .filter(|b| price <= b.max_price_pesos_per_ton + PRICE_EPS)
        .map(|b| b.tons)
        .sum();
    ask_tons - bid_tons
}

fn build_fills(
    base_price: f32,
    current_balance: i32,
    bound: i32,
    bids: &[ClearBid],
    asks: &[ClearAsk],
    price: f32,
) -> (Vec<ClearFill>, i32) {
    let mut fills = Vec::new();
    let mut marginal_bids = Vec::new();
    let mut marginal_asks = Vec::new();

    for ask in asks {
        if ask.min_price_pesos_per_ton < price - PRICE_EPS {
            fills.push(WorkFill {
                ship_id: ask.ship_id,
                tons_signed: -ask.tons,
                marginal: false,
            });
        } else if approx_price(ask.min_price_pesos_per_ton, price) {
            marginal_asks.push(*ask);
        }
    }
    for bid in bids {
        if bid.max_price_pesos_per_ton > price + PRICE_EPS {
            fills.push(WorkFill {
                ship_id: bid.ship_id,
                tons_signed: bid.tons,
                marginal: false,
            });
        } else if approx_price(bid.max_price_pesos_per_ton, price) {
            marginal_bids.push(*bid);
        }
    }

    let full_delta = balance_delta(&fills);
    if !marginal_bids.is_empty() || !marginal_asks.is_empty() {
        let marginal_bid_tons: f32 = marginal_bids.iter().map(|b| b.tons).sum();
        let marginal_ask_tons: f32 = marginal_asks.iter().map(|a| a.tons).sum();
        let target_balance = continuous_balance_for_price(base_price, price, bound);
        let target_delta = (target_balance - current_balance as f32)
            .clamp(
                full_delta - marginal_bid_tons,
                full_delta + marginal_ask_tons,
            )
            .clamp(
                -(current_balance + bound) as f32,
                (bound - current_balance) as f32,
            );

        if target_delta > full_delta + TON_EPS {
            push_ask_fills(&mut fills, &marginal_asks, target_delta - full_delta, true);
        } else if target_delta < full_delta - TON_EPS {
            push_bid_fills(&mut fills, &marginal_bids, full_delta - target_delta, true);
        }
    }

    trim_to_bounds(&mut fills, current_balance, bound);
    fills.retain(|f| f.tons_signed.abs() > TON_EPS);

    let final_delta = balance_delta(&fills);
    let new_balance = rounded_balance(current_balance as f32 + final_delta, bound);
    (aggregate_fills(fills, price), new_balance)
}

fn push_ask_fills(fills: &mut Vec<WorkFill>, asks: &[ClearAsk], mut tons: f32, marginal: bool) {
    for ask in asks {
        if tons <= TON_EPS {
            break;
        }
        let take = ask.tons.min(tons);
        if take > TON_EPS {
            fills.push(WorkFill {
                ship_id: ask.ship_id,
                tons_signed: -take,
                marginal,
            });
            tons -= take;
        }
    }
}

fn push_bid_fills(fills: &mut Vec<WorkFill>, bids: &[ClearBid], mut tons: f32, marginal: bool) {
    for bid in bids {
        if tons <= TON_EPS {
            break;
        }
        let take = bid.tons.min(tons);
        if take > TON_EPS {
            fills.push(WorkFill {
                ship_id: bid.ship_id,
                tons_signed: take,
                marginal,
            });
            tons -= take;
        }
    }
}

fn trim_to_bounds(fills: &mut [WorkFill], current_balance: i32, bound: i32) {
    let max_delta = (bound - current_balance) as f32;
    let min_delta = -(current_balance + bound) as f32;
    let delta = balance_delta(fills);
    if delta > max_delta + TON_EPS {
        trim_asks(fills, delta - max_delta);
    } else if delta < min_delta - TON_EPS {
        trim_bids(fills, min_delta - delta);
    }
}

fn trim_asks(fills: &mut [WorkFill], mut tons: f32) {
    trim_side(fills, &mut tons, true, true);
    trim_side(fills, &mut tons, true, false);
}

fn trim_bids(fills: &mut [WorkFill], mut tons: f32) {
    trim_side(fills, &mut tons, false, true);
    trim_side(fills, &mut tons, false, false);
}

fn trim_side(fills: &mut [WorkFill], tons: &mut f32, asks: bool, marginal: bool) {
    for fill in fills.iter_mut().rev() {
        if *tons <= TON_EPS {
            break;
        }
        let is_ask = fill.tons_signed < 0.0;
        if is_ask == asks && fill.marginal == marginal {
            let reduction = fill.tons_signed.abs().min(*tons);
            if fill.tons_signed < 0.0 {
                fill.tons_signed += reduction;
            } else {
                fill.tons_signed -= reduction;
            }
            *tons -= reduction;
        }
    }
}

fn aggregate_fills(fills: Vec<WorkFill>, price: f32) -> Vec<ClearFill> {
    let mut by_ship: BTreeMap<u32, (f32, f32)> = BTreeMap::new();
    for fill in fills {
        let entry = by_ship.entry(fill.ship_id).or_insert((0.0, 0.0));
        entry.0 += fill.tons_signed;
        entry.1 += fill.tons_signed * price;
    }
    by_ship
        .into_iter()
        .filter_map(|(ship_id, (tons_signed, pesos_signed))| {
            (tons_signed.abs() > TON_EPS).then_some(ClearFill {
                ship_id,
                tons_signed,
                pesos_signed,
            })
        })
        .collect()
}

fn balance_delta(fills: &[WorkFill]) -> f32 {
    -fills.iter().map(|f| f.tons_signed).sum::<f32>()
}

fn rounded_balance(balance: f32, bound: i32) -> i32 {
    (balance.round() as i32).clamp(-bound, bound)
}

fn price_at_balance(base_price: f32, balance: i32, bound: i32) -> f32 {
    base_price * price_multiplier(balance, bound)
}

fn continuous_balance_for_price(base_price: f32, price: f32, bound: i32) -> f32 {
    if base_price <= 0.0 {
        return 0.0;
    }
    invert_multiplier(price / base_price) * bound as f32
}

fn approx_price(a: f32, b: f32) -> bool {
    (a - b).abs() <= PRICE_EPS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, eps: f32) {
        assert!((a - b).abs() <= eps, "left={a}, right={b}, eps={eps}");
    }

    fn fill(result: &ClearResult, ship_id: u32) -> Option<&ClearFill> {
        result.fills.iter().find(|f| f.ship_id == ship_id)
    }

    #[test]
    fn empty_orders_return_current_curve_price() {
        let result = clear(100.0, 25, 100, &[], &[]);
        assert!(result.fills.is_empty());
        assert_eq!(result.new_balance, 25);
        approx(
            result.clearing_price_pesos_per_ton,
            100.0 * price_multiplier(25, 100),
            1e-5,
        );
    }

    #[test]
    fn one_bid_with_headroom_fills_and_moves_balance_down() {
        let result = clear(
            100.0,
            0,
            100,
            &[ClearBid {
                ship_id: 7,
                tons: 10.0,
                max_price_pesos_per_ton: 200.0,
            }],
            &[],
        );
        assert_eq!(result.new_balance, -10);
        approx(fill(&result, 7).unwrap().tons_signed, 10.0, 1e-3);
        assert!(result.clearing_price_pesos_per_ton > 100.0);
        approx(
            result.clearing_price_pesos_per_ton,
            100.0 * price_multiplier(-10, 100),
            0.01,
        );
    }

    #[test]
    fn one_ask_with_headroom_fills_and_moves_balance_up() {
        let result = clear(
            100.0,
            0,
            100,
            &[],
            &[ClearAsk {
                ship_id: 8,
                tons: 10.0,
                min_price_pesos_per_ton: 40.0,
            }],
        );
        assert_eq!(result.new_balance, 10);
        approx(fill(&result, 8).unwrap().tons_signed, -10.0, 1e-3);
        assert!(result.clearing_price_pesos_per_ton < 100.0);
        approx(
            result.clearing_price_pesos_per_ton,
            100.0 * price_multiplier(10, 100),
            0.01,
        );
    }

    #[test]
    fn crossing_bid_and_ask_clear_near_base_at_neutral_balance() {
        let result = clear(
            100.0,
            0,
            100,
            &[ClearBid {
                ship_id: 1,
                tons: 10.0,
                max_price_pesos_per_ton: 120.0,
            }],
            &[ClearAsk {
                ship_id: 2,
                tons: 10.0,
                min_price_pesos_per_ton: 80.0,
            }],
        );
        assert_eq!(result.new_balance, 0);
        approx(result.clearing_price_pesos_per_ton, 100.0, 0.01);
        approx(fill(&result, 1).unwrap().tons_signed, 10.0, 1e-3);
        approx(fill(&result, 2).unwrap().tons_signed, -10.0, 1e-3);
    }

    #[test]
    fn non_crossing_bid_and_ask_do_not_trade() {
        let result = clear(
            100.0,
            0,
            100,
            &[ClearBid {
                ship_id: 1,
                tons: 10.0,
                max_price_pesos_per_ton: 90.0,
            }],
            &[ClearAsk {
                ship_id: 2,
                tons: 10.0,
                min_price_pesos_per_ton: 110.0,
            }],
        );
        assert!(result.fills.is_empty());
        assert_eq!(result.new_balance, 0);
        approx(result.clearing_price_pesos_per_ton, 100.0, 0.01);
    }

    #[test]
    fn bids_on_deep_glut_clear_at_low_price_and_drain_inventory() {
        let bids: Vec<_> = (0..5)
            .map(|i| ClearBid {
                ship_id: i,
                tons: 10.0,
                max_price_pesos_per_ton: 100.0,
            })
            .collect();
        let result = clear(100.0, 100, 100, &bids, &[]);
        assert!(result.clearing_price_pesos_per_ton < 100.0);
        assert!(result.new_balance < 100);
        approx(result.fills.iter().map(|f| f.tons_signed).sum(), 50.0, 1e-3);
    }

    #[test]
    fn asks_on_deep_shortage_clear_at_high_price_and_refill_inventory() {
        let asks: Vec<_> = (0..5)
            .map(|i| ClearAsk {
                ship_id: i,
                tons: 10.0,
                min_price_pesos_per_ton: 100.0,
            })
            .collect();
        let result = clear(100.0, -100, 100, &[], &asks);
        assert!(result.clearing_price_pesos_per_ton > 100.0);
        assert!(result.new_balance > -100);
        approx(
            result.fills.iter().map(|f| f.tons_signed).sum(),
            -50.0,
            1e-3,
        );
    }

    #[test]
    fn shuffled_inputs_produce_identical_sorted_fills() {
        let bids_a = vec![
            ClearBid {
                ship_id: 3,
                tons: 4.0,
                max_price_pesos_per_ton: 150.0,
            },
            ClearBid {
                ship_id: 1,
                tons: 6.0,
                max_price_pesos_per_ton: 150.0,
            },
            ClearBid {
                ship_id: 2,
                tons: 8.0,
                max_price_pesos_per_ton: 150.0,
            },
        ];
        let asks_a = vec![
            ClearAsk {
                ship_id: 12,
                tons: 8.0,
                min_price_pesos_per_ton: 50.0,
            },
            ClearAsk {
                ship_id: 10,
                tons: 4.0,
                min_price_pesos_per_ton: 50.0,
            },
            ClearAsk {
                ship_id: 11,
                tons: 6.0,
                min_price_pesos_per_ton: 50.0,
            },
        ];
        let bids_b = vec![bids_a[2], bids_a[0], bids_a[1]];
        let asks_b = vec![asks_a[1], asks_a[2], asks_a[0]];
        let a = clear(100.0, 0, 100, &bids_a, &asks_a);
        let b = clear(100.0, 0, 100, &bids_b, &asks_b);
        assert_eq!(a.new_balance, b.new_balance);
        approx(
            a.clearing_price_pesos_per_ton,
            b.clearing_price_pesos_per_ton,
            1e-6,
        );
        assert_eq!(a.fills, b.fills);
    }

    #[test]
    fn thousand_ship_auction_uses_bounded_bisection_and_returns_sane_result() {
        assert_eq!(MAX_BISECTION_ITERS, 32);
        let bids: Vec<_> = (0..500)
            .map(|i| ClearBid {
                ship_id: i,
                tons: 1.0,
                max_price_pesos_per_ton: 140.0,
            })
            .collect();
        let asks: Vec<_> = (500..1000)
            .map(|i| ClearAsk {
                ship_id: i,
                tons: 1.0,
                min_price_pesos_per_ton: 70.0,
            })
            .collect();
        let result = clear(100.0, 0, 1000, &bids, &asks);
        assert!(result.clearing_price_pesos_per_ton.is_finite());
        assert!((-1000..=1000).contains(&result.new_balance));
    }

    #[test]
    fn balance_clamping_trims_asks_at_upper_bound() {
        let asks = vec![
            ClearAsk {
                ship_id: 1,
                tons: 10.0,
                min_price_pesos_per_ton: 1.0,
            },
            ClearAsk {
                ship_id: 2,
                tons: 10.0,
                min_price_pesos_per_ton: 1.0,
            },
        ];
        let result = clear(100.0, 95, 100, &[], &asks);
        assert_eq!(result.new_balance, 100);
        approx(
            -result.fills.iter().map(|f| f.tons_signed).sum::<f32>(),
            5.0,
            1e-3,
        );
    }

    #[test]
    fn marginal_bid_at_clearing_price_gets_partial_fill() {
        let full = ClearBid {
            ship_id: 1,
            tons: 10.0,
            max_price_pesos_per_ton: 500.0,
        };
        let marginal_price = 100.0 * price_multiplier(-10, 100);
        let marginal = ClearBid {
            ship_id: 2,
            tons: 5.0,
            max_price_pesos_per_ton: marginal_price,
        };
        let result = clear(100.0, 0, 100, &[marginal, full], &[]);
        approx(fill(&result, 1).unwrap().tons_signed, 10.0, 1e-3);
        let marginal_fill = fill(&result, 2).map(|f| f.tons_signed).unwrap_or(0.0);
        assert!(
            (0.0..5.0).contains(&marginal_fill),
            "marginal fill was {marginal_fill}"
        );
    }

    #[test]
    fn huge_contention_over_twice_bound_returns_sane_clamped_result() {
        let bids = vec![ClearBid {
            ship_id: 1,
            tons: 1_000.0,
            max_price_pesos_per_ton: 10_000.0,
        }];
        let asks = vec![ClearAsk {
            ship_id: 2,
            tons: 1_000.0,
            min_price_pesos_per_ton: 0.01,
        }];
        let result = clear(100.0, 0, 100, &bids, &asks);
        assert!(result.clearing_price_pesos_per_ton.is_finite());
        assert!((-100..=100).contains(&result.new_balance));
        assert!(result
            .fills
            .iter()
            .all(|f| f.tons_signed.is_finite() && f.pesos_signed.is_finite()));
    }

    #[test]
    fn one_sided_trade_at_bound_that_would_push_farther_returns_no_fills() {
        let result = clear(
            100.0,
            -100,
            100,
            &[ClearBid {
                ship_id: 1,
                tons: 10.0,
                max_price_pesos_per_ton: 1_000.0,
            }],
            &[],
        );
        assert!(result.fills.is_empty());
        assert_eq!(result.new_balance, -100);
        approx(
            result.clearing_price_pesos_per_ton,
            100.0 * price_multiplier(-100, 100),
            1e-5,
        );
    }

    #[test]
    fn malformed_base_price_returns_no_trade() {
        let bid = ClearBid {
            ship_id: 1,
            tons: 5.0,
            max_price_pesos_per_ton: 100.0,
        };
        for bad in [0.0_f32, -10.0, f32::NAN, f32::INFINITY] {
            let r = clear(bad, 0, 100, &[bid], &[]);
            assert!(r.fills.is_empty(), "should not trade with base={bad}");
            assert_eq!(r.new_balance, 0);
        }
    }
}
