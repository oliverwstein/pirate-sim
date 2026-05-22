use crate::cargo::Cargo;
use crate::nav::NavTrack;
use crate::port::Faction;
use crate::types::{Position, WindVector};
use serde::Deserialize;

/// Ship performance characteristics.
#[derive(Clone, Debug, Deserialize)]
pub struct ShipStats {
    pub speed_typical: f32,       // knots in moderate trade winds
    pub speed_max: f32,           // absolute maximum
    pub windward_ability: f32,    // 0.0-1.0 (how well it sails upwind)
    pub no_go_half_angle: f32,    // degrees from wind that ship cannot sail into
    pub crew: u32,                // crew complement (determines provision consumption)
    pub provision_capacity: f32,  // max tons of provisions (separate from trade hold)
    pub cargo_capacity_tons: f32, // max tons of trade cargo
}

impl ShipStats {
    pub fn sloop() -> Self {
        Self {
            speed_typical: 9.0,
            speed_max: 12.0,
            windward_ability: 0.8,
            no_go_half_angle: 40.0,
            crew: 25,
            provision_capacity: 6.0, // ~130 days of food for 25 crew — historical 17C ocean-going ships carried 3–4 months of provisions for transatlantic crossings
            cargo_capacity_tons: 60.0, // typical sloop trade hold (Phase 2 starter)
        }
    }

    /// Daily provision consumption in tons for a ship of design
    /// crew complement. Historical: ~4 lbs/man/day total food =
    /// 0.0018 tons/man/day. Use `Ship::daily_provision_burn` for
    /// the *actual* current burn rate (scales with `crew_alive`).
    pub fn daily_provision_consumption(&self) -> f32 {
        self.crew as f32 * 0.0018
    }

    /// Minimum crew to safely put to sea. Derived as 40% of the
    /// design complement until per-type minimums land in the RON.
    /// See `planning/crewing-plan.md §2`.
    pub fn crew_min(&self) -> u16 {
        let m = (self.crew as f32 * 0.4).ceil() as u16;
        m.max(2)
    }

    /// Design complement (`stats.crew` rendered as u16 for crew
    /// arithmetic). Will become its own RON field in a later step.
    pub fn crew_typical(&self) -> u16 {
        self.crew as u16
    }

    /// Estimated voyage time in days for a great-circle distance, used
    /// for AI reachability/provisioning decisions. The 0.55 factor
    /// derates `speed_typical` for tacking, calms, and storm slow-downs;
    /// it's deliberately conservative so the AI plans with a margin.
    pub fn estimated_voyage_days(&self, distance_nm: f32) -> f32 {
        let avg_kt = (self.speed_typical * 0.55).max(0.1);
        distance_nm / (avg_kt * 24.0)
    }
}

/// The physical state of a ship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipState {
    Sailing,
    Docked,
    Anchored,
    /// Freshly-built or freshly-discharged hull awaiting a crew.
    /// World ticks daily and draws sailors from the home port's
    /// `PortDemographics`; transitions to `Docked` when
    /// `crew_alive >= stats.crew_min()`. See `planning/crewing-plan.md §3`.
    Hiring,
}

/// Default starting silver (pesos) for a freshly-spawned merchant ship.
/// Roughly enough to fill its provision hold and bunkers a few times over,
/// and to buy a partial speculative cargo of sugar at base price.
pub const STARTING_SILVER_PESOS: f32 = 5000.0;

