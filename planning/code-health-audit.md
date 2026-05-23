# Code Health Audit — God Functions on `phase-3`

> **Audit context.** Branch `phase-3` at `2c69419` (Step 3.a landed,
> Step 3.b queued: crew-on-ship + Hiring state). Read-only static
> audit; no builds/tests run. Three planning docs read in full
> (`phase-3-plan.md`, `crewing-plan.md`, `architecture-revised.md`)
> plus relevant `development-log.md` entries.

## Executive Summary

Three functions stand out. The rest of the tree is healthy.

1. **`World::tick` (`world.rs:126–299`) — *Do now (small surgery only).*** 174 lines mixing a monthly economic/shipyard block with the per-hour AI+physics loop. Step 5 will pipeline-refactor the hourly side; the *monthly* side is orthogonal to that work and is already painful to read. Extract `tick_monthly()` and `apply_ship_movement()` *before* Step 3.b inserts the daily Hiring tick, which would otherwise wedge a third loop into the same body.

2. **`ShipBtContext::execute_action` (`ai.rs:304–590`) — *Do soon, fold into Step 5.*** A 286-line `match` whose `ACT_BUY_BEST` (≈100 lines) and `ACT_SAIL` (≈85 lines) arms each do their own multi-step orchestration. Step 5 mandates rewriting this anyway (BT becomes read-only, leaves emit `ShipCommand`s). Don't pre-refactor — but *do* split arms into named methods as the first commit of Step 5 so the diff is reviewable.

3. **`ShipAI::tick` signature (`ai.rs:191–214`) — *Do soon, free with Step 5.*** Eight arguments with two `Option<&mut …>` slots; `#[allow(clippy::too_many_arguments)]` is already on the wall. Step 5's `ShipBtContext` plan deletes this naturally. No standalone refactor needed.

Everything else is either single-purpose (`Navmesh::build`), well-bounded (`PortMarket::*`), or visual (`draw_ship_panel`) and can wait.

## Methodology

I read `world.rs` and `ai.rs` in full, skimmed every other `sim-core/src/*.rs` and `sim-viz/src/main.rs`, and listed every `fn`/`pub fn` to find length outliers (>80 lines or >5 sequential responsibilities). For each candidate I cross-referenced the Phase 3 step list and `crewing-plan.md` §3 (Steps 3.b, 5, 6, 9) to predict whether the function will be touched, displaced, or grow.

"God function" in this audit means a function that (a) is materially harder to read than its peers, *and* (b) is on or near the path of imminent change. A merely-long function in a quiet corner is not flagged.

---

## 1. `World::tick` — `crates/sim-core/src/world.rs:126`  *(Do now, narrowly)*

### Current shape

174 lines, one `impl` method. Three discrete concerns interleaved:

- **L126–134** Setup: read `month`, build a representative `pathfind_stats` from the sloop.
- **L140–215** Monthly block (fires once per month transition): market tick, demographics tick, average-profit computation, shipyard build pass with a `newly_built` deferral vector, snapshot reset of `silver_at_month_start`.
- **L217–296** Per-hour loop: build `PathfindContext`, snapshot `ids`, then per-ship `ship_stats` lookup → wind lookup → split-borrow `ai.tick(…)` (8 args) → `tick_resources` → land-snap rescue → swept-movement physics.
- **L298** `self.date.advance_hours(1)`.

The monthly block alone is 75 lines and contains two of the loop's three uses of `self.ports`/`self.markets`/`self.ships` together, which is precisely the borrow surface Step 5 wants to redesign.

### Diagnosis

Three reasons this is worth touching *before* Step 3.b:

1. **Step 3.b will add a third frequency.** `crewing-plan.md` §3.2 wants a *daily* Hiring tick: ships in `ShipState::Hiring` draw from `PortDemographics` once per game-day. Adding `if day != self.last_day { … }` to a body that already has hourly + monthly branches will push it well past 200 lines and four cadences.
2. **The monthly block is independent of the Step 5 pipeline refactor.** Step 5 explicitly refactors the *hourly* AI tick into AI/Resolution/Mutation/Cleanup. The monthly block doesn't fit any of those phases — it's a macro-scale economic tick. Leaving it inline through Step 5 forces every Step-5 reviewer to scroll past it.
3. **The `silver_at_month_start` bookkeeping (L150–168, L209–212) is genuinely confusing in-context** — three forward references and one subtle comment about new-ship ordering. As an isolated helper it would be self-documenting.

### Proposed refactor

Pure mechanical extraction. No semantic change.

```rust
impl World {
    pub fn tick(&mut self) {
        let month = self.date.month();
        if month != self.last_market_month {
            self.tick_monthly(month);
        }
        // Step 3.b will add: if day != self.last_day { self.tick_daily(day); }
        self.tick_hourly_ai_and_physics(month);
        self.date.advance_hours(1);
    }

    fn tick_monthly(&mut self, month: u8) { /* current L141–214 */ }
    fn tick_hourly_ai_and_physics(&mut self, month: u8) { /* current L217–296 */ }
}
```

