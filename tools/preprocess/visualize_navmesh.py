"""
Visualize navmesh output and/or the shore-buffer contour.

Two independent overlays — combine freely:

  Land + buffer contour (requires --shp):
      python visualize_navmesh.py \\
          --shp data/raw/ne_10m_land/ne_10m_land.shp \\
          --shore-margin-nm 0.25 \\
          --output planning/buffer-contour.png \\
          --dpi 200 --fig-width-in 24

  Navmesh tiles + centroids (requires --input):
      python visualize_navmesh.py \\
          --input data/grids/navmesh.bin \\
          --output planning/navmesh-tiling.png

  Combined — land background + buffer + tiles + portals:
      python visualize_navmesh.py \\
          --input data/grids/navmesh.bin \\
          --shp   data/raw/ne_10m_land/ne_10m_land.shp \\
          --shore-margin-nm 0.25 \\
          --portals \\
          --output planning/navmesh-full.png

navmesh.bin format expected (little-endian, portal-aware):
    u32 num_tiles
    per tile:
        u32 num_vertices
        (f32 x, f32 y) × num_vertices
        f32 centroid_x, f32 centroid_y
        u32 num_neighbors
        (u32 tile_index, f32 portal_x, f32 portal_y) × num_neighbors
"""

from __future__ import annotations

import argparse
import colorsys
import struct
import sys
from pathlib import Path
from typing import Optional

import re
from collections import defaultdict

import matplotlib.pyplot as plt
import numpy as np
import scipy.sparse
import scipy.sparse.csgraph
from matplotlib.collections import LineCollection, PolyCollection
from scipy.spatial import KDTree

# ---------------------------------------------------------------------------
# Coordinate constants (must match preprocess_navmesh.py)
# ---------------------------------------------------------------------------

ORIGIN_LAT = 17.5
ORIGIN_LON = -72.5
LAT_MIN, LAT_MAX = -5.0, 60.0
LON_MIN, LON_MAX = -90.0, 15.0


def latlon_to_nm(lat: float, lon: float) -> tuple[float, float]:
    return (lon - ORIGIN_LON) * 60.0, (lat - ORIGIN_LAT) * 60.0


# ---------------------------------------------------------------------------
# navmesh.bin reader (portal-aware format)
# ---------------------------------------------------------------------------


def read_navmesh(path: Path):
    """Parse navmesh.bin (portal-aware format).

    Binary layout (little-endian):
        u32  num_tiles
        per tile:
            u32  num_vertices
            (f32 x, f32 y) × num_vertices     -- CCW polygon, NM-from-origin
            f32  centroid_x, f32 centroid_y
            u32  num_neighbors
            (u32 tile_index, f32 portal_x, f32 portal_y) × num_neighbors

    Returns:
        tiles      – list of (n, 2) float32 arrays, CCW vertex coordinates
        centroids  – list of (cx, cy) floats
        neighbors  – list of lists of (tile_index, portal_x, portal_y)
    """
    buf = path.read_bytes()
    offset = 0

    n_tiles = struct.unpack_from("<I", buf, offset)[0]
    offset += 4

    tiles = []
    centroids = []
    neighbors = []
    for _ in range(n_tiles):
        nv = struct.unpack_from("<I", buf, offset)[0]
        offset += 4

        # Read all vertex XY pairs in one call for alignment safety.
        coords = np.array(
            struct.unpack_from(f"<{2 * nv}f", buf, offset), dtype=np.float32
        ).reshape(nv, 2)
        offset += 8 * nv

        cx, cy = struct.unpack_from("<ff", buf, offset)
        offset += 8

        nn = struct.unpack_from("<I", buf, offset)[0]
        offset += 4

        # Each neighbor: u32 index + f32 portal_x + f32 portal_y (<Iff, 12 bytes).
        nbrs = []
        for _ in range(nn):
            ni, px, py = struct.unpack_from("<Iff", buf, offset)
            offset += 12
            nbrs.append((ni, px, py))

        tiles.append(coords)
        centroids.append((cx, cy))
        neighbors.append(nbrs)
    return tiles, centroids, neighbors


