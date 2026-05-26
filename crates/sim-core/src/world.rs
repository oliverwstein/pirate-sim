use std::path::Path;

use slotmap::{Key, SecondaryMap, SlotMap};

use crate::ai::{ShipAI, ShipSnapshot};
use crate::coastline::{CoastlineMap, LandMesh};
use crate::equilibrium::{self, EquilibriumScenario, FreightCostModel, PortSpec};
use crate::goods::GoodsRegistry;
use crate::harbor::HarborMap;
use crate::map::MapSystem;
use crate::market::{archetype_for, seed_balance_from_equilibrium, PortMarket};
use crate::money::Pesos;
use crate::navmesh::Navmesh;
use crate::pathfind::PathfindContext;
use crate::pop::{self, PortDemographics};
use crate::port::{all_ports, Port};
use crate::ship::{Ship, ShipPolicy, ShipState, ShipStats};
use crate::shiptype::{self, ShipTypeRegistry};
use crate::shipyard::{self, BuildOutcome};
use crate::sim_rng::SimRng;
use crate::spatial::SpatialHash;
use crate::tile_mesh::TileMesh;
use crate::types::{ShipId, SimDate};
use crate::weather::WeatherSystem;

/// Phase 6: per-port single-price call auction record.
struct AuctionBid {
    ship_id: ShipId,
    tons: f32,
    limit_price: f32,
    /// Resupply bids fill into `ship.provisions` instead of cargo.
    is_resupply: bool,
}
struct AuctionAsk {
    ship_id: ShipId,
    tons: f32,
    limit_price: f32,
}

pub struct World {
    pub map: MapSystem,
    pub weather: WeatherSystem,
    pub ports: Vec<Port>,
    pub harbors: HarborMap,
    pub navmesh: Navmesh,
    /// Phase A: portal-aware convex-tile navmesh loaded offline by
    /// `tools/preprocess/preprocess_navmesh.py`. Anchored to data at
    /// `data/grids/navmesh.bin`. Phase B uses it to derive each
    /// port's anchor tile; Phases C–E migrate path planning and
    /// motion onto it.
    pub tile_mesh: TileMesh,
    pub coastline: CoastlineMap,
    pub land_mesh: LandMesh,
    pub goods: GoodsRegistry,
    /// Catalog of ship designs. A `Ship` indexes in via its
    /// `ship_type` field to fetch per-tick stats and (for shipyard
    /// ports) build costs.
    pub ship_types: ShipTypeRegistry,
    /// Per-port economic state, parallel to `ports` (index = port index).
    pub markets: Vec<PortMarket>,
    /// Per-entity observability aggregates: one entry per port,
    /// indexed by port_idx. Mutated as a byproduct of clearing
    /// and docking; never read by sim logic. See `telemetry.rs`
    /// and `planning/observability-plan.md`.
    pub port_telemetry: Vec<crate::telemetry::PortTelemetry>,
    /// Per-faction observability aggregates: fixed-size array
    /// indexed by `Faction as usize`. See `telemetry.rs`.
    pub faction_telemetry: [crate::telemetry::FactionTelemetry; crate::telemetry::N_FACTIONS],
    /// Faction trade-policy table: per-port docking permission, per-good
    /// legality, and ad-valorem duties as a function of the visiting
    /// ship's flag. Constructed at load time from `faction_defaults()`
    /// overlaid with `port_policies.ron` deltas; read-only thereafter.
    pub policy: crate::policy::PolicyResolver,
    /// Pre-computed shortest paths to every port over the static
    /// navmesh. Built once at load by `PortRouteCache::build`; every
    /// per-tick `find_path_to_harbor` call becomes a constant-time
    /// lookup + predecessor walk. See `portroutes.rs`.
    pub port_routes: crate::portroutes::PortRouteCache,
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
    silver_at_month_start: SecondaryMap<ShipId, Pesos>,
    /// Last completed month's average per-ship silver delta (pesos).
    /// Used by `shipyard::try_build` as the expected per-ship monthly
    /// profit for new vessels. Starts at 0 (no fleet history); first
    /// month's tick updates it.
    pub last_month_avg_profit: Pesos,
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
    /// Phase 4 §3c-2b: prizes currently sailing to port under tow
    /// (`prize_owner = Some(victor)`). Counter is incremented on
    /// successful tow setup in `resolve_prize_action` and decremented
    /// when the prize either docks (rolls into `prizes_sold`) or is
    /// orphaned by a victor sinking (`prizes_orphaned`).
    pub prizes_in_tow: u32,
    pub prizes_orphaned: u32,
    /// Step 11.a: deterministic RNG for stochastic combat outcomes
    /// (prize handling, future morale rolls, etc.). Seeded once at
    /// `World::load`; same world state → same outcome trace.
    pub combat_rng: SimRng,
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
    /// Phase 6 instrumentation: wall-clock nanoseconds spent in the
    /// AI Phase of the most recent hourly tick (read-only world,
    /// pushes commands). Used by `bench_ai_tick` to measure the
    /// payoff of parallelizing the AI phase. Zero before the first
    /// hourly tick of a run.
    pub last_ai_phase_ns: u64,
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
        let tile_mesh = TileMesh::load(&data_dir.join("grids/navmesh.bin"))
            .expect("load data/grids/navmesh.bin (run tools/preprocess/preprocess_navmesh.py)");
        let harbors = HarborMap::build(&map.land, &tile_mesh, &ports);
        let navmesh = Navmesh::build(&map.land);
        let coastline =
            CoastlineMap::load(&data_dir.join("grids/coastline.bin")).unwrap_or_default();
        let land_mesh = LandMesh::load(&data_dir.join("grids/land_polys.bin")).unwrap_or_default();
        let goods = GoodsRegistry::starter();
        let port_specs: Vec<PortSpec<'_>> = ports
            .iter()
            .map(|p| {
                let archetype = archetype_for(&p.name);
                PortSpec::from_world(p, archetype.recipe())
            })
            .collect();
        let mut markets: Vec<PortMarket> = port_specs
            .iter()
            .map(|spec| PortMarket::with_recipe(&goods, spec.recipe.clone()))
            .collect();
        // Use the linear freight model as a mechanism-free equilibrium baseline;
        // ship operations then discover route profitability from these seeded prices.
        let freight = FreightCostModel::Linear {
            pesos_per_ton_nm: 0.05,
        };
        let equilibrium_solution = equilibrium::solve(&EquilibriumScenario {
            ports: port_specs,
            goods: &goods,
            freight,
        });
        for (port_idx, market) in markets.iter_mut().enumerate() {
            seed_balance_from_equilibrium(market, port_idx, &equilibrium_solution, &goods);
        }
        let goods_seeded = (0..ports.len())
            .flat_map(|port_idx| goods.iter().map(move |good| (port_idx, good.id)))
            .filter(|(port_idx, good)| equilibrium_solution.price_at(*port_idx, *good).is_some())
            .count();
        eprintln!(
            "equilibrium market seed: ports={} flows={} surplus={:+.0} seeded_cells={}",
            ports.len(),
            equilibrium_solution.flows.len(),
            equilibrium_solution.objective,
            goods_seeded
        );
        let port_telemetry: Vec<crate::telemetry::PortTelemetry> = (0..ports.len())
            .map(|_| crate::telemetry::PortTelemetry::default())
            .collect();
        // Faction trade policy: load bundled per-port overrides (if
        // the file is missing, every port falls back to its faction
        // default). Unknown port names in the overrides are a fatal
        // typo — surfaced by an early panic so they're caught in CI.
        let policy = match crate::policy::load_port_policies(&ports, &goods) {
            Ok(overrides) => crate::policy::PolicyResolver::build(&ports, &overrides),
            Err(e) => panic!("failed to load port_policies.ron: {e}"),
        };
        let demographics: Vec<PortDemographics> = ports
            .iter()
            .map(|p| PortDemographics::seed(p.category, p.faction))
            .collect();

