use std::path::Path;

use crate::ai::ShipAI;
use crate::map::MapSystem;
use crate::port::{Port, all_ports};
use crate::ship::{Ship, ShipState, ShipStats};
use crate::types::SimDate;
use crate::weather::WeatherSystem;

pub struct World {
    pub map: MapSystem,
    pub weather: WeatherSystem,
    pub ports: Vec<Port>,
    pub ships: Vec<Ship>,
    pub ship_ais: Vec<ShipAI>,
    pub date: SimDate,
}

impl World {
    pub fn load(data_dir: &Path) -> Self {
        let map = MapSystem::load(data_dir);
        let weather = WeatherSystem::load(data_dir);

        Self {
            map,
            weather,
            ports: all_ports(),
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

        for i in 0..self.ships.len() {
            let wind = self.weather.wind.wind_at(self.ships[i].position, self.date.month());

            // AI decides heading (or docks/undocks)
            self.ship_ais[i].tick(&mut self.ships[i], &stats, &wind);

            if self.ships[i].state != ShipState::Sailing {
                continue;
            }

            // Physics: compute movement
            let new_pos = self.ships[i].compute_next_position(&stats, &wind, 1.0);

            // Land collision check
            if !self.map.land.is_land(new_pos) {
                self.ships[i].position = new_pos;
                self.ships[i].speed = self.ships[i].effective_speed(&stats, &wind);
            } else {
                self.ships[i].speed = 0.0;
            }
        }

        self.date.advance_hours(1);
    }
}
