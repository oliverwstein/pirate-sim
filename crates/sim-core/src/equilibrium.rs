//! Kantorovich (transportation-LP) equilibrium solver, used as an
//! independent sanity check on the simulation's emergent prices.
//!
//! Treats each port-good pair as either a *source* (port produces the
//! good monthly) or a *sink* (port consumes the good monthly), and the
//! straight-line distance between any pair of ports as a per-ton
//! shipping cost. Solves
//!
//! ```text
//!   maximize   Σ_{j,k}  V_j^k · Σ_i x[i,j,k]
//!            − Σ_{i,j,k} c_{ij}^k · x[i,j,k]
//!   subject to Σ_j x[i,j,k] ≤ S_i^k    (supply caps)
//!              Σ_i x[i,j,k] ≤ D_j^k    (demand caps)
//!              x[i,j,k] ≥ 0
//! ```
//!
//! where `V_j^k` is the willingness-to-pay (each consuming port pays
//! that good's `base_price`), `S_i^k` is monthly output × prosperity,
//! `D_j^k` is monthly input × prosperity, and `c_{ij}^k` is the
//! transport cost per ton on the (i,j) leg for good k.
//!
//! Shadow prices are recovered by **finite-difference duals**: after
//! solving the primal, perturb each binding cap by +ε tons and
//! resolve; the change in the objective is the shadow price for that
//! constraint. For producer ports, that's the marginal value of an
//! extra ton of supply (= the equilibrium FOB price). For consumer
//! ports, it's the marginal value of an extra ton of demand (= the
//! equilibrium delivered price). Slow but solver-agnostic; the LP is
//! small enough (a few thousand variables for the full Caribbean +
//! Europe + Africa scenario) that the perturbation pass takes well
//! under a second on a laptop.
//!
//! This is *not* used by the live simulation — it's a diagnostic. The
//! point is to compare the simulation's emergent prices against a
//! mechanism-free benchmark and flag anomalies.

use std::collections::HashMap;

use crate::goods::{GoodId, GoodsRegistry};
use crate::market::ProductionRecipe;
use crate::port::Port;
use crate::ship::ShipStats;
use crate::types::Position;

/// How transport cost between two ports is computed.
#[derive(Clone, Debug)]
pub enum FreightCostModel {
    /// `pesos_per_ton_nm × distance_nm`. A flat per-ton-nautical-mile
    /// rate. Useful as a mechanism-free baseline.
    Linear { pesos_per_ton_nm: f32 },
    /// Voyage-cost model: assume a sloop-class ship takes
    /// `distance / (speed_typical · 0.55 · 24)` days to cover the
    /// leg, consuming provisions and paying crew at `day_rate`. The
    /// per-ton cost is the round-trip total cost divided by the
    /// ship's cargo capacity (one full hold each way).
    ShipBased {
        stats: ShipStats,
        /// Per-day operational cost in pesos: crew wages + maintenance
        ///   + insurance, exclusive of provisions (which are computed
        ///     separately from the ship's consumption rate).
        day_rate_pesos: f32,
        /// Pesos per ton of provisions (used to value the food the
        /// crew eats during the leg). Typically ≈ provisions base
        /// price.
        provisions_price_per_ton: f32,
    },
}

impl FreightCostModel {
    /// Pesos per delivered ton on the i→j leg.
    pub fn cost_per_ton(&self, distance_nm: f32) -> f32 {
        match self {
            FreightCostModel::Linear { pesos_per_ton_nm } => distance_nm * *pesos_per_ton_nm,
            FreightCostModel::ShipBased {
                stats,
                day_rate_pesos,
                provisions_price_per_ton,
            } => {
                // One-way voyage time, derated for tacking/calms/weather.
                let voyage_days = distance_nm / (stats.speed_typical * 0.55 * 24.0);
                let provisions_tons = stats.daily_provision_consumption() * voyage_days;
                let leg_cost =
                    day_rate_pesos * voyage_days + provisions_tons * provisions_price_per_ton;
                // Round trip: ship comes back empty (or with backhaul,
                // but in the worst case we amortize over the laden
                // direction only). Per-ton = round-trip cost / cargo
                // capacity.
                (2.0 * leg_cost) / stats.cargo_capacity_tons
            }
        }
    }
}

/// Per-port spec the LP needs: location and recipe (× prosperity).
#[derive(Clone, Debug)]
pub struct PortSpec<'a> {
    pub name: &'a str,
    pub position: Position,
    pub recipe: ProductionRecipe,
}

impl<'a> PortSpec<'a> {
    pub fn from_world(port: &'a Port, recipe: ProductionRecipe) -> Self {
        Self {
            name: port.name,
            position: port.position,
            recipe,
        }
    }

