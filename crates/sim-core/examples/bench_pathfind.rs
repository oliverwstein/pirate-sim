//! Plan a route between every ordered pair of ports and print the
//! per-route compute time (wall-clock, route planning only — no simulated
//! travel). Reports a summary at the end.
//!
//! Usage: `cargo run --release --example bench_pathfind`

use sim_core::pathfind::{find_path_to_harbor, PathfindContext};
use sim_core::ship::ShipStats;
use sim_core::world::World;
use std::path::Path;
use std::time::Instant;

fn main() {
    let world = World::load(Path::new("data/"));
    let stats = ShipStats::sloop();
    let pf = PathfindContext::new(&world.map.land, &world.weather.wind, &stats, 0);

    let n = world.ports.len();
    println!("Benchmarking {} ports = {} ordered pairs ({} directed routes)",
        n, n * n, n * (n - 1));

    let mut total_us: u128 = 0;
    let mut max_us: u128 = 0;
    let mut max_route = String::new();
    let mut ok = 0usize;
    let mut fail = 0usize;
    let mut fail_us: u128 = 0;
    let mut max_fail_us: u128 = 0;
    let mut max_fail_route = String::new();

    println!();
    println!("{:>8}  {:<28} -> {:<28} {:>5}  status",
        "time(ms)", "from", "to", "wpts");
    println!("{}", "-".repeat(90));

    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            let from = &world.ports[i];
            let to = &world.ports[j];
            let harbor = match world.harbors.for_port(j) {
                Some(h) => h,
                None => continue,
            };

            let t0 = Instant::now();
            let result = find_path_to_harbor(&pf, from.position, harbor);
            let dt = t0.elapsed();
            let us = dt.as_micros();

            match &result {
                Some(path) => {
                    ok += 1;
                    total_us += us;
                    if us > max_us {
                        max_us = us;
                        max_route = format!("{} -> {}", from.name, to.name);
                    }
                    println!("{:>8.2}  {:<28} -> {:<28} {:>5}  ok",
                        us as f64 / 1000.0, from.name, to.name, path.len());
                }
                None => {
                    fail += 1;
                    fail_us += us;
                    if us > max_fail_us {
                        max_fail_us = us;
                        max_fail_route = format!("{} -> {}", from.name, to.name);
                    }
                    println!("{:>8.2}  {:<28} -> {:<28} {:>5}  NO PATH",
                        us as f64 / 1000.0, from.name, to.name, 0);
                }
            }
        }
    }

    let total = ok + fail;
    println!();
    println!("=== Summary ===");
    println!("Routes attempted: {} ({} ok, {} no-path)", total, ok, fail);
    if ok > 0 {
        println!("Successful planning total: {:.2} ms  avg: {:.2} ms  max: {:.2} ms",
            total_us as f64 / 1000.0,
            (total_us as f64 / ok as f64) / 1000.0,
            max_us as f64 / 1000.0);
        println!("  slowest success: {}", max_route);
    }
    if fail > 0 {
        println!("Failed planning total:     {:.2} ms  avg: {:.2} ms  max: {:.2} ms",
            fail_us as f64 / 1000.0,
            (fail_us as f64 / fail as f64) / 1000.0,
            max_fail_us as f64 / 1000.0);
        println!("  slowest failure: {}", max_fail_route);
    }
    let grand_total_ms = (total_us + fail_us) as f64 / 1000.0;
    println!("Grand total wall time (planning only): {:.2} ms", grand_total_ms);
}
