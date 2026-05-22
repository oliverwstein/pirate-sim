# Copilot Cloud Agent Instructions: Pirate Sim
Never end the conversation without being told, through the ask-user-question tool, that the user is done. After each action, use the ask user question tool to prompt the user for the next step. Always wait for the user's response before taking another action. Before committing code, always ask the user to review the changes and confirm that you should proceed. If the user says no, ask them what changes they would like to see and make those changes before asking them to review again. After committing, ask what is next. 

*Remember, always use the ask user question tool when you finish working.*

## Development Logging (`development-log.md`)
Maintain a running log of decisions in `planning/development-log.md`. Whenever you and the user finalize a significant design choice, solve a complex architectural problem, or pivot from an older plan, you must append an entry to this file. If something needs to be changed, or turns out not to work, a note on that should also be added. 
*   **Format:** Append to the bottom of the file. Use a brief, clear markdown format.
*   **Content:** Include the problem/context, alternatives considered, and the rationale for the agreed-upon solution. 


## 1. High-Level Codebase Details
**Pirate Sim** is a continuous-time, variable-rate RTS economic and naval simulation of the 17th-century Caribbean. 
*   **Architecture Pattern:** Strict Data-Oriented Design (DOD). The codebase uses an ECS-lite approach (moving toward Generational Indices via `slotmap`).
*   **Core Principle:** "Headless-First". The simulation logic is 100% decoupled from rendering. The core library (`sim-core`) represents the simulation. The visualizer (`sim-viz`) is a read-only observer of the world state.
*   **AI Pattern:** Flyweight Behavior Trees. Ships are agents that read a snapshot of the world and emit "Intents" (Commands). A central resolver turns these into "Consequences" (Events) that mutate the world state.

### Standard Build & Lint
Always format and lint your generated code to pass CI.
```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo build --workspace
```

### Running the Visualizer
To view the simulation in action (this also allows the user to see the visuals, so they can report behaviors to you).
```bash
cargo run --release -p sim-viz
```

### Running Calibration Benchmarks
Whenever you modify economic logic, trade rules, ship stats, or pathfinding, you may want to run these benchmarks as part of your validation process. 
**1. Trade & Economy Calibration (Runs a 60-day headless sim):**
Validates that ships don't bankrupt themselves and that prices don't spiral.
```bash
cargo run --release -p sim-core --example bench_trade
```

**2. Pathfinding Validation:**
Validates the `Navmesh` and checks the A* route between all ordered port pairs.
```bash
cargo run --release -p sim-core --example bench_pathfind
```

**3. Economic Equilibrium (Kantorovich LP Solver):**
Compares emergent simulation prices against a mathematical baseline.
```bash
cargo run --release -p sim-core --example equilibrium_report
```