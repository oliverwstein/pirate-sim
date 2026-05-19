pub mod wind;

use wind::WindGrid;

pub struct WeatherSystem {
    pub wind: WindGrid,
}

impl WeatherSystem {
    pub fn load(data_dir: &std::path::Path) -> Self {
        let wind = WindGrid::load(&data_dir.join("grids/wind_grid.bin"));
        Self { wind }
    }
}
