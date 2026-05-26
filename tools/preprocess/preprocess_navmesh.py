"""
Preprocess the navigable sea region into a convex-tile navmesh.

Produces a tiling of the sea (world bounds minus buffered land) where:
  * every tile is convex;
  * adjacent tiles are connected via a "portal" — the exact midpoint of
    their shared edge — so the Rust pathfinder routes
    Centroid_A → Portal_AB → Centroid_B, guaranteeing no land crossing
    even on merged "L-shaped" tiles.

Pipeline:
    1. Load Natural Earth land polygons.
    2. Project to EPSG:3857 (Web Mercator) for meter-accurate geometry.
    3. Buffer outward by shore_margin (meters) with round joins.
    4. Simplify with Douglas-Peucker in meters.
    5. Project back to WGS-84 lon/lat.
    6. Sea polygon = world box − buffered land.
    7. Project sea to NM-from-origin coordinates.
    8. Constrained Delaunay triangulation (with Steiner anchors).
    9. Hertel–Mehlhorn convex merge.
    10. Emit tiles + centroids + adjacency + portals.

Binary format (little-endian):
    u32 num_tiles
    for each tile:
        u32 num_vertices
        for each vertex: f32 x, f32 y    -- CCW, NM-from-origin
        f32 centroid_x, f32 centroid_y
        u32 num_neighbors
        for each neighbor:
            u32 tile_index
            f32 portal_x, f32 portal_y   -- midpoint of shared edge

Usage:
    python preprocess_navmesh.py \\
        --input  data/raw/ne_10m_land/ne_10m_land.shp \\
        --output data/grids/navmesh.bin
"""

import argparse
import math
import struct
import sys
from collections import defaultdict
from pathlib import Path
from typing import Iterable

import numpy as np
import pyproj
import shapefile  # pyshp
import triangle as tr
from shapely.geometry import MultiPolygon, MultiPoint, Point, Polygon, box
from shapely.geometry import shape as shapely_shape
from shapely.ops import transform, unary_union

# ---------------------------------------------------------------------------
# Coordinate system constants
# ---------------------------------------------------------------------------

ORIGIN_LAT = 17.5
ORIGIN_LON = -72.5
LAT_MIN = -5.0
LAT_MAX = 60.0
LON_MIN = -90.0
LON_MAX = 15.0

# EPSG:4326 → EPSG:3857 (Web Mercator) projectors — reused across calls.
_PROJ_TO_MERC = pyproj.Transformer.from_crs(
    "EPSG:4326", "EPSG:3857", always_xy=True
)
_PROJ_TO_WGS84 = pyproj.Transformer.from_crs(
    "EPSG:3857", "EPSG:4326", always_xy=True
)


def to_merc(geom):
    """Project a WGS-84 (lon/lat) Shapely geometry into Web Mercator (m)."""
    return transform(_PROJ_TO_MERC.transform, geom)


def to_wgs84(geom):
    """Project a Web Mercator Shapely geometry back to WGS-84 (lon/lat)."""
    return transform(_PROJ_TO_WGS84.transform, geom)


def latlon_to_nm(lat: float, lon: float) -> tuple[float, float]:
    dx = (lon - ORIGIN_LON) * 60.0
    dy = (lat - ORIGIN_LAT) * 60.0
    return dx, dy


def project_polygon(poly: Polygon) -> Polygon:
    """Project a lon/lat polygon (with holes) to NM-from-origin coordinates."""

    def proj_ring(ring) -> list[tuple[float, float]]:
        return [latlon_to_nm(lat, lon) for lon, lat in ring]

    outer = proj_ring(list(poly.exterior.coords))
    holes = [proj_ring(list(r.coords)) for r in poly.interiors]
    return Polygon(outer, holes)


# ---------------------------------------------------------------------------
# Shapefile loading
# ---------------------------------------------------------------------------


