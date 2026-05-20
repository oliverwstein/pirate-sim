//! Navigation utilities: VMG calculation, tacking, waypoint following, and
//! reactive land deflection. Used by the AI layer to translate goals into
//! heading commands.

use std::collections::VecDeque;

use crate::map::land::LandMap;
use crate::ship::{angle_diff, normalize_angle, speed_at_heading, ShipStats};
use crate::types::{Position, WindVector};

/// Which tack the navigator is currently on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tack {
    Port,
    Starboard,
}

/// Distance (NM) at which a waypoint is considered "reached".
const WAYPOINT_REACHED_NM: f32 = 6.0;

/// Distance (NM) at which the final destination (or its sea-anchor) is
/// considered "arrived". A few cell widths so that swept-collision near a
/// coastal port doesn't keep the ship just short of the threshold forever.
const ARRIVAL_NM: f32 = 12.0;

/// How far ahead the reactive deflection probe looks, in NM. Sized so that
/// an entire one-hour tick of travel fits comfortably (max sloop speed ≈ 12 kt).
const DEFLECT_LOOKAHEAD_NM: f32 = 14.0;

/// Navigation state for a ship (owned by AI, not by Ship itself).
pub struct NavState {
    /// Final goal — once cleared, the ship has arrived.
    pub destination: Option<Position>,
    /// Ordered intermediate waypoints (front = next target). The final
    /// element should equal `destination` when a path was planned.
    pub waypoints: VecDeque<Position>,
    pub tack: Tack,
}

impl NavState {
    pub fn new() -> Self {
        Self {
            destination: None,
            waypoints: VecDeque::new(),
            tack: Tack::Starboard,
        }
    }

    pub fn with_destination(dest: Position) -> Self {
        Self {
            destination: Some(dest),
            waypoints: VecDeque::new(),
            tack: Tack::Starboard,
        }
    }

    /// Replace the current waypoint queue with a planned path. The final
    /// waypoint is taken to be the destination.
    pub fn set_path(&mut self, waypoints: Vec<Position>) {
        if let Some(last) = waypoints.last().copied() {
            self.destination = Some(last);
        }
        self.waypoints = waypoints.into();
    }

    /// Clear any planned path (keeps `destination` in place).
    pub fn clear_path(&mut self) {
        self.waypoints.clear();
    }

    /// The current heading target: front waypoint if any, else destination.
    fn current_target(&self) -> Option<Position> {
        self.waypoints.front().copied().or(self.destination)
    }

    /// Compute the optimal heading given current position, wind, and destination.
    /// Returns None if no destination is set or already arrived.
    ///
    /// `land` is optional; when provided, the chosen heading is reactively
    /// deflected to clear nearby coastline (a safety net for when wind /
    /// tacking pushes the ship close to land between waypoints).
    pub fn compute_heading(
        &mut self,
        pos: Position,
        stats: &ShipStats,
        wind: &WindVector,
        land: Option<&LandMap>,
    ) -> Option<f32> {
        // Advance through waypoints we've already reached.
        while let Some(&wp) = self.waypoints.front() {
            if pos.distance(wp) < WAYPOINT_REACHED_NM {
                self.waypoints.pop_front();
            } else {
                break;
            }
        }

        let dest = self.destination?;

        // Final arrival check (only when no intermediate waypoints remain).
        if self.waypoints.is_empty() && pos.distance(dest) < ARRIVAL_NM {
            self.destination = None;
            return None;
        }

        let target = self.current_target()?;

        let delta = target - pos;
        let bearing_to_target = normalize_angle(delta.x.atan2(delta.y).to_degrees());
        let wind_from = wind.direction_from();

        let angle_to_wind = angle_diff(bearing_to_target, wind_from).abs();

        let chosen = if angle_to_wind > stats.no_go_half_angle + 5.0 {
            // Can sail direct
            bearing_to_target
        } else {
            // Must tack: compute VMG for each side
            let port_heading = normalize_angle(wind_from - stats.no_go_half_angle);
            let starboard_heading = normalize_angle(wind_from + stats.no_go_half_angle);

            let port_vmg = vmg(port_heading, bearing_to_target, speed_at_heading(port_heading, stats, wind));
            let starboard_vmg = vmg(starboard_heading, bearing_to_target, speed_at_heading(starboard_heading, stats, wind));

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

            match self.tack {
                Tack::Port => port_heading,
                Tack::Starboard => starboard_heading,
            }
        };

        // Reactive land deflection (optional). Keeps the ship from sailing
        // into a coast between planner waypoints (e.g., when blown sideways).
        Some(match land {
            Some(land) => deflect_for_land(pos, chosen, land, DEFLECT_LOOKAHEAD_NM),
            None => chosen,
        })
    }
}

/// If the heading from `pos` would hit land within `lookahead_nm`, sweep
/// outward from `desired` in 10° steps (preferring the smaller deflection)
/// until we find a heading whose forward ray is clear. Falls back to the
/// desired heading if nothing better is found within ±90°.
fn deflect_for_land(pos: Position, desired: f32, land: &LandMap, lookahead_nm: f32) -> f32 {
    let probe = |h: f32| -> bool {
        let rad = h.to_radians();
        let end = pos + Position::new(rad.sin() * lookahead_nm, rad.cos() * lookahead_nm);
        land.line_is_clear(pos, end)
    };

    if probe(desired) {
        return desired;
    }

    // Sweep ±10°, ±20°, … up to ±90°.
    for offset_i in 1..=9 {
        let offset = offset_i as f32 * 10.0;
        for &sign in &[1.0_f32, -1.0] {
            let candidate = normalize_angle(desired + offset * sign);
            if probe(candidate) {
                return candidate;
            }
        }
    }
    desired
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
        let heading = nav.compute_heading(Position::ZERO, &stats, &wind, None).unwrap();
        assert!((heading - 90.0).abs() < 1.0);
    }

    #[test]
    fn test_tacking_heading() {
        let mut nav = NavState::with_destination(Position::new(0.0, 100.0));
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: -15.0 }; // from north
        let heading = nav.compute_heading(Position::ZERO, &stats, &wind, None).unwrap();
        let angle_from_wind = angle_diff(heading, 0.0).abs();
        assert!(angle_from_wind >= stats.no_go_half_angle - 1.0);
    }

    #[test]
    fn test_arrival_clears_destination() {
        let mut nav = NavState::with_destination(Position::new(3.0, 0.0));
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 };
        let result = nav.compute_heading(Position::ZERO, &stats, &wind, None);
        assert!(result.is_none()); // within 5 NM = arrived
        assert!(nav.destination.is_none());
    }

    #[test]
    fn test_vmg_calculation() {
        assert!((vmg(90.0, 90.0, 10.0) - 10.0).abs() < 0.01);
        assert!(vmg(0.0, 90.0, 10.0).abs() < 0.01);
    }
}
