//! AI layer: uses a behavior tree to decide ship actions each tick.
//!
//! The BT structure:
//!   Selector [
//!     Sequence [Condition(IsDocked), dock_tree],          // Handle port activities
//!     Sequence [Condition(IsLowProvisions), Action(DivertToPort)], // Emergency resupply
//!     Sequence [Condition(HasDestination), Action(Sail)], // Navigate to goal
//!     Action(ChooseDestination),                          // Pick somewhere to go
//!   ]
//!
//! dock_tree = Sequence [
//!     Action(Resupply),   // Running until full, then Success
//!     Action(Careen),     // Running until clean, then Success
//!     Action(Undock),     // Undock and succeed
//! ]

use crate::bt::{self, Behavior, BtContext, BtState, Status};
use crate::command::ShipCommand;
use crate::goods::GoodsRegistry;
use crate::harbor::HarborMap;
use crate::market::PortMarket;
use crate::nav::{self, NavGoal};
use crate::pathfind::{self, PathfindContext};
use crate::port::{Faction, Port};
use crate::ship::{
    angle_diff, normalize_angle, speed_at_heading, DockAction, Ship, ShipPolicy, ShipState,
    ShipStats,
};
use crate::sim_rng::SimRng;
use crate::spatial::SpatialHash;
use crate::types::{Position, ShipId, WindVector};
use slotmap::SecondaryMap;

// --- Action IDs ---
const ACT_SAIL: usize = 0;
const ACT_RESUPPLY: usize = 1;
const ACT_CAREEN: usize = 2;
const ACT_UNDOCK: usize = 3;
const ACT_CHOOSE_DESTINATION: usize = 4;
const ACT_DIVERT_TO_PORT: usize = 5;
const ACT_SELL_ALL: usize = 6;
const ACT_BUY_BEST: usize = 7;
const ACT_PURSUE: usize = 8;
const ACT_FLEE: usize = 9;
// Phase 4 §3c-1 (symmetric redesign): engaged-subtree leaf actions.
// ACT_DISENGAGE emits Command::Disengage to mutually clear the
// engagement flag and start a cooldown. ACT_HOLD is a no-op (no Steer
// command; the ship coasts on its previous heading while it reloads).
const ACT_DISENGAGE: usize = 10;
const ACT_HOLD: usize = 11;
// Phase 4 §3c-2: ACT_STRIKE emits Command::Strike { to: engaged_with }
// — the surrendering ship is the prize, and the victor is its current
// engagement counterpart. The world's command resolver dispatches the
// surrendered hull through the shared prize-action machinery (take /
// sell / sink / release).
const ACT_STRIKE: usize = 12;
// Phase 4 §3c-3: ACT_BOARD commits a pirate to closing on a
// rigging-crippled engaged counterpart and emitting `AttemptBoard`.
// Distinct from `act_pursue` only by where it sits in the engaged
// subtree priority — placing the *decision to board* above the
// fight/flee branches lets a pirate take a crippled prize even when
// his magazine is empty (the old `should_fight` gate required
// ordnance and silently routed magazine-empty pirates into flee).
const ACT_BOARD: usize = 13;

// --- Condition IDs ---
const COND_IS_DOCKED: usize = 0;
const COND_HAS_DESTINATION: usize = 1;
const COND_IS_LOW_PROVISIONS: usize = 2;
const COND_IS_SAILING_PIRATE: usize = 3;
const COND_IS_SAILING_MERCHANT: usize = 4;
const COND_SEE_PREY: usize = 5;
const COND_SEE_THREAT: usize = 6;
// Phase 4 §3c-1 (symmetric redesign): engaged-subtree conditions.
// COND_IS_ENGAGED gates the whole engaged subtree; the three tactical
// conditions below answer "what should I do this hour?" using only
// own state plus visible-ship snapshots (CA-style — every ship makes
// its own decision, no roles, no global authority). Each tactical
// condition also writes `goal.pursue_target` / `goal.flee_from` as a
// side effect so the existing ACT_PURSUE / ACT_FLEE handlers reach
// the engaged counterpart.
const COND_IS_ENGAGED: usize = 7;
const COND_SHOULD_DISENGAGE: usize = 8;
const COND_SHOULD_FIGHT: usize = 9;
const COND_SHOULD_FLEE: usize = 10;
// Phase 4 §3c-2: COND_SHOULD_STRIKE fires when the ship's own
// morale × hull_frac has collapsed below the strike threshold AND it
// is in no position to outrun the counterpart (catastrophic hull, no
// ordnance, or counterpart visibly stronger). Priority is *above*
// disengage so a hopelessly-beaten ship surrenders rather than tries
// to break off into a hail of fire.
const COND_SHOULD_STRIKE: usize = 11;
// Phase 4 §3c-3: COND_SHOULD_BOARD fires when this ship is a Pirate
// (only pirates board in v1), is engaged with a counterpart whose
// rigging is below the boarding threshold, and has at least two crew
// alive (the minimum boarding party). Side effect on success: sets
// `goal.pursue_target = engaged_with` so `act_board` (which reads
// from pursue_target like `act_pursue`) steers at the engaged
// counterpart.
const COND_SHOULD_BOARD: usize = 12;

/// Step 6: visual range (NM) at which a captain can identify another
/// ship — her flag, her trim, her freeboard (loaded vs light). Sized
/// at 12 NM, matching the period horizon for a sloop's masthead
/// lookout in clear weather, and matching `nav::ARRIVAL_NM` so a ship
/// is "in sight" exactly when it's also "in the same harbor zone"
/// when stalking near a port.
pub const VISUAL_RANGE_NM: f32 = 12.0;

/// Step 6: once a pirate has locked onto a prey (or a merchant onto a
/// threat), they keep chasing/fleeing until the target slips outside
/// this range. Wider than `VISUAL_RANGE_NM` to give hysteresis — a
/// target oscillating at the edge of detection shouldn't make the
/// pursuer/fugitive thrash. Slightly larger than the per-tick max
/// sloop travel (~12 NM) so a one-tick speed burst can't strand a
/// pursuer.
pub const PURSUE_BREAKOFF_NM: f32 = 24.0;

/// Hard panic floor: if a sailing ship has fewer than this many days
/// of provisions left and no destination set, divert to the nearest
/// port. The reachability-based check below handles the normal case
/// where a destination exists; this is the safety net for ships that
/// somehow lost their plan or are choosing one.
const HARD_PANIC_DAYS: f32 = 2.0;

/// Extra days the AI insists on having beyond the estimated voyage
/// time before deciding it can still reach its destination. Smaller
/// than the trade-planner buffer because at this point the voyage is
/// underway and most of the uncertainty is already known.
const REACHABILITY_BUFFER_DAYS: f32 = 3.0;

/// If a ship's waypoint queue is empty but it is still farther than this
/// from its destination, request a fresh plan. Larger than `ARRIVAL_NM` so
/// that ships about to dock don't waste a planner call, but small enough
/// that a ship drifting past its smoothed route promptly recovers.
const REPLAN_DISTANCE_NM: f32 = 25.0;

/// Operating float kept aboard after a home-port settlement: enough
/// to top up provisions at any foreign port, plus a modest reserve
/// for incidentals (pilotage fees, minor repairs). All silver above
/// this is paid out to the owner port on docking.
pub const HOME_PORT_FLOAT_SILVER: crate::money::Pesos = crate::money::Pesos::from_pesos(500);

/// Phase 4 §1.3: how many full broadsides' worth of powder / shot a
/// merchant ship aims to keep aboard. 20 ≈ a long convoy's worth of
/// defensive engagements without restocking, in line with the
/// "magazine for the voyage" rule of thumb 17C indiamen sailed under.
const MERCHANT_BROADSIDES_TARGET: f32 = 20.0;

/// Phase 4 §1.3: how many full broadsides a pirate aims to keep aboard.
/// Double the merchant figure: pirates expect to *initiate* combat,
/// often several engagements per cruise, and have no second supply
/// line if they exhaust their magazine far from a friendly port.
const PIRATE_BROADSIDES_TARGET: f32 = 40.0;

/// Phase 4 §1.3: desired tonnage of powder and shot to keep in a ship's
/// magazine after selling cargo and before sailing. Scales with the
/// ship's cannon count via `combat::broadside_supply_cost`, which is
/// also what `world.rs` debits on every fire — i.e., the target is
/// expressed directly in "broadsides of reserve".
///
/// Pirates aim for `PIRATE_BROADSIDES_TARGET` broadsides; merchants for
/// `MERCHANT_BROADSIDES_TARGET`. A 0-gun ship returns `(0.0, 0.0)` and
/// will not buy ordnance at all.
pub fn ordnance_target(ship: &Ship, stats: &ShipStats) -> (f32, f32) {
    if stats.cannons == 0 {
        return (0.0, 0.0);
    }
    let broadsides = match ship.policy {
        ShipPolicy::Pirate => PIRATE_BROADSIDES_TARGET,
        ShipPolicy::Merchant => MERCHANT_BROADSIDES_TARGET,
    };
    let (powder_per_broadside, shot_per_broadside) =
        crate::combat::broadside_supply_cost(stats.cannons);
    (
        powder_per_broadside * broadsides,
        shot_per_broadside * broadsides,
    )
}

/// On outfitting at home, the ship will try to top its strongbox up
/// to this many times the estimated cost of one full outbound hold.
/// Gives a cushion for partial-cargo top-ups at intermediate ports
/// and small contingencies en route.
pub const OUTFIT_DRAW_MULTIPLE: f32 = 2.0;

/// No single outfit draw can take more than this fraction of the home
/// port's silver. Prevents a busy yard's working capital from being
/// drained by one ship's outbound cargo.
pub const OUTFIT_PORT_FRACTION_CAP: f32 = 0.2;

/// Tramping credit: at any non-home port, a captain with no silver
/// but a profitable arbitrage opportunity may draw against the port
/// factor (consigned cargo / freight charter) up to this fraction of
/// the port's treasury. Booked as ship debt and repaid at the next
/// docking from sale proceeds.
pub const TRAMP_PORT_FRACTION_CAP: f32 = 0.10;

