//! Step 10.b: non-combat attrition. Hourly stochastic rolls for storms,
//! foundering, and fire; per-hour teredo accumulation; daily age bump.
//!
//! Calibration target (from `planning/research/ship-attrition-economics-
//! 1650-1720.md`): 1.5-2.5%/yr peacetime loss in the open Atlantic,
//! 3-6%/yr in the Caribbean basin. Cause mix per Davis (1962) and
//! Jarvis (1954): storms ~50-65%, foundering 10-20%, fire 5-10%.
//!
//! All RNG flows through a single `HazardSystem.rng_state` for
//! determinism; the same world seeded the same way produces the same
//! attrition trace.
//!
//! Position convention: y axis is northing in NM from origin
//! (17.5°N, 72.5°W). 1° latitude ≈ 60 NM, so the 25°N tropical line
//! sits at y = (25.0 - 17.5) * 60.0 = 450.0 NM.

use crate::ship::{Ship, ShipState};
use crate::types::Position;

/// Latitudes south of this y-value are considered "tropical" for the
/// purposes of teredo accumulation and hurricane risk. Corresponds to
/// 25°N (slightly above the Bahamas / Bermuda line).
const TROPICAL_Y_NM: f32 = 450.0;

/// Annual base storm probability for tropical waters (Caribbean +
/// Gulf). Davis 1962: ~3-4% all-cause peacetime, of which ~60% storms.
const STORM_RATE_TROPICAL: f32 = 0.025;
/// Annual base storm probability for open-ocean / temperate routes.
const STORM_RATE_OPEN: f32 = 0.012;
/// Hurricane-season multiplier applied to `STORM_RATE_TROPICAL` in
/// August, September, and October. Real Caribbean hurricane climatology
/// shows ~85% of land-falling storms in this window.
const HURRICANE_MONTH_MULTIPLIER: f32 = 3.0;
/// Months considered hurricane season (1-indexed, like `Date::month`).
const HURRICANE_MONTHS: &[u8] = &[8, 9, 10];

/// Fraction of storm events that are catastrophic (always-sink
/// hurricanes/squalls/strandings). The other 60% are survivable
/// damage events that bleed hull integrity. Calibrated so that with
/// a tropical base rate of 2.5%/yr × ~0.4 catastrophic ≈ 1%/yr
/// outright storm sinkings, in line with Davis 1962's peacetime
/// numbers.
const STORM_CATASTROPHIC_FRACTION: f32 = 0.40;

/// Annual fire probability while sailing. Davis et al.: fire is the
/// cause in ~5-10% of all losses, dominated by spirits/powder cargoes.
const FIRE_RATE_SAILING: f32 = 0.004;

/// Annual foundering probability ceiling once teredo damage saturates.
/// Foundering is a structural-failure event triggered by hull rot in
/// the absence of careening; in our model it ramps from 0 at
/// `TEREDO_FOUNDERING_THRESHOLD` to this ceiling at full teredo.
const FOUNDERING_RATE_AT_MAX_TEREDO: f32 = 0.03;
const TEREDO_FOUNDERING_THRESHOLD: f32 = 30.0;
const TEREDO_MAX: f32 = 100.0;

/// Per-hour teredo accumulation in tropical water (NOT careening).
/// 0.005/hr × 24 × 30 = 3.6 points/month, so a clean hull reaches the
/// 30-point foundering threshold in ~8 months and 80 points (the
/// research's "structurally dangerous" mark) in ~22 months — matching
/// the 18-36 month figure in §1.3.
const TEREDO_RATE_TROPICAL_PER_HOUR: f32 = 0.005;
/// Per-hour teredo accumulation in non-tropical water. Roughly 1/5
/// the tropical rate; teredo navalis prefers warm salty waters.
const TEREDO_RATE_OPEN_PER_HOUR: f32 = 0.001;

const HOURS_PER_YEAR: f32 = 24.0 * 365.25;

/// What a hazard roll produced for a single ship-hour. The world tick
/// applies these to the ship (damage, sinking, lost stores) and
/// updates the per-cause counters in the `HazardSystem`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HazardEvent {
    /// Storm damaged the ship but didn't sink it. `hull_loss` is in
    /// the same units as `Ship::hull_integrity`.
    StormDamage { hull_loss: f32 },
    /// Storm reduced hull integrity below zero — apply Sunk.
    StormSunk,
    /// Hull foundered (structural failure from teredo / old age) —
    /// apply Sunk. No warning event; foundering is a one-shot.
    Foundered,
    /// Fire aboard. If `sunk` is true the world should mark the ship
    /// Sunk; otherwise apply `hull_loss` and clear the magazine.
    Fire { hull_loss: f32, sunk: bool },
}

