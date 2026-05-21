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
use crate::goods::GoodsRegistry;
use crate::harbor::HarborMap;
use crate::market::PortMarket;
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
const ACT_SELL_ALL: usize = 6;
const ACT_BUY_BEST: usize = 7;

// --- Condition IDs ---
const COND_IS_DOCKED: usize = 0;
const COND_HAS_DESTINATION: usize = 1;
const COND_IS_LOW_PROVISIONS: usize = 2;

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

/// What the ship is doing while docked (for display purposes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DockAction {
    Idle,
    Resupplying,
    Careening,
}

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
        // Priority 1: If docked, do port activities
        Behavior::Sequence(vec![
            Behavior::Condition(COND_IS_DOCKED),
            dock_tree,
        ]),
        // Priority 2: If low on provisions, divert to nearest port and
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
        // Priority 3: If has destination, sail
        Behavior::Sequence(vec![
            Behavior::Condition(COND_HAS_DESTINATION),
            Behavior::Action(ACT_SAIL),
        ]),
        // Priority 4: Choose a new destination (random fallback)
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
    ///
    /// `markets` and `goods`, when both `Some`, route resupply through the
    /// docked port's market (consuming silver and stockpile). When either
    /// is `None`, resupply is free — used by toy/demo tests that don't
    /// model an economy.
    pub fn tick(
        &mut self,
        ship: &mut Ship,
        stats: &ShipStats,
        wind: &WindVector,
        ports: &[Port],
        harbors: &HarborMap,
        pathfind: Option<&PathfindContext<'_>>,
        markets: Option<&mut [PortMarket]>,
        goods: Option<&GoodsRegistry>,
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
            markets,
            goods,
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
    markets: Option<&'a mut [PortMarket]>,
    goods: Option<&'a GoodsRegistry>,
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
                    let port_idx = self.nav.dest_port;
                    self.ship.dock();
                    *self.dock_action = DockAction::Idle;
                    self.nav.docked_at_port = port_idx;
                    self.nav.destination = None;
                    self.nav.dest_port = None;
                    self.nav.clear_path();
                    // Home-port settlement: if this is the owner port,
                    // the supercargo books proceeds with the owners.
                    // Silver above the operating float is paid into
                    // the port treasury (dividend to shareholders);
                    // the ship keeps just enough to cover provisions
                    // and incidentals at the next port of call.
                    if let (Some(idx), Some(owner)) = (port_idx, self.ship.owner_port) {
                        if idx == owner {
                            if let Some(markets) = self.markets.as_deref_mut() {
                                if idx < markets.len() {
                                    let paid = markets[idx].deposit_owner_profit(
                                        self.ship,
                                        HOME_PORT_FLOAT_SILVER,
                                    );
                                    self.ship.lifetime_dividends += paid;
                                }
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
                } else if self.nav.dest_port.is_some()
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
                    if let Some(idx) = self.nav.dest_port {
                        self.replan_to_port(idx);
                    }
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
                let done = match (self.nav.docked_at_port, self.markets.as_deref_mut(), self.goods) {
                    (Some(idx), Some(markets), Some(goods)) if idx < markets.len() => {
                        self.ship.tick_resupply_at_market(self.stats, &mut markets[idx], goods)
                    }
                    // No market wired (test scenario, or unknown port) —
                    // fall back to free resupply so legacy tests pass.
                    _ => self.ship.tick_resupply(self.stats),
                };
                if done {
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
                    self.nav.docked_at_port = None;
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
            ACT_SELL_ALL => {
                // Sell every ton of cargo we arrived with at the
                // docked port's market. If markets/goods aren't wired
                // (toy tests), this is a no-op success.
                let (Some(idx), Some(markets), Some(goods)) =
                    (self.nav.docked_at_port, self.markets.as_deref_mut(), self.goods)
                else {
                    return Status::Success;
                };
                if idx >= markets.len() {
                    return Status::Success;
                }
                let market = &mut markets[idx];
                let entries: Vec<(crate::goods::GoodId, f32)> =
                    self.ship.cargo.iter().collect();
                for (gid, tons) in entries {
                    if tons > 0.0 {
                        // Best-effort: ignore "port out of silver" by
                        // selling whatever the port can afford. For
                        // v1 we just attempt the full amount; if the
                        // port can't pay, the cargo stays aboard and
                        // we'll try again on the next leg.
                        let _ = market.sell(self.ship, gid, tons, goods);
                    }
                }
                Status::Success
            }
            ACT_BUY_BEST => {
                // Pick the best (good, dest) and load up. Sets the
                // ship's destination as a side effect so ACT_UNDOCK
                // has somewhere to go.
                let (Some(idx), Some(markets), Some(goods)) =
                    (self.nav.docked_at_port, self.markets.as_deref_mut(), self.goods)
                else {
                    return Status::Success;
                };
                if idx >= markets.len() {
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
                    markets,
                    goods,
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
                let market = &mut markets[idx];
                let unit = market.buy_price(plan.good, goods).max(0.0001);
                let cargo_room = self.stats.cargo_capacity_tons
                    - self.ship.cargo.total_tons();

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
                        market.draw_for_outfit(
                            self.ship,
                            target,
                            OUTFIT_PORT_FRACTION_CAP,
                        );
                    }
                }

                let affordable = self.ship.silver / unit;
                let in_stock = market.stockpile.get(plan.good);
                let tons = cargo_room.min(affordable).min(in_stock).max(0.0);
                if tons > 0.0 {
                    let _ = market.buy(self.ship, self.stats, plan.good, tons, goods);
                }

                // Always set the destination — even when the buy fell
                // through (broke / no room) — so the ship still sails
                // to the chosen port and tries selling/buying again.
                self.assign_destination_port(plan.dest_port);
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
                    if let Some(dest) = self.nav.destination {
                        let dist = self.ship.position.distance(dest);
                        let voyage_days = self.stats.estimated_voyage_days(dist);
                        days_remaining < voyage_days + REACHABILITY_BUFFER_DAYS
                    } else {
                        // No destination → use the hard panic floor.
                        days_remaining < HARD_PANIC_DAYS
                    }
                }
            }
            _ => false,
        };
        if result { Status::Success } else { Status::Failure }
    }
}

