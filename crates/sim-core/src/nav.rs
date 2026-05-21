//! Navigation utilities: VMG-based steering, waypoint following, and
//! reactive land deflection. Used by the AI layer to translate goals into
//! heading + speed commands.
//!
//! We do not physically simulate tacking. When the bearing to the next
//! waypoint lies inside the no-go zone, the ship still sails *directly*
//! toward the waypoint, but at the velocity-made-good (VMG) it would have
//! achieved by tacking optimally to either side. This is geometrically
//! equivalent over distances larger than a few NM and avoids the whole
//! class of bugs where physical zig-zagging pushes a ship into a coastline.

use std::collections::VecDeque;

use crate::map::land::LandMap;
use crate::ship::{angle_diff, normalize_angle, speed_at_heading, ShipStats};
use crate::types::{Position, WindVector};

/// A steering command: the heading to set on the ship and the speed it
/// should make good toward its target this tick (pre-fouling).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Steering {
    pub heading: f32,
    pub speed: f32,
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

/// Minimum speed (kt) the VMG model will report for an upwind leg. Keeps
/// long voyages from grinding to literal zero in light winds.
const MIN_UPWIND_VMG: f32 = 0.5;

/// Navigation state for a ship (owned by AI, not by Ship itself).
pub struct NavState {
    /// Final goal — once cleared, the ship has arrived.
    pub destination: Option<Position>,
    /// Index of the destination port, when the destination is one. Enables
    /// harbor-zone arrival in the AI layer; geometric arrival is still used
    /// for free-form destinations (None).
    pub dest_port: Option<usize>,
    /// Index of the port the ship is currently docked at, if any. Set when
    /// `ACT_SAIL` transitions into Docked, cleared on undock. Lets dock-time
    /// behaviors (resupply, careen, trade) find the right port market.
    pub docked_at_port: Option<usize>,
    /// Ordered intermediate waypoints (front = next target). The final
    /// element should equal `destination` when a path was planned.
    pub waypoints: VecDeque<Position>,
}

impl NavState {
    pub fn new() -> Self {
        Self {
            destination: None,
            dest_port: None,
            docked_at_port: None,
            waypoints: VecDeque::new(),
        }
    }