def shapefile_polygons(path: Path) -> Iterable[Polygon]:
    """Yield Shapely Polygons from a WGS-84 shapefile."""
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


# ---------------------------------------------------------------------------
# Sea polygon construction
# ---------------------------------------------------------------------------


def load_sea_polygon(
    in_path: Path,
    simplify_nm: float = 0.0,
    shore_margin_nm: float = 0.0,
) -> Polygon | MultiPolygon:
    """Sea = world box − buffered-land.

    Buffering and simplification are performed in Web Mercator (meters) so
    distances are physically accurate at all latitudes.
    """
    print(f"Loading land polygons from {in_path}...")
    clip_latlon = box(LON_MIN, LAT_MIN, LON_MAX, LAT_MAX)
    land_pieces: list[Polygon] = []
    for poly in shapefile_polygons(in_path):
        clipped = poly.intersection(clip_latlon)
        if clipped.is_empty:
            continue
        if isinstance(clipped, MultiPolygon):
            land_pieces.extend(g for g in clipped.geoms if not g.is_empty)
        elif isinstance(clipped, Polygon):
            land_pieces.append(clipped)

    print(f"  {len(land_pieces)} land pieces in bbox; unioning...")
    land_union = unary_union(land_pieces)

    if shore_margin_nm > 0.0:
        margin_m = shore_margin_nm * 1852.0
        # Project to Web Mercator for isotropic meter-based buffer.
        land_merc = to_merc(land_union)
        before = sum(
            len(p.exterior.coords)
            for p in (land_merc.geoms if isinstance(land_merc, MultiPolygon) else [land_merc])
        )
        land_merc = land_merc.buffer(margin_m, join_style=1, resolution=4)
        after = sum(
            len(p.exterior.coords)
            for p in (land_merc.geoms if isinstance(land_merc, MultiPolygon) else [land_merc])
        )
        land_union = to_wgs84(land_merc)
        print(
            f"  shore margin {shore_margin_nm:.3f} NM ({margin_m:.0f} m, EPSG:3857): "
            f"exterior verts {before} → {after}"
        )

    if simplify_nm > 0.0:
        simplify_m = simplify_nm * 1852.0
        land_merc = to_merc(land_union)
        before = sum(
            len(p.exterior.coords)
            for p in (land_merc.geoms if isinstance(land_merc, MultiPolygon) else [land_merc])
        )
        land_merc = land_merc.simplify(simplify_m, preserve_topology=True)
        after = sum(
            len(p.exterior.coords)
            for p in (land_merc.geoms if isinstance(land_merc, MultiPolygon) else [land_merc])
        )
        land_union = to_wgs84(land_merc)
        print(
            f"  simplify({simplify_nm:.3f} NM = {simplify_m:.0f} m): "
            f"exterior verts {before} → {after}"
        )

    x_min, y_min = latlon_to_nm(LAT_MIN, LON_MIN)
    x_max, y_max = latlon_to_nm(LAT_MAX, LON_MAX)

    world_box_latlon = box(LON_MIN, LAT_MIN, LON_MAX, LAT_MAX)
    sea_latlon = world_box_latlon.difference(land_union)

    print("  Projecting sea polygon to NM...")
    if isinstance(sea_latlon, Polygon):
        sea_nm = project_polygon(sea_latlon)
    elif isinstance(sea_latlon, MultiPolygon):
        sea_nm = MultiPolygon([project_polygon(p) for p in sea_latlon.geoms])
    else:
        raise RuntimeError(f"Unexpected sea geometry type: {type(sea_latlon)}")

    print(
        f"  Sea bounds (NM): x={x_min:.1f}..{x_max:.1f}, "
        f"y={y_min:.1f}..{y_max:.1f}"
    )
    return sea_nm


# ---------------------------------------------------------------------------
# Constrained Delaunay triangulation
# ---------------------------------------------------------------------------


