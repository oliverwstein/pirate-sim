use crate::types::{Position, WindVector};

/// Minimal ship stats for Phase 1 (hardcoded sloop-like vessel).
pub struct ShipStats {
    pub speed_typical: f32,    // knots in moderate trade winds
    pub speed_max: f32,        // absolute maximum
    pub windward_ability: f32, // 0.0-1.0 (how well it sails upwind)
    pub no_go_half_angle: f32, // degrees from wind that ship cannot sail into
}

impl ShipStats {
    /// Default sloop stats.
    pub fn sloop() -> Self {
        Self {
            speed_typical: 9.0,
            speed_max: 12.0,
            windward_ability: 0.8,
            no_go_half_angle: 40.0, // can't sail closer than 40° to wind
        }
    }
}

/// Which tack the ship is on when beating upwind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tack {
    Port,      // wind coming from port (left) side
    Starboard, // wind coming from starboard (right) side
}

pub struct Ship {
    pub position: Position,
    pub heading: f32,                  // degrees (0=N, 90=E, clockwise)
    pub speed: f32,                    // current speed in knots
    pub destination: Option<Position>, // where we're trying to go
    pub tack: Tack,                    // current tack when beating
}

impl Ship {
    pub fn new(position: Position, destination: Option<Position>) -> Self {
        Self {
            position,
            heading: 0.0,
            speed: 0.0,
            destination,
            tack: Tack::Starboard,
        }
    }

    /// Calculate effective speed based on wind angle and strength.
    pub fn effective_speed(&self, stats: &ShipStats, wind: &WindVector) -> f32 {
        let wind_to = wind.direction_to();
        let relative_angle = angle_diff(self.heading, wind_to).abs();
        let efficiency = sail_efficiency(relative_angle, stats.windward_ability);
        let wind_factor = (wind.speed() / 15.0).clamp(0.3, 1.5);
        (stats.speed_typical * efficiency * wind_factor).clamp(0.5, stats.speed_max)
    }

    /// Compute speed for a hypothetical heading (used by VMG calculation).
    fn speed_at_heading(&self, heading: f32, stats: &ShipStats, wind: &WindVector) -> f32 {
        let wind_to = wind.direction_to();
        let relative_angle = angle_diff(heading, wind_to).abs();
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

    /// VMG-based heading selection: picks optimal heading toward destination
    /// considering wind. Tacks when beating upwind.
    pub fn update_heading_toward_destination(&mut self, stats: &ShipStats, wind: &WindVector) {
        let Some(dest) = self.destination else { return };

        let delta = dest - self.position;
        let bearing_to_dest = normalize_angle(delta.x.atan2(delta.y).to_degrees());
        let wind_from = wind.direction_from();

        // Angle between desired bearing and wind source
        let angle_to_wind = angle_diff(bearing_to_dest, wind_from).abs();

        if angle_to_wind > stats.no_go_half_angle + 5.0 {
            // Can sail direct — not in the no-go zone
            self.heading = bearing_to_dest;
        } else {
            // In the no-go zone: must tack. Compute VMG for port and starboard tack.
            let port_heading = normalize_angle(wind_from - stats.no_go_half_angle);
            let starboard_heading = normalize_angle(wind_from + stats.no_go_half_angle);

            let port_vmg = vmg(port_heading, bearing_to_dest, self.speed_at_heading(port_heading, stats, wind));
            let starboard_vmg = vmg(starboard_heading, bearing_to_dest, self.speed_at_heading(starboard_heading, stats, wind));

            // Hysteresis: only switch tack if the other side is >20% better VMG
            let hysteresis = 1.2;
            let new_tack = match self.tack {
                Tack::Port => {
                    if starboard_vmg > port_vmg * hysteresis {
                        Tack::Starboard
                    } else {
                        Tack::Port
                    }
                }
                Tack::Starboard => {
                    if port_vmg > starboard_vmg * hysteresis {
                        Tack::Port
                    } else {
                        Tack::Starboard
                    }
                }
            };

            self.tack = new_tack;
            self.heading = match self.tack {
                Tack::Port => port_heading,
                Tack::Starboard => starboard_heading,
            };
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

/// Velocity Made Good: component of speed toward the destination bearing.
fn vmg(heading: f32, bearing_to_dest: f32, speed: f32) -> f32 {
    let angle_off = angle_diff(heading, bearing_to_dest).abs();
    speed * angle_off.to_radians().cos()
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

/// Normalize angle to [0, 360).
fn normalize_angle(mut a: f32) -> f32 {
    while a < 0.0 {
        a += 360.0;
    }
    while a >= 360.0 {
        a -= 360.0;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_running_fast() {
        // Ship heading north (0°), wind from south (180°) = running = fast
        let ship = Ship { position: Position::ZERO, heading: 0.0, speed: 0.0, destination: None, tack: Tack::Starboard };
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 }; // blowing north, from south
        let speed = ship.effective_speed(&stats, &wind);
        assert!(speed > 10.0, "Running should be fast, got {}", speed);
    }

    #[test]
    fn test_beating_slow() {
        // Ship heading north (0°), wind from north (0°) = beating = slow
        let ship = Ship { position: Position::ZERO, heading: 0.0, speed: 0.0, destination: None, tack: Tack::Starboard };
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: -15.0 }; // blowing south, from north (0°)
        let speed = ship.effective_speed(&stats, &wind);
        assert!(speed < 5.0, "Beating should be slow, got {}", speed);
    }

    #[test]
    fn test_heading_toward_destination_direct() {
        // Wind from south, destination due east — not in no-go zone, should sail direct
        let mut ship = Ship::new(Position::ZERO, Some(Position::new(100.0, 0.0)));
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 }; // from south (180°)
        ship.update_heading_toward_destination(&stats, &wind);
        assert!((ship.heading - 90.0).abs() < 1.0, "Expected ~90°, got {}", ship.heading);
    }

    #[test]
    fn test_tacking_when_beating() {
        // Wind from north (blowing south), destination due north — must tack
        let mut ship = Ship::new(Position::ZERO, Some(Position::new(0.0, 100.0)));
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: -15.0 }; // from north (0°)
        ship.update_heading_toward_destination(&stats, &wind);
        // Should not be heading directly north (0°) — should be offset by no_go_half_angle
        let angle_from_wind = angle_diff(ship.heading, 0.0).abs();
        assert!(
            angle_from_wind >= stats.no_go_half_angle - 1.0,
            "Should tack away from wind, heading={}, angle_from_wind={}",
            ship.heading, angle_from_wind
        );
    }

    #[test]
    fn test_vmg_running() {
        // Heading directly toward destination = max VMG
        let v = vmg(90.0, 90.0, 10.0);
        assert!((v - 10.0).abs() < 0.01, "VMG should be 10 when heading matches bearing, got {}", v);
    }

    #[test]
    fn test_vmg_perpendicular() {
        // Heading perpendicular to destination = zero VMG
        let v = vmg(0.0, 90.0, 10.0);
        assert!(v.abs() < 0.01, "VMG should be ~0 when perpendicular, got {}", v);
    }
}
