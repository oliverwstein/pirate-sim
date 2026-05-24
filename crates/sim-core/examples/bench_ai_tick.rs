//! Headless micro-benchmark for the AI phase of the hourly tick.
//!
//! Loads the historical fleet, runs N hourly ticks, and records
//! `world.last_ai_phase_ns` after each tick. Reports avg / p50 / p95
//! / max latency for the AI phase only. Designed to measure the
//! payoff of parallelizing the AI tick (Phase 6 follow-up):
//! compare runs before and after.
//!
//! Usage:
//!   cargo run --release -p sim-core --example bench_ai_tick [hours]
//! Default: 720 hours (30 in-game days). The historical fleet is
//! ~480 ships, so each tick exercises ~480 AI invocations.

use std::path::Path;

use sim_core::world::World;

const DEFAULT_HOURS: u32 = 720;
const WARMUP_HOURS: u32 = 24;

fn main() {
    let hours: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_HOURS);

    let mut world = World::load(Path::new("data/"));
    let _ = world.seed_historical_fleet(0xCAFE_1680);
    let n_ships = world.ships.len();

    println!(
        "bench_ai_tick: {} ships, {} hours ({} warmup + {} measured)",
        n_ships,
        WARMUP_HOURS + hours,
        WARMUP_HOURS,
        hours,
    );

    // Warm-up so first-tick allocations / pathfind cache priming
    // don't poison the percentiles.
    for _ in 0..WARMUP_HOURS {
        world.tick();
    }

    let mut samples: Vec<u64> = Vec::with_capacity(hours as usize);
    let wall_start = std::time::Instant::now();
    for _ in 0..hours {
        world.tick();
        samples.push(world.last_ai_phase_ns);
    }
    let wall_elapsed_ms = wall_start.elapsed().as_millis();

    samples.sort_unstable();
    let n = samples.len() as u64;
    let sum: u64 = samples.iter().sum();
    let avg = sum / n;
    let p50 = samples[(n / 2) as usize];
    let p95 = samples[((n * 95) / 100) as usize];
    let max = *samples.last().unwrap();

    println!();
    println!("AI phase per-tick latency (ns):");
    println!("  avg : {:>10}  ({:.3} ms)", avg, avg as f64 / 1.0e6);
    println!("  p50 : {:>10}  ({:.3} ms)", p50, p50 as f64 / 1.0e6);
    println!("  p95 : {:>10}  ({:.3} ms)", p95, p95 as f64 / 1.0e6);
    println!("  max : {:>10}  ({:.3} ms)", max, max as f64 / 1.0e6);
    println!();
    println!(
        "Per-ship-per-tick avg: {:.2} µs",
        (avg as f64) / (n_ships as f64) / 1.0e3
    );
    println!(
        "Total wall time (all phases, {} measured ticks): {} ms",
        hours, wall_elapsed_ms
    );
    println!(
        "AI-phase share of wall time: {:.1}%",
        100.0 * (sum as f64) / 1.0e6 / (wall_elapsed_ms as f64).max(1.0)
    );
}