def cdt_sea(
    sea: Polygon | MultiPolygon,
    steiner_spacing_nm: float,
) -> tuple[np.ndarray, np.ndarray]:
    """CDT of the NM-coordinate sea polygon.

    Injects a regular Steiner grid at `steiner_spacing_nm` to bound
    open-ocean triangle size. Uses a vectorised MultiPoint intersection
    to avoid O(N) per-point contains checks.

    Returns (vertices [N×2], triangles [T×3]) CCW.
    """
    pieces: list[Polygon] = (
        [sea] if isinstance(sea, Polygon) else list(sea.geoms)
    )

    vert_index: dict[tuple[int, int], int] = {}
    verts: list[tuple[float, float]] = []
    segs: list[tuple[int, int]] = []
    holes: list[tuple[float, float]] = []

    DEDUP_SCALE = 10000.0

    def add_vertex(p: tuple[float, float]) -> int:
        key = (round(p[0] * DEDUP_SCALE), round(p[1] * DEDUP_SCALE))
        if key in vert_index:
            return vert_index[key]
        vert_index[key] = len(verts)
        verts.append((float(p[0]), float(p[1])))
        return vert_index[key]

    def add_ring(coords: list[tuple[float, float]]) -> None:
        if len(coords) > 1 and coords[0] == coords[-1]:
            coords = coords[:-1]
        if len(coords) < 3:
            return
        max_seg = max(steiner_spacing_nm, 1.0)
        n = len(coords)
        densified: list[tuple[float, float]] = []
        for i in range(n):
            a = coords[i]
            b = coords[(i + 1) % n]
            densified.append(a)
            dx = b[0] - a[0]
            dy = b[1] - a[1]
            seg_len = (dx * dx + dy * dy) ** 0.5
            if seg_len > max_seg:
                steps = int(seg_len // max_seg)
                for k in range(1, steps + 1):
                    t = k / (steps + 1)
                    densified.append((a[0] + dx * t, a[1] + dy * t))
        coords = densified
        nd = len(coords)
        idxs = [add_vertex(p) for p in coords]
        for i in range(nd):
            a = idxs[i]
            b = idxs[(i + 1) % nd]
            if a != b:
                segs.append((a, b))

    for piece in pieces:
        add_ring(list(piece.exterior.coords))
        for ring in piece.interiors:
            add_ring(list(ring.coords))
            hole_poly = Polygon(list(ring.coords))
            rp = hole_poly.representative_point()
            holes.append((float(rp.x), float(rp.y)))

    # --- Steiner anchor injection (vectorised) ---
    if steiner_spacing_nm > 0.0:
        bounds = sea.bounds
        x0, y0, x1, y1 = bounds
        half = steiner_spacing_nm / 2.0
        xs = np.arange(x0 + half, x1, steiner_spacing_nm)
        ys = np.arange(y0 + half, y1, steiner_spacing_nm)
        # Build a MultiPoint and intersect once — vastly faster than per-point
        # contains() on a complex MultiPolygon.
        grid_mp = MultiPoint([(float(x), float(y)) for x in xs for y in ys])
        valid = sea.intersection(grid_mp)
        added = 0
        if not valid.is_empty:
            pts = [valid] if valid.geom_type == "Point" else list(valid.geoms)
            for pt in pts:
                add_vertex((pt.x, pt.y))
                added += 1
        print(f"  Steiner: added {added} anchor points at {steiner_spacing_nm} NM spacing")

    print(
        f"  CDT input: {len(verts)} verts, {len(segs)} segments, "
        f"{len(holes)} holes"
    )

    pslg: dict = {
        "vertices": np.asarray(verts, dtype=np.float64),
        "segments": np.asarray(segs, dtype=np.int32),
    }
    if holes:
        pslg["holes"] = np.asarray(holes, dtype=np.float64)

    out = tr.triangulate(pslg, "p")
    out_verts = np.asarray(out["vertices"], dtype=np.float64)
    out_tris = np.asarray(out["triangles"], dtype=np.int32)
    print(f"  CDT output: {len(out_verts)} verts, {len(out_tris)} triangles")

    out_tris = ensure_ccw(out_verts, out_tris)
    return out_verts, out_tris


def ensure_ccw(verts: np.ndarray, tris: np.ndarray) -> np.ndarray:
    out = tris.copy()
    for i, (a, b, c) in enumerate(tris):
        va, vb, vc = verts[a], verts[b], verts[c]
        cz = (vb[0] - va[0]) * (vc[1] - va[1]) - (vb[1] - va[1]) * (vc[0] - va[0])
        if cz < 0:
            out[i] = [a, c, b]
    return out


# ---------------------------------------------------------------------------
# Hertel–Mehlhorn convex partition
# ---------------------------------------------------------------------------


def cross_z(o: np.ndarray, a: np.ndarray, b: np.ndarray) -> float:
    return float((a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]))


