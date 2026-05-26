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

use arrayvec::ArrayVec;

use crate::coastline_geom::CoastlineGeom;
use crate::map::land::LandMap;
use crate::ship::{angle_diff, normalize_angle, speed_at_heading, ShipStats};
use crate::sim_rng::SimRng;
use crate::types::{Position, ShipId, WindVector};

/// Bundle of land-truth queries the steering layer uses for reactive
/// deflection. The polygon-truth [`CoastlineGeom`] is the oracle;
/// [`LandMap`] is its raster pre-filter (`CoastlineGeom` queries take
/// it as a parameter — see `coastline_geom.rs` for the bilevel design).
#[derive(Clone, Copy)]
pub struct NavTerrain<'a> {
    pub geom: &'a CoastlineGeom,
    pub land: &'a LandMap,
}

/// Maximum number of intermediate waypoints a planned path may hold.
/// Pathfind benchmark (1406 ordered port pairs) tops out at 37 with the old
/// full-interior LOS smoother. The boundary-only smoother used in production
/// keeps every interior mesh node, which on long routes (e.g. Caribbean to
/// Europe) routinely reaches the low hundreds. 512 leaves comfortable
/// headroom for any plausible Caribbean-world route while keeping
/// `ArrayVec<Position, _>` inline (~4 KB per ship — fine at ~500 ships).
pub const MAX_WAYPOINTS: usize = 512;

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

// --- Period-correct navigation (Nav-1/2/3) ---

/// Per-hour latitude DR error standard deviation (NM) under normal
/// cruising. Latitude is resettable via noon sight; this is what
/// accumulates between sights.
pub const DR_ERROR_LAT_NM_PER_HOUR: f32 = 0.05;
/// Per-hour longitude DR error standard deviation (NM). Longitude has
/// no equivalent to the noon sight in 1680 (the chronometer doesn't
/// arrive until ~1761), so this accumulates unbounded until a
/// landmark fix.
pub const DR_ERROR_LON_NM_PER_HOUR: f32 = 0.15;
/// Range (NM) at which a captain can recognize a known port / landfall
/// well enough to fix his position. Sized so Caribbean islands at
/// typical inter-port spacing (~150–300 NM) regularly provide fixes
/// without two ports' sight-circles overlapping ambiguously.
pub const LANDMARK_SIGHT_NM: f32 = 20.0;
/// Latitude noise (NM) introduced by a noon sight (sextant ≈ 1' arc ≈
/// 1 NM at best; we model 0.5 NM stddev as good-conditions accuracy).
const NOON_SIGHT_NOISE_NM: f32 = 0.5;
/// Position noise (NM) introduced by a landmark fix. Recognizing the
/// silhouette of an island puts you within a mile or so of where you
/// think you are — far better than a noon sight in both axes.
const LANDMARK_FIX_NOISE_NM: f32 = 1.0;

// xorshift64 — legacy. Removed after PCG migration; the project-wide
// generator now lives in `crate::sim_rng::SimRng`.

/// Walk the dead-reckoning estimate by one hour. Error scales with the
/// commanded speed — a ship hove-to at zero kt doesn't drift. The
/// `error_multiplier` is reserved for the storm step (Nav-5+); pass 1.0
/// in normal conditions.
pub fn apply_dr_error(
    estimate: &mut Position,
    speed_kt: f32,
    error_multiplier: f32,
    rng: &mut SimRng,
) {
    let scale = (speed_kt.max(0.0) / 6.0) * error_multiplier; // 6 kt ≈ typical sloop cruise
    estimate.x += rng.gaussian() * DR_ERROR_LON_NM_PER_HOUR * scale;
    estimate.y += rng.gaussian() * DR_ERROR_LAT_NM_PER_HOUR * scale;
}

/// Attempt a noon sight: at most once per simulated day. Returns `true`
/// on success. Snaps the estimate's **latitude** to truth.y plus
/// small Gaussian noise. Longitude is unchanged — there was no way to
/// measure it from a sextant alone in 1680.
pub fn try_noon_sight(
    estimate: &mut Position,
    truth: Position,
    day_of_year: u16,
    last_sight_day: &mut u16,
    rng: &mut SimRng,
) -> bool {
    if day_of_year == 0 || day_of_year == *last_sight_day {
        return false;
    }
    estimate.y = truth.y + rng.gaussian() * NOON_SIGHT_NOISE_NM;
    *last_sight_day = day_of_year;
    true
}