/// Per-cause totals over the run, for bench reporting.
#[derive(Debug, Clone, Copy, Default)]
pub struct HazardCounters {
    pub storms_damaged: u32,
    pub storms_sunk: u32,
    pub foundered: u32,
    pub fires: u32,
    pub fires_sunk: u32,
}

pub struct HazardSystem {
    rng_state: u64,
    pub counters: HazardCounters,
}

impl HazardSystem {
    pub fn new(seed: u64) -> Self {
        Self {
            // Seed of 0 would zero the xorshift forever, so XOR in a
            // golden-ratio constant to keep the stream lively.
            rng_state: seed ^ 0x9E37_79B9_7F4A_7C15,
            counters: HazardCounters::default(),
        }
    }

    /// Returns true if a position is in tropical water (south of 25°N).
    pub fn is_tropical(pos: Position) -> bool {
        pos.y < TROPICAL_Y_NM
    }

    /// Returns true if `month` (1-12) is a hurricane-season month.
    pub fn is_hurricane_month(month: u8) -> bool {
        HURRICANE_MONTHS.contains(&month)
    }

    /// One-hour environmental update: teredo accumulation while in
    /// water (anywhere except when actively careened — we treat
    /// `Sailing | Anchored | Docked` as "in water"; ships in `Hiring`
    /// are also docked so they accumulate too).
    pub fn tick_environment(ship: &mut Ship, pos: Position) {
        if ship.state == ShipState::Sunk {
            return;
        }
        // Careened ships are physically heeled out of the water; no
        // teredo accumulation while the worm-side planking dries.
        if matches!(ship.dock_action, crate::ship::DockAction::Careening) {
            return;
        }
        let rate = if Self::is_tropical(pos) {
            TEREDO_RATE_TROPICAL_PER_HOUR
        } else {
            TEREDO_RATE_OPEN_PER_HOUR
        };
        ship.teredo_damage = (ship.teredo_damage + rate).min(TEREDO_MAX);
    }

    /// Per-day age bump. Called once per day by `World::tick_daily_*`.
    pub fn tick_age(ship: &mut Ship) {
        if ship.state == ShipState::Sunk {
            return;
        }
        ship.age_days = ship.age_days.saturating_add(1);
    }

