### 1. Architectural Review & Programming Patterns

The simulation is built on a solid foundation of Game Programming Patterns (Nystrom) and Data-Oriented Design (Fabian). 

**The ECS-Lite and Generational Indices**
Using `slotmap` for `World::ships` and `SecondaryMap` for `ship_ais` and `silver_at_month_start` is textbook Data-Oriented Design. By decoupling the static data (the hull's physics and cargo) from the volatile data (the AI's state and goal), you achieve excellent memory locality and safely bypass the ABA problem (stale pointers to dead ships). The Cleanup Phase at the end of the tick gracefully reaps sunk ships without invalidating indices.

**The Flyweight Behavior Tree**
Your `bt.rs` implementation is gorgeous. By defining `Behavior` as pure, stateless data (enums with indices rather than boxed closures) and passing a mutable `BtState` cursor alongside it, you've created a classic Flyweight pattern. Thousands of ships can share the same memory footprint for the tree definition, tracking only their path through it. 

**The Dynamic Spatial Hash**
`spatial.rs` operates on a 10 NM grid, rebuilt completely every hour over `Sailing` ships. Fabian would approve: instead of writing complex, bug-prone logic to track when a ship crosses a cell boundary to update its bucket, you just clear and rebuild the hash. At the scale of hundreds (or even low thousands) of ships, iterating a flat array and bucketing by coordinates is computationally cheaper than managing delta updates.

**The Command Pattern (and its Fatal Flaw)**
Your `architecture-revised.md` and `phase-3-plan.md` make a bold claim:
> *DOD Invariant: During the AI Tick, the ships array is strictly Read-Only... The World will then resolve these commands. (Read-Compute-Write)*

This is a beautiful Cellular Automata model. **However, your code betrays your plan.** 
Look at `World::tick_hourly_ai_and_physics` (around line 430 in `world.rs`):
```rust
for id in ids {
    // 1. Tick the AI, which pushes to self.commands
    ai.tick(&mut inputs);
    
    // 2. IMMEDIATELY drain the commands and mutate the world
    for (target, cmd) in self.commands.drain(..) {
        // ... applies Steering, FireBroadside, AttemptBoard to self.ships
    }
}
```
You are draining the command buffer *inside* the ship iteration loop. This means the simulation is **not double-buffered**, and it is **not a Cellular Automaton**. 

Because mutations happen mid-loop, iteration order dictates combat outcomes. If Ship A and Ship B are in range, and `ids` lists Ship A first, Ship A fires and damages Ship B. When Ship B's turn comes milliseconds later, it evaluates its BT using its *already damaged* state (rigging down, crew dead). If Ship B had been listed first, the inverse would happen. 

**The Fix:** To fulfill your design document's promise and make the simulation order-independent (and ready for Rayon parallelization), you must lift the `self.commands.drain(..)` loop *outside* and *after* the `for id in ids` loop.

---

### 2. How the Elements Enlace: Mechanics vs. Plans

**Demographics and Crewing**
Your `crewing-plan.md` envisioned a rich model of seasoned vs. unseasoned sailors. The port-level demographics (`pop.rs`) correctly execute this with organic growth, maturation, and tropical mortality. 
*Alignment Gap:* The plan called for `Ship.crew_seasoned: u16` alongside `crew_alive` to affect combat modifiers. In `ship.rs`, you currently only track `crew_alive`. When hiring, you deplete seasoned sailors from the port first, but they become generic "crew" on the ship. This is a sensible v1 abstraction, but means veteran pirate crews currently fight with the same mathematical skill as freshly-pressed merchant landsmen (offset only by the `morale` multiplier).

**Navigation and Dead Reckoning**
The implementation of the captain's "belief state" (`NavGoal.estimated_position`) versus the ship's physical `truth` is a masterpiece of simulation design. 
By calculating routing, leeway, and visibility against the *estimate*, but checking harbor zone arrival and land collisions against the *truth*, you perfectly capture the terror of 17th-century navigation. The fact that the simulation surfaced a bug where ships "teleported" into ports because their estimate drifted there (noted in `development-log.md`) proves the systems are interacting with deep, emergent complexity.

