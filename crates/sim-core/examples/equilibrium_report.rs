//! Solve the Kantorovich (transportation-LP) equilibrium for the full
//! port set under two transport-cost models, and print a port × good
//! table comparing the equilibrium prices to what the simulation
//! produces on day 0.
//!
//! Use this to sanity-check whether the simulation's emergent prices
//! are in the right ballpark — if a port-good cell shows a sim price
//! 5× the equilibrium delivered price, something's off.
//!
//! Usage: `cargo run --release --example equilibrium_report`

use sim_core::equilibrium::{self, EquilibriumScenario, FreightCostModel, PortSpec};
use sim_core::goods::GoodsRegistry;
use sim_core::market::{archetype_for, seed_balance_from_equilibrium, PortMarket};
use sim_core::port::all_ports;
use sim_core::ship::ShipStats;
use sim_core::shiptype::ShipTypeRegistry;

fn main() {
    let ship_types = ShipTypeRegistry::starter();
    let ports = all_ports(&ship_types);
    let goods = GoodsRegistry::starter();

    // Build per-port specs from the same archetype recipes the
    // simulation uses.
    let port_specs: Vec<PortSpec> = ports
        .iter()
        .map(|p| {
            let recipe = archetype_for(&p.name).recipe();
            PortSpec::from_world(p, recipe)
        })
        .collect();

    // Day-0 simulation prices. Start from the same recipe heuristic as
    // World::load, then apply the linear LP shadow-price seed below.
    let mut markets: Vec<PortMarket> = ports
        .iter()
        .map(|p| PortMarket::with_recipe(&goods, archetype_for(&p.name).recipe()))
        .collect();

    // ── Cost model A: linear $0.05 / ton-NM. Tuneable; this is
    //    roughly what historical ocean freight rates worked out to
    //    when expressed per-ton-NM (a very rough order-of-magnitude
    //    fit, not a calibrated number).
    let cost_simple = FreightCostModel::Linear {
        pesos_per_ton_nm: 0.05,
    };

    // ── Cost model B: voyage-based for a sloop. Day-rate is wages +
    //    maintenance + insurance for a 25-man crew, very loosely
    //    benchmarked against historical merchant accounts.
    let cost_realistic = FreightCostModel::ShipBased {
        stats: ShipStats::sloop(),
        day_rate_pesos: 8.0,
        provisions_price_per_ton: 18.0, // matches PROVISIONS base price
    };

    println!("Solving equilibrium under linear freight model…");
    let sol_simple = equilibrium::solve(&EquilibriumScenario {
        ports: port_specs.clone(),
        goods: &goods,
        freight: cost_simple.clone(),
    });
    println!("  total surplus: {:+.0} pesos/month", sol_simple.objective);
    println!("  active flows:  {}", sol_simple.flows.len());
    let non_zero_linear = sol_simple.supply_prices.len() + sol_simple.demand_prices.len();
    println!("  non-zero shadow prices: {}", non_zero_linear);
    for (port_idx, market) in markets.iter_mut().enumerate() {
        seed_balance_from_equilibrium(market, port_idx, &sol_simple, &goods);
    }

    println!();
    println!("Solving equilibrium under voyage-cost (sloop) model…");
    let sol_real = equilibrium::solve(&EquilibriumScenario {
        ports: port_specs.clone(),
        goods: &goods,
        freight: cost_realistic.clone(),
    });
    println!("  total surplus: {:+.0} pesos/month", sol_real.objective);
    println!("  active flows:  {}", sol_real.flows.len());
    let non_zero_voyage = sol_real.supply_prices.len() + sol_real.demand_prices.len();
    println!("  non-zero shadow prices: {}", non_zero_voyage);

    // ── Per (port, good) report. For each cell: equilibrium price
    //    under each model, simulation day-0 price, and base price for
    //    reference.
    println!();
    println!(
        "{:<24} {:<18} {:>8} {:>10} {:>10} {:>10}",
        "port", "good", "base", "p_eq_lin", "p_eq_voy", "p_sim_d0"
    );
    println!("{}", "-".repeat(86));

    for (port_idx, port) in ports.iter().enumerate() {
        let spec = &port_specs[port_idx];
        let mentioned_goods: Vec<sim_core::goods::GoodId> = spec
            .recipe
            .monthly_outputs
            .iter()
            .chain(spec.recipe.monthly_inputs.iter())
            .map(|(g, _)| *g)
            .collect();
        // de-dupe
        let mut seen = std::collections::HashSet::new();
        for good in mentioned_goods {
            if !seen.insert(good) {
                continue;
            }
            let g = goods.get(good);
            let p_lin = sol_simple.price_at(port_idx, good);
            let p_voy = sol_real.price_at(port_idx, good);
            let p_sim = markets[port_idx].price_at(good, &goods);
            let lin_str = p_lin
                .map(|p| format!("{:.1}", p))
                .unwrap_or_else(|| "  —  ".to_string());
            let voy_str = p_voy
                .map(|p| format!("{:.1}", p))
                .unwrap_or_else(|| "  —  ".to_string());
            println!(
                "{:<24} {:<18} {:>8.1} {:>10} {:>10} {:>10.1}",
                port.name, g.name, g.base_price_pesos, lin_str, voy_str, p_sim
            );
        }
    }

    // ── Summary divergence. For port-good cells where both models
    //    produced an equilibrium price, compute mean absolute
    //    percentage deviation between sim_day0 and the realistic
    //    equilibrium. Big number = sim is far from equilibrium at
    //    startup; small number = simulation prices already encode
    //    the right shape.
    let mut diffs = Vec::new();
    for (port_idx, _port) in ports.iter().enumerate() {
        let spec = &port_specs[port_idx];
        let mut seen = std::collections::HashSet::new();
        for (good, _) in spec
            .recipe
            .monthly_outputs
            .iter()
            .chain(spec.recipe.monthly_inputs.iter())
        {
            if !seen.insert(*good) {
                continue;
            }
            if let Some(p_eq) = sol_real.price_at(port_idx, *good) {
                let p_sim = markets[port_idx].price_at(*good, &goods);
                if p_eq > 1.0 {
                    diffs.push(((p_sim - p_eq) / p_eq).abs());
                }
            }
        }
    }
    if !diffs.is_empty() {
        let mean = diffs.iter().sum::<f32>() / diffs.len() as f32;
        let max = diffs.iter().cloned().fold(0.0_f32, f32::max);
        println!();
        println!("Sim day-0 vs voyage-cost equilibrium:");
        println!("  cells compared: {}", diffs.len());
        println!("  mean abs % deviation: {:.1}%", mean * 100.0);
        println!("  max  abs % deviation: {:.1}%", max * 100.0);
    }
}