/// Build the ship AI behavior tree.
fn build_ship_bt() -> Behavior {
    // Dock cycle: sell whatever we arrived with, top up provisions,
    // load the most profitable next leg's cargo, careen if needed,
    // and undock toward the new destination.
    let dock_tree = Behavior::Sequence(vec![
        Behavior::Action(ACT_SELL_ALL),
        Behavior::Action(ACT_RESUPPLY),
        Behavior::Action(ACT_BUY_BEST),
        Behavior::Action(ACT_CAREEN),
        Behavior::Action(ACT_UNDOCK),
    ]);

    Behavior::Selector(vec![
        // Priority 1: If docked, do port activities. Docked ships do
        // not pursue or flee — pirates loitering at Tortuga ignore
        // passing merchants, and a merchant at the dock can't run.
        Behavior::Sequence(vec![Behavior::Condition(COND_IS_DOCKED), dock_tree]),
        // Priority 2 (Phase 4 §3c-1, symmetric redesign): engaged
        // subtree. Top-priority because once shots have been
        // exchanged the ship is in mortal danger and trade / patrol
        // can wait. Inside the subtree, four tactical branches are
        // tried in order each hour:
        //   1. Disengage — out of ordnance, badly outclassed,
        //      outnumbered, or lost contact: emit Disengage to
        //      mutually clear the flag and earn a cooldown.
        //   2. Fight — firepower / speed / hull advantage: pursue
        //      and broadside the engaged counterpart. Pirates may
        //      also attempt to board via the existing maybe_board
        //      path inside ACT_PURSUE.
        //   3. Flee — slower, weaker, or low on shot but not yet
        //      ready to disengage: flee toward the nearest port and
        //      bark back with stern chasers.
        //   4. Hold — default: no Steer command; ship coasts on its
        //      previous course while it reloads.
        // Both parties run this same selector — there is no Attacker
        // / Defender distinction. Tactical truth is local: each ship
        // decides from its own state and the snapshots it can see.
        Behavior::Sequence(vec![
            Behavior::Condition(COND_IS_ENGAGED),
            Behavior::Selector(vec![
                Behavior::Sequence(vec![
                    Behavior::Condition(COND_SHOULD_STRIKE),
                    Behavior::Action(ACT_STRIKE),
                ]),
                Behavior::Sequence(vec![
                    Behavior::Condition(COND_SHOULD_BOARD),
                    Behavior::Action(ACT_BOARD),
                ]),
                Behavior::Sequence(vec![
                    Behavior::Condition(COND_SHOULD_DISENGAGE),
                    Behavior::Action(ACT_DISENGAGE),
                ]),
                Behavior::Sequence(vec![
                    Behavior::Condition(COND_SHOULD_FIGHT),
                    Behavior::Action(ACT_PURSUE),
                ]),
                Behavior::Sequence(vec![
                    Behavior::Condition(COND_SHOULD_FLEE),
                    Behavior::Action(ACT_FLEE),
                ]),
                Behavior::Action(ACT_HOLD),
            ]),
        ]),
        // Priority 4 (Step 6): A sailing pirate that sees prey chases
        // it. Side effect: `see_prey` records / refreshes the target
        // id in `goal.pursue_target`; `act_pursue` reads truth-position
        // from the per-tick snapshot. Above trade because hunting is
        // the pirate's reason for being at sea — interrupts any active
        // voyage. Below docked so a pirate at port stays at port.
        Behavior::Sequence(vec![
            Behavior::Condition(COND_IS_SAILING_PIRATE),
            Behavior::Condition(COND_SEE_PREY),
            Behavior::Action(ACT_PURSUE),
        ]),
        // Priority 3 (Step 6): A sailing merchant that sees a threat
        // (any Pirate within visual range) flees toward the nearest
        // port. As soon as the threat slips outside `PURSUE_BREAKOFF_NM`
        // the condition fails and the merchant resumes its trade
        // voyage (its `goal.destination` was never touched).
        Behavior::Sequence(vec![
            Behavior::Condition(COND_IS_SAILING_MERCHANT),
            Behavior::Condition(COND_SEE_THREAT),
            Behavior::Action(ACT_FLEE),
        ]),
        // Priority 4: If low on provisions, divert to nearest port and
        // continue steering toward it. The Sail action *must* run after
        // DivertToPort: DivertToPort only updates `nav.destination` and
        // re-plans, but doesn't compute_steering. Without Sail in the
        // same sequence, a low-provisions ship keeps its old heading
        // forever, which is how ships used to drift to a halt and "get
        // stuck" mid-voyage when provisions ran out before arrival.
        Behavior::Sequence(vec![
            Behavior::Condition(COND_IS_LOW_PROVISIONS),
            Behavior::Action(ACT_DIVERT_TO_PORT),
            Behavior::Action(ACT_SAIL),
        ]),
        // Priority 5: If has destination, sail
        Behavior::Sequence(vec![
            Behavior::Condition(COND_HAS_DESTINATION),
            Behavior::Action(ACT_SAIL),
        ]),
        // Priority 6: Choose a new destination (random fallback)
        Behavior::Action(ACT_CHOOSE_DESTINATION),
    ])
}

/// AI state for a single ship (the captain). Owns the high-level goal
/// (where the ship is trying to go) and the BT execution state; the ship
/// itself owns the in-flight nav tracking (waypoints, dock) and dock
/// action.
pub struct ShipAI {
    pub goal: NavGoal,
    tree: Behavior,
    state: BtState,
    /// PCG RNG for destination selection.
    rng: SimRng,
    /// Independent PCG RNG for the navigator (DR noise, fix noise).
    /// Kept separate so per-ship navigation jitter doesn't perturb the
    /// destination-choice RNG sequence — important for reproducible
    /// bench economics across noise tuning passes.
    nav_rng: SimRng,
    /// Previous tick's true ship position. Used by the navigator pass
    /// to advance the captain's dead-reckoning estimate by the ship's
    /// actual displacement (plus accumulating DR noise). `None` on the
    /// first tick — initialized on next iteration.
    prev_truth: Option<Position>,
}

impl Default for ShipAI {
    fn default() -> Self {
        Self::new()
    }
}

impl ShipAI {
    pub fn new() -> Self {
        Self {
            goal: NavGoal::new(),
            tree: build_ship_bt(),
            state: BtState::new(),
            rng: SimRng::new(12345),
            nav_rng: SimRng::new(0x9E3779B97F4A7C15),
            prev_truth: None,
        }
    }

    pub fn with_destination(dest: Position) -> Self {
        Self {
            goal: NavGoal::with_destination(dest),
            tree: build_ship_bt(),
            state: BtState::new(),
            rng: SimRng::new(12345),
            nav_rng: SimRng::new(0x9E3779B97F4A7C15),
            prev_truth: None,
        }
    }

    /// Create AI with a specific RNG seed (for variety among multiple ships).
    pub fn with_seed(seed: u64) -> Self {
        // The nav RNG is derived from the AI seed (xor with the golden-
        // ratio constant) so different ships diverge in their navigation
        // jitter without colliding with the destination-choice sequence.
        Self {
            goal: NavGoal::new(),
            tree: build_ship_bt(),
            state: BtState::new(),
            rng: SimRng::new(seed),
            nav_rng: SimRng::new(seed ^ 0x9E3779B97F4A7C15),
            prev_truth: None,
        }
    }

    /// Called each tick: run the behavior tree.
    ///
    /// `inputs` carries the per-tick external state the BT leaves need to
    /// read/mutate (ship, wind, ports, markets, etc.). The AI merges those
    /// inputs with its own internal state (`goal`, `rng_state`, BT cursor)
    /// to form the BT context.
    pub fn tick(&mut self, inputs: &mut ShipTickInputs<'_>) {
        // Reactivity guard: the root Selector resumes from its last
        // Running child every tick, so higher-priority branches are
        // skipped while a lower-priority branch is still ticking. If
        // the ship has since become `Docked` (e.g., daily hiring just
        // transitioned a freshly-built hull from Hiring → Docked, or
        // the ship was force-docked outside the BT), reset the BT
        // state so this tick re-evaluates from `COND_IS_DOCKED` at
        // the top of the Selector. Without this, built ships sit at
        // dock running ACT_SAIL forever and never enter the dock
        // tree's SELL/RESUPPLY/BUY/UNDOCK cycle.
        if inputs.ship.state == ShipState::Docked && !self.state.running_child.is_empty() {
            self.state.reset();
        }

        // Navigator pass (Nav-1/2/3): maintain the captain's belief
        // about where the ship is. Runs BEFORE the BT so all
        // planning, arrival checks, and BT conditions see the
        // refreshed estimate. Order matters:
        //   1. Lazy-init the estimate at launch (perfect fix).
        //   2. Advance the estimate by the ship's true displacement
        //      since the previous tick — this is the dead-reckoning
        //      plot (the captain knows roughly how far he sailed on
        //      what heading) — then accumulate small per-hour noise.
        //   3. Try a noon sight (latitude only, once per day).
        //   4. Try a landmark fix (both axes, snaps to truth).
        let truth = inputs.ship.position;
        if self.goal.estimated_position.is_none() {
            self.goal.estimated_position = Some(truth);
        }
        {
            let estimate = self.goal.estimated_position.as_mut().unwrap();
            if let Some(prev) = self.prev_truth {
                estimate.x += truth.x - prev.x;
                estimate.y += truth.y - prev.y;
            }
            // DR error scales with the previous tick's speed (no
            // drift at anchor / dock).
            nav::apply_dr_error(estimate, inputs.ship.speed, 1.0, &mut self.nav_rng);
            nav::try_noon_sight(
                estimate,
                truth,
                inputs.day_of_year,
                &mut self.goal.last_noon_sight_day,
                &mut self.nav_rng,
            );
            // Landmark fix only while underway. A docked captain knows
            // where the dock is — re-snapping to truth+N(0,1) every
            // hour would just add noise to a known position and
            // perturb departure headings.
            if inputs.ship.speed > 0.1 {
                nav::try_landmark_fix(estimate, truth, inputs.ports, &mut self.nav_rng);
            }
        }
        self.prev_truth = Some(truth);

        let mut ctx = ShipBtContext {
            me: inputs.me,
            ship: inputs.ship,
            stats: inputs.stats,
            wind: inputs.wind,
            goal: &mut self.goal,
            ports: inputs.ports,
            harbors: inputs.harbors,
            rng: &mut self.rng,
            pathfind: inputs.pathfind,
            markets: inputs.markets,
            goods: inputs.goods,
            policy: inputs.policy,
            port_telemetry: inputs.port_telemetry,
            commands: inputs.commands,
            snapshots: inputs.snapshots,
            spatial: inputs.spatial,
        };

        let status = bt::tick(&self.tree, &mut self.state, &mut ctx, 0);

        // If the tree completed (Success/Failure), reset for next tick
        if status != Status::Running {
            self.state.reset();
        }
    }

