"""
Preprocess ERA5 wind data into a binary wind grid.

Usage:
    python preprocess_wind.py --input raw/ERA5_wind_monthly.nc --output data/grids/wind_grid.bin
    python preprocess_wind.py --generate-test --output data/grids/wind_grid.bin

The --generate-test flag creates a synthetic Caribbean wind field based on
known trade wind patterns, for development without needing real ERA5 data.
"""

import argparse
import math
import struct
import sys
from pathlib import Path

import numpy as np

# Coordinate system
ORIGIN_LAT = 17.5
ORIGIN_LON = -72.5

# Caribbean bounding box
LAT_MIN = -5.0
LAT_MAX = 60.0
LON_MIN = -90.0
LON_MAX = 15.0

# Output resolution
CELL_DEG = 0.25
MONTHS = 12

# Conversion factor: m/s to knots
MS_TO_KT = 1.94384


def latlon_to_nm(lat: float, lon: float) -> tuple[float, float]:
    """Convert lat/lon to nautical miles from origin (equirectangular projection)."""
    dy = (lat - ORIGIN_LAT) * 60.0
    dx = (lon - ORIGIN_LON) * 60.0  # No cos(lat) — matches grid's uniform cell size
    return (dx, dy)


def process_ncep(u_path: str, v_path: str, output_path: str):
    """Process NCEP/NCAR Reanalysis monthly long-term mean wind into binary grid."""
    import netCDF4 as nc

    print(f"Loading {u_path} and {v_path}...")
    ds_u = nc.Dataset(u_path)
    ds_v = nc.Dataset(v_path)

    lat = ds_u.variables['lat'][:]
    lon = ds_u.variables['lon'][:]
    uwnd = ds_u.variables['uwnd'][:]  # (12, 73, 144), m/s
    vwnd = ds_v.variables['vwnd'][:]

    ds_u.close()
    ds_v.close()

    print(f"Source grid: {len(lat)}x{len(lon)} at 2.5° resolution, 12 months")

    # Convert longitude from 0-360 to -180-180
    lon_shifted = np.where(lon > 180, lon - 360, lon)
    sort_idx = np.argsort(lon_shifted)
    lon_shifted = lon_shifted[sort_idx]
    uwnd = uwnd[:, :, sort_idx]
    vwnd = vwnd[:, :, sort_idx]

    # Target grid at our resolution
    target_lats = np.arange(LAT_MAX, LAT_MIN, -CELL_DEG)  # north to south
    target_lons = np.arange(LON_MIN, LON_MAX, CELL_DEG)
    height = len(target_lats)
    width = len(target_lons)

    print(f"Output grid: {width}x{height} cells at {CELL_DEG}° resolution")

    u_out = np.zeros((MONTHS, height, width), dtype=np.float32)
    v_out = np.zeros((MONTHS, height, width), dtype=np.float32)

    # Bilinear interpolation from 2.5° to target resolution
    # lat is sorted descending (90 to -90) in NCEP
    lat_sorted = lat if lat[0] > lat[-1] else lat[::-1]
    if lat[0] > lat[-1]:
        uwnd_oriented = uwnd
        vwnd_oriented = vwnd
    else:
        uwnd_oriented = uwnd[:, ::-1, :]
        vwnd_oriented = vwnd[:, ::-1, :]

    for row, tlat in enumerate(target_lats):
        # Find bounding lat indices (lat goes N to S)
        lat_idx = np.searchsorted(-lat_sorted, -tlat)
        lat_idx = min(max(lat_idx, 1), len(lat_sorted) - 1)
        lat0 = lat_idx - 1
        lat1 = lat_idx
        lat_frac = (lat_sorted[lat0] - tlat) / (lat_sorted[lat0] - lat_sorted[lat1]) if lat_sorted[lat0] != lat_sorted[lat1] else 0

        for col, tlon in enumerate(target_lons):
            lon_idx = np.searchsorted(lon_shifted, tlon)
            lon_idx = min(max(lon_idx, 1), len(lon_shifted) - 1)
            lon0 = lon_idx - 1
            lon1 = lon_idx
            lon_frac = (tlon - lon_shifted[lon0]) / (lon_shifted[lon1] - lon_shifted[lon0]) if lon_shifted[lon1] != lon_shifted[lon0] else 0

            for m in range(MONTHS):
                # Bilinear interpolation
                u00 = uwnd_oriented[m, lat0, lon0]
                u10 = uwnd_oriented[m, lat0, lon1]
                u01 = uwnd_oriented[m, lat1, lon0]
                u11 = uwnd_oriented[m, lat1, lon1]
                u_val = (u00 * (1-lon_frac) * (1-lat_frac) +
                         u10 * lon_frac * (1-lat_frac) +
                         u01 * (1-lon_frac) * lat_frac +
                         u11 * lon_frac * lat_frac)

                v00 = vwnd_oriented[m, lat0, lon0]
                v10 = vwnd_oriented[m, lat0, lon1]
                v01 = vwnd_oriented[m, lat1, lon0]
                v11 = vwnd_oriented[m, lat1, lon1]
                v_val = (v00 * (1-lon_frac) * (1-lat_frac) +
                         v10 * lon_frac * (1-lat_frac) +
                         v01 * (1-lon_frac) * lat_frac +
                         v11 * lon_frac * lat_frac)

                u_out[m, row, col] = u_val * MS_TO_KT
                v_out[m, row, col] = v_val * MS_TO_KT

    nw_x, nw_y = latlon_to_nm(LAT_MAX, LON_MIN)
    cell_size_nm = CELL_DEG * 60.0

    write_binary(output_path, width, height, nw_x, nw_y, cell_size_nm, u_out, v_out)

    # Sample report
    mid_row, mid_col = height // 2, width // 2
    for m in [0, 6]:
        su, sv = u_out[m, mid_row, mid_col], v_out[m, mid_row, mid_col]
        speed = math.sqrt(su**2 + sv**2)
        print(f"  Month {m+1} center: u={su:.1f} v={sv:.1f} speed={speed:.1f} kt")


