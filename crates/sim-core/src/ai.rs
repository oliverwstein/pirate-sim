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
use crate::nav::NavState;
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

/// Rates for port actions.
const RESUPPLY_RATE: f32 = 0.5;    // tons per hour at a port
const CAREEN_RATE_PORT: f32 = 3.0; // fouling points removed per hour

/// Below this many days of provisions, divert to nearest port.
const LOW_PROVISIONS_DAYS: f32 = 10.0;

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
    pub fn tick(&mut self, ship: &mut Ship, stats: &ShipStats, wind: &WindVector, ports: &[Port]) {
        let mut ctx = ShipBtContext {
            ship,
            stats,
            wind,
            nav: &mut self.nav,
            dock_action: &mut self.dock_action,
            ports,
            rng_state: &mut self.rng_state,
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
    rng_state: &'a mut u64,
}

impl<'a> BtContext for ShipBtContext<'a> {
    fn execute_action(&mut self, id: usize) -> Status {
        match id {
            ACT_SAIL => {
                if let Some(heading) = self.nav.compute_heading(self.ship.position, self.stats, self.wind) {
                    self.ship.set_heading(heading);
                    Status::Running
                } else {
                    // Arrived — dock
                    self.ship.dock();
                    *self.dock_action = DockAction::Idle;
                    Status::Success
                }
            }
            ACT_RESUPPLY => {
                *self.dock_action = DockAction::Resupplying;
                self.ship.provisions = (self.ship.provisions + RESUPPLY_RATE).min(self.stats.provision_capacity);
                if self.ship.provisions >= self.stats.provision_capacity {
                    *self.dock_action = DockAction::Idle;
                    Status::Success
                } else {
                    Status::Running
                }
            }
            ACT_CAREEN => {
                *self.dock_action = DockAction::Careening;
                self.ship.hull_fouling = (self.ship.hull_fouling - CAREEN_RATE_PORT).max(0.0);
                if self.ship.hull_fouling <= 0.0 {
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
                // Pick a random port that isn't too close to current position
                let idx = (xorshift64(self.rng_state) as usize) % self.ports.len();
                let port = &self.ports[idx];
                // Skip if we're already very close to this port
                if self.ship.position.distance(port.position) < 20.0 {
                    let alt_idx = (idx + 1) % self.ports.len();
                    self.nav.destination = Some(self.ports[alt_idx].position);
                } else {
                    self.nav.destination = Some(port.position);
                }
                Status::Success
            }
            ACT_DIVERT_TO_PORT => {
                // Set destination to nearest port, then sail there
                if let Some(nearest) = self.ports.iter()
                    .min_by(|a, b| {
                        let da = self.ship.position.distance(a.position);
                        let db = self.ship.position.distance(b.position);
                        da.partial_cmp(&db).unwrap()
                    })
                {
                    self.nav.destination = Some(nearest.position);
                }
                // Now sail (will be picked up next tick by HasDestination → Sail)
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