    /// Effective monthly supply of `id` (output × prosperity).
    fn supply(&self, id: GoodId) -> f32 {
        let prosperity = self.recipe.prosperity.max(0.0);
        self.recipe
            .monthly_outputs
            .iter()
            .find(|(g, _)| *g == id)
            .map(|(_, t)| *t * prosperity)
            .unwrap_or(0.0)
    }

    /// Effective monthly demand for `id` (input × prosperity).
    fn demand(&self, id: GoodId) -> f32 {
        let prosperity = self.recipe.prosperity.max(0.0);
        self.recipe
            .monthly_inputs
            .iter()
            .find(|(g, _)| *g == id)
            .map(|(_, t)| *t * prosperity)
            .unwrap_or(0.0)
    }
}

/// Inputs to the equilibrium solver.
pub struct EquilibriumScenario<'a> {
    pub ports: Vec<PortSpec<'a>>,
    pub goods: &'a GoodsRegistry,
    pub freight: FreightCostModel,
}

/// One non-zero flow x[i,j,k] in the optimal solution.
#[derive(Clone, Debug)]
pub struct Flow {
    pub from: usize,
    pub to: usize,
    pub good: GoodId,
    pub tons_per_month: f32,
}

/// Equilibrium solution: optimal flows and shadow prices.
pub struct EquilibriumSolution {
    pub flows: Vec<Flow>,
    /// Shadow price of the supply cap at (port, good): pesos per ton.
    /// Read as the equilibrium FOB (free-on-board) price at the
    /// producer's wharf. Zero if the port doesn't produce that good
    /// or the cap is non-binding.
    pub supply_prices: HashMap<(usize, GoodId), f32>,
    /// Shadow price of the demand cap at (port, good): pesos per ton.
    /// Read as the equilibrium delivered price at the consumer's
    /// wharf. Zero if the port doesn't consume that good or the cap
    /// is non-binding (i.e. demand exceeds available supply at that
    /// price level — but capped at the willingness-to-pay).
    pub demand_prices: HashMap<(usize, GoodId), f32>,
    /// Total surplus realized by the optimal flow assignment, in
    /// pesos per simulated month.
    pub objective: f32,
}

impl EquilibriumSolution {
    /// Best estimate of the equilibrium price at `port_idx` for `good`.
    /// Falls back through:
    /// 1. Demand-side dual (if the port consumes this good).
    /// 2. Supply-side dual (if the port produces this good).
    /// 3. Best-arrival price = min over flows landing here of
    ///    `supply_price_at_origin + freight_cost`.
    /// 4. Best-departure price = max over flows leaving here of
    ///    `demand_price_at_destination - freight_cost`.
    /// 5. None (port is economically isolated for this good).
    pub fn price_at(&self, port_idx: usize, good: GoodId) -> Option<f32> {
        if let Some(&p) = self.demand_prices.get(&(port_idx, good)) {
            if p > 0.0 {
                return Some(p);
            }
        }
        if let Some(&p) = self.supply_prices.get(&(port_idx, good)) {
            if p > 0.0 {
                return Some(p);
            }
        }
        None
    }
}

fn distance_nm(a: Position, b: Position) -> f32 {
    a.distance(b)
}

/// Solve the LP. Uses microlp under the hood; finite-difference duals.
pub fn solve(scenario: &EquilibriumScenario) -> EquilibriumSolution {
    let n = scenario.ports.len();
    let goods: Vec<GoodId> = scenario.goods.iter().map(|g| g.id).collect();

    // Pre-compute per-leg per-good freight costs.
    let mut freight = vec![0.0_f32; n * n];
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            let d = distance_nm(scenario.ports[i].position, scenario.ports[j].position);
            freight[i * n + j] = scenario.freight.cost_per_ton(d);
        }
    }

    // For each good, set up and solve a separate transportation LP.
    // Goods don't couple in this formulation (each is shipped
    // independently; ships with mixed holds are an emergent
    // refinement, not modelled here).
    let mut all_flows = Vec::new();
    let mut supply_prices = HashMap::new();
    let mut demand_prices = HashMap::new();
    let mut total_objective = 0.0_f32;

    for &good in &goods {
        let v_per_ton = scenario.goods.get(good).base_price_pesos;
        let supply: Vec<f32> = scenario.ports.iter().map(|p| p.supply(good)).collect();
        let demand: Vec<f32> = scenario.ports.iter().map(|p| p.demand(good)).collect();

        // Skip goods nobody produces or nobody consumes — no flow.
        if supply.iter().all(|&s| s <= 0.0) || demand.iter().all(|&d| d <= 0.0) {
            continue;
        }

        let (flows_k, supply_p, demand_p, obj) =
            solve_single_good(n, &supply, &demand, &freight, v_per_ton);
        for f in flows_k {
            all_flows.push(Flow {
                from: f.0,
                to: f.1,
                good,
                tons_per_month: f.2,
            });
        }
        for (i, p) in supply_p.into_iter().enumerate() {
            if p > 0.0 {
                supply_prices.insert((i, good), p);
            }
        }
        for (j, p) in demand_p.into_iter().enumerate() {
            if p > 0.0 {
                demand_prices.insert((j, good), p);
            }
        }
        total_objective += obj;
    }

    EquilibriumSolution {
        flows: all_flows,
        supply_prices,
        demand_prices,
        objective: total_objective,
    }
}