def is_convex_at(verts: np.ndarray, prev_i: int, curr_i: int, next_i: int) -> bool:
    return cross_z(verts[prev_i], verts[curr_i], verts[next_i]) >= -1e-9


def merge_tiles_via_edge(
    tile_a: list[int],
    tile_b: list[int],
    edge: tuple[int, int],
) -> list[int]:
    u, v = edge
    ia = -1
    n_a = len(tile_a)
    for i in range(n_a):
        if tile_a[i] == u and tile_a[(i + 1) % n_a] == v:
            ia = i
            break
    if ia < 0:
        raise RuntimeError("edge (u,v) not found in tile_a")
    ib = -1
    n_b = len(tile_b)
    for j in range(n_b):
        if tile_b[j] == v and tile_b[(j + 1) % n_b] == u:
            ib = j
            break
    if ib < 0:
        raise RuntimeError("edge (v,u) not found in tile_b")
    out: list[int] = []
    for k in range(n_a):
        out.append(tile_a[(ia + 1 + k) % n_a])
    for k in range(n_b - 2):
        out.append(tile_b[(ib + 2 + k) % n_b])
    return out


def hertel_mehlhorn(
    verts: np.ndarray,
    tris: np.ndarray,
    max_tile_diameter_nm: float = float("inf"),
) -> list[list[int]]:
    """Greedily merge triangles into convex tiles (Hertel–Mehlhorn)."""
    max_d_sq = max_tile_diameter_nm * max_tile_diameter_nm

    def diameter_sq(tile: list[int]) -> float:
        pts = verts[tile]
        d_max = 0.0
        for i in range(len(pts)):
            for j in range(i + 1, len(pts)):
                dx = pts[i, 0] - pts[j, 0]
                dy = pts[i, 1] - pts[j, 1]
                d = dx * dx + dy * dy
                if d > d_max:
                    d_max = d
        return float(d_max)

    tiles: list[list[int] | None] = [list(t) for t in tris]
    edge_map: dict[tuple[int, int], int] = {}
    for ti, tile in enumerate(tiles):
        nt = len(tile)
        for i in range(nt):
            edge_map[(tile[i], tile[(i + 1) % nt])] = ti

    def collect_candidates() -> list[tuple[int, int]]:
        out = []
        for (u, v), _ in edge_map.items():
            if u < v and (v, u) in edge_map:
                out.append((u, v))
        return out

    def can_merge(tile_a: list[int], tile_b: list[int], edge: tuple[int, int]) -> bool:
        u, v = edge
        n_a = len(tile_a)
        ia = -1
        for i in range(n_a):
            if tile_a[i] == u and tile_a[(i + 1) % n_a] == v:
                ia = i
                break
        n_b = len(tile_b)
        ib = -1
        for j in range(n_b):
            if tile_b[j] == v and tile_b[(j + 1) % n_b] == u:
                ib = j
                break
        if ia < 0 or ib < 0:
            return False
        p_a = tile_a[(ia - 1) % n_a]
        s_b = tile_b[(ib + 2) % n_b]
        if not is_convex_at(verts, p_a, u, s_b):
            return False
        s_a = tile_a[(ia + 2) % n_a]
        p_b = tile_b[(ib - 1) % n_b]
        if not is_convex_at(verts, p_b, v, s_a):
            return False
        if max_d_sq < float("inf"):
            merged = merge_tiles_via_edge(tile_a, tile_b, (u, v))
            if diameter_sq(merged) > max_d_sq:
                return False
        return True

    def do_merge(ti_a: int, ti_b: int, edge: tuple[int, int]) -> None:
        u, v = edge
        tile_a = tiles[ti_a]
        tile_b = tiles[ti_b]
        merged = merge_tiles_via_edge(tile_a, tile_b, (u, v))
        for tile_idx, tile in ((ti_a, tile_a), (ti_b, tile_b)):
            nt = len(tile)
            for i in range(nt):
                e = (tile[i], tile[(i + 1) % nt])
                if edge_map.get(e) == tile_idx:
                    del edge_map[e]
        tiles[ti_a] = merged
        tiles[ti_b] = None
        nm = len(merged)
        for i in range(nm):
            edge_map[(merged[i], merged[(i + 1) % nm])] = ti_a

    pass_no = 0
    while True:
        pass_no += 1
        merged_count = 0
        candidates = collect_candidates()
        for u, v in candidates:
            ta = edge_map.get((u, v))
            tb = edge_map.get((v, u))
            if ta is None or tb is None or ta == tb:
                continue
            tile_a = tiles[ta]
            tile_b = tiles[tb]
            if tile_a is None or tile_b is None:
                continue
            if can_merge(tile_a, tile_b, (u, v)):
                do_merge(ta, tb, (u, v))
                merged_count += 1
        print(f"  Hertel–Mehlhorn pass {pass_no}: {merged_count} merges")
        if merged_count == 0:
            break

    return [t for t in tiles if t is not None]