def process_era5(input_path: str, output_path: str):
    """Process real ERA5 NetCDF into binary wind grid."""
    import netCDF4 as nc

    ds = nc.Dataset(input_path)

    # ERA5 variables: 'latitude', 'longitude', 'time', 'u10', 'v10'
    lat = ds.variables['latitude'][:]
    lon = ds.variables['longitude'][:]
    u10 = ds.variables['u10'][:]  # shape: (time, lat, lon), m/s
    v10 = ds.variables['v10'][:]

    # Subset to Caribbean
    lat_mask = (lat >= LAT_MIN) & (lat <= LAT_MAX)
    lon_mask = (lon >= LON_MIN) & (lon <= LON_MAX)

    lat_sub = lat[lat_mask]
    lon_sub = lon[lon_mask]
    u_sub = u10[:, lat_mask, :][:, :, lon_mask]
    v_sub = v10[:, lat_mask, :][:, :, lon_mask]

    # Average by month (ERA5 monthly means should already be 12 time steps)
    n_times = u_sub.shape[0]
    if n_times == 12:
        u_monthly = u_sub
        v_monthly = v_sub
    else:
        # Group by month if we have multiple years
        u_monthly = np.zeros((12, u_sub.shape[1], u_sub.shape[2]))
        v_monthly = np.zeros((12, v_sub.shape[1], v_sub.shape[2]))
        for m in range(12):
            mask = np.arange(n_times) % 12 == m
            u_monthly[m] = u_sub[mask].mean(axis=0)
            v_monthly[m] = v_sub[mask].mean(axis=0)

    # Resample to target resolution
    target_lats = np.arange(LAT_MAX, LAT_MIN, -CELL_DEG)
    target_lons = np.arange(LON_MIN, LON_MAX, CELL_DEG)
    height = len(target_lats)
    width = len(target_lons)

    print(f"Output grid: {width}x{height} cells × {MONTHS} months")

    u_out = np.zeros((MONTHS, height, width), dtype=np.float32)
    v_out = np.zeros((MONTHS, height, width), dtype=np.float32)

    for row, tlat in enumerate(target_lats):
        lat_idx = np.argmin(np.abs(lat_sub - tlat))
        for col, tlon in enumerate(target_lons):
            lon_idx = np.argmin(np.abs(lon_sub - tlon))
            for m in range(MONTHS):
                u_out[m, row, col] = u_monthly[m, lat_idx, lon_idx] * MS_TO_KT
                v_out[m, row, col] = v_monthly[m, lat_idx, lon_idx] * MS_TO_KT

    ds.close()

    nw_x, nw_y = latlon_to_nm(LAT_MAX, LON_MIN)
    cell_size_nm = CELL_DEG * 60.0

    write_binary(output_path, width, height, nw_x, nw_y, cell_size_nm, u_out, v_out)


