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

1. **Human Capital Constraint:** A ship needs a crew. Crew is a first-class
   property of the ship (`Ship.crew_alive: u16`), like the hull — *not* a
   cargo good, not consumed at the shipyard. Crews are hired from a port's
   sailor pool on launch and discharged back into it on docking. Crew size
   drives provisions burn rate and `effective_speed` (under- and over-crew
   curves). In ports, ships can hire to replace casualties.
2. **Organic Chase:** A pirate sloop spots a merchant. Both BTs emit steering commands reacting to each other. The faster ship dictates the range, affected continuously by wind and fouling.
3. **Multi-tick Engagement:** Ships in range emit `FireBroadside` commands. These resolve into `DamageEvent`s applied between ticks. Rigging damage slows the victim; hull damage sinks them. This means ships must have cannons and we must track their supplies of powder and shot.
4. **Prize Crews:** A pirate capturing a ship must split its own crew to sail the prize to a haven. If it lacks the manpower, it must burn the prize instead.
5. **Recruitment & Mutiny:** Bankrupt merchants with plummeting morale strike their colors and turn pirate — `ShipPolicy::Pirate` is a state of mind, not a faction. A privateer with a Letter of Marque against Spain is a different state again. 

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

### 3. Port Demographics (two-tier sailor pool)
Ports need populations to crew ships. Calibrated against
`planning/research/sailor-recruitment.md`.

- **Action:** Add `PortDemographics` alongside `PortMarket`:
  ```rust
  struct PortDemographics {
      seasoned: u32,            // low mortality, full performance
      unseasoned: u32,          // high mortality, maturing into seasoned
      port_category: PortCategory,  // EuropeanHub | CaribbeanEntrepot | SmallColonial | PirateHaven
      // monthly_growth_rate, mortality_rate derived from port_category
  }
  ```
- **Two tiers, not one.** Unseasoned sailors die at 1–2%/month from
  "seasoning" disease; a small fraction (~3%/month) matures to seasoned.
  Seasoned sailors die at ~0.5%/month. This naturally caps pool sizes.
- **Tiered organic monthly growth** (from research §7.1):
  - European hub: ~50–170/month
  - Caribbean entrepot: ~2–5/month
  - Small colonial: ~0.5–1.5/month
  - Pirate haven: ~0 (negative without ship arrivals)
- **Transient supply:** each ship arrival adds 1–8 sailors to the
  unseasoned pool, scaled by ship size.
- **Hiring:** ship launch / refill draws from seasoned first, then
  unseasoned. Faction-specific fill rates (research §7.3) — English
  fastest, Spanish slowest. `shipyard::try_build` requires
  `stats.crew_min` available sailors at the launching port.
- **Post-war demobilization shocks:** deferred to Phase 4 (no wars yet).

---

## Phase 3 Architecture: The Command/Event Pipeline

To allow ships to fight without fighting the Rust borrow checker, we decouple the AI's *desire* from the world's *physics* using **Double Buffering**. 

### 1. The Data Shapes