/// Attempt a landmark fix: if any port is within `LANDMARK_SIGHT_NM` of
/// the ship's true position, snap the estimate to truth plus small
/// Gaussian noise. Returns the port index on success.
///
/// Caller-side LoS gating (e.g., requiring an unobstructed line through
/// the navmesh land grid) can be added later by passing a pre-filtered
/// `ports` slice or wrapping this call. For Nav-3 we accept that, at
/// 20 NM, line-of-sight to a Caribbean island is essentially always
/// available.
pub fn try_landmark_fix(
    estimate: &mut Position,
    truth: Position,
    ports: &[crate::port::Port],
    rng: &mut SimRng,
) -> Option<usize> {
    for (idx, port) in ports.iter().enumerate() {
        if truth.distance(port.position) <= LANDMARK_SIGHT_NM {
            estimate.x = truth.x + rng.gaussian() * LANDMARK_FIX_NOISE_NM;
            estimate.y = truth.y + rng.gaussian() * LANDMARK_FIX_NOISE_NM;
            return Some(idx);
        }
    }
    None
}

/// The captain's long-term intent. Owned by `ShipAI` (the captain) — when
/// the captain is replaced, the goal is replaced too. Phase-3 split: this
/// is the "where I want to go" half of the legacy `NavState`.
///
/// As of Nav-1/2/3, also owns the captain's *belief* about where the
/// ship is (`estimated_position`). Plans, arrival checks, and BT
/// conditions evaluate against this estimate; the gap between
/// `estimated_position` and `ship.position` (truth) is what creates the
/// period-realistic getting-lost / landmark-fix behaviors.
#[derive(Debug, Clone, Default)]
pub struct NavGoal {
    /// Final goal — once cleared, the ship has arrived.
    pub destination: Option<Position>,
    /// Index of the destination port, when the destination is one. Enables
    /// harbor-zone arrival in the AI layer; geometric arrival is still used
    /// for free-form destinations (None).
    pub dest_port: Option<usize>,
    /// Captain's dead-reckoning estimate of the ship's position. `None`
    /// until the AI's first tick, when it's lazily initialized to the
    /// true position (perfect noon-sight at launch). After that it walks
    /// via DR error and snaps on noon sights / landmark fixes.
    pub estimated_position: Option<Position>,
    /// `SimDate::day_of_year` on which we last took a noon sight (snapped
    /// the latitude). 0 means "never". Used to throttle noon sights to
    /// once per day per ship.
    pub last_noon_sight_day: u16,
    /// Step 6: a Pirate ship's current quarry. Set/cleared by the
    /// `see_prey` condition; consumed by `act_pursue`. `None` means
    /// "not currently chasing anything". Survives across ticks for
    /// chase coherence (hysteresis: a pirate keeps chasing past the
    /// initial detection range as long as the target stays in a
    /// slightly wider band — see `ai::PURSUE_BREAKOFF_NM`).
    pub pursue_target: Option<ShipId>,
    /// Step 6: a Merchant ship's current threat. Set/cleared by the
    /// `see_threat` condition; consumed by `act_flee`. Same hysteresis
    /// as `pursue_target` — once seen, the merchant keeps fleeing
    /// until the threat falls outside the breakoff range.
    pub flee_from: Option<ShipId>,
}

impl NavGoal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_destination(dest: Position) -> Self {
        Self {
            destination: Some(dest),
            dest_port: None,
            estimated_position: None,
            last_noon_sight_day: 0,
            pursue_target: None,
            flee_from: None,
        }
    }

    pub fn clear(&mut self) {
        self.destination = None;
        self.dest_port = None;
    }
}

