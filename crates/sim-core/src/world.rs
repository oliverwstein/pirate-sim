use std::path::Path;

use slotmap::{SecondaryMap, SlotMap};

use crate::ai::{ShipAI, ShipSnapshot};
use crate::coastline::{CoastlineMap, LandMesh};
use crate::goods::GoodsRegistry;
use crate::harbor::HarborMap;
use crate::map::MapSystem;
use crate::market::{archetype_for, PortMarket};
use crate::navmesh::Navmesh;
use crate::pathfind::PathfindContext;
use crate::pop::{self, PortDemographics};
use crate::port::{all_ports, Port};
use crate::ship::{Ship, ShipPolicy, ShipState, ShipStats};
use crate::shiptype::{self, ShipTypeRegistry};
use crate::shipyard::{self, BuildOutcome};
use crate::spatial::SpatialHash;
use crate::types::{ShipId, SimDate};
use crate::weather::WeatherSystem;

pub struct World {
    pub map: MapSystem,
    pub weather: WeatherSystem,
    pub ports: Vec<Port>,
    pub harbors: HarborMap,
    pub navmesh: Navmesh,
    pub coastline: CoastlineMap,
    pub land_mesh: LandMesh,
    pub goods: GoodsRegistry,
    /// Catalog of ship designs. A `Ship` indexes in via its
    /// `ship_type` field to fetch per-tick stats and (for shipyard
    /// ports) build costs.
    pub ship_types: ShipTypeRegistry,
    /// Per-port economic state, parallel to `ports` (index = port index).
    pub markets: Vec<PortMarket>,
    /// Per-port sailor population, parallel to `ports`. Evolves on the
    /// monthly tick: organic growth + maturation + mortality.
    /// See `planning/crewing-plan.md`.
    pub demographics: Vec<PortDemographics>,
    /// All live ships, keyed by generational `ShipId`. Sunken ships are
    /// removed from the map; their ids become permanently invalid,
    /// preventing aliasing.
    pub ships: SlotMap<ShipId, Ship>,
    /// AI controller for each ship, keyed by the same `ShipId`.
    pub ship_ais: SecondaryMap<ShipId, ShipAI>,
    pub date: SimDate,
    /// The month for which `markets` last received their monthly tick.
    /// Used to fire production exactly once per month transition.
    last_market_month: u8,
    /// The day-of-year for which the hiring loop last ran. Used to
    /// fire the daily Hiring tick exactly once per day transition.
    last_hire_day: u16,
    /// Per-ship silver at the start of the current month. Keyed by
    /// `ShipId`. Used at the next month transition to compute monthly
    /// profit (silver delta), which feeds the shipyard "math pencils"
    /// decision. A freshly-spawned ship's entry is initialized to its
    /// starting silver so its first-month delta is meaningful.
    silver_at_month_start: SecondaryMap<ShipId, f32>,
    /// Last completed month's average per-ship silver delta (pesos).
    /// Used by `shipyard::try_build` as the expected per-ship monthly
    /// profit for new vessels. Starts at 0 (no fleet history); first
    /// month's tick updates it.
    pub last_month_avg_profit: f32,
    /// Diagnostic counter: total number of ships built by the
    /// shipyard system since `World::load`.
    pub ships_built: u32,
    /// Diagnostic counter: cumulative mutiny flips since `World::load`
    /// (Step 9). Incremented in the per-ship tick when `try_mutiny`
    /// returns true.
    pub mutinies_total: u32,
    /// Step 10.b: cumulative non-combat losses since `World::load`,
    /// split by cause. Storm/foundering/fire totals only count
    /// **sinkings**; damage-only events live in
    /// `weather.hazards.counters`. Read by the bench attrition table.
    pub attrition_storms: u32,
    pub attrition_foundered: u32,
    pub attrition_fires: u32,
    /// Step 11.a: prize-outcome ledger for successful boardings.
    /// `prizes_taken` is the rare case where the prize joins the
    /// pirate fleet (real upgrade); other outcomes strip cargo + silver
    /// and either sink the hull or release it.
    pub prizes_taken: u32,
    pub prizes_sold: u32,
    pub prizes_sunk: u32,
    pub prizes_released: u32,
    /// Step 11.a: deterministic RNG for stochastic combat outcomes
    /// (prize handling, future morale rolls, etc.). Seeded once at
    /// `World::load`; same world state → same outcome trace.
    pub combat_rng_state: u64,
    /// Dynamic spatial index over Sailing ships, rebuilt at the top
    /// of every hourly tick. Read by viz (Step 4.d) and, in Step 6+,
    /// by AI `SeePrey` / `Pursue` / `Flee` conditions. Docked /
    /// Hiring / Anchored ships are intentionally not indexed —
    /// inter-ship interaction at sea is the only consumer.
    pub spatial: SpatialHash,
    /// Per-tick command buffer (Step 5.c). Filled during the AI Phase
    /// by `ShipBtContext` (via `ShipTickInputs::commands`), drained by
    /// the Resolution Phase before physics. Lives on `World` so the
    /// allocation is reused across ticks.
    pub commands: Vec<(ShipId, crate::command::ShipCommand)>,
    /// Phase 4 §3a: monotonic minute counter advanced by `MINUTES_PER_HOUR`
    /// each hourly tick. Drives the sub-tick combat reload clock so that
    /// `Ship::next_fire_at_minute` (and the upcoming `Fort` equivalent)
    /// can be compared against an absolute wall-clock value rather than
    /// a per-hour reset. Wraps at u64::MAX, which is ~35 trillion sim
    /// years — i.e., never.
    pub sim_minute: u64,
}

/// Phase 4 §3a: minutes per hourly tick. Sub-tick combat (§3b) divides
/// this into 12 five-minute steps; reload formulas in `combat.rs` are
/// expressed in real minutes against `sim_minute`.
pub const MINUTES_PER_HOUR: u64 = 60;