```rust
// crates/sim-core/src/faction.rs (NEW)
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct FactionId(pub u8);

// Five factions for v1: Spain, England, France, Netherlands, Free.
// "Pirate" is NOT a faction — it's a per-ship policy (see ShipPolicy).
pub struct FactionRegistry {
    factions: Vec<FactionData>,
    relations: Vec<Stance>, // Size: 5 * 5 = 25
}

// Per-ship policy / "state of mind". Layered on top of faction.
// A French-flagged ship with ShipPolicy::Privateer{against:[Spain]} is
// hostile to Spain but treated as a friendly French merchant by England.
pub enum ShipPolicy {
    Merchant,
    Privateer { against: FactionSet },  // bitfield over 5 factions
    Pirate,                              // hostile to all flagged factions
    Navy,                                // hostile per FactionRegistry relations
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

> **Sequencing philosophy:** plumbing-first. Steps 1–5 are mostly invisible
> refactors that get the architecture right; the chase becomes visible at
> Step 6. The combat tick is hourly throughout (no sub-tick). See
> `planning/development-log.md` 2026-05-22 for the decision record.

**Progress (as of 2026-05-23):**
- ✅ Step 1 — SlotMap migration (commit `05e62b6`)
- ✅ Step 2 — RON extraction: goods + ship types (`b3dc793`), ports (`ce0219e`)
- ⏭ Step 3 — Port demographics + crew on ships *(next)*

1. **Generational Indices (SlotMap).** Swap `Vec<Ship>` and `Vec<ShipAI>` for `SlotMap<ShipId, _>`. Update internal indexers, viz selection, bench_trade. *Bench + tests unchanged.*
2. **RON extraction.** Move `GoodsRegistry`, `ShipTypeRegistry`, and the port list into `data/*.ron` via `serde + ron`. Reserve a slot for `factions.ron`. *Bench + tests unchanged.* **Loading order precedent:** registries with no dependencies first; downstream registries take upstream as `&` and resolve named references at load (e.g. port shipyard names → `ShipTypeId`). Apply the same pattern to factions in Step 4.
3. **Port demographics + crew on ships.** Add the two-tier `PortDemographics` per the section above. Add `Ship.crew_alive: u16`, hired at launch, discharged at dock, lost to attrition at sea. Monthly tick: pool growth + mortality + maturation. Provisions burn rate scales with `crew_alive`; `effective_speed` gets an under/over-crew curve. `shipyard::try_build` requires `stats.crew_min` sailors. `bench_trade` prints crew + pool columns.
4. **Factions & Dynamic Spatial Hash.** Five factions (Spain, England, France, Netherlands, Free); Relations Matrix; faction colors. `Ship.faction: FactionId`; `Port.faction: FactionId`. Dynamic spatial hash (10 NM cells). Viz draws ships in faction colors and faint sight-lines between ships of differing factions within visual range. **Includes renaming `Faction::Holland` → `Faction::Netherlands`** (kept under the old name through Steps 1–3 for back-compat with the existing enum).
5. **The Pipeline Refactor (Double Buffering).** Rewrite `World::tick` into the `AI Phase → Resolution → Mutation → Cleanup` pipeline. Introduce `ShipCommand::Steer(Steering)` as the only command initially. `ShipBtContext` becomes strictly read-only re ships. *Behavior identical; bench + tests unchanged.*
6. **The Chase (Maneuver AI).** Add `Ship.policy: ShipPolicy`. Add `Pursue` and `Flee` BT nodes. `SeePrey` condition consults spatial hash + relations + policy. Hardcoded scenario: spawn a pirate sloop near Tortuga. First visible Phase 3 behavior.
7. **Gunnery & Damage Events.** Add `FireBroadside { attacker, target }` commands when within cannon range (~0.25 NM = 500 yd). Resolution emits `DamageEvent { hull, rigging, crew_killed }`. Hull and rigging integrity on `Ship`; rigging damage cuts `effective_speed`. Powder and shot are new Goods consumed per broadside. Ships can now be battered to a halt.
8. **Boarding & Sinking.** Add `AttemptBoard { attacker, target }` when within ~0.05 NM and victim's rigging is sufficiently damaged. Deterministic single-tick resolution: `crew_alive * (1 + morale_bonus)` per side; larger force wins; proportional losses. Winner takes the prize (drains their crew into a prize crew) or burns it if below threshold. Sunk ships are reaped in Cleanup.
9. **Bankruptcy → Piracy.** Add `Ship.morale: f32`. Drops with debt, low wages, damage; rises with prize money. Mutiny trigger: `debt > MAX_SHIP_DEBT * 1.5 && morale < 0.25 && at_sea` → ship.policy becomes `Pirate`. Closes the Phase 2 economic loop into Phase 3 violence.
10. **Calibration Pass.** Run a 1-year headless simulation. Tune gunnery lethality, sailor-pool growth/mortality (calibrated against research §7.1, §7.5), mutiny threshold, and pirate spawn rates until the Caribbean maintains a stable equilibrium of merchants, pirates, prizes, and wrecks. `bench_trade` reports active pirates, sunken ships, captured prizes, and per-port sailor pool totals.

## Explicitly OUT of Phase 3 (deferred to Phase 4+)

- **Rhai trade-law / Navigation Acts hook** — comes once factions matter for trade flow.
- **Multi-tick boarding with morale rolls** — deterministic single-tick is v1.
- **Sub-tick (15-min) combat resolution** — hourly is v1.
- **Post-war demobilization shocks** to the sailor pool — no wars yet.
- **Officer / specialist crew roles** (master, gunner, surgeon) — single `crew_alive` count for v1.
- **Letters of Marque as data structures** with expiry and issuance rules — `ShipPolicy::Privateer{against}` is just an enum for now.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| **Event Queue Bloat:** Millions of commands per tick. | Ships only emit commands. Steering is 1 per ship. Combat commands only fire when in range. Negligible memory overhead compared to pathfinding. |
| **BT Synchronization:** A ship fires at a target that sank *this tick*. | Safe because `SlotMap` IDs are generational. `resolve_combat` checks if `target` ID is still valid before applying damage. |
| **Prize Crew Math:** Pirates capture ships but have 0 crew left. | BT `AttemptBoard` logic will require a minimum crew threshold. If below threshold, they emit `BurnShip` instead of `Capture`. |
| **Sailor Starvation:** Pirates kill too many sailors; global trade halts. | Calibrate monthly `Pop` growth and migration rates. A sailor shortage naturally reduces the number of merchant targets, starving the pirates and resetting the cycle. |