# ---------------------------------------------------------------------------
# Centroid + adjacency (with portals)
# ---------------------------------------------------------------------------


def compute_centroid(verts: np.ndarray, tile: list[int]) -> tuple[float, float]:
    """Area-weighted centroid of a convex polygon (CCW vertex list)."""
    n = len(tile)
    if n < 3:
        pts = verts[tile]
        return float(pts[:, 0].mean()), float(pts[:, 1].mean())
    a_acc = 0.0
    cx = 0.0
    cy = 0.0
    v0 = verts[tile[0]]
    for i in range(1, n - 1):
        v1 = verts[tile[i]]
        v2 = verts[tile[i + 1]]
        ax = v1[0] - v0[0]
        ay = v1[1] - v0[1]
        bx = v2[0] - v0[0]
        by = v2[1] - v0[1]
        a = 0.5 * (ax * by - ay * bx)
        a_acc += a
        cx += a * (v0[0] + v1[0] + v2[0]) / 3.0
        cy += a * (v0[1] + v1[1] + v2[1]) / 3.0
    if abs(a_acc) < 1e-12:
        pts = verts[tile]
        return float(pts[:, 0].mean()), float(pts[:, 1].mean())
    return cx / a_acc, cy / a_acc


# Each neighbor entry: (neighbor_tile_index, portal_x, portal_y)
Neighbor = tuple[int, float, float]