# ---------------------------------------------------------------------------
# Land polygon loading (for background + buffer)
# ---------------------------------------------------------------------------


def load_land_polygons_nm(shp_path: Path) -> list[np.ndarray]:
    """Load land polygons from a WGS-84 shapefile, projected to NM coords.

    Returns a list of (n, 2) exterior-ring coordinate arrays.
    """
    try:
        import shapefile
        from shapely.geometry import MultiPolygon, Polygon, box
        from shapely.geometry import shape as shapely_shape
        from shapely.ops import unary_union
    except ImportError as e:
        print(f"  WARNING: cannot load land shapefile ({e}); skipping land background")
        return []

    clip = box(LON_MIN, LAT_MIN, LON_MAX, LAT_MAX)
    pieces = []
    sf = shapefile.Reader(str(shp_path))
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
        clipped = geom.intersection(clip)
        if clipped.is_empty:
            continue
        if isinstance(clipped, MultiPolygon):
            pieces.extend(g for g in clipped.geoms if not g.is_empty)
        elif isinstance(clipped, Polygon):
            pieces.append(clipped)

    polys_nm = []
    for p in pieces:
        coords = list(p.exterior.coords)
        nm = np.array([latlon_to_nm(lat, lon) for lon, lat in coords], dtype=np.float32)
        if len(nm) >= 3:
            polys_nm.append(nm)
    return polys_nm


def compute_buffer_contour_nm(
    shp_path: Path,
    shore_margin_nm: float,
) -> list[np.ndarray]:
    """Compute the shore-margin buffer boundary in NM coordinates.

    Uses EPSG:3857 (Web Mercator) for meter-accurate, isotropic buffering
    at all latitudes — identical to the preprocess_navmesh.py pipeline.
    Returns a list of (n, 2) line-segment arrays for drawing.
    """
    try:
        import pyproj
        import shapefile
        from shapely.geometry import MultiPolygon, Polygon, box
        from shapely.geometry import shape as shapely_shape
        from shapely.ops import transform, unary_union
    except ImportError as e:
        print(f"  WARNING: cannot compute buffer ({e}); skipping buffer contour")
        return []

    to_merc = pyproj.Transformer.from_crs(
        "EPSG:4326", "EPSG:3857", always_xy=True
    ).transform
    to_wgs84 = pyproj.Transformer.from_crs(
        "EPSG:3857", "EPSG:4326", always_xy=True
    ).transform

    clip = box(LON_MIN, LAT_MIN, LON_MAX, LAT_MAX)
    pieces = []
    sf = shapefile.Reader(str(shp_path))
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
        clipped = geom.intersection(clip)
        if clipped.is_empty:
            continue
        if isinstance(clipped, MultiPolygon):
            pieces.extend(g for g in clipped.geoms if not g.is_empty)
        elif isinstance(clipped, Polygon):
            pieces.append(clipped)

    land_union = unary_union(pieces)
    margin_m = shore_margin_nm * 1852.0
    land_merc = transform(to_merc, land_union)
    buffered_merc = land_merc.buffer(margin_m, join_style=1, resolution=4)
    buffered_wgs84 = transform(to_wgs84, buffered_merc)

    # Extract all boundary rings of the buffered land as NM-coordinate lines.
    segments = []
    geoms = (
        list(buffered_wgs84.geoms)
        if isinstance(buffered_wgs84, MultiPolygon)
        else [buffered_wgs84]
    )
    for poly in geoms:
        rings = [list(poly.exterior.coords)] + [list(r.coords) for r in poly.interiors]
        for ring in rings:
            nm = np.array([latlon_to_nm(lat, lon) for lon, lat in ring], dtype=np.float32)
            if len(nm) >= 2:
                segments.append(nm)
    return segments


# ---------------------------------------------------------------------------
# Port loading from ports.ron
# ---------------------------------------------------------------------------