impl World {
    pub fn load(data_dir: &Path) -> Self {
        let map = MapSystem::load(data_dir);
        let weather = WeatherSystem::load(data_dir);
        let ship_types = ShipTypeRegistry::starter();
        let ports = all_ports(&ship_types);
        let harbors = HarborMap::build(&map.land, &ports);
        let navmesh = Navmesh::build(&map.land);
        let coastline =
            CoastlineMap::load(&data_dir.join("grids/coastline.bin")).unwrap_or_default();
        let land_mesh = LandMesh::load(&data_dir.join("grids/land_polys.bin")).unwrap_or_default();
        let goods = GoodsRegistry::starter();
        let markets: Vec<PortMarket> = ports
            .iter()
            .map(|p| {
                let archetype = archetype_for(&p.name);
                PortMarket::with_recipe(&goods, archetype.recipe())
            })
            .collect();
        let demographics: Vec<PortDemographics> = ports
            .iter()
            .map(|p| PortDemographics::seed(p.category, p.faction))
            .collect();

        let date = SimDate::new(1680, 0, 1);
        let last_market_month = date.month();
        let last_hire_day = date.day_of_year;

        Self {
            map,
            weather,
            ports,
            harbors,
            navmesh,
            coastline,
            land_mesh,
            goods,
            ship_types,
            markets,
            demographics,
            ships: SlotMap::with_key(),
            ship_ais: SecondaryMap::new(),
            date,
            last_market_month,
            last_hire_day,
            silver_at_month_start: SecondaryMap::new(),
            last_month_avg_profit: 0.0,
            ships_built: 0,
            mutinies_total: 0,
            attrition_storms: 0,
            attrition_foundered: 0,
            attrition_fires: 0,
            prizes_taken: 0,
            prizes_sold: 0,
            prizes_sunk: 0,
            prizes_released: 0,
            combat_rng_state: 0x5052_495A_4520_5247_u64 ^ 0x9E37_79B9_7F4A_7C15,
            spatial: SpatialHash::new(),
            commands: Vec::new(),
            sim_minute: 0,
        }
    }

    /// Add a ship with its AI controller. Returns the freshly-minted
    /// `ShipId` so callers can hold a stable handle.
    pub fn add_ship(&mut self, ship: Ship, ai: ShipAI) -> ShipId {
        let starting = ship.silver;
        let id = self.ships.insert(ship);
        self.ship_ais.insert(id, ai);
        self.silver_at_month_start.insert(id, starting);
        id
    }

    /// Step 6: spawn a pirate sloop at the named port (case-sensitive
    /// match against `Port.name`). Returns `Some(id)` on success or
    /// `None` if the port doesn't exist. The ship starts `Docked` at
    /// the port (matches the seed-fleet shape used by `bench_trade`
    /// and viz `spawn_demo_ships`) so the BT's docked branch runs
    /// once and undocks it on the first tick out of port. The
    /// pirate's `policy` is set to `Pirate` and its `faction` to
    /// `Free` regardless of the host port's flag (a haven hosts
    /// pirates, but the ships fly their own colors).
    pub fn spawn_pirate_sloop_at(&mut self, port_name: &str, seed: u64) -> Option<ShipId> {
        let idx = self.ports.iter().position(|p| p.name == port_name)?;
        let port_pos = self.ports[idx].position;
        let mut ship = Ship::seeded_at_port(port_pos, idx, crate::port::Faction::Free);
        ship.policy = ShipPolicy::Pirate;
        ship.nav.docked_at_port = Some(idx);
        // Step 7: seed pirates with magazine + shot locker so they can
        // actually fire when they catch prey. 4 t each is enough for
        // ~50 broadsides from an 8-gun sloop — plenty for Step 7's
        // bench window, and a clean signal that combat is wired.
        ship.cargo.add(crate::goods::ids::GUNPOWDER, 4.0);
        ship.cargo.add(crate::goods::ids::CANNON_SHOT, 4.0);
        let ai = ShipAI::with_seed(seed);
        Some(self.add_ship(ship, ai))
    }

    /// Step 10: seed a historically-scaled starter fleet across every
    /// port in the world. Per `planning/research/atlantic-fleet-numbers-1650-1720`
    /// the Caribbean basin held ~400–800 active hulls c. 1680; with
    /// 38 ports this method targets ~480 ships. Counts and type mixes
    /// scale by `PortCategory`:
    ///
    /// | Category          | Ships per port | Type mix                                              |
    /// |-------------------|----------------|-------------------------------------------------------|
    /// | EuropeanHub       | 30             | 30% ship, 40% fluyt, 20% brigantine, 10% bark        |
    /// | CaribbeanEntrepot | 25             | 15% ship, 25% fluyt, 30% brigantine, 30% sloop       |
    /// | SmallColonial     | 8              | 50% sloop, 30% brigantine, 20% bark                  |
    /// | PirateHaven       | 6              | 100% sloop (Pirate policy, extra powder)             |
    ///
    /// All ships start `Docked` at their home port with full crew,
    /// full provisions, and a defensive powder+shot loadout. RNG is
    /// deterministic in `base_seed + port_idx` so the same call
    /// produces the same fleet across runs. Returns the spawned
    /// ShipIds in spawn order.
    pub fn seed_historical_fleet(&mut self, base_seed: u64) -> Vec<ShipId> {
        use crate::pop::PortCategory;
        use crate::shiptype::ids as st;
        let mut ids = Vec::new();
        let n_ports = self.ports.len();
        for port_idx in 0..n_ports {
            let category = self.ports[port_idx].category;
            let faction = self.ports[port_idx].faction;
            let port_pos = self.ports[port_idx].position;
            let port_seed = base_seed
                .wrapping_add(port_idx as u64)
                .wrapping_mul(2654435761);

            let (count, mix): (usize, &[(crate::shiptype::ShipTypeId, u32)]) = match category {
                PortCategory::EuropeanHub => (
                    30,
                    &[
                        (st::SHIP, 30),
                        (st::FLUYT, 40),
                        (st::BRIGANTINE, 20),
                        (st::BARK, 10),
                    ],
                ),
                PortCategory::CaribbeanEntrepot => (
                    25,
                    &[
                        (st::SHIP, 15),
                        (st::FLUYT, 25),
                        (st::BRIGANTINE, 30),
                        (st::SLOOP, 30),
                    ],
                ),
                PortCategory::SmallColonial => {
                    (8, &[(st::SLOOP, 50), (st::BRIGANTINE, 30), (st::BARK, 20)])
                }
                PortCategory::PirateHaven => (6, &[(st::SLOOP, 100)]),
            };
            let weight_total: u32 = mix.iter().map(|(_, w)| *w).sum();

            for k in 0..count {
                let mut s = port_seed.wrapping_add((k as u64).wrapping_mul(1442695040888963407));
                // Pick a type from the weighted mix.
                let pick = (s % weight_total as u64) as u32;
                s ^= s >> 17;
                let mut acc = 0u32;
                let mut chosen = mix[0].0;
                for (ty, w) in mix {
                    acc += *w;
                    if pick < acc {
                        chosen = *ty;
                        break;
                    }
                }
                let stats = self.ship_types.get(chosen).stats.clone();
                // Starting silver: roughly enough to buy a hold's worth
                // of cheap cargo. The shipyard sizing uses ~30 pesos/ton
                // of capacity; we use a slightly leaner factor here so
                // seeded fleets don't drown the simulation in cash.
                let starting_silver = (stats.cargo_capacity_tons * 25.0).max(1500.0);
                let mut ship = Ship::seeded_at_port_typed(
                    port_pos,
                    port_idx,
                    faction,
                    chosen,
                    &stats,
                    starting_silver,
                );
                ship.nav.docked_at_port = Some(port_idx);
                // Defensive armament — even ordinary merchants carried
                // a few guns. Pirates get a heavier magazine.
                let (powder, shot) = if category == PortCategory::PirateHaven {
                    ship.policy = ShipPolicy::Pirate;
                    // Pirate sloops fly their own colors irrespective
                    // of the host haven's nominal flag.
                    ship.faction = crate::port::Faction::Free;
                    (4.0, 4.0)
                } else {
                    (1.0, 1.0)
                };
                ship.cargo.add(crate::goods::ids::GUNPOWDER, powder);
                ship.cargo.add(crate::goods::ids::CANNON_SHOT, shot);
                let ai_seed = s.wrapping_add(0xdeadbeef);
                let ai = ShipAI::with_seed(ai_seed);
                ids.push(self.add_ship(ship, ai));
            }
        }
        ids
    }

