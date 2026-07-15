//! Dumps the Tier-1 geodesic mesh (see `sim_core::geo`) as JSON for
//! visual sanity-checking. Not used at runtime — a debugging/inspection
//! aid while the global map expansion is under development.
//!
//! Usage: `cargo run --release --example geo_dump -- 4 > mesh.json`
//! (subdivision level defaults to 4 if omitted; tile count is `10*4^n+2`).

use sim_core::geo::GeoMesh;
use std::io::Write;

fn main() {
    let subdivisions: u32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let mesh = GeoMesh::build(subdivisions);

    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());

    writeln!(out, "{{").unwrap();
    writeln!(out, "  \"subdivisions\": {},", mesh.subdivisions).unwrap();
    writeln!(out, "  \"tile_count\": {},", mesh.tile_count()).unwrap();
    writeln!(out, "  \"tiles\": [").unwrap();
    for i in 0..mesh.tile_count() {
        let neighbors = mesh.neighbors[i]
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            out,
            "    {{\"id\":{},\"lat\":{:.4},\"lon\":{:.4},\"pentagon\":{},\"neighbors\":[{}]}}{}",
            i,
            mesh.lat_deg[i],
            mesh.lon_deg[i],
            mesh.is_pentagon(i as u32),
            neighbors,
            if i + 1 < mesh.tile_count() { "," } else { "" }
        )
        .unwrap();
    }
    writeln!(out, "  ]").unwrap();
    writeln!(out, "}}").unwrap();
}
