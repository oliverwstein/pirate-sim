# Phase 3: Populations, Cellular Automata, and Combat

> **Goal of Phase 3:** Transition the simulation from a system of solitary agents interacting with a static map into a reactive ecosystem where ships interact with *each other*. 
>
> Phase 3 introduces **Human Capital** (populations and crews) and **Organic Combat** driven by a Cellular Automata (CA) architecture. A pirate sees a merchant, gives chase, exchanges broadsides, and boards. The loss of the merchant drains sailors from the global pool, making hiring crews more expensive and causing economic ripples.

## 1. Executive Summary: The State of the Sea

Our current implementation of Phase 2 has successfully established a **Data-Oriented** economic foundation. We have a functioning trade-loop, ROI-driven ship production, and a robust behavior tree (BT) engine. 

However, we are at an architectural inflection point. Until now, ships have been "Solitary Automata"—they interact with the map and markets, but ignore one another. To move into **Phase 3 (Encounters & Piracy)**, we must transition to a **Cellular Automata (CA)** paradigm. This will allow combat and social interaction to emerge organically from tick-by-tick decisions rather than being a "canned" sub-game.

---

## 2. Programming Patterns: Review & Evolution

### 2.1 Current Successes
*   **Headless-First Design:** The separation of `sim-core` and `sim-viz` is pristine. The visualizer is a pure observer of the `&World`, which is the gold standard for simulation stability.
*   **Flyweight Behavior Trees:** The `bt.rs` implementation is highly efficient. By separating `Behavior` (static data) from `BtState` (volatile progress), we are ready to scale to thousands of agents.
*   **The Harbor Zone Pattern:** Using BFS-generated sea-cell masks for ports is a masterstroke of DOD. It solves the "river-navigation" problem without requiring complex steering in narrow waters.

### 2.2 The "Neighborhood" Problem
In Phase 2, a ship's "neighborhood" was its own status (provisions, fouling). In Phase 3, the neighborhood includes other entities. 
*   **The Pattern:** **Spatial Hashing.** We must implement a dynamic spatial hash (grid-based) to allow ships to "sense" nearby ships without an $O(N^2)$ search.
*   **The Pattern:** **Double Buffering (Intents).** To satisfy the Rust borrow checker during inter-ship interaction, we will adopt a "Read-Compute-Write" cycle. AI will *read* the world and *write* a `ShipCommand` to a queue. The `World` will then resolve these commands.

---

## 3. ## Design Philosophy Notes

1.  **DOD Invariant:** During the `AI Tick`, the `ships` array is strictly **Read-Only**. All interactions are deferred to the `Command Resolution` phase. This avoids all mutable borrow conflicts.
2.  **Cellular Automata Purity:** Avoid "global" combat managers. If five ships are firing at each other, they are five independent agents each deciding to fire at a target. The fact that it looks like a "fleet battle" is an emergent property of their individual BTs.
3.  **Headless Performance:** Because we use a spatial hash and a flat command queue, we can eventually parallelize the `AI Tick` using `Rayon`. Each ship's decision is independent of its neighbor's decision *within the same tick*.


## Where we are at the end of Phase 2

- **Economic skeleton:** 9 goods, port markets, production recipes.
- **Navigation:** Full programmatic navmesh and harbor zones are operational (A* routing is complete and lightning fast).
- **AI:** Trader BTs successfully navigate, trade, and refit.
- **The Gap:** Ships ignore each other. We use `Vec<Ship>`, which makes destroying ships unsafe (index shifting). The BT mutates ships directly during its tick, preventing safe inter-ship interaction due to the Rust borrow checker.

## The Thesis

> **Populations, Intent, and Resolution.** Ships are no longer spawned for free; they require human capital (Sailors). Combat is not a hardcoded "sub-game" that halts the simulation. Instead, ships operate as **Cellular Automata**—they read the world, emit an *intent* (a Command), and the world resolves those intents into *consequences* (Events) in a tightly packed, Data-Oriented pipeline.

## What "Sound" Means for Phase 3

The system is sound if all of these hold in a 90-day demo:

1. **Human Capital Constraint:** A ship needs a crew. Just as the ship's hull gets foul, a ship's crew must eat, attrition, and act. They may eat and drink more or less, with effects on ship performance and attrition. In ports, ships can add new sailors.
2. **Organic Chase:** A pirate sloop spots a merchant. Both BTs emit steering commands reacting to each other. The faster ship dictates the range, affected continuously by wind and fouling.
3. **Multi-tick Engagement:** Ships in range emit `FireBroadside` commands. These resolve into `DamageEvent`s applied between ticks. Rigging damage slows the victim; hull damage sinks them. This means ships must have cannons and we must track their supplies of powder and shot.
4. **Prize Crews:** A pirate capturing a ship must split its own crew to sail the prize to a haven. If it lacks the manpower, it must burn the prize instead.
5. **Recruitment:** Bankrupt merchants with plummeting morale strike their colors and turn pirate. 