**Combat and The Granularity Problem**
The `combat::min_distance_over_tick` function is a clever mathematical bandage over a structural problem. Because your tick is 1 hour, ships moving at 8 knots jump 8 nautical miles at a time. The 0.5 NM cannon range would constantly be stepped over. 
By interpolating the closest approach, you fix the geometry, but you lose the tactical maneuvering. The windward gauge, raking fire, and multi-ship pack tactics (detailed in `naval-combat.md`) cannot exist in a system where ships interpolate their closest approach mathematically rather than physically steering through it. 

**The Economy and the "Home Bias" Pathology**
Your `bench_trade` log notes a pathology where ships lock into hyper-profitable short routes (e.g., Barbados to Martinique) because the `home_bias` multiplier dominates the trade planner once they accumulate silver.
This happens because `find_best_trade` evaluates single legs greedily. In reality, a ship full of silver wouldn't endlessly ferry salt cod back and forth; it would undertake a triangular route (e.g., sail ballast to Europe, buy high-margin manufactures, sail to the Caribbean). The Kantorovich solver in `equilibrium.rs` proves your underlying pricing math is sound; the AI's limited horizon is what is breaking the economy over 730-day runs.

---

### 3. Programming Beyond Phase 3 (The Roadmap)

As you transition into Phase 4 (Factions, Diplomacy, and War), here is what demands your attention:

**1. Fix the Read-Compute-Write Pipeline (Immediate)**
As mentioned, extract the command resolution loop in `world.rs`. 
* AI Loop: All ships read `snapshots` and `spatial_hash`, writing to `World::commands`.
* Resolution Loop: Drain `commands`, aggregating damage and state changes.
* Mutation Loop: Apply the aggregated changes to `ships`.

**2. The Relations Matrix and Letters of Marque**
Currently, `SeePrey` and `SeeThreat` trigger hardcoded on `ShipPolicy::Pirate`. 
In Phase 4, you must implement the `RelationsMatrix` (from `diplomacy.md`). A ship flying the English flag with `ShipPolicy::Privateer` should check the matrix. If England is at war with France, French ships return `true` for `SeePrey`. If peace is declared, that same logic should cleanly fall back to neutral passing—unless the privateer's morale is low and debt is high, prompting a mutiny into `ShipPolicy::Pirate`.

**3. Ordnance as an Economic Good**
Right now, `ShipStats` has a fixed `cannons: u16` field. To capture the Caribbean arms trade (detailed in `goods-taxonomy.md`), cannons must become a physical commodity.
When a shipyard builds a hull, it should require buying cannons from the port market. When a pirate takes a prize, they should be able to strip the cannons to up-gun their own sloop. This integrates the violence of Phase 3 directly into the economy of Phase 2.

**4. Multi-Hop Trade Planning**
To fix the Amsterdam Fluyt saturation and the Home Bias loop, `trade.rs` needs a horizon upgrade. Instead of `find_best_trade` evaluating `A -> B`, it should evaluate `A -> B -> Home`. If `B` has no profitable exports back to `Home` (or to a lucrative `C`), the overall score of the route must be penalized. Ships (plan) to travel on circuits and return to home ports on mercantile routes; only pirates and military vessels should pursue other behavioral patterns. 

**5. The Sub-Tick Combat Dilemma**
You decided to keep the tick at 1 hour to avoid complexity. As you introduce Naval Squadrons and Forts (from `naval-combat.md`), this will break down. A fort guarding a 1 NM channel will be "stepped over" by a ship doing 8 knots in an hour. 
*Recommendation:* Introduce a dynamic tick. If `spatial.neighbors` returns hostile ships within 3 NM, the simulation seamlessly shifts to a 15-minute tick for those actors (or the whole world) until the engagement resolves. Alternatively, we can have the 'base' tick be 10 minutes, with most systems updating not on every tick; we must consider the best way to handle it.

### Conclusion
The codebase is a triumph of rigorous historical research applied to systems engineering. You have successfully mapped the brutal economic realities of 17th-century maritime labor and mercantilism into a stable, headless Rust simulation. Correcting the command-drain loop to achieve true Double Buffering will solidify the architecture, perfectly positioning you for the diplomatic and tactical complexities of Phase 4.