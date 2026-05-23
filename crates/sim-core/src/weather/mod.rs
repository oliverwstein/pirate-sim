pub mod hazards;
pub mod wind;

use hazards::HazardSystem;
use wind::WindGrid;

pub struct WeatherSystem {
    pub wind: WindGrid,
    pub hazards: HazardSystem,
}

impl WeatherSystem {
    pub fn load(data_dir: &std::path::Path) -> Self {
        let wind = WindGrid::load(&data_dir.join("grids/wind_grid.bin"));
        Self {
            wind,
            // Seed is arbitrary but stable; bench reproducibility is
            // anchored at the `World::seed_historical_fleet(seed)` call.
            hazards: HazardSystem::new(0x4252_4545_5A45_5300),
        }
    }
}
