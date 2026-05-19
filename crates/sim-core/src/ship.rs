use crate::types::{Position, WindVector};

/// Minimal ship stats for Phase 1 (hardcoded sloop-like vessel).
pub struct ShipStats {
    pub speed_typical: f32,    // knots in moderate trade winds
    pub speed_max: f32,        // absolute maximum
    pub windward_ability: f32, // 0.0-1.0 (how well it sails upwind)
}

impl ShipStats {
    /// Default sloop stats.
    pub fn sloop() -> Self {
        Self {
            speed_typical: 9.0,
            speed_max: 12.0,
            windward_ability: 0.8,
        }
    }
}

pub struct Ship {
    pub position: Position,
    pub heading: f32,                  // degrees (0=N, 90=E, clockwise)
    pub speed: f32,                    // current speed in knots
    pub destination: Option<Position>, // where we're trying to go
}

impl Ship {
    pub fn new(position: Position, destination: Option<Position>) -> Self {
        Self {
            position,
            heading: 0.0,
            speed: 0.0,
            destination,
        }
    }

    /// Calculate effective speed based on wind angle and strength.
    pub fn effective_speed(&self, stats: &ShipStats, wind: &WindVector) -> f32 {
        let wind_to = wind.direction_to();
        // Relative wind angle: 0° = wind pushing from behind (running), 180° = head-on (beating)
        // We compare our heading with the direction wind is going TO.
        // If they match, wind is behind us (running). If opposite, we're beating into it.
        let relative_angle = angle_diff(self.heading, wind_to).abs();

        let efficiency = sail_efficiency(relative_angle, stats.windward_ability);
        let wind_factor = (wind.speed() / 15.0).clamp(0.3, 1.5);

        (stats.speed_typical * efficiency * wind_factor).clamp(0.5, stats.speed_max)
    }

    /// Compute the position after dt_hours of sailing on current heading.
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

    /// Point heading toward destination (simple direct approach).
    pub fn update_heading_toward_destination(&mut self) {
        if let Some(dest) = self.destination {
            let delta = dest - self.position;
            // atan2(x, y) gives angle from north (Y-axis), clockwise
            self.heading = delta.x.atan2(delta.y).to_degrees();
            if self.heading < 0.0 {
                self.heading += 360.0;
            }
        }
    }

    /// Check if we've arrived at the destination.
    pub fn check_arrival(&mut self) -> bool {
        if let Some(dest) = self.destination {
            if self.position.distance(dest) < 5.0 {
                self.destination = None;
                self.speed = 0.0;
                return true;
            }
        }
        false
    }
}

/// Sail efficiency based on relative wind angle.
/// relative_angle: 0° = wind directly behind (running), 180° = directly into wind (beating).
fn sail_efficiency(relative_angle: f32, windward_ability: f32) -> f32 {
    let a = relative_angle.abs();
    if a < 30.0 {
        1.3 // running: wind directly behind, bonus speed
    } else if a < 60.0 {
        1.3 - (a - 30.0) / 30.0 * 0.3 // broad reach
    } else if a < 90.0 {
        1.0 // beam reach: wind perpendicular
    } else if a < 135.0 {
        // Close reach → close-hauled: windward_ability matters
        1.0 - (a - 90.0) / 45.0 * (1.0 - 0.4 * windward_ability)
    } else {
        // Beating into wind: heavily penalized
        0.1 + 0.3 * windward_ability
    }
}

/// Signed angle difference in degrees, normalized to [-180, 180].
fn angle_diff(heading: f32, wind_from: f32) -> f32 {
    let mut diff = heading - wind_from;
    while diff > 180.0 {
        diff -= 360.0;
    }
    while diff < -180.0 {
        diff += 360.0;
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_running_fast() {
        // Ship heading north (0°), wind from south (180°) = running = fast
        let ship = Ship { position: Position::ZERO, heading: 0.0, speed: 0.0, destination: None };
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 }; // blowing north, from south
        let speed = ship.effective_speed(&stats, &wind);
        assert!(speed > 10.0, "Running should be fast, got {}", speed);
    }

    #[test]
    fn test_beating_slow() {
        // Ship heading north (0°), wind from north (0°) = beating = slow
        let ship = Ship { position: Position::ZERO, heading: 0.0, speed: 0.0, destination: None };
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: -15.0 }; // blowing south, from north (0°)
        let speed = ship.effective_speed(&stats, &wind);
        assert!(speed < 5.0, "Beating should be slow, got {}", speed);
    }

    #[test]
    fn test_heading_toward_destination() {
        let mut ship = Ship::new(Position::ZERO, Some(Position::new(100.0, 0.0)));
        ship.update_heading_toward_destination();
        // Due east = heading 90°
        assert!((ship.heading - 90.0).abs() < 1.0, "Expected ~90°, got {}", ship.heading);
    }
}
