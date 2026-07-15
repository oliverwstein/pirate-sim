"""
Bake land/sea classification onto the Tier-1 global geodesic mesh
(`sim_core::geo::GeoMesh`) from Natural Earth land polygons.

The mesh's topology (tile centers, adjacency) is procedural and
deterministic from `subdivisions` alone -- see `crates/sim-core/src/geo/mesh.rs`
-- so this script does not rebuild it. Instead it takes the mesh's
per-tile (lat, lon) centers (dumped by `cargo run --release -p sim-core
--example geo_dump`) and classifies each one as land or sea via
point-in-polygon against a land-polygon dataset, then writes a compact
binary flag array in the same tile-id order.

Data source: Natural Earth land polygons (any resolution; 50m is a good
balance of file size vs. coastline fidelity at ~40-70 km tile spacing --
110m is too coarse for many islands/straits, 10m is overkill and slow).
Download the "land" GeoJSON/shapefile for your chosen resolution into
`raw/` (gitignored, same convention as the other preprocessors).

Binary format (`data/grids/geo_tiles.bin`), little-endian:
    u32  subdivisions
    u32  tile_count
    u8 x tile_count   -- 1 = land, 0 = sea, indexed by tile id

Usage:
    cargo run --release -p sim-core --example geo_dump -- 6 > /tmp/geo_tiles.json
    python preprocess_geo_tiles.py \
        --tiles /tmp/geo_tiles.json \
        --land ../../raw/ne_50m_land.geojson \
        --output ../../data/grids/geo_tiles.bin
"""

import argparse
import json
import struct
import sys
from pathlib import Path

from shapely.geometry import Point, shape
from shapely.strtree import STRtree


def load_land_polygons(path: Path):
    data = json.loads(path.read_text())
    polygons = [shape(feature["geometry"]) for feature in data["features"]]
    return polygons


def classify_tiles(tiles: list[dict], polygons: list) -> bytes:
    tree = STRtree(polygons)
    flags = bytearray(len(tiles))
    for tile in tiles:
        point = Point(tile["lon"], tile["lat"])
        for idx in tree.query(point):
            candidate = polygons[idx]
            if candidate.contains(point) or candidate.intersects(point):
                flags[tile["id"]] = 1
                break
    return bytes(flags)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--tiles", required=True, type=Path, help="JSON dump from `geo_dump` example"
    )
    parser.add_argument(
        "--land", required=True, type=Path, help="Natural Earth land polygons (GeoJSON)"
    )
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    mesh = json.loads(args.tiles.read_text())
    subdivisions = mesh["subdivisions"]
    tiles = mesh["tiles"]
    tile_count = mesh["tile_count"]
    assert len(tiles) == tile_count, "tile dump is internally inconsistent"

    print(f"Loading land polygons from {args.land}...")
    polygons = load_land_polygons(args.land)
    print(f"Classifying {tile_count} tiles (subdivisions={subdivisions})...")
    flags = classify_tiles(tiles, polygons)

    land_count = sum(flags)
    print(f"{land_count} land tiles, {tile_count - land_count} sea tiles")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with open(args.output, "wb") as f:
        f.write(struct.pack("<II", subdivisions, tile_count))
        f.write(flags)
    print(f"Wrote {args.output}")


if __name__ == "__main__":
    sys.exit(main())
