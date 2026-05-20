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
use crate::harbor::HarborMap;
use crate::nav::NavState;
use crate::pathfind::{self, PathfindContext};
use crate::port::Port;
use crate::ship::{Ship, ShipState, ShipStats};
use crate::types::{Position, WindVector};

// --- Action IDs ---
const ACT_SAIL: usize = 0;
const ACT_RESUPPLY: usize = 1;
const ACT_CAREEN: usize = 2;
const ACT_UNDOCK: usize = 3;
const ACT_CHOOSE_DESTINATION: usize = 4;
const ACT_DIVERT_TO_PORT: usize = 5;

// --- Condition IDs ---
const COND_IS_DOCKED: usize = 0;
const COND_HAS_DESTINATION: usize = 1;
const COND_IS_LOW_PROVISIONS: usize = 2;

/// Below this many days of provisions, divert to nearest port.
const LOW_PROVISIONS_DAYS: f32 = 10.0;

/// If a ship's waypoint queue is empty but it is still farther than this
/// from its destination, request a fresh plan. Larger than `ARRIVAL_NM` so
/// that ships about to dock don't waste a planner call, but small enough
/// that a ship drifting past its smoothed route promptly recovers.
const REPLAN_DISTANCE_NM: f32 = 25.0;

/// What the ship is doing while docked (for display purposes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DockAction {
    Idle,
    Resupplying,
    Careening,
}

/// Build the ship AI behavior tree.
fn build_ship_bt() -> Behavior {
    let dock_tree = Behavior::Sequence(vec![
        Behavior::Action(ACT_RESUPPLY),
        Behavior::Action(ACT_CAREEN),
        Behavior::Action(ACT_UNDOCK),
    ]);

    Behavior::Selector(vec![
        // Priority 1: If docked, do port activities
        Behavior::Sequence(vec![
            Behavior::Condition(COND_IS_DOCKED),
            dock_tree,
        ]),
        // Priority 2: If low on provisions, divert to nearest port
        Behavior::Sequence(vec![
            Behavior::Condition(COND_IS_LOW_PROVISIONS),
            Behavior::Action(ACT_DIVERT_TO_PORT),
        ]),
        // Priority 3: If has destination, sail
        Behavior::Sequence(vec![
            Behavior::Condition(COND_HAS_DESTINATION),
            Behavior::Action(ACT_SAIL),
        ]),
        // Priority 4: Choose a new destination
        Behavior::Action(ACT_CHOOSE_DESTINATION),
    ])
}

/// AI state for a single ship.
pub struct ShipAI {
    pub nav: NavState,
    pub dock_action: DockAction,
    tree: Behavior,
    state: BtState,
    /// Simple RNG state (xorshift) for destination selection.
    rng_state: u64,
}

impl ShipAI {
    pub fn new() -> Self {
        Self {
            nav: NavState::new(),
            dock_action: DockAction::Idle,
            tree: build_ship_bt(),
            state: BtState::new(),
            rng_state: 12345,
        }
    }

    pub fn with_destination(dest: Position) -> Self {
        Self {
            nav: NavState::with_destination(dest),
            dock_action: DockAction::Idle,
            tree: build_ship_bt(),
            state: BtState::new(),
            rng_state: 12345,
        }
    }

    /// Create AI with a specific RNG seed (for variety among multiple ships).
    pub fn with_seed(seed: u64) -> Self {
        Self {
            nav: NavState::new(),
            dock_action: DockAction::Idle,
            tree: build_ship_bt(),
            state: BtState::new(),
            rng_state: seed,
        }
    }

    /// Called each tick: run the behavior tree.
    ///
    /// `pathfind` is optional — when `Some`, the AI will plan obstacle-aware
    /// waypoint routes whenever it picks a new destination. When `None` it
    /// falls back to straight-line navigation (useful for unit tests with
    /// synthetic toy ports).
    pub fn tick(
        &mut self,
        ship: &mut Ship,
        stats: &ShipStats,
        wind: &WindVector,
        ports: &[Port],
        harbors: &HarborMap,
        pathfind: Option<&PathfindContext<'_>>,
    ) {
        let mut ctx = ShipBtContext {
            ship,
            stats,
            wind,
            nav: &mut self.nav,
            dock_action: &mut self.dock_action,
            ports,
            harbors,
            rng_state: &mut self.rng_state,
            pathfind,
        };

        let status = bt::tick(&self.tree, &mut self.state, &mut ctx, 0);

        // If the tree completed (Success/Failure), reset for next tick
        if status != Status::Running {
            self.state.reset();
        }
    }

    /// Give the AI a new destination.
    pub fn set_destination(&mut self, dest: Position) {
        self.nav.destination = Some(dest);
        self.nav.dest_port = None;
        self.nav.clear_path();
    }
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
struct ShipBtContext<'a> {
    ship: &'a mut Ship,
    stats: &'a ShipStats,
    wind: &'a WindVector,
    nav: &'a mut NavState,
    dock_action: &'a mut DockAction,
    ports: &'a [Port],
    harbors: &'a HarborMap,
    rng_state: &'a mut u64,
    pathfind: Option<&'a PathfindContext<'a>>,
}

