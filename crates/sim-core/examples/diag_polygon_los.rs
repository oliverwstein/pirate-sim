//! Benchmark polygon-aware LOS (`CoastlineGeom::line_is_clear`) vs
//! raster LOS (`LandMap::line_is_clear`) on the production world.
//!
//! Generates `N_SEGMENTS` random sea→? segments and times each
//! variant. Reports average, P50, P99, and max query times, plus the
//! agreement rate between the two oracles.
//!
//! Usage: `cargo run --release --example diag_polygon_los`

use sim_core::coastline_geom::CoastlineGeom;
use sim_core::types::Position;
use sim_core::world::World;
use std::path::Path;
use std::time::Instant;

const N_SEGMENTS: usize = 10_000;
const MAX_SEGMENT_NM: f32 = 200.0;
const SEED: u64 = 0xC0FFEE_BEEF;

/// Tiny xorshift64* PRNG so the bench is deterministic without an
/// extra crate dep.
struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn p(sorted: &[u128], q: f32) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let i = ((sorted.len() as f32 - 1.0) * q).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn main() {
    let world = World::load(Path::new("data/"));
    let geom = CoastlineGeom::build(&world.map.land, &world.coastline, &world.land_mesh);

    println!(
        "Loaded world: {}x{} cells, coastline polylines={}, mesh tris={}",
        world.map.land.width,
        world.map.land.height,
        world.coastline.lines.len(),
        world.land_mesh.indices.len() / 3,
    );
    println!(
        "CoastlineGeom: has_polylines={} has_triangles={}",
        geom.has_polylines(),
        geom.has_triangles(),
    );

    // World extent in NM. LandMap origin sits at the NW corner with
    // world-Y decreasing as row index increases; safest bounds are
    // taken from the LandMap's own extents.
    let w = world.map.land.width as f32 * world.map.land.cell_size_nm;
    let h = world.map.land.height as f32 * world.map.land.cell_size_nm;
    let origin = world.map.land.origin;
    let x_min = origin.x;
    let x_max = origin.x + w;
    let y_max = origin.y;
    let y_min = origin.y - h;

    // Generate segments. We deliberately allow endpoints on land so
    // the bench exercises the polygon refinement path; both oracles
    // are queried on identical inputs.
    let mut rng = Rng(SEED);
    let mut segs: Vec<(Position, Position)> = Vec::with_capacity(N_SEGMENTS);
    while segs.len() < N_SEGMENTS {
        let ax = x_min + rng.next_f32() * (x_max - x_min);
        let ay = y_min + rng.next_f32() * (y_max - y_min);
        // Random direction + length up to MAX_SEGMENT_NM.
        let angle = rng.next_f32() * std::f32::consts::TAU;
        let len = rng.next_f32() * MAX_SEGMENT_NM + 1.0;
        let bx = ax + angle.cos() * len;
        let by = ay + angle.sin() * len;
        // Clamp endpoint into bounds so we measure interior work.
        let bx = bx.clamp(x_min, x_max);
        let by = by.clamp(y_min, y_max);
        segs.push((Position::new(ax, ay), Position::new(bx, by)));
    }

    // Warm up.
    let mut sink = 0u32;
    for &(a, b) in &segs[..256] {
        if world.map.land.line_is_clear(a, b) {
            sink = sink.wrapping_add(1);
        }
        if geom.line_is_clear(a, b) {
            sink = sink.wrapping_add(1);
        }
    }

    // --- raster ---
    let mut raster_us: Vec<u128> = Vec::with_capacity(N_SEGMENTS);
    let mut raster_clear = 0usize;
    for &(a, b) in &segs {
        let t0 = Instant::now();
        let r = world.map.land.line_is_clear(a, b);
        raster_us.push(t0.elapsed().as_nanos());
        if r {
            raster_clear += 1;
        }
    }

    // --- polygon-aware ---
    let mut poly_us: Vec<u128> = Vec::with_capacity(N_SEGMENTS);
    let mut poly_clear = 0usize;
    for &(a, b) in &segs {
        let t0 = Instant::now();
        let r = geom.line_is_clear(a, b);
        poly_us.push(t0.elapsed().as_nanos());
        if r {
            poly_clear += 1;
        }
    }

    // Agreement: how often do the two oracles return the same verdict?
    let mut agree = 0usize;
    let mut raster_blocked_poly_clear = 0usize;
    let mut raster_clear_poly_blocked = 0usize;
    for (i, &(a, b)) in segs.iter().enumerate() {
        let r = world.map.land.line_is_clear(a, b);
        let p = geom.line_is_clear(a, b);
        if r == p {
            agree += 1;
        } else if !r && p {
            raster_blocked_poly_clear += 1;
        } else {
            raster_clear_poly_blocked += 1;
        }
        let _ = i;
    }

    let mut rs = raster_us.clone();
    rs.sort_unstable();
    let mut ps = poly_us.clone();
    ps.sort_unstable();

    let avg = |v: &[u128]| v.iter().sum::<u128>() as f64 / v.len() as f64;

    println!();
    println!(
        "N segments: {} (max length {} NM)",
        N_SEGMENTS, MAX_SEGMENT_NM
    );
    println!("sink={} (anti-DCE)", sink);
    println!();
    println!(
        "{:<14} {:>10} {:>10} {:>10} {:>10}  {:>10}",
        "oracle", "avg(ns)", "p50(ns)", "p99(ns)", "max(ns)", "clear%"
    );
    println!("{}", "-".repeat(72));
    println!(
        "{:<14} {:>10.0} {:>10} {:>10} {:>10}  {:>9.1}%",
        "raster",
        avg(&rs),
        p(&rs, 0.5),
        p(&rs, 0.99),
        rs.last().copied().unwrap_or(0),
        100.0 * raster_clear as f32 / N_SEGMENTS as f32,
    );
    println!(
        "{:<14} {:>10.0} {:>10} {:>10} {:>10}  {:>9.1}%",
        "polygon",
        avg(&ps),
        p(&ps, 0.5),
        p(&ps, 0.99),
        ps.last().copied().unwrap_or(0),
        100.0 * poly_clear as f32 / N_SEGMENTS as f32,
    );

    println!();
    println!(
        "Agreement: {}/{} ({:.2}%)",
        agree,
        N_SEGMENTS,
        100.0 * agree as f32 / N_SEGMENTS as f32
    );
    println!(
        "  raster=blocked, polygon=clear: {} (polygon refined misclassified land)",
        raster_blocked_poly_clear
    );
    println!(
        "  raster=clear,   polygon=blocked: {} (should be 0 — polygon never adds land)",
        raster_clear_poly_blocked
    );

    let avg_poly_us = avg(&ps) / 1000.0;
    let p99_poly_us = p(&ps, 0.99) as f64 / 1000.0;
    println!();
    println!(
        "Polygon LOS: avg {:.2} µs, p99 {:.2} µs  (targets: < 5 µs / < 50 µs)",
        avg_poly_us, p99_poly_us
    );
    if avg_poly_us < 5.0 && p99_poly_us < 50.0 {
        println!("✓ within targets");
    } else {
        println!("✗ exceeds targets");
    }
}