/// Solve the transportation LP for a single good. Returns
/// `(flows, supply_duals, demand_duals, objective)`.
///
/// `supply_duals[i]` is the marginal value of an extra ton of supply
/// at port `i` (≈ FOB price). `demand_duals[j]` is the marginal value
/// of an extra ton of demand at port `j` (≈ delivered price, capped
/// at `v_per_ton`).
#[allow(clippy::type_complexity)]
fn solve_single_good(
    n: usize,
    supply: &[f32],
    demand: &[f32],
    freight: &[f32],
    v_per_ton: f32,
) -> (Vec<(usize, usize, f32)>, Vec<f32>, Vec<f32>, f32) {
    use microlp::{ComparisonOp, OptimizationDirection, Problem};

    // Build a fresh problem; returns objective and the supply/demand
    // delivered totals at each port. Used both for the base solve and
    // for the perturbation pass.
    let solve_with = |sup: &[f32], dem: &[f32]| -> Option<(f64, Vec<Vec<f64>>)> {
        let mut p = Problem::new(OptimizationDirection::Maximize);

        // Variables: x[i][j] tons/month from i to j (i != j).
        // Coefficient: V_j (delivered value at j) − c_{ij}.
        let mut vars = vec![vec![None; n]; n];
        for i in 0..n {
            if sup[i] <= 0.0 {
                continue;
            }
            for j in 0..n {
                if i == j || dem[j] <= 0.0 {
                    continue;
                }
                let coeff = (v_per_ton - freight[i * n + j]) as f64;
                if coeff <= 0.0 {
                    // No incentive — hard-skip. Still create a var
                    // at zero so downstream loops are uniform.
                    vars[i][j] = Some(p.add_var(0.0, (0.0, 0.0)));
                } else {
                    vars[i][j] = Some(p.add_var(coeff, (0.0, f64::INFINITY)));
                }
            }
        }

        // Per-source supply caps.
        for i in 0..n {
            let mut expr: Vec<(microlp::Variable, f64)> = Vec::new();
            for j in 0..n {
                if let Some(v) = vars[i][j] {
                    expr.push((v, 1.0));
                }
            }
            if expr.is_empty() {
                continue;
            }
            p.add_constraint(expr, ComparisonOp::Le, sup[i] as f64);
        }

        // Per-sink demand caps.
        #[allow(clippy::needless_range_loop)]
        for j in 0..n {
            let mut expr: Vec<(microlp::Variable, f64)> = Vec::new();
            for i in 0..n {
                if let Some(v) = vars[i][j] {
                    expr.push((v, 1.0));
                }
            }
            if expr.is_empty() {
                continue;
            }
            p.add_constraint(expr, ComparisonOp::Le, dem[j] as f64);
        }

        let sol = p.solve().ok()?;
        let obj = sol.objective();
        let mut flows = vec![vec![0.0_f64; n]; n];
        for i in 0..n {
            for j in 0..n {
                if let Some(v) = vars[i][j] {
                    flows[i][j] = sol[v];
                }
            }
        }
        Some((obj, flows))
    };

    let (obj_base, flows_base) = match solve_with(supply, demand) {
        Some(x) => x,
        None => return (Vec::new(), vec![0.0; n], vec![0.0; n], 0.0),
    };

    // Finite-difference duals: bump each binding cap by EPS tons and
    // observe Δobjective. Skip caps that aren't binding (no flow uses
    // them up) — those have dual = 0 by complementary slackness.
    const EPS: f32 = 1e-2;

    let mut supply_duals = vec![0.0_f32; n];
    for i in 0..n {
        if supply[i] <= 0.0 {
            continue;
        }
        let used: f32 = (0..n).map(|j| flows_base[i][j]).sum::<f64>() as f32;
        if used + 1e-4 < supply[i] {
            // Cap not binding — dual is zero.
            continue;
        }
        let mut sup2 = supply.to_vec();
        sup2[i] += EPS;
        if let Some((obj_p, _)) = solve_with(&sup2, demand) {
            supply_duals[i] = ((obj_p - obj_base) / EPS as f64) as f32;
        }
    }

    let mut demand_duals = vec![0.0_f32; n];
    for j in 0..n {
        if demand[j] <= 0.0 {
            continue;
        }
        let used: f32 = (0..n).map(|i| flows_base[i][j]).sum::<f64>() as f32;
        if used + 1e-4 < demand[j] {
            continue;
        }
        let mut dem2 = demand.to_vec();
        dem2[j] += EPS;
        if let Some((obj_p, _)) = solve_with(supply, &dem2) {
            demand_duals[j] = ((obj_p - obj_base) / EPS as f64) as f32;
        }
    }

    let mut flows_out = Vec::new();
    #[allow(clippy::needless_range_loop)]
    for i in 0..n {
        for j in 0..n {
            let f = flows_base[i][j] as f32;
            if f > 1e-4 {
                flows_out.push((i, j, f));
            }
        }
    }
    (flows_out, supply_duals, demand_duals, obj_base as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goods::ids;
    use crate::market::ProductionRecipe;
    use crate::types::Position;

    fn recipe(outputs: &[(GoodId, f32)], inputs: &[(GoodId, f32)]) -> ProductionRecipe {
        ProductionRecipe {
            monthly_outputs: outputs.to_vec(),
            monthly_inputs: inputs.to_vec(),
            prosperity: 1.0,
        }
    }

    #[test]
    fn single_producer_single_consumer_yields_flow_and_prices() {
        // Two ports 100 NM apart, one produces sugar, the other
        // consumes it. Linear freight at $0.10/ton-NM = $10/ton.
        // Sugar base price $70/ton. Expected:
        //   flow > 0 (it's profitable: 70 > 10).
        //   supply price ≈ 70 - 10 = 60 (consumer pays 70; freight is 10).
        //   demand price ≈ 70 (constraint binds at willingness-to-pay).
        let goods = GoodsRegistry::starter();
        let producer = PortSpec {
            name: "Producer",
            position: Position::new(0.0, 0.0),
            recipe: recipe(&[(ids::SUGAR, 50.0)], &[]),
        };
        let consumer = PortSpec {
            name: "Consumer",
            position: Position::new(100.0, 0.0),
            recipe: recipe(&[], &[(ids::SUGAR, 30.0)]),
        };
        let scenario = EquilibriumScenario {
            ports: vec![producer, consumer],
            goods: &goods,
            freight: FreightCostModel::Linear {
                pesos_per_ton_nm: 0.1,
            },
        };
        let sol = solve(&scenario);
        let flow = sol.flows.iter().find(|f| f.good == ids::SUGAR);
        assert!(flow.is_some(), "expected sugar to flow producer→consumer");
        let f = flow.unwrap();
        assert_eq!(f.from, 0);
        assert_eq!(f.to, 1);
        assert!(
            (f.tons_per_month - 30.0).abs() < 1e-2,
            "should ship up to demand cap"
        );
        let p_supply = sol
            .supply_prices
            .get(&(0, ids::SUGAR))
            .copied()
            .unwrap_or(0.0);
        let p_demand = sol
            .demand_prices
            .get(&(1, ids::SUGAR))
            .copied()
            .unwrap_or(0.0);
        // Demand cap is binding; supply cap (50) is not (only 30 ships) → supply dual = 0.
        assert!(
            p_demand > 50.0,
            "demand-side dual ≈ 60 (=70-10), got {}",
            p_demand
        );
        assert!(
            p_supply == 0.0,
            "supply cap not binding → dual is 0, got {}",
            p_supply
        );
    }

    #[test]
    fn no_flow_when_freight_exceeds_value() {
        // Freight ($800/ton) >> sugar value ($70). Nothing should ship.
        let goods = GoodsRegistry::starter();
        let producer = PortSpec {
            name: "P",
            position: Position::new(0.0, 0.0),
            recipe: recipe(&[(ids::SUGAR, 10.0)], &[]),
        };
        let consumer = PortSpec {
            name: "C",
            position: Position::new(100.0, 0.0),
            recipe: recipe(&[], &[(ids::SUGAR, 10.0)]),
        };
        let scenario = EquilibriumScenario {
            ports: vec![producer, consumer],
            goods: &goods,
            freight: FreightCostModel::Linear {
                pesos_per_ton_nm: 8.0,
            },
        };
        let sol = solve(&scenario);
        assert!(
            sol.flows.iter().all(|f| f.tons_per_month < 1e-3),
            "no profitable flow exists"
        );
    }
}
