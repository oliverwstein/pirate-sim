use crate::cargo::Cargo;
use crate::money::Pesos;
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
    /// Step 7: number of broadside-firing cannons (one broadside fires
    /// `cannons` guns per tick when in range and supplied). Static per
    /// ship type for now; a later step may promote cannons to a Good.
    pub cannons: u16,
    /// Step 7: maximum hull integrity (HP). Damage saturates at 0; for
    /// Step 7 a hulled ship stays afloat (sinking lands in Step 8).
    pub hull_integrity_max: f32,
    /// Step 7: maximum rigging integrity. Effective speed scales with
    /// `rigging_integrity / rigging_integrity_max` — knocking a chaser's
    /// rigging down is how a slower merchant can break contact.
    pub rigging_integrity_max: f32,
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
            cannons: 8,
            hull_integrity_max: 100.0,
            rigging_integrity_max: 80.0,
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
    /// Step 8: ship has been sunk this tick (hull integrity at 0, or
    /// boarded and burned by a pirate too short-crewed to take a prize).
    /// Sunk ships are skipped by the rest of the hourly loop and removed
    /// from the world by the Cleanup Phase at end-of-tick. The id then
    /// becomes permanently invalid (SlotMap generation bumps).
    Sunk,
}

/// Default starting silver (pesos) for a freshly-spawned merchant ship.
/// Roughly enough to fill its provision hold and bunkers a few times over,
/// and to buy a partial speculative cargo of sugar at base price.
pub const STARTING_SILVER_PESOS: Pesos = Pesos::from_pesos(5000);