    /// Advance the simulation by one hour.
    /// Advance the simulation by one hour. Dispatches to per-cadence
    /// helpers; see `tick_monthly`, `tick_daily_hiring`, and
    /// `tick_hourly_ai_and_physics`.
    pub fn tick(&mut self) {
        let month = self.date.month();
        // PathfindContext uses a single "representative" stats — the
        // sloop's profile — because the navmesh is shared and the
        // wind-routed cost is the same shape for every merchant rig
        // we currently model. A future refinement could maintain a
        // per-type PathfindContext (or a per-type ship_stats lookup
        // inside the planner) without changing the navmesh.
        let pathfind_stats = self.ship_types.get(shiptype::ids::SLOOP).stats.clone();

        self.tick_monthly(month);
        self.tick_daily_hiring();
        self.tick_hourly_ai_and_physics(month, &pathfind_stats);

        self.date.advance_hours(1);
        // Phase 4 §3a: keep the absolute minute clock in lock-step with
        // the calendar tick. Sub-tick combat (§3b) will read this to
        // schedule reloads at sub-minute precision.
        self.sim_minute = self.sim_minute.saturating_add(MINUTES_PER_HOUR);
    }

    /// Monthly economic tick: market production/consumption, sailor
    /// pool dynamics, fleet profit snapshot, shipyard build decisions,
    /// and per-ship silver snapshot reset. Fires exactly once per
    /// month transition (gated on `self.last_market_month`).
    fn tick_monthly(&mut self, month: u8) {
        if month == self.last_market_month {
            return;
        }
        for market in &mut self.markets {
            market.tick_month();
        }
        // Monthly sailor-pool tick: growth, maturation, mortality.
        // Parallel index with `ports` and `markets`.
        for (i, d) in self.demographics.iter_mut().enumerate() {
            pop::tick_monthly(d, self.ports[i].faction);
        }

        // Average per-ship silver delta over the just-completed month.
        // The new-ship delta is implicitly excluded for ships added
        // mid-month: their snapshot was their starting silver, so
        // their first-month delta represents however much (or little)
        // they actually traded.
        if !self.ships.is_empty() {
            let total_delta: f32 = self
                .ships
                .iter()
                .filter_map(|(id, s)| {
                    self.silver_at_month_start
                        .get(id)
                        .map(|prev| s.silver - prev)
                })
                .sum();
            self.last_month_avg_profit = total_delta / self.ships.len() as f32;
        } else {
            self.last_month_avg_profit = 0.0;
        }

        // Shipyards decide whether to build. Collect new ships first
        // (each build mutates its own market; we can't iterate over
        // self.ports and call methods that borrow self.markets
        // mutably in a single pass).
        let mut newly_built: Vec<(Ship, ShipAI)> = Vec::new();
        for (idx, port) in self.ports.iter().enumerate() {
            if port.shipyard.is_none() {
                continue;
            }
            let market = &mut self.markets[idx];
            let (outcome, ship) = shipyard::try_build(
                port,
                idx,
                market,
                &self.goods,
                &self.ship_types,
                self.last_month_avg_profit,
            );
            if let (BuildOutcome::Built { .. }, Some(mut ship)) = (outcome, ship) {
                // New ship docks at home port immediately; the AI's
                // BUY_BEST tree will pick its first destination on
                // the first dock-cycle tick. We seed
                // `ship.nav.docked_at_port = idx` so the dock tree
                // knows which market to trade with.
                let ai =
                    ShipAI::with_seed(0xA15E_C0FF_u64 ^ (idx as u64) ^ (self.ships_built as u64));
                ship.nav.docked_at_port = Some(idx);
                newly_built.push((ship, ai));
            }
        }
        for (ship, ai) in newly_built {
            self.ships_built += 1;
            self.add_ship(ship, ai);
        }

        // Reset snapshots for the new month — *after* the new ships
        // were appended, so their starting silver is what we'll
        // compare against next month.
        self.silver_at_month_start.clear();
        for (id, ship) in &self.ships {
            self.silver_at_month_start.insert(id, ship.silver);
        }

        self.last_market_month = month;
    }

