# Pirate Sim: System Architecture (Draft)

## Note: this is a pre-code document, which may become outdated with time. Note when that happens. 

## Core Principles

1. **Headless-first:** `sim-core` is a pure library with zero rendering/UI/windowing dependencies. Any program can drive the simulation (visualizer, CLI, test harness, AI trainer).
2. **Modular systems:** The simulation is composed of separable modules that can be added, removed, or replaced independently. The map module provides spatial reality; weather bolts onto it; economy bolts onto that.
3. **Frontend reads state:** The visualizer (sim-viz) borrows `&World` each frame and renders it. It never mutates simulation state. Rendering is a read-only observer.

---

## Core Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | Performance, safety, data-oriented |
| Data model | Classic DOD with modifier system | Clear types, DSL-friendly, Vic3-inspired |
| Static data format | RON | Rust-native, type-aware, validatable |
| Scripting/rules | Rhai | Pure Rust, sandboxed, game-friendly |
| Time resolution | Continuous RTS with variable system frequencies | Hourly movement, daily trade, monthly economy |
| Spatial model | Continuous 2D coordinates (nautical miles) | Unified for travel + combat |
| Zone overlays | ~22 sea zones + land provinces | Wind, weather, ownership, patrol intensity |
| Ship behavior | Independent automata with behavior trees | Emergent fleet dynamics, organic navigation |
| Fleet model | Intent-based grouping (ships try to stay together) | Realistic scattering, convoy stragglers |
| Settlement model | Vic3-style: buildings with levels, employing pops | Build-up gameplay, physical logistics |
| Economic model | Agent-based: merchants as pricing intermediaries, local inventory | Emergent prices from agent heuristics, not omniscient markets |
| Currency | Silver is a physical good (GoodId::Silver), not an abstraction | Barter with silver as numeraire; money = cargo |

---

## Simulation Loop (Variable-Rate RTS)

The simulation is continuous. Different systems update at different frequencies:

```
HOURLY (~1h game time):
  1. Ship movement: heading + speed → new position (wind from ERA5 grid)
  2. Land collision check (from GEBCO land mask)
  3. Spatial hash update
  4. Proximity detection: ship sees another ship → may trigger AI

EVERY ~4 HOURS (or event-triggered):
  4. Ship AI re-evaluation: sense → decide → act
     - Query nearby ships (spatial hash within visual range)
     - Evaluate behavior tree (flee, pursue, patrol, navigate, trade)
     - Set new heading/speed

COMBAT (~15 min intervals, only when engaged):
  5. Range calculation, maneuver, firing, boarding
     - Same coordinate space as movement
     - Weather gauge from wind + relative position

DAILY:
  6. Port interactions: ships at port load/unload cargo
  7. Trade transactions: goods exchanged at local prices
  8. Storm checks: random events per zone (hurricane season)
  9. Ship maintenance: crew eats provisions from cargo

MONTHLY:
  10. Production: buildings consume inputs + labor → produce outputs
  11. Population: births, deaths, migration, slave trade arrivals
  12. Consumption: pops consume goods from settlement warehouse
  13. Market update: recalculate local prices from supply/demand
  14. Construction: advance building projects
  15. Exogenous events: wars, policy changes (Rhai-driven)
  16. AI faction decisions: build, recruit, deploy fleets, set policy
```

### Market Model: Merchant as Intermediary

Each port's market IS its resident merchant(s) — not an abstract price oracle:

```
[Industry: Plantations, Shipyards, Cooperages, etc.]
        ↕ sells output to merchant (sugar, rum, tobacco, ships)
        ↕ buys inputs from merchant (provisions, tools, copper, timber)
[Port Merchant — holds warehouse, sets buy/sell prices for BOTH directions]
        ↕ buys exports from industry, sells to visiting ships
        ↕ buys imports from ships, sells to local industry
[Ship Captain — trades with merchant, loads/unloads cargo]
```

- "Industry" = the buildings in our model. Each building level produces AND consumes.
- Merchant responds to both local supply (what industry produces) and local demand (what industry needs)
- Pricing reflects scarcity on both sides: provisions expensive if scarce, sugar cheap if oversupplied
- Merchant buys from industry at one price, sells to ships at another (and vice versa)
- Margins are fat (30-50% spread) because transport loss rates are 20-30%
- MVP: merchant is a pricing function on the settlement; later: full agent with personality
- Multiple merchants at large ports → competition → tighter spreads
- Single merchant at small port → monopoly → exploitative pricing
- No merchant at pirate haven → direct barter at whatever terms

---

### Money as Physical Commodity

Silver (pesos / pieces of eight) is `GoodId::Silver` — a physical good with:
- Weight: 0.056 lbs per peso (27g coin)
- Perishability: none
- Value-to-weight: ~440 (highest of any good)
- Universal acceptance across all factions

Trade is fundamentally barter denominated in silver equivalents:
- Most transactions swap goods directly (sugar ↔ provisions)
- Silver changes hands only when trade is unbalanced
- A settlement's "treasury" is literally stored silver coins
- A pirate's loot is physical cargo (silver, sugar, textiles, etc.)
- Price of any good = how much silver it trades for at this port