def load_ports_ron(ron_path: Path) -> list[tuple[str, float, float]]:
    """Parse ports.ron and return [(name, x_nm, y_nm), ...].

    ports.ron stores real-world WGS84 `coord: (lat, lon)`; we project
    to the simulator's NM grid using the same equirectangular anchor
    as the rest of the preprocessor pipeline (ORIGIN_LAT/ORIGIN_LON).
    """
    text = ron_path.read_text(encoding="utf-8")
    pattern = re.compile(
        r'name:\s*"([^"]+)"[^)]*coord:\s*\(([^,]+),\s*([^)]+)\)'
    )
    ports = []
    for m in pattern.finditer(text):
        name = m.group(1)
        try:
            lat = float(m.group(2).strip())
            lon = float(m.group(3).strip())
            x = (lon - ORIGIN_LON) * 60.0
            y = (lat - ORIGIN_LAT) * 60.0
            ports.append((name, x, y))
        except ValueError:
            pass
    return ports


# ---------------------------------------------------------------------------
# Shortcut edge detection
# ---------------------------------------------------------------------------


def load_land_union_nm(shp_path: Path, shore_margin_nm: float):
    """Return the buffered land union as a Shapely geometry in NM coordinates.

    Used for the shortcut water-route check.
    """
    try:
        import pyproj
        import shapefile as _sf
        from shapely.geometry import MultiPolygon, Polygon, box
        from shapely.geometry import shape as shapely_shape
        from shapely.ops import transform, unary_union
    except ImportError as e:
        print(f"  WARNING: cannot load land for shortcut check ({e})")
        return None

    to_merc = pyproj.Transformer.from_crs("EPSG:4326", "EPSG:3857", always_xy=True).transform
    to_wgs84 = pyproj.Transformer.from_crs("EPSG:3857", "EPSG:4326", always_xy=True).transform

    clip = box(LON_MIN, LAT_MIN, LON_MAX, LAT_MAX)
    pieces = []
    sf = _sf.Reader(str(shp_path))
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
        clipped = geom.intersection(clip)
        if clipped.is_empty:
            continue
        if isinstance(clipped, MultiPolygon):
            pieces.extend(g for g in clipped.geoms if not g.is_empty)
        elif isinstance(clipped, Polygon):
            pieces.append(clipped)

    land = unary_union(pieces)
    if shore_margin_nm > 0.0:
        land = transform(to_wgs84, transform(to_merc, land).buffer(shore_margin_nm * 1852.0, join_style=1, resolution=4))

    # Project to NM coordinates.
    def proj_poly(p: Polygon) -> Polygon:
        outer = [latlon_to_nm(lat, lon) for lon, lat in p.exterior.coords]
        holes = [[latlon_to_nm(lat, lon) for lon, lat in r.coords] for r in p.interiors]
        return Polygon(outer, holes)

    if isinstance(land, MultiPolygon):
        return MultiPolygon([proj_poly(p) for p in land.geoms])
    return proj_poly(land)