def generate_test_wind(output_path: str):
    """
    Generate synthetic Caribbean trade wind field.

    Based on research:
    - Trade winds: ENE (from ~070°) at 15-20 knots
    - Stronger in winter (Dec-Mar), lighter in summer (Jun-Sep)
    - Stronger at lower latitudes, weaker near land
    - ITCZ causes lighter/variable winds near 5-10°N in summer
    """
    target_lats = np.arange(LAT_MAX, LAT_MIN, -CELL_DEG)
    target_lons = np.arange(LON_MIN, LON_MAX, CELL_DEG)
    height = len(target_lats)
    width = len(target_lons)

    print(f"Generating test wind grid: {width}x{height} × {MONTHS} months")

    u_out = np.zeros((MONTHS, height, width), dtype=np.float32)
    v_out = np.zeros((MONTHS, height, width), dtype=np.float32)

    for month in range(MONTHS):
        # Seasonal strength factor: stronger in winter
        # Peak in Jan-Feb, weakest in Aug-Sep
        seasonal_factor = 1.0 + 0.25 * math.cos(2.0 * math.pi * (month - 1) / 12.0)

        for row, lat in enumerate(target_lats):
            for col, lon in enumerate(target_lons):
                # Base trade wind: from ENE (070°), speed 15 kt
                # "From 070°" means wind blows toward 250° (WSW)
                base_speed = 15.0 * seasonal_factor

                # Latitude modulation:
                # - Strongest 10-20°N (trade wind belt)
                # - Weaker above 25°N (horse latitudes)
                # - Weaker below 8°N in summer (ITCZ)
                if lat > 25.0:
                    lat_factor = 1.0 - (lat - 25.0) / 5.0 * 0.5
                elif lat < 10.0:
                    # ITCZ weakening in summer months
                    itcz_factor = 0.3 if 5 <= month <= 9 else 0.7
                    lat_factor = itcz_factor + (lat - 5.0) / 5.0 * (1.0 - itcz_factor)
                else:
                    lat_factor = 1.0

                speed = base_speed * max(lat_factor, 0.1)

                # Trade wind direction: from ENE (070°)
                # Wind blowing TO direction = 070 + 180 = 250° (WSW)
                # In meteorological u/v:
                #   u = wind speed * sin(to_direction_rad)  (eastward component)
                #   v = wind speed * cos(to_direction_rad)  (northward component)
                # "To 250°" means mostly westward with slight southward
                wind_to_deg = 250.0

                # Add some latitude variation: more westerly at higher latitudes
                if lat > 20.0:
                    wind_to_deg = 250.0 + (lat - 20.0) * 2.0  # trending more W/NW

                wind_to_rad = math.radians(wind_to_deg)
                u_out[month, row, col] = speed * math.sin(wind_to_rad)
                v_out[month, row, col] = speed * math.cos(wind_to_rad)

    nw_x, nw_y = latlon_to_nm(LAT_MAX, LON_MIN)
    cell_size_nm = CELL_DEG * 60.0

    write_binary(output_path, width, height, nw_x, nw_y, cell_size_nm, u_out, v_out)

    # Report sample values
    mid_row = height // 2
    mid_col = width // 2
    sample_u = u_out[0, mid_row, mid_col]
    sample_v = v_out[0, mid_row, mid_col]
    sample_speed = math.sqrt(sample_u**2 + sample_v**2)
    print(f"Sample wind at center (Jan): u={sample_u:.1f}, v={sample_v:.1f}, speed={sample_speed:.1f} kt")


def write_binary(output_path: str, width: int, height: int,
                 origin_x: float, origin_y: float, cell_size: float,
                 u_data: np.ndarray, v_data: np.ndarray):
    """Write the binary wind grid file."""
    Path(output_path).parent.mkdir(parents=True, exist_ok=True)

    with open(output_path, 'wb') as f:
        # Header: 24 bytes
        f.write(struct.pack('<I', width))
        f.write(struct.pack('<I', height))
        f.write(struct.pack('<f', origin_x))
        f.write(struct.pack('<f', origin_y))
        f.write(struct.pack('<f', cell_size))
        f.write(struct.pack('<B', MONTHS))
        f.write(b'\x00' * 3)  # padding

        # Data: u then v, both as flat f32 arrays
        f.write(u_data.astype(np.float32).tobytes())
        f.write(v_data.astype(np.float32).tobytes())

    file_size = Path(output_path).stat().st_size
    print(f"Written: {output_path} ({file_size:,} bytes)")


def main():
    parser = argparse.ArgumentParser(description="Preprocess wind data")
    parser.add_argument('--input', help='Path to ERA5 NetCDF file')
    parser.add_argument('--ncep-u', help='Path to NCEP u-wind NetCDF (uwnd.mon.ltm.nc)')
    parser.add_argument('--ncep-v', help='Path to NCEP v-wind NetCDF (vwnd.mon.ltm.nc)')
    parser.add_argument('--output', required=True, help='Output binary file path')
    parser.add_argument('--generate-test', action='store_true',
                        help='Generate synthetic test wind data')
    args = parser.parse_args()

    if args.generate_test:
        generate_test_wind(args.output)
    elif args.ncep_u and args.ncep_v:
        process_ncep(args.ncep_u, args.ncep_v, args.output)
    elif args.input:
        process_era5(args.input, args.output)
    else:
        print("Error: provide --input (ERA5), --ncep-u + --ncep-v, or --generate-test",
              file=sys.stderr)
        sys.exit(1)


if __name__ == '__main__':
    main()