/// A ship: purely physical entity. Heading is set externally by AI/player.
pub struct Ship {
    pub position: Position,
    pub heading: f32, // degrees (0=N, 90=E, clockwise)
    pub speed: f32,   // current speed in knots
    pub state: ShipState,
    pub provisions: f32,   // tons of food remaining (separate from trade hold)
    pub cargo: Cargo,      // trade goods (subject to cargo_capacity_tons)
    pub hull_fouling: f32, // 0 = clean, 100 = fully encrusted
    /// Pesos in the ship's strongbox. Spent at port markets to buy
    /// provisions and trade goods; earned by selling cargo.
    pub silver: f32,
    /// The port that originally launched this ship (its "home port"
    /// for owner-of-record purposes). `None` for ships spawned by
    /// tests or seeded into the world outside the shipyard system.
    /// Stage 2 of the shipbuilding system will use this for
    /// profit-remittance and refinancing at the home port.
    pub owner_port: Option<usize>,
    /// What kind of ship this is. Indexes into the world's
    /// `ShipTypeRegistry` to look up stats, build cost, etc. Defaults
    /// to `shiptype::ids::SLOOP` for back-compat with `Ship::new`.
    pub ship_type: crate::shiptype::ShipTypeId,
    /// The silver this ship was born with. Stays constant for the
    /// life of the ship; used by analytics (P/L = silver - starting_silver)
    /// so newly-built ships can be reported accurately without the
    /// caller having to race against the build moment.
    pub starting_silver: f32,
    /// Cumulative silver this ship has paid back to its owner port
    /// across all completed voyages. Each time the ship docks at its
    /// `owner_port`, any silver above the operating float is deposited
    /// into the port treasury and added here. True lifetime P/L for a
    /// home-ported ship is `(silver - starting_silver) + lifetime_dividends`.
    pub lifetime_dividends: f32,
    /// Outstanding credit drawn from port chandlers/factors —
    /// either provisions taken on tick when broke, or freight cargo
    /// (tramping) advanced against the next sale. Repaid out of any
    /// surplus silver at the next port docking, before dividends.
    /// Settles fungibly across the port network — historically this
    /// is what bills of exchange between merchant correspondents
    /// enabled.
    pub debt: f32,
    /// Live head-count. Distinct from `stats.crew_typical()` (the
    /// design complement). Ships need `>= stats.crew_min()` to put
    /// to sea; provisions burn and effective speed scale with this
    /// in Step 3.c. See `planning/crewing-plan.md`.
    pub crew_alive: u16,
    /// Wages accrued to the crew but not yet paid. Accrues while at
    /// sea at `crew_alive * WAGE_PESOS_PER_MAN_MONTH / (30 * 24)`
    /// per hour; paid out of `Ship.silver` into the destination
    /// port's market silver on each dock visit. See
    /// `planning/crewing-plan.md §6`.
    pub wages_owed_pesos: f32,
    /// Crew morale in `[0.0, 1.0]`. 1.0 = content, 0.0 = mutinous.
    /// Ticks hourly per `planning/crewing-plan.md §8`: drops with
    /// low provisions, unpaid wages, damage (Step 7), and rises
    /// with rest in port and prize money (Step 8). Effect bands
    /// throttle recruitment (0.4–0.7) and speed (0.25–0.4); deeper
    /// bands trigger mutiny / desertion in Step 9.
    pub morale: f32,
    /// Flag this ship sails under. For shipyard-built hulls this is the
    /// owner port's faction at launch. Can change at runtime (e.g., when
    /// a ship is captured as a prize in Step 8 it takes on the
    /// capturer's faction). Test/scaffolding ships built via
    /// `Ship::new` default to `Faction::Free`.
    pub faction: Faction,
    /// In-flight navigation tracking — which port the ship is currently
    /// moored at (if any) and the waypoint queue it's following.
    /// Distinct from the captain's `NavGoal` (which lives on `ShipAI`):
    /// these are the ship's commitments to the world that persist across
    /// captain swaps. See `planning/development-log.md` Step 5.b.
    pub nav: NavTrack,
    /// What the ship is doing while docked (Resupplying, Careening, or
    /// Idle). Drives display + dock-tree action selection. Moved off
    /// `ShipAI` in Step 5.b because a careen in progress is a property
    /// of the hull (paint scraped, ship beached), not the captain.
    pub dock_action: DockAction,
}

/// What the ship is doing while docked. Used by the docking sequence in
/// the AI's BT, and by the viz to show port activity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DockAction {
    Idle,
    Resupplying,
    Careening,
}