        let date = SimDate::new(1680, 0, 1);
        let last_market_month = date.month();
        let last_hire_day = date.day_of_year;

        // Phase D: pre-compute SSSP-to-each-port tables over the
        // tile mesh. Per-tick voyage planning becomes a lookup + the
        // shared funnel-stitch instead of a live A*. Must be built
        // after `harbors` and `tile_mesh`; a few ms per port.
        let port_routes = crate::portroutes::PortRouteCache::build(&tile_mesh, &harbors);

        Self {
            map,
            weather,
            ports,
            harbors,
            navmesh,
            tile_mesh,
            coastline,
            land_mesh,
            goods,
            ship_types,
            markets,
            port_telemetry,
            faction_telemetry: Default::default(),
            policy,
            port_routes,
            demographics,
            ships: SlotMap::with_key(),
            ship_ais: SecondaryMap::new(),
            date,
            last_market_month,
            last_hire_day,
            silver_at_month_start: SecondaryMap::new(),
            last_month_avg_profit: Pesos::ZERO,
            ships_built: 0,
            mutinies_total: 0,
            attrition_storms: 0,
            attrition_foundered: 0,
            attrition_fires: 0,
            prizes_taken: 0,
            prizes_sold: 0,
            prizes_sunk: 0,
            prizes_released: 0,
            prizes_in_tow: 0,
            prizes_orphaned: 0,
            combat_rng: SimRng::new(0x5052_495A_4520_5247_u64 ^ 0x9E37_79B9_7F4A_7C15),
            spatial: SpatialHash::new(),
            commands: Vec::new(),
            sim_minute: 0,
            last_ai_phase_ns: 0,
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
                let starting_silver =
                    Pesos::from_pesos_f32((stats.cargo_capacity_tons * 25.0).max(1500.0));
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
            let total_delta: Pesos = self
                .ships
                .iter()
                .filter_map(|(id, s)| {
                    self.silver_at_month_start
                        .get(id)
                        .map(|prev| s.silver - *prev)
                })
                .sum();
            self.last_month_avg_profit =
                Pesos::from_centavos(total_delta.as_centavos() / self.ships.len() as i64);
        } else {
            self.last_month_avg_profit = Pesos::ZERO;
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
                self.last_month_avg_profit.as_pesos_f32(),
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
            self.faction_telemetry[ship.faction as usize].ships_built += 1;
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
            let (port_idx, want, ship_type, ship_silver, is_hiring): (usize, u16, _, Pesos, bool) =
                match self.ships.get(id) {
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
            let affordable = if ship_silver.is_positive() {
                (ship_silver.as_centavos() / crate::ship::SIGN_ON_BOUNTY_PESOS.as_centavos()).max(0)
                    as u16
            } else {
                0
            };
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
            let bounty = crate::ship::SIGN_ON_BOUNTY_PESOS.scale(drawn as f32);
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
        )
        .with_port_routes(&self.port_routes)
        .with_tile_mesh(&self.tile_mesh);

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
                        hull_frac: if stats.hull_integrity_max > 0.0 {
                            (ship.hull_integrity / stats.hull_integrity_max).clamp(0.0, 1.0)
                        } else {
                            1.0
                        },
                        cannons: stats.cannons,
                    },
                );
            }
        }
        // Sort the spatial index once now that all inserts are done.
        // `neighbors` is then a `&self` query; this lets AI code hold
        // a `&SpatialHash` alongside other `&self` borrows on `World`.
        self.spatial.finalize();

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

        // ─── §3c-2b: Copy-owner-nav pass ─────────────────────────────
        // Before any ship makes an AI decision this tick, propagate
        // each victor's current destination/dest_port into every prize
        // they have in tow. The prize will then run its normal AI tick
        // — `act_sail` will plan a route to the same port. If the
        // victor has sunk between ticks (no entry in `self.ships`), the
        // prize is orphaned: clear `prize_owner` and let her continue
        // with her last-known destination under her own (now-pirate)
        // colors. This is the "no rescue from beyond the grave" rule.
        let owner_goals: SecondaryMap<ShipId, (Option<crate::types::Position>, Option<usize>)> = {
            let mut m = SecondaryMap::new();
            for id in ids.iter().copied() {
                if let Some(ai) = self.ship_ais.get(id) {
                    m.insert(id, (ai.goal.destination, ai.goal.dest_port));
                }
            }
            m
        };
        for id in ids.iter().copied() {
            let owner = match self.ships.get(id).and_then(|s| s.prize_owner) {
                Some(o) => o,
                None => continue,
            };
            // Owner gone (sunk between ticks) → orphan.
            if !self.ships.contains_key(owner) {
                if let Some(s) = self.ships.get_mut(id) {
                    s.prize_owner = None;
                }
                self.prizes_in_tow = self.prizes_in_tow.saturating_sub(1);
                self.prizes_orphaned += 1;
                continue;
            }
            // Owner has no plan yet (e.g., just docked / replanning).
            // Leave the prize's current goal untouched this tick.
            let Some(&(dest, dest_port)) = owner_goals.get(owner) else {
                continue;
            };
            if dest.is_none() && dest_port.is_none() {
                continue;
            }
            if let Some(ai) = self.ship_ais.get_mut(id) {
                if ai.goal.destination != dest || ai.goal.dest_port != dest_port {
                    ai.goal.destination = dest;
                    ai.goal.dest_port = dest_port;
                    // Invalidate any cached waypoints — the new port
                    // forces a fresh A* on the next `act_sail` tick.
                    if let Some(s) = self.ships.get_mut(id) {
                        s.nav.waypoints.clear();
                    }
                }
            }
        }

        // ─── AI Phase (read-only over other ships) ───────────────────
        // Each ship's AI ticks against the pre-tick world snapshot
        // (`snapshots` + `spatial`) and pushes its intents into a
        // per-task local buffer. No ship may mutate another ship
        // here; ship self-mutation is allowed (cargo bookkeeping at
        // dock, etc.).
        //
        // Parallelism: ships are independent during the AI phase
        // (read-only world, write-only to per-ship buffer), so we
        // Rayon-parallelize the per-ship loop. Determinism is
        // preserved by sorting per-ship outputs by `ShipId.data()`
        // before flattening into the global command buffer, so the
        // downstream resolution phase sees the same drain order as
        // the serial implementation.
        let ai_phase_start = std::time::Instant::now();

        // Materialize disjoint (id, &mut Ship, &mut ShipAI) triples
        // by zipping the two slotmaps. Both iterate in slot-index
        // order, and every ship has an AI (set at spawn), so the
        // keys line up; we `debug_assert_eq!` to catch any drift.
        // 503 ships × 24 bytes ≈ 12 KB; trivial per-tick churn.
        let pairs: Vec<(ShipId, &mut crate::ship::Ship, &mut crate::ai::ShipAI)> = self
            .ships
            .iter_mut()
            .zip(self.ship_ais.iter_mut())
            .map(|((sid, ship), (aid, ai))| {
                debug_assert_eq!(sid, aid, "ship/ai slotmap keys diverged");
                (sid, ship, ai)
            })
            .collect();

        // Reborrow world state as plain `&_` so each Rayon task
        // captures a Sync reference (PathfindContext, SpatialHash,
        // WindGrid, etc. are all read-only data).
        let ports = &self.ports;
        let harbors = &self.harbors;
        let markets = &self.markets;
        let goods = &self.goods;
        let policy_ref = &self.policy;
        let port_telemetry_ref = &self.port_telemetry[..];
        let snapshots_ref = &snapshots;
        let spatial_ref = &self.spatial;
        let pathfind_ref = &pathfind;
        let ship_types = &self.ship_types;
        let weather_wind = &self.weather.wind;
        let day_of_year = self.date.day_of_year;

        use rayon::prelude::*;
        let mut results: Vec<(ShipId, Vec<(ShipId, crate::command::ShipCommand)>)> = pairs
            .into_par_iter()
            .map(|(id, ship, ai)| {
                // Perf Phase 1: borrow the registry's ShipStats instead
                // of cloning per ship per tick. `ship_types` is captured
                // as `&self.ship_types` and outlives this closure; the
                // returned reference is bound to that lifetime.
                let ship_stats: &ShipStats = &ship_types.get(ship.ship_type).stats;
                let wind = weather_wind.wind_at(ship.position, month);
                let mut local_commands: Vec<(ShipId, crate::command::ShipCommand)> = Vec::new();
                {
                    let mut inputs = crate::ai::ShipTickInputs {
                        me: id,
                        ship,
                        stats: ship_stats,
                        wind: &wind,
                        ports,
                        harbors,
                        pathfind: Some(pathfind_ref),
                        markets,
                        goods,
                        policy: policy_ref,
                        port_telemetry: port_telemetry_ref,
                        commands: &mut local_commands,
                        day_of_year,
                        snapshots: snapshots_ref,
                        spatial: spatial_ref,
                    };
                    ai.tick(&mut inputs);
                }
                (id, local_commands)
            })
            .collect();

        // Determinism: sort by ShipId slot index so the resolution
        // phase below sees the same drain order as a serial loop
        // would. (par_bridge / par_iter are unordered.)
        results.sort_by_key(|(id, _)| id.data());
        for (_id, cmds) in results {
            self.commands.extend(cmds);
        }
        self.last_ai_phase_ns = ai_phase_start.elapsed().as_nanos() as u64;

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
        let mut strikes: Vec<(ShipId, ShipId)> = Vec::new();
        // Phase 6: market intents collected during the drain are
        // resolved by `clear_markets` below as a per-port single-price
        // call auction. Held as `(ship_id, command)` pairs so the
        // auction can preserve deterministic ship-id ordering for any
        // tiebreaks (pro-rata allocation, silver-only op ordering).
        let mut market_intents: Vec<(ShipId, crate::command::ShipCommand)> = Vec::new();
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
                // Phase 4 §3c-1 (symmetric redesign): mutually clear
                // engagement flags and stamp a 60-minute cooldown on
                // both sides so the next fired broadside this hour
                // does not immediately re-engage them. Silent no-op if
                // either ship is gone or the pair is no longer
                // engaged (idempotent).
                crate::command::ShipCommand::Disengage { other } => {
                    let cooldown_until = self.sim_minute + 60;
                    for x in [attacker, other] {
                        if let Some(s) = self.ships.get_mut(x) {
                            s.engaged_with = None;
                            s.disengaged_until_minute = cooldown_until;
                        }
                    }
                }
                crate::command::ShipCommand::Strike { to } => {
                    // Phase 4 §3c-2: the issuing ship surrenders to
                    // `to`. Clear engagement on both immediately so
                    // any later command this hour does not re-fire
                    // on a struck prize, then defer the prize-action
                    // resolution to after the drain (cannot
                    // double-borrow `self` inside the drain loop).
                    let prize_id = attacker;
                    let victor_id = to;
                    for x in [prize_id, victor_id] {
                        if let Some(s) = self.ships.get_mut(x) {
                            s.engaged_with = None;
                        }
                    }
                    strikes.push((victor_id, prize_id));
                }
                // Phase 6: market intents are deferred to the per-port
                // call auction below (`clear_markets`). Preserve the
                // ship-id so the auction sees deterministic ordering.
                m @ (crate::command::ShipCommand::MarketBid { .. }
                | crate::command::ShipCommand::MarketAsk { .. }
                | crate::command::ShipCommand::MarketResupplyBid { .. }
                | crate::command::ShipCommand::MarketDeposit { .. }
                | crate::command::ShipCommand::MarketCollectDebt { .. }
                | crate::command::ShipCommand::MarketDrawOutfit { .. }
                | crate::command::ShipCommand::MarketCreditBid { .. }) => {
                    market_intents.push((attacker, m));
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
        // Phase 4 §3c-2: Strike surrenders. Resolved *after* sub-tick
        // combat and boarding so a ship that has already sunk this
        // hour is correctly skipped. Engagement was already cleared
        // when the command was drained; this pass dispatches the
        // surrendered hull through the shared prize resolver.
        for (victor_id, prize_id) in strikes {
            let prize_alive = self
                .ships
                .get(prize_id)
                .map(|s| s.state != ShipState::Sunk)
                .unwrap_or(false);
            let victor_alive = self
                .ships
                .get(victor_id)
                .map(|s| s.state != ShipState::Sunk)
                .unwrap_or(false);
            if prize_alive && victor_alive {
                self.resolve_prize_action(victor_id, prize_id);
            }
        }

        // Phase 6: per-port single-price call-auction over the market
        // intents that were emitted (and deferred) during the AI Phase.
        // This is where ships actually transact: pre-existing
        // ship-side mutation has already happened (none, in fact, for
        // these intents), and now the resolver applies clearing prices,
        // fills, and port-side ledger movement in one deterministic pass.
        if !market_intents.is_empty() {
            self.clear_markets(market_intents);
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
            let mutiny_roll = self.combat_rng.uniform_f32();
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
                    let hourly = crate::ship::WAGE_PESOS_PER_MAN_MONTH
                        .scale(ship.crew_alive as f32 / (30.0 * 24.0));
                    ship.wages_owed_pesos += hourly;
                }
                ShipState::Docked => {
                    if ship.wages_owed_pesos.is_positive() {
                        if let Some(port_idx) = ship.nav.docked_at_port {
                            let pay = ship.wages_owed_pesos.min(ship.silver.max_zero());
                            if pay.is_positive() {
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

        // ─── §3c-2b: Pay-at-port pass ────────────────────────────────
        // Any prize-in-tow that finished her voyage this tick (i.e.,
        // her AI docked her at her owner's destination port) settles:
        // the victor receives `cargo_silver + hull_bounty` from her
        // holds + hull, and the prize is marked Sunk so the Cleanup
        // phase below despawns her. If the victor sank earlier this
        // tick, payment is forfeited — the prize simply sinks
        // (orphan-after-docking is a vanishingly rare race; the dock
        // arrival itself means the orphan-detection pass above didn't
        // fire this tick because the victor was still alive at AI Phase
        // start). Mirrors the cargo_silver / hull_bounty formulae used
        // in `resolve_prize_action` for instant-sell.
        let arrived: Vec<ShipId> = self
            .ships
            .iter()
            .filter_map(|(id, s)| {
                if s.state == ShipState::Docked && s.prize_owner.is_some() {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();
        for id in arrived {
            let (owner, cargo_silver, hull_bounty) = match self.ships.get(id) {
                Some(p) => {
                    let owner = p.prize_owner.expect("filtered to Some");
                    let cargo_silver = Pesos::from_pesos_f32(p.cargo.total_tons() * 20.0);
                    let hull_bounty = Pesos::from_pesos_f32(p.hull_integrity * 8.0);
                    (owner, cargo_silver, hull_bounty)
                }
                None => continue,
            };
            if let Some(a) = self.ships.get_mut(owner) {
                a.silver += cargo_silver + hull_bounty;
                a.morale = (a.morale + crate::ship::MORALE_GAIN_PRIZE_TAKEN).clamp(0.0, 1.0);
            }
            if let Some(p) = self.ships.get_mut(id) {
                p.prize_owner = None;
                p.hull_integrity = 0.0;
                p.rigging_integrity = 0.0;
                p.state = ShipState::Sunk;
            }
            self.prizes_in_tow = self.prizes_in_tow.saturating_sub(1);
            self.prizes_sold += 1;
        }

        // Phase 4 §3c-1: clear engagements that hit a terminal
        // condition this hour (counterpart sunk, defender escaped).
        // Runs after physics so the post-physics range/rigging values
        // are what the escape check sees; runs before Cleanup so a
        // ship sunk this hour is still visible with `state == Sunk`
        // for the counterpart-gone check.
        self.check_engagement_terminations();

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
            if let Some(s) = self.ships.get(id) {
                self.faction_telemetry[s.faction as usize].ships_lost += 1;
            }
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
    /// Phase 6: drain market intents emitted by the AI Phase and resolve
    /// them as a per-port single-price call auction.
    ///
    /// Determinism: ports processed in port-index order; within each
    /// port, silver-only operations (debt collection, profit deposit,
    /// outfit draw, credit advance) play through in ship-id order
    /// against the live treasury; then each good with bids and/or asks
    /// is cleared at a *single* price derived from the post-tick
    /// signed balance, with pro-rata seller payouts when the
    /// port can't fully cover. This eliminates the "first-bidder gets
    /// the start-of-tick price" artifact of sequential resolution.
    fn clear_markets(&mut self, intents: Vec<(ShipId, crate::command::ShipCommand)>) {
        use crate::command::ShipCommand;
        use std::collections::BTreeMap;

        // Bucket intents by port. BTreeMap iteration is port-index
        // ordered, which is the deterministic processing order.
        let mut by_port: BTreeMap<usize, Vec<(ShipId, ShipCommand)>> = BTreeMap::new();
        for (id, cmd) in intents {
            let port = match &cmd {
                ShipCommand::MarketBid { port, .. }
                | ShipCommand::MarketAsk { port, .. }
                | ShipCommand::MarketResupplyBid { port, .. }
                | ShipCommand::MarketDeposit { port, .. }
                | ShipCommand::MarketCollectDebt { port }
                | ShipCommand::MarketDrawOutfit { port, .. }
                | ShipCommand::MarketCreditBid { port, .. } => *port,
                _ => continue,
            };
            by_port.entry(port).or_default().push((id, cmd));
        }

        for (port_idx, port_intents) in by_port {
            if port_idx >= self.markets.len() {
                continue;
            }
            self.clear_port_intents(port_idx, port_intents);
        }
    }

    /// Resolve all of one port's queued market intents for this tick.
    /// See `clear_markets` for the per-tick ordering invariants.
    fn clear_port_intents(
        &mut self,
        port_idx: usize,
        intents: Vec<(ShipId, crate::command::ShipCommand)>,
    ) {
        use crate::command::ShipCommand;
        use crate::ship::{CHANDLER_PORT_FRACTION_CAP, MAX_SHIP_DEBT};

        // Stable deterministic order across all intents at this port.
        // ShipId's underlying KeyData implements Ord (generational
        // tie-break), so sorting by `.data()` gives a total order that
        // doesn't depend on push order.
        let mut intents = intents;
        intents.sort_by_key(|(id, _)| id.data());

        // ── Step A: silver-only operations in ship-id order ──
        //
        // These don't affect prices; they just shuffle the port
        // treasury. Played strictly in order so a port that runs out
        // of silver mid-tick correctly underpays only the later ships
        // in line (deterministic).
        for (ship_id, cmd) in &intents {
            let ship = match self.ships.get_mut(*ship_id) {
                Some(s) => s,
                None => continue,
            };
            let market = &mut self.markets[port_idx];
            match cmd {
                ShipCommand::MarketCollectDebt { .. } => {
                    market.collect_debt(ship, super::ai::HOME_PORT_FLOAT_SILVER);
                }
                ShipCommand::MarketDeposit { amount, .. } => {
                    // Cap at what the ship still has — debt collection
                    // above may have eaten the surplus.
                    let pay = (*amount).min(ship.silver).max_zero();
                    if pay.is_positive() {
                        ship.silver -= pay;
                        market.silver += pay;
                        ship.lifetime_dividends += pay;
                        // Telemetry: trade profits remitted to a
                        // home-port treasury. Disjoint-field borrow
                        // OK — `self.ships` / `self.markets` borrows
                        // are unrelated to `self.faction_telemetry`.
                        let f = ship.faction as usize;
                        self.faction_telemetry[f].silver_returned_home += pay;
                    }
                }
                ShipCommand::MarketDrawOutfit { target_silver, .. } => {
                    market.draw_for_outfit(
                        ship,
                        *target_silver,
                        super::ai::OUTFIT_PORT_FRACTION_CAP,
                    );
                }
                ShipCommand::MarketCreditBid { max_amount, .. } => {
                    market.extend_credit(
                        ship,
                        *max_amount,
                        super::ai::TRAMP_PORT_FRACTION_CAP,
                        MAX_SHIP_DEBT,
                    );
                }
                _ => {}
            }
        }

        // ── Step B: per-good single-price call auction ──
        //
        // Group bids and asks by good, then for each (port, good) find
        // a single clearing price that all crossing orders transact at.
        // The clearing price is the formula price evaluated at the
        // *post-tick* signed balance (current ± net trade flow),
        // which gives every order the marginal price the trade itself
        // would induce — no more "first ship at the dock gets the
        // pre-tick price".
        use std::collections::HashMap;
        let mut bids: HashMap<crate::goods::GoodId, Vec<AuctionBid>> = HashMap::new();
        let mut asks: HashMap<crate::goods::GoodId, Vec<AuctionAsk>> = HashMap::new();

        for (ship_id, cmd) in &intents {
            match cmd {
                ShipCommand::MarketBid {
                    good,
                    tons,
                    limit_price,
                    ..
                } => {
                    if *tons > 0.0 {
                        bids.entry(*good).or_default().push(AuctionBid {
                            ship_id: *ship_id,
                            tons: *tons,
                            limit_price: *limit_price,
                            is_resupply: false,
                        });
                    }
                }
                ShipCommand::MarketResupplyBid {
                    tons, limit_price, ..
                } => {
                    if *tons > 0.0 {
                        bids.entry(crate::goods::ids::PROVISIONS)
                            .or_default()
                            .push(AuctionBid {
                                ship_id: *ship_id,
                                tons: *tons,
                                limit_price: *limit_price,
                                is_resupply: true,
                            });
                    }
                }
                ShipCommand::MarketAsk {
                    good,
                    tons,
                    limit_price,
                    ..
                } => {
                    if *tons > 0.0 {
                        asks.entry(*good).or_default().push(AuctionAsk {
                            ship_id: *ship_id,
                            tons: *tons,
                            limit_price: *limit_price,
                        });
                    }
                }
                _ => {}
            }
        }

        // Union of goods to clear. Sort for determinism.
        let mut goods: Vec<crate::goods::GoodId> = bids
            .keys()
            .chain(asks.keys())
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        goods.sort();

        for good in goods {
            let bids_for_good = bids.remove(&good).unwrap_or_default();
            let asks_for_good = asks.remove(&good).unwrap_or_default();
            self.clear_one_good(port_idx, good, bids_for_good, asks_for_good);
            let _ = (CHANDLER_PORT_FRACTION_CAP,); // silence unused import warning if both
        }
    }

    /// Inner helper: clear bids and asks for one (port, good) at a
    /// single auction price. Mutates port balance/treasury and the
    /// participating ships' cargo/silver.
    #[allow(clippy::too_many_lines)]
    fn clear_one_good(
        &mut self,
        port_idx: usize,
        good: crate::goods::GoodId,
        bids: Vec<AuctionBid>,
        asks: Vec<AuctionAsk>,
    ) {
        let total_bid_tons: f32 = bids.iter().map(|b| b.tons).sum();
        let total_ask_tons: f32 = asks.iter().map(|a| a.tons).sum();
        if total_bid_tons <= 0.0 && total_ask_tons <= 0.0 {
            return;
        }

        #[derive(Clone, Copy, Debug, Default)]
        struct FillAux {
            buy_duty: Option<f32>,
            sell_duty: Option<f32>,
            is_resupply: bool,
        }

        use crate::market_clearer::{ClearAsk, ClearBid};
        use std::collections::BTreeMap;

        let mut aux: BTreeMap<u32, FillAux> = BTreeMap::new();
        let mut ship_ids: BTreeMap<u32, ShipId> = BTreeMap::new();
        let mut clear_bids = Vec::with_capacity(bids.len());
        let mut clear_asks = Vec::with_capacity(asks.len());

        for b in bids {
            let flag = match self.ships.get(b.ship_id) {
                Some(s) => s.faction,
                None => continue,
            };
            let buy_duty = match self.policy.buy_legality(port_idx, flag, good) {
                crate::policy::TradeLegality::Legal { duty } => duty,
                crate::policy::TradeLegality::Prohibited => continue,
            };
            let clear_ship_id = b.ship_id.data().as_ffi() as u32;
            if let Some(existing) = ship_ids.insert(clear_ship_id, b.ship_id) {
                debug_assert_eq!(existing, b.ship_id);
                if existing != b.ship_id {
                    ship_ids.insert(clear_ship_id, existing);
                    continue;
                }
            }
            let entry = aux.entry(clear_ship_id).or_default();
            entry.buy_duty = Some(buy_duty);
            entry.is_resupply |= b.is_resupply;
            clear_bids.push(ClearBid {
                ship_id: clear_ship_id,
                tons: b.tons,
                max_price_pesos_per_ton: b.limit_price / (1.0 + buy_duty),
            });
        }

        for a in asks {
            let flag = match self.ships.get(a.ship_id) {
                Some(s) => s.faction,
                None => continue,
            };
            let sell_duty = match self.policy.sell_legality(port_idx, flag, good) {
                crate::policy::TradeLegality::Legal { duty } => duty,
                crate::policy::TradeLegality::Prohibited => continue,
            };
            if sell_duty >= 1.0 {
                continue;
            }
            let clear_ship_id = a.ship_id.data().as_ffi() as u32;
            if let Some(existing) = ship_ids.insert(clear_ship_id, a.ship_id) {
                debug_assert_eq!(existing, a.ship_id);
                if existing != a.ship_id {
                    ship_ids.insert(clear_ship_id, existing);
                    continue;
                }
            }
            aux.entry(clear_ship_id).or_default().sell_duty = Some(sell_duty);
            clear_asks.push(ClearAsk {
                ship_id: clear_ship_id,
                tons: a.tons,
                min_price_pesos_per_ton: a.limit_price / (1.0 - sell_duty),
            });
        }

        if clear_bids.is_empty() && clear_asks.is_empty() {
            return;
        }

        let market = &self.markets[port_idx];
        let current_balance = market.balance.get(good);
        let bound = market.effective_bound(good).max(1);
        let result = crate::market_clearer::clear(
            self.goods.get(good).base_price_pesos,
            current_balance,
            bound,
            &clear_bids,
            &clear_asks,
        );
        if result.fills.is_empty() {
            return;
        }

        let clearing_price = result.clearing_price_pesos_per_ton;
        let mut fills = result.fills;
        fills.sort_by_key(|f| f.ship_id);

        // Pre-pass: cap sell fills to actual cargo, cap buy fills to ship
        // silver, so the pro-rata payout_ratio reflects what will really
        // settle. The clearer doesn't know about per-ship inventory or
        // solvency caps; if it produced more sell tons than a ship holds,
        // the apply pass below would silently shrink the sell, but a
        // naive total_sell_base sum would still include the un-shrunk
        // tons and depress payout_ratio for everyone.
        let mut total_buy_base = 0.0_f32;
        let mut total_sell_base = 0.0_f32;
        for fill in &fills {
            let ship_id = match ship_ids.get(&fill.ship_id) {
                Some(id) => *id,
                None => continue,
            };
            let side = match aux.get(&fill.ship_id) {
                Some(side) => *side,
                None => continue,
            };
            let ship = match self.ships.get(ship_id) {
                Some(s) => s,
                None => continue,
            };
            if fill.tons_signed > 0.0 {
                let buy_duty = match side.buy_duty {
                    Some(d) => d,
                    None => continue,
                };
                let tons = fill.tons_signed;
                let gross_unit = clearing_price * (1.0 + buy_duty);
                let cost = crate::money::Pesos::from_pesos_f32(tons * gross_unit);
                if cost > ship.silver {
                    continue;
                }
                total_buy_base += tons * clearing_price;
            } else if fill.tons_signed < 0.0 {
                if side.sell_duty.is_none() {
                    continue;
                }
                let requested = -fill.tons_signed;
                let sell_tons = requested.min(ship.cargo.get(good));
                if sell_tons <= 0.0 {
                    continue;
                }
                total_sell_base += sell_tons * clearing_price;
            }
        }
        let available = self.markets[port_idx].silver.as_pesos_f32() + total_buy_base;
        let payout_ratio = if total_sell_base > 0.0 {
            (available / total_sell_base).clamp(0.0, 1.0)
        } else {
            1.0
        };

        let mut total_buy_tons = 0.0_f32;
        let mut applied_tons_signed = 0.0_f32;
        let mut buyer_payments_base = crate::money::Pesos::ZERO;
        let mut actual_payout_base = crate::money::Pesos::ZERO;
        let mut total_buy_duty_pesos = crate::money::Pesos::ZERO;
        let mut total_sell_duty_pesos = crate::money::Pesos::ZERO;
        let port_faction = self.ports[port_idx].faction;

        for fill in &fills {
            let ship_id = match ship_ids.get(&fill.ship_id) {
                Some(id) => *id,
                None => continue,
            };
            let side = match aux.get(&fill.ship_id) {
                Some(side) => *side,
                None => continue,
            };
            let ship = match self.ships.get_mut(ship_id) {
                Some(s) => s,
                None => continue,
            };

            if fill.tons_signed > 0.0 {
                let buy_duty = match side.buy_duty {
                    Some(duty) => duty,
                    None => continue,
                };
                let tons = fill.tons_signed;
                let gross_unit = clearing_price * (1.0 + buy_duty);
                let cost = crate::money::Pesos::from_pesos_f32(tons * gross_unit);
                if cost > ship.silver {
                    continue;
                }
                ship.silver -= cost;
                if side.is_resupply {
                    ship.provisions += tons;
                } else {
                    ship.cargo.add(good, tons);
                }
                let base = crate::money::Pesos::from_pesos_f32(tons * clearing_price);
                let duty_pesos =
                    crate::money::Pesos::from_pesos_f32(tons * clearing_price * buy_duty);
                buyer_payments_base += base;
                total_buy_duty_pesos += duty_pesos;
                total_buy_tons += tons;
                applied_tons_signed += tons;
            } else if fill.tons_signed < 0.0 {
                let sell_duty = match side.sell_duty {
                    Some(duty) => duty,
                    None => continue,
                };
                let requested_tons = -fill.tons_signed;
                let have = ship.cargo.get(good);
                let sell_tons = requested_tons.min(have);
                if sell_tons <= 0.0 {
                    continue;
                }
                ship.cargo.remove(good, sell_tons);
                let net_unit = clearing_price * (1.0 - sell_duty) * payout_ratio;
                let pay = crate::money::Pesos::from_pesos_f32(net_unit * sell_tons);
                ship.silver += pay;
                let duty_pesos = crate::money::Pesos::from_pesos_f32(
                    sell_tons * clearing_price * sell_duty * payout_ratio,
                );
                let payout_base =
                    crate::money::Pesos::from_pesos_f32(sell_tons * clearing_price * payout_ratio);
                total_sell_duty_pesos += duty_pesos;
                actual_payout_base += payout_base;
                applied_tons_signed -= sell_tons;
            }
        }

        // ── Apply to port treasury, bounded balance, and crown ──
        let market = &mut self.markets[port_idx];
        market.silver += buyer_payments_base;
        market.silver = (market.silver - actual_payout_base).max_zero();
        market.crown_silver += total_buy_duty_pesos + total_sell_duty_pesos;

        let applied_delta = -applied_tons_signed;
        let actual_balance = (current_balance as f32 + applied_delta)
            .round()
            .clamp(-bound as f32, bound as f32) as i32;
        market.balance.set(good, actual_balance);

        // ── Telemetry: port + faction aggregates ───────────────────
        let total_duty = total_buy_duty_pesos + total_sell_duty_pesos;
        let production =
            crate::money::Pesos::from_pesos_f32(total_buy_tons.max(0.0) * clearing_price);
        let port_tel = &mut self.port_telemetry[port_idx];
        port_tel.lifetime_duties += total_duty;
        if production.is_positive() {
            port_tel.add_production(good, production);
        }
        let faction_tel = &mut self.faction_telemetry[port_faction as usize];
        faction_tel.crown_revenue += total_duty;
    }

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
                // Phase 4 §3c-1: any landed broadside mutually engages
                // both ships if they are not already locked into a
                // different engagement. First-engaged wins — a third
                // ship that fires later does not pull either party out
                // of the active duel. Skipped if the target sank on
                // this fire (no point engaging a wreck).
                let target_alive = self
                    .ships
                    .get(target_id)
                    .map(|t| t.state != ShipState::Sunk)
                    .unwrap_or(false);
                if target_alive {
                    self.engage(attacker_id, target_id);
                }
            }
        }
    }

    /// Phase 4 §3c-1 (symmetric redesign): mutually flip both ships
    /// into an engagement. No role distinction — both parties make
    /// independent tactical decisions each hour via the BT's engaged
    /// subtree (disengage / pursue+fire / flee+fire / hold).
    ///
    /// First-engaged wins: if either ship already has
    /// `engaged_with == Some(_)` (pointing at any ship, including a
    /// different one), its state is left alone — the existing
    /// engagement keeps priority until it clears.
    ///
    /// Disengage cooldown: a ship within its `disengaged_until_minute`
    /// window is not re-engaged here, so a tactical Disengage
    /// commitment is not undone by the next stray broadside.
    fn engage(&mut self, firer_id: ShipId, victim_id: ShipId) {
        let now = self.sim_minute;
        for (a_id, b_id) in [(firer_id, victim_id), (victim_id, firer_id)] {
            if let Some(a) = self.ships.get_mut(a_id) {
                if a.engaged_with.is_none() && now >= a.disengaged_until_minute {
                    a.engaged_with = Some(b_id);
                    a.engagement_started_at_minute = now;
                }
            }
        }
    }

    /// Phase 4 §3c-1 (symmetric redesign): clear stale engagement
    /// flags. Runs once per hour after the Mutation/Physics Phase, so
    /// positions/rigging/state reflect this hour's outcomes.
    ///
    /// Only one terminal condition is handled here: **counterpart
    /// gone** (sunk or reaped). All other engagement ends — out of
    /// ordnance, escape, lost contact — are now tactical-judgment
    /// decisions made each hour by the BT's engaged subtree, which
    /// emits `Command::Disengage` to mutually clear the flag.
    fn check_engagement_terminations(&mut self) {
        let ids: Vec<ShipId> = self.ships.keys().collect();
        for id in ids {
            let other_id = match self.ships.get(id) {
                Some(s) => match s.engaged_with {
                    Some(o) => o,
                    None => continue,
                },
                None => continue,
            };
            let other_dead = match self.ships.get(other_id) {
                None => true,
                Some(o) => o.state == ShipState::Sunk,
            };
            if other_dead {
                if let Some(s) = self.ships.get_mut(id) {
                    s.engaged_with = None;
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
        let (a_pos, a_vel, a_crew, a_morale, _a_min_crew) = match self.ships.get(attacker_id) {
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
        // Phase 4 §3c-2: prize outcome is shared between boarding-victory
        // and Strike-surrender paths. Delegate to the unified resolver.
        self.resolve_prize_action(attacker_id, tgt);
    }

    /// Phase 4 §3c-2: shared prize-action resolver. Called by
    /// `resolve_boarding` after the attacker wins on deck *and* by the
    /// `Strike` command resolution (surrender without boarding). Decides
    /// take / sell / sink / release using the historical-weighted roll
    /// and applies all state changes (cargo transfer, silver bonus,
    /// crew detachment, flag flip, prize ledger). The caller is
    /// responsible for clearing `engaged_with` on both ships and for
    /// any boarding-specific crew losses *before* invoking this helper.
    #[allow(clippy::too_many_lines)]
    fn resolve_prize_action(&mut self, victor: ShipId, prize: ShipId) {
        let attacker_id = victor;
        let tgt = prize;
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
        let (a_surviving, a_min_crew) = match self.ships.get(attacker_id) {
            Some(a) => {
                let stats = &self.ship_types.get(a.ship_type).stats;
                (a.crew_alive, stats.crew_min())
            }
            None => return,
        };

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
        let cargo_silver = Pesos::from_pesos_f32(target_cargo_tons * 20.0);
        let hull_bounty = Pesos::from_pesos_f32(target_hull * 8.0);

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

        let roll = self.combat_rng.uniform_f32();
        // Outcome weights (sum to 1.0):
        //   take    : 0.05 (only if real upgrade + crew spareable)
        //   sell    : 0.30 (silver bonus, prize sunk)
        //   sink    : 0.50 (default — cargo stripped, hull released/burned)
        //   release : 0.15 (cargo stripped, target lives to trade again)
        let take = could_upgrade && can_spare_crew && roll < 0.05;
        let mut sell = !take && roll < 0.35;
        let release = !take && !sell && roll >= 0.85;
        // (sink is the default if none of the above)

        // §3c-2b: if we picked `sell`, try the tow path first. We need
        // a skeleton crew to sail the prize to port; if the victor
        // can't spare one we fall through to instant-sell behavior.
        let tow_crew = ((a_surviving as f32) * crate::combat::PRIZE_TOW_CREW_SPLIT)
            .round()
            .max(2.0) as u16;
        let attacker_after_tow = a_surviving.saturating_sub(tow_crew);
        let can_spare_tow = attacker_after_tow >= a_min_crew && tow_crew >= 2;
        let sell_tow = sell && can_spare_tow;
        // Instant-sell is the fallback when we picked sell but cannot
        // spare crew for a skeleton.
        let sell_instant = sell && !sell_tow;
        // Keep `sell` true for backward-compatible code paths below
        // that pay silver / sink the prize — those should only run
        // for sell_instant.
        sell = sell_instant;

        // Apply stripped cargo + silver to attacker
        // (and clear from target unless we're taking it).
        if !take && !sell_tow {
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
                t.silver = t.silver.scale(0.1).max_zero(); // boarders took most of it
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
        } else if sell_tow {
            // §3c-2b: prize sails to victor's port as a tow.
            // Detach a skeleton prize crew from the victor, flip
            // policy + faction so allies don't engage her, clear
            // engagement + waypoints + destination (the pre-AI
            // copy-owner pass will populate them from the victor
            // each tick), and stamp `prize_owner`. Cargo + silver
            // stay aboard and settle on arrival.
            self.prizes_in_tow += 1;
            let new_faction = self.ships.get(attacker_id).map(|a| a.faction);
            let (detached, detached_seasoned) = match self.ships.get_mut(attacker_id) {
                Some(a) => {
                    let d = a.detach_prize_crew(tow_crew);
                    a.morale = (a.morale + crate::ship::MORALE_GAIN_PRIZE_TAKEN).clamp(0.0, 1.0);
                    d
                }
                None => (0, 0),
            };
            debug_assert_eq!(
                self.ships.get(attacker_id).map(|a| a.crew_alive),
                Some(attacker_after_tow),
                "detach_prize_crew should leave attacker at attacker_after_tow"
            );
            if let Some(t) = self.ships.get_mut(tgt) {
                t.crew_alive = t.crew_alive.saturating_add(detached);
                t.crew_seasoned = t.crew_seasoned.saturating_add(detached_seasoned);
                t.policy = ShipPolicy::Pirate;
                if let Some(f) = new_faction {
                    t.faction = f;
                }
                t.morale = 0.7;
                t.speed = 0.0;
                t.nav.waypoints.clear();
                t.engaged_with = None;
                t.prize_owner = Some(attacker_id);
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

// Free-function `combat_rng_step` removed; the per-tick combat RNG now
// lives on `World.combat_rng: SimRng` and is called as
// `self.combat_rng.uniform_f32()` even from inside drain loops.