    /// Give the AI a new destination. Clears any in-flight path on the ship
    /// (the old path was planned to the previous destination and is stale).
    pub fn set_destination(&mut self, dest: Position, ship: &mut Ship) {
        self.goal.destination = Some(dest);
        self.goal.dest_port = None;
        ship.nav.clear_path();
    }
}

/// Per-tick external inputs to the ship AI. The AI merges these with its
/// own internal state (`goal`, `rng_state`, BT cursor) inside `tick`.
pub struct ShipTickInputs<'a> {
    /// The id of the ship being ticked. Stamped onto every command this
    /// AI emits so the Resolution Phase can route intents back to the
    /// issuer (and, in later steps, to targets).
    pub me: ShipId,
    pub ship: &'a mut Ship,
    pub stats: &'a ShipStats,
    pub wind: &'a WindVector,
    pub ports: &'a [Port],
    pub harbors: &'a HarborMap,
    pub pathfind: Option<&'a PathfindContext<'a>>,
    pub markets: &'a [PortMarket],
    pub goods: &'a GoodsRegistry,
    /// Precomputed faction trade policy (docking permission, per-good
    /// legality, ad-valorem duties). Read-only; the planner uses it
    /// to filter destinations and goods and to compute post-duty
    /// effective prices for arbitrage scoring.
    pub policy: &'a crate::policy::PolicyResolver,
    /// Read-only per-port telemetry. Used by `act_sail` at the
    /// docking transition to bump `dockings_by_flag[ship.faction]`
    /// via the atomic counter. Other fields (production, duties)
    /// are written only in the serial clearing phase.
    pub port_telemetry: &'a [crate::telemetry::PortTelemetry],
    /// Output buffer for `ShipCommand`s emitted this tick. Owned by the
    /// world and drained by the Resolution Phase.
    pub commands: &'a mut Vec<(ShipId, ShipCommand)>,
    /// Current `SimDate::day_of_year` (1-365). Drives the noon-sight
    /// cadence in the captain's navigator pass.
    pub day_of_year: u16,
    /// Step 6: read-only view of every Sailing ship's position +
    /// identifying metadata, rebuilt each tick by the world. Lets the
    /// AI inspect *other* ships without taking a second borrow on
    /// `world.ships` (which is mutably borrowed for `me`). Empty for
    /// tests that don't care about inter-ship interaction.
    pub snapshots: &'a SecondaryMap<ShipId, ShipSnapshot>,
    /// Step 6: spatial hash over Sailing ships. Used by `see_prey` /
    /// `see_threat` to find neighbors within `VISUAL_RANGE_NM`
    /// without an O(N) scan. Same per-tick lifetime as `snapshots`.
    pub spatial: &'a SpatialHash,
}

/// Step 6: per-tick, read-only fingerprint of one Sailing ship —
/// enough for any other ship's AI to identify it as friend / foe /
/// prey without taking a borrow on `world.ships`. Built by the world
/// at the top of each hourly tick (the same place the spatial hash
/// is rebuilt) and discarded at tick end. All fields are by-value
/// copies (positions, enums, scalars) — there's no aliasing issue.
///
/// Indexed by `ShipId`; matches the set of ships inserted into the
/// `SpatialHash` (Sailing only, no docked/hiring/anchored).
#[derive(Debug, Clone, Copy)]
pub struct ShipSnapshot {
    pub position: Position,
    pub policy: ShipPolicy,
    pub faction: Faction,
    /// Maximum design speed (kt). Lets a pirate identify slower prey
    /// without consulting the ship-type registry.
    pub max_speed: f32,
    /// Cargo hold capacity (tons). Lets a pirate identify richer
    /// prey (bigger merchantmen) without consulting the registry.
    pub cargo_capacity_tons: f32,
    /// Velocity vector (vx, vy) in NM/hr at the moment of snapshot
    /// (top of the hourly tick, pre-Steer-this-tick). Used by Step 8
    /// closest-approach gating for broadside and boarding actions —
    /// the AI needs to know roughly where the target will be over
    /// the next hour to know whether a shot is even worth trying.
    pub velocity: (f32, f32),
    /// Current rigging integrity, as a fraction of
    /// `stats.rigging_integrity_max`. Used by Step 8 boarding-gate:
    /// a target with healthy rigging can outrun grapples.
    pub rigging_frac: f32,
    /// Current hull integrity, as a fraction of
    /// `stats.hull_integrity_max`. Used by §3c-1 tactical-judgment
    /// conditions to compare own hull state against the engaged
    /// counterpart (e.g., disengage if I'm under 30% and they're over
    /// 70%).
    pub hull_frac: f32,
    /// Number of broadside cannons. Used by §3c-1 tactical-judgment
    /// conditions to compare firepower against the engaged
    /// counterpart without consulting the ship-type registry.
    pub cannons: u16,
}

// Simple xorshift64 RNG — legacy. Replaced by `crate::sim_rng::SimRng`.

/// Convert a steering command (heading in degrees CW from N, speed in
/// knots) to a velocity vector in (East, North) NM/hr. Used by Step 8
/// closest-approach gating so the AI can predict the next-tick segment
/// of motion the physics step is about to apply.
fn velocity_from(heading: f32, speed: f32) -> (f32, f32) {
    let h = heading.to_radians();
    (speed * h.sin(), speed * h.cos())
}

/// Context struct that connects BT leaf nodes to actual ship logic.
/// Borrows the per-tick `ship` (which owns `nav` and `dock_action`) plus
/// AI-side `goal` and `rng_state`, plus all the read-only world state.
pub struct ShipBtContext<'a> {
    me: ShipId,
    ship: &'a mut Ship,
    stats: &'a ShipStats,
    wind: &'a WindVector,
    goal: &'a mut NavGoal,
    ports: &'a [Port],
    harbors: &'a HarborMap,
    rng: &'a mut SimRng,
    pathfind: Option<&'a PathfindContext<'a>>,
    markets: &'a [PortMarket],
    goods: &'a GoodsRegistry,
    policy: &'a crate::policy::PolicyResolver,
    port_telemetry: &'a [crate::telemetry::PortTelemetry],
    commands: &'a mut Vec<(ShipId, ShipCommand)>,
    snapshots: &'a SecondaryMap<ShipId, ShipSnapshot>,
    spatial: &'a SpatialHash,
}

impl<'a> ShipBtContext<'a> {
    /// The captain's belief about where the ship is. Decisions inside
    /// the BT — pathfinding origin, arrival checks, nearest-port lookups,
    /// distance-to-destination tests — must use this rather than
    /// `self.ship.position` (truth). The gap between estimate and truth
    /// is what creates the period-realistic getting-lost behaviour. The
    /// navigator pass at the top of `ShipAI::tick` guarantees the
    /// estimate is `Some` by the time any BT code runs.
    fn estimated_position(&self) -> Position {
        self.goal.estimated_position.unwrap_or(self.ship.position)
    }

    /// Set a destination *port* (by index) and plan a path to its harbor
    /// zone. The destination is recorded even if planning fails — the ship
    /// then falls back to straight-line nav with reactive deflection.
    fn assign_destination_port(&mut self, port_index: usize) {
        let port = &self.ports[port_index];
        self.goal.destination = Some(port.position);
        self.goal.dest_port = Some(port_index);
        self.ship.nav.clear_path();

        if let (Some(pf), Some(harbor)) = (self.pathfind, self.harbors.for_port(port_index)) {
            if let Some(path) = pathfind::find_path_to_harbor(pf, self.estimated_position(), harbor)
            {
                // Track the path's terminal waypoint as the geometric
                // arrival target — typically the harbor anchor, which may
                // differ slightly from the port's literal coordinate when
                // the port sits inside a polygonal harbor zone.
                if let Some(last) = path.last().copied() {
                    self.goal.destination = Some(last);
                }
                self.ship.nav.set_path(path);
            }
        }
    }

    /// Re-plan a path to the current destination port without resetting
    /// other navigation state. Called when the existing waypoint queue has
    /// emptied but the ship is still mid-voyage.
    fn replan_to_port(&mut self, port_index: usize) {
        if let (Some(pf), Some(harbor)) = (self.pathfind, self.harbors.for_port(port_index)) {
            if let Some(path) = pathfind::find_path_to_harbor(pf, self.estimated_position(), harbor)
            {
                if let Some(last) = path.last().copied() {
                    self.goal.destination = Some(last);
                }
                self.ship.nav.set_path(path);
            }
        }
    }

    /// True if the ship is in its destination port's harbor zone.
    fn in_destination_harbor(&self) -> bool {
        let port_idx = match self.goal.dest_port {
            Some(i) => i,
            None => return false,
        };
        let pf = match self.pathfind {
            Some(pf) => pf,
            None => return false,
        };
        let harbor = match self.harbors.for_port(port_idx) {
            Some(h) => h,
            None => return false,
        };
        // Use TRUTH: the ship has truly entered harbor when its hull is
        // inside the harbor zone. Using the captain's estimate here lets
        // DR drift teleport the ship into the harbor before the hull is
        // there (instant docking → instant profit settlement → new
        // voyage starts while truth still mid-ocean). The pilot/lookout
        // recognizes the harbor on physical entry.
        harbor.contains_pos(pf.land, self.ship.position)
    }

    // ───────── BT action implementations ─────────
    //
    // One method per `ACT_*` id, dispatched from `execute_action`
    // below. Extracted from the original 286-line `match` per
    // `planning/code-health-audit.md` §2, ahead of Step 5.c's
    // command-queue rewrite (which will change every arm's
    // mutation pattern). Keeping each arm self-contained makes
    // that subsequent diff reviewable arm-by-arm.