Optionally extract the swept-movement block (L268–295) as `apply_ship_movement(&mut self, id, &ship_stats, &wind)` — but that requires care around the land-snap mutating `ship.position` via `self.map.land`. Defer that sub-extraction; the two-method split is the high-value cut.

### When / why now

**Now**, before Step 3.b. The split is purely mechanical (no behavior change, no signature change to public API, tests all stay green). It removes ~70 lines from `tick()` so Step 5's pipeline refactor lands as a focused diff against `tick_hourly_ai_and_physics`, not the whole tick.

### Tests / risks

- All existing tests are black-box over `World::tick`; none reach into the body. Zero test churn expected.
- `bench_trade` and `diag_nav` examples call `world.tick()`; unaffected.
- Risk: accidentally changing the order of (monthly tick) vs (pathfind context construction) — keep `pathfind` construction inside `tick_hourly_ai_and_physics` so the per-month profile rebuild lands at the same point in the call graph.
- Effort: ~30 minutes, ~80 lines moved, ~10 lines of new function headers.

---

## 2. `ShipBtContext::execute_action` — `crates/sim-core/src/ai.rs:304`  *(Do soon, as the opening commit of Step 5)*

### Current shape

286 lines. A `match id { ACT_SAIL => …, ACT_RESUPPLY => …, … }` over 8 action IDs. Two arms dominate:

- **`ACT_SAIL` (L306–392, ~85 lines):** harbor-arrival detection → debt settlement → owner dividend → replan-on-empty-waypoints → steering compute → "false-dock prevention" replan → fallback free-form dock.
- **`ACT_BUY_BEST` (L473–575, ~100 lines):** build provision budget → compute home-bias → call `trade::find_best_trade` → optional outfit draw → optional tramping credit → execute buy → assign next port. Six conceptually distinct sub-steps with intermediate state.

### Diagnosis

This function is on the **direct line of fire** for three upcoming changes:

| Step | Effect on `execute_action` |
|---|---|
| 3.b (next) | Adds `ACT_HIRE` and `COND_HAS_MIN_CREW`; touches `ACT_UNDOCK` (gate on crew) and `ACT_BUY_BEST` (provisions burn now keyed on `crew_alive`). |
| 5 | BT becomes read-only re ships. `ACT_SAIL` no longer calls `ship.set_steering`; it pushes `ShipCommand::Steer(…)` to a queue. Every arm that mutates `self.ship` must change. |
| 6 | Adds `ACT_PURSUE`, `ACT_FLEE`, `COND_SEE_PREY`. More arms, more conditions. |

If the function is still a single 300-line match when Step 5 hits, the Step-5 commit becomes unreviewable — every arm changes signature simultaneously.

### Proposed refactor

**Don't pre-refactor in isolation.** Bundle the extraction into Step 5's *first* commit, before any semantic changes:

```rust
fn execute_action(&mut self, id: usize) -> Status {
    match id {
        ACT_SAIL              => self.act_sail(),
        ACT_RESUPPLY          => self.act_resupply(),
        ACT_CAREEN            => self.act_careen(),
        ACT_UNDOCK            => self.act_undock(),
        ACT_CHOOSE_DESTINATION=> self.act_choose_destination(),
        ACT_SELL_ALL          => self.act_sell_all(),
        ACT_BUY_BEST          => self.act_buy_best(),
        ACT_DIVERT_TO_PORT    => self.act_divert_to_port(),
        _                     => Status::Failure,
    }
}
```

Then within `act_sail`, factor the harbor-arrival path (settlement + dividend + state transition) into `arrive_at_destination_harbor()`. Within `act_buy_best`, factor `outfit_draw_if_home(idx)` and `tramping_credit_if_needed(idx, plan)`.

### When / why now (or rather, why *not yet*)

**Not now.** Three reasons:

1. The arms work as-is and tests cover them. The cost of a standalone "factor and ship" refactor is non-zero (re-running scenarios, viz spot-check) and the dividend is small until Step 5 forces every arm to change.
2. Step 5's pipeline refactor will re-shape borrow patterns inside each arm anyway (e.g. `ACT_SAIL` will stop taking `&mut Ship`). Factoring twice — once for cosmetics, again for semantics — wastes the first pass.
3. Step 3.b *adds* one arm (`ACT_HIRE`) without changing the existing ones much. The function survives one more accretion.

**Do it as the first commit of Step 5** (the "no-semantic-change rename" commit) so the subsequent semantic commit is reviewable arm-by-arm.

### Tests / risks

- Existing `bt.rs`, `nav.rs` tests are black-box. Risk is `self.ship` / `self.markets.as_deref_mut()` reborrow patterns — moving an arm into its own method may force the closure-style `let-else` patterns to convert to early returns. Mechanical but compiler-driven.
- Effort: ~1 hour, ~290 lines moved into 8 methods, no logic change.

---