impl<'a> ShipBtContext<'a> {
    /// Set a destination *port* (by index) and plan a path to its harbor
    /// zone. The destination is recorded even if planning fails — the ship
    /// then falls back to straight-line nav with reactive deflection.
    fn assign_destination_port(&mut self, port_index: usize) {
        let port = &self.ports[port_index];
        self.nav.destination = Some(port.position);
        self.nav.dest_port = Some(port_index);
        self.nav.clear_path();

        if let (Some(pf), Some(harbor)) = (self.pathfind, self.harbors.for_port(port_index)) {
            if let Some(path) = pathfind::find_path_to_harbor(pf, self.ship.position, harbor) {
                self.nav.set_path(path);
            }
        }
    }

    /// Re-plan a path to the current destination port without resetting
    /// other navigation state. Called when the existing waypoint queue has
    /// emptied but the ship is still mid-voyage.
    fn replan_to_port(&mut self, port_index: usize) {
        if let (Some(pf), Some(harbor)) = (self.pathfind, self.harbors.for_port(port_index)) {
            if let Some(path) = pathfind::find_path_to_harbor(pf, self.ship.position, harbor) {
                self.nav.set_path(path);
            }
        }
    }

    /// True if the ship is in its destination port's harbor zone.
    fn in_destination_harbor(&self) -> bool {
        let port_idx = match self.nav.dest_port {
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
        harbor.contains_pos(pf.land, self.ship.position)
    }
}

impl<'a> BtContext for ShipBtContext<'a> {
    fn execute_action(&mut self, id: usize) -> Status {
        match id {
            ACT_SAIL => {
                // Harbor-zone arrival: if we're inside the destination's
                // harbor zone, transition to Docked immediately. The literal
                // port coordinate may still be far away (e.g., Philadelphia
                // up the Delaware) — that's fine.
                if self.in_destination_harbor() {
                    self.ship.dock();
                    *self.dock_action = DockAction::Idle;
                    self.nav.destination = None;
                    self.nav.dest_port = None;
                    self.nav.clear_path();
                    return Status::Success;
                }

                // Replan when our planned route has been exhausted but we're
                // still nowhere near the destination. Without this, a ship
                // that drifts/tacks past its last waypoint will dead-reckon
                // straight toward the destination — through land, if any —
                // and pin against the coast.
                if self.nav.waypoints.is_empty() && self.nav.dest_port.is_some() {
                    if let Some(dest) = self.nav.destination {
                        if self.ship.position.distance(dest) > REPLAN_DISTANCE_NM {
                            if let Some(idx) = self.nav.dest_port {
                                self.replan_to_port(idx);
                            }
                        }
                    }
                }

                let land = self.pathfind.map(|c| c.land);
                if let Some(s) = self.nav.compute_steering(self.ship.position, self.stats, self.wind, land) {
                    self.ship.set_steering(s.heading, s.speed);
                    Status::Running
                } else {
                    // Arrived at a free-form destination (no harbor zone).
                    self.ship.dock();
                    *self.dock_action = DockAction::Idle;
                    Status::Success
                }
            }
            ACT_RESUPPLY => {
                *self.dock_action = DockAction::Resupplying;
                if self.ship.tick_resupply(self.stats) {
                    *self.dock_action = DockAction::Idle;
                    Status::Success
                } else {
                    Status::Running
                }
            }
            ACT_CAREEN => {
                *self.dock_action = DockAction::Careening;
                if self.ship.tick_careen() {
                    *self.dock_action = DockAction::Idle;
                    Status::Success
                } else {
                    Status::Running
                }
            }
            ACT_UNDOCK => {
                if self.nav.destination.is_some() {
                    self.ship.undock();
                    *self.dock_action = DockAction::Idle;
                    Status::Success
                } else {
                    *self.dock_action = DockAction::Idle;
                    Status::Failure
                }
            }
            ACT_CHOOSE_DESTINATION => {
                if self.ports.is_empty() {
                    return Status::Failure;
                }
                let mut idx = (xorshift64(self.rng_state) as usize) % self.ports.len();
                if self.ship.position.distance(self.ports[idx].position) < 20.0 {
                    idx = (idx + 1) % self.ports.len();
                }
                self.assign_destination_port(idx);
                Status::Success
            }
            ACT_DIVERT_TO_PORT => {
                if let Some((nearest_idx, _)) = self.ports.iter().enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let da = self.ship.position.distance(a.position);
                        let db = self.ship.position.distance(b.position);
                        da.partial_cmp(&db).unwrap()
                    })
                {
                    self.assign_destination_port(nearest_idx);
                }
                Status::Success
            }
            _ => Status::Failure,
        }
    }

    fn check_condition(&mut self, id: usize) -> Status {
        let result = match id {
            COND_IS_DOCKED => self.ship.state == ShipState::Docked,
            COND_HAS_DESTINATION => self.nav.destination.is_some(),
            COND_IS_LOW_PROVISIONS => {
                let days = self.ship.provisions_days_remaining(self.stats);
                days < LOW_PROVISIONS_DAYS && self.ship.state == ShipState::Sailing
            }
            _ => false,
        };
        if result { Status::Success } else { Status::Failure }
    }
}

