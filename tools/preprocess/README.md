# Data Preprocessing Pipeline

Converts real-world geophysical datasets into compact binary grids for the simulation.

## Data Sources

### GEBCO (Bathymetry / Land Mask)
- **Source:** https://www.gebco.net/data_and_products/gridded_bathymetry_data/
- **File:** GEBCO_2024 Grid (NetCDF), subset to Caribbean
- **Resolution:** 15 arc-second (~0.5 km)
- **Download:** Use the GEBCO grid download tool, select region 5°N–30°N, 90°W–55°W

### ERA5 (Wind Climatology)
- **Source:** https://cds.climate.copernicus.eu/
- **Dataset:** ERA5 monthly averaged reanalysis on single levels
- **Variables:** 10m u-component of wind, 10m v-component of wind
- **Region:** 5°N–30°N, 90°W–55°W
- **Time:** All 12 months (climatology: average across available years)
- **Resolution:** 0.25° (~28 km)

## Directory Setup

```
raw/                    # Downloaded source data (NOT committed to git)
├── GEBCO_caribbean.nc
└── ERA5_wind_monthly.nc
```

## Running

```bash
cd tools/preprocess
pip install -r requirements.txt

# Generate land mask (produces ../../data/grids/land_mask.bin)
python preprocess_land.py --input ../../raw/GEBCO_caribbean.nc --output ../../data/grids/land_mask.bin

# Generate wind grid (produces ../../data/grids/wind_grid.bin)
python preprocess_wind.py --input ../../raw/ERA5_wind_monthly.nc --output ../../data/grids/wind_grid.bin
```

## Binary Formats

### land_mask.bin
```
Header (20 bytes):
  width:     u32 LE
  height:    u32 LE
  origin_x:  f32 LE  (NW corner X in nautical miles)
  origin_y:  f32 LE  (NW corner Y in nautical miles)
  cell_size: f32 LE  (nautical miles per cell)

Data (width × height bytes):
  u8 per cell: 0 = sea, 255 = land
  Row-major, top-to-bottom (north to south)
```

### wind_grid.bin
```
Header (24 bytes):
  width:     u32 LE
  height:    u32 LE
  origin_x:  f32 LE  (NW corner X in nautical miles)
  origin_y:  f32 LE  (NW corner Y in nautical miles)
  cell_size: f32 LE  (nautical miles per cell)
  months:    u8
  padding:   3 bytes (zeros)

Data:
  u_data: f32 LE × (months × height × width)  -- eastward wind component (knots)
  v_data: f32 LE × (months × height × width)  -- northward wind component (knots)
  Layout: month-major, then row-major (north to south)
```

## Coordinate System

Origin: 17.5°N, 72.5°W (center of Caribbean)
- X axis: East (positive)
- Y axis: North (positive)
- Units: Nautical miles
- 1° latitude = 60 NM
- 1° longitude = 60 × cos(latitude) NM