    /// Roll all stochastic hazards for one ship-hour. Returns the
    /// (possibly empty) list of events for the world to apply. The
    /// caller is responsible for updating the ship's hull, sinking it,
    /// and bumping the per-cause counters.
    pub fn roll_hazards(&mut self, ship: &Ship, pos: Position, month: u8) -> Vec<HazardEvent> {
        if ship.state == ShipState::Sunk {
            return Vec::new();
        }
        let mut out = Vec::new();

        let sailing_or_anchored = matches!(ship.state, ShipState::Sailing | ShipState::Anchored);
        let sailing = ship.state == ShipState::Sailing;

        // --- Storms: at sea or at anchor (docked ships are protected
        //     by the harbor). Hurricane months ×3 in the tropics.
        //     30% of storm events are catastrophic (loss > current
        //     hull → sinks outright); 70% leave the hull crippled but
        //     afloat. This matches the historical record where storm
        //     "losses" generally meant the ship was a total loss
        //     rather than a damaged survivor. ---
        if sailing_or_anchored {
            let base = if Self::is_tropical(pos) {
                STORM_RATE_TROPICAL
            } else {
                STORM_RATE_OPEN
            };
            let mult = if Self::is_tropical(pos) && Self::is_hurricane_month(month) {
                HURRICANE_MONTH_MULTIPLIER
            } else {
                1.0
            };
            let per_hour = base * mult / HOURS_PER_YEAR;
            if self.uniform() < per_hour {
                let catastrophic = self.uniform() < STORM_CATASTROPHIC_FRACTION;
                if catastrophic {
                    self.counters.storms_sunk += 1;
                    out.push(HazardEvent::StormSunk);
                } else {
                    // Survivable damage: 20-50% of current hull. Repeated
                    // storms will eventually compound a hull to zero.
                    let frac = 0.20 + 0.30 * self.uniform();
                    let loss = frac * hull_loss_scale(ship);
                    if ship.hull_integrity - loss <= 0.0 {
                        self.counters.storms_sunk += 1;
                        out.push(HazardEvent::StormSunk);
                    } else {
                        self.counters.storms_damaged += 1;
                        out.push(HazardEvent::StormDamage { hull_loss: loss });
                    }
                }
            }
        }

        // --- Foundering: only when sailing (anchored ships are calm,
        //     docked ships are tied up). Probability ramps with teredo
        //     and multiplies by age. ---
        if sailing && ship.teredo_damage > TEREDO_FOUNDERING_THRESHOLD {
            let t = (ship.teredo_damage - TEREDO_FOUNDERING_THRESHOLD)
                / (TEREDO_MAX - TEREDO_FOUNDERING_THRESHOLD);
            let age_mult = (ship.age_days as f32 / (365.0 * 10.0)).max(1.0);
            let per_year = FOUNDERING_RATE_AT_MAX_TEREDO * t * age_mult;
            let per_hour = per_year / HOURS_PER_YEAR;
            if self.uniform() < per_hour {
                self.counters.foundered += 1;
                out.push(HazardEvent::Foundered);
            }
        }

        // --- Fire: only when sailing. Bench has no powder-cargo signal
        //     yet, so a flat rate; later we can multiply by the
        //     magazine load. ---
        if sailing {
            let per_hour = FIRE_RATE_SAILING / HOURS_PER_YEAR;
            if self.uniform() < per_hour {
                self.counters.fires += 1;
                // 60% of fires sink outright (powder cookoff, hold
                // ablaze); 40% leave a crippled hull.
                if self.uniform() < 0.60 {
                    self.counters.fires_sunk += 1;
                    out.push(HazardEvent::Fire {
                        hull_loss: hull_loss_scale(ship),
                        sunk: true,
                    });
                } else {
                    let loss = 0.5 * hull_loss_scale(ship);
                    out.push(HazardEvent::Fire {
                        hull_loss: loss,
                        sunk: false,
                    });
                }
            }
        }

        out
    }

    /// xorshift64 step, returning a uniform sample in [0, 1).
    fn uniform(&mut self) -> f32 {
        self.rng_state ^= self.rng_state << 13;
        self.rng_state ^= self.rng_state >> 7;
        self.rng_state ^= self.rng_state << 17;
        // Mix to avoid low-bit correlation, then map to [0, 1).
        let r = self.rng_state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (r >> 11) as f32 / ((1u64 << 53) as f32)
    }
}

