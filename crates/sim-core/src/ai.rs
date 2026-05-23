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

// --- Condition IDs ---
const COND_IS_DOCKED: usize = 0;
const COND_HAS_DESTINATION: usize = 1;
const COND_IS_LOW_PROVISIONS: usize = 2;
const COND_IS_SAILING_PIRATE: usize = 3;
const COND_IS_SAILING_MERCHANT: usize = 4;
const COND_SEE_PREY: usize = 5;
const COND_SEE_THREAT: usize = 6;

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
const HOME_PORT_FLOAT_SILVER: f32 = 500.0;

/// On outfitting at home, the ship will try to top its strongbox up
/// to this many times the estimated cost of one full outbound hold.
/// Gives a cushion for partial-cargo top-ups at intermediate ports
/// and small contingencies en route.
const OUTFIT_DRAW_MULTIPLE: f32 = 2.0;

/// No single outfit draw can take more than this fraction of the home
/// port's silver. Prevents a busy yard's working capital from being
/// drained by one ship's outbound cargo.
const OUTFIT_PORT_FRACTION_CAP: f32 = 0.2;

/// Tramping credit: at any non-home port, a captain with no silver
/// but a profitable arbitrage opportunity may draw against the port
/// factor (consigned cargo / freight charter) up to this fraction of
/// the port's treasury. Booked as ship debt and repaid at the next
/// docking from sale proceeds.
const TRAMP_PORT_FRACTION_CAP: f32 = 0.10;

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
        // Priority 2 (Step 6): A sailing pirate that sees prey chases
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
    /// Simple RNG state (xorshift) for destination selection.
    rng_state: u64,
    /// Independent RNG state for the navigator (DR noise, fix noise).
    /// Kept separate so per-ship navigation jitter doesn't perturb the
    /// destination-choice RNG sequence — important for reproducible
    /// bench economics across noise tuning passes.
    nav_rng_state: u64,
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
            rng_state: 12345,
            nav_rng_state: 0x9E3779B97F4A7C15,
            prev_truth: None,
        }
    }

    pub fn with_destination(dest: Position) -> Self {
        Self {
            goal: NavGoal::with_destination(dest),
            tree: build_ship_bt(),
            state: BtState::new(),
            rng_state: 12345,
            nav_rng_state: 0x9E3779B97F4A7C15,
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
            rng_state: seed,
            nav_rng_state: seed ^ 0x9E3779B97F4A7C15,
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
            nav::apply_dr_error(estimate, inputs.ship.speed, 1.0, &mut self.nav_rng_state);
            nav::try_noon_sight(
                estimate,
                truth,
                inputs.day_of_year,
                &mut self.goal.last_noon_sight_day,
                &mut self.nav_rng_state,
            );
            // Landmark fix only while underway. A docked captain knows
            // where the dock is — re-snapping to truth+N(0,1) every
            // hour would just add noise to a known position and
            // perturb departure headings.
            if inputs.ship.speed > 0.1 {
                nav::try_landmark_fix(estimate, truth, inputs.ports, &mut self.nav_rng_state);
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
            rng_state: &mut self.rng_state,
            pathfind: inputs.pathfind,
            markets: inputs.markets,
            goods: inputs.goods,
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
    pub markets: &'a mut [PortMarket],
    pub goods: &'a GoodsRegistry,
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
}

/// Simple xorshift64 RNG — deterministic and fast.
fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
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
    rng_state: &'a mut u64,
    pathfind: Option<&'a PathfindContext<'a>>,
    markets: &'a mut [PortMarket],
    goods: &'a GoodsRegistry,
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
            self.ship.dock();
            self.ship.dock_action = DockAction::Idle;
            self.ship.nav.docked_at_port = port_idx;
            self.goal.destination = None;
            self.goal.dest_port = None;
            self.ship.nav.clear_path();
            // Settle any outstanding chandler/freight debt at
            // this port first — creditors come before owners.
            // Fungible: it doesn't matter which port originally
            // advanced the credit; the merchant network settles
            // it via bills of exchange between correspondents.
            if let Some(idx) = port_idx {
                if idx < self.markets.len() {
                    self.markets[idx].collect_debt(self.ship, HOME_PORT_FLOAT_SILVER);
                }
            }
            // Home-port settlement: if this is the owner port,
            // the supercargo books proceeds with the owners.
            // Silver above the operating float is paid into
            // the port treasury (dividend to shareholders);
            // the ship keeps just enough to cover provisions
            // and incidentals at the next port of call.
            if let (Some(idx), Some(owner)) = (port_idx, self.ship.owner_port) {
                if idx == owner && idx < self.markets.len() {
                    let paid =
                        self.markets[idx].deposit_owner_profit(self.ship, HOME_PORT_FLOAT_SILVER);
                    self.ship.lifetime_dividends += paid;
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
        let done = match self.ship.nav.docked_at_port {
            Some(idx) if idx < self.markets.len() => {
                self.ship
                    .tick_resupply_at_market(self.stats, &mut self.markets[idx], self.goods)
            }
            // Unknown / out-of-range port — fall back to free resupply.
            _ => self.ship.tick_resupply(self.stats),
        };
        if done {
            self.ship.dock_action = DockAction::Idle;
            Status::Success
        } else {
            Status::Running
        }
    }

    fn act_careen(&mut self) -> Status {
        self.ship.dock_action = DockAction::Careening;
        if self.ship.tick_careen() {
            self.ship.dock_action = DockAction::Idle;
            Status::Success
        } else {
            Status::Running
        }
    }

    fn act_undock(&mut self) -> Status {
        if self.goal.destination.is_some() {
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
        let mut idx = (xorshift64(self.rng_state) as usize) % self.ports.len();
        if self.estimated_position().distance(self.ports[idx].position) < 20.0 {
            idx = (idx + 1) % self.ports.len();
        }
        self.assign_destination_port(idx);
        Status::Success
    }

    fn act_sell_all(&mut self) -> Status {
        // Sell every ton of cargo we arrived with at the docked port's
        // market. Silent no-op when docked_at_port isn't set or is out of
        // range (test ports beyond the markets slice).
        let Some(idx) = self.ship.nav.docked_at_port else {
            return Status::Success;
        };
        if idx >= self.markets.len() {
            return Status::Success;
        }
        let market = &mut self.markets[idx];
        let entries: Vec<(crate::goods::GoodId, f32)> = self.ship.cargo.iter().collect();
        for (gid, tons) in entries {
            if tons > 0.0 {
                // Best-effort: ignore "port out of silver" by
                // selling whatever the port can afford. For
                // v1 we just attempt the full amount; if the
                // port can't pay, the cargo stays aboard and
                // we'll try again on the next leg.
                let _ = market.sell(self.ship, gid, tons, self.goods);
            }
        }
        Status::Success
    }

    fn act_buy_best(&mut self) -> Status {
        // Pick the best (good, dest) and load up. Sets the
        // ship's destination as a side effect so ACT_UNDOCK
        // has somewhere to go.
        let Some(idx) = self.ship.nav.docked_at_port else {
            return Status::Success;
        };
        if idx >= self.markets.len() {
            return Status::Success;
        }
        // Reachability budget: assume we leave with a full
        // provisions hold. The trade planner uses this to
        // skip destinations we can't physically reach.
        let daily = self.stats.daily_provision_consumption().max(1e-6);
        let provision_budget_days = self.stats.provision_capacity / daily;
        // Home bias: as the ship's strongbox swells above the
        // operating float, increase its pull toward home. This
        // models the supercargo's fiduciary duty to settle
        // proceeds with the owners — a ship sitting on a fat
        // purse won't keep chasing marginal arbitrage forever.
        let home_bias = self.ship.owner_port.map(|home_idx| {
            let surplus = (self.ship.silver - HOME_PORT_FLOAT_SILVER).max(0.0);
            // Roughly: a ship sitting on +5k surplus pulls
            // toward home with a 25 peso/ton bias, fully
            // dominating ordinary arbitrage. The cap of 200
            // ensures a flush ship will home-in even against
            // the fattest opportunistic margin.
            crate::trade::HomeBias {
                home_port: home_idx,
                bias_pesos_per_ton: (surplus / 200.0).min(200.0),
            }
        });
        let plan = crate::trade::find_best_trade(
            idx,
            self.ports,
            self.markets,
            self.goods,
            self.stats,
            provision_budget_days,
            home_bias,
        );
        let plan = match plan {
            Some(p) => p,
            None => return Status::Success,
        };

        // Buy as many tons as we can afford, that fit in the
        // hold, and that the port can supply.
        let market = &mut self.markets[idx];
        let unit = market.buy_price(plan.good, self.goods).max(0.0001);
        let cargo_room = self.stats.cargo_capacity_tons - self.ship.cargo.total_tons();

        // Outfitting draw: if this is the owner port, top the
        // ship's strongbox up from the port treasury before
        // computing what we can afford. Historically the
        // outbound cargo was paid for with capital drawn from
        // the home-port owners, not from cash earned on prior
        // voyages — those proceeds were settled on arrival.
        if let Some(owner) = self.ship.owner_port {
            if owner == idx {
                let want_tons = cargo_room.min(market.stockpile.get(plan.good));
                let target = unit * want_tons * OUTFIT_DRAW_MULTIPLE;
                market.draw_for_outfit(self.ship, target, OUTFIT_PORT_FRACTION_CAP);
            }
        }

        // Tramping / freight credit: at any other port, if we
        // still can't load anything meaningful (too little
        // silver for the hold space we have), take cargo on
        // consignment from the local factor. Booked as debt;
        // repaid out of the sale proceeds at the destination.
        let want_tons = cargo_room.min(market.stockpile.get(plan.good));
        let need_silver = unit * want_tons;
        if self.ship.silver < need_silver
            && self.ship.debt < crate::ship::MAX_SHIP_DEBT
            && want_tons > 0.0
        {
            let shortfall = need_silver - self.ship.silver;
            market.extend_credit(
                self.ship,
                shortfall,
                TRAMP_PORT_FRACTION_CAP,
                crate::ship::MAX_SHIP_DEBT,
            );
        }

        let affordable = self.ship.silver / unit;
        let in_stock = market.stockpile.get(plan.good);
        let tons = cargo_room.min(affordable).min(in_stock).max(0.0);
        if tons > 0.0 {
            let _ = market.buy(self.ship, self.stats, plan.good, tons, self.goods);
        }

        // Always set the destination — even when the buy fell
        // through (broke / no room) — so the ship still sails
        // to the chosen port and tries selling/buying again.
        self.assign_destination_port(plan.dest_port);
        Status::Success
    }

    fn act_divert_to_port(&mut self) -> Status {
        if let Some((nearest_idx, _)) = self.ports.iter().enumerate().min_by(|(_, a), (_, b)| {
            let da = self.estimated_position().distance(a.position);
            let db = self.estimated_position().distance(b.position);
            da.partial_cmp(&db).unwrap()
        }) {
            self.assign_destination_port(nearest_idx);
        }
        Status::Success
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
        Status::Running
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
        Status::Running
    }

    // ───────── Step 6: see_prey / see_threat ─────────

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
            _ => false,
        };
        if result {
            Status::Success
        } else {
            Status::Failure
        }
    }
}