    pub fn with_destination(dest: Position) -> Self {
        Self {
            destination: Some(dest),
            dest_port: None,
            docked_at_port: None,
            waypoints: VecDeque::new(),
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

    /// Compute steering (heading + commanded speed) given current position,
    /// wind, and destination. Returns None if no destination is set or the
    /// ship has arrived.
    ///
    /// The returned heading is always direct toward the next waypoint /
    /// destination. When that bearing lies inside the no-go zone, the
    /// returned `speed` reflects the velocity-made-good of an optimal
    /// (instantaneous) tack, so coastal voyages don't drift sideways.
    ///
    /// `land` is optional; when provided, the heading is reactively deflected
    /// to clear nearby coastline (a safety net for when wind / drift pushes
    /// the ship close to land between planner waypoints).
    pub fn compute_steering(
        &mut self,
        pos: Position,
        stats: &ShipStats,
        wind: &WindVector,
        land: Option<&LandMap>,
    ) -> Option<Steering> {
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

        let speed = if angle_to_wind > stats.no_go_half_angle + 5.0 {
            // Direct sailing: speed at the actual heading.
            speed_at_heading(bearing_to_target, stats, wind)
        } else {
            // Upwind: model the ship as sailing directly at the VMG of an
            // optimal tack. We pick whichever side gives better progress.
            let port_h = normalize_angle(wind_from - stats.no_go_half_angle);
            let stbd_h = normalize_angle(wind_from + stats.no_go_half_angle);
            let port_vmg = vmg(port_h, bearing_to_target, speed_at_heading(port_h, stats, wind));
            let stbd_vmg = vmg(stbd_h, bearing_to_target, speed_at_heading(stbd_h, stats, wind));
            port_vmg.max(stbd_vmg).max(MIN_UPWIND_VMG)
        };

        // Reactive land deflection (optional). Keeps the ship from sailing
        // into a coast between planner waypoints (e.g., when blown sideways).
        let heading = match land {
            Some(land) => deflect_for_land(pos, bearing_to_target, land, DEFLECT_LOOKAHEAD_NM),
            None => bearing_to_target,
        };

        Some(Steering { heading, speed })
    }
}

/// If the heading from `pos` would hit land within `lookahead_nm`, sweep
/// outward from `desired` in 10° steps (preferring the smaller deflection)
/// until we find a heading whose forward ray is clear.
///
/// Two-tier fallback for tight waters: if no heading clears the full
/// lookahead at any deflection up to ±90°, retry with a quarter of the
/// lookahead. That lets the ship pick a viable short-tack heading (it will
/// only make a fraction of normal progress, but it won't pin to zero).
/// As a last resort returns the desired heading.
fn deflect_for_land(pos: Position, desired: f32, land: &LandMap, lookahead_nm: f32) -> f32 {
    if let Some(h) = sweep_clear(pos, desired, land, lookahead_nm) {
        return h;
    }
    // Tight-water fallback: shorter horizon, accept any direction that
    // gives us at least a short clear ray.
    if let Some(h) = sweep_clear(pos, desired, land, (lookahead_nm * 0.25).max(2.0)) {
        return h;
    }
    desired
}

/// Sweep ±10°, ±20°, … ±90° around `desired`, returning the first heading
/// whose forward `lookahead_nm` ray is clear of land. `desired` itself is
/// tried first (offset 0).
fn sweep_clear(pos: Position, desired: f32, land: &LandMap, lookahead_nm: f32) -> Option<f32> {
    let probe = |h: f32| -> bool {
        let rad = h.to_radians();
        let end = pos + Position::new(rad.sin() * lookahead_nm, rad.cos() * lookahead_nm);
        land.line_is_clear(pos, end)
    };
    if probe(desired) {
        return Some(desired);
    }
    for offset_i in 1..=9 {
        let offset = offset_i as f32 * 10.0;
        for &sign in &[1.0_f32, -1.0] {
            let candidate = normalize_angle(desired + offset * sign);
            if probe(candidate) {
                return Some(candidate);
            }
        }
    }
    None
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
        let s = nav.compute_steering(Position::ZERO, &stats, &wind, None).unwrap();
        assert!((s.heading - 90.0).abs() < 1.0);
        assert!(s.speed > 5.0, "beam reach should be fast");
    }

    #[test]
    fn test_upwind_uses_vmg_speed() {
        // Heading is direct toward target even when in the no-go zone, but
        // commanded speed is the VMG of an optimal tack.
        let mut nav = NavState::with_destination(Position::new(0.0, 100.0));
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: -15.0 }; // from north
        let s = nav.compute_steering(Position::ZERO, &stats, &wind, None).unwrap();
        // Heading is direct (north).
        assert!(angle_diff(s.heading, 0.0).abs() < 1.0);
        // Speed is reduced (VMG, not full hull speed).
        assert!(s.speed < stats.speed_typical, "upwind VMG should be slower than typical");
        assert!(s.speed > MIN_UPWIND_VMG, "should still make some progress");
    }

    #[test]
    fn test_arrival_clears_destination() {
        let mut nav = NavState::with_destination(Position::new(3.0, 0.0));
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 };
        let result = nav.compute_steering(Position::ZERO, &stats, &wind, None);
        assert!(result.is_none()); // within ARRIVAL_NM
        assert!(nav.destination.is_none());
    }

    #[test]
    fn test_vmg_calculation() {
        assert!((vmg(90.0, 90.0, 10.0) - 10.0).abs() < 0.01);
        assert!(vmg(0.0, 90.0, 10.0).abs() < 0.01);
    }
}
