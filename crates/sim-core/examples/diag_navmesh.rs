//! Build the navmesh and print stats: node/edge counts, build time, average
//! degree, and per-port reachability. Useful for tuning `OPEN_STRIDE_NM`,
//! `MIN_OFFSHORE_NM`, `MIN_CHANNEL_NM`, `MAX_EDGE_NM`.
//!
//! Usage: `cargo run --release --example diag_navmesh`

use sim_core::navmesh::Navmesh;
use sim_core::world::World;
use std::path::Path;
use std::time::Instant;

fn main() {
    let world = World::load(Path::new("data/"));

    let t0 = Instant::now();
    let nm = Navmesh::build(&world.map.land);
    let build_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let edges: usize = nm.adj.iter().map(|v| v.len()).sum::<usize>() / 2;
    let avg_deg =
        nm.adj.iter().map(|v| v.len()).sum::<usize>() as f32 / nm.nodes.len().max(1) as f32;

    println!("navmesh built in {:.1} ms", build_ms);
    println!("  nodes: {}", nm.nodes.len());
    println!("  edges: {} (undirected)", edges);
    println!("  avg degree: {:.2}", avg_deg);

    println!("\nPort connectivity (visible nodes within 60 NM, then within 120 NM) [from harbor.anchor]:");
    let mut unreachable = 0usize;
    for (idx, port) in world.ports.iter().enumerate() {
        let anchor = match world.harbors.for_port(idx) {
            Some(h) => h.anchor,
            None => port.position,
        };
        let v60 = nm.visible_from(&world.map.land, anchor, 60.0, 32, 0.0);
        let v120 = nm.visible_from(&world.map.land, anchor, 120.0, 32, 0.0);
        let nearest = nm
            .nodes_within(anchor, 200.0)
            .into_iter()
            .map(|i| nm.nodes[i as usize].pos.distance(anchor))
            .fold(f32::INFINITY, f32::min);
        if v120.is_empty() {
            unreachable += 1;
        }
        println!(
            "  {:<22} nearest={:6.1} NM  visible@60={:>3}  visible@120={:>3}",
            port.name,
            nearest,
            v60.len(),
            v120.len(),
        );
    }
    println!(
        "\n{} of {} ports have NO visible navmesh node within 120 NM",
        unreachable,
        world.ports.len()
    );

    // Sample routing: end-to-end across a few representative pairs.
    println!("\nSample routes (start->goal entry-set, then graph A*):");
    let routes: &[(&str, &str)] = &[
        ("Bridgetown", "Tobago"),
        ("Bridgetown", "Port Royal"),
        ("Havana", "Nassau"),
        ("Petit-Goâve", "Cartagena"),
        ("Cartagena", "New York"),
        ("Cartagena", "Philadelphia"),
        ("Port Royal", "Cartagena"),
        ("Trinidad", "Bridgetown"),
    ];
    for (a, b) in routes {
        let ai = match world.ports.iter().position(|p| p.name == *a) {
            Some(i) => i,
            None => {
                println!("  {} -> {}: missing", a, b);
                continue;
            }
        };
        let bi = match world.ports.iter().position(|p| p.name == *b) {
            Some(i) => i,
            None => {
                println!("  {} -> {}: missing", a, b);
                continue;
            }
        };
        let pa = world
            .harbors
            .for_port(ai)
            .map(|h| h.anchor)
            .unwrap_or(world.ports[ai].position);
        let pb = world
            .harbors
            .for_port(bi)
            .map(|h| h.anchor)
            .unwrap_or(world.ports[bi].position);
        let starts = nm.visible_from(&world.map.land, pa, 120.0, 16, 0.0);
        let goals = nm.visible_from(&world.map.land, pb, 120.0, 16, 0.0);
        let t = Instant::now();
        let route = nm.route(&starts, &goals);
        let us = t.elapsed().as_micros();
        match route {
            Some(path) => println!(
                "  {:<14} -> {:<14}  {:>4} hops  {:>5} us  (starts={} goals={})",
                a,
                b,
                path.len(),
                us,
                starts.len(),
                goals.len()
            ),
            None => println!(
                "  {:<14} -> {:<14}  NO ROUTE       (starts={} goals={})",
                a,
                b,
                starts.len(),
                goals.len()
            ),
        }
    }
}
