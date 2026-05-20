//! Navigation utilities: VMG calculation and optimal heading computation.
//! Used by the AI layer to translate goals into heading commands.

use crate::ship::{angle_diff, normalize_angle, speed_at_heading, ShipStats};
use crate::types::{Position, WindVector};

/// Which tack the navigator is currently on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tack {
    Port,
    Starboard,
}

/// Navigation state for a ship (owned by AI, not by Ship itself).
pub struct NavState {
    pub destination: Option<Position>,
    pub tack: Tack,
}

impl NavState {
    pub fn new() -> Self {
        Self {
            destination: None,
            tack: Tack::Starboard,
        }
    }

    pub fn with_destination(dest: Position) -> Self {
        Self {
            destination: Some(dest),
            tack: Tack::Starboard,
        }
    }

    /// Compute the optimal heading given current position, wind, and destination.
    /// Returns None if no destination is set or already arrived.
    pub fn compute_heading(
        &mut self,
        pos: Position,
        stats: &ShipStats,
        wind: &WindVector,
    ) -> Option<f32> {
        let dest = self.destination?;

        // Check arrival (10 NM threshold — accounts for tacking overshoot)
        if pos.distance(dest) < 10.0 {
            self.destination = None;
            return None;
        }

        let delta = dest - pos;
        let bearing_to_dest = normalize_angle(delta.x.atan2(delta.y).to_degrees());
        let wind_from = wind.direction_from();

        let angle_to_wind = angle_diff(bearing_to_dest, wind_from).abs();

        if angle_to_wind > stats.no_go_half_angle + 5.0 {
            // Can sail direct
            Some(bearing_to_dest)
        } else {
            // Must tack: compute VMG for each side
            let port_heading = normalize_angle(wind_from - stats.no_go_half_angle);
            let starboard_heading = normalize_angle(wind_from + stats.no_go_half_angle);

            let port_vmg = vmg(port_heading, bearing_to_dest, speed_at_heading(port_heading, stats, wind));
            let starboard_vmg = vmg(starboard_heading, bearing_to_dest, speed_at_heading(starboard_heading, stats, wind));

            // Hysteresis: only switch tack if other side is >20% better
            let hysteresis = 1.2;
            self.tack = match self.tack {
                Tack::Port => {
                    if starboard_vmg > port_vmg * hysteresis { Tack::Starboard } else { Tack::Port }
                }
                Tack::Starboard => {
                    if port_vmg > starboard_vmg * hysteresis { Tack::Port } else { Tack::Starboard }
                }
            };

            Some(match self.tack {
                Tack::Port => port_heading,
                Tack::Starboard => starboard_heading,
            })
        }
    }
}

/// Velocity Made Good: component of speed toward the destination bearing.
pub fn vmg(heading: f32, bearing_to_dest: f32, speed: f32) -> f32 {
    let angle_off = angle_diff(heading, bearing_to_dest).abs();
    speed * angle_off.to_radians().cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direct_heading() {
        let mut nav = NavState::with_destination(Position::new(100.0, 0.0));
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 }; // from south
        let heading = nav.compute_heading(Position::ZERO, &stats, &wind).unwrap();
        assert!((heading - 90.0).abs() < 1.0);
    }

    #[test]
    fn test_tacking_heading() {
        let mut nav = NavState::with_destination(Position::new(0.0, 100.0));
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: -15.0 }; // from north
        let heading = nav.compute_heading(Position::ZERO, &stats, &wind).unwrap();
        let angle_from_wind = angle_diff(heading, 0.0).abs();
        assert!(angle_from_wind >= stats.no_go_half_angle - 1.0);
    }

    #[test]
    fn test_arrival_clears_destination() {
        let mut nav = NavState::with_destination(Position::new(3.0, 0.0));
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 };
        let result = nav.compute_heading(Position::ZERO, &stats, &wind);
        assert!(result.is_none()); // within 5 NM = arrived
        assert!(nav.destination.is_none());
    }

    #[test]
    fn test_vmg_calculation() {
        assert!((vmg(90.0, 90.0, 10.0) - 10.0).abs() < 0.01);
        assert!(vmg(0.0, 90.0, 10.0).abs() < 0.01);
    }
}