    fn act_sail(&mut self) -> Status {
        // Harbor-zone arrival: if we're inside the destination's
        // harbor zone, transition to Docked immediately. The literal
        // port coordinate may still be far away (e.g., Philadelphia
        // up the Delaware) — that's fine.
        if self.in_destination_harbor() {
            let port_idx = self.goal.dest_port;
            // Docking gate: an arrival event at a port that refuses
            // this flag is silently rejected. The planner already
            // filters closed ports out of its destination set, so a
            // refusal here only happens for prize-chase detours,
            // player-issued commands, or future AI behaviors. We
            // clear the destination and let `act_choose_destination`
            // (or the next tick's planner) pick a new heading —
            // no event, no log spam, per the design note.
            if let Some(idx) = port_idx {
                if !matches!(
                    self.policy.dock_legality(idx, self.ship.faction),
                    crate::policy::DockLegality::Open
                ) {
                    self.goal.destination = None;
                    self.goal.dest_port = None;
                    self.ship.nav.clear_path();
                    return Status::Failure;
                }
            }
            self.ship.dock();
            self.ship.dock_action = DockAction::Idle;
            self.ship.nav.docked_at_port = port_idx;
            if let Some(idx) = port_idx {
                if let Some(t) = self.port_telemetry.get(idx) {
                    t.record_docking(self.ship.faction);
                }
            }
            self.goal.destination = None;
            self.goal.dest_port = None;
            self.ship.nav.clear_path();
            // Settle any outstanding chandler/freight debt at
            // this port first — creditors come before owners.
            // Fungible: it doesn't matter which port originally
            // advanced the credit; the merchant network settles
            // it via bills of exchange between correspondents.
            //
            // Phase 6: emitted as `MarketCollectDebt` intent; the
            // world's auction-pass resolver mutates ship + treasury
            // in ship-id order (silver-only ops happen before bids
            // and asks clear, so subsequent same-tick trade pricing
            // sees the post-debt ship balance — matching the old
            // mutate-in-place semantics).
            if let Some(idx) = port_idx {
                if idx < self.markets.len() && self.ship.debt.is_positive() {
                    self.commands
                        .push((self.me, ShipCommand::MarketCollectDebt { port: idx }));
                }
            }
            // Home-port settlement: if this is the owner port,
            // the supercargo books proceeds with the owners.
            // Silver above the operating float is paid into
            // the port treasury (dividend to shareholders);
            // the ship keeps just enough to cover provisions
            // and incidentals at the next port of call.
            //
            // Phase 6: surplus is computed at AI-tick read-time
            // (matches the value the resolver will see after the
            // earlier CollectDebt pass at the same port in the
            // same tick — since CollectDebt only reduces silver,
            // a positive surplus here is conservative).
            if let (Some(idx), Some(owner)) = (port_idx, self.ship.owner_port) {
                if idx == owner && idx < self.markets.len() {
                    let surplus = (self.ship.silver - HOME_PORT_FLOAT_SILVER).max_zero();
                    if surplus.is_positive() {
                        self.commands.push((
                            self.me,
                            ShipCommand::MarketDeposit {
                                port: idx,
                                amount: surplus,
                            },
                        ));
                    }
                }
            }
            return Status::Success;
        }

        // Replan when our planned route has been exhausted but we're
        // still nowhere near the destination. Without this, a ship
        // that drifts/tacks past its last waypoint will dead-reckon
        // straight toward the destination — through land, if any —
        // and pin against the coast.
        if self.ship.nav.waypoints.is_empty() && self.goal.dest_port.is_some() {
            if let Some(dest) = self.goal.destination {
                if self.estimated_position().distance(dest) > REPLAN_DISTANCE_NM {
                    if let Some(idx) = self.goal.dest_port {
                        self.replan_to_port(idx);
                    }
                }
            }
        }

        let land = self.pathfind.map(|c| c.land);
        let pos_estimate = self.estimated_position();
        let pos_truth = self.ship.position;

        // Path-stale check: if land has come between the ship's actual
        // position and its next target (waypoint or destination), the
        // remaining path is invalid — `compute_steering` will sail us
        // straight into the coast. Replan from truth. This covers the
        // common failure mode where wind/tacking drifts the ship a few
        // NM off the planned corridor near a small harbor and the next
        // waypoint is no longer reachable in a straight line.
        if let (Some(pf), Some(_)) = (self.pathfind, self.goal.destination) {
            let next_target = self
                .ship
                .nav
                .waypoints
                .first()
                .copied()
                .or(self.goal.destination);
            if let Some(target) = next_target {
                if !pf.land.corridor_is_clear(pos_truth, target, 2.0) {
                    if let Some(idx) = self.goal.dest_port {
                        self.replan_to_port(idx);
                    } else if let Some(dest) = self.goal.destination {
                        if let Some(path) = pathfind::find_path(pf, pos_truth, dest) {
                            if let Some(last) = path.last().copied() {
                                self.goal.destination = Some(last);
                            }
                            self.ship.nav.set_path(path);
                        }
                    }
                }
            }
        }

        let steering = self.ship.nav.compute_steering(
            self.goal,
            pos_estimate,
            pos_truth,
            self.stats,
            self.wind,
            land,
        );
        if let Some(s) = steering {
            self.commands.push((
                self.me,
                ShipCommand::Steer {
                    heading: s.heading,
                    speed: s.speed,
                },
            ));
            Status::Running
        } else if self.goal.dest_port.is_some()
            && self.pathfind.is_some()
            && !self.in_destination_harbor()
        {
            // We "arrived" at the path's last waypoint (the harbor
            // anchor) but the ship's actual position is just outside
            // the harbor cell set. Replan from here so the new path
            // ends with the ship inside the harbor zone, rather than
            // false-docking in open water (which is the canonical
            // way ships used to get stuck near small-harbor ports
            // like Amsterdam/Nantes).
            if let Some(idx) = self.goal.dest_port {
                self.replan_to_port(idx);
            }
            Status::Running
        } else {
            // Arrived at a free-form destination (no harbor zone).
            self.ship.dock();
            self.ship.dock_action = DockAction::Idle;
            Status::Success
        }
    }

    fn act_resupply(&mut self) -> Status {
        self.ship.dock_action = DockAction::Resupplying;
        // Phase 6: emit a provisions bid (and any credit-advance bid
        // we need to fund it) and let the world's auction pass do the
        // actual transfer. Status is Running while we still need
        // provisions and the port can sell them; Success otherwise.
        let done = match self.ship.nav.docked_at_port {
            Some(idx) if idx < self.markets.len() => self.tick_resupply_bid(idx),
            // Unknown / out-of-range port — fall back to free resupply
            // (matches the pre-Phase-6 behavior for test ports beyond
            // the markets slice).
            _ => self.ship.tick_resupply(self.stats),
        };
        if done {
            self.ship.dock_action = DockAction::Idle;
            Status::Success
        } else {
            Status::Running
        }
    }

    /// Phase 6: emit per-tick resupply intent. Returns `true` when the
    /// AI judges the resupply phase complete (full / market dry /
    /// nothing to bid with). Mirrors the predicate logic of the
    /// pre-Phase-6 `Ship::tick_resupply_at_market`, but emits
    /// `MarketResupplyBid` (+ optional `MarketCreditBid`) instead of
    /// mutating ship/market state directly.
    fn tick_resupply_bid(&mut self, idx: usize) -> bool {
        let provisions_id = crate::goods::ids::PROVISIONS;
        let space = (self.stats.provision_capacity - self.ship.provisions).max(0.0);
        if space <= 0.0 {
            return true;
        }
        let market = &self.markets[idx];
        let stockpile = market.stockpile.get(provisions_id);
        if stockpile <= 0.0 {
            return true;
        }
        // Provisions are the lifeblood of the voyage; if a port
        // bans them on this flag (essentially a wartime starvation
        // tactic), treat the chandler as dry. Otherwise, fold the
        // duty wedge into the unit price so the bid limit and
        // affordability checks both work in gross pesos.
        let buy_duty = match self
            .policy
            .buy_legality(idx, self.ship.faction, provisions_id)
        {
            crate::policy::TradeLegality::Legal { duty } => duty,
            crate::policy::TradeLegality::Prohibited => return true,
        };
        let unit_base = market.buy_price(provisions_id, self.goods).max(0.0001);
        let unit_price = unit_base * (1.0 + buy_duty);
        let hour_bill =
            crate::money::Pesos::from_pesos_f32(unit_price * crate::ship::RESUPPLY_RATE_PER_HOUR);
        // Chandler credit bid: if we can't pay cash but have debt
        // headroom, ask the chandler to advance one hour's bill.
        // Resolver caps by liquidity and ship debt headroom.
        if self.ship.silver < hour_bill && self.ship.debt < crate::ship::MAX_SHIP_DEBT {
            self.commands.push((
                self.me,
                ShipCommand::MarketCreditBid {
                    port: idx,
                    max_amount: hour_bill,
                },
            ));
        }
        let desired = crate::ship::RESUPPLY_RATE_PER_HOUR
            .min(space)
            .min(stockpile);
        if desired <= 0.0 {
            return true;
        }
        // Limit price: pay up to a moderate premium over the formula
        // price (the formula already prices scarcity; the cushion
        // keeps the bid from being filtered out by an in-tick price
        // tick from concurrent buys).
        let limit = unit_price * 1.2;
        self.commands.push((
            self.me,
            ShipCommand::MarketResupplyBid {
                port: idx,
                tons: desired,
                limit_price: limit,
            },
        ));
        // Done predicate is *predictive* — true means "don't tick
        // again next AI hour": ship is full, or the port has nothing
        // more to sell, or we're so broke we couldn't even bid for a
        // tiny slice next tick.
        let full = self.ship.provisions + desired >= self.stats.provision_capacity - 1e-4;
        let market_dry = (stockpile - desired) <= 0.0;
        let broke = self.ship.silver.as_pesos_f32() + hour_bill.as_pesos_f32() < unit_price * 0.05;
        full || market_dry || broke
    }

    fn act_careen(&mut self) -> Status {
        self.ship.dock_action = DockAction::Careening;
        // Phase 4 §2: careening at port now also restores combat /
        // storm damage to the hull. Both operations tick in parallel
        // (the carpenters and the boatswain's gang work on different
        // parts of the ship). Action only completes when both the
        // fouling-and-teredo scrape *and* the hull-integrity rebuild
        // are finished. Cost is silver-or-debt; see
        // `Ship::tick_repair_hull`.
        let fouling_done = self.ship.tick_careen();
        let hull_done = self.ship.tick_repair_hull(self.stats);
        if fouling_done && hull_done {
            self.ship.dock_action = DockAction::Idle;
            Status::Success
        } else {
            Status::Running
        }
    }

    fn act_undock(&mut self) -> Status {
        if self.goal.destination.is_some() {
            // Phase 4 §2: top off rigging from bo's'n stores on the
            // way out. One-shot (unlike hull, which ticks gradually);
            // matches the historical practice where a port turnaround
            // included re-rigging as a normal item, not a separate
            // refit. Silver-or-debt.
            self.ship.top_off_rigging(self.stats);
            self.ship.undock();
            self.ship.nav.docked_at_port = None;
            self.ship.dock_action = DockAction::Idle;
            Status::Success
        } else {
            self.ship.dock_action = DockAction::Idle;
            Status::Failure
        }
    }