    /// Daily hiring tick. Both `Hiring` (newly-built / refitting) and
    /// `Docked` ships at port can draw sailors from the local
    /// `PortDemographics` (seasoned-first), up to `crew_typical`.
    /// `Hiring` ships use `owner_port`; `Docked` ships use their
    /// current `docked_at_port` — sailors are not faction-loyal, and
    /// any port that has a crew available will sell their time. A
    /// `Hiring` hull transitions to `Docked` once it reaches `crew_min`
    /// (it can put to sea undermanned), but daily top-ups continue
    /// from then on until the design complement is reached. This
    /// matches user direction: "hiring sailors, especially unseasoned
    /// sailors in Europe or decently prosperous Caribbean ports,
    /// should basically always be possible."
    fn tick_daily_hiring(&mut self) {
        let today = self.date.day_of_year;
        if today == self.last_hire_day {
            return;
        }
        // Step 10.b: age every live ship by one day. Sits on the same
        // day-of-year gate as hiring so both fire exactly once per
        // calendar day. `HazardSystem::tick_age` is a no-op on Sunk
        // ships.
        for (_, ship) in self.ships.iter_mut() {
            crate::weather::hazards::HazardSystem::tick_age(ship);
        }
        const HIRE_PER_DAY: u16 = 5;
        let ids: Vec<ShipId> = self.ships.keys().collect();
        for id in ids {
            // Resolve the port we're hiring at: owner_port while Hiring,
            // docked_at_port (from AI nav) while Docked. Anything else
            // (Sailing/Anchored) skips this tick.
            let (port_idx, want, ship_type, ship_silver, is_hiring) = match self.ships.get(id) {
                Some(s) if s.state == ShipState::Hiring => {
                    let port = match s.owner_port {
                        Some(p) => p,
                        None => continue,
                    };
                    let stats = self.ship_types.get(s.ship_type).stats.clone();
                    let typical = stats.crew_typical();
                    if s.crew_alive >= typical {
                        continue;
                    }
                    (port, typical - s.crew_alive, s.ship_type, s.silver, true)
                }
                Some(s) if s.state == ShipState::Docked => {
                    let stats = self.ship_types.get(s.ship_type).stats.clone();
                    let typical = stats.crew_typical();
                    if s.crew_alive >= typical {
                        continue;
                    }
                    let port = match s.nav.docked_at_port {
                        Some(p) => p,
                        None => continue,
                    };
                    (port, typical - s.crew_alive, s.ship_type, s.silver, false)
                }
                _ => continue,
            };
            let stats = &self.ship_types.get(ship_type).stats;
            let morale = self.ships.get(id).map(|s| s.morale).unwrap_or(1.0);
            let rate_mult = if (0.4..0.7).contains(&morale) {
                0.9
            } else {
                1.0
            };
            let per_day_cap = ((HIRE_PER_DAY as f32) * rate_mult).floor() as u16;
            let cap = want.min(per_day_cap.max(1));
            let affordable = (ship_silver / crate::ship::SIGN_ON_BOUNTY_PESOS)
                .floor()
                .max(0.0) as u16;
            let cap = cap.min(affordable);
            let demo = match self.demographics.get_mut(port_idx) {
                Some(d) => d,
                None => continue,
            };
            let from_seasoned = (demo.seasoned as u16).min(cap);
            let remaining = cap - from_seasoned;
            let from_unseasoned = (demo.unseasoned as u16).min(remaining);
            let drawn = from_seasoned + from_unseasoned;
            demo.seasoned -= from_seasoned as u32;
            demo.unseasoned -= from_unseasoned as u32;
            let bounty = drawn as f32 * crate::ship::SIGN_ON_BOUNTY_PESOS;
            if let Some(s) = self.ships.get_mut(id) {
                s.crew_alive += drawn;
                // Track the seasoned slice of this hire so the ship's
                // `crew_seasoned` reflects the port's seasoned-first
                // draw policy. Invariant: `crew_seasoned <= crew_alive`
                // (holds because `from_seasoned <= drawn`).
                s.crew_seasoned = s.crew_seasoned.saturating_add(from_seasoned);
                s.silver -= bounty;
                // Hiring → Docked transition once we cross crew_min:
                // the ship is now seaworthy, but further top-ups will
                // continue while it stays at port.
                if is_hiring && s.crew_alive >= stats.crew_min() {
                    s.state = ShipState::Docked;
                }
            }
            if let Some(market) = self.markets.get_mut(port_idx) {
                market.silver += bounty;
            }
        }
        self.last_hire_day = today;
    }