def compute_shortcut_edges(
    centroids: list[tuple[float, float]],
    neighbors: list[list],
    max_dist_nm: float,
    land_union_nm=None,
) -> list[tuple[int, int]]:
    """Find (i, j) pairs worth a shortcut edge.

    A pair qualifies when:
      1. Euclidean distance ≤ max_dist_nm.
      2. They are not already adjacent in the navmesh.
      3. The direct line segment is entirely over water (no land crossing).
      4. The shortest graph path between them is > 2× the direct distance.
    """
    from shapely.geometry import LineString
    from shapely.prepared import prep

    n = len(centroids)
    cx = np.array([c[0] for c in centroids], dtype=np.float64)
    cy = np.array([c[1] for c in centroids], dtype=np.float64)
    pts = np.column_stack([cx, cy])

    # Build adjacency set and sparse distance matrix.
    adj_set: set[tuple[int, int]] = set()
    rows, cols, data = [], [], []
    for i, nbrs in enumerate(neighbors):
        for entry in nbrs:
            j = entry[0] if isinstance(entry, tuple) else entry
            key = (min(i, j), max(i, j))
            if key not in adj_set:
                adj_set.add(key)
                dx, dy = cx[i] - cx[j], cy[i] - cy[j]
                d = float((dx * dx + dy * dy) ** 0.5)
                rows.append(i)
                cols.append(j)
                data.append(d)

    adj_mat = scipy.sparse.csr_matrix((data, (rows, cols)), shape=(n, n))
    # Make symmetric for undirected Dijkstra.
    adj_mat_sym = adj_mat + adj_mat.T

    # KD-tree candidate search.
    tree = KDTree(pts)
    raw_pairs = tree.query_pairs(max_dist_nm)  # set of (i, j), i < j

    candidates = [(i, j) for i, j in raw_pairs if (i, j) not in adj_set]
    print(f"    {len(candidates)} pairs within {max_dist_nm} NM, not adjacent")

    # Water check.
    if land_union_nm is not None:
        land_prep = prep(land_union_nm)
        water_ok = []
        for i, j in candidates:
            seg = LineString([(cx[i], cy[i]), (cx[j], cy[j])])
            if not land_prep.intersects(seg):
                water_ok.append((i, j))
        print(f"    {len(water_ok)} pass water check")
        candidates = water_ok

    # Graph-path check: path(i,j) > 2 × direct(i,j).
    # Group targets by source to batch Dijkstra calls.
    src_targets: dict[int, list[tuple[int, float]]] = defaultdict(list)
    direct: dict[tuple[int, int], float] = {}
    for i, j in candidates:
        dx, dy = cx[i] - cx[j], cy[i] - cy[j]
        d = float((dx * dx + dy * dy) ** 0.5)
        direct[(i, j)] = d
        src_targets[i].append((j, d))
        src_targets[j].append((i, d))

    shortcuts: set[tuple[int, int]] = set()
    for src, targets in src_targets.items():
        cutoff = max(2.0 * d for _, d in targets) + 0.1
        dist_arr = scipy.sparse.csgraph.dijkstra(
            adj_mat_sym, indices=src, limit=cutoff, directed=False
        )
        for tgt, d in targets:
            if dist_arr[tgt] > 2.0 * d:
                shortcuts.add((min(src, tgt), max(src, tgt)))

    print(f"    {len(shortcuts)} shortcut edges qualify")
    return list(shortcuts)


# ---------------------------------------------------------------------------
# Connected-component coloring
# ---------------------------------------------------------------------------


def connected_components(neighbors: list[list]) -> list[int]:
    n = len(neighbors)
    comp = [-1] * n
    cid = 0
    for start in range(n):
        if comp[start] >= 0:
            continue
        stack = [start]
        while stack:
            t = stack.pop()
            if comp[t] >= 0:
                continue
            comp[t] = cid
            for entry in neighbors[t]:
                nb = entry[0] if isinstance(entry, tuple) else entry
                if comp[nb] < 0:
                    stack.append(nb)
        cid += 1
    return comp


