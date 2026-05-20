use crate::types::{Position, WindVector};

/// Ship performance characteristics.
pub struct ShipStats {
    pub speed_typical: f32,    // knots in moderate trade winds
    pub speed_max: f32,        // absolute maximum
    pub windward_ability: f32, // 0.0-1.0 (how well it sails upwind)
    pub no_go_half_angle: f32, // degrees from wind that ship cannot sail into
}

impl ShipStats {
    pub fn sloop() -> Self {
        Self {
            speed_typical: 9.0,
            speed_max: 12.0,
            windward_ability: 0.8,
            no_go_half_angle: 40.0,
        }
    }
}

/// The physical state of a ship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipState {
    Sailing,
    Docked,
    Anchored,
}

/// A ship: purely physical entity. Heading is set externally by AI/player.
pub struct Ship {
    pub position: Position,
    pub heading: f32,     // degrees (0=N, 90=E, clockwise)
    pub speed: f32,       // current speed in knots
    pub state: ShipState,
}

impl Ship {
    pub fn new(position: Position, state: ShipState) -> Self {
        Self {
            position,
            heading: 0.0,
            speed: 0.0,
            state,
        }
    }

    /// Set heading (the primary control input from AI/player).
    pub fn set_heading(&mut self, heading: f32) {
        self.heading = heading;
    }

    /// Transition to sailing state.
    pub fn undock(&mut self) {
        self.state = ShipState::Sailing;
    }

    /// Dock at current position.
    pub fn dock(&mut self) {
        self.state = ShipState::Docked;
        self.speed = 0.0;
    }

    /// Anchor at current position.
    pub fn anchor(&mut self) {
        self.state = ShipState::Anchored;
        self.speed = 0.0;
    }

    /// Calculate effective speed based on current heading, wind, and stats.
    pub fn effective_speed(&self, stats: &ShipStats, wind: &WindVector) -> f32 {
        speed_at_heading(self.heading, stats, wind)
    }

    /// Advance position by one time step. Returns new position (doesn't apply it).
    pub fn compute_next_position(
        &self,
        stats: &ShipStats,
        wind: &WindVector,
        dt_hours: f32,
    ) -> Position {
        let speed = self.effective_speed(stats, wind);
        let distance_nm = speed * dt_hours;
        let rad = self.heading.to_radians();
        let dx = distance_nm * rad.sin();
        let dy = distance_nm * rad.cos();
        self.position + Position::new(dx, dy)
    }
}

/// Calculate speed for a given heading (public utility for AI/nav).
pub fn speed_at_heading(heading: f32, stats: &ShipStats, wind: &WindVector) -> f32 {
    let wind_to = wind.direction_to();
    let relative_angle = angle_diff(heading, wind_to).abs();
    let efficiency = sail_efficiency(relative_angle, stats.windward_ability);
    let wind_factor = (wind.speed() / 15.0).clamp(0.3, 1.5);
    (stats.speed_typical * efficiency * wind_factor).clamp(0.5, stats.speed_max)
}

/// Sail efficiency based on relative wind angle.
fn sail_efficiency(relative_angle: f32, windward_ability: f32) -> f32 {
    let a = relative_angle.abs();
    if a < 30.0 {
        1.3
    } else if a < 60.0 {
        1.3 - (a - 30.0) / 30.0 * 0.3
    } else if a < 90.0 {
        1.0
    } else if a < 135.0 {
        1.0 - (a - 90.0) / 45.0 * (1.0 - 0.4 * windward_ability)
    } else {
        0.1 + 0.3 * windward_ability
    }
}

/// Signed angle difference in degrees, normalized to [-180, 180].
pub fn angle_diff(a: f32, b: f32) -> f32 {
    let mut diff = a - b;
    while diff > 180.0 { diff -= 360.0; }
    while diff < -180.0 { diff += 360.0; }
    diff
}

/// Normalize angle to [0, 360).
pub fn normalize_angle(mut a: f32) -> f32 {
    while a < 0.0 { a += 360.0; }
    while a >= 360.0 { a -= 360.0; }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_running_fast() {
        let ship = Ship { position: Position::ZERO, heading: 0.0, speed: 0.0, state: ShipState::Sailing };
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 };
        assert!(ship.effective_speed(&stats, &wind) > 10.0);
    }

    #[test]
    fn test_beating_slow() {
        let ship = Ship { position: Position::ZERO, heading: 0.0, speed: 0.0, state: ShipState::Sailing };
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: -15.0 };
        assert!(ship.effective_speed(&stats, &wind) < 5.0);
    }

    #[test]
    fn test_state_transitions() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        assert_eq!(ship.state, ShipState::Docked);

        ship.undock();
        assert_eq!(ship.state, ShipState::Sailing);

        ship.anchor();
        assert_eq!(ship.state, ShipState::Anchored);
        assert_eq!(ship.speed, 0.0);

        ship.undock();
        ship.dock();
        assert_eq!(ship.state, ShipState::Docked);
    }
}