    fn act_choose_destination(&mut self) -> Status {
        if self.ports.is_empty() {
            return Status::Failure;
        }
        // Filter to ports that will actually let us dock. A ship that picks
        // a closed port would sail there only for `act_sail` to silently
        // reject the arrival and re-plan — wasted miles and (more
        // importantly) a re-plan loop if every random pick lands on a
        // closed port. Filtering here keeps wander behavior alive even
        // under restrictive faction policies.
        let here = self.estimated_position();
        let candidates: Vec<usize> = (0..self.ports.len())
            .filter(|&i| {
                matches!(
                    self.policy.dock_legality(i, self.ship.faction),
                    crate::policy::DockLegality::Open
                )
            })
            .filter(|&i| here.distance(self.ports[i].position) >= 20.0)
            .collect();
        if candidates.is_empty() {
            return Status::Failure;
        }
        let pick = candidates[(self.rng.next_u64() as usize) % candidates.len()];
        self.assign_destination_port(pick);
        Status::Success
    }

    fn act_sell_all(&mut self) -> Status {
        // Sell every ton of cargo we arrived with at the docked port's
        // market. Silent no-op when docked_at_port isn't set or is out of
        // range (test ports beyond the markets slice).
        //
        // Phase 6: emits one `MarketAsk` per good above the keep-line.
        // The world's auction pass clears them at a single per-good
        // price derived from the post-tick effective stockpile, with
        // pro-rata seller payouts if the port treasury can't cover.
        // Returns Success once nothing remains above the keep-line —
        // multi-tick behavior emerges when the auction doesn't fill
        // all asks in one go (rare, but possible at illiquid ports).
        let Some(idx) = self.ship.nav.docked_at_port else {
            return Status::Success;
        };
        if idx >= self.markets.len() {
            return Status::Success;
        }
        let (powder_keep, shot_keep) = ordnance_target(self.ship, self.stats);
        let market = &self.markets[idx];
        let entries: Vec<(crate::goods::GoodId, f32)> = self.ship.cargo.iter().collect();
        let mut anything_remaining = false;
        for (gid, tons) in entries {
            if tons <= 0.0 {
                continue;
            }
            let sellable = if gid == crate::goods::ids::GUNPOWDER {
                (tons - powder_keep).max(0.0)
            } else if gid == crate::goods::ids::CANNON_SHOT {
                (tons - shot_keep).max(0.0)
            } else {
                tons
            };
            if sellable > 0.0 {
                // Skip goods the port refuses to import on this
                // flag (e.g., English enumerated colonial staples
                // on a Dutch hull). The captain's emergency reflex
                // would be to dump such cargo elsewhere; in v1 we
                // just don't emit an ask, leaving the cargo aboard.
                let sell_duty = match self.policy.sell_legality(idx, self.ship.faction, gid) {
                    crate::policy::TradeLegality::Legal { duty } => duty,
                    crate::policy::TradeLegality::Prohibited => continue,
                };
                anything_remaining = true;
                let unit = market.sell_price(gid, self.goods);
                // Auction returns a net-of-duty price to the ship;
                // scale the 80% reservation floor by the same wedge
                // so a duty-bearing leg remains reachable.
                let limit = (unit * (1.0 - sell_duty) * 0.8).max(0.01);
                self.commands.push((
                    self.me,
                    ShipCommand::MarketAsk {
                        port: idx,
                        good: gid,
                        tons: sellable,
                        limit_price: limit,
                    },
                ));
            }
        }
        self.replenish_ordnance(idx, powder_keep, shot_keep);
        // Even if the auction only partially fills, returning Success
        // here lets the dock sequence advance into buy/careen/undock.
        // The unfilled remainder gets re-asked next tick if the ship
        // stays docked (which it usually does — careen takes hours).
        let _ = anything_remaining;
        Status::Success
    }

    /// Phase 6: emit buy bids for gunpowder / cannon shot up to the
    /// per-policy target tonnage. Auction-cleared.
    fn replenish_ordnance(&mut self, market_idx: usize, powder_target: f32, shot_target: f32) {
        let market = &self.markets[market_idx];
        for (good, target) in [
            (crate::goods::ids::GUNPOWDER, powder_target),
            (crate::goods::ids::CANNON_SHOT, shot_target),
        ] {
            let have = self.ship.cargo.get(good);
            if have >= target {
                continue;
            }
            // Skip ordnance the port refuses to export to this flag
            // (e.g., gunpowder under a wartime embargo, modeled as
            // Prohibited via a future per-port override).
            let buy_duty = match self
                .policy
                .buy_legality(market_idx, self.ship.faction, good)
            {
                crate::policy::TradeLegality::Legal { duty } => duty,
                crate::policy::TradeLegality::Prohibited => continue,
            };
            let want = target - have;
            let unit_base = market.buy_price(good, self.goods).max(0.0001);
            let unit_gross = unit_base * (1.0 + buy_duty);
            let affordable = (self.ship.silver.as_pesos_f32() / unit_gross).max(0.0);
            let in_stock = market.stockpile.get(good);
            let cargo_room = self.stats.cargo_capacity_tons - self.ship.cargo.total_tons();
            let tons = want.min(affordable).min(in_stock).min(cargo_room).max(0.0);
            if tons > 0.0 {
                // 20% headroom over gross-of-duty price — captain's
                // limit is what they're willing to pay all-in.
                let limit = unit_gross * 1.2;
                self.commands.push((
                    self.me,
                    ShipCommand::MarketBid {
                        port: market_idx,
                        good,
                        tons,
                        limit_price: limit,
                    },
                ));
            }
        }
    }

    fn act_buy_best(&mut self) -> Status {
        // Pick the best (good, dest) and emit bids. Sets the
        // ship's destination as a side effect so ACT_UNDOCK
        // has somewhere to go.
        //
        // Phase 6: emits MarketDrawOutfit (owner port only), then
        // MarketCreditBid (tramping), then MarketBid for the chosen
        // good. The world's auction pass clears the bid at a single
        // per-good price; silver-only ops (outfit/credit) run before
        // the auction so the ship's silver is updated before the
        // limit-price check filters the bid.
        let Some(idx) = self.ship.nav.docked_at_port else {
            return Status::Success;
        };
        if idx >= self.markets.len() {
            return Status::Success;
        }
        let daily = self.stats.daily_provision_consumption().max(1e-6);
        let provision_budget_days = self.stats.provision_capacity / daily;
        let home_bias = self.ship.owner_port.map(|home_idx| {
            let surplus = (self.ship.silver - HOME_PORT_FLOAT_SILVER).max_zero();
            crate::trade::HomeBias {
                home_port: home_idx,
                bias_pesos_per_ton: (surplus.as_pesos_f32() / 200.0).min(200.0),
            }
        });
        let plan = crate::trade::find_best_trade(
            idx,
            self.ship.faction,
            self.policy,
            self.ports,
            self.markets,
            self.goods,
            self.stats,
            provision_budget_days,
            home_bias,
            Some(self.rng),
        );
        let plan = match plan {
            Some(p) => p,
            None => return Status::Success,
        };

        let market = &self.markets[idx];
        // The planner already verified this leg is legal at origin
        // (`buy_legality != Prohibited`); look up the duty here so
        // every cost calculation in the bid block uses the gross
        // (silver-out-of-pocket) price the captain will actually pay.
        let buy_duty = match self.policy.buy_legality(idx, self.ship.faction, plan.good) {
            crate::policy::TradeLegality::Legal { duty } => duty,
            crate::policy::TradeLegality::Prohibited => return Status::Success,
        };
        let unit_base = market.buy_price(plan.good, self.goods).max(0.0001);
        let unit = unit_base * (1.0 + buy_duty);
        let cargo_room = self.stats.cargo_capacity_tons - self.ship.cargo.total_tons();
        let want_tons = cargo_room.min(market.stockpile.get(plan.good));

        // Outfit draw bid (owner port only). Resolver caps by
        // OUTFIT_PORT_FRACTION_CAP × treasury.
        if let Some(owner) = self.ship.owner_port {
            if owner == idx {
                let target =
                    crate::money::Pesos::from_pesos_f32(unit * want_tons * OUTFIT_DRAW_MULTIPLE);
                if target > self.ship.silver {
                    self.commands.push((
                        self.me,
                        ShipCommand::MarketDrawOutfit {
                            port: idx,
                            target_silver: target,
                        },
                    ));
                }
            }
        }

        // Tramping / freight credit bid: if we still can't load
        // anything meaningful, ask the local factor for an advance.
        let need_silver = crate::money::Pesos::from_pesos_f32(unit * want_tons);
        if self.ship.silver < need_silver
            && self.ship.debt < crate::ship::MAX_SHIP_DEBT
            && want_tons > 0.0
        {
            let shortfall = (need_silver - self.ship.silver).max_zero();
            if shortfall.is_positive() {
                self.commands.push((
                    self.me,
                    ShipCommand::MarketCreditBid {
                        port: idx,
                        max_amount: shortfall,
                    },
                ));
            }
        }

        // Main trade bid. Affordability is computed against the AI-
        // tick read of ship.silver *plus* any in-tick credit/outfit
        // ask above — those run before bids in the resolver, so by
        // the time the bid clears the silver picture matches what we
        // assume here.
        let assumed_silver = self.ship.silver
            + self
                .ship
                .owner_port
                .filter(|o| *o == idx)
                .map(|_| {
                    let target = crate::money::Pesos::from_pesos_f32(
                        unit * want_tons * OUTFIT_DRAW_MULTIPLE,
                    );
                    (target - self.ship.silver).max_zero()
                })
                .unwrap_or(crate::money::Pesos::ZERO)
            + if self.ship.silver < need_silver
                && self.ship.debt < crate::ship::MAX_SHIP_DEBT
                && want_tons > 0.0
            {
                (need_silver - self.ship.silver).max_zero()
            } else {
                crate::money::Pesos::ZERO
            };
        let affordable = assumed_silver.as_pesos_f32() / unit;
        let in_stock = market.stockpile.get(plan.good);
        let tons = cargo_room.min(affordable).min(in_stock).max(0.0);
        if tons > 0.0 {
            // Headroom premium: pay up to 30% above formula price so
            // concurrent same-tick buys don't push the clearing price
            // out of reach.
            let limit = unit * 1.3;
            self.commands.push((
                self.me,
                ShipCommand::MarketBid {
                    port: idx,
                    good: plan.good,
                    tons,
                    limit_price: limit,
                },
            ));
        }

        self.assign_destination_port(plan.dest_port);
        Status::Success
    }