impl Ship {
    pub fn new(position: Position, state: ShipState) -> Self {
        let stats = ShipStats::sloop();
        Self {
            position,
            heading: 0.0,
            speed: 0.0,
            state,
            provisions: stats.provision_capacity,
            cargo: Cargo::new(),
            hull_fouling: 0.0,
            silver: STARTING_SILVER_PESOS,
            owner_port: None,
            ship_type: crate::shiptype::ids::SLOOP,
            starting_silver: STARTING_SILVER_PESOS,
            lifetime_dividends: 0.0,
            debt: 0.0,
            // Test / seed-fleet ships start fully crewed; the Hiring
            // loop is for shipyard-built hulls only.
            crew_alive: stats.crew_typical(),
            wages_owed_pesos: 0.0,
            morale: 1.0,
            faction: Faction::Free,
            nav: NavTrack::new(),
            dock_action: DockAction::Idle,
        }
    }

    /// Construct a ship seeded into the world at a specific home port
    /// (the typical entry point for the starter fleet in `bench_trade`
    /// and headless scenarios). Unlike `freshly_built`, this ship is
    /// fully crewed and ready to sail — there's no shipyard `Hiring`
    /// phase. The ship inherits the port's faction and silver default.
    pub fn seeded_at_port(position: Position, owner_port: usize, faction: Faction) -> Self {
        Self {
            faction,
            owner_port: Some(owner_port),
            ..Self::new(position, ShipState::Docked)
        }
    }

    /// Construct a ship freshly built at a specific shipyard port, with
    /// a custom amount of starting silver (sized at build time to be
    /// roughly enough to buy one hold of cargo at the home port). The
    /// ship's `owner_port` is set so future remittance logic can find it.
    pub fn freshly_built(
        position: Position,
        owner_port: usize,
        starting_silver: f32,
        ship_type: crate::shiptype::ShipTypeId,
        stats: &ShipStats,
        faction: Faction,
    ) -> Self {
        Self {
            position,
            heading: 0.0,
            speed: 0.0,
            // Built hulls start in Hiring — they need a crew before
            // the AI's dock tree can do anything with them.
            state: ShipState::Hiring,
            provisions: stats.provision_capacity,
            cargo: Cargo::new(),
            hull_fouling: 0.0,
            silver: starting_silver,
            owner_port: Some(owner_port),
            ship_type,
            starting_silver,
            lifetime_dividends: 0.0,
            debt: 0.0,
            crew_alive: 0,
            wages_owed_pesos: 0.0,
            morale: 1.0,
            faction,
            nav: NavTrack::new(),
            dock_action: DockAction::Idle,
        }
    }