/// The ship's in-flight navigation tracking — what waypoints it's currently
/// following and what port (if any) it's moored at. Owned by `Ship` because
/// it's a property of the hull's commitments to the world, not the captain.
/// If the captain is swapped (Phase 4 player/scripted/scripted captains),
/// the ship still has the same waypoints queued and the same mooring.
#[derive(Debug, Clone, Default)]
pub struct NavTrack {
    /// Index of the port the ship is currently docked at, if any. Set when
    /// `ACT_SAIL` transitions into Docked, cleared on undock. Lets dock-time
    /// behaviors (resupply, careen, trade) find the right port market.
    pub docked_at_port: Option<usize>,
    /// Ordered intermediate waypoints (front = next target). The final
    /// element should equal the captain's `NavGoal.destination` when a path
    /// was planned. Capped at [`MAX_WAYPOINTS`] so the queue lives inline
    /// in the `Ship` struct (no per-ship heap allocation, no pointer chase
    /// when combat/weather iterate over fleets).
    pub waypoints: ArrayVec<Position, MAX_WAYPOINTS>,
}

impl NavTrack {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the current waypoint queue with a planned path. Truncates
    /// silently if `waypoints.len() > MAX_WAYPOINTS`; with the current
    /// 64-slot cap and a measured max of 37, this never fires in practice
    /// — a debug_assert catches it in dev so we notice if a future route
    /// (e.g. a longer trans-oceanic leg) starts to need a bigger cap.
    pub fn set_path(&mut self, waypoints: Vec<Position>) {
        debug_assert!(
            waypoints.len() <= MAX_WAYPOINTS,
            "planned path of {} waypoints exceeds MAX_WAYPOINTS={}",
            waypoints.len(),
            MAX_WAYPOINTS
        );
        self.waypoints.clear();
        for wp in waypoints.into_iter().take(MAX_WAYPOINTS) {
            self.waypoints.push(wp);
        }
    }

    /// Clear any planned path (keeps `goal.destination` in place).
    pub fn clear_path(&mut self) {
        self.waypoints.clear();
    }

    /// The current heading target: front waypoint if any, else goal destination.
    fn current_target(&self, goal: &NavGoal) -> Option<Position> {
        self.waypoints.first().copied().or(goal.destination)
    }

    /// Compute steering (heading + commanded speed) given current position,
    /// wind, and destination. Returns None if no destination is set or the
    /// ship has arrived. Mutates self (waypoint advancement) and goal
    /// (destination cleared on final arrival).
    ///
    /// The returned heading is always direct toward the next waypoint /
    /// destination. When that bearing lies inside the no-go zone, the
    /// returned `speed` reflects the velocity-made-good of an optimal
    /// (instantaneous) tack, so coastal voyages don't drift sideways.
    ///
    /// `pos_estimate` is the captain's belief about where the ship is —
    /// used to plot bearings and advance waypoints (the captain crosses
    /// a waypoint when he thinks he has). `pos_truth` is the actual
    /// position — used **only** to decide that the voyage is complete
    /// and clear `goal.destination`. (A real captain doesn't declare
    /// arrival; the lookout shouts "land ho!" when truth reaches port.
    /// The landmark fix is what then collapses estimate onto truth.)
    ///
    /// `terrain` is optional; when provided, the heading is reactively
    /// deflected to clear nearby coastline (a safety net for when wind
    /// / drift pushes the ship close to land between planner waypoints).
    /// Phase E: now consults polygon-truth `CoastlineGeom` instead of
    /// the raster `LandMap` directly.
    #[allow(clippy::too_many_arguments)]
    pub fn compute_steering(
        &mut self,
        goal: &mut NavGoal,
        pos_estimate: Position,
        pos_truth: Position,
        stats: &ShipStats,
        wind: &WindVector,
        terrain: Option<NavTerrain<'_>>,
    ) -> Option<Steering> {
        // Advance through waypoints we've already reached (per the
        // captain's belief — he checks them off as he passes them).
        // This is honest: a real captain looks at his chart and his
        // DR plot, not at a GPS truth. The minor "corner cutting"
        // this causes when estimate is biased is bounded by DR
        // noise (a NM or two) — well under WAYPOINT_REACHED_NM.
        // ArrayVec has no pop_front; remove(0) is O(n) but n <= 64 and
        // typically we pop at most a handful per tick.
        while let Some(&wp) = self.waypoints.first() {
            if pos_estimate.distance(wp) < WAYPOINT_REACHED_NM {
                self.waypoints.remove(0);
            } else {
                break;
            }
        }

        let dest = goal.destination?;

        // Final arrival check uses TRUTH: a ship has arrived when its
        // hull is at the harbor, not when its captain thinks it has.
        if self.waypoints.is_empty() && pos_truth.distance(dest) < ARRIVAL_NM {
            goal.destination = None;
            return None;
        }

        let target = self.current_target(goal)?;

        let delta = target - pos_estimate;
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
            let port_vmg = vmg(
                port_h,
                bearing_to_target,
                speed_at_heading(port_h, stats, wind),
            );
            let stbd_vmg = vmg(
                stbd_h,
                bearing_to_target,
                speed_at_heading(stbd_h, stats, wind),
            );
            port_vmg.max(stbd_vmg).max(MIN_UPWIND_VMG)
        };