    fn act_divert_to_port(&mut self) -> Status {
        // Pick the nearest port that actually has provisions to sell.
        // Without the stockpile filter, a low-provisions ship would
        // dock-loop at the closest sugar island whose chandler ran
        // dry weeks ago: resupply returns Success without filling,
        // ship undocks still hungry, this branch reselects the same
        // dry port, and we ping-pong forever. Skipping dry ports lets
        // the ship instead keep its original course toward a real
        // resupply port (or, if every port within range is dry, push
        // on toward the trade destination and fail gracefully).
        let provisions = crate::goods::ids::PROVISIONS;
        let ship_flag = self.ship.faction;
        let nearest = self
            .ports
            .iter()
            .enumerate()
            .filter(|(idx, _)| {
                // Closed harbors can't save a hungry crew — `act_sail` would
                // turn us away on arrival. Also require Provisions to be
                // legally sellable on our flag (a port that prohibits
                // foreign provisioning is just as useless as a dry one).
                if !matches!(
                    self.policy.dock_legality(*idx, ship_flag),
                    crate::policy::DockLegality::Open
                ) {
                    return false;
                }
                if matches!(
                    self.policy.buy_legality(*idx, ship_flag, provisions),
                    crate::policy::TradeLegality::Prohibited
                ) {
                    return false;
                }
                self.markets
                    .get(*idx)
                    .map(|m| m.stockpile.get(provisions) > 0.5)
                    .unwrap_or(false)
            })
            .min_by(|(_, a), (_, b)| {
                let da = self.estimated_position().distance(a.position);
                let db = self.estimated_position().distance(b.position);
                da.partial_cmp(&db).unwrap()
            })
            .map(|(idx, _)| idx);
        if let Some(nearest_idx) = nearest {
            self.assign_destination_port(nearest_idx);
            Status::Success
        } else {
            // Every reachable port is out of provisions; let the BT
            // fall through to the regular sail-to-destination branch
            // instead of looping back to the same dry harbor.
            Status::Failure
        }
    }

    // ───────── Step 6: pursue / flee ─────────
    //
    // Both `act_pursue` and `act_flee` deliberately *do not* touch
    // `goal.destination` / `goal.dest_port` — those describe the
    // captain's *trade* plan. Pursue/flee is a momentary detour. As
    // soon as the prey escapes (or the threat slips outside
    // `PURSUE_BREAKOFF_NM`), the higher-priority pursue/flee branch
    // fails its condition and the BT falls through to the existing
    // sail-to-destination branch, which resumes the original voyage.

    fn act_pursue(&mut self) -> Status {
        // `goal.pursue_target` was set by `see_prey`; if it's somehow
        // missing here (target sank between condition and action,
        // tests calling act_pursue directly) bail out cleanly so the
        // selector falls through to the next branch.
        let Some(target_id) = self.goal.pursue_target else {
            return Status::Failure;
        };
        let Some(target) = self.snapshots.get(target_id) else {
            // Snapshot dropped: target left Sailing state (docked) or
            // was removed from the world. Clear and let the selector
            // re-evaluate next tick.
            self.goal.pursue_target = None;
            return Status::Failure;
        };
        // Steer at the target's truth position (lookouts see the real
        // hull on the horizon, not the captain's dead-reckoning of
        // where his own ship is). Commanded speed is whatever the
        // ship's stats allow — the pirate runs all canvas.
        let from = self.ship.position;
        let dx = target.position.x - from.x;
        let dy = target.position.y - from.y;
        let heading = normalize_angle(dx.atan2(dy).to_degrees());
        // Use the wind-adjusted top speed on this bearing so the
        // emitted command matches what the physics step will actually
        // deliver — no "commanded 12 kt, made good 4 kt" mismatch.
        let speed = speed_at_heading(heading, self.stats, self.wind);
        self.commands
            .push((self.me, ShipCommand::Steer { heading, speed }));
        // Step 7/8: emit broadside + boarding intents using the velocity
        // we're about to command (so the closest-approach gate sees the
        // pursuit, not the previous tick's lazier heading). Target
        // velocity comes from its snapshot at top-of-tick.
        let attacker_vel = velocity_from(heading, speed);
        self.maybe_fire_at(target_id, target.position, attacker_vel, target.velocity);
        // Only pirates board; merchants firing back in act_flee do not.
        if self.ship.policy == ShipPolicy::Pirate {
            self.maybe_board(target_id, target.position, attacker_vel, target);
        }
        // Phase 4 §3c-1 (symmetric redesign): return Success rather
        // than Running so the outer BT Selector resets its memory
        // each tick. The engaged subtree must be free to preempt on
        // the next hour (e.g., switch from Fight to Disengage once
        // ordnance runs out). Returning Running would pin the
        // Selector at this child forever, masking the engaged
        // branch's higher priority. CA-style re-evaluation requires
        // a fresh selector pass each hour.
        Status::Success
    }

    fn act_flee(&mut self) -> Status {
        let Some(threat_id) = self.goal.flee_from else {
            return Status::Failure;
        };
        let Some(threat) = self.snapshots.get(threat_id) else {
            self.goal.flee_from = None;
            return Status::Failure;
        };
        // Find the nearest port (any port — friendly-faction filtering
        // arrives with the relations matrix in Phase 4). Distance uses
        // the captain's estimate so a lost merchant runs toward
        // whatever port he *thinks* is closest, which can be subtly
        // wrong but is still better than not running.
        let est = self.estimated_position();
        let nearest_port_pos = self
            .ports
            .iter()
            .min_by(|a, b| {
                let da = est.distance(a.position);
                let db = est.distance(b.position);
                da.partial_cmp(&db).unwrap()
            })
            .map(|p| p.position);

        // Bearing to safety. If somehow there are no ports (test
        // harness), fall back to straight downwind — anything that
        // opens the range from the pirate is better than holding
        // course.
        let from = self.ship.position;
        let heading = match nearest_port_pos {
            Some(target) => {
                let dx = target.x - from.x;
                let dy = target.y - from.y;
                normalize_angle(dx.atan2(dy).to_degrees())
            }
            None => {
                // Downwind = direction the wind is blowing toward.
                normalize_angle(self.wind.u.atan2(self.wind.v).to_degrees())
            }
        };
        // If running directly toward the nearest port would steer us
        // *toward* the threat (port is past the pirate from our
        // perspective), bias the heading 90° away from the threat
        // instead. Simple test: if angle to port is within 30° of
        // angle to threat, sheer off perpendicular to the threat.
        let to_threat = {
            let dx = threat.position.x - from.x;
            let dy = threat.position.y - from.y;
            normalize_angle(dx.atan2(dy).to_degrees())
        };
        let heading = if angle_diff(heading, to_threat).abs() < 30.0 {
            // Run perpendicular to threat-bearing, on whichever side
            // is closer to the planned port heading.
            let port_side = normalize_angle(to_threat + 90.0);
            let starboard_side = normalize_angle(to_threat - 90.0);
            if angle_diff(port_side, heading).abs() < angle_diff(starboard_side, heading).abs() {
                port_side
            } else {
                starboard_side
            }
        } else {
            heading
        };
        let speed = speed_at_heading(heading, self.stats, self.wind);
        self.commands
            .push((self.me, ShipCommand::Steer { heading, speed }));
        // Step 7: fleeing merchants may still bark back if the pirate
        // closes inside cannon range. A lucky hit to the chaser's
        // rigging is the textbook way for a slower merchant to break
        // off a pursuit.
        let attacker_vel = velocity_from(heading, speed);
        self.maybe_fire_at(threat_id, threat.position, attacker_vel, threat.velocity);
        // See `act_pursue`: return Success so the BT Selector
        // re-evaluates priorities from the top each tick.
        Status::Success
    }

    /// Step 7: emit a `FireBroadside` if the target's *closest approach*
    /// over the next tick lies within `combat::CANNON_RANGE_NM` and this
    /// ship carries both gunpowder and cannon shot. Closest-approach
    /// rather than end-of-tick distance because at hourly granularity
    /// two ships closing at 5+ kt can pass through each other's combat
    /// envelope in a single tick — see `combat::min_distance_over_tick`.
    /// Silently no-ops otherwise (out of supply, out of range, no guns).
    fn maybe_fire_at(
        &mut self,
        target: ShipId,
        target_pos: crate::types::Position,
        attacker_vel: (f32, f32),
        target_vel: (f32, f32),
    ) {
        if self.stats.cannons == 0 {
            return;
        }
        let range = crate::combat::min_distance_over_tick(
            (self.ship.position.x, self.ship.position.y),
            attacker_vel,
            (target_pos.x, target_pos.y),
            target_vel,
        );
        if range > crate::combat::CANNON_RANGE_NM {
            return;
        }
        let (powder_need, shot_need) = crate::combat::broadside_supply_cost(self.stats.cannons);
        if self.ship.cargo.get(crate::goods::ids::GUNPOWDER) < powder_need
            || self.ship.cargo.get(crate::goods::ids::CANNON_SHOT) < shot_need
        {
            return;
        }
        self.commands
            .push((self.me, ShipCommand::FireBroadside { target }));
    }

    /// Step 8: emit an `AttemptBoard` if the target is within
    /// `combat::BOARDING_RANGE_NM` at closest approach this tick AND
    /// its rigging has been beaten below
    /// `combat::BOARDING_RIGGING_THRESHOLD`. Boarding only resolves
    /// when the prey can no longer outrun the grapples — otherwise
    /// the pirate just keeps chasing and shooting.
    fn maybe_board(
        &mut self,
        target: ShipId,
        target_pos: crate::types::Position,
        attacker_vel: (f32, f32),
        target_snap: &ShipSnapshot,
    ) {
        // Need a crew aboard to put a boarding party over.
        if self.ship.crew_alive < 2 {
            return;
        }
        // Target must be crippled enough that we can come alongside.
        if target_snap.rigging_frac >= crate::combat::BOARDING_RIGGING_THRESHOLD {
            return;
        }
        let range = crate::combat::min_distance_over_tick(
            (self.ship.position.x, self.ship.position.y),
            attacker_vel,
            (target_pos.x, target_pos.y),
            target_snap.velocity,
        );
        if range > crate::combat::BOARDING_RANGE_NM {
            return;
        }
        self.commands
            .push((self.me, ShipCommand::AttemptBoard { target }));
    }