def tile_colors(
    tiles: list[np.ndarray],
    centroids: list[tuple[float, float]],
    comp: list[int],
) -> np.ndarray:
    n_comps = max(comp) + 1
    phi = 0.61803398875
    comp_hue = [(i * phi) % 1.0 for i in range(n_comps)]
    out = np.empty((len(tiles), 3), dtype=np.float32)
    for i, (cx, cy) in enumerate(centroids):
        h = comp_hue[comp[i]]
        kx = ((cx * 0.1734 + 17.13) % 1.0 + 1.0) % 1.0
        ky = ((cy * 0.2913 + 31.41) % 1.0 + 1.0) % 1.0
        s = 0.55 + 0.35 * kx
        v = 0.55 + 0.35 * ky
        r, g, b = colorsys.hsv_to_rgb(h, s, v)
        out[i] = (r, g, b)
    return out


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Visualize navmesh tiles and/or shore-buffer contour."
    )
    ap.add_argument(
        "--input",
        default=None,
        help="navmesh.bin (portal-aware format). Omit to show land/buffer only.",
    )
    ap.add_argument(
        "--shp",
        default=None,
        help="ne_10m_land.shp path. Enables land background and buffer contour.",
    )
    ap.add_argument(
        "--shore-margin-nm",
        type=float,
        default=0.0,
        help="Buffer distance (NM) for the contour overlay. Requires --shp.",
    )
    ap.add_argument("--output", required=True)
    ap.add_argument("--dpi", type=int, default=200)
    ap.add_argument(
        "--fig-width-in",
        type=float,
        default=24.0,
        help="Figure width in inches (default 24 for full-map high-res output).",
    )
    ap.add_argument(
        "--centroid-size",
        type=float,
        default=1.5,
        help="Centroid dot size in points² (default 1.5).",
    )
    ap.add_argument(
        "--portals",
        action="store_true",
        help="Draw portal midpoints as small orange dots.",
    )
    ap.add_argument(
        "--no-tiles",
        action="store_true",
        help="Suppress tile polygons (show centroids/portals only).",
    )
    ap.add_argument(
        "--mst",
        action="store_true",
        help="Draw the MST of the centroid adjacency graph instead of tiles.",
    )
    ap.add_argument(
        "--full-graph",
        action="store_true",
        help="Draw ALL navmesh adjacency edges (centroid to centroid). "
        "Shows the complete routing graph, not just the MST backbone.",
    )
    ap.add_argument(
        "--shortcuts-nm",
        type=float,
        default=0.0,
        help="Draw shortcut edges: pairs within this distance (NM) where the "
        "graph path is > 2× the direct distance and the line is over water. "
        "12 NM is a good starting value. Requires --input and --shp.",
    )
    ap.add_argument(
        "--ports-ron",
        default=None,
        help="Path to ports.ron. Draws port markers and labels on the map.",
    )
    ap.add_argument(
        "--bbox",
        nargs=4,
        type=float,
        metavar=("X0", "Y0", "X1", "Y1"),
        default=None,
        help="Crop to NM bbox: X0 Y0 X1 Y1 (e.g. -- -900 -400 400 500).",
    )
    args = ap.parse_args()

    if args.input is None and args.shp is None:
        print("ERROR: supply at least one of --input or --shp", file=sys.stderr)
        return 1

    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    # --- Load navmesh -------------------------------------------------------
    tiles, centroids, neighbors = [], [], []
    if args.input:
        in_path = Path(args.input)
        print(f"Reading {in_path}...")
        tiles, centroids, neighbors = read_navmesh(in_path)
        print(f"  {len(tiles)} tiles")

    # --- Load land background -----------------------------------------------
    land_polys_nm: list[np.ndarray] = []
    if args.shp:
        print(f"Loading land polygons from {args.shp}...")
        land_polys_nm = load_land_polygons_nm(Path(args.shp))
        print(f"  {len(land_polys_nm)} land polygons")

    # --- Compute buffer contour ---------------------------------------------
    buffer_segs: list[np.ndarray] = []
    if args.shp and args.shore_margin_nm > 0.0:
        print(
            f"Computing {args.shore_margin_nm:.3f} NM buffer contour "
            f"({args.shore_margin_nm * 1852:.0f} m, EPSG:3857)..."
        )
        buffer_segs = compute_buffer_contour_nm(Path(args.shp), args.shore_margin_nm)
        print(f"  {len(buffer_segs)} contour segments")

    # --- Figure extent ------------------------------------------------------
    all_x: list[float] = []
    all_y: list[float] = []
    for t in tiles:
        all_x.extend(t[:, 0].tolist())
        all_y.extend(t[:, 1].tolist())
    for p in land_polys_nm:
        all_x.extend(p[:, 0].tolist())
        all_y.extend(p[:, 1].tolist())

    if not all_x:
        # Fallback: full world extent in NM
        x_min, y_min = latlon_to_nm(LAT_MIN, LON_MIN)
        x_max, y_max = latlon_to_nm(LAT_MAX, LON_MAX)
    else:
        x_min, x_max = min(all_x), max(all_x)
        y_min, y_max = min(all_y), max(all_y)

    if args.bbox:
        bx0, by0, bx1, by1 = args.bbox
        x_min, x_max = bx0, bx1
        y_min, y_max = by0, by1
        print(f"  cropping to bbox: x={x_min}..{x_max}, y={y_min}..{y_max}")

    width_nm = x_max - x_min
    height_nm = y_max - y_min
    fig_w = args.fig_width_in
    fig_h = fig_w * height_nm / width_nm
    print(f"  extent: {width_nm:.0f} × {height_nm:.0f} NM")
    print(f"  figure: {fig_w:.1f} × {fig_h:.1f} in @ {args.dpi} dpi")

    # --- Render -------------------------------------------------------------
    bg = "#0a1a2e"  # dark navy
    fig, ax = plt.subplots(figsize=(fig_w, fig_h), facecolor=bg)
    ax.set_facecolor(bg)

    # Land background
    if land_polys_nm:
        land_col = PolyCollection(
            land_polys_nm,
            facecolor="#1a1a1a",
            edgecolor="#2a2a2a",
            linewidths=0.2,
            zorder=1,
        )
        ax.add_collection(land_col)

    # Navmesh tiles
    if tiles and not args.no_tiles:
        print("Computing components + coloring tiles...")
        comp = connected_components(neighbors)
        n_comps = max(comp) + 1
        print(f"  {n_comps} components")
        colors = tile_colors(tiles, centroids, comp)
        tile_col = PolyCollection(
            tiles,
            facecolors=colors,
            edgecolors=(0, 0, 0, 0.3),
            linewidths=0.15,
            zorder=2,
            alpha=0.85,
        )
        ax.add_collection(tile_col)

    # Centroids
    if centroids:
        cx = np.array([c[0] for c in centroids], dtype=np.float32)
        cy = np.array([c[1] for c in centroids], dtype=np.float32)
        ax.scatter(cx, cy, s=args.centroid_size, c="white", linewidths=0, zorder=4)

    # Full adjacency graph (all centroid-to-centroid edges)
    if args.full_graph and centroids and neighbors:
        cx_a = np.array([c[0] for c in centroids], dtype=np.float64)
        cy_a = np.array([c[1] for c in centroids], dtype=np.float64)
        fg_segs = []
        seen_fg: set[tuple[int, int]] = set()
        for ti, nbrs in enumerate(neighbors):
            for entry in nbrs:
                nb = entry[0] if isinstance(entry, tuple) else entry
                key = (min(ti, nb), max(ti, nb))
                if key not in seen_fg:
                    seen_fg.add(key)
                    fg_segs.append([(cx_a[ti], cy_a[ti]), (cx_a[nb], cy_a[nb])])
        print(f"Full adjacency graph: {len(fg_segs)} edges")
        if fg_segs:
            zoom = max(0.5, min(4.0, 2400.0 / width_nm))
            ax.add_collection(LineCollection(
                fg_segs,
                colors="#00ff88",
                linewidths=0.2 * zoom,
                alpha=0.5,
                zorder=3,
            ))

    # MST of centroid adjacency graph
    if args.mst and centroids and neighbors:
        print("Computing MST of centroid adjacency graph...")
        n = len(centroids)
        cx_arr = np.array([c[0] for c in centroids], dtype=np.float64)
        cy_arr = np.array([c[1] for c in centroids], dtype=np.float64)

        # Build sparse weighted adjacency: weight = Euclidean centroid distance.
        rows, cols, data = [], [], []
        for ti, nbrs in enumerate(neighbors):
            for entry in nbrs:
                nb = entry[0]
                if nb > ti:  # upper triangle only
                    dx = cx_arr[ti] - cx_arr[nb]
                    dy = cy_arr[ti] - cy_arr[nb]
                    d = (dx * dx + dy * dy) ** 0.5
                    rows.append(ti)
                    cols.append(nb)
                    data.append(d)
        mat = scipy.sparse.csr_matrix(
            (data, (rows, cols)), shape=(n, n), dtype=np.float64
        )
        mst = scipy.sparse.csgraph.minimum_spanning_tree(mat)
        mst_coo = mst.tocoo()
        print(f"  MST: {len(mst_coo.data)} edges across {n} nodes")

        mst_segs = []
        for i, j in zip(mst_coo.row, mst_coo.col):
            mst_segs.append(
                [(cx_arr[i], cy_arr[i]), (cx_arr[j], cy_arr[j])]
            )
        if mst_segs:
            zoom = max(0.5, min(4.0, 2400.0 / width_nm))
            ax.add_collection(LineCollection(
                mst_segs,
                colors="#00ff88",
                linewidths=0.25 * zoom,
                alpha=0.7,
                zorder=3,
            ))

    # Shortcut edges
    if args.shortcuts_nm > 0.0 and centroids and neighbors:
        print(f"Computing shortcut edges (max {args.shortcuts_nm} NM)...")
        land_union_nm = None
        if args.shp:
            print("  loading land union for water check...")
            land_union_nm = load_land_union_nm(Path(args.shp), args.shore_margin_nm)
        sc_pairs = compute_shortcut_edges(
            centroids, neighbors, args.shortcuts_nm, land_union_nm
        )
        if sc_pairs:
            cx_a = np.array([c[0] for c in centroids], dtype=np.float64)
            cy_a = np.array([c[1] for c in centroids], dtype=np.float64)
            sc_segs = [
                [(cx_a[i], cy_a[i]), (cx_a[j], cy_a[j])]
                for i, j in sc_pairs
            ]
            zoom = max(0.5, min(4.0, 2400.0 / width_nm))
            ax.add_collection(LineCollection(
                sc_segs,
                colors="#ff8800",
                linewidths=0.5 * zoom,
                alpha=0.9,
                zorder=7,
                label=f"shortcuts (≤{args.shortcuts_nm} NM, path>2×direct)",
            ))
            ax.legend(
                loc="lower right",
                framealpha=0.85,
                facecolor="#0a2540",
                edgecolor="white",
                labelcolor="white",
                fontsize=9,
            )

    # Portal dots
    if args.portals and neighbors:
        all_px = []
        all_py = []
        seen_portals: set[tuple[int, int]] = set()
        for ti, nbrs in enumerate(neighbors):
            for nb_idx, px, py in nbrs:
                key = (min(ti, nb_idx), max(ti, nb_idx))
                if key not in seen_portals:
                    seen_portals.add(key)
                    all_px.append(px)
                    all_py.append(py)
        if all_px:
            ax.scatter(
                all_px,
                all_py,
                s=args.centroid_size * 0.6,
                c="#ff8800",
                linewidths=0,
                zorder=5,
            )

    # Buffer contour
    if buffer_segs:
        zoom_factor = max(0.5, min(4.0, 2400.0 / width_nm))
        lw = 0.6 * zoom_factor
        buf_col = LineCollection(
            buffer_segs,
            colors="#ffff00",
            linewidths=lw,
            zorder=6,
            label=f"{args.shore_margin_nm:.3f} NM buffer",
        )
        ax.add_collection(buf_col)
        ax.legend(
            loc="lower right",
            framealpha=0.85,
            facecolor="#0a2540",
            edgecolor="white",
            labelcolor="white",
            fontsize=10,
        )

    # Port markers
    if args.ports_ron:
        ports = load_ports_ron(Path(args.ports_ron))
        dot_size = max(8.0, args.centroid_size * 6)
        font_size = max(5.0, min(11.0, 5400.0 / width_nm))
        for name, px, py in ports:
            if x_min <= px <= x_max and y_min <= py <= y_max:
                ax.scatter(px, py, s=dot_size, c="#ff4444", linewidths=0.5,
                           edgecolors="white", zorder=8)
                ax.annotate(
                    name,
                    (px, py),
                    textcoords="offset points",
                    xytext=(4, 4),
                    fontsize=font_size,
                    color="white",
                    fontweight="bold",
                    zorder=9,
                )

    ax.set_xlim(x_min, x_max)
    ax.set_ylim(y_min, y_max)
    ax.set_aspect("equal")
    ax.set_xticks([])
    ax.set_yticks([])
    for sp in ax.spines.values():
        sp.set_visible(False)
    fig.tight_layout(pad=0)

    print(f"Writing {out_path}...")
    fig.savefig(out_path, dpi=args.dpi, facecolor=bg, bbox_inches="tight")
    plt.close(fig)
    size = out_path.stat().st_size
    print(f"  wrote {size:,} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
