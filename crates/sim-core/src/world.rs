use std::path::Path;

use crate::ai::ShipAI;
use crate::harbor::HarborMap;
use crate::map::MapSystem;
use crate::pathfind::PathfindContext;
use crate::port::{Port, all_ports};
use crate::ship::{Ship, ShipState, ShipStats};
use crate::types::SimDate;
use crate::weather::WeatherSystem;

pub struct World {
    pub map: MapSystem,
    pub weather: WeatherSystem,
    pub ports: Vec<Port>,
    pub harbors: HarborMap,
    pub ships: Vec<Ship>,
    pub ship_ais: Vec<ShipAI>,
    pub date: SimDate,
}

impl World {
    pub fn load(data_dir: &Path) -> Self {
        let map = MapSystem::load(data_dir);
        let weather = WeatherSystem::load(data_dir);
        let ports = all_ports();
        let harbors = HarborMap::build(&map.land, &ports);

        Self {
            map,
            weather,
            ports,
            harbors,
            ships: Vec::new(),
            ship_ais: Vec::new(),
            date: SimDate::new(1680, 0, 1),
        }
    }

    /// Add a ship with its AI controller.
    pub fn add_ship(&mut self, ship: Ship, ai: ShipAI) {
        self.ships.push(ship);
        self.ship_ais.push(ai);
    }

    /// Advance the simulation by one hour.
    pub fn tick(&mut self) {
        let stats = ShipStats::sloop();
        let month = self.date.month();
        let pathfind = PathfindContext::new(&self.map.land, &self.weather.wind, &stats, month);

        for i in 0..self.ships.len() {
            let wind = self.weather.wind.wind_at(self.ships[i].position, month);

            // AI decides heading (or docks/undocks)
            self.ship_ais[i].tick(
                &mut self.ships[i],
                &stats,
                &wind,
                &self.ports,
                &self.harbors,
                Some(&pathfind),
            );

            // Resource consumption
            self.ships[i].tick_resources(&stats);

            if self.ships[i].state != ShipState::Sailing {
                continue;
            }

            // Physics: compute movement, swept against land so a single
            // tick never tunnels through a coastline.
            //
            // Rescue: a ship may legitimately be inside a land cell — e.g.,
            // it just undocked from a port whose literal coordinates fall
            // on land at our 1 NM/cell resolution. From inside land,
            // `farthest_clear_point` would otherwise refuse all motion and
            // strand the ship. Snap to the nearest sea cell first.
            if self.map.land.is_land(self.ships[i].position) {
                if let Some(cell) = self.map.land.pos_to_cell(self.ships[i].position) {
                    if let Some(sea) = self.map.land.nearest_sea_cell(cell.0, cell.1, 32) {
                        self.ships[i].position = self.map.land.cell_to_pos(sea.0, sea.1);
                    }
                }
            }

            let new_pos = self.ships[i].compute_next_position(&stats, &wind, 1.0);
            let old_pos = self.ships[i].position;
            let safe_pos = self.map.land.farthest_clear_point(old_pos, new_pos);

            if safe_pos.distance(old_pos) > 0.05 {
                self.ships[i].position = safe_pos;
                // Speed reflects how far we actually traveled.
                let traveled = safe_pos.distance(old_pos);
                self.ships[i].speed = traveled; // 1 hour tick → NM == kt
            } else {
                self.ships[i].speed = 0.0;
            }
        }

        self.date.advance_hours(1);
    }
}