/// Hull-loss scale for a ship. Derived from current hull integrity so
/// damage events feel proportionate across ship types: a 100-point
/// sloop hull loses up to 80 from a bad storm; a 400-point ship loses
/// up to 320. Falls back to a constant if the ship is already at zero.
fn hull_loss_scale(ship: &Ship) -> f32 {
    // We use a fixed scale tied to the ship's *current* hull rather
    // than its max so that a near-dead ship still gets a survivable
    // event in the common case — but combat-damaged hulls fare worse,
    // which is realistic.
    if ship.hull_integrity > 1.0 {
        ship.hull_integrity
    } else {
        50.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::Faction;
    use crate::ship::{Ship, ShipState};
    use glam::Vec2;

    fn test_ship(state: ShipState) -> Ship {
        let mut s = Ship::seeded_at_port(Vec2::new(0.0, 0.0), 0, Faction::Free);
        s.state = state;
        s
    }

    #[test]
    fn tropical_zone_below_25n() {
        assert!(HazardSystem::is_tropical(Vec2::new(0.0, 0.0)));
        assert!(HazardSystem::is_tropical(Vec2::new(100.0, 449.0)));
        assert!(!HazardSystem::is_tropical(Vec2::new(0.0, 451.0)));
    }

    #[test]
    fn hurricane_months_aug_sep_oct() {
        assert!(HazardSystem::is_hurricane_month(8));
        assert!(HazardSystem::is_hurricane_month(9));
        assert!(HazardSystem::is_hurricane_month(10));
        assert!(!HazardSystem::is_hurricane_month(7));
        assert!(!HazardSystem::is_hurricane_month(11));
    }

    #[test]
    fn teredo_accumulates_in_tropics_faster_than_open_water() {
        let mut tropical = test_ship(ShipState::Sailing);
        let mut open = test_ship(ShipState::Sailing);
        for _ in 0..(24 * 30 * 6) {
            HazardSystem::tick_environment(&mut tropical, Vec2::new(0.0, 0.0));
            HazardSystem::tick_environment(&mut open, Vec2::new(0.0, 1000.0));
        }
        assert!(tropical.teredo_damage > open.teredo_damage * 3.0);
    }

    #[test]
    fn teredo_does_not_accumulate_while_careening() {
        let mut s = test_ship(ShipState::Docked);
        s.dock_action = crate::ship::DockAction::Careening;
        for _ in 0..1000 {
            HazardSystem::tick_environment(&mut s, Vec2::new(0.0, 0.0));
        }
        assert_eq!(s.teredo_damage, 0.0);
    }

    #[test]
    fn sunk_ships_are_inert() {
        let mut s = test_ship(ShipState::Sunk);
        let mut sys = HazardSystem::new(0xDEAD);
        for _ in 0..1000 {
            HazardSystem::tick_environment(&mut s, Vec2::new(0.0, 0.0));
            HazardSystem::tick_age(&mut s);
            let evs = sys.roll_hazards(&s, Vec2::new(0.0, 0.0), 9);
            assert!(evs.is_empty());
        }
        assert_eq!(s.teredo_damage, 0.0);
        assert_eq!(s.age_days, 0);
    }

    #[test]
    fn docked_ships_skip_storm_rolls() {
        let mut sys = HazardSystem::new(0xCAFE);
        let s = test_ship(ShipState::Docked);
        let mut any = 0;
        // 10 sim-years of hour rolls in hurricane season at 0,0
        for _ in 0..(24 * 365 * 10) {
            any += sys.roll_hazards(&s, Vec2::new(0.0, 0.0), 9).len();
        }
        assert_eq!(any, 0, "docked ships should be storm-protected");
    }

    #[test]
    fn open_water_storm_rate_is_in_calibration_envelope() {
        // 1000 ships × 5 sim-years × non-hurricane month → expected
        // total storm events ≈ 1000 × 5 × STORM_RATE_OPEN ≈ 60. We
        // accept a wide band to keep the test robust.
        let mut sys = HazardSystem::new(0xBADC0FFEE);
        let mut s = test_ship(ShipState::Sailing);
        s.position = Vec2::new(0.0, 1000.0); // open water
        let pos = s.position;
        let mut events = 0;
        for _ in 0..1000 {
            for _ in 0..(24 * 365 * 5) {
                events += sys.roll_hazards(&s, pos, 4).len();
            }
        }
        let expected = (1000.0 * 5.0 * STORM_RATE_OPEN) as i32;
        assert!(
            (events as i32 - expected).abs() < (expected / 2),
            "open-water storm count out of band: got {}, expected ~{}",
            events,
            expected
        );
    }

    #[test]
    fn hurricane_season_amplifies_tropical_storms() {
        let mut sys_normal = HazardSystem::new(1);
        let mut sys_hurricane = HazardSystem::new(2);
        let mut s = test_ship(ShipState::Sailing);
        s.position = Vec2::new(0.0, 0.0); // tropical
        let pos = s.position;
        let mut normal = 0;
        let mut hurricane = 0;
        for _ in 0..5000 {
            for _ in 0..(24 * 365) {
                normal += sys_normal.roll_hazards(&s, pos, 4).len();
                hurricane += sys_hurricane.roll_hazards(&s, pos, 9).len();
            }
        }
        // Hurricane month should be ~3x normal. Accept anything > 2x.
        assert!(
            hurricane as f32 > normal as f32 * 2.0,
            "hurricane season {} not > 2x normal {}",
            hurricane,
            normal
        );
    }

    #[test]
    fn foundering_requires_teredo_above_threshold() {
        let mut sys = HazardSystem::new(0xF00D);
        let mut s = test_ship(ShipState::Sailing);
        s.teredo_damage = 10.0;
        let mut events = 0;
        for _ in 0..(24 * 365 * 50) {
            for ev in sys.roll_hazards(&s, Vec2::new(0.0, 0.0), 4) {
                if matches!(ev, HazardEvent::Foundered) {
                    events += 1;
                }
            }
        }
        assert_eq!(events, 0, "ships with low teredo should not founder");
    }

    #[test]
    fn careening_reduces_teredo() {
        let mut s = test_ship(ShipState::Docked);
        s.teredo_damage = 50.0;
        s.hull_fouling = 50.0;
        for _ in 0..200 {
            s.tick_careen();
        }
        assert_eq!(s.teredo_damage, 0.0);
        assert_eq!(s.hull_fouling, 0.0);
    }
}