## 3. `ShipAI::tick` argument list — `crates/sim-core/src/ai.rs:191`  *(Do soon, free with Step 5)*

### Current shape

```rust
#[allow(clippy::too_many_arguments)]
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
) { … }
```

Eight arguments, two of them `Option<&mut …>` to support market-less toy tests. The body wraps them all into a `ShipBtContext` literal (L202–214) and dispatches.

### Diagnosis

Two smells:

1. **The `Option<&mut [PortMarket]>` + `Option<&GoodsRegistry>` pair only exists for legacy toy tests.** The `_at_market` variant of resupply was added to keep these tests green (`ship.rs:218–223`). Production callers always pass `Some(_)`. Carrying the `Option` through every arm of `execute_action` adds noise (`let (Some(idx), Some(markets), Some(goods)) = …` appears 3× in `ai.rs`).
2. **`World::tick` already constructs `ShipStats`, wind, and the pathfind context per ship and threads them through this signature.** When Step 5 introduces a `ShipBtContext` shared across the AI Phase, this 8-arg call collapses to `ai.tick(&ctx)`.

### Proposed refactor

`phase-3-plan.md` §The Cellular Automata BT Context already specifies the target shape. Don't invent a competing design. Just adopt it during Step 5. Specifically:

- Promote `ShipBtContext` from a `struct` in `ai.rs` to a public type owned by `World::tick_hourly_ai_and_physics`, built once per tick with all read-only fields and a `&mut Vec<ShipCommand>` output buffer.
- Reduce `ShipAI::tick` to `pub fn tick(&mut self, ctx: &mut ShipBtContext<'_>)`.
- Migrate test scaffolding to construct a minimal `ShipBtContext`. The two `Option<…>` parameters disappear: tests pass real (empty) markets and a real goods registry, both cheap to construct.

### When / why now

**Not now, but don't carry it past Step 5.** The signature is annoying but not blocking; rewriting it standalone wastes effort because Step 5 deletes the call site anyway. The `#[allow(too_many_arguments)]` is the load-bearing comment — leave it as the trail-marker.

### Tests / risks

- All AI tests construct `ShipAI` directly. Touching the signature ripples to `bench_trade`, `diag_nav`, and `ship.rs`/`nav.rs` test modules.
- Risk: the Option-stripping forces toy tests to either build a real `GoodsRegistry::starter()` (cheap) or accept that they're now integration tests. Acceptable.
- Effort: ~2 hours including test migration; folded into Step 5 ≈ free.

---

## Appendix — Considered but rejected

| Function | File:line | Verdict | Why |
|---|---|---|---|
| `Navmesh::build` | `navmesh.rs:101` (173 lines) | **Leave alone** | One-shot at world load; densely commented; single purpose (open-water pass → channel pass → edges → component prune). No churn expected in Phase 3. |
| `solve_single_good` | `equilibrium.rs:273` (133 lines) | **Leave alone** | Internal to the diagnostic LP solver, not on the sim hot path. The closure-over-LP pattern is idiomatic for the dual perturbation. |
| `shipyard::try_build` | `shipyard.rs:144` (95 lines) | **Defer** | Two clean halves (score loop, commit). Step 3.b adds one `crew_min` gate (~5 lines); Step 9 won't touch it. Decompose only if a third concern lands. |
| `PortMarket::tick_month` + `buy`/`sell` | `market.rs:150,253,298` | **Leave alone** | All under 50 lines, transactional, well-tested. Good shape. |
| `Ship::tick_resupply_at_market` | `ship.rs:232` (57 lines) | **Leave alone** | Linear, three named branches (chandler credit / desired calc / done-conditions). No nesting. Phase-3 crew/wage additions will likely live in a sibling method, not here. |
| `LandMap::farthest_clear_point` | `map/land.rs:236` (40 lines) | **Leave alone** | Tight DDA sweep. Not overgrown. |
| `draw_ship_panel` | `sim-viz/src/main.rs:366` (130 lines) | **Defer** | Will grow with crew/morale/damage panels. Viz code, mechanical to update, no architectural risk. Revisit at Step 7 when there's enough new state to justify splitting into header / status / cargo sub-renderers. |
| `World::load` | `world.rs:67–113` | **Leave alone** | 46 lines of straight-line construction. Long but flat. |
| `Cargo`, `NavState`, `BtState` impls | various | **Leave alone** | Single-purpose data containers; nothing overgrown. |

---

## Footnote on `World::tick` and Step 5

The Step 5 plan (pipeline refactor into AI → Resolution → Mutation → Cleanup) lands cleanly *only on the hourly portion* of today's `tick()`. The monthly economic tick — `market.tick_month()`, demographics, shipyard `try_build`, profit averaging — is a different cadence with a different borrow shape and doesn't fit any of the four pipeline phases. The §1 refactor above isolates the monthly block so Step 5 can ignore it. That's the single highest-leverage change in this audit: 30 minutes of work, zero behavior change, materially smaller Step-5 diff.
