"""
Preprocess GEBCO bathymetry data into a binary land mask.

Usage:
    python preprocess_land.py --input raw/GEBCO_caribbean.nc --output data/grids/land_mask.bin
    python preprocess_land.py --generate-test --output data/grids/land_mask.bin

The --generate-test flag creates a synthetic Caribbean land mask for development
without needing to download the real GEBCO dataset.
"""

import argparse
import math
import struct
import sys
from pathlib import Path

import numpy as np

# Coordinate system
ORIGIN_LAT = 17.5   # degrees N
ORIGIN_LON = -72.5  # degrees E (negative = west)

# Caribbean bounding box
LAT_MIN = -5.0
LAT_MAX = 60.0
LON_MIN = -90.0
LON_MAX = 15.0

# Output resolution: 0.25° per cell (matches ERA5 wind grid)
CELL_DEG = 0.25


def latlon_to_nm(lat: float, lon: float) -> tuple[float, float]:
    """Convert lat/lon to nautical miles from origin (equirectangular projection)."""
    dy = (lat - ORIGIN_LAT) * 60.0
    dx = (lon - ORIGIN_LON) * 60.0  # No cos(lat) — matches grid's uniform cell size
    return (dx, dy)


def process_gebco(input_path: str, output_path: str):
    """Process real GEBCO NetCDF into binary land mask."""
    import netCDF4 as nc

    print(f"Loading {input_path}...")
    ds = nc.Dataset(input_path)

    # GEBCO variables: 'lat', 'lon', 'elevation'
    lat = ds.variables['lat'][:]
    lon = ds.variables['lon'][:]

    # Target grid at CELL_DEG resolution
    target_lats = np.arange(LAT_MAX, LAT_MIN, -CELL_DEG)  # north to south
    target_lons = np.arange(LON_MIN, LON_MAX, CELL_DEG)    # west to east

    height = len(target_lats)
    width = len(target_lons)

    print(f"Output grid: {width}x{height} cells at {CELL_DEG}° resolution")
    print(f"Source grid: {len(lat)}x{len(lon)} (GEBCO 15 arc-sec)")

    land_mask = np.zeros((height, width), dtype=np.uint8)

    # For each target cell, find the nearest source cell and check elevation
    # Use searchsorted for efficiency (lat/lon arrays are sorted)
    for row, tlat in enumerate(target_lats):
        if row % 20 == 0:
            print(f"  Processing row {row}/{height} (lat={tlat:.1f}°)...")
        lat_idx = np.searchsorted(lat, tlat)
        lat_idx = min(max(lat_idx, 0), len(lat) - 1)

        # Read this entire row from the source to avoid repeated disk access
        elev_row = ds.variables['elevation'][lat_idx, :]

        for col, tlon in enumerate(target_lons):
            lon_idx = np.searchsorted(lon, tlon)
            lon_idx = min(max(lon_idx, 0), len(lon) - 1)
            if elev_row[lon_idx] >= 0:
                land_mask[row, col] = 255

    ds.close()

    # Compute NW corner in NM
    nw_x, nw_y = latlon_to_nm(LAT_MAX, LON_MIN)
    cell_size_nm = CELL_DEG * 60.0  # approximate (varies with latitude, but close enough)

    write_binary(output_path, width, height, nw_x, nw_y, cell_size_nm, land_mask)


def generate_test_land(output_path: str):
    """Generate a synthetic Caribbean land mask for testing."""
    target_lats = np.arange(LAT_MAX, LAT_MIN, -CELL_DEG)  # north to south
    target_lons = np.arange(LON_MIN, LON_MAX, CELL_DEG)    # west to east

    height = len(target_lats)
    width = len(target_lons)

    print(f"Generating test land mask: {width}x{height} cells")

    land_mask = np.zeros((height, width), dtype=np.uint8)

    # Approximate major Caribbean landmasses as simple shapes
    islands = [
        # Cuba: roughly 20-23°N, 84-74°W
        (19.5, 23.5, -85.0, -74.0),
        # Jamaica: roughly 17.5-18.5°N, 78.5-76°W
        (17.5, 18.5, -78.5, -76.0),
        # Hispaniola: roughly 18-20°N, 74.5-68.5°W
        (18.0, 20.0, -74.5, -68.5),
        # Puerto Rico: roughly 17.9-18.5°N, 67.3-65.5°W
        (17.9, 18.5, -67.3, -65.5),
        # Trinidad: roughly 10-10.8°N, 61.9-60.5°W
        (10.0, 10.8, -61.9, -60.5),
        # Lesser Antilles chain (simplified as small blocks)
        (15.5, 16.5, -62.0, -61.0),  # Guadeloupe
        (14.0, 14.8, -61.2, -60.8),  # Martinique
        (12.9, 13.3, -59.8, -59.4),  # Barbados
        (11.9, 12.3, -61.9, -61.5),  # Grenada
        # Central America coast (simplified)
        (8.0, 18.0, -90.0, -83.0),   # Central America
        # South America north coast
        (5.0, 12.0, -76.0, -60.0),   # Venezuela/Colombia coast (simplified)
        # Florida
        (24.5, 30.0, -87.5, -80.0),
        # Bahamas (scattered)
        (23.0, 27.0, -79.5, -73.0),
    ]

    for lat_min, lat_max, lon_min, lon_max in islands:
        for row, tlat in enumerate(target_lats):
            if lat_min <= tlat <= lat_max:
                for col, tlon in enumerate(target_lons):
                    if lon_min <= tlon <= lon_max:
                        land_mask[row, col] = 255

    # Make South America/Venezuela coast more realistic (only coastal strip)
    # Override: make interior of SA more solid, clear open ocean
    # The simple rectangles above are good enough for Phase 1 testing

    nw_x, nw_y = latlon_to_nm(LAT_MAX, LON_MIN)
    cell_size_nm = CELL_DEG * 60.0

    write_binary(output_path, width, height, nw_x, nw_y, cell_size_nm, land_mask)
    print(f"Land cells: {np.sum(land_mask == 255)} / {width * height} total")


def write_binary(output_path: str, width: int, height: int,
                 origin_x: float, origin_y: float, cell_size: float,
                 data: np.ndarray):
    """Write the binary land mask file."""
    Path(output_path).parent.mkdir(parents=True, exist_ok=True)

    with open(output_path, 'wb') as f:
        # Header: 20 bytes
        f.write(struct.pack('<I', width))
        f.write(struct.pack('<I', height))
        f.write(struct.pack('<f', origin_x))
        f.write(struct.pack('<f', origin_y))
        f.write(struct.pack('<f', cell_size))
        # Data
        f.write(data.tobytes())

    file_size = Path(output_path).stat().st_size
    print(f"Written: {output_path} ({file_size:,} bytes)")


def main():
    parser = argparse.ArgumentParser(description="Preprocess GEBCO data to land mask")
    parser.add_argument('--input', help='Path to GEBCO NetCDF file')
    parser.add_argument('--output', required=True, help='Output binary file path')
    parser.add_argument('--generate-test', action='store_true',
                        help='Generate synthetic test data instead of processing real data')
    args = parser.parse_args()

    if args.generate_test:
        generate_test_land(args.output)
    elif args.input:
        process_gebco(args.input, args.output)
    else:
        print("Error: either --input or --generate-test is required", file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
