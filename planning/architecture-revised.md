# Pirate Sim: System Architecture

## Core Principles

1. **Headless-first:** `sim-core` is a pure library with zero rendering/UI/windowing dependencies. Any program can drive the simulation (visualizer, CLI, test harness, AI trainer).
2. **Modular systems:** The simulation is composed of separable modules that can be added, removed, or replaced independently. The map module provides spatial reality; weather bolts onto it; economy bolts onto that.
3. **Frontend reads state:** The visualizer (sim-viz) borrows `&World` each frame and renders it. It never mutates simulation state. Rendering is a read-only observer.
4. **Cellular Automata AI:** To satisfy Rust's borrow checker and ensure determinism, agents (ships) never mutate the world directly. They read a snapshot of the world and emit *Intents* (Commands). A unified mutation phase resolves these commands into *Consequences* (Events).

---

## Core Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | Performance, safety, data-oriented |
| Entity Management | Generational Indices (`slotmap`) | O(1) lookups, zero `Rc<RefCell>` overhead, safe entity destruction |
| Data model | Classic DOD with Double Buffering | Solves mutable aliasing; allows lock-free AI parallelization |
| Static data format | RON | Rust-native, type-aware, validatable (`goods.ron`, `ship_types.ron`) |
| Time resolution | Continuous RTS with variable system frequencies | Hourly movement, 15m combat, daily trade, monthly economy |
| Spatial model | Continuous 2D coordinates (nautical miles) | Unified for travel + combat |
| AI Pattern | Flyweight Behavior Trees | Zero-allocation decision making; context passed dynamically |
| Settlement model | Production Recipes + Pop Demographics | Abstracted building logic; requires human capital to operate |
| Economic model | Agent-based: local inventory & debt | Emergent prices from agent heuristics, not omniscient markets |
| Currency | Silver is a physical good (`GoodId::Silver`) | Barter with silver as numeraire; money = cargo |

---

## Simulation Loop (The Data Pipeline)

The simulation is continuous. Instead of entities mutating each other mid-loop, the tick is a strict Data-Oriented pipeline.

```text
HOURLY PIPELINE (~1h game time, or ~15m when engaged in combat):
  1. AI Phase (Read-Only)
     - Ships query the Dynamic Spatial Hash for neighbors.
     - Behavior Trees evaluate (flee, pursue, patrol, navigate, trade).
     - BTs emit `ShipCommand`s (e.g., Steer, FireBroadside, Board) to a central queue.
  2. Resolution Phase (Math)
     - Translate inter-ship commands into consequences.
     - E.g., `FireBroadside` + Gunnery Math -> emits `DamageEvent`.
  3. Mutation Phase (Write)
     - Apply Steering: Heading + Speed -> New Position (checked against LandMap).
     - Apply Damage: Decrease hull/rigging, inflict casualties.
     - State changes: Sinking, capturing, docking.
  4. Cleanup Phase
     - Destroy dead entities from the SlotMap.
     - Update the Dynamic Spatial Hash for ships that crossed grid boundaries.

MONTHLY PIPELINE:
  5. Production: Ports consume inputs + labor → produce outputs.
  6. Demographics: Population births, deaths, migration, pirate recruitment.
  7. Market Update: Recalculate local prices from supply/demand & hinterland debt.
  8. Shipyards: Commission new ships if ROI pencils out.
```

---

### Market Model: Debt & Demographics

Each port's market acts as the pricing intermediary and labor pool.

- **Dynamic Pricing:** Prices are set by `base_price * (1.0 + K * (target_stock - current_stock) / target_stock)`.
- **Hinterland Debt:** Ships can buy locally produced goods even when the wharf is empty. This borrows against next month's production, causing prices to spike exponentially (modeling local scarcity).
- **Human Capital:** Ports track populations (`Sailors`, for now, possibly more later). Ships must have crew to leave port. A full complement is not required, but there is an effective minimum crew for each type of ship as well as a maximum crew capacity. If a ship normally would have 10-40 sailors (for a merchant ship), it may need at least 25% of that complement to sail at all, with reduced performance when under-crewed and some improved performance when over-crewed. 

---

## Core Data Types

### Static Data (loaded from RON at startup)

