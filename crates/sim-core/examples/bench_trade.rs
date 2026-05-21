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
use sim_core::goods::ids;
use sim_core::ship::{Ship, ShipState};
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
    let mut starting_silver = Vec::new();
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
        starting_silver.push(ship.silver);
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
        "{:<3}  {:<14}  {:>10}  {:>10}  {:>10}  {:<10}  {:<30}",
        "#", "from", "silver_in", "silver_out", "P/L", "state", "cargo"
    );
    let mut total_pl = 0.0f32;
    let mut bankrupt = 0;
    for (i, ship) in world.ships.iter().enumerate() {
        let pl = ship.silver - starting_silver[i];
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
        println!(
            "{:<3}  {:<14}  {:>10.0}  {:>10.0}  {:>+10.0}  {:<10}  {:<30}",
            i, origin_names[i], starting_silver[i], ship.silver, pl, state, cargo.join(",")
        );
    }
    println!();
    println!("Fleet total P/L: {:+.0} pesos", total_pl);
    println!("Bankrupt ships:  {}/{}", bankrupt, n_ships);

    // A coarse "calibration health" verdict. In absence of piracy and
    // shipwreck (Phase 3 work), every ship that follows the trader AI
    // should reliably prosper — bankruptcies signal an underpaying
    // route, and a flat zero-profit fleet would mean the AI is just
    // sitting still. Wide variance between ships is fine and expected
    // (geography matters).
    println!();
    println!("Calibration verdict:");
    let losers = world.ships.iter().enumerate()
        .filter(|(i, s)| s.silver < starting_silver[*i])
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
    } else if total_pl < starting_silver.iter().sum::<f32>() * 0.5 {
        println!(
            "  ⚠ fleet barely profitable — trade margins or world prices may be too thin"
        );
    } else {
        println!("  ✓ every ship in the black with healthy variance");
    }
}