---

## Core Data Types

### Static Data (loaded from RON at startup)

```rust
// Ship type template (14 types from research)
struct ShipTypeData {
    id: ShipTypeId,
    name: String,
    tonnage_range: (u16, u16),
    crew_min: u16,
    crew_typical: u16,
    crew_max: u16,
    cargo_capacity: u16,        // tons
    guns_range: (u8, u8),
    broadside_weight: f32,      // lbs
    speed_typical: f32,         // knots
    speed_max: f32,
    windward_ability: f32,      // 0.0–1.0 (1.0 = perfect upwind)
    draft: f32,                 // feet
    build_cost_goods: Vec<(GoodId, f32)>,
    build_time_months: u8,
    monthly_maintenance: Vec<(GoodId, f32)>,
    careening_interval_months: u8,
    primary_use: ShipRole,
}

// Good definition (23 goods from research)
struct GoodData {
    id: GoodId,
    name: String,
    category: GoodCategory,
    unit_name: String,          // "hogshead", "barrel", etc.
    unit_weight_lbs: f32,
    perishability: Perishability,
    strategic_class: StrategicClass,
    base_world_price: f32,      // reference price in pesos
}

// Building type definition
struct BuildingTypeData {
    id: BuildingTypeId,
    name: String,
    // Production per level per month
    outputs: Vec<(GoodId, f32)>,
    inputs: Vec<(GoodId, f32)>,
    // Labor
    fixed_labor: Vec<(PopType, u16)>,    // must have (overseer, etc.)
    scalable_labor: Vec<(PopType, u16)>, // more = more output
    // Construction
    construction_goods: Vec<(GoodId, f32)>,
    construction_months: u8,
    // Maintenance (monthly, per level)
    maintenance_goods: Vec<(GoodId, f32)>,
}

// Sea zone overlay (game mechanics — NOT the wind/current source)
struct SeaZoneData {
    id: SeaZoneId,
    name: String,
    bounds: Polygon,            // approximate boundary
    owner: Option<FactionId>,   // territorial waters
    patrol_intensity: f32,      // 0.0–1.0
    storm_risk: f32,           // probability per month in hurricane season
    strategic_notes: String,
}
```

### Dynamic World State

```rust
struct World {
    // === MODULES (composable systems) ===
    
    // Map module: spatial reality (land, depth, coordinates)
    map: MapSystem,
    
    // Weather module: wind + currents (bolts onto map)
    weather: WeatherSystem,
    
    // === GAME STATE ===
    
    // Entities
    ships: Vec<Ship>,
    settlements: Vec<Settlement>,
    factions: Vec<Faction>,
    
    // Spatial index (updated from map positions)
    spatial_hash: SpatialHash,
    
    // Game mechanics overlay
    sea_zones: Vec<SeaZone>,
    
    // Time
    date: SimDate,
    
    // Global state
    world_prices: HashMap<GoodId, f32>,  // exogenous European prices
    active_wars: Vec<War>,
    active_treaties: Vec<Treaty>,
    
    // Event system
    event_queue: Vec<ScheduledEvent>,
}

/// The map module: spatial foundation everything else builds on
struct MapSystem {
    land_map: LandMap,          // from GEBCO: is_land(), depth_at()
    // Future: coastline polygons, port approach lanes, etc.
}

/// Weather module: atmospheric/ocean conditions (plugs into map)
struct WeatherSystem {
    wind_grid: WindGrid,        // from ERA5: wind_at(pos, month)
    current_grid: CurrentGrid,  // from OSCAR: current_at(pos)
    // Future: storm events, seasonal variation noise
}
```

The tick orchestration queries modules in sequence:
```rust
impl World {
    fn tick(&mut self) {
        for ship in &mut self.ships {
            // 1. Query weather for conditions at ship's position
            let wind = self.weather.wind_grid.wind_at(ship.position, self.date.month());
            let current = self.weather.current_grid.current_at(ship.position);
            
            // 2. Ship computes desired movement
            let ship_type = &self.ship_types[ship.ship_type.0 as usize];
            let new_pos = ship.compute_movement(ship_type, &wind, &current, 1.0);
            
            // 3. Map validates (collision check)
            if !self.map.land_map.is_land(new_pos) {
                ship.apply_position(new_pos);
            }
        }
        self.date.advance_hours(1);
    }
}
```

struct Ship {
    id: ShipId,
    ship_type: ShipTypeId,
    name: String,
    faction: FactionId,
    
    // Spatial
    position: Vec2,
    heading: f32,
    speed: f32,
    
    // Grouping
    fleet: Option<FleetId>,
    
    // State
    crew: CrewState,
    cargo: Vec<CargoSlot>,
    guns: u8,
    condition: f32,
    
    // AI
    orders: ShipOrders,
    behavior: BehaviorTreeId,
    
    modifiers: Vec<ShipModifier>,
}

struct Settlement {
    id: SettlementId,
    name: String,
    position: Vec2,
    faction: FactionId,
    province: ProvinceId,
    