        // Reactive land deflection (optional). Keeps the ship from sailing
        // into a coast between planner waypoints (e.g., when blown sideways).
        // Uses TRUTH, not estimate: this represents the lookout/crew seeing
        // breakers ahead of the actual hull. Without this, DR error could
        // put the captain's mental ship safely offshore while the real hull
        // grounds on a reef the lookouts could plainly see.
        let heading = match terrain {
            Some(t) => deflect_for_land(pos_truth, bearing_to_target, t, DEFLECT_LOOKAHEAD_NM),
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
fn deflect_for_land(
    pos: Position,
    desired: f32,
    terrain: NavTerrain<'_>,
    lookahead_nm: f32,
) -> f32 {
    if let Some(h) = sweep_clear(pos, desired, terrain, lookahead_nm) {
        return h;
    }
    // Tight-water fallback: shorter horizon, accept any direction that
    // gives us at least a short clear ray.
    if let Some(h) = sweep_clear(pos, desired, terrain, (lookahead_nm * 0.25).max(2.0)) {
        return h;
    }
    desired
}

/// Sweep ±10°, ±20°, … ±90° around `desired`, returning the first heading
/// whose forward `lookahead_nm` ray is clear of land. `desired` itself is
/// tried first (offset 0).
fn sweep_clear(
    pos: Position,
    desired: f32,
    terrain: NavTerrain<'_>,
    lookahead_nm: f32,
) -> Option<f32> {
    let probe = |h: f32| -> bool {
        let rad = h.to_radians();
        let end = pos + Position::new(rad.sin() * lookahead_nm, rad.cos() * lookahead_nm);
        terrain.geom.line_is_clear(terrain.land, pos, end)
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
        let mut goal = NavGoal::with_destination(Position::new(100.0, 0.0));
        let mut nav = NavTrack::new();
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 }; // from south
        let s = nav
            .compute_steering(
                &mut goal,
                Position::ZERO,
                Position::ZERO,
                &stats,
                &wind,
                None,
            )
            .unwrap();
        assert!((s.heading - 90.0).abs() < 1.0);
        assert!(s.speed > 5.0, "beam reach should be fast");
    }

    #[test]
    fn test_upwind_uses_vmg_speed() {
        // Heading is direct toward target even when in the no-go zone, but
        // commanded speed is the VMG of an optimal tack.
        let mut goal = NavGoal::with_destination(Position::new(0.0, 100.0));
        let mut nav = NavTrack::new();
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: -15.0 }; // from north
        let s = nav
            .compute_steering(
                &mut goal,
                Position::ZERO,
                Position::ZERO,
                &stats,
                &wind,
                None,
            )
            .unwrap();
        // Heading is direct (north).
        assert!(angle_diff(s.heading, 0.0).abs() < 1.0);
        // Speed is reduced (VMG, not full hull speed).
        assert!(
            s.speed < stats.speed_typical,
            "upwind VMG should be slower than typical"
        );
        assert!(s.speed > MIN_UPWIND_VMG, "should still make some progress");
    }

    #[test]
    fn test_arrival_clears_destination() {
        let mut goal = NavGoal::with_destination(Position::new(3.0, 0.0));
        let mut nav = NavTrack::new();
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 };
        let result = nav.compute_steering(
            &mut goal,
            Position::ZERO,
            Position::ZERO,
            &stats,
            &wind,
            None,
        );
        assert!(result.is_none()); // within ARRIVAL_NM
        assert!(goal.destination.is_none());
    }

    #[test]
    fn test_vmg_calculation() {
        assert!((vmg(90.0, 90.0, 10.0) - 10.0).abs() < 0.01);
        assert!(vmg(0.0, 90.0, 10.0).abs() < 0.01);
    }
}