    /// Condition: this pirate ship currently has a viable prey within
    /// detection range (or already-locked prey within breakoff range).
    /// Side effect: writes/clears `goal.pursue_target`.
    fn see_prey(&mut self) -> bool {
        let me_pos = self.ship.position;
        let me_id = self.me;

        // First: if we already have a lock, keep it as long as the
        // target is still Sailing (i.e., in the snapshot map) and
        // within breakoff range. Hysteresis prevents thrash at the
        // edge of `VISUAL_RANGE_NM`.
        if let Some(target_id) = self.goal.pursue_target {
            if let Some(t) = self.snapshots.get(target_id) {
                if me_pos.distance(t.position) <= PURSUE_BREAKOFF_NM
                    && t.policy != ShipPolicy::Pirate
                {
                    return true;
                }
            }
            // Lock invalidated; fall through to a fresh search.
            self.goal.pursue_target = None;
        }

        // Pirate quarry filter: any non-pirate ship within
        // `VISUAL_RANGE_NM` that's either fatter (more cargo capacity)
        // or slower than us. Sloops are the smallest hulls in the
        // current registry, so "fatter" catches every brig / bark /
        // ship / fluyt; "slower" is the corner case (a fully-laden
        // sloop is still worth chasing).
        let my_cargo = self.stats.cargo_capacity_tons;
        let my_top_speed = self.stats.speed_max;
        let mut best: Option<(ShipId, f32)> = None;
        for nbr_id in self
            .spatial
            .neighbors(me_pos, VISUAL_RANGE_NM, |id| id != me_id)
        {
            let Some(snap) = self.snapshots.get(nbr_id) else {
                continue;
            };
            if snap.policy == ShipPolicy::Pirate {
                continue;
            }
            let richer = snap.cargo_capacity_tons > my_cargo;
            let slower = snap.max_speed < my_top_speed;
            if !(richer || slower) {
                continue;
            }
            let d = me_pos.distance(snap.position);
            match best {
                None => best = Some((nbr_id, d)),
                Some((_, bd)) if d < bd => best = Some((nbr_id, d)),
                _ => {}
            }
        }
        if let Some((id, _)) = best {
            self.goal.pursue_target = Some(id);
            true
        } else {
            false
        }
    }

    /// Condition: this merchant ship currently has a Pirate within
    /// detection range (or already-tracked threat within breakoff
    /// range). Side effect: writes/clears `goal.flee_from`.
    fn see_threat(&mut self) -> bool {
        let me_pos = self.ship.position;
        let me_id = self.me;

        if let Some(threat_id) = self.goal.flee_from {
            if let Some(t) = self.snapshots.get(threat_id) {
                if me_pos.distance(t.position) <= PURSUE_BREAKOFF_NM
                    && t.policy == ShipPolicy::Pirate
                {
                    return true;
                }
            }
            self.goal.flee_from = None;
        }

        // Closest pirate within visual range, if any.
        let mut best: Option<(ShipId, f32)> = None;
        for nbr_id in self
            .spatial
            .neighbors(me_pos, VISUAL_RANGE_NM, |id| id != me_id)
        {
            let Some(snap) = self.snapshots.get(nbr_id) else {
                continue;
            };
            if snap.policy != ShipPolicy::Pirate {
                continue;
            }
            let d = me_pos.distance(snap.position);
            match best {
                None => best = Some((nbr_id, d)),
                Some((_, bd)) if d < bd => best = Some((nbr_id, d)),
                _ => {}
            }
        }
        if let Some((id, _)) = best {
            self.goal.flee_from = Some(id);
            true
        } else {
            false
        }
    }

    // ── Phase 4 §3c-1 (symmetric redesign): engaged-subtree leaves ──

    /// Action: mutually clear the engagement with `engaged_with` by
    /// emitting `Command::Disengage`. Also clears local goal pointers
    /// so the next tick's normal selector branches start clean. If
    /// `engaged_with` is somehow unset, falls through harmlessly.
    fn act_disengage(&mut self) -> Status {
        let Some(other) = self.ship.engaged_with else {
            return Status::Failure;
        };
        self.commands
            .push((self.me, ShipCommand::Disengage { other }));
        // Drop stale pursue/flee targets so the next tick re-evaluates
        // see_prey / see_threat freshly rather than locking back on
        // the just-disengaged ship inside the cooldown window.
        if self.goal.pursue_target == Some(other) {
            self.goal.pursue_target = None;
        }
        if self.goal.flee_from == Some(other) {
            self.goal.flee_from = None;
        }
        Status::Success
    }

    /// Action: hold station — emit no Steer command. The ship coasts
    /// on its previous heading and speed (physics integrates the last
    /// commanded velocity). This is the default for an engaged ship
    /// that has no compelling fight, flee, or disengage decision —
    /// typically because it is mid-reload and waiting for the next
    /// sub-tick fire window.
    fn act_hold(&mut self) -> Status {
        Status::Success
    }

    /// Action (Phase 4 §3c-2): strike colors — emit `Strike { to: other }`
    /// where `other` is the current engagement counterpart. World
    /// resolution clears engagement on both ships and dispatches the
    /// surrendered hull through the shared prize-action machinery.
    /// Also clears any stale pursue/flee target on this ship so post-
    /// resolution (if the prize is `release`'d) it does not immediately
    /// try to re-engage its captor.
    fn act_strike(&mut self) -> Status {
        let Some(other) = self.ship.engaged_with else {
            return Status::Failure;
        };
        self.commands
            .push((self.me, ShipCommand::Strike { to: other }));
        if self.goal.pursue_target == Some(other) {
            self.goal.pursue_target = None;
        }
        if self.goal.flee_from == Some(other) {
            self.goal.flee_from = None;
        }
        Status::Success
    }