/// What a ship is "doing" at the strategic level — drives BT branch
/// selection in Step 6 (pursue vs flee vs trade). Distinct from
/// `ShipState` (Docked/Sailing/Hiring/Anchored), which is the
/// physical/operational state of the hull. Defaults to `Merchant`.
///
/// Step 6 keeps the enum small: only `Merchant` and `Pirate`. Future
/// steps will extend with `Privateer { against: FactionSet }` and
/// `Navy`. Per-ship rather than per-faction because piracy/privateering
/// is a contract held by a captain, not a property of his flag (a Free
/// ship can be merchant; an English ship can be privateer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShipPolicy {
    /// Default: trade, avoid pirates, run for friendly port if threatened.
    #[default]
    Merchant,
    /// Hunt richer/slower merchant prey within visual range.
    Pirate,
}

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
    pub silver: Pesos,
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
    pub starting_silver: Pesos,
    /// Cumulative silver this ship has paid back to its owner port
    /// across all completed voyages. Each time the ship docks at its
    /// `owner_port`, any silver above the operating float is deposited
    /// into the port treasury and added here. True lifetime P/L for a
    /// home-ported ship is `(silver - starting_silver) + lifetime_dividends`.
    pub lifetime_dividends: Pesos,
    /// Total dock events across the ship's life (instrumentation).
    pub lifetime_dock_count: u32,
    /// Outstanding credit drawn from port chandlers/factors —
    /// either provisions taken on tick when broke, or freight cargo
    /// (tramping) advanced against the next sale. Repaid out of any
    /// surplus silver at the next port docking, before dividends.
    /// Settles fungibly across the port network — historically this
    /// is what bills of exchange between merchant correspondents
    /// enabled.
    pub debt: Pesos,
    /// Live head-count. Distinct from `stats.crew_typical()` (the
    /// design complement). Ships need `>= stats.crew_min()` to put
    /// to sea; provisions burn and effective speed scale with this
    /// in Step 3.c. See `planning/crewing-plan.md`.
    pub crew_alive: u16,
    /// Of `crew_alive`, how many are seasoned (low-mortality, fully
    /// proficient) hands — invariant: `crew_seasoned <= crew_alive`.
    /// Tracked here (vs. derived) because the seasoned/unseasoned split
    /// shifts asymmetrically through the ship's lifecycle: hiring draws
    /// seasoned-first from the port pool, casualties take pro-rata
    /// losses, and prize-crew transfers split pro-rata. Reserved as the
    /// hook for combat modifiers (`seasoned_ratio()`); not yet wired
    /// into the gunnery/boarding math. See `planning/crewing-plan.md
    /// §7.3` and `planning/phase-3-postmortem.md §2`.
    pub crew_seasoned: u16,
    /// Wages accrued to the crew but not yet paid. Accrues while at
    /// sea at `crew_alive * WAGE_PESOS_PER_MAN_MONTH / (30 * 24)`
    /// per hour; paid out of `Ship.silver` into the destination
    /// port's market silver on each dock visit. See
    /// `planning/crewing-plan.md §6`.
    pub wages_owed_pesos: Pesos,
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
    /// Strategic policy — Merchant (default), Pirate, etc. Drives the
    /// high-priority pursue/flee branches in the BT (Step 6). Distinct
    /// from `faction`: a Pirate ship can fly any flag. Captured prizes
    /// (Step 8) and bankrupt merchants turning to piracy (Step 9) will
    /// flip this at runtime.
    pub policy: ShipPolicy,
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
    /// Step 7: current hull integrity in `[0, stats.hull_integrity_max]`.
    /// Decremented by broadside hits in the Resolution Phase. A hulled
    /// ship still floats and still fights for Step 7; sinking thresholds
    /// arrive with Step 8 (boarding & sinking).
    pub hull_integrity: f32,
    /// Step 7: current rigging integrity. Multiplies `effective_speed`
    /// by `rigging_integrity / stats.rigging_integrity_max` — a fully-
    /// dismasted ship is dead in the water and easy prize for boarders.
    pub rigging_integrity: f32,
    /// Step 10.b: structural worm damage in `[0, 100]`. Distinct from
    /// `hull_fouling` (which is barnacles/weed and only hurts speed):
    /// teredo eats the planking and drives the foundering hazard roll
    /// in `weather::hazards`. Accumulates fastest in tropical water
    /// and is reduced by careening. See
    /// `planning/research/ship-attrition-economics-1650-1720.md §1.3`.
    pub teredo_damage: f32,
    /// Step 10.b: age in days since seeding/build. Multiplies the
    /// foundering hazard rate so older hulls fail more readily, and
    /// will eventually drive economic obsolescence in later steps.
    pub age_days: u32,
    /// Phase 4 §3a: the absolute `World::sim_minute` at which this
    /// ship is next ready to fire a broadside. Compared against the
    /// sub-tick wall clock by §3b's combat resolver. Default 0 means
    /// "ready immediately"; a fresh ship can fire on its first hour.
    /// Updated to `current_minute + reload_minutes` on every fire.
    pub next_fire_at_minute: u64,
    /// Phase 4 §3c-1 (symmetric redesign): the other ship this one is
    /// currently engaged with. `Some(_)` flips on the first landed
    /// broadside (mutually set on both ships via `World::engage`) and
    /// clears on counterpart sunk / reaped, or on either side
    /// tactically deciding to disengage (BT emits
    /// `Command::Disengage`).
    ///
    /// While engaged, the BT's Engaged subtree is the top-priority
    /// selector, branching each hour between disengage, pursue+fire,
    /// flee+fire, and hold. There is no Attacker/Defender role — both
    /// ships make symmetric tactical judgments from their own world
    /// view (cellular-automaton style).
    pub engaged_with: Option<crate::types::ShipId>,
    /// Phase 4 §3c-1: the absolute `World::sim_minute` at which the
    /// current engagement began. Used by surrender / prize heuristics
    /// in later §3c sub-commits; for now just a record.
    pub engagement_started_at_minute: u64,
    /// Phase 4 §3c-1: cooldown stamp written by `Command::Disengage`.
    /// While `sim_minute < disengaged_until_minute`, `World::engage`
    /// refuses to set `engaged_with` on this ship, preventing the
    /// fire/disengage/re-fire thrash that would otherwise emerge from
    /// see_prey re-targeting the same ship next tick.
    pub disengaged_until_minute: u64,
    /// Phase 4 §3c-2b: when set, this ship is a captured prize in tow
    /// to `Some(victor_id)`. Its AI still runs normally, but a pre-AI
    /// pass each tick copies the victor's `goal.destination` /
    /// `goal.dest_port` into this ship's goal so the prize sails to
    /// the same port as her captor. On docking, a post-AI pass pays
    /// the victor `cargo_silver + hull_bounty` and despawns the prize
    /// (state = Sunk, `prizes_sold` ++). If the victor sinks en route,
    /// the prize is orphaned (`prize_owner` is cleared, the ship
    /// continues with its last-known destination under its own AI;
    /// captor's faction was already stamped at capture). `None` for
    /// normal ships and for prizes resolved through `take` / `sink` /
    /// `release` (those outcomes do not use the tow path).
    pub prize_owner: Option<crate::types::ShipId>,
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
            lifetime_dividends: Pesos::ZERO,
            lifetime_dock_count: 0,
            debt: Pesos::ZERO,
            // Test / seed-fleet ships start fully crewed; the Hiring
            // loop is for shipyard-built hulls only. Seeded ships are
            // veteran crews (100% seasoned) — bench fleets represent
            // established merchant captains.
            crew_alive: stats.crew_typical(),
            crew_seasoned: stats.crew_typical(),
            wages_owed_pesos: Pesos::ZERO,
            morale: 1.0,
            faction: Faction::Free,
            policy: ShipPolicy::Merchant,
            nav: NavTrack::new(),
            dock_action: DockAction::Idle,
            hull_integrity: stats.hull_integrity_max,
            rigging_integrity: stats.rigging_integrity_max,
            teredo_damage: 0.0,
            age_days: 0,
            next_fire_at_minute: 0,
            engaged_with: None,
            engagement_started_at_minute: 0,
            disengaged_until_minute: 0,
            prize_owner: None,
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

    /// Typed variant of `seeded_at_port` — same idea, but lets the
    /// caller pick the ship type and supply per-type stats so the
    /// hull/provisions/crew are sized correctly. Starting silver is
    /// caller-supplied (sized to a hold's worth of cargo at the home
    /// port; the historical-fleet seeder calibrates per-type).
    pub fn seeded_at_port_typed(
        position: Position,
        owner_port: usize,
        faction: Faction,
        ship_type: crate::shiptype::ShipTypeId,
        stats: &ShipStats,
        starting_silver: Pesos,
    ) -> Self {
        Self {
            position,
            heading: 0.0,
            speed: 0.0,
            state: ShipState::Docked,
            provisions: stats.provision_capacity,
            cargo: Cargo::new(),
            hull_fouling: 0.0,
            silver: starting_silver,
            owner_port: Some(owner_port),
            ship_type,
            starting_silver,
            lifetime_dividends: Pesos::ZERO,
            lifetime_dock_count: 0,
            debt: Pesos::ZERO,
            // Seed-fleet ships are fully crewed (no Hiring loop) and
            // are assumed to be veteran crews — see `Ship::new` rationale.
            crew_alive: stats.crew_typical(),
            crew_seasoned: stats.crew_typical(),
            wages_owed_pesos: Pesos::ZERO,
            morale: 1.0,
            faction,
            policy: ShipPolicy::Merchant,
            nav: NavTrack::new(),
            dock_action: DockAction::Idle,
            hull_integrity: stats.hull_integrity_max,
            rigging_integrity: stats.rigging_integrity_max,
            teredo_damage: 0.0,
            age_days: 0,
            next_fire_at_minute: 0,
            engaged_with: None,
            engagement_started_at_minute: 0,
            disengaged_until_minute: 0,
            prize_owner: None,
        }
    }

    /// Construct a ship freshly built at a specific shipyard port, with
    /// a custom amount of starting silver (sized at build time to be
    /// roughly enough to buy one hold of cargo at the home port). The
    /// ship's `owner_port` is set so future remittance logic can find it.
    pub fn freshly_built(
        position: Position,
        owner_port: usize,
        starting_silver: Pesos,
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
            lifetime_dividends: Pesos::ZERO,
            lifetime_dock_count: 0,
            debt: Pesos::ZERO,
            crew_alive: 0,
            crew_seasoned: 0,
            wages_owed_pesos: Pesos::ZERO,
            morale: 1.0,
            faction,
            policy: ShipPolicy::Merchant,
            nav: NavTrack::new(),
            dock_action: DockAction::Idle,
            hull_integrity: stats.hull_integrity_max,
            rigging_integrity: stats.rigging_integrity_max,
            teredo_damage: 0.0,
            age_days: 0,
            next_fire_at_minute: 0,
            engaged_with: None,
            engagement_started_at_minute: 0,
            disengaged_until_minute: 0,
            prize_owner: None,
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
        self.lifetime_dock_count = self.lifetime_dock_count.saturating_add(1);
    }

    /// Anchor at current position.
    pub fn anchor(&mut self) {
        self.state = ShipState::Anchored;
        self.speed = 0.0;
    }

    /// Velocity vector in NM/hr (= knots), derived from current
    /// `(heading, speed)`. Used by the Resolution Phase to compute
    /// closest-approach distance between ships over the hour-long tick
    /// — see `combat::min_distance_over_tick`. Returns `(vx, vy)` in
    /// the same axes as `position` (x = East, y = North).
    pub fn velocity(&self) -> (f32, f32) {
        // heading is degrees CW from North: 0=N, 90=E.
        // So vx (east) = speed * sin(heading), vy (north) = speed * cos(heading).
        let h = self.heading.to_radians();
        (self.speed * h.sin(), self.speed * h.cos())
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
        // Step 7: rigging damage proportionally caps speed. A ship with
        // half its rigging shot away makes half its rigged speed.
        let rigging_mult = if stats.rigging_integrity_max > 0.0 {
            (self.rigging_integrity / stats.rigging_integrity_max).clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.speed * fouling_penalty * crew_mult * morale_mult * rigging_mult
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

    /// Fraction of the live crew that are seasoned hands, in `[0, 1]`.
    /// Returns 0.0 when there is no crew (so the ratio is well-defined
    /// at edge cases). Reserved as the input to combat modifiers
    /// (gunnery rate, boarding power) — see `planning/crewing-plan.md
    /// §7.3`. Wired now via `apply_crew_losses` and `detach_prize_crew`
    /// so the value is meaningful before the modifiers land.
    pub fn seasoned_ratio(&self) -> f32 {
        if self.crew_alive == 0 {
            return 0.0;
        }
        (self.crew_seasoned as f32 / self.crew_alive as f32).clamp(0.0, 1.0)
    }

    /// Apply `losses` to the crew, splitting them pro-rata between
    /// seasoned and unseasoned. Losses larger than `crew_alive`
    /// saturate. Preserves the `crew_seasoned <= crew_alive` invariant.
    ///
    /// Pro-rata is the right v1 model for boarding casualties: a
    /// pike-thrust through a pressed landsman is as fatal as one
    /// through a Spanish *maestre*, and the historical accounts
    /// (Earle, Rediker) don't suggest officers/seasoned hands were
    /// systematically spared during boardings. When v2 introduces
    /// melee command (officers leading from the front), this can
    /// shift to "seasoned slightly over-represented in losses."
    pub fn apply_crew_losses(&mut self, losses: u16) {
        if losses == 0 || self.crew_alive == 0 {
            return;
        }
        let losses = losses.min(self.crew_alive);
        // Integer pro-rata: rounds toward zero, which (slightly) biases
        // losses toward unseasoned. Acceptable for v1 and avoids any
        // floating-point determinism concern.
        let seasoned_loss =
            ((self.crew_seasoned as u32 * losses as u32) / (self.crew_alive as u32).max(1)) as u16;
        self.crew_alive -= losses;
        self.crew_seasoned = self
            .crew_seasoned
            .saturating_sub(seasoned_loss)
            .min(self.crew_alive);
    }

    /// Detach `amount` crew from this ship, pro-rata over seasoned.
    /// Returns `(detached_alive, detached_seasoned)` so the caller can
    /// credit the recipient ship (prize crew transfer). Saturates at
    /// `crew_alive`. Preserves the invariant on both ships when the
    /// caller mirrors the (alive, seasoned) pair on the recipient.
    pub fn detach_prize_crew(&mut self, amount: u16) -> (u16, u16) {
        if amount == 0 || self.crew_alive == 0 {
            return (0, 0);
        }
        let detached = amount.min(self.crew_alive);
        let detached_seasoned = ((self.crew_seasoned as u32 * detached as u32)
            / (self.crew_alive as u32).max(1)) as u16;
        self.crew_alive -= detached;
        self.crew_seasoned = self
            .crew_seasoned
            .saturating_sub(detached_seasoned)
            .min(self.crew_alive);
        (detached, detached_seasoned)
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
        let monthly_bill = WAGE_PESOS_PER_MAN_MONTH.scale(self.crew_alive as f32);
        if monthly_bill.is_positive() && self.wages_owed_pesos > monthly_bill + monthly_bill {
            delta -= MORALE_LOSS_WAGES_OVERDUE;
        }

        // Heavy debt: chandler credit blown past (or sitting at) the ceiling.
        if self.debt >= MAX_SHIP_DEBT {
            delta -= MORALE_LOSS_DEBT_HEAVY;
        }

        // Rested in port: docked, full belly, no wage debt.
        if self.state == ShipState::Docked
            && days_left >= MORALE_PROVISIONS_LOW_DAYS
            && self.wages_owed_pesos <= Pesos::ZERO
        {
            delta += MORALE_GAIN_RESTED_IN_PORT;
        }

        self.morale = (self.morale + delta).clamp(0.0, 1.0);
    }

    /// Check the mutiny trigger (Step 9, retuned in Step 11.b): a
    /// Merchant ship currently at sea, whose combined chandler debt +
    /// overdue wages exceeds `MUTINY_DEBT_THRESHOLD`, with morale
    /// below `MUTINY_MORALE_THRESHOLD`, gets one stochastic roll per
    /// hour with probability `MUTINY_PROBABILITY_PER_HOUR`. Returns
    /// `true` if the policy flipped this tick. The caller is
    /// responsible for clearing any merchant-route NavGoal on the
    /// ShipAI so the new pirate captain re-plans next tick.
    ///
    /// Combining `debt` and `wages_owed_pesos` matters because the
    /// chandler caps `debt` at `MAX_SHIP_DEBT`: a captain who maxes
    /// out chandler credit AND stops paying his crew is the realistic
    /// distress profile, not raw debt alone.
    ///
    /// `mutiny_roll` must be a uniform sample in `[0, 1)` drawn from
    /// the world's combat RNG; pass `combat_rng_step(&mut state)` at
    /// the call site so the result is deterministic.
    ///
    /// Privateers / already-Pirate ships and any non-Sailing state are
    /// ignored.
    pub fn try_mutiny(&mut self, mutiny_roll: f32) -> bool {
        if self.policy != ShipPolicy::Merchant {
            return false;
        }
        if self.state != ShipState::Sailing {
            return false;
        }
        let financial_distress = self.debt + self.wages_owed_pesos.max_zero();
        if financial_distress <= MUTINY_DEBT_THRESHOLD {
            return false;
        }
        if self.morale >= MUTINY_MORALE_THRESHOLD {
            return false;
        }
        if mutiny_roll >= MUTINY_PROBABILITY_PER_HOUR {
            return false;
        }
        self.policy = ShipPolicy::Pirate;
        // The new pirate crew torches the ship's books — debt to the
        // chandler ashore and unpaid wages are no longer their problem.
        self.debt = Pesos::ZERO;
        self.wages_owed_pesos = Pesos::ZERO;
        self.morale = MUTINY_POST_FLIP_MORALE;
        true
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
    /// from the port's balance, paying out of `self.silver` at the
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

        let available = market.available_to_buy(provisions_id);
        if available <= 0.0 {
            return true;
        }

        let unit_price = market.price_at(provisions_id, goods).max(0.0001);

        // Chandler credit: if we can't pay cash but have debt
        // headroom (and the port chandler has any silver to lend),
        // take provisions on tick. The advance is sized to one hour's
        // resupply rate — small, repeated calls accumulate naturally
        // for a multi-hour top-up.
        let hour_bill = Pesos::from_pesos_f32(unit_price * RESUPPLY_RATE_PER_HOUR);
        if self.silver < hour_bill && self.debt < MAX_SHIP_DEBT {
            market.extend_credit(self, hour_bill, CHANDLER_PORT_FRACTION_CAP, MAX_SHIP_DEBT);
        }

        let affordable = self.silver.as_pesos_f32() / unit_price;

        let desired = RESUPPLY_RATE_PER_HOUR
            .min(space)
            .min(available)
            .min(affordable);
        if desired <= 0.0 {
            return true;
        }

        let cost = Pesos::from_pesos_f32(desired * unit_price);
        self.silver -= cost;
        market.silver += cost;
        let bound = market.effective_bound(provisions_id);
        market.balance.set(
            provisions_id,
            (market.balance.get(provisions_id) - desired.ceil() as i32).clamp(-bound, bound),
        );
        self.provisions += desired;

        // Done when full, broke, or market dry. The "broke" case only
        // returns true when we couldn't afford even the next slice —
        // we keep going as long as there's *some* progress this tick.
        let full = self.provisions >= stats.provision_capacity - 1e-4;
        let market_dry = market.available_to_buy(provisions_id) <= 0.0;
        let broke = self.silver.as_pesos_f32() < unit_price * 0.05; // less than 5% of an hour's rate
        full || market_dry || broke
    }

    /// Careen the hull for one hour at a port. Returns `true` once the
    /// hull is fully clean. Reduces both fouling (barnacles/weed) and
    /// teredo damage (worm-eaten planking) — the latter at half the
    /// rate, since structural plank replacement is slower work than
    /// scraping the bottom. See
    /// `planning/research/ship-attrition-economics-1650-1720.md §1.3`.
    pub fn tick_careen(&mut self) -> bool {
        self.hull_fouling = (self.hull_fouling - CAREEN_RATE_PER_HOUR).max(0.0);
        self.teredo_damage = (self.teredo_damage - CAREEN_RATE_PER_HOUR * 0.5).max(0.0);
        self.hull_fouling <= 0.0
    }

    /// Phase 4 §2: repair combat / storm damage to the hull for one
    /// hour at a docked port. Restores up to `HULL_REPAIR_RATE_PER_HOUR`
    /// HP and bills `HULL_REPAIR_COST_PESOS_PER_HP` per HP from
    /// `silver`; any shortfall is recorded as drydock debt on
    /// `Ship::debt` (which composes with the wage / chandler debt
    /// machinery already in place — bankruptcy threshold, shipyard
    /// scrap, mutiny pressure). Returns `true` when the hull is at
    /// (or above) `stats.hull_integrity_max`.
    ///
    /// Carpenters work whether or not the captain can pay — bills
    /// accrue as debt. This matches the historical practice: dockyard
    /// pursers extended credit to known masters, and pursued unpaid
    /// accounts through the admiralty courts.
    pub fn tick_repair_hull(&mut self, stats: &ShipStats) -> bool {
        let deficit = (stats.hull_integrity_max - self.hull_integrity).max(0.0);
        if deficit <= 0.0 {
            return true;
        }
        let restored = deficit.min(HULL_REPAIR_RATE_PER_HOUR);
        self.hull_integrity += restored;
        let bill = HULL_REPAIR_COST_PESOS_PER_HP.scale(restored);
        let paid = bill.min(self.silver.max_zero());
        self.silver -= paid;
        self.debt += bill - paid;
        self.hull_integrity >= stats.hull_integrity_max
    }

    /// Phase 4 §2: one-shot rigging top-off applied on undock. Rigging
    /// damage is purely a combat reserve — sails, cordage, spars are
    /// bo's'n stores carried aboard and replaced wholesale during a
    /// port turnaround. Charges `RIGGING_REPAIR_COST_PESOS_PER_HP`
    /// per HP restored from silver; any shortfall accrues to debt
    /// (same rules as `tick_repair_hull`). Returns the HP delta
    /// restored.
    pub fn top_off_rigging(&mut self, stats: &ShipStats) -> f32 {
        let deficit = (stats.rigging_integrity_max - self.rigging_integrity).max(0.0);
        if deficit <= 0.0 {
            return 0.0;
        }
        self.rigging_integrity = stats.rigging_integrity_max;
        let bill = RIGGING_REPAIR_COST_PESOS_PER_HP.scale(deficit);
        let paid = bill.min(self.silver.max_zero());
        self.silver -= paid;
        self.debt += bill - paid;
        deficit
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
pub const RESUPPLY_RATE_PER_HOUR: f32 = 0.5;

/// Fouling points removed per hour while careening at a port.
const CAREEN_RATE_PER_HOUR: f32 = 3.0;

/// Phase 4 §2: hull HP restored per hour at a docked port. Sized so a
/// 100-HP hull rebuild takes ~14 days (≈ 333 h), aligned with the
/// historical 3–6 week refit cycle for a battle-damaged 4th-rate.
pub const HULL_REPAIR_RATE_PER_HOUR: f32 = 0.3;

/// Phase 4 §2: silver cost per HP of hull repair. A full 100-HP rebuild
/// for a sloop is ~600 pesos — about 30% of the build cost, matching
/// the Royal Navy's "great repair" line-item ratios for the era.
pub const HULL_REPAIR_COST_PESOS_PER_HP: Pesos = Pesos::from_pesos(6);

/// Phase 4 §2: silver cost per HP of rigging restored at undock. Cheap
/// — cordage + sailcloth come from bo's'n stores which the ship
/// already carries; the charge models the port chandler's mark-up on
/// resupply. A fully-dismasted sloop's rigging (80 HP) costs ~120
/// pesos to replace, well within a single voyage's profit.
pub const RIGGING_REPAIR_COST_PESOS_PER_HP: Pesos = Pesos::from_centavos(150);

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
pub const WAGE_PESOS_PER_MAN_MONTH: Pesos = Pesos::from_pesos(4);

/// Sign-on bounty paid per recruit at hire, in pesos. One month's
/// wage per crewing-plan §6.2 — historical contracts paid a month
/// up front to seal the enlistment.
pub const SIGN_ON_BOUNTY_PESOS: Pesos = WAGE_PESOS_PER_MAN_MONTH;

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
/// Hourly morale loss while the ship is carrying chandler debt above
/// `MAX_SHIP_DEBT` (i.e., the captain has run out of legitimate credit
/// and is shipping freight on tramping terms). Sized to push a chronically
/// indebted crew toward the mutiny threshold over weeks, not hours.
pub const MORALE_LOSS_DEBT_HEAVY: f32 = 0.0015;
/// One-shot morale boost applied to a boarding attacker when they
/// successfully take a prize (Step 8). Models the lift from prize-share
/// distribution. Applied in the AttemptBoard Resolution arm.
pub const MORALE_GAIN_PRIZE_TAKEN: f32 = 0.30;

/// Maximum outstanding chandler/factor debt a single ship can
/// accumulate before further credit is refused. Sized to cover a
/// few hold-fillings of cheap cargo plus a season's provisions.
pub const MAX_SHIP_DEBT: Pesos = Pesos::from_pesos(5000);

/// Debt threshold above which an at-sea Merchant crew may mutiny
/// (§9). Step 11.b: raised from 1.5× to 3× `MAX_SHIP_DEBT` to match
/// the historical record where significant mutinies (Royal James,
/// Anstis, Phillips) were rare events at the *end* of a long string
/// of failures, not first-sign-of-trouble flips. With the chandler
/// debt cap at MAX_SHIP_DEBT, the only way to reach this threshold
/// is to also be carrying ~10000 pesos in unpaid wages — roughly a
/// year's wages for a small merchant crew.
pub const MUTINY_DEBT_THRESHOLD: Pesos = Pesos::from_pesos(15000);
/// Morale ceiling below which an at-sea Merchant with crushing debt
/// flips to Pirate.
pub const MUTINY_MORALE_THRESHOLD: f32 = 0.25;
/// Morale baseline assigned to a fresh pirate crew after a mutiny —
/// they're not euphoric (they just murdered their officers) but the
/// immediate grievances are gone.
pub const MUTINY_POST_FLIP_MORALE: f32 = 0.55;

/// Step 11.b: per-hour probability that a qualifying crew (debt +
/// wages over threshold, morale below threshold, at sea) actually
/// mutinies. At ~0.0002/hr a ship that stays in the mutiny zone for
/// a full year has a ~1 − exp(−24 × 365 × 0.0002) ≈ 83% chance of
/// flipping; over a typical week-or-two distress window the
/// per-incident chance is more like 3-7%. Models the "everyone's
/// grumbling but it takes weeks of conspiring" dynamic seen in
/// Rediker's accounts.
pub const MUTINY_PROBABILITY_PER_HOUR: f32 = 0.0002;

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
            Pesos::from_pesos(1000),
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
        let monthly = WAGE_PESOS_PER_MAN_MONTH.scale(ship.crew_alive as f32);
        ship.wages_owed_pesos = monthly + monthly + monthly;
        let before = ship.morale;
        ship.tick_morale(&stats);
        assert!((before - ship.morale - MORALE_LOSS_WAGES_OVERDUE).abs() < 1e-5);
    }

    #[test]
    fn morale_recovers_in_port_when_fed_and_paid() {
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.provisions = stats.provision_capacity;
        ship.wages_owed_pesos = Pesos::ZERO;
        ship.morale = 0.5;
        ship.tick_morale(&stats);
        assert!((ship.morale - (0.5 + MORALE_GAIN_RESTED_IN_PORT)).abs() < 1e-5);
    }

    #[test]
    fn morale_drops_on_heavy_debt() {
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.provisions = stats.provision_capacity;
        ship.wages_owed_pesos = Pesos::ZERO;
        ship.debt = MAX_SHIP_DEBT;
        let before = ship.morale;
        ship.tick_morale(&stats);
        assert!((before - ship.morale - MORALE_LOSS_DEBT_HEAVY).abs() < 1e-5);
    }

    #[test]
    fn mutiny_flips_indebted_low_morale_merchant_at_sea() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.policy = ShipPolicy::Merchant;
        ship.debt = MUTINY_DEBT_THRESHOLD + Pesos::from_pesos(1);
        ship.morale = MUTINY_MORALE_THRESHOLD - 0.01;
        ship.wages_owed_pesos = Pesos::from_pesos(999);
        // Pass a roll guaranteed to fall under the probability gate.
        assert!(ship.try_mutiny(0.0));
        assert_eq!(ship.policy, ShipPolicy::Pirate);
        assert_eq!(ship.debt, Pesos::ZERO);
        assert_eq!(ship.wages_owed_pesos, Pesos::ZERO);
        assert!((ship.morale - MUTINY_POST_FLIP_MORALE).abs() < 1e-5);
    }

    #[test]
    fn mutiny_does_not_trigger_when_docked() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.debt = MUTINY_DEBT_THRESHOLD + Pesos::from_pesos(1000);
        ship.morale = 0.0;
        assert!(!ship.try_mutiny(0.0));
        assert_eq!(ship.policy, ShipPolicy::Merchant);
    }

    #[test]
    fn mutiny_does_not_trigger_below_thresholds() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.debt = MUTINY_DEBT_THRESHOLD - Pesos::from_pesos(1);
        ship.wages_owed_pesos = Pesos::ZERO;
        ship.morale = 0.05;
        assert!(!ship.try_mutiny(0.0), "below distress threshold");
        ship.debt = MUTINY_DEBT_THRESHOLD + Pesos::from_pesos(1);
        ship.morale = MUTINY_MORALE_THRESHOLD + 0.01;
        assert!(!ship.try_mutiny(0.0), "above morale threshold");
    }

    #[test]
    fn mutiny_triggers_on_wages_pushing_total_over_threshold() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        // Maxed out chandler credit + heavy unpaid wages combine to
        // push the crew over the edge.
        ship.debt = MAX_SHIP_DEBT;
        ship.wages_owed_pesos = MUTINY_DEBT_THRESHOLD - MAX_SHIP_DEBT + Pesos::from_pesos(1);
        ship.morale = 0.1;
        assert!(ship.try_mutiny(0.0));
        assert_eq!(ship.policy, ShipPolicy::Pirate);
    }

    #[test]
    fn mutiny_ignores_already_pirate_ships() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.policy = ShipPolicy::Pirate;
        ship.debt = MUTINY_DEBT_THRESHOLD + MUTINY_DEBT_THRESHOLD;
        ship.morale = 0.0;
        assert!(!ship.try_mutiny(0.0));
    }

    /// Postmortem §2 / crewing-plan §7.3: pro-rata casualty splits keep
    /// the seasoned ratio sensible across attrition. A ship that starts
    /// half-seasoned and loses a quarter of its crew should still be
    /// roughly half-seasoned (within integer rounding).
    #[test]
    fn apply_crew_losses_keeps_seasoned_ratio() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.crew_alive = 40;
        ship.crew_seasoned = 20;
        let ratio_before = ship.seasoned_ratio();
        ship.apply_crew_losses(10);
        assert_eq!(ship.crew_alive, 30);
        // 20 * 10 / 40 = 5 seasoned lost -> 15 seasoned remain.
        assert_eq!(ship.crew_seasoned, 15);
        // Ratio held to within 1 head (integer rounding).
        assert!((ship.seasoned_ratio() - ratio_before).abs() < 1.0 / 30.0);
    }

    /// Saturating losses must not violate the invariant
    /// `crew_seasoned <= crew_alive`.
    #[test]
    fn apply_crew_losses_saturates_and_preserves_invariant() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.crew_alive = 5;
        ship.crew_seasoned = 5;
        ship.apply_crew_losses(100);
        assert_eq!(ship.crew_alive, 0);
        assert_eq!(ship.crew_seasoned, 0);
    }

    /// Prize-crew detachment splits seasoned pro-rata; the caller can
    /// then credit the recipient with the same `(alive, seasoned)`
    /// pair and the invariant holds on both ships.
    #[test]
    fn detach_prize_crew_splits_seasoned_pro_rata() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.crew_alive = 20;
        ship.crew_seasoned = 10;
        let (alive, seasoned) = ship.detach_prize_crew(8);
        // 10 * 8 / 20 = 4 seasoned in the prize crew.
        assert_eq!((alive, seasoned), (8, 4));
        assert_eq!(ship.crew_alive, 12);
        assert_eq!(ship.crew_seasoned, 6);
    }

    /// Step 11.b: the per-hour probability gate suppresses mutinies
    /// when the random roll is above the configured threshold, even
    /// when every other condition is met. This is what keeps the
    /// long-run mutiny rate sane.
    #[test]
    fn mutiny_skips_when_roll_above_probability() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.policy = ShipPolicy::Merchant;
        ship.debt = MUTINY_DEBT_THRESHOLD + Pesos::from_pesos(1);
        ship.morale = 0.0;
        // Just above the probability gate -> no flip.
        assert!(!ship.try_mutiny(MUTINY_PROBABILITY_PER_HOUR + 1e-6));
        assert_eq!(ship.policy, ShipPolicy::Merchant);
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
        assert_eq!(ship.wages_owed_pesos, Pesos::ZERO);
        let stats = ShipStats::sloop();
        let built = Ship::freshly_built(
            Position::ZERO,
            0,
            Pesos::from_pesos(1000),
            crate::shiptype::ids::SLOOP,
            &stats,
            Faction::Free,
        );
        assert_eq!(built.wages_owed_pesos, Pesos::ZERO);
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
        assert!(ship.silver > Pesos::ZERO);
    }

    #[test]
    fn test_market_resupply_consumes_silver_and_balance() {
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
        let balance_before = market.balance.get(ids::PROVISIONS);

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
        // Spent ≈ earned (no leakage; exact integer ledger).
        let spent = ship_silver_before - ship.silver;
        let earned = market.silver - port_silver_before;
        assert_eq!(spent, earned, "spent {} vs earned {}", spent, earned);
        // Stockpile dropped by ≈ amount loaded.
        assert!(market.balance.get(ids::PROVISIONS) < balance_before);
    }

    #[test]
    fn test_market_resupply_halts_when_market_dry() {
        use crate::goods::{ids, GoodsRegistry};
        use crate::market::{PortArchetype, PortMarket};

        let goods = GoodsRegistry::starter();
        let mut market =
            PortMarket::with_recipe(&goods, PortArchetype::NorthAmericanFarming.recipe());
        // Drain the market.
        let bound = market.effective_bound(ids::PROVISIONS);
        market.balance.set(ids::PROVISIONS, -bound);

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

    // ── Phase 4 §2 — repair at port ─────────────────────────────────

    #[test]
    fn tick_repair_hull_restores_at_documented_rate_and_charges_silver() {
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.silver = Pesos::from_pesos(10_000);
        ship.hull_integrity = stats.hull_integrity_max - 1.0;

        let silver_before = ship.silver;
        let hull_before = ship.hull_integrity;
        let done = ship.tick_repair_hull(&stats);

        let restored = ship.hull_integrity - hull_before;
        assert!(
            (restored - HULL_REPAIR_RATE_PER_HOUR).abs() < 1e-4
                || (ship.hull_integrity - stats.hull_integrity_max).abs() < 1e-4,
            "expected ~{} HP restored, got {}",
            HULL_REPAIR_RATE_PER_HOUR,
            restored
        );
        let paid = silver_before - ship.silver;
        let expected = HULL_REPAIR_COST_PESOS_PER_HP.scale(restored);
        assert_eq!(
            paid, expected,
            "billed {} for {} HP; expected {}",
            paid, restored, expected
        );
        assert!(!done || (ship.hull_integrity - stats.hull_integrity_max).abs() < 1e-4);
    }

    #[test]
    fn tick_repair_hull_caps_at_max_and_reports_done() {
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.silver = Pesos::from_pesos(10_000);
        // Damage less than one tick of repair.
        ship.hull_integrity = stats.hull_integrity_max - 0.01;
        let done = ship.tick_repair_hull(&stats);
        assert!(done);
        assert!((ship.hull_integrity - stats.hull_integrity_max).abs() < 1e-4);

        // Already full → no-op, no charge.
        let silver = ship.silver;
        let done2 = ship.tick_repair_hull(&stats);
        assert!(done2);
        assert_eq!(ship.silver, silver);
        assert_eq!(ship.debt, Pesos::ZERO);
    }

    #[test]
    fn tick_repair_hull_insufficient_silver_creates_debt_not_partial_freebie() {
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        let starting = Pesos::from_centavos(50);
        ship.silver = starting; // Less than one HP of hull at 6 pesos/HP.
        ship.debt = Pesos::ZERO;
        ship.hull_integrity = stats.hull_integrity_max - 1.0;

        let hull_before = ship.hull_integrity;
        ship.tick_repair_hull(&stats);
        let restored = ship.hull_integrity - hull_before;
        let bill = HULL_REPAIR_COST_PESOS_PER_HP.scale(restored);

        // Full HP delta was applied — the carpenters worked.
        assert!(restored > 0.0);
        // Silver drained.
        assert_eq!(ship.silver, Pesos::ZERO);
        // Remainder is debt, not a freebie.
        assert_eq!(
            ship.debt,
            bill - starting,
            "debt {} should be bill {} minus the starting silver {}",
            ship.debt,
            bill,
            starting
        );
    }

    #[test]
    fn top_off_rigging_one_shot_full_restore() {
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.silver = Pesos::from_pesos(10_000);
        ship.rigging_integrity = stats.rigging_integrity_max * 0.5;

        let silver_before = ship.silver;
        let delta = ship.top_off_rigging(&stats);

        let expected_delta = stats.rigging_integrity_max * 0.5;
        assert!((delta - expected_delta).abs() < 1e-3);
        assert!((ship.rigging_integrity - stats.rigging_integrity_max).abs() < 1e-4);
        let expected_bill = RIGGING_REPAIR_COST_PESOS_PER_HP.scale(expected_delta);
        assert_eq!(silver_before - ship.silver, expected_bill);
    }

    #[test]
    fn top_off_rigging_full_health_is_noop() {
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.silver = Pesos::from_pesos(10_000);
        let silver = ship.silver;
        let delta = ship.top_off_rigging(&stats);
        assert_eq!(delta, 0.0);
        assert_eq!(ship.silver, silver);
        assert_eq!(ship.debt, Pesos::ZERO);
    }

    #[test]
    fn top_off_rigging_insufficient_silver_goes_to_debt() {
        let stats = ShipStats::sloop();
        let mut ship = Ship::new(Position::ZERO, ShipState::Docked);
        ship.silver = Pesos::ZERO;
        ship.debt = Pesos::ZERO;
        ship.rigging_integrity = 0.0;

        let delta = ship.top_off_rigging(&stats);
        assert!((delta - stats.rigging_integrity_max).abs() < 1e-3);
        assert!((ship.rigging_integrity - stats.rigging_integrity_max).abs() < 1e-4);
        let expected_debt = RIGGING_REPAIR_COST_PESOS_PER_HP.scale(stats.rigging_integrity_max);
        assert_eq!(ship.debt, expected_debt);
    }

    #[test]
    fn apply_crew_losses_pro_rata_preserves_invariant() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.crew_alive = 100;
        ship.crew_seasoned = 40;
        ship.apply_crew_losses(50);
        assert_eq!(ship.crew_alive, 50);
        // 40/100 of 50 = 20 seasoned losses → 20 seasoned remain.
        assert_eq!(ship.crew_seasoned, 20);
        assert!(ship.crew_seasoned <= ship.crew_alive);
    }

    #[test]
    fn apply_crew_losses_saturates_at_alive_and_zeroes_seasoned() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.crew_alive = 20;
        ship.crew_seasoned = 5;
        ship.apply_crew_losses(999);
        assert_eq!(ship.crew_alive, 0);
        assert_eq!(ship.crew_seasoned, 0);
    }

    #[test]
    fn detach_prize_crew_returns_pro_rata_split() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.crew_alive = 50;
        ship.crew_seasoned = 20;
        let (alive, seasoned) = ship.detach_prize_crew(10);
        assert_eq!(alive, 10);
        // 20/50 of 10 = 4 seasoned detached → 16 seasoned remain.
        assert_eq!(seasoned, 4);
        assert_eq!(ship.crew_alive, 40);
        assert_eq!(ship.crew_seasoned, 16);
        assert!(ship.crew_seasoned <= ship.crew_alive);
    }

    #[test]
    fn seasoned_ratio_handles_empty_crew() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.crew_alive = 0;
        ship.crew_seasoned = 0;
        assert_eq!(ship.seasoned_ratio(), 0.0);
    }

    #[test]
    fn seasoned_ratio_is_a_proper_fraction() {
        let mut ship = Ship::new(Position::ZERO, ShipState::Sailing);
        ship.crew_alive = 80;
        ship.crew_seasoned = 20;
        assert!((ship.seasoned_ratio() - 0.25).abs() < 1e-4);
    }
}
