# Phase 1: Time and Space

## Goal
Prove the physical foundation: ships moving through real Caribbean geography affected by real wind patterns.

**What you see:** A macroquad window showing the Caribbean coastline (from GEBCO), wind arrows (from ERA5), and ship triangles sailing from A to B.

**What it proves:** The map module, weather module, and movement physics all work together. Everything else builds on this.

## Success Criteria
1. A ship placed near Barbados with destination Jamaica arrives in ~6 days (trade winds)
2. The same trip in reverse takes ~14 days (beating upwind)
3. Ships cannot sail through islands
4. Wind visibly varies by month (stronger in winter, lighter in summer)
5. The visualization renders at 60fps with pan/zoom

---

## Scope (What's IN)

- Preprocessing pipeline (Python → binary grids)
- Land mask from GEBCO bathymetry
- Wind grid from ERA5 monthly climatology
- Ship movement with wind physics
- Land collision (ships can't cross land)
- macroquad visualization (land, wind arrows, ships)
- Camera controls (pan, zoom)
- Time controls (pause, play, speed up)

## Scope (What's NOT in Phase 1)

- No settlements, factions, sea zones
- No cargo, goods, trading
- No combat or AI
- No RON data files (just hardcoded test ship)
- No Rhai scripting
- No save/load

---

## Project Structure

```
pirate-sim/
├── Cargo.toml                   (workspace)
├── crates/
│   ├── sim-core/                (pure library — zero rendering deps)
│   │   ├── Cargo.toml           (glam, bytemuck, serde)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs         (Position, WindVector, SimDate)
│   │       ├── world.rs         (World struct, tick)
│   │       ├── map/
│   │       │   ├── mod.rs       (MapSystem)
│   │       │   └── land.rs      (LandMap: load, is_land, depth_at)
│   │       ├── weather/
│   │       │   ├── mod.rs       (WeatherSystem)
│   │       │   └── wind.rs      (WindGrid: load, wind_at)
│   │       └── ship.rs          (Ship, movement physics)
│   └── sim-viz/                 (macroquad visualizer)
│       ├── Cargo.toml           (macroquad, sim-core)
│       └── src/
│           └── main.rs          (render loop, camera, HUD)
├── tools/
│   └── preprocess/              (Python: real data → binary grids)
│       ├── requirements.txt     (netCDF4, numpy)
│       ├── preprocess_land.py   (GEBCO → land_mask.bin)
│       ├── preprocess_wind.py   (ERA5 → wind_grid.bin)
│       └── README.md
└── data/
    └── grids/
        ├── land_mask.bin        (output of preprocessing)
        └── wind_grid.bin        (output of preprocessing)
```

---

## Implementation Order

### 1. Workspace Setup
Create the Cargo workspace with sim-core and sim-viz crates. Verify they build.

### 2. Core Types (sim-core/types.rs)

```rust
pub type Position = glam::Vec2;  // nautical miles from origin (17.5°N, 72.5°W)

pub struct WindVector {
    pub u: f32,  // east component (knots)
    pub v: f32,  // north component (knots)
}

impl WindVector {
    pub fn speed(&self) -> f32 { (self.u * self.u + self.v * self.v).sqrt() }
    pub fn direction(&self) -> f32 { self.u.atan2(self.v).to_degrees() }  // meteorological convention
}

pub struct SimDate {
    pub year: u16,
    pub day_of_year: u16,  // 1-365
    pub hour: u8,          // 0-23
}

impl SimDate {
    pub fn month(&self) -> u8 { /* 0-11 from day_of_year */ }
    pub fn advance_hours(&mut self, n: u32) { /* wrap day/year */ }
}
```

### 3. Map Module (sim-core/map/)

```rust
/// Binary format: [width: u32][height: u32][origin_x: f32][origin_y: f32][cell_size: f32][data: u8×(w×h)]
/// Data: 0 = sea, 255 = land (Phase 1 only needs binary; depth bands later)
pub struct LandMap {
    data: Vec<u8>,
    width: u32,
    height: u32,
    origin: Position,      // NW corner in NM
    cell_size_nm: f32,     // NM per cell
}

impl LandMap {
    pub fn load(path: &Path) -> Self { /* read binary header + data */ }
    pub fn is_land(&self, pos: Position) -> bool { /* grid lookup */ }
}

pub struct MapSystem {
    pub land: LandMap,
}
```

### 4. Weather Module (sim-core/weather/)

```rust
/// Binary format: [width: u32][height: u32][origin_x: f32][origin_y: f32][cell_size: f32][months: u8]
///                [u_data: f32×(months×h×w)][v_data: f32×(months×h×w)]
pub struct WindGrid {
    u: Vec<f32>,
    v: Vec<f32>,
    width: u32,
    height: u32,
    origin: Position,
    cell_size_nm: f32,
    months: u8,
}

impl WindGrid {
    pub fn load(path: &Path) -> Self { /* read binary */ }
    
    /// Bilinear interpolation of wind at any position for a given month
    pub fn wind_at(&self, pos: Position, month: u8) -> WindVector {
        // Convert pos to grid coordinates
        // Interpolate from 4 nearest grid points
    }
}

pub struct WeatherSystem {
    pub wind: WindGrid,
}
```

### 5. Ship Movement (sim-core/ship.rs)

```rust
pub struct Ship {
    pub position: Position,
    pub heading: f32,        // degrees (0=N, 90=E)
    pub speed: f32,          // current speed in knots
    pub destination: Option<Position>,
}

// Ship type stats (hardcoded for Phase 1 — just one type: "sloop")
pub struct ShipStats {
    pub speed_typical: f32,     // 9.0 kt
    pub speed_max: f32,         // 12.0 kt
    pub windward_ability: f32,  // 0.8
}

impl Ship {
    /// Compute effective speed based on wind angle
    pub fn effective_speed(&self, stats: &ShipStats, wind: &WindVector) -> f32 {
        let wind_dir = wind.direction();
        let relative_angle = angle_diff(self.heading, wind_dir).abs();
        
        // Sail efficiency curve:
        // Running (0°): 1.3x  (wind behind)
        // Beam reach (90°): 1.0x  (wind abeam)
        // Close-hauled (135°): 0.5x × windward_ability
        // In irons (180°): 0.1x × windward_ability
        let efficiency = sail_efficiency(relative_angle, stats.windward_ability);
        
        let wind_factor = (wind.speed() / 15.0).clamp(0.3, 1.5);
        (stats.speed_typical * efficiency * wind_factor).clamp(0.5, stats.speed_max)
    }
    
    /// Compute new position after dt hours of sailing
    pub fn compute_next_position(&self, stats: &ShipStats, wind: &WindVector, dt_hours: f32) -> Position {
        let speed = self.effective_speed(stats, wind);
        let distance_nm = speed * dt_hours;
        let dx = distance_nm * self.heading.to_radians().sin();
        let dy = distance_nm * self.heading.to_radians().cos();
        self.position + Position::new(dx, dy)
    }
}

fn sail_efficiency(relative_wind_angle: f32, windward_ability: f32) -> f32 {
    // Piecewise: running bonus → beam reach → close-hauled penalty
    match relative_wind_angle {
        a if a < 30.0 => 1.3,                                    // running
        a if a < 60.0 => 1.3 - (a - 30.0) / 30.0 * 0.3,        // broad reach
        a if a < 90.0 => 1.0,                                    // beam reach
        a if a < 135.0 => 1.0 - (a - 90.0) / 45.0 * (1.0 - 0.4 * windward_ability), // close reach
        _ => 0.1 + 0.3 * windward_ability,                       // beating
    }
}
```

### 6. World & Tick (sim-core/world.rs)

```rust
pub struct World {
    pub map: MapSystem,
    pub weather: WeatherSystem,
    pub ships: Vec<Ship>,
    pub date: SimDate,
}

impl World {
    pub fn load(data_dir: &Path) -> Self {
        let map = MapSystem { land: LandMap::load(&data_dir.join("grids/land_mask.bin")) };
        let weather = WeatherSystem { wind: WindGrid::load(&data_dir.join("grids/wind_grid.bin")) };
        World { map, weather, ships: vec![], date: SimDate { year: 1680, day_of_year: 1, hour: 0 } }
    }
    
    pub fn tick(&mut self) {
        let stats = ShipStats { speed_typical: 9.0, speed_max: 12.0, windward_ability: 0.8 };
        
        for ship in &mut self.ships {
            if ship.destination.is_none() { continue; }
            
            // Point at destination
            let dest = ship.destination.unwrap();
            ship.heading = (dest - ship.position).to_angle().to_degrees();
            
            // Get wind, compute movement
            let wind = self.weather.wind.wind_at(ship.position, self.date.month());
            let new_pos = ship.compute_next_position(&stats, &wind, 1.0);
            
            // Land collision check
            if !self.map.land.is_land(new_pos) {
                ship.position = new_pos;
                ship.speed = ship.effective_speed(&stats, &wind);
            } else {
                ship.speed = 0.0;
                // TODO: route around obstacle
            }
            
            // Arrival check
            if ship.position.distance(dest) < 5.0 {
                ship.destination = None;
                ship.speed = 0.0;
            }
        }
        self.date.advance_hours(1);
    }
}
```

### 7. Preprocessing Pipeline (tools/preprocess/)

**preprocess_land.py:**
- Input: GEBCO NetCDF (bathymetry for 5°N–30°N, 90°W–55°W)
- Process: threshold at 0m → binary land/sea mask
- Output: `land_mask.bin` with header + u8 array
- Resolution: 0.25° (~15 NM/cell, or ~28km)

**preprocess_wind.py:**
- Input: ERA5 monthly mean 10m wind (u,v components), same region
- Process: select 12 months, convert m/s → knots (×1.944)
- Output: `wind_grid.bin` with header + f32 arrays
- Resolution: 0.25° grid, 12 months

**Coordinate conversion in both scripts:**
```python
ORIGIN_LAT = 17.5  # degrees N
ORIGIN_LON = -72.5  # degrees W (negative = west)

def latlon_to_nm(lat, lon):
    """Convert lat/lon to nautical miles from origin."""
    dy = (lat - ORIGIN_LAT) * 60.0  # 1° lat = 60 NM
    dx = (lon - ORIGIN_LON) * 60.0 * math.cos(math.radians(lat))
    return (dx, dy)
```

### 8. Visualization (sim-viz/main.rs)

```rust
use macroquad::prelude::*;
use sim_core::World;

#[macroquad::main("Pirate Sim - Phase 1")]
async fn main() {
    let mut world = World::load(Path::new("data/"));
    
    // Spawn test ship: Barbados → Jamaica
    world.ships.push(Ship {
        position: Position::new(780.0, -140.0),  // Barbados approx
        heading: 0.0,
        speed: 0.0,
        destination: Some(Position::new(-462.0, 50.0)),  // Port Royal approx
    });
    
    let mut cam_offset = Vec2::ZERO;
    let mut zoom: f32 = 0.3;  // NM to pixels
    let mut paused = false;
    let mut ticks_per_frame: u32 = 1;
    
    loop {
        // --- Input ---
        if is_key_pressed(KeyCode::Space) { paused = !paused; }
        if is_key_pressed(KeyCode::Right) { ticks_per_frame = (ticks_per_frame * 2).min(48); }
        if is_key_pressed(KeyCode::Left) { ticks_per_frame = (ticks_per_frame / 2).max(1); }
        // Pan with WASD, zoom with scroll
        handle_camera(&mut cam_offset, &mut zoom);
        
        // --- Tick ---
        if !paused {
            for _ in 0..ticks_per_frame {
                world.tick();
            }
        }
        
        // --- Render ---
        clear_background(Color::from_rgba(20, 50, 90, 255));
        
        // Land (iterate visible cells, draw filled rects for land)
        draw_land(&world.map.land, cam_offset, zoom);
        
        // Wind arrows (sample every N cells, draw direction+magnitude)
        draw_wind(&world.weather.wind, world.date.month(), cam_offset, zoom);
        
        // Ships
        for ship in &world.ships {
            let sp = world_to_screen(ship.position, cam_offset, zoom);
            draw_triangle_rotated(sp, ship.heading, 8.0, YELLOW);
        }
        
        // HUD
        draw_text(&format!("{} Day {} {:02}:00 | {}x | {}",
            world.date.year, world.date.day_of_year, world.date.hour,
            ticks_per_frame, if paused {"⏸"} else {"▶"}),
            10.0, 20.0, 20.0, WHITE);
        
        next_frame().await;
    }
}
```

---

## Implementation Sequence

1. `cargo init` workspace + both crates (verify builds)
2. Types + SimDate (trivial, just get the foundation compiling)
3. Preprocessing scripts (need data before Rust can load it)
4. Map module (load land_mask.bin, implement is_land)
5. Weather module (load wind_grid.bin, implement wind_at with interpolation)
6. Ship + World tick (movement physics, land collision)
7. sim-viz (render land, wind, ships, camera, time controls)
8. Tuning: verify Barbados→Jamaica ~6 days, reverse ~14 days
