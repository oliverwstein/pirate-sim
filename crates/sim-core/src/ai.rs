//! AI layer: owns ship goals, translates them into heading commands.
//! Each ship has a ShipAI that decides what to do each tick.

use crate::nav::NavState;
use crate::ship::{Ship, ShipState, ShipStats};
use crate::types::{Position, WindVector};

/// AI state for a single ship.
pub struct ShipAI {
    pub nav: NavState,
}

impl ShipAI {
    pub fn new() -> Self {
        Self {
            nav: NavState::new(),
        }
    }

    pub fn with_destination(dest: Position) -> Self {
        Self {
            nav: NavState::with_destination(dest),
        }
    }

    /// Called each tick: decide what heading to set on the ship.
    /// Returns true if the ship should dock (arrived at destination).
    pub fn tick(&mut self, ship: &mut Ship, stats: &ShipStats, wind: &WindVector) -> bool {
        match ship.state {
            ShipState::Sailing => {
                if let Some(heading) = self.nav.compute_heading(ship.position, stats, wind) {
                    ship.set_heading(heading);
                    false
                } else {
                    // Arrived at destination
                    ship.dock();
                    true
                }
            }
            ShipState::Docked | ShipState::Anchored => {
                // If we have a new destination, undock and start sailing
                if self.nav.destination.is_some() {
                    ship.undock();
                    false
                } else {
                    false
                }
            }
        }
    }

    /// Give the AI a new destination (it will start sailing toward it).
    pub fn set_destination(&mut self, dest: Position) {
        self.nav.destination = Some(dest);
    }
}
