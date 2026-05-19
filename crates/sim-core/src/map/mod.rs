pub mod land;

use land::LandMap;

pub struct MapSystem {
    pub land: LandMap,
}

impl MapSystem {
    pub fn load(data_dir: &std::path::Path) -> Self {
        let land = LandMap::load(&data_dir.join("grids/land_mask.bin"));
        Self { land }
    }
}
