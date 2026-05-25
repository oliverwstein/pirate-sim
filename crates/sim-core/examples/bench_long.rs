//! Long-horizon diagnostic bench. Runs the headless sim for N years
//! and snapshots system state at a fixed cadence so we can observe
//! slow-burn degenerations that don't surface in the 60-day
//! `bench_trade` smoke (e.g. ships piling up at one port, mass
//! attrition, treasury concentration, debt cap saturation).
//!
//! Usage:
//!   cargo run --release --example bench_long             # 10-year default, 90-day cadence
//!   cargo run --release --example bench_long -- 5        # 5 years
//!   cargo run --release --example bench_long -- 5 30     # 5 years, monthly snapshots

use sim_core::goods::{ids, GoodId};
use sim_core::ship::{ShipState, MAX_SHIP_DEBT};
use sim_core::world::World;
use std::path::Path;

const DEFAULT_SIM_YEARS: u32 = 10;
const DEFAULT_SNAPSHOT_DAYS: u32 = 90;

/// Watch-list of ports we always want a column for, in addition to
/// whatever currently has the most docked ships. Chosen to cover the
/// reported pathologies (Amsterdam jam, Elmina sink) plus regional
/// reference points.
const WATCH_PORTS: &[&str] = &[
    "Amsterdam",
    "London",
    "Nantes",
    "Cadiz",
    "Elmina",
    "Bridgetown",
    "Havana",
    "Port Royal",
];

const WATCH_GOODS: &[(GoodId, &str)] = &[
    (ids::SUGAR, "sugar"),
    (ids::RUM, "rum"),
    (ids::MANUFACTURES, "manu"),
    (ids::PROVISIONS, "prov"),
    (ids::ENSLAVED_PERSONS, "slaves"),
];

/// How many ships are loitering "near" Amsterdam (not docked, just
/// drifting around the harbor). Reproduces the visualizer's
/// "traffic-jam outside Amsterdam" symptom in headless form.
const NEAR_PORT_RADIUS_NM: f32 = 50.0;
const JAM_PROBES: &[&str] = &["Amsterdam", "Nantes", "Elmina"];

#[derive(Default, Clone)]
struct ShipDistribution {
    n_alive: usize,
    n_sailing: usize,
    n_docked: usize,
    n_anchored: usize,
    n_hiring: usize,
    silver_p10: f32,
    silver_p50: f32,
    silver_p90: f32,
    debt_p50: f32,
    debt_p90: f32,
    n_at_debt_cap: usize,
    n_broke: usize,          // silver < 50 pesos
    n_low_provisions: usize, // < 30 days remaining
}

fn percentile(sorted: &[f32], q: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f32 - 1.0) * q).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn summarize_ships(world: &World) -> ShipDistribution {
    let mut d = ShipDistribution::default();
    let mut silvers: Vec<f32> = Vec::with_capacity(world.ships.len());
    let mut debts: Vec<f32> = Vec::with_capacity(world.ships.len());
    let debt_cap = MAX_SHIP_DEBT.as_pesos_f32();
    for (_, s) in &world.ships {
        d.n_alive += 1;
        match s.state {
            ShipState::Sailing => d.n_sailing += 1,
            ShipState::Docked => d.n_docked += 1,
            ShipState::Anchored => d.n_anchored += 1,
            ShipState::Hiring => d.n_hiring += 1,
            ShipState::Sunk => {}
        }
        let silver = s.silver.as_pesos_f32();
        let debt = s.debt.as_pesos_f32();
        silvers.push(silver);
        debts.push(debt);
        if silver < 50.0 {
            d.n_broke += 1;
        }
        if debt >= debt_cap - 1.0 {
            d.n_at_debt_cap += 1;
        }
        let stats = &world.ship_types.get(s.ship_type).stats;
        let daily = stats.daily_provision_consumption().max(1e-6);
        if (s.provisions / daily) < 30.0 {
            d.n_low_provisions += 1;
        }
    }
    silvers.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    debts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    d.silver_p10 = percentile(&silvers, 0.10);
    d.silver_p50 = percentile(&silvers, 0.50);
    d.silver_p90 = percentile(&silvers, 0.90);
    d.debt_p50 = percentile(&debts, 0.50);
    d.debt_p90 = percentile(&debts, 0.90);
    d
}