---

## Phase 2.5: Technical Debt & Prerequisites

Before ships can fight, we must stabilize the data structures to handle creation, destruction, and inter-entity referencing safely.

### 1. Generational Indexing (`slotmap`)
Currently, `world.ships` is a `Vec<Ship>`. Sinking a ship at index `i` invalidates all subsequent indices.
- **Action:** Introduce the `slotmap` crate. `Vec<Ship>` becomes `SlotMap<ShipId, Ship>`. 
- **Result:** Entities reference each other via `ShipId(index, generation)`. If a ship sinks, its ID becomes permanently invalid, preventing "ghost ship" bugs without the overhead of `Rc<RefCell>`.

### 2. Static Data Extraction (RON)
Hardcoded `starter()` lists will become unmaintainable as we add combat stats and faction rules.
- **Action:** Move `GoodsRegistry`, `ShipTypeRegistry`, and `Ports` to `data/*.ron` files using `serde` and `ron`.

### 3. Port Demographics
Ports need populations to crew ships.
- **Action:** Add `PortDemographics` alongside `PortMarket`. Introduce `PopPool` tracking `Sailors`. There may be other kinds of pops later in a port, but for now, sailors will do, along with prosperity. 
- **Action:** Update `shipyard::try_build` to require `stats.crew` number of `Sailors`.

---

## Phase 3 Architecture: The Command/Event Pipeline

To allow ships to fight without fighting the Rust borrow checker, we decouple the AI's *desire* from the world's *physics* using **Double Buffering**. 

### 1. The Data Shapes

```rust
// crates/sim-core/src/faction.rs (NEW)
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct FactionId(pub u8);

// A flattened 1D array for O(1) cache-friendly lookups
pub struct FactionRegistry {
    factions: Vec<FactionData>,
    relations: Vec<Stance>, // Size: NUM_FACTIONS * NUM_FACTIONS
}
```

```rust
// crates/sim-core/src/combat_events.rs (NEW)
pub enum ShipCommand {
    Steer(Steering),
    FireBroadside(ShipId), // Intent to fire at target
    AttemptBoard(ShipId),  // Intent to melee
    StrikeColors,          // Intent to surrender
}

pub struct DamageEvent {
    pub target: ShipId,
    pub hull_damage: f32,
    pub rigging_damage: f32,
    pub crew_killed: u16,
}
```

### 2. The Cellular Automata BT Context

The Behavior Tree is now **strictly read-only** regarding the world.

```rust
pub struct ShipBtContext<'a> {
    pub me: ShipId,
    pub my_ship: &'a Ship,
    pub stats: &'a ShipStats,
    pub wind: &'a WindVector,
    
    // THE SENSOR HOOKS
    pub spatial_hash: &'a SpatialHash, 
    pub world_ships: &'a SlotMap<ShipId, Ship>, // Read-only access to all ships
    pub factions: &'a FactionRegistry,

    // THE OUTPUT BUFFER
    pub commands: &'a mut Vec<ShipCommand>, 
}
```

### 3. The New Simulation Loop Orchestration

The `World::tick` method becomes a pristine data-transformation pipeline.

```rust
fn tick(&mut self) {
    self.commands.clear();
    self.damage_events.clear();

    // 1. AI PHASE (Read-only CA evaluation)
    // Ships observe the world and push to `self.commands`
    for (id, ship) in &self.ships {
        self.ship_ais[id].tick(..., &self.ships, &mut self.commands);
    }

    // 2. COMBAT RESOLUTION PHASE
    // Turn intents into mathematical consequences
    for cmd in &self.commands {
        match cmd {
            ShipCommand::FireBroadside(target) => {
                let dmg = calculate_gunnery(&self.ships[me], &self.ships[target]);
                self.damage_events.push(dmg);
            }
            // ... resolve boarding, etc.
        }
    }

    // 3. MUTATION PHASE (The T+1 Step)
    // The only place ships are actually mutated.
    for cmd in &self.commands { apply_steering(cmd); }
    for event in &self.damage_events { apply_damage(event); }
    
    // 4. CLEANUP PHASE
    // Remove dead ships, update spatial hash
    self.reap_sunk_ships();
    self.spatial_hash.update(&self.ships);
}
```

