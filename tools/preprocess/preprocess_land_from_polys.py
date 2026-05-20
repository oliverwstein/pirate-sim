"""
Rasterize Natural Earth 10m land polygons into a 1-NM land mask.

Drop-in replacement for the GEBCO-derived land_mask.bin: produces the
same binary format (header + u8 grid, 0=sea, 255=land) at the same
resolution (1 NM/cell) on the same equirectangular grid, but classifies
cells by *cell-center point-in-polygon* against the NE 10m land
polygons. This makes mixed coastal cells (estuaries, river mouths, deep
bays) navigable and aligns the physics raster with the visible coast.

Usage:
    python preprocess_land_from_polys.py \
        --input data/raw/ne_10m_land/ne_10m_land.shp \
        --output data/grids/land_mask.bin
"""

import argparse
import math
import struct
import sys
from pathlib import Path

import numpy as np
import shapefile  # pyshp
import shapely
from shapely.geometry import Polygon, MultiPolygon, box, shape as shapely_shape

ORIGIN_LAT = 17.5
ORIGIN_LON = -72.5

LAT_MIN = -5.0
LAT_MAX = 60.0
LON_MIN = -90.0
LON_MAX = 15.0

# 1 NM / cell. Matches preprocess_land.py.
CELL_DEG = 1.0 / 60.0
CELL_NM = CELL_DEG * 60.0  # 1.0


def latlon_to_nm(lat: float, lon: float) -> tuple[float, float]:
    dy = (lat - ORIGIN_LAT) * 60.0
    dx = (lon - ORIGIN_LON) * 60.0
    return dx, dy


def shapefile_polygons(path: Path):
    sf = shapefile.Reader(str(path))
    for shp in sf.shapes():
        if not shp.points:
            continue
        try:
            geom = shapely_shape(shp.__geo_interface__)
        except Exception:
            continue
        if not geom.is_valid:
            geom = geom.buffer(0)
        if geom.is_empty:
            continue
        if isinstance(geom, MultiPolygon):
            for sub in geom.geoms:
                if not sub.is_empty:
                    yield sub
        elif isinstance(geom, Polygon):
            yield geom


def project_polygon(poly: Polygon) -> Polygon:
    """Project a lat/lon polygon's coords to NM-from-origin space."""
    def proj_ring(ring):
        return [latlon_to_nm(lat, lon) for lon, lat in ring]

    outer = list(poly.exterior.coords)
    if outer and outer[0] == outer[-1]:
        outer = outer[:-1]
    holes = []
    for ring in poly.interiors:
        h = list(ring.coords)
        if h and h[0] == h[-1]:
            h = h[:-1]
        holes.append(h)
    return Polygon(proj_ring(outer), [proj_ring(h) for h in holes])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--input", required=True)
    ap.add_argument("--output", required=True)
    args = ap.parse_args()

    in_path = Path(args.input)
    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    # Grid setup matching preprocess_land.py.
    width = int(round((LON_MAX - LON_MIN) / CELL_DEG))
    height = int(round((LAT_MAX - LAT_MIN) / CELL_DEG))
    nw_x, nw_y = latlon_to_nm(LAT_MAX, LON_MIN)  # NW corner in NM
    cell_size_nm = CELL_NM
    print(f"Grid: {width}x{height} = {width*height:,} cells, cell={cell_size_nm} NM")

    # Cell-center coords: x = nw_x + (col + 0.5)*cs, y = nw_y - (row + 0.5)*cs
    # Build the per-axis coordinate arrays once.
    col_centers_x = nw_x + (np.arange(width, dtype=np.float64) + 0.5) * cell_size_nm
    row_centers_y = nw_y - (np.arange(height, dtype=np.float64) + 0.5) * cell_size_nm

    # 5-point per-cell sample: 4 corners + center. A cell is land iff
    # >=3 of those samples are inside polygon. This is more permissive
    # than cell-center-only (which quantizes narrow harbor entrances
    # closed, e.g. Kingston Harbour behind the Palisadoes) yet still
    # excludes mixed coastal cells dominated by water (estuaries, deep
    # bays). The trade-off: islands much smaller than 1 NM may collapse
    # to sea, but they're invisible at this scale and the visualizer
    # draws the precise polygon mesh independently.
    sample_ox = np.array([-0.5, 0.5, -0.5, 0.5, 0.0], dtype=np.float64) * cell_size_nm
    sample_oy = np.array([-0.5, -0.5, 0.5, 0.5, 0.0], dtype=np.float64) * cell_size_nm
    n_samples = len(sample_ox)
    inside_count = np.zeros((height, width), dtype=np.uint8)

    print(f"Reading {in_path}...")
    clip = box(LON_MIN, LAT_MIN, LON_MAX, LAT_MAX)

    polys_processed = 0
    for poly in shapefile_polygons(in_path):
        # Clip in lat/lon (cheaper than projecting first).
        clipped = poly.intersection(clip)
        if clipped.is_empty:
            continue
        if isinstance(clipped, Polygon):
            sub_polys = [clipped]
        elif isinstance(clipped, MultiPolygon):
            sub_polys = list(clipped.geoms)
        else:
            sub_polys = [g for g in getattr(clipped, "geoms", []) if isinstance(g, Polygon)]

        for sp in sub_polys:
            if sp.is_empty:
                continue
            proj = project_polygon(sp)
            if proj.is_empty:
                continue

            minx, miny, maxx, maxy = proj.bounds
            # Translate the polygon's NM bbox into raster cell indices,
            # with a 1-cell pad so we don't miss boundary cells.
            col0 = max(0, int(math.floor((minx - nw_x) / cell_size_nm)) - 1)
            col1 = min(width, int(math.ceil((maxx - nw_x) / cell_size_nm)) + 1)
            row0 = max(0, int(math.floor((nw_y - maxy) / cell_size_nm)) - 1)
            row1 = min(height, int(math.ceil((nw_y - miny) / cell_size_nm)) + 1)
            if col1 <= col0 or row1 <= row0:
                continue

            xs = col_centers_x[col0:col1]
            ys = row_centers_y[row0:row1]
            xx, yy = np.meshgrid(xs, ys)
            for sx, sy in zip(sample_ox, sample_oy):
                inside = shapely.contains_xy(proj, (xx + sx).ravel(), (yy + sy).ravel())
                inside = inside.reshape(yy.shape)
                # Add 1 to count for cells whose this-sample is in polygon.
                inside_count[row0:row1, col0:col1] += inside.astype(np.uint8)
            polys_processed += 1
        if polys_processed % 200 == 0 and polys_processed > 0:
            print(f"  ...processed {polys_processed} polygons")

    # Threshold: cell is land iff at least 3 of the 5 samples are in polygon.
    mask = np.where(inside_count >= 3, 255, 0).astype(np.uint8)

    land_cells = int(np.count_nonzero(mask))
    print(f"Done. {polys_processed} polygons processed; {land_cells:,} land cells "
          f"({100.0 * land_cells / mask.size:.1f}% of bbox).")

    # Write same binary format as preprocess_land.py.
    with open(out_path, "wb") as f:
        f.write(struct.pack("<I", width))
        f.write(struct.pack("<I", height))
        f.write(struct.pack("<f", nw_x))
        f.write(struct.pack("<f", nw_y))
        f.write(struct.pack("<f", cell_size_nm))
        f.write(mask.tobytes())

    size = out_path.stat().st_size
    print(f"Wrote {out_path} ({size:,} bytes).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