    /// Hourly per-ship AI + physics tick: each ship gets an AI
    /// decision, consumes resources, and (if sailing) advances its
    /// position with land-collision sweep.
    fn tick_hourly_ai_and_physics(&mut self, month: u8, pathfind_stats: &ShipStats) {
        let pathfind = PathfindContext::new(
            &self.map.land,
            &self.weather.wind,
            pathfind_stats,
            month,
            &self.navmesh,
        );

        // Rebuild the spatial index over Sailing ships before any AI
        // decisions are made this tick. Cheap (single pass, BTreeMap
        // insertion); rebuilt-each-tick keeps the API stable as we
        // move toward the Step-5 read/mutate phase split. Docked,
        // Hiring, and Anchored ships are intentionally excluded —
        // they are not candidates for at-sea interaction.
        self.spatial.clear();
        let mut snapshots: SecondaryMap<ShipId, ShipSnapshot> = SecondaryMap::new();
        for (id, ship) in &self.ships {
            if ship.state == ShipState::Sailing {
                self.spatial.insert(id, ship.position);
                // Step 6: parallel snapshot map so AI code can look up
                // any other Sailing ship's identifying fields without
                // taking a second borrow on `self.ships` (which is
                // mutably borrowed for the active ship). Stats come
                // from the type registry, copied by value into the
                // snapshot — cheap (5 scalars per ship per tick).
                let stats = &self.ship_types.get(ship.ship_type).stats;
                snapshots.insert(
                    id,
                    ShipSnapshot {
                        position: ship.position,
                        policy: ship.policy,
                        faction: ship.faction,
                        max_speed: stats.speed_max,
                        cargo_capacity_tons: stats.cargo_capacity_tons,
                        velocity: ship.velocity(),
                        rigging_frac: if stats.rigging_integrity_max > 0.0 {
                            (ship.rigging_integrity / stats.rigging_integrity_max).clamp(0.0, 1.0)
                        } else {
                            1.0
                        },
                    },
                );
            }
        }

        // Post-Phase-3 cleanup (postmortem §1/§3.1): the AI Phase, the
        // Resolution Phase, and the Mutation/Physics Phase are now three
        // sequential passes over the ship set, not one fused loop. This
        // makes per-tick outcomes independent of SlotMap iteration order
        // (inside a phase, the only intra-phase dependency is Resolution
        // — and that one is deterministic in `commands` push order, which
        // mirrors the AI Phase's `ids` order). It also unblocks future
        // Rayon parallelization of the AI Phase: every ship's AI tick
        // observes the same pre-tick world snapshot.
        //
        // The command buffer is the seam between AI (write-only) and
        // Resolution (read+mutate). Cleared once per tick.
        self.commands.clear();

        // Snapshot the live ship ids so we can iterate while mutating
        // both `ships` and `ship_ais`. SlotMap iteration order is not
        // documented as stable; collecting upfront also pins per-tick
        // ordering for determinism across all three phases below.
        let ids: Vec<ShipId> = self.ships.keys().collect();

        // ─── AI Phase (read-only over other ships) ───────────────────
        // Each ship's AI ticks against the pre-tick world snapshot
        // (`snapshots` + `spatial`) and pushes its intents into
        // `self.commands`. No ship may mutate another ship here; ship
        // self-mutation is allowed (cargo bookkeeping at dock, etc.).
        for id in ids.iter().copied() {
            let ship_stats: ShipStats = match self.ships.get(id) {
                Some(s) => self.ship_types.get(s.ship_type).stats.clone(),
                None => continue,
            };
            let wind = self.weather.wind.wind_at(self.ships[id].position, month);

            let ai = match self.ship_ais.get_mut(id) {
                Some(a) => a,
                None => continue,
            };
            let ship = match self.ships.get_mut(id) {
                Some(s) => s,
                None => continue,
            };
            let mut inputs = crate::ai::ShipTickInputs {
                me: id,
                ship,
                stats: &ship_stats,
                wind: &wind,
                ports: &self.ports,
                harbors: &self.harbors,
                pathfind: Some(&pathfind),
                markets: &mut self.markets,
                goods: &self.goods,
                commands: &mut self.commands,
                day_of_year: self.date.day_of_year,
                snapshots: &snapshots,
                spatial: &self.spatial,
            };
            ai.tick(&mut inputs);
        }

        // ─── Resolution Phase ────────────────────────────────────────
        // Drain the command buffer in push order (== AI-tick id order).
        // Steering writes apply inline. Combat commands (FireBroadside,
        // AttemptBoard) are collected into intent vectors and processed
        // in two phases:
        //   1. Sub-tick combat (Phase 4 §3b) — converts each
        //      FireBroadside intent into up to `SUB_TICKS_PER_HOUR`
        //      actual fires, gated by reload + range + ordnance.
        //   2. Boarding — runs after sub-tick so that rigging damage
        //      from this hour's exchange is visible to the
        //      `BOARDING_RIGGING_THRESHOLD` gate.
        //
        // Determinism: both intent vectors preserve drain order, which
        // is AI-tick (i.e., ShipId) order.
        let mut engagements: Vec<(ShipId, ShipId)> = Vec::new();
        let mut boardings: Vec<(ShipId, ShipId)> = Vec::new();
        for (attacker, cmd) in self.commands.drain(..) {
            match cmd {
                crate::command::ShipCommand::Steer { heading, speed } => {
                    // The "attacker" slot for `Steer` is the issuing
                    // ship — kept as the tuple's first element so the
                    // command shape is uniform across variants.
                    if let Some(target_ship) = self.ships.get_mut(attacker) {
                        target_ship.set_steering(heading, speed);
                    }
                }
                // Phase 4 §3b: a FireBroadside intent unlocks up to
                // SUB_TICKS_PER_HOUR shots over the hour, depending on
                // reload, range, and ordnance. Defer to the sub-tick
                // loop below.
                crate::command::ShipCommand::FireBroadside { target: tgt } => {
                    engagements.push((attacker, tgt));
                }
                crate::command::ShipCommand::AttemptBoard { target: tgt } => {
                    boardings.push((attacker, tgt));
                }
            }
        }

        // Phase 4 §3b: convert engagement intents into a multi-broadside
        // exchange resolved at 5-minute sub-tick precision.
        self.run_sub_tick_combat(&engagements);

        // Boarding actions are resolved after sub-tick so that any
        // rigging damage taken this hour is visible to the boarding
        // gate. See the original Step 8 implementation below.
        for (attacker, tgt) in boardings {
            self.resolve_boarding(attacker, tgt);
        }
        // ─── Mutation / Physics Phase ────────────────────────────────
        // Per-ship state updates that depend on the world *after*
        // Resolution: provisions burn, morale, mutiny rolls, weather
        // hazards, wage accrual/payout, and swept-movement physics.
        // Iterates `ids` in the same order as the AI Phase so the
        // per-tick RNG sequence (mutiny / hazard rolls share
        // `self.combat_rng_state` and `self.weather.hazards`) is
        // deterministic across runs. No cross-ship dependency inside
        // this loop — each ship's state evolves independently.
        for id in ids.iter().copied() {
            let ship_stats: ShipStats = match self.ships.get(id) {
                Some(s) => self.ship_types.get(s.ship_type).stats.clone(),
                None => continue,
            };
            let wind = self.weather.wind.wind_at(self.ships[id].position, month);

            let ship = match self.ships.get_mut(id) {
                Some(s) => s,
                None => continue,
            };

            // Resource consumption
            ship.tick_resources(&ship_stats);
            // Morale tick (after resources so days_left reflects this hour's burn).
            ship.tick_morale(&ship_stats);
            // Step 9: mutiny check. On flip, clear the merchant-route
            // NavGoal so the new pirate captain re-plans next tick.
            // Step 11.b: pass a uniform sample from the world combat
            // RNG; ship.try_mutiny now rolls stochastically instead of
            // firing deterministically the moment conditions are met.
            let mutiny_roll = combat_rng_step(&mut self.combat_rng_state);
            if ship.try_mutiny(mutiny_roll) {
                self.mutinies_total += 1;
                if let Some(ai) = self.ship_ais.get_mut(id) {
                    ai.goal.destination = None;
                    ai.goal.dest_port = None;
                    ai.goal.pursue_target = None;
                    ai.goal.flee_from = None;
                }
                ship.nav.waypoints.clear();
            }

            // Step 10.b: non-combat attrition. Teredo accumulates
            // hourly while the hull is wet; storms / foundering / fire
            // get one stochastic roll each. Sinking events flip
            // `ship.state = Sunk` so the Cleanup phase at end-of-tick
            // reaps the slot. Damage-only events reduce hull integrity
            // and may push a previously combat-damaged hull under.
            let pos = ship.position;
            crate::weather::hazards::HazardSystem::tick_environment(ship, pos);
            let events = self.weather.hazards.roll_hazards(ship, pos, month);
            for ev in events {
                use crate::weather::hazards::HazardEvent;
                match ev {
                    HazardEvent::StormDamage { hull_loss } => {
                        ship.hull_integrity = (ship.hull_integrity - hull_loss).max(0.0);
                        if ship.hull_integrity <= 0.0 && ship.state != ShipState::Sunk {
                            ship.state = ShipState::Sunk;
                            self.attrition_storms += 1;
                        }
                    }
                    HazardEvent::StormSunk => {
                        if ship.state != ShipState::Sunk {
                            ship.hull_integrity = 0.0;
                            ship.state = ShipState::Sunk;
                            self.attrition_storms += 1;
                        }
                    }
                    HazardEvent::Foundered => {
                        if ship.state != ShipState::Sunk {
                            ship.hull_integrity = 0.0;
                            ship.state = ShipState::Sunk;
                            self.attrition_foundered += 1;
                        }
                    }
                    HazardEvent::Fire { hull_loss, sunk } => {
                        ship.hull_integrity = (ship.hull_integrity - hull_loss).max(0.0);
                        if sunk && ship.state != ShipState::Sunk {
                            ship.state = ShipState::Sunk;
                            self.attrition_fires += 1;
                        }
                    }
                }
            }
            if ship.state == ShipState::Sunk {
                continue;
            }

            // Wages: accrue while at sea, pay out into the port's
            // market silver while docked. See crewing-plan §6 / §3.3.
            // Wage payout flows from ship.silver to the docked port's
            // PortMarket.silver (sailors immediately spend pay on grog
            // and supplies — closed-economy property preserved).
            match ship.state {
                ShipState::Sailing => {
                    let hourly = (ship.crew_alive as f32) * crate::ship::WAGE_PESOS_PER_MAN_MONTH
                        / (30.0 * 24.0);
                    ship.wages_owed_pesos += hourly;
                }
                ShipState::Docked => {
                    if ship.wages_owed_pesos > 0.0 {
                        if let Some(port_idx) = ship.nav.docked_at_port {
                            let pay = ship.wages_owed_pesos.min(ship.silver.max(0.0));
                            if pay > 0.0 {
                                ship.silver -= pay;
                                ship.wages_owed_pesos -= pay;
                                if let Some(market) = self.markets.get_mut(port_idx) {
                                    market.silver += pay;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }

            if ship.state != ShipState::Sailing {
                continue;
            }

            // Physics: compute movement, swept against land so a single
            // tick never tunnels through a coastline.
            //
            // Rescue: a ship may legitimately be inside a land cell — e.g.,
            // it just undocked from a port whose literal coordinates fall
            // on land at our 1 NM/cell resolution. From inside land,
            // `farthest_clear_point` would otherwise refuse all motion and
            // strand the ship. Snap to the nearest sea cell first.
            if self.map.land.is_land(ship.position) {
                if let Some(cell) = self.map.land.pos_to_cell(ship.position) {
                    if let Some(sea) = self.map.land.nearest_sea_cell(cell.0, cell.1, 32) {
                        ship.position = self.map.land.cell_to_pos(sea.0, sea.1);
                    }
                }
            }

            let new_pos = ship.compute_next_position(&ship_stats, &wind, 1.0);
            let old_pos = ship.position;
            let safe_pos = self.map.land.farthest_clear_point(old_pos, new_pos);

            if safe_pos.distance(old_pos) > 0.05 {
                ship.position = safe_pos;
                // Speed reflects how far we actually traveled.
                let traveled = safe_pos.distance(old_pos);
                ship.speed = traveled; // 1 hour tick → NM == kt
            } else {
                ship.speed = 0.0;
            }
        }

        // Step 8: Cleanup Phase. Reap any ships marked Sunk this tick
        // (by broadside hull breach or by burning a captured prize).
        // Removing from `ships` bumps the SlotMap generation, so the
        // ShipId becomes permanently invalid — no future tick can race
        // on a ghost. The `ship_ais` SecondaryMap is keyed by the same
        // ShipId; `slotmap` guarantees a stale key returns `None`, but
        // we explicitly remove to free the memory.
        let sunk: Vec<ShipId> = self
            .ships
            .iter()
            .filter_map(|(id, s)| {
                if s.state == ShipState::Sunk {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();
        for id in sunk {
            self.ships.remove(id);
            self.ship_ais.remove(id);
            self.silver_at_month_start.remove(id);
        }
    }

    /// Phase 4 §3b: sub-tick combat resolver. Each `(attacker, target)`
    /// engagement runs over `SUB_TICKS_PER_HOUR` 5-minute steps inside
    /// the hour about to elapse. At each step, the attacker fires if
    /// its `next_fire_at_minute` has reached the current sub-tick, the
    /// target is within `CANNON_RANGE_NM` at the interpolated positions
    /// for this sub-tick, and the magazine has the required powder +
    /// shot. Successful fires debit ordnance, apply deterministic
    /// damage, and advance the attacker's reload clock by
    /// `combat::reload_minutes`. Sunk targets are flagged immediately
    /// so subsequent sub-ticks short-circuit. Positions are linear-
    /// interpolated between hour-start (`ship.position`) and projected
    /// hour-end (`position + velocity * 1h`); this matches the linear
    /// assumption already used by `combat::min_distance_over_tick`.
    fn run_sub_tick_combat(&mut self, engagements: &[(ShipId, ShipId)]) {
        if engagements.is_empty() {
            return;
        }
        let hour_start = self.sim_minute;
        // Cache hour-start position + velocity per participant.
        // Avoids repeated borrowck dances inside the sub-tick loop.
        type StartState = ((f32, f32), (f32, f32));
        let mut start: SecondaryMap<ShipId, StartState> = SecondaryMap::new();
        for &(a, t) in engagements {
            for id in [a, t] {
                if start.contains_key(id) {
                    continue;
                }
                if let Some(s) = self.ships.get(id) {
                    start.insert(id, ((s.position.x, s.position.y), s.velocity()));
                }
            }
        }
        for step in 0..crate::combat::SUB_TICKS_PER_HOUR {
            let now = hour_start + step * crate::combat::MINUTES_PER_SUB_TICK;
            let dt_h = (step * crate::combat::MINUTES_PER_SUB_TICK) as f32 / 60.0;
            for &(attacker_id, target_id) in engagements {
                // Read attacker state and gate.
                let (cannons, seasoned_ratio, next_fire, a_start, a_vel) =
                    match (self.ships.get(attacker_id), start.get(attacker_id)) {
                        (Some(a), Some(&(p, v))) => {
                            if a.state == ShipState::Sunk {
                                continue;
                            }
                            (
                                self.ship_types.get(a.ship_type).stats.cannons,
                                a.seasoned_ratio(),
                                a.next_fire_at_minute,
                                p,
                                v,
                            )
                        }
                        _ => continue,
                    };
                if cannons == 0 || next_fire > now {
                    continue;
                }
                let (t_start, t_vel) = match (self.ships.get(target_id), start.get(target_id)) {
                    (Some(t), Some(&(p, v))) => {
                        if t.state == ShipState::Sunk {
                            continue;
                        }
                        (p, v)
                    }
                    _ => continue,
                };
                // Interpolated positions at this sub-tick.
                let a_pos = (a_start.0 + a_vel.0 * dt_h, a_start.1 + a_vel.1 * dt_h);
                let t_pos = (t_start.0 + t_vel.0 * dt_h, t_start.1 + t_vel.1 * dt_h);
                let dx = a_pos.0 - t_pos.0;
                let dy = a_pos.1 - t_pos.1;
                let range = (dx * dx + dy * dy).sqrt();
                if range > crate::combat::CANNON_RANGE_NM {
                    continue;
                }
                let (powder_need, shot_need) = crate::combat::broadside_supply_cost(cannons);
                let fired = match self.ships.get_mut(attacker_id) {
                    Some(a) => {
                        let have_p = a.cargo.get(crate::goods::ids::GUNPOWDER);
                        let have_s = a.cargo.get(crate::goods::ids::CANNON_SHOT);
                        if have_p < powder_need || have_s < shot_need {
                            false
                        } else {
                            a.cargo.remove(crate::goods::ids::GUNPOWDER, powder_need);
                            a.cargo.remove(crate::goods::ids::CANNON_SHOT, shot_need);
                            a.next_fire_at_minute =
                                now + crate::combat::reload_minutes(seasoned_ratio);
                            true
                        }
                    }
                    None => false,
                };
                if !fired {
                    continue;
                }
                let (hull_dmg, rig_dmg) = crate::combat::compute_broadside_damage(cannons, range);
                if let Some(target_ship) = self.ships.get_mut(target_id) {
                    target_ship.hull_integrity = (target_ship.hull_integrity - hull_dmg).max(0.0);
                    target_ship.rigging_integrity =
                        (target_ship.rigging_integrity - rig_dmg).max(0.0);
                    if target_ship.hull_integrity <= 0.0 && target_ship.state != ShipState::Sunk {
                        target_ship.state = ShipState::Sunk;
                    }
                }
            }
        }
    }

    /// Resolves a single boarding intent. Extracted from the old
    /// in-line Resolution Phase loop so Phase 4 §3b can run all
    /// boardings *after* the sub-tick combat exchange (so this hour's
    /// rigging damage is visible to the boarding-gate test).
    #[allow(clippy::too_many_lines)]
    fn resolve_boarding(&mut self, attacker: ShipId, tgt: ShipId) {
        let attacker_id = attacker;
        // ── original Step 8 boarding body, unchanged ──
        let (a_pos, a_vel, a_crew, a_morale, a_min_crew) = match self.ships.get(attacker_id) {
            Some(a) => {
                let stats = &self.ship_types.get(a.ship_type).stats;
                (
                    a.position,
                    a.velocity(),
                    a.crew_alive,
                    a.morale,
                    stats.crew_min(),
                )
            }
            None => return,
        };
        let (t_pos, t_vel, t_crew, t_morale, t_rig_frac) = match self.ships.get(tgt) {
            Some(t) => {
                if t.state == ShipState::Sunk {
                    return;
                }
                let stats = &self.ship_types.get(t.ship_type).stats;
                let frac = if stats.rigging_integrity_max > 0.0 {
                    (t.rigging_integrity / stats.rigging_integrity_max).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                (t.position, t.velocity(), t.crew_alive, t.morale, frac)
            }
            None => return,
        };
        // Re-gate range and rigging — same checks the
        // AI applied, repeated here in case another
        // command in this drain bumped either ship.
        let range = crate::combat::min_distance_over_tick(
            (a_pos.x, a_pos.y),
            a_vel,
            (t_pos.x, t_pos.y),
            t_vel,
        );
        if range > crate::combat::BOARDING_RANGE_NM {
            return;
        }
        if t_rig_frac >= crate::combat::BOARDING_RIGGING_THRESHOLD {
            return;
        }
        if a_crew < 2 || t_crew == 0 {
            return;
        }
        let outcome = crate::combat::resolve_boarding(a_crew, a_morale, t_crew, t_morale);
        // Apply attacker losses (pro-rata across
        // seasoned/unseasoned — boarding cuts down
        // veteran and landsman alike).
        if let Some(a) = self.ships.get_mut(attacker_id) {
            a.apply_crew_losses(outcome.attacker_losses);
        }
        // Apply defender losses.
        if let Some(t) = self.ships.get_mut(tgt) {
            t.apply_crew_losses(outcome.defender_losses);
        }
        if !outcome.attacker_wins {
            // Defender holds the deck. Boarders fall
            // back; no transfer, no flag change.
            return;
        }
        // Step 11.a: prize outcome. Historical pattern
        // (Rediker, Earle): pirates almost never kept
        // captured hulls — they stripped cargo, took
        // any silver, and either released the prize
        // (after a hostage scare), sank her, or sailed
        // her to a friendly haven to sell. Only rarely
        // did a prize join the fleet, and then only if
        // she was a real upgrade. Old behavior — "every
        // prize becomes a pirate" — produced runaway
        // pirate growth in long benches.
        let a_surviving = self
            .ships
            .get(attacker_id)
            .map(|a| a.crew_alive)
            .unwrap_or(0);

        // Compute prize value (cargo + hull bounty)
        // before mutating either ship. Cargo is valued
        // at a flat wholesale ~20 pesos/ton (close to
        // average bench prices for bulk goods like
        // sugar/molasses); hull bounty scales with
        // current hull integrity to reflect sale value
        // at a pirate haven.
        let (target_cargo_tons, target_hull, target_hull_max) = match self.ships.get(tgt) {
            Some(t) => {
                let stats = &self.ship_types.get(t.ship_type).stats;
                (
                    t.cargo.total_tons(),
                    t.hull_integrity,
                    stats.hull_integrity_max,
                )
            }
            None => (0.0, 0.0, 0.0),
        };
        let cargo_silver = target_cargo_tons * 20.0;
        let hull_bounty = target_hull * 8.0;

        // Decide outcome. Real-upgrade check first
        // (a 200-ton fluyt is no upgrade for a 60-ton
        // sloop pirate); then crew-spareable check;
        // then the weighted roll.
        let attacker_hull_max = self
            .ships
            .get(attacker_id)
            .map(|a| self.ship_types.get(a.ship_type).stats.hull_integrity_max)
            .unwrap_or(0.0);
        let could_upgrade = target_hull_max > attacker_hull_max * 1.2;
        let prize_crew = ((a_surviving as f32) * crate::combat::PRIZE_CREW_SPLIT).round() as u16;
        let attacker_after = a_surviving.saturating_sub(prize_crew);
        let can_spare_crew = attacker_after >= a_min_crew && prize_crew >= 2;

        let roll = combat_rng_step(&mut self.combat_rng_state);
        // Outcome weights (sum to 1.0):
        //   take    : 0.05 (only if real upgrade + crew spareable)
        //   sell    : 0.30 (silver bonus, prize sunk)
        //   sink    : 0.50 (default — cargo stripped, hull released/burned)
        //   release : 0.15 (cargo stripped, target lives to trade again)
        let take = could_upgrade && can_spare_crew && roll < 0.05;
        let sell = !take && roll < 0.35;
        let release = !take && !sell && roll >= 0.85;
        // (sink is the default if none of the above)

        // Apply stripped cargo + silver to attacker
        // (and clear from target unless we're taking it).
        if !take {
            if let Some(a) = self.ships.get_mut(attacker_id) {
                let bonus = if sell {
                    cargo_silver + hull_bounty
                } else {
                    cargo_silver
                };
                a.silver += bonus;
                a.morale = (a.morale + crate::ship::MORALE_GAIN_PRIZE_TAKEN).clamp(0.0, 1.0);
            }
            if let Some(t) = self.ships.get_mut(tgt) {
                t.cargo = crate::cargo::Cargo::new();
                t.silver = (t.silver * 0.1).max(0.0); // boarders took most of it
            }
        }

        if take {
            // Real upgrade — flip the prize. This is
            // the *only* outcome that adds to the
            // pirate fleet.
            self.prizes_taken += 1;
            let new_faction = self.ships.get(attacker_id).map(|a| a.faction);
            // Detach the prize crew from the attacker
            // pro-rata over seasoned (veterans split with
            // the rest of the boarding party). Note:
            // `prize_crew` was computed against
            // `a_surviving` (post-melee), and the attacker
            // already had `attacker_losses` applied above,
            // so `crew_alive` here equals `a_surviving` —
            // the detach below leaves it at `attacker_after`.
            let (detached, detached_seasoned) = match self.ships.get_mut(attacker_id) {
                Some(a) => {
                    let d = a.detach_prize_crew(prize_crew);
                    a.morale = (a.morale + crate::ship::MORALE_GAIN_PRIZE_TAKEN).clamp(0.0, 1.0);
                    d
                }
                None => (0, 0),
            };
            debug_assert_eq!(
                self.ships.get(attacker_id).map(|a| a.crew_alive),
                Some(attacker_after),
                "detach_prize_crew should leave attacker at attacker_after"
            );
            if let Some(t) = self.ships.get_mut(tgt) {
                t.crew_alive = t.crew_alive.saturating_add(detached);
                t.crew_seasoned = t.crew_seasoned.saturating_add(detached_seasoned);
                t.policy = ShipPolicy::Pirate;
                if let Some(f) = new_faction {
                    t.faction = f;
                }
                t.morale = 0.8;
                t.speed = 0.0;
                t.nav.waypoints.clear();
            }
            if let Some(ai) = self.ship_ais.get_mut(tgt) {
                ai.goal.destination = None;
                ai.goal.dest_port = None;
                ai.goal.pursue_target = None;
                ai.goal.flee_from = None;
            }
        } else if sell || (!release) {
            // Sell-at-haven and sink-as-default both
            // result in the prize being removed from
            // the world (we don't model the actual
            // voyage to a pirate haven yet). The
            // distinction lives in `prizes_sold` vs
            // `prizes_sunk` for bench reporting.
            if sell {
                self.prizes_sold += 1;
            } else {
                self.prizes_sunk += 1;
            }
            if let Some(t) = self.ships.get_mut(tgt) {
                t.hull_integrity = 0.0;
                t.rigging_integrity = 0.0;
                t.state = ShipState::Sunk;
            }
        } else {
            // release — target survives, returns to
            // trade with empty holds. The captain re-
            // plans next tick (cargo is gone, but the
            // hull and crew live).
            self.prizes_released += 1;
        }
    }
}

/// Free-function form of `World::combat_uniform`, taking the rng state
/// by `&mut` so it can be called from inside `self.commands.drain(..)`
/// loops that already hold a mutable borrow on `self`. xorshift64 with
/// a multiplicative mixer.
fn combat_rng_step(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    let r = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
    (r >> 11) as f32 / ((1u64 << 53) as f32)
}