---

## Behavioral Additions

### The Pirate BT Variant
Because the BTs are modular, a Pirate is just a ship assigned the `Predator` BT.
```text
Selector:
  Sequence: [IsDocked, SellLoot, Resupply, RecruitCrew, Undock]
  Sequence: [IsMoraleBroken, Flee]
  Sequence: [InMeleeRange, AttemptBoard]
  Sequence: [InCannonRange, FireBroadside, CloseDistance]
  Sequence: [SeePrey, Pursue]
  Sequence: [NeedsHavenRun, NavigateToHaven]
  Sequence: [PickLane, Patrol]
```

### Proximity & The Dynamic Spatial Hash
We cannot do $O(N^2)$ distance checks to evaluate `SeePrey`.
- We will add a **Dynamic Spatial Hash** (e.g., 10x10 NM grid).
- A ship updates its cell in the hash *only* when it crosses a grid boundary during the Mutation Phase. 
- During the AI Phase, `SeePrey` simply queries the ship's current bucket and adjacent buckets.

---

## File-Level Diff Outline

```text
crates/sim-core/src/
├── faction.rs          NEW (FactionId, RelationsMatrix)
├── combat.rs           NEW (Command resolution, DamageEvent math)
├── spatial.rs          NEW (Dynamic Ship Spatial Hash)
├── pop.rs              NEW (PortDemographics, PopPool)
├── ai.rs               → Refactored to read-only BT context, Command Queues
├── ship.rs             + hull_integrity, rigging_integrity, crew, morale
├── world.rs            → Pipelined tick architecture
└── data.rs             → RON loaders integration

data/
├── factions.ron        NEW
├── goods.ron           NEW (Extracted from hardcode)
├── ship_types.ron      NEW (Extracted from hardcode)
└── ports.ron           + Pop demographics & Faction assignments
```

---

## Implementation Sequence (Each step ships green)

1. **Phase 2.5: Generational Indices & RON.** Swap `Vec<Ship>` for `SlotMap`. Extract `GoodsRegistry` and `ShipTypeRegistry` into RON files. *Bench unchanged.*
2. **Phase 2.5: Populations.** Add `PortDemographics`. Shipyards now subtract `Sailor` pops when launching ships.
3. **Factions & Spatial Hash.** Implement `FactionRegistry` and the Dynamic Spatial Hash. Viz draws faint lines between ships of different factions in visual range.
4. **The Pipeline Refactor (Double Buffering).** Rewrite `World::tick` into the `AI Phase -> Resolution -> Mutation` pipeline. Introduce `ShipCommand::Steer`. *Behavior is identical, but architecture is now CA-ready.*
5. **The Chase (Maneuver AI).** Add `Pursue` and `Flee` BT nodes. Spawn a pirate. Watch it chase merchants based on wind speed until it catches them or they reach port.
6. **Gunnery & Damage Events.** Add `FireBroadside` commands. Implement hull/rigging damage. Rigging damage lowers `effective_speed`. Ships can now be battered to a halt.
7. **Boarding & Sinking.** Add `AttemptBoard`. Introduce `Captured` and `Sinking` states. Dead ships are safely removed from the `SlotMap`. Captured ships drain the pirate's crew to create a prize crew.
8. **Bankruptcy to Piracy.** Wire `ship.debt` and `ship.morale` to a mutiny trigger. The pirate fleet is now organically sustained by the economic pressures of Phase 2.
9. **Calibration Pass.** Run a 1-year simulation. Tune gunnery lethality, base population growth, and morale thresholds until the Caribbean maintains a stable equilibrium of merchants, pirates, and sunken wrecks.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| **Event Queue Bloat:** Millions of commands per tick. | Ships only emit commands. Steering is 1 per ship. Combat commands only fire when in range. Negligible memory overhead compared to pathfinding. |
| **BT Synchronization:** A ship fires at a target that sank *this tick*. | Safe because `SlotMap` IDs are generational. `resolve_combat` checks if `target` ID is still valid before applying damage. |
| **Prize Crew Math:** Pirates capture ships but have 0 crew left. | BT `AttemptBoard` logic will require a minimum crew threshold. If below threshold, they emit `BurnShip` instead of `Capture`. |
| **Sailor Starvation:** Pirates kill too many sailors; global trade halts. | Calibrate monthly `Pop` growth and migration rates. A sailor shortage naturally reduces the number of merchant targets, starving the pirates and resetting the cycle. |
