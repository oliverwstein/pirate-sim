use std::path::Path;

use slotmap::{SecondaryMap, SlotMap};

use crate::ai::ShipAI;
use crate::coastline::{CoastlineMap, LandMesh};
use crate::goods::GoodsRegistry;
use crate::harbor::HarborMap;
use crate::map::MapSystem;
use crate::market::{archetype_for, PortMarket};
use crate::navmesh::Navmesh;
use crate::pathfind::PathfindContext;
use crate::pop::{self, PortDemographics};
use crate::port::{all_ports, Port};
use crate::ship::{Ship, ShipState, ShipStats};
use crate::shiptype::{self, ShipTypeRegistry};
use crate::shipyard::{self, BuildOutcome};
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
}

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
            silver_at_month_start: SecondaryMap::new(),
            last_month_avg_profit: 0.0,
            ships_built: 0,
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

    /// Advance the simulation by one hour.
    pub fn tick(&mut self) {
        let month = self.date.month();
        // PathfindContext uses a single "representative" stats — the
        // sloop's profile — because the navmesh is shared and the
        // wind-routed cost is the same shape for every merchant rig
        // we currently model. A future refinement could maintain a
        // per-type PathfindContext (or a per-type ship_stats lookup
        // inside the planner) without changing the navmesh.
        let pathfind_stats = self.ship_types.get(shiptype::ids::SLOOP).stats.clone();

        // Monthly economic tick: produce outputs, consume inputs at every
        // port. Fired exactly once per month transition. After production
        // settles, compute last month's average per-ship silver delta and
        // let each shipyard port decide whether to commission a new ship.
        if month != self.last_market_month {
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
                if let (BuildOutcome::Built { .. }, Some(ship)) = (outcome, ship) {
                    // New ship docks at home port immediately; the AI's
                    // BUY_BEST tree will pick its first destination on
                    // the first dock-cycle tick. We seed
                    // `nav.docked_at_port = idx` so the dock tree knows
                    // which market to trade with.
                    let mut ai = ShipAI::with_seed(
                        0xA15E_C0FF_u64 ^ (idx as u64) ^ (self.ships_built as u64),
                    );
                    ai.nav.docked_at_port = Some(idx);
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

        let pathfind = PathfindContext::new(
            &self.map.land,
            &self.weather.wind,
            &pathfind_stats,
            month,
            &self.navmesh,
        );

        // Snapshot the live ship ids so we can iterate while mutating
        // both `ships` and `ship_ais`. SlotMap iteration order is not
        // documented as stable; collecting upfront also pins per-tick
        // ordering for determinism.
        let ids: Vec<ShipId> = self.ships.keys().collect();
        for id in ids {
            let ship_stats: ShipStats = {
                let ship = match self.ships.get(id) {
                    Some(s) => s,
                    None => continue, // defensive: ship was removed mid-tick
                };
                self.ship_types.get(ship.ship_type).stats.clone()
            };
            let wind = self.weather.wind.wind_at(self.ships[id].position, month);

            // AI decides heading (or docks/undocks). Two distinct
            // SecondaryMap/SlotMap fields => safe split borrow.
            let ai = match self.ship_ais.get_mut(id) {
                Some(a) => a,
                None => continue,
            };
            let ship = match self.ships.get_mut(id) {
                Some(s) => s,
                None => continue,
            };
            ai.tick(
                ship,
                &ship_stats,
                &wind,
                &self.ports,
                &self.harbors,
                Some(&pathfind),
                Some(&mut self.markets),
                Some(&self.goods),
            );

            // Resource consumption
            ship.tick_resources(&ship_stats);

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

        self.date.advance_hours(1);
    }
}