    /// Set heading and commanded speed (the primary control inputs from
    /// AI/player). The commanded speed is what the ship will actually make
    /// good this tick (before fouling); the navigator is responsible for
    /// reducing it to reflect upwind tacking, sail damage, etc.
    pub fn set_steering(&mut self, heading: f32, speed: f32) {
        self.heading = heading;
        self.speed = speed;
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

    /// Calculate effective speed: the commanded speed (set by the navigator)
    /// reduced by hull fouling (up to 30% penalty at full fouling).
    ///
    /// `_stats` and `_wind` are kept in the signature for API compatibility
    /// and future use (e.g., gust gusts overriding command), but the speed
    /// model is now driven by the navigator via `set_steering`.
    pub fn effective_speed(&self, stats: &ShipStats, _wind: &WindVector) -> f32 {
        let fouling_penalty = 1.0 - self.hull_fouling * 0.003;
        let crew_mult = self.crew_speed_multiplier(stats);
        // Morale band 0.25–0.4: sullen crew, -20% speed (crewing-plan §8.2).
        // Above 0.4 = no effect; below 0.25 the ship is heading for
        // mutiny (Step 9) but for now still moves at the sullen rate.
        let morale_mult = if self.morale >= 0.4 { 1.0 } else { 0.8 };
        self.speed * fouling_penalty * crew_mult * morale_mult
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

    /// Actual current daily provision burn in tons, scaled by
    /// `crew_alive`. See `planning/crewing-plan.md §7.2`.
    pub fn daily_provision_burn(&self) -> f32 {
        self.crew_alive as f32 * 0.0018
    }

    /// Crew-driven speed multiplier in `[0.0, 1.0]`. See
    /// `planning/crewing-plan.md §7.1`. Piecewise: below `crew_min`
    /// the ship cannot sail (0.0); at `crew_min` it sails at 60% of
    /// rigged speed; at 60% of `crew_typical` it reaches 84%; at
    /// `crew_typical` it reaches 100%; overcrewing does not boost
    /// speed (capped at 1.0).
    ///
    /// Note: the spec text in §7.1 had an internal inconsistency
    /// between the formula (which yielded 1.0 at ratio=0.6) and the
    /// "60% → 84%" annotation. We implement the annotation; calibration
    /// in 3.d can revisit.
    pub fn crew_speed_multiplier(&self, stats: &ShipStats) -> f32 {
        let typical = stats.crew_typical().max(1) as f32;
        let min = stats.crew_min();
        if self.crew_alive < min {
            return 0.0;
        }
        let ratio = (self.crew_alive as f32 / typical).min(1.0);
        let min_ratio = min as f32 / typical;
        if ratio <= 0.6 {
            // Lerp 0.60 → 0.84 over [min_ratio, 0.6].
            let span = (0.6 - min_ratio).max(1e-6);
            let t = (ratio - min_ratio) / span;
            0.60 + 0.24 * t
        } else {
            // Lerp 0.84 → 1.00 over [0.6, 1.0].
            let t = (ratio - 0.6) / 0.4;
            0.84 + 0.16 * t
        }
    }

    /// Tick morale by one hour per `planning/crewing-plan.md §8.1`.
    /// Modifiers wired now: low/critical provisions, overdue wages,
    /// rested-in-port recovery. Instant deltas from prize money
    /// (Step 8) and damage events (Step 7) are stubbed.
    ///
    /// Note: `_stats` is accepted for API consistency; current
    /// modifiers read only from `self`.
    pub fn tick_morale(&mut self, _stats: &ShipStats) {
        let days_left = self.daily_provision_burn().recip() * self.provisions;
        let mut delta = 0.0_f32;

        // Provisions effects (critical replaces low — not additive).
        if days_left < MORALE_PROVISIONS_CRITICAL_DAYS {
            delta -= MORALE_LOSS_PROVISIONS_CRITICAL;
        } else if days_left < MORALE_PROVISIONS_LOW_DAYS {
            delta -= MORALE_LOSS_PROVISIONS_LOW;
        }

        // Wages overdue: > 2x current monthly wage bill.
        let monthly_bill = self.crew_alive as f32 * WAGE_PESOS_PER_MAN_MONTH;
        if monthly_bill > 0.0 && self.wages_owed_pesos > 2.0 * monthly_bill {
            delta -= MORALE_LOSS_WAGES_OVERDUE;
        }

        // Rested in port: docked, full belly, no wage debt.
        if self.state == ShipState::Docked
            && days_left >= MORALE_PROVISIONS_LOW_DAYS
            && self.wages_owed_pesos <= 0.0
        {
            delta += MORALE_GAIN_RESTED_IN_PORT;
        }

        self.morale = (self.morale + delta).clamp(0.0, 1.0);
    }

    /// Consume provisions and accumulate fouling for one hour.
    /// Called by world tick. Returns true if provisions are exhausted.
    pub fn tick_resources(&mut self, _stats: &ShipStats) -> bool {
        // TODO: provisions should only be consumed while sailing. Likewise, a ship should not accumulate fouling while careened, and should accumulate more while docked or anchored than while sailing.
        // Provision consumption: per hour = daily / 24, scaled by crew_alive.
        let hourly_consumption = self.daily_provision_burn() / 24.0;
        self.provisions = (self.provisions - hourly_consumption).max(0.0);

        // Hull fouling: accumulates ~1 point per 5 days in tropics
        // = 1/(5*24) per hour ≈ 0.0083/hour
        self.hull_fouling = (self.hull_fouling + 0.0083).min(100.0);

        self.provisions <= 0.0
    }

    /// Resupply provisions for one hour at a port without payment. Used
    /// by tests/scenarios that don't model markets. Returns `true` once
    /// provisions have reached capacity.
    pub fn tick_resupply(&mut self, stats: &ShipStats) -> bool {
        self.provisions = (self.provisions + RESUPPLY_RATE_PER_HOUR).min(stats.provision_capacity);
        self.provisions >= stats.provision_capacity
    }

    /// Resupply provisions for one hour at a port market: buy provisions
    /// from the port's stockpile, paying out of `self.silver` at the
    /// market's buy price. Returns `true` when no further resupply is
    /// possible — either the hold is full, the ship is broke, or the
    /// market is dry.
    ///
    /// `goods` provides the canonical PROVISIONS handle and base price.
    pub fn tick_resupply_at_market(
        &mut self,
        stats: &ShipStats,
        market: &mut crate::market::PortMarket,
        goods: &crate::goods::GoodsRegistry,
    ) -> bool {
        let provisions_id = crate::goods::ids::PROVISIONS;
        let space = (stats.provision_capacity - self.provisions).max(0.0);
        if space <= 0.0 {
            return true;
        }

        let stockpile = market.stockpile.get(provisions_id);
        if stockpile <= 0.0 {
            return true;
        }

        let unit_price = market.buy_price(provisions_id, goods).max(0.0001);

        // Chandler credit: if we can't pay cash but have debt
        // headroom (and the port chandler has any silver to lend),
        // take provisions on tick. The advance is sized to one hour's
        // resupply rate — small, repeated calls accumulate naturally
        // for a multi-hour top-up.
        if self.silver < unit_price * RESUPPLY_RATE_PER_HOUR && self.debt < MAX_SHIP_DEBT {
            let target_advance = unit_price * RESUPPLY_RATE_PER_HOUR;
            market.extend_credit(
                self,
                target_advance,
                CHANDLER_PORT_FRACTION_CAP,
                MAX_SHIP_DEBT,
            );
        }

        let affordable = self.silver / unit_price;

        let desired = RESUPPLY_RATE_PER_HOUR
            .min(space)
            .min(stockpile)
            .min(affordable);
        if desired <= 0.0 {
            return true;
        }

        let cost = desired * unit_price;
        self.silver -= cost;
        market.silver += cost;
        market.stockpile.remove(provisions_id, desired);
        self.provisions += desired;

        // Done when full, broke, or market dry. The "broke" case only
        // returns true when we couldn't afford even the next slice —
        // we keep going as long as there's *some* progress this tick.
        let full = self.provisions >= stats.provision_capacity - 1e-4;
        let market_dry = market.stockpile.get(provisions_id) <= 0.0;
        let broke = self.silver < unit_price * 0.05; // less than 5% of an hour's rate
        full || market_dry || broke
    }

    /// Careen the hull for one hour at a port. Returns `true` once the
    /// hull is fully clean.
    pub fn tick_careen(&mut self) -> bool {
        self.hull_fouling = (self.hull_fouling - CAREEN_RATE_PER_HOUR).max(0.0);
        self.hull_fouling <= 0.0
    }

    /// Days of provisions remaining at current consumption rate
    /// (scaled by `crew_alive`).
    pub fn provisions_days_remaining(&self, _stats: &ShipStats) -> f32 {
        let daily = self.daily_provision_burn();
        if daily > 0.0 {
            self.provisions / daily
        } else {
            f32::INFINITY
        }
    }
}

/// Tons of provisions taken on per hour while resupplying at a port.
const RESUPPLY_RATE_PER_HOUR: f32 = 0.5;

/// Fouling points removed per hour while careening at a port.
const CAREEN_RATE_PER_HOUR: f32 = 3.0;

/// Monthly wage per crewman, pesos. Historical reference: an ordinary
/// English seaman c. 1670–1680 earned ~15–25 shillings/month, with a
/// peso worth ~4–5 shillings — giving a baseline of roughly 3 pesos/
/// month. We use 3.0 as the peacetime baseline and add a ~30% Caribbean
/// tropical premium (yellow-fever, hurricane, scurvy risk) for ~4.0
/// pesos/man/month. Dutch and Spanish merchant rates were in the same
/// order of magnitude (2–3 and 4–8 pesos respectively). See
/// `planning/crewing-plan.md §6.1`; calibration in 3.d may revisit.
/// Faction-conditional rates (privateer/pirate share systems, Navy
/// back-pay) land in 3.c.3 or 3.d.
pub const WAGE_PESOS_PER_MAN_MONTH: f32 = 4.0;

/// Sign-on bounty paid per recruit at hire, in pesos. One month's
/// wage per crewing-plan §6.2 — historical contracts paid a month
/// up front to seal the enlistment.
pub const SIGN_ON_BOUNTY_PESOS: f32 = WAGE_PESOS_PER_MAN_MONTH;

// --- Morale (crewing-plan §8) -------------------------------------------------

/// Provisions days remaining below which morale starts to drop at
/// the low rate (§8.1).
pub const MORALE_PROVISIONS_LOW_DAYS: f32 = 14.0;
/// Provisions days remaining below which morale drops at the high rate.
pub const MORALE_PROVISIONS_CRITICAL_DAYS: f32 = 7.0;
/// Hourly morale loss when provisions are merely low (< 14 days).
pub const MORALE_LOSS_PROVISIONS_LOW: f32 = 0.001;
/// Hourly morale loss when provisions are critical (< 7 days). Replaces
/// (does not stack with) the low-provisions rate.
pub const MORALE_LOSS_PROVISIONS_CRITICAL: f32 = 0.005;
/// Hourly morale loss when wages owed exceed 2× the ship's current
/// monthly wage bill.
pub const MORALE_LOSS_WAGES_OVERDUE: f32 = 0.001;
/// Hourly morale gain while docked with provisions fully topped up
/// and no outstanding wage debt (the "rested in port" recovery).
pub const MORALE_GAIN_RESTED_IN_PORT: f32 = 0.001;

/// Maximum outstanding chandler/factor debt a single ship can
/// accumulate before further credit is refused. Sized to cover a
/// few hold-fillings of cheap cargo plus a season's provisions.
pub const MAX_SHIP_DEBT: f32 = 5000.0;

/// Fraction of a port's silver that any single chandler-credit
/// advance may consume. Keeps a string of broke ships from
/// draining a small port's working capital.
pub const CHANDLER_PORT_FRACTION_CAP: f32 = 0.05;
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
    while diff > 180.0 {
        diff -= 360.0;
    }
    while diff < -180.0 {
        diff += 360.0;
    }
    diff
}

/// Normalize angle to [0, 360).
pub fn normalize_angle(mut a: f32) -> f32 {
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
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 };
        // Simulate navigator commanding running speed.
        ship.speed = speed_at_heading(ship.heading, &stats, &wind);
        assert!(ship.effective_speed(&stats, &wind) > 10.0);
    }

    #[test]
    fn test_beating_slow() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.heading = 0.0; // heading north
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: -15.0 }; // from north
                                                    // Simulate navigator commanding the raw upwind hull speed (slow).
        ship.speed = speed_at_heading(ship.heading, &stats, &wind);
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

    #[test]
    fn test_provisions_consumption() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        let stats = ShipStats::sloop();
        let initial = ship.provisions;

        // Tick 24 hours
        for _ in 0..24 {
            ship.tick_resources(&stats);
        }

        let consumed = initial - ship.provisions;
        let expected_daily = stats.daily_provision_consumption();
        assert!(
            (consumed - expected_daily).abs() < 0.001,
            "Expected ~{:.4} tons consumed in a day, got {:.4}",
            expected_daily,
            consumed
        );
    }

    #[test]
    fn fresh_ship_has_full_morale() {
        let ship = Ship::new(Position::ZERO, ShipState::Sailing);
        assert_eq!(ship.morale, 1.0);
        let stats = ShipStats::sloop();
        let built = Ship::freshly_built(
            Position::ZERO,
            0,
            1000.0,
            crate::shiptype::ids::SLOOP,
            &stats,
            Faction::Free,
        );
        assert_eq!(built.morale, 1.0);
    }

    #[test]
    fn morale_drops_on_critical_provisions() {
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        // Force critical: < 7 days at current burn.
        ship.provisions = ship.daily_provision_burn() * 5.0;
        let before = ship.morale;
        ship.tick_morale(&stats);
        assert!((before - ship.morale - MORALE_LOSS_PROVISIONS_CRITICAL).abs() < 1e-5);
    }

    #[test]
    fn morale_drops_on_overdue_wages() {
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        // Plenty of provisions so only the wages branch fires.
        ship.provisions = stats.provision_capacity;
        let monthly = ship.crew_alive as f32 * WAGE_PESOS_PER_MAN_MONTH;
        ship.wages_owed_pesos = 3.0 * monthly;
        let before = ship.morale;
        ship.tick_morale(&stats);
        assert!((before - ship.morale - MORALE_LOSS_WAGES_OVERDUE).abs() < 1e-5);
    }

    #[test]
    fn morale_recovers_in_port_when_fed_and_paid() {
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.provisions = stats.provision_capacity;
        ship.wages_owed_pesos = 0.0;
        ship.morale = 0.5;
        ship.tick_morale(&stats);
        assert!((ship.morale - (0.5 + MORALE_GAIN_RESTED_IN_PORT)).abs() < 1e-5);
    }

    #[test]
    fn morale_band_throttles_speed() {
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 };
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.speed = 10.0;

        ship.morale = 0.8;
        let happy = ship.effective_speed(&stats, &wind);
        ship.morale = 0.3;
        let sullen = ship.effective_speed(&stats, &wind);
        assert!(
            (sullen - happy * 0.8).abs() < 1e-3,
            "happy={happy} sullen={sullen}"
        );
    }

    #[test]
    fn fresh_ship_has_zero_wages_owed() {
        let ship = Ship::new(Position::ZERO, ShipState::Sailing);
        assert_eq!(ship.wages_owed_pesos, 0.0);
        let stats = ShipStats::sloop();
        let built = Ship::freshly_built(
            Position::ZERO,
            0,
            1000.0,
            crate::shiptype::ids::SLOOP,
            &stats,
            Faction::Free,
        );
        assert_eq!(built.wages_owed_pesos, 0.0);
    }

    #[test]
    fn crew_speed_multiplier_piecewise() {
        let stats = ShipStats::sloop();
        let typical = stats.crew_typical(); // 25
        let min = stats.crew_min(); // 10
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);

        // Below min: zero.
        ship.crew_alive = min - 1;
        assert_eq!(ship.crew_speed_multiplier(&stats), 0.0);

        // At min: 60%.
        ship.crew_alive = min;
        assert!((ship.crew_speed_multiplier(&stats) - 0.60).abs() < 1e-3);

        // At 60% of typical: 84%.
        ship.crew_alive = (typical as f32 * 0.6) as u16;
        assert!((ship.crew_speed_multiplier(&stats) - 0.84).abs() < 1e-2);

        // At typical: 100%.
        ship.crew_alive = typical;
        assert!((ship.crew_speed_multiplier(&stats) - 1.00).abs() < 1e-3);

        // Overcrew: still 100%.
        ship.crew_alive = typical + 20;
        assert!((ship.crew_speed_multiplier(&stats) - 1.00).abs() < 1e-3);
    }

    #[test]
    fn provision_burn_scales_with_crew_alive() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.crew_alive = 20;
        let full = ship.daily_provision_burn();
        ship.crew_alive = 10;
        let half = ship.daily_provision_burn();
        assert!((half * 2.0 - full).abs() < 1e-3);
        ship.crew_alive = 0;
        assert_eq!(ship.daily_provision_burn(), 0.0);
    }

    #[test]
    fn test_hull_fouling_speed_penalty() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        let stats = ShipStats::sloop();
        let wind = WindVector { u: 0.0, v: 15.0 }; // from south, running
        ship.speed = speed_at_heading(ship.heading, &stats, &wind);
        let clean_speed = ship.effective_speed(&stats, &wind);

        ship.hull_fouling = 50.0;
        let fouled_speed = ship.effective_speed(&stats, &wind);
        assert!(fouled_speed < clean_speed, "Fouled ship should be slower");
        // 50 fouling = 15% penalty
        let expected_ratio = 1.0 - 50.0 * 0.003;
        let actual_ratio = fouled_speed / clean_speed;
        assert!((actual_ratio - expected_ratio).abs() < 0.01);
    }

    #[test]
    fn test_provisions_days_remaining() {
        let ship = Ship::new(Position::ZERO, ShipState::Sailing);
        let stats = ShipStats::sloop();
        let days = ship.provisions_days_remaining(&stats);
        // 6.0 tons / (25 * 0.0018 tons/day) = ~133 days
        assert!(
            days > 120.0 && days < 140.0,
            "Expected ~133 days, got {}",
            days
        );
    }

    #[test]
    fn test_new_ship_has_empty_cargo() {
        let ship = Ship::new(Position::ZERO, ShipState::Docked);
        assert!(ship.cargo.is_empty());
        assert_eq!(ship.cargo.total_tons(), 0.0);
    }

    #[test]
    fn test_cargo_capacity_is_separate_from_provisions() {
        let stats = ShipStats::sloop();
        // Cargo hold and provisions hold are independent budgets — a fully
        // provisioned ship has its entire trade hold still available.
        assert!(stats.cargo_capacity_tons > 0.0);
        assert!(stats.provision_capacity > 0.0);
        assert!(
            stats.cargo_capacity_tons > stats.provision_capacity,
            "Trade hold should dwarf the provisions hold for a merchant ship"
        );
    }

    #[test]
    fn test_ship_starts_with_silver() {
        let ship = Ship::new(Position::ZERO, ShipState::Docked);
        assert!(ship.silver > 0.0);
    }

    #[test]
    fn test_market_resupply_consumes_silver_and_stockpile() {
        use crate::goods::{ids, GoodsRegistry};
        use crate::market::{PortArchetype, PortMarket};

        let goods = GoodsRegistry::starter();
        let mut market =
            PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.provisions = 0.0; // Empty hold.

        let ship_silver_before = ship.silver;
        let port_silver_before = market.silver;
        let stockpile_before = market.stockpile.get(ids::PROVISIONS);

        // Tick to completion (or 200 hours, whichever first).
        let mut iters = 0;
        while !ship.tick_resupply_at_market(&stats, &mut market, &goods) && iters < 200 {
            iters += 1;
        }

        // Hold should be at (or very near) capacity.
        assert!(
            ship.provisions > stats.provision_capacity * 0.99,
            "expected near-full provisions, got {}",
            ship.provisions
        );
        // Silver moved from ship to port.
        assert!(
            ship.silver < ship_silver_before,
            "ship should have spent silver"
        );
        assert!(
            market.silver > port_silver_before,
            "port should have earned silver"
        );
        // Spent ≈ earned (no leakage; small float drift over many ticks).
        let spent = ship_silver_before - ship.silver;
        let earned = market.silver - port_silver_before;
        assert!(
            (spent - earned).abs() < 0.5,
            "spent {} vs earned {}",
            spent,
            earned
        );
        // Stockpile dropped by ≈ amount loaded.
        assert!(market.stockpile.get(ids::PROVISIONS) < stockpile_before);
    }

    #[test]
    fn test_market_resupply_halts_when_market_dry() {
        use crate::goods::{ids, GoodsRegistry};
        use crate::market::{PortArchetype, PortMarket};

        let goods = GoodsRegistry::starter();
        let mut market =
            PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        // Drain the market.
        let stockpile = market.stockpile.get(ids::PROVISIONS);
        market.stockpile.remove(ids::PROVISIONS, stockpile);

        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.provisions = 0.0;
        let provisions_before = ship.provisions;
        // Single tick should flag done immediately.
        let done = ship.tick_resupply_at_market(&stats, &mut market, &goods);
        assert!(done);
        assert_eq!(
            ship.provisions, provisions_before,
            "no provisions should load when market is dry"
        );
    }
}