    // Physical state
    buildings: Vec<Building>,
    pops: Vec<Pop>,
    warehouse: HashMap<GoodId, f32>,
    
    // Infrastructure
    harbor: HarborStats,
    fortification_level: u8,
    
    // Market
    local_prices: HashMap<GoodId, f32>,
    
    modifiers: Vec<SettlementModifier>,
}

struct Building {
    building_type: BuildingTypeId,
    level: u8,
    under_construction: Option<ConstructionProgress>,
}

struct Pop {
    pop_type: PopType,          // Enslaved, FreePlanter, Merchant, Soldier, Sailor, Artisan
    count: u32,
    origin: Origin,             // African, English, Spanish, French, Dutch, Indigenous
    needs_satisfaction: f32,    // 0.0–1.0; affects productivity
}

struct Fleet {
    id: FleetId,
    name: String,
    commander: ShipId,
    members: Vec<ShipId>,
    orders: FleetOrders,
}

struct Faction {
    id: FactionId,
    name: String,
    faction_type: FactionType,  // Metropolitan, Colonial, Pirate, Indigenous
    
    // Relationships
    relations: HashMap<FactionId, i16>,  // -100 to +100
    at_war_with: Vec<FactionId>,
    
    // Policy (for trade rules)
    trade_policy: TradePolicyId,  // references Rhai rule set
    
    // Resources (for metropolitan powers — exogenous)
    annual_military_budget: f32,
    available_reinforcements: u32,
}
```

---

## Trade Rules System (Rhai)

Trade legality is evaluated per-shipment by calling Rhai functions:

```rhai
// trade_rules/england.rhai
fn is_legal_trade(ship, good, origin, destination, date) {
    // Navigation Acts 1660
    if date >= 1660 {
        // Ship must be English-owned with 75% English crew
        if ship.faction != "England" { return deny("foreign_ship"); }
        if ship.crew_english_ratio < 0.75 { return deny("crew_requirement"); }
        
        // Enumerated goods must go to England
        if good in ENUMERATED_GOODS && destination.faction != "England" {
            return deny("enumerated_good");
        }
    }
    
    // Staple Act 1663
    if date >= 1663 && origin.region == "Europe" && origin.faction != "England" {
        return deny("staple_act");
    }
    
    allow()
}

fn calculate_duty(good, quantity, origin, destination, date) {
    if good == "sugar" { return quantity * 0.05; }  // plantation duty
    if good == "tobacco" { return quantity * 0.01; }
    0.0
}
```

---

## File Structure (planned)

```
pirate-sim/
├── Cargo.toml                   (workspace)
├── crates/
│   ├── sim-core/                (pure library — NO rendering deps)
│   │   ├── Cargo.toml           (ron, serde, glam, bytemuck)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs         (IDs, Position, newtypes)
│   │       ├── world.rs         (World struct, tick orchestration)
│   │       ├── map/
│   │       │   ├── mod.rs       (MapSystem)
│   │       │   └── land.rs      (LandMap: is_land, depth_at)
│   │       ├── weather/
│   │       │   ├── mod.rs       (WeatherSystem)
│   │       │   ├── wind.rs      (WindGrid: wind_at)
│   │       │   └── current.rs   (CurrentGrid: current_at)
│   │       ├── ship.rs          (Ship struct, movement physics)
│   │       ├── settlement.rs    (Settlement, Building, Pop)
│   │       ├── economy.rs       (market, merchant, pricing)
│   │       ├── combat.rs        (engagement resolution)
│   │       └── data.rs          (RON loaders)
│   └── sim-viz/                 (optional visualizer — reads &World)
│       ├── Cargo.toml           (macroquad, sim-core)
│       └── src/
│           └── main.rs
├── tools/
│   └── preprocess/              (Python: ERA5/GEBCO → binary grids)
│       ├── requirements.txt
│       ├── preprocess_land.py
│       ├── preprocess_wind.py
│       └── README.md
├── data/
│   ├── grids/                   (binary map data — from preprocessing)
│   │   ├── land_mask.bin
│   │   └── wind_grid.bin
│   ├── goods.ron
│   ├── ship_types.ron
│   ├── building_types.ron
│   ├── settlements.ron
│   ├── sea_zones.ron
│   └── factions.ron
├── scripts/                     (Rhai rules — future)
│   ├── trade_rules/
│   ├── events/
│   └── ai/
└── planning/                    (research + architecture docs)
```

---

## MVP Scope

The minimum playable simulation needs:
1. ✅ Goods system (23 goods with properties)
2. ✅ Ship types (14 types with stats)
3. ✅ Map (real-world grids: GEBCO land, ERA5 wind; zones for game mechanics only)
4. ✅ Ship movement (daily tick, wind-affected)
5. ✅ Settlement production (monthly, inputs → outputs)
6. ✅ Local markets (supply/demand pricing)
7. ✅ Trade rules (Rhai evaluation of legality + duties)
8. ✅ Ship AI (basic: merchant trades, pirate hunts, navy patrols)
9. ✅ Basic combat resolution (when ships meet)

Everything else (diplomacy, population growth, fortifications, events) layers on top.
