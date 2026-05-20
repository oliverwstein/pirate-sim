"""
Preprocess Natural Earth 10m land polygons into a binary triangle mesh.

The output is a flat triangle list ready for direct rendering. Holes
(lakes inside larger landmasses) are handled by the earcut triangulator.

Pipeline:
    1. Read each polygon from the shapefile (with rings = outer + holes).
    2. Clip to the project bounding box using shapely (handles polygons
       crossing the bbox edge correctly, returning a Polygon or
       MultiPolygon).
    3. Reproject every vertex with the *same* equirectangular formula as
       preprocess_land.py (no cos(lat) on x), so the result aligns with
       the LandMap raster used for collisions.
    4. Triangulate via mapbox_earcut, producing a flat (triangle_count*3)
       index list referring to a per-polygon vertex list.
    5. Emit a single global vertex array and triangle index array.

Binary format:
    u32  num_vertices
    f32  x_0, y_0, x_1, y_1, ...   (NM-from-origin)
    u32  num_indices               (== triangle_count * 3)
    u32  i_0, i_1, i_2, ...

Usage:
    python preprocess_land_polys.py \
        --input data/raw/ne_10m_land/ne_10m_land.shp \
        --output data/grids/land_polys.bin
"""

import argparse
import math
import struct
import sys
from pathlib import Path

import numpy as np
import shapefile  # pyshp
import mapbox_earcut as earcut
from shapely.geometry import Polygon, MultiPolygon, box, shape as shapely_shape
from shapely.ops import unary_union

ORIGIN_LAT = 17.5
ORIGIN_LON = -72.5

LAT_MIN = -5.0
LAT_MAX = 60.0
LON_MIN = -90.0
LON_MAX = 15.0


def latlon_to_nm(lat: float, lon: float) -> tuple[float, float]:
    # Equirectangular, no cos(lat) on x — matches preprocess_land.py and
    # therefore the LandMap grid frame.
    dy = (lat - ORIGIN_LAT) * 60.0
    dx = (lon - ORIGIN_LON) * 60.0
    return dx, dy


def shapefile_polygons(path: Path):
    """Yield shapely Polygons from a shapefile. We use pyshp's GeoJSON
    `__geo_interface__` which correctly identifies ring nesting (outers
    vs holes) — natively handling cases like ne_10m_land where each
    record is a continent-sized MultiPolygon containing many separate
    islands plus genuine holes (lakes)."""
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


def clip_to_bbox(poly: Polygon, clip: Polygon):
    inter = poly.intersection(clip)
    if inter.is_empty:
        return []
    if isinstance(inter, Polygon):
        return [inter]
    if isinstance(inter, MultiPolygon):
        return [g for g in inter.geoms if not g.is_empty]
    # GeometryCollection: pull out polygons only
    out = []
    for g in getattr(inter, "geoms", []):
        if isinstance(g, Polygon) and not g.is_empty:
            out.append(g)
    return out


def triangulate(poly: Polygon) -> tuple[np.ndarray, np.ndarray]:
    """Return (vertices_xy, triangle_indices) for a single polygon.

    Vertices are projected to NM. Triangle indices are local to this
    vertex array.
    """
    outer = list(poly.exterior.coords)
    # Drop closing duplicate vertex if present.
    if len(outer) > 1 and outer[0] == outer[-1]:
        outer = outer[:-1]

    holes = []
    for ring in poly.interiors:
        h = list(ring.coords)
        if len(h) > 1 and h[0] == h[-1]:
            h = h[:-1]
        holes.append(h)

    # Project everything to NM up front.
    def proj_ring(ring):
        return [latlon_to_nm(lat, lon) for lon, lat in ring]

    outer_nm = proj_ring(outer)
    holes_nm = [proj_ring(h) for h in holes]

    if len(outer_nm) < 3:
        return np.empty((0, 2), dtype=np.float32), np.empty(0, dtype=np.uint32)

    # earcut input: flat float array, plus a list of *ring end* indices
    # (one per ring, including the outer). [outer_end, hole1_end, ...].
    flat = list(outer_nm)
    ring_ends: list[int] = [len(flat)]
    for h in holes_nm:
        if len(h) < 3:
            continue
        flat.extend(h)
        ring_ends.append(len(flat))

    coords = np.asarray(flat, dtype=np.float32).reshape(-1, 2)
    rings = np.asarray(ring_ends, dtype=np.uint32)
    indices = earcut.triangulate_float32(coords, rings)
    return coords, indices.astype(np.uint32)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--input", required=True)
    ap.add_argument("--output", required=True)
    args = ap.parse_args()

    in_path = Path(args.input)
    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    print(f"Reading {in_path}...")
    clip = box(LON_MIN, LAT_MIN, LON_MAX, LAT_MAX)

    all_verts: list[np.ndarray] = []
    all_idx: list[np.ndarray] = []
    vert_offset = 0

    polys_in = 0
    polys_out = 0
    for poly in shapefile_polygons(in_path):
        polys_in += 1
        for clipped in clip_to_bbox(poly, clip):
            polys_out += 1
            v, idx = triangulate(clipped)
            if v.size == 0 or idx.size == 0:
                continue
            all_verts.append(v)
            all_idx.append(idx + vert_offset)
            vert_offset += v.shape[0]

    if not all_verts:
        print("No land polygons inside bbox — output will be empty.", file=sys.stderr)

    verts = np.concatenate(all_verts) if all_verts else np.empty((0, 2), dtype=np.float32)
    idx = np.concatenate(all_idx) if all_idx else np.empty(0, dtype=np.uint32)

    print(
        f"Polys in: {polys_in}, after clip: {polys_out}, "
        f"vertices: {verts.shape[0]}, triangles: {idx.shape[0] // 3}"
    )

    with open(out_path, "wb") as f:
        f.write(struct.pack("<I", verts.shape[0]))
        f.write(verts.astype("<f4").tobytes())
        f.write(struct.pack("<I", idx.shape[0]))
        f.write(idx.astype("<u4").tobytes())

    size = out_path.stat().st_size
    print(f"Wrote {out_path} ({size:,} bytes).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