```rust
struct ShipTypeData {
    id: ShipTypeId,
    name: String,
    cargo_capacity_tons: f32,
    crew: u16,                  // standard complement
    speed_typical: f32,         // knots
    speed_max: f32,
    windward_ability: f32,      // 0.0–1.0 (1.0 = perfect upwind)
    build_cost_goods: Vec<(GoodId, f32)>,
    expected_lifetime_months: f32,
}

struct GoodData {
    id: GoodId,
    name: String,
    category: GoodCategory,
    tons_per_unit: f32,
    base_price_pesos: f32,
    europe_price_pesos: f32,    // Exogenous demand sink
}

struct FactionData {
    id: FactionId,
    name: String,
    kind: FactionKind,          // Metropolitan, Colonial, Pirate
    primary_color: (u8, u8, u8),
}
```

### Dynamic World State (The ECS-Lite)

```rust
struct World {
    // === MODULES (composable systems) ===
    pub map: MapSystem,             // LandMap, depth, collisions
    pub weather: WeatherSystem,     // ERA5 WindGrids
    pub navmesh: Navmesh,           // Graph-based open water routing
    
    // === ENTITIES (Generational Arenas) ===
    pub ships: SlotMap<ShipId, Ship>,
    pub ship_ais: SlotMap<ShipId, ShipAI>,
    
    // === STATIC ARRAYS (Indexed by IDs) ===
    pub ports: Vec<Port>,
    pub markets: Vec<PortMarket>,   // Parallel to ports
    pub factions: FactionRegistry,  // Faction data + Relations Matrix
    
    // === SPATIAL ===
    pub spatial_hash: DynamicSpatialHash, // Updated during Mutation Phase
    
    // === EVENT QUEUES (Double Buffering) ===
    pub commands: Vec<ShipCommand>,
    pub damage_events: Vec<DamageEvent>,
    
    pub date: SimDate,
}

// Inter-Agent Intents (Emitted by AI Phase)
enum ShipCommand {
    Steer(Steering),
    FireBroadside { attacker: ShipId, target: ShipId },
    AttemptBoard { attacker: ShipId, target: ShipId },
    StrikeColors(ShipId),
}

// Inter-Agent Consequences (Emitted by Resolution Phase)
struct DamageEvent {
    pub target: ShipId,
    pub hull_damage: f32,
    pub rigging_damage: f32,
    pub crew_casualties: u16,
}
```

### The Ship & Port Entities

```rust
struct Ship {
    // Identity
    id: ShipId,
    ship_type: ShipTypeId,
    faction: FactionId,
    owner_port: Option<usize>,
    
    // Spatial
    position: Vec2,
    heading: f32,
    speed: f32,
    state: ShipState,           // Sailing, Docked, Anchored
    
    // Economics & Status
    silver: f32,
    debt: f32,
    cargo: Cargo,
    provisions: f32,
    
    // Combat Status
    hull_fouling: f32,
    hull_integrity: f32,        // 100.0 to 0.0 (Sinking)
    rigging_integrity: f32,     // Affects effective_speed
    crew_alive: u16,
    morale: f32,                // Drops with damage/debt -> triggers surrender/mutiny
}

struct PortMarket {
    recipe: ProductionRecipe,
    stockpile: Cargo,
    debt: Cargo,                // Hinterland debt (borrowed against future production)
    silver: f32,
    demographics: PortDemographics,
}

struct PortDemographics {
    sailors: u32,
    merchants: u32,
    planters: u32,
    enslaved: u32,
    morale: f32,
}
```

---

## File Structure

```text
pirate-sim/
├── Cargo.toml                   (workspace)
├── crates/
│   ├── sim-core/                (pure library — NO rendering deps)
│   │   ├── Cargo.toml           
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── types.rs         
│   │       ├── world.rs         (Data Pipeline Orchestration)
│   │       ├── map/             (LandMap, Coastline)
│   │       ├── weather/         (ERA5 WindGrid)
│   │       ├── combat/          (Command/Event Resolution)
│   │       ├── ai/              (Flyweight BTs & Contexts)
│   │       ├── economy/         (Goods, Cargo, Market, Trade)
│   │       ├── faction.rs       (Factions & Relations Matrix)
│   │       ├── navmesh.rs       (Graph routing)
│   │       ├── pop.rs           (Demographics)
│   │       └── ship.rs          
│   └── sim-viz/                 (Visualizer — reads &World)
│       └── src/
│           └── main.rs
├── tools/
│   └── preprocess/              (Python: ERA5/GEBCO → binary grids)
├── data/
│   ├── grids/                   (Binary map data)
│   ├── goods.ron                (Phase 2.5: Extracted Static Data)
│   ├── ship_types.ron
│   ├── factions.ron
│   └── ports.ron
└── planning/                    (Documentation & Roadmaps)
```