/// Returns docked-ship count per port (parallel to `world.ports`).
fn docked_counts(world: &World) -> Vec<u32> {
    let mut counts = vec![0u32; world.ports.len()];
    for (_, s) in &world.ships {
        if let Some(idx) = s.nav.docked_at_port {
            if idx < counts.len() {
                counts[idx] += 1;
            }
        }
    }
    counts
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let sim_years: u32 = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SIM_YEARS);
    let snap_days: u32 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_SNAPSHOT_DAYS);

    let sim_hours: u32 = sim_years * 365 * 24;
    let snap_hours: u32 = snap_days * 24;

    let mut world = World::load(Path::new("data/"));
    let _seeded = world.seed_historical_fleet(0xCAFE_1680);

    println!(
        "bench_long: {} years ({} days), snapshot every {} days, fleet seed 0xCAFE_1680",
        sim_years,
        sim_years * 365,
        snap_days
    );
    println!("Initial ships: {}", world.ships.len());

    // Resolve watch-port indices once.
    let watch_idxs: Vec<(String, Option<usize>)> = WATCH_PORTS
        .iter()
        .map(|n| (n.to_string(), world.ports.iter().position(|p| p.name == *n)))
        .collect();

    let mut prev_built: u32 = world.faction_telemetry.iter().map(|t| t.ships_built).sum();
    let mut prev_lost: u32 = world.faction_telemetry.iter().map(|t| t.ships_lost).sum();

    print_snapshot_header(&watch_idxs);
    print_snapshot(&world, &watch_idxs, 0, 0, 0);

    for h in 1..=sim_hours {
        world.tick();
        if h % snap_hours == 0 {
            let cur_built: u32 = world.faction_telemetry.iter().map(|t| t.ships_built).sum();
            let cur_lost: u32 = world.faction_telemetry.iter().map(|t| t.ships_lost).sum();
            let built_delta = cur_built - prev_built;
            let lost_delta = cur_lost - prev_lost;
            print_snapshot(&world, &watch_idxs, h / 24, built_delta, lost_delta);
            prev_built = cur_built;
            prev_lost = cur_lost;
        }
    }

    // Final summary: which ports are over/under-attended, total balance,
    // and a per-faction birth/death table.
    println!();
    println!("=== Final per-faction ledger ===");
    println!(
        "{:<12} {:>10} {:>10} {:>14} {:>14}",
        "faction", "built", "lost", "crown_rev", "silver_home"
    );
    for (i, t) in world.faction_telemetry.iter().enumerate() {
        let name = match i {
            0 => "Spain",
            1 => "England",
            2 => "France",
            3 => "Netherlands",
            4 => "Free",
            _ => "?",
        };
        println!(
            "{:<12} {:>10} {:>10} {:>14.0} {:>14.0}",
            name,
            t.ships_built,
            t.ships_lost,
            t.crown_revenue.as_pesos_f32(),
            t.silver_returned_home.as_pesos_f32(),
        );
    }

    println!();
    println!("=== Final port-treasury / docked-count table (sorted by docked desc) ===");
    let mut rows: Vec<(String, u32, f32)> = world
        .ports
        .iter()
        .enumerate()
        .map(|(i, p)| {
            (
                p.name.clone(),
                docked_counts(&world)[i],
                world.markets[i].silver.as_pesos_f32(),
            )
        })
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    println!("{:<28} {:>8} {:>14}", "port", "docked", "treasury");
    for (name, docked, treasury) in rows.iter().take(15) {
        println!("{:<28} {:>8} {:>14.0}", name, docked, treasury);
    }
}

fn print_snapshot_header(watch: &[(String, Option<usize>)]) {
    print!(
        "\n{:>5} {:>4} {:>4} {:>5} {:>5}  {:>5} {:>5} {:>5} {:>5}   {:>4} {:>4} {:>4}  {:>6} {:>6}",
        "day",
        "year",
        "n",
        "+",
        "-",
        "sail",
        "dock",
        "anch",
        "hire",
        "brk",
        "cap",
        "lowP",
        "silP50",
        "silP10",
    );
    for (name, _) in watch {
        // 4-char port abbreviation + docked count + treasury
        let abbr: String = name.chars().take(4).collect();
        print!(" {:>4}d {:>5}k", abbr, abbr);
    }
    for (_, abbr) in WATCH_GOODS {
        print!(" {:>7}", abbr);
    }
    for probe in JAM_PROBES {
        let abbr: String = probe.chars().take(4).collect();
        print!(" {:>4}", format!("~{}", abbr));
    }
    println!();
}

fn print_snapshot(
    world: &World,
    watch: &[(String, Option<usize>)],
    day: u32,
    built: u32,
    lost: u32,
) {
    let d = summarize_ships(world);
    let counts = docked_counts(world);
    print!(
        "{:>5} {:>4} {:>4} {:>5} {:>5}  {:>5} {:>5} {:>5} {:>5}   {:>4} {:>4} {:>4}  {:>6.0} {:>6.0}",
        day,
        world.date.year,
        d.n_alive,
        built,
        lost,
        d.n_sailing,
        d.n_docked,
        d.n_anchored,
        d.n_hiring,
        d.n_broke,
        d.n_at_debt_cap,
        d.n_low_provisions,
        d.silver_p50,
        d.silver_p10,
    );
    for (_, idx_opt) in watch {
        if let Some(idx) = idx_opt {
            print!(
                " {:>5} {:>6.0}",
                counts[*idx],
                world.markets[*idx].silver.as_pesos_f32() / 1000.0
            );
        } else {
            print!(" {:>5} {:>6}", "?", "?");
        }
    }
    // Aggregate signed balance across all ports for watch goods.
    for (gid, _) in WATCH_GOODS {
        let total: i32 = world.markets.iter().map(|m| m.balance.get(*gid)).sum();
        print!(" {:>7}", total);
    }
    // Traffic-jam probes: number of non-docked ships within
    // NEAR_PORT_RADIUS_NM of each named port. A persistent high
    // value at a single port = pile-up at its harbor entrance.
    for probe_name in JAM_PROBES {
        let port_pos = world
            .ports
            .iter()
            .find(|p| p.name == *probe_name)
            .map(|p| p.position);
        let count: usize = match port_pos {
            Some(pos) => world
                .ships
                .iter()
                .filter(|(_, s)| s.nav.docked_at_port.is_none())
                .filter(|(_, s)| s.position.distance(pos) < NEAR_PORT_RADIUS_NM)
                .count(),
            None => 0,
        };
        print!(" {:>4}", count);
    }
    println!();
}
