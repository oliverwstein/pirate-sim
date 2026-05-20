use std::path::Path;

use crate::map::MapSystem;
use crate::port::{Port, all_ports};
use crate::ship::{Ship, ShipStats};
use crate::types::SimDate;
use crate::weather::WeatherSystem;

pub struct World {
    pub map: MapSystem,
    pub weather: WeatherSystem,
    pub ports: Vec<Port>,
    pub ships: Vec<Ship>,
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
            date: SimDate::new(1680, 0, 1), // January 1, 1680
        }
    }

    /// Advance the simulation by one hour.
    pub fn tick(&mut self) {
        let stats = ShipStats::sloop();

        for ship in &mut self.ships {
            if ship.destination.is_none() {
                continue;
            }

            // Point at destination
            ship.update_heading_toward_destination();

            // Get wind at ship's position
            let wind = self.weather.wind.wind_at(ship.position, self.date.month());

            // Compute proposed new position
            let new_pos = ship.compute_next_position(&stats, &wind, 1.0);

            // Land collision check
            if !self.map.land.is_land(new_pos) {
                ship.position = new_pos;
                ship.speed = ship.effective_speed(&stats, &wind);
            } else {
                ship.speed = 0.0;
                // TODO: pathfinding around obstacles
            }

            // Arrival check
            ship.check_arrival();
        }

        self.date.advance_hours(1);
    }
}