def build_adjacency_with_portals(
    verts: np.ndarray,
    tiles: list[list[int]],
) -> list[list[Neighbor]]:
    """Build tile adjacency carrying the portal (shared-edge midpoint).

    Two tiles are neighbors iff they share an undirected edge (u, v).
    The portal is the exact midpoint of that edge in NM coordinates.
    Routing via portal avoids the Centroid LOS Fallacy on "L-shaped"
    merged tiles.
    """
    edge_to_tile: dict[tuple[int, int], int] = {}
    # Dict per tile: neighbor_index → (portal_x, portal_y).
    # Using a dict ensures no duplicate entries for degenerate shared edges.
    nb_map: list[dict[int, tuple[float, float]]] = [{} for _ in tiles]

    for ti, tile in enumerate(tiles):
        n = len(tile)
        for i in range(n):
            a, b = tile[i], tile[(i + 1) % n]
            key = (a, b) if a < b else (b, a)
            other = edge_to_tile.get(key)
            if other is not None and other != ti:
                px = (verts[a][0] + verts[b][0]) / 2.0
                py = (verts[a][1] + verts[b][1]) / 2.0
                nb_map[ti][other] = (px, py)
                nb_map[other][ti] = (px, py)
            else:
                edge_to_tile[key] = ti

    return [
        sorted(
            [(nb, px, py) for nb, (px, py) in nbs.items()],
            key=lambda x: x[0],
        )
        for nbs in nb_map
    ]


def keep_largest_component(
    tiles: list[list[int]],
    neighbors: list[list[Neighbor]],
) -> tuple[list[list[int]], list[list[Neighbor]], list[int]]:
    """Keep only tiles in the largest connected component."""
    seen = [False] * len(tiles)
    best: list[int] = []
    for start in range(len(tiles)):
        if seen[start]:
            continue
        stack = [start]
        comp: list[int] = []
        while stack:
            t = stack.pop()
            if seen[t]:
                continue
            seen[t] = True
            comp.append(t)
            for nb, _px, _py in neighbors[t]:
                if not seen[nb]:
                    stack.append(nb)
        if len(comp) > len(best):
            best = comp
    keep = sorted(best)
    remap = {old: new for new, old in enumerate(keep)}
    new_tiles = [tiles[i] for i in keep]
    new_neighbors: list[list[Neighbor]] = [
        sorted(
            [(remap[nb], px, py) for nb, px, py in neighbors[i] if nb in remap],
            key=lambda x: x[0],
        )
        for i in keep
    ]
    return new_tiles, new_neighbors, keep


# ---------------------------------------------------------------------------
# Binary output
# ---------------------------------------------------------------------------


def write_navmesh(
    out_path: Path,
    verts: np.ndarray,
    tiles: list[list[int]],
    centroids: list[tuple[float, float]],
    neighbors: list[list[Neighbor]],
) -> None:
    """Write navmesh.bin in the portal-aware format.

    Format (little-endian):
        u32  num_tiles
        per tile:
            u32  num_vertices
            (f32 x, f32 y) × num_vertices     -- CCW, NM-from-origin
            f32  centroid_x, f32 centroid_y
            u32  num_neighbors
            (u32 tile_index, f32 portal_x, f32 portal_y) × num_neighbors
    """
    with open(out_path, "wb") as f:
        f.write(struct.pack("<I", len(tiles)))
        for tile, (cx, cy), nbrs in zip(tiles, centroids, neighbors, strict=True):
            f.write(struct.pack("<I", len(tile)))
            for vi in tile:
                x, y = verts[vi]
                f.write(struct.pack("<ff", float(x), float(y)))
            f.write(struct.pack("<ff", float(cx), float(cy)))
            f.write(struct.pack("<I", len(nbrs)))
            for nb_idx, px, py in nbrs:
                f.write(struct.pack("<Iff", nb_idx, float(px), float(py)))


# ---------------------------------------------------------------------------
# Stats / reporting
# ---------------------------------------------------------------------------


