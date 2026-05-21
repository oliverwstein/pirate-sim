//! Run a 60-day calibration demo: spawn a small trader fleet, tick the
//! simulation hour-by-hour, and report each ship's profit-and-loss
//! together with a sample of port stockpiles and prices.
//!
//! Used during Phase 2 step 9 to tune prosperity, monthly outputs, and
//! pricing constants. Look for: most ships in the black, 1–2 in the
//! red (some volatility), prices that oscillate but don't explode.
//!
//! Usage: `cargo run --release --example bench_trade`

use sim_core::ai::ShipAI;
use sim_core::equilibrium::{
    self, EquilibriumScenario, FreightCostModel, PortSpec,
};
use sim_core::goods::{ids, GoodId};
use sim_core::market::archetype_for;
use sim_core::ship::{Ship, ShipState, ShipStats};
use sim_core::world::World;
use std::path::Path;

const SIM_DAYS: u32 = 60;
const SIM_HOURS: u32 = SIM_DAYS * 24;

fn main() {
    let mut world = World::load(Path::new("data/"));

    // Starter fleet: 5 ships seeded across the map.
    let starts: &[(&str, u64)] = &[
        ("Bridgetown", 7),
        ("Port Royal", 13),
        ("Boston", 21),
        ("Charleston", 33),
        ("Cartagena", 41),
        ("Havana", 53),
        ("Fort-Royal", 67),
        ("London", 79),
        ("Amsterdam", 89),
        ("Nantes", 97),
    ];
    let mut origin_names = Vec::new();
    for (name, seed) in starts {
        let Some(idx) = world.ports.iter().position(|p| p.name == *name) else {
            println!("(skip) port {} not found", name);
            continue;
        };
        let port_pos = world.ports[idx].position;
        let ship = Ship::new(port_pos, ShipState::Docked);
        let mut ai = ShipAI::with_seed(*seed);
        ai.nav.docked_at_port = Some(idx);
        origin_names.push(*name);
        world.add_ship(ship, ai);
    }

    let n_ships = world.ships.len();
    println!("Phase 2 calibration run: {} days, {} ships", SIM_DAYS, n_ships);
    println!();

    // Sample a few ports each week so we can see prices oscillate.
    let sample_port_names: Vec<&str> = vec!["Bridgetown", "Boston", "Cartagena", "Havana"];
    let sample_idxs: Vec<usize> = sample_port_names
        .iter()
        .filter_map(|n| world.ports.iter().position(|p| p.name == *n))
        .collect();

    println!(
        "{:>5}  {:<28}  {:>10}  {:>10}",
        "day", "port", "sugar buy", "sugar sell"
    );
    for idx in &sample_idxs {
        let m = &world.markets[*idx];
        println!(
            "{:>5}  {:<28}  {:>10.2}  {:>10.2}",
            0,
            world.ports[*idx].name,
            m.buy_price(ids::SUGAR, &world.goods),
            m.sell_price(ids::SUGAR, &world.goods),
        );
    }

    // Simulate.
    for h in 1..=SIM_HOURS {
        world.tick();
        // Track origin names for any newly-built ships so the report
        // can label them. Their starting silver lives on the Ship
        // itself (`ship.starting_silver`), so there's no race here.
        while world.ships.len() > origin_names.len() {
            let i = origin_names.len();
            let owner_name = world.ships[i]
                .owner_port
                .and_then(|idx| world.ports.get(idx).map(|p| p.name))
                .unwrap_or("?");
            origin_names.push(owner_name);
        }
        if h % (24 * 7) == 0 {
            let day = h / 24;
            for idx in &sample_idxs {
                let m = &world.markets[*idx];
                println!(
                    "{:>5}  {:<28}  {:>10.2}  {:>10.2}",
                    day,
                    world.ports[*idx].name,
                    m.buy_price(ids::SUGAR, &world.goods),
                    m.sell_price(ids::SUGAR, &world.goods),
                );
            }
        }
    }

    println!();
    println!("Per-ship P/L after {} days:", SIM_DAYS);
    println!(
        "{:<3}  {:<14}  {:<10}  {:>10}  {:>10}  {:>10}  {:<10}  {:<30}",
        "#", "from", "type", "silver_in", "silver_out", "P/L", "state", "cargo"
    );
    let mut total_pl = 0.0f32;
    let mut bankrupt = 0;
    for (i, ship) in world.ships.iter().enumerate() {
        let pl = ship.silver - ship.starting_silver;
        total_pl += pl;
        if ship.silver < 50.0 {
            bankrupt += 1;
        }
        let state = match ship.state {
            ShipState::Sailing => "sailing",
            ShipState::Docked => "docked",
            ShipState::Anchored => "anchored",
        };
        let cargo: Vec<String> = ship.cargo.iter()
            .filter(|(_, t)| *t > 0.01)
            .map(|(id, t)| format!("{}:{:.1}", world.goods.get(id).name, t))
            .collect();
        let built_tag = if i >= n_ships { " (built)" } else { "" };
        let type_name = world.ship_types.get(ship.ship_type).name;
        println!(
            "{:<3}  {:<14}  {:<10}  {:>10.0}  {:>10.0}  {:>+10.0}  {:<10}  {:<30}{}",
            i, origin_names[i], type_name, ship.starting_silver, ship.silver, pl, state, cargo.join(","), built_tag
        );
    }
    println!();
    println!("Fleet total P/L: {:+.0} pesos", total_pl);
    println!("Bankrupt ships:  {}/{}", bankrupt, world.ships.len());
    println!("Ships built by shipyards: {}  (last_month_avg_profit = {:+.0})",
        world.ships_built, world.last_month_avg_profit);

    // Builds-by-type summary: only count ships beyond the starter
    // fleet (those are the shipyard's actual output, not what we
    // seeded for the demo).
    if world.ships.len() > n_ships {
        let mut counts: std::collections::BTreeMap<&'static str, u32> =
            std::collections::BTreeMap::new();
        for ship in world.ships.iter().skip(n_ships) {
            let name = world.ship_types.get(ship.ship_type).name;
            *counts.entry(name).or_insert(0) += 1;
        }
        let parts: Vec<String> = counts.iter()
            .map(|(n, c)| format!("{} {}", c, n))
            .collect();
        println!("  by type: {}", parts.join(", "));
    }

    // ── Equilibrium divergence diagnostic. Solve the Kantorovich LP
    //    against the same world (linear and voyage-cost models). For
    //    every (port, good) cell where the equilibrium has an opinion,
    //    average the day-0 vs end-of-run simulation prices and compare
    //    to the equilibrium "delivered" price. Big numbers = the
    //    simulation is structurally far from where a frictionless
    //    trade-allocator would push it.
    let port_specs: Vec<PortSpec> = world.ports
        .iter()
        .map(|p| PortSpec::from_world(p, archetype_for(p.name).recipe()))
        .collect();
    let voyage_freight = FreightCostModel::ShipBased {
        stats: ShipStats::sloop(),
        day_rate_pesos: 8.0,
        provisions_price_per_ton: 18.0,
    };
    let eq = equilibrium::solve(&EquilibriumScenario {
        ports: port_specs.clone(),
        goods: &world.goods,
        freight: voyage_freight,
    });

    let mut diffs: Vec<(String, &str, f32, f32, f32)> = Vec::new();
    for (port_idx, port) in world.ports.iter().enumerate() {
        let spec = &port_specs[port_idx];
        let mut seen = std::collections::HashSet::new();
        for (good, _) in spec.recipe.monthly_outputs.iter()
            .chain(spec.recipe.monthly_inputs.iter())
        {
            let good: GoodId = *good;
            if !seen.insert(good) {
                continue;
            }
            if let Some(p_eq) = eq.price_at(port_idx, good) {
                let p_sim = world.markets[port_idx].buy_price(good, &world.goods);
                let pct = if p_eq > 1.0 {
                    ((p_sim - p_eq) / p_eq).abs()
                } else {
                    0.0
                };
                diffs.push((
                    port.name.to_string(),
                    world.goods.get(good).name,
                    p_sim,
                    p_eq,
                    pct,
                ));
            }
        }
    }

    println!();
    println!("Equilibrium vs end-of-run simulation prices (voyage-cost LP):");
    println!("  cells compared: {}", diffs.len());
    if !diffs.is_empty() {
        let mean = diffs.iter().map(|d| d.4).sum::<f32>() / diffs.len() as f32;
        let max = diffs.iter().map(|d| d.4).fold(0.0_f32, f32::max);
        println!("  mean abs % deviation: {:.0}%", mean * 100.0);
        println!("  max  abs % deviation: {:.0}%", max * 100.0);

        // Show the 10 worst-divergence cells — these are the leading
        // candidates for "something's mispriced" investigation.
        let mut sorted = diffs.clone();
        sorted.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap());
        println!();
        println!("  Top 10 mispriced cells (|sim − eq| / eq):");
        println!(
            "    {:<24} {:<18} {:>10} {:>10} {:>8}",
            "port", "good", "sim", "eq", "Δ%"
        );
        for d in sorted.iter().take(10) {
            println!(
                "    {:<24} {:<18} {:>10.1} {:>10.1} {:>7.0}%",
                d.0, d.1, d.2, d.3, d.4 * 100.0
            );
        }
    }

    // A coarse "calibration health" verdict. In absence of piracy and
    // shipwreck (Phase 3 work), every ship that follows the trader AI
    // should reliably prosper — bankruptcies signal an underpaying
    // route, and a flat zero-profit fleet would mean the AI is just
    // sitting still. Wide variance between ships is fine and expected
    // (geography matters).
    println!();
    println!("Calibration verdict:");
    let losers = world.ships.iter()
        .filter(|s| s.silver < s.starting_silver)
        .count();
    if bankrupt > 0 {
        println!(
            "  ⚠ {} bankrupt ship(s) — Phase 2 should not bankrupt anyone before piracy/wreck risk is added",
            bankrupt
        );
    } else if losers > 0 {
        println!(
            "  ⚠ {} ship(s) ended in the red — successful voyages should reliably profit",
            losers
        );
    } else if total_pl < world.ships.iter().map(|s| s.starting_silver).sum::<f32>() * 0.5 {
        println!(
            "  ⚠ fleet barely profitable — trade margins or world prices may be too thin"
        );
    } else {
        println!("  ✓ every ship in the black with healthy variance");
    }
}
