"""
Preprocess Natural Earth 10m coastline into a binary polyline file.

The shapefile contains the world's coastline as MultiLineStrings in lat/lon.
We clip to the project bounding box, reproject to NM-from-origin (matching
the LandMap / WindGrid coordinate frame), and write a compact binary:

    u32  num_lines
    repeated num_lines times:
        u32  num_points
        f32  x_0, y_0, x_1, y_1, ...  (NM)

Usage:
    python preprocess_coast.py \
        --input data/raw/ne_10m_coastline/ne_10m_coastline.shp \
        --output data/grids/coastline.bin
"""

import argparse
import math
import struct
import sys
from pathlib import Path

import shapefile  # pyshp

ORIGIN_LAT = 17.5
ORIGIN_LON = -72.5

LAT_MIN = -5.0
LAT_MAX = 60.0
LON_MIN = -90.0
LON_MAX = 15.0


def latlon_to_nm(lat: float, lon: float) -> tuple[float, float]:
    # Matches preprocess_land.py: equirectangular, no cos(lat) on x.
    # The LandMap is built on a uniform grid in this projection, so the
    # coastline must use the identical formula or it will offset visibly.
    dy = (lat - ORIGIN_LAT) * 60.0
    dx = (lon - ORIGIN_LON) * 60.0
    return dx, dy


def in_bbox(lat: float, lon: float) -> bool:
    return LAT_MIN <= lat <= LAT_MAX and LON_MIN <= lon <= LON_MAX


def clip_polyline(points_lonlat: list[tuple[float, float]]) -> list[list[tuple[float, float]]]:
    """Split a polyline into runs that lie within the bounding box.

    Vertices outside the box drop out; the polyline is broken into separate
    runs at any out-of-box gap. Runs of length < 2 are discarded.
    """
    runs: list[list[tuple[float, float]]] = []
    cur: list[tuple[float, float]] = []
    for lon, lat in points_lonlat:
        if in_bbox(lat, lon):
            cur.append((lon, lat))
        else:
            if len(cur) >= 2:
                runs.append(cur)
            cur = []
    if len(cur) >= 2:
        runs.append(cur)
    return runs


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--input", required=True, help="Path to ne_10m_coastline.shp")
    ap.add_argument("--output", required=True, help="Output coastline.bin")
    args = ap.parse_args()

    in_path = Path(args.input)
    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    print(f"Reading {in_path}...")
    sf = shapefile.Reader(str(in_path))

    lines: list[list[tuple[float, float]]] = []
    total_input_lines = 0

    for shape in sf.shapes():
        # Each shape can be a (multi)polyline; `parts` indexes the start of
        # each segment within `points`.
        pts = shape.points
        if not pts:
            continue
        parts = list(shape.parts) + [len(pts)]
        for i in range(len(parts) - 1):
            seg_lonlat = pts[parts[i]:parts[i + 1]]
            total_input_lines += 1
            for run in clip_polyline(seg_lonlat):
                proj = [latlon_to_nm(lat, lon) for lon, lat in run]
                lines.append(proj)

    total_points = sum(len(line) for line in lines)
    print(
        f"Kept {len(lines)} polylines "
        f"({total_points} points) from {total_input_lines} input segments."
    )

    with open(out_path, "wb") as f:
        f.write(struct.pack("<I", len(lines)))
        for line in lines:
            f.write(struct.pack("<I", len(line)))
            for x, y in line:
                f.write(struct.pack("<ff", x, y))

    size = out_path.stat().st_size
    print(f"Wrote {out_path} ({size:,} bytes).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