def report_stats(
    tiles: list[list[int]],
    centroids: list[tuple[float, float]],
    neighbors: list[list[Neighbor]],
) -> None:
    vert_counts = np.array([len(t) for t in tiles])
    nbr_counts = np.array([len(n) for n in neighbors])
    print(f"  Tiles:           {len(tiles)}")
    print(
        f"  Vertices/tile:   min={vert_counts.min()} "
        f"avg={vert_counts.mean():.1f} max={vert_counts.max()}"
    )
    print(
        f"  Neighbors/tile:  min={nbr_counts.min()} "
        f"avg={nbr_counts.mean():.1f} max={nbr_counts.max()}"
    )
    isolated = int((nbr_counts == 0).sum())
    if isolated:
        print(f"  WARNING: {isolated} isolated tiles (no neighbors)")
    seen = [False] * len(tiles)
    comp_sizes = []
    for start in range(len(tiles)):
        if seen[start]:
            continue
        stack = [start]
        size = 0
        while stack:
            t = stack.pop()
            if seen[t]:
                continue
            seen[t] = True
            size += 1
            for nb, _px, _py in neighbors[t]:
                if not seen[nb]:
                    stack.append(nb)
        comp_sizes.append(size)
    comp_sizes.sort(reverse=True)
    print(
        f"  Components: {len(comp_sizes)} "
        f"(largest {comp_sizes[0]}, "
        f"next: {comp_sizes[1:6] if len(comp_sizes) > 1 else '—'})"
    )


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--input", required=True, help="ne_10m_land.shp")
    ap.add_argument("--output", required=True, help="navmesh.bin")
    ap.add_argument(
        "--no-merge",
        action="store_true",
        help="Skip Hertel-Mehlhorn (debug; output is raw CDT triangles)",
    )
    ap.add_argument(
        "--steiner-spacing-nm",
        type=float,
        default=50.0,
        help="Steiner anchor grid spacing (NM). Bounds open-ocean tile size. Default 50.",
    )
    ap.add_argument(
        "--max-tile-diameter-nm",
        type=float,
        default=50.0,
        help="Reject merges that would yield a tile diameter > this (NM). Default 50.",
    )
    ap.add_argument(
        "--keep-all-components",
        action="store_true",
        help="Don't filter to the largest connected component (debug/viz).",
    )
    ap.add_argument(
        "--simplify-nm",
        type=float,
        default=0.0,
        help="Douglas-Peucker simplification tolerance in NM (applied in EPSG:3857 meters). 0 disables.",
    )
    ap.add_argument(
        "--shore-margin-nm",
        type=float,
        default=0.25,
        help="Buffer land outward by this distance (NM) in EPSG:3857 before CDT. "
        "0.25 NM keeps narrow channels open while preventing coastal wedging. "
        "Default 0.25.",
    )
    args = ap.parse_args()

    in_path = Path(args.input)
    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    sea = load_sea_polygon(
        in_path,
        simplify_nm=args.simplify_nm,
        shore_margin_nm=args.shore_margin_nm,
    )
    print("Triangulating sea (CDT)...")
    verts, tris = cdt_sea(sea, args.steiner_spacing_nm)

    if args.no_merge:
        print("(--no-merge) skipping convex partition; emitting raw CDT")
        tiles = [list(t) for t in tris]
    else:
        print(
            f"Running Hertel–Mehlhorn convex partition "
            f"(max tile diameter {args.max_tile_diameter_nm} NM)..."
        )
        tiles = hertel_mehlhorn(verts, tris, args.max_tile_diameter_nm)

    print("Computing centroids + adjacency (with portals)...")
    centroids = [compute_centroid(verts, t) for t in tiles]
    neighbors = build_adjacency_with_portals(verts, tiles)

    print("Stats (all components):")
    report_stats(tiles, centroids, neighbors)

    if not args.keep_all_components:
        print("Filtering to largest connected component...")
        tiles, neighbors, kept = keep_largest_component(tiles, neighbors)
        centroids = [centroids[i] for i in kept]
        print(f"  kept {len(tiles)} tiles in the navigable sea")

        print("Stats (largest component only):")
        report_stats(tiles, centroids, neighbors)
    else:
        print("(--keep-all-components) skipping largest-component filter")

    print(f"Writing {out_path}...")
    write_navmesh(out_path, verts, tiles, centroids, neighbors)
    size = out_path.stat().st_size
    print(f"  wrote {size:,} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