    /// Tactical condition (Phase 4 §3c-2): should this ship strike
    /// colors this hour?
    ///
    /// True when *both* of the following hold:
    ///   * **Position is hopeless** — `morale × hull_fraction` has
    ///     fallen below `STRIKE_THRESHOLD` (0.15). Captures the
    ///     "fight is lost, save the crew" inflection.
    ///   * **Cannot outrun the counterpart** — own effective speed
    ///     (`speed_max × rigging_frac`) is no greater than the
    ///     counterpart's, *or* own rigging is itself crippled below
    ///     the boarding threshold (the rigging is so torn that even a
    ///     theoretically-faster hull cannot run).
    ///
    /// Priority is above `should_disengage` and `should_flee` so a
    /// catastrophically beaten ship surrenders cleanly rather than
    /// trying to disengage under fire or limp away.
    fn should_strike(&self) -> bool {
        let Some(other) = self.ship.engaged_with else {
            return false;
        };
        let Some(other_snap) = self.snapshots.get(other) else {
            return false;
        };

        let hull_frac = if self.stats.hull_integrity_max > 0.0 {
            (self.ship.hull_integrity / self.stats.hull_integrity_max).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let composite = self.ship.morale * hull_frac;
        if composite >= crate::combat::STRIKE_THRESHOLD {
            return false;
        }

        let my_rig_frac = if self.stats.rigging_integrity_max > 0.0 {
            (self.ship.rigging_integrity / self.stats.rigging_integrity_max).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if my_rig_frac < crate::combat::BOARDING_RIGGING_THRESHOLD {
            // Rigging is so torn the hull cannot run regardless of nominal speed.
            return true;
        }

        let my_eff_speed = self.stats.speed_max * my_rig_frac;
        let their_eff_speed = other_snap.max_speed * other_snap.rigging_frac.max(0.0);
        my_eff_speed <= their_eff_speed
    }

    /// Action (Phase 4 §3c-3): commit to boarding the engaged
    /// counterpart. Steers at the target at full wind-adjusted speed,
    /// emits a softening broadside if magazine + range permit
    /// (`maybe_fire_at`), and emits `AttemptBoard` if the ship will
    /// close inside `BOARDING_RANGE_NM` this tick
    /// (`maybe_board`). The world's `resolve_boarding` re-gates range
    /// and rigging and, on attacker-win, dispatches the prize through
    /// the shared `resolve_prize_action` resolver (§3c-2). Returns
    /// `Success` to release Selector memory each tick so the engaged
    /// subtree re-evaluates priorities every hour.
    fn act_board(&mut self) -> Status {
        // `should_board` set `goal.pursue_target = engaged_with`. Bail
        // cleanly if state has shifted under us between condition and
        // action (e.g., target struck/sank in another command's wake).
        let Some(target_id) = self.goal.pursue_target else {
            return Status::Failure;
        };
        let Some(target) = self.snapshots.get(target_id) else {
            self.goal.pursue_target = None;
            return Status::Failure;
        };
        let from = self.ship.position;
        let dx = target.position.x - from.x;
        let dy = target.position.y - from.y;
        let heading = normalize_angle(dx.atan2(dy).to_degrees());
        let speed = speed_at_heading(heading, self.stats, self.wind);
        self.commands
            .push((self.me, ShipCommand::Steer { heading, speed }));
        let attacker_vel = velocity_from(heading, speed);
        // Soften the deck if we can; never *required* for boarding to
        // resolve, but a free contribution if we have magazine.
        self.maybe_fire_at(target_id, target.position, attacker_vel, target.velocity);
        // The real ask — emit the boarding intent if range allows.
        self.maybe_board(target_id, target.position, attacker_vel, target);
        Status::Success
    }

    /// Tactical condition (Phase 4 §3c-3): should this ship commit to
    /// boarding the engaged counterpart this hour?
    ///
    /// True when *all* of:
    ///   * I am a Pirate (only pirates board in v1; the naval/
    ///     privateer board path lands with Phase 5 relations).
    ///   * I am engaged with someone visible in the snapshot map.
    ///   * Engaged counterpart's rigging fraction is below
    ///     `BOARDING_RIGGING_THRESHOLD` (she cannot slip the grapples).
    ///   * I have at least two crew alive (minimum boarding party).
    ///
    /// Side effect on success: sets `goal.pursue_target = engaged_with`
    /// so `act_board` (which reads `pursue_target` like `act_pursue`)
    /// steers at and grapples the engaged counterpart.
    ///
    /// Priority is *above* `should_fight` so a magazine-empty pirate
    /// commits to the grapple instead of falling through to flee, and
    /// *above* `should_disengage` so the disengage rule's "no fire +
    /// no board option" line cannot itself preempt a viable board.
    fn should_board(&mut self) -> bool {
        if self.ship.policy != ShipPolicy::Pirate {
            return false;
        }
        let Some(other) = self.ship.engaged_with else {
            return false;
        };
        let Some(other_snap) = self.snapshots.get(other) else {
            return false;
        };
        if self.ship.crew_alive < 2 {
            return false;
        }
        if other_snap.rigging_frac >= crate::combat::BOARDING_RIGGING_THRESHOLD {
            return false;
        }
        self.goal.pursue_target = Some(other);
        true
    }

    /// Tactical condition: should this ship break off the engagement
    /// this hour?
    ///
    /// True if any of:
    ///   * **Lost contact** — engaged counterpart is not in the
    ///     snapshot map (sailed out of visual range, docked, or
    ///     reaped). No point keeping the flag set.
    ///   * **Out of ordnance with no boarding option** — cannot fire
    ///     a broadside this hour AND either I'm not a Pirate or the
    ///     target's rigging is still healthy enough to outrun a
    ///     grapple.
    ///   * **Badly outclassed** — my hull is below 30% while the
    ///     target's is above 70%. Time to live to fight another day.
    ///   * **Outnumbered** — counting hostile (different-policy)
    ///     vs allied (same-policy) ships within visual range, the
    ///     hostiles outnumber my allies by more than one. The
    ///     engaged counterpart counts as one hostile.
    fn should_disengage(&self) -> bool {
        let Some(other) = self.ship.engaged_with else {
            return false;
        };
        // Lost contact.
        let Some(other_snap) = self.snapshots.get(other) else {
            return true;
        };

        // Out of ordnance + no realistic board option.
        let no_fire = if self.stats.cannons == 0 {
            true
        } else {
            let (need_p, need_s) = crate::combat::broadside_supply_cost(self.stats.cannons);
            self.ship.cargo.get(crate::goods::ids::GUNPOWDER) < need_p
                || self.ship.cargo.get(crate::goods::ids::CANNON_SHOT) < need_s
        };
        let can_board = self.ship.policy == ShipPolicy::Pirate
            && self.ship.crew_alive >= 2
            && other_snap.rigging_frac < crate::combat::BOARDING_RIGGING_THRESHOLD;
        if no_fire && !can_board {
            return true;
        }

        // Badly outclassed.
        let my_hull_frac = if self.stats.hull_integrity_max > 0.0 {
            (self.ship.hull_integrity / self.stats.hull_integrity_max).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if my_hull_frac < 0.3 && other_snap.hull_frac > 0.7 {
            return true;
        }

        // Outnumbered. Walk visible neighbours (cheap — handful of
        // ships per tick at sea); count by policy. The engaged
        // counterpart shows up in the hostile tally.
        let mut hostiles: u16 = 0;
        let mut allies: u16 = 0;
        for nbr_id in self
            .spatial
            .neighbors(self.ship.position, VISUAL_RANGE_NM, |id| id != self.me)
        {
            let Some(snap) = self.snapshots.get(nbr_id) else {
                continue;
            };
            if snap.policy == self.ship.policy {
                allies += 1;
            } else {
                hostiles += 1;
            }
        }
        if hostiles > allies + 1 {
            return true;
        }

        false
    }

    /// Tactical condition: should this ship press the attack this
    /// hour? True if it has ordnance AND at least one of:
    ///   * firepower advantage (own cannons ≥ target cannons),
    ///   * speed advantage (own effective speed > target effective
    ///     speed — can dictate the range),
    ///   * target's rigging already crippled (can close to board for
    ///     pirates; safe to slug for everyone).
    ///
    /// Side effect on success: sets `goal.pursue_target = engaged_with`
    /// so `act_pursue` (which reads from `goal.pursue_target`) steers
    /// at and fires on the engaged counterpart.
    fn should_fight(&mut self) -> bool {
        let Some(other) = self.ship.engaged_with else {
            return false;
        };
        let Some(other_snap) = self.snapshots.get(other) else {
            return false;
        };

        // Need at least one broadside's worth of magazine.
        if self.stats.cannons == 0 {
            return false;
        }
        let (need_p, need_s) = crate::combat::broadside_supply_cost(self.stats.cannons);
        if self.ship.cargo.get(crate::goods::ids::GUNPOWDER) < need_p
            || self.ship.cargo.get(crate::goods::ids::CANNON_SHOT) < need_s
        {
            return false;
        }

        let firepower_edge = self.stats.cannons >= other_snap.cannons;
        let my_eff_speed = self.stats.speed_max
            * (self.ship.rigging_integrity / self.stats.rigging_integrity_max).max(0.0);
        let their_eff_speed = other_snap.max_speed * other_snap.rigging_frac.max(0.0);
        let speed_edge = my_eff_speed > their_eff_speed;
        let prey_crippled = other_snap.rigging_frac < crate::combat::BOARDING_RIGGING_THRESHOLD;

        if firepower_edge || speed_edge || prey_crippled {
            self.goal.pursue_target = Some(other);
            true
        } else {
            false
        }
    }

    /// Tactical condition: should this ship flee while still firing
    /// back? True whenever the ship is engaged and has not chosen to
    /// fight (the BT tries `should_fight` first). The ordnance check
    /// is skipped: a magazine-empty merchant should still run rather
    /// than sit and reload while the pirate closes — and `act_flee`
    /// gracefully no-ops on the back-fire if there's no shot left.
    ///
    /// Side effect on success: sets `goal.flee_from = engaged_with`
    /// so `act_flee` heads away from the engaged counterpart.
    fn should_flee(&mut self) -> bool {
        let Some(other) = self.ship.engaged_with else {
            return false;
        };
        if self.snapshots.get(other).is_none() {
            return false;
        }
        self.goal.flee_from = Some(other);
        true
    }
}

impl<'a> BtContext for ShipBtContext<'a> {
    fn execute_action(&mut self, id: usize) -> Status {
        match id {
            ACT_SAIL => self.act_sail(),
            ACT_RESUPPLY => self.act_resupply(),
            ACT_CAREEN => self.act_careen(),
            ACT_UNDOCK => self.act_undock(),
            ACT_CHOOSE_DESTINATION => self.act_choose_destination(),
            ACT_SELL_ALL => self.act_sell_all(),
            ACT_BUY_BEST => self.act_buy_best(),
            ACT_DIVERT_TO_PORT => self.act_divert_to_port(),
            ACT_PURSUE => self.act_pursue(),
            ACT_FLEE => self.act_flee(),
            ACT_DISENGAGE => self.act_disengage(),
            ACT_HOLD => self.act_hold(),
            ACT_STRIKE => self.act_strike(),
            ACT_BOARD => self.act_board(),
            _ => Status::Failure,
        }
    }

    fn check_condition(&mut self, id: usize) -> Status {
        let result = match id {
            COND_IS_DOCKED => self.ship.state == ShipState::Docked,
            COND_HAS_DESTINATION => self.goal.destination.is_some(),
            COND_IS_LOW_PROVISIONS => {
                if self.ship.state != ShipState::Sailing {
                    false
                } else {
                    let days_remaining = self.ship.provisions_days_remaining(self.stats);
                    // If we have a planned destination, only flag low
                    // provisions when we estimate we *cannot* reach it
                    // with a small safety buffer. A ship within range
                    // of its destination keeps pushing on even if the
                    // hold is nearly empty — diverting at 80% of a
                    // transatlantic voyage just wastes the trip.
                    if let Some(dest) = self.goal.destination {
                        let dist = self.estimated_position().distance(dest);
                        let voyage_days = self.stats.estimated_voyage_days(dist);
                        days_remaining < voyage_days + REACHABILITY_BUFFER_DAYS
                    } else {
                        // No destination → use the hard panic floor.
                        days_remaining < HARD_PANIC_DAYS
                    }
                }
            }
            COND_IS_SAILING_PIRATE => {
                self.ship.state == ShipState::Sailing && self.ship.policy == ShipPolicy::Pirate
            }
            COND_IS_SAILING_MERCHANT => {
                self.ship.state == ShipState::Sailing && self.ship.policy == ShipPolicy::Merchant
            }
            COND_SEE_PREY => self.see_prey(),
            COND_SEE_THREAT => self.see_threat(),
            // Phase 4 §3c-1 (symmetric redesign): gate for the entire
            // engaged subtree. Only true while sailing — a docked
            // ship cannot be mid-fight (engagement clears via
            // counterpart-gone if the other side docks too; if not,
            // the next tactical pass will Disengage on lost-contact).
            COND_IS_ENGAGED => {
                self.ship.state == ShipState::Sailing && self.ship.engaged_with.is_some()
            }
            COND_SHOULD_DISENGAGE => self.should_disengage(),
            COND_SHOULD_FIGHT => self.should_fight(),
            COND_SHOULD_FLEE => self.should_flee(),
            COND_SHOULD_STRIKE => self.should_strike(),
            COND_SHOULD_BOARD => self.should_board(),
            _ => false,
        };
        if result {
            Status::Success
        } else {
            Status::Failure
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ship::{Ship, ShipState, ShipStats};
    use crate::types::Position;

    fn make_ship(policy: ShipPolicy) -> Ship {
        let mut s = Ship::new(Position { x: 0.0, y: 0.0 }, ShipState::Sailing);
        s.policy = policy;
        s
    }

    /// Phase 4 §1.3 — the magazine target scales linearly with cannon
    /// count, expressed in "broadsides of reserve". A 0-gun ship buys
    /// nothing.
    #[test]
    fn ordnance_target_scales_with_cannons() {
        let mut stats_small = ShipStats::sloop();
        stats_small.cannons = 8;
        let mut stats_big = ShipStats::sloop();
        stats_big.cannons = 24;
        let mut stats_unarmed = ShipStats::sloop();
        stats_unarmed.cannons = 0;
        let ship = make_ship(ShipPolicy::Merchant);

        let (p_small, s_small) = ordnance_target(&ship, &stats_small);
        let (p_big, s_big) = ordnance_target(&ship, &stats_big);
        let (p_zero, s_zero) = ordnance_target(&ship, &stats_unarmed);

        assert!(p_small > 0.0 && s_small > 0.0);
        assert!(p_big > p_small * 2.9 && p_big < p_small * 3.1);
        assert!(s_big > s_small * 2.9 && s_big < s_small * 3.1);
        assert_eq!(p_zero, 0.0);
        assert_eq!(s_zero, 0.0);
    }

    /// Phase 4 §1.3 — pirates aim to keep twice as many broadsides aboard
    /// as merchants, holding cannon count constant.
    #[test]
    fn pirate_target_exceeds_merchant_target() {
        let mut stats = ShipStats::sloop();
        stats.cannons = 12;
        let merchant = make_ship(ShipPolicy::Merchant);
        let pirate = make_ship(ShipPolicy::Pirate);

        let (p_merchant, _) = ordnance_target(&merchant, &stats);
        let (p_pirate, _) = ordnance_target(&pirate, &stats);

        assert!(
            p_pirate > p_merchant * 1.9 && p_pirate < p_merchant * 2.1,
            "pirate target {} should be ~2× merchant {}",
            p_pirate,
            p_merchant
        );
    }
}
