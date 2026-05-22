# Development Log
Phases 1 & 2 were implemented prior to the creation of this log; you can now expand it.
---

## 2026-05-22 — Phase 3 Planning Session

**Context:** Phases 1 & 2 complete; entering Phase 3 (Populations, CA combat, piracy). Reviewed `phase-3-plan.md`, `architecture-revised.md`, `overall-plan-sketch.md`. Spun up a background research agent (`planning/research/sailor-recruitment.md`) to inform the pop-growth model.

**Decisions (with rationale):**

1. **Sequencing — plumbing-first as written.** Considered a "vertical slice" (visible chase ASAP, defer RON/Pops) but accepted the architecturally clean ordering: SlotMap → RON → Pops → pipeline refactor → chase → gunnery → boarding → mutiny → calibration. Multi-session investment up front pays off in correctness for everything after.

2. **Combat tick — hourly only.** Considered a 15-min sub-tick "when engaged" (per architecture-revised.md and overall-plan-sketch.md). Rejected for v1 because hourly chases are period-realistic (engagements last hours-to-days) and adding two timescales complicates the pipeline. May revisit if combat feels mushy.

3. **Crew as ship property, not "consumed pop."** Pushed back on the phrasing "ships consume sailors at shipyards" — crews are a first-class ship property like the hull. `Ship.crew_alive: u16`. Drawn from the port pool on hire/launch, discharged back at dock, lost via combat or attrition at sea. Crew size drives provisions burn and effective_speed (under/over-crew curves).

4. **Five factions, piracy is a state of mind.** Spain, England, France, Netherlands, Free. Piracy is `Ship.policy: ShipPolicy` (Merchant / Privateer{against:FactionSet} / Pirate / Navy), not faction membership — this cleanly accommodates privateers (a French ship lawfully raiding Spanish under English LoM is hostile to Spain but neutral to England).

5. **Boarding — deterministic single-tick.** `crew * (1 + morale_bonus)` per side; larger force wins; proportional losses. Multi-tick melee with morale rolls deferred until v1 boarding feels too binary.

6. **Sailor pop — tiered organic growth + monthly mortality.** Per research/sailor-recruitment.md §7.1: port categories (European hub / Caribbean entrepot / Small colonial / Pirate haven) drive monthly growth rate (5–60/yr depending on tier). Two-tier pool: unseasoned (high mortality, matures into seasoned) and seasoned (low mortality). Transient supply from ship arrivals. Faction-specific fill rates when hiring (English fastest, Spanish slowest). Post-war demobilization shocks deferred to Phase 4 once wars exist.

7. **Rhai / Navigation Acts — deferred to Phase 4.** Phase 3 stays focused on populations + combat + piracy; trade-law hook lands once factions actually matter for trade flow.

**Research artifact:** `planning/research/sailor-recruitment.md` (260 lines, citations to Rodger, Rediker, Frykman, Johnson). Provides quantitative calibration targets: organic pool growth, mortality (25–40%/yr unseasoned, 8–15%/yr seasoned), faction fill times, mortality dominated by disease over combat.

**Open / deferred for later in Phase 3:**
- Officer / specialist crew roles (master, gunner, surgeon) — single `crew_alive` count for v1.
- Letters of Marque as a data structure — `Privateer{against:FactionSet}` is just an enum for now.
- Multi-tick boarding with morale rolls.

**Next action:** Refine `planning/phase-3-plan.md` to incorporate the above (crew-on-ship semantics, ShipPolicy, two-tier sailor pool, port categories), then begin Step 1 (SlotMap migration).

---

## 2026-05-23 — Step 1: SlotMap migration

**Goal:** Replace `Vec<Ship>` with a generationally-indexed container so handles to ships survive deletion (needed for combat targets, captured prizes, prize-crew transfers, and the eventual command queue without index invalidation).

**Implementation:**
- Added `slotmap = "1"` to `sim-core`. Defined `ShipId` via `slotmap::new_key_type!` in `types.rs`.
- `World`: `ships: SlotMap<ShipId, Ship>`, `ship_ais: SecondaryMap<ShipId, ShipAI>`, `silver_at_month_start: SecondaryMap<ShipId, f32>`. `add_ship` now returns `ShipId`.
- Tick loop rewritten: collect keys upfront (`let ids: Vec<ShipId> = self.ships.keys().collect()`), then iterate fetching `self.ships.get_mut(id)` and `self.ship_ais.get_mut(id)` as separate split borrows.
- All examples migrated (`bench_trade`, `diag_nav`). `bench_trade` keeps `ship_ids: Vec<ShipId>` parallel to `origin_names` for stable per-ship reporting; uses a `HashSet<ShipId>` to detect newly-built ships across ticks (SlotMap iteration order is not guaranteed stable).
- `sim-viz`: `selected_ship: Option<ShipId>`, `pick_ship_at` returns `Option<ShipId>`, panels accept `ShipId` and use `.get(id)` defensively.

**Considered alternatives:**
- Plain `Vec<Ship>` + tombstones: rejected — every dereference would need a "still alive?" check, and indices still get reused.
- Custom `(generation, index)` newtype: rejected — `slotmap` is the well-tested standard answer in Rust and gives us `SecondaryMap` for free, which is exactly the pattern we want for `ship_ais` and per-ship debt/silver bookkeeping.
- Migrating `Vec<Port>` at the same time: rejected — ports don't get created or destroyed in normal play, so the `usize` index is fine and the churn would be wasted. Revisit if Phase 4 adds port sieges that destroy ports.

**Verification:**
- `cargo build --workspace` clean; `cargo test --workspace` 75 + 19 + 0 = 94 passed.
- `cargo run --release -p sim-core --example bench_trade` produces identical calibration verdict to baseline ("5 bankrupt ships" warning — pre-existing Phase 2 calibration quirk, not introduced by this migration).
- Clippy error count unchanged at 22 (all pre-existing in unrelated files: `equilibrium.rs`, `shiptype.rs`, `trade.rs`, etc.). No new lints from migrated code.

**Notes for next step:**
- `SlotMap` iteration order isn't stable across removals — anywhere that needs deterministic ordering (calibration reports, save files) should sort by `ShipId` or carry an explicit ordering vec.
- `slotmap` has an opt-in `serde` feature; will enable when Step 2 (RON extraction) lands save/load.

**Next action:** Step 2 — extract `GoodsRegistry`, `ShipTypeRegistry`, and the port list into `data/*.ron`. Add `serde` + `ron` deps; derive `Deserialize`; load at `World::load`.

---

## 2026-05-23 — Step 2a: RON extraction (goods + ship types)

**Goal:** Move the hardcoded goods and ship-type catalogs into RON data files so they're editable as data rather than code, and establish the loading pattern for everything that follows (ports, factions, future content packs).

**Scope decision:** Goods + ship types this commit; ports deferred to Step 2b. Ports are entangled with the `Faction` enum and `&'static [ShipTypeId]` shipyard slices, which deserve their own design pass and a separate, focused diff.

**Data-layout decision (DOP framing):**
- **Chose** owned `String` for record `name` fields over `Box::leak`-to-`&'static str`. The whole point of RON extraction is that the data no longer has the lifetime of the binary — it has the lifetime of the registry that owns it. `String` expresses that honestly; `Box::leak` would have kept call sites unchanged at the cost of a small permanent leak per load and a runtime sleight-of-hand about lifetimes.
- Cost: ~5 mechanical call-site changes (`let name = world.goods.get(gid).name;` → `let name = &world.goods.get(gid).name;`). All in display/format paths in `bench_trade` and `sim-viz`.

**Implementation:**
- Added `serde = { version = "1", features = ["derive"] }` and `ron = "0.8"` to `sim-core`.
- New `data/registries/goods.ron` (9 goods) and `data/registries/ship_types.ron` (5 types) with header comments documenting the field schema.
- `Good` and `ShipType` keep their runtime shape but `name: &'static str` becomes `name: String`. New private `GoodRecord` / `ShipTypeRecord` structs are the actual serde-derived shapes; `GoodsRegistry::from_ron_str` (and the sibling on `ShipTypeRegistry`) stamps in the position-derived `GoodId` / `ShipTypeId`.
- `GoodsRegistry::starter()` and `ShipTypeRegistry::starter()` keep their infallible signatures by calling `from_ron_str(include_str!(…))` on the bundled RON. Editing the RON requires a rebuild *for now*; a runtime path-loader is one method call away when we want true hot-reload.
- `GoodCategory` and `Perishability` gained `Deserialize` derives; `ShipStats` (in `ship.rs`) likewise so it can be deserialized as a nested field of `ShipTypeRecord`.

**Stable IDs preserved:** The `goods::ids::*` and `shiptype::ids::*` constants still match positions in the RON files. The existing `ids_resolve_to_expected_goods` and `ids_match_indices` tests catch any accidental re-ordering.

**Considered alternatives:**
- Named-handle lookup (`registry.by_name("Muscovado Sugar")` everywhere instead of `ids::SUGAR`): rejected — turns every reference into a runtime fallible lookup and a string literal, which is worse ergonomics than a `const GoodId`.
- TOML / JSON / YAML: rejected — RON's struct/enum syntax matches Rust types natively, no string-keyed-map gymnastics for enum variants.
- `serde_with` for `Position` etc.: not needed; no nested unusual types in this step.

**Verification:**
- `cargo build --workspace --tests --examples` clean.
- `cargo test --workspace` 94 tests pass (75 + 19 + 0 + 0).
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `bench_trade` produces identical calibration verdict ("5 bankrupt ships" — Phase 2 pre-existing, unchanged).

**Next action:** Step 2b — port list extraction. Open question for that step: how to model the `Faction` enum and the per-port shipyard list in RON (named lookup vs index list).



---

## 2026-05-23 — Step 2b: RON extraction (ports)

**Goal:** Move the 40-port catalog into `data/registries/ports.ron`, completing the registry-extraction work begun in Step 2a.

**Design decisions:**
- `Port.name: &'static str` → `String`; `Port.shipyard: Option<&'static [ShipTypeId]>` → `Option<Vec<ShipTypeId>>`. Same lifetime-honesty reasoning as Step 2a.
- **Shipyard list uses ship-type *names*, not indices**, in the RON file (e.g. `shipyard: Some(["sloop", "brigantine"])`). Resolved to `ShipTypeId` at load by linear search against `ShipTypeRegistry`. Names survive registry reordering; indices wouldn't. Lookup is O(ports × names_per_port × ship_types) once at boot — irrelevant at 40 × 2 × 5.
- New `PortLoadError::UnknownShipType { port, name }` makes typos in the RON loud at startup rather than silently dropping shipyards.
- **Two-phase construction in `World::load`:** ship types first (no dependencies), then ports (need ship-type registry for name resolution). This sets the precedent for the dependency-aware loading order we'll need as more registries come online (factions → ports → ships).
- **`Position` in RON uses `(f32, f32)` tuple**, not glam's native serde format. Decouples the on-disk schema from any future glam version bump.
- Kept `Faction::Holland` (not renamed to `Netherlands`) for back-compat with the existing enum; rename is a Phase 3 Step 4 concern when factions get their own pass.

**Call-site fan-out:** ~10 sites touched (test fixtures in `harbor.rs`, `trade.rs`, `shipyard.rs`, `ai_behavior.rs`; bench_trade origin-name tracking; viz `draw_text` and dock-status; examples calling `all_ports()` and `archetype_for(p.name)`). Mechanical `&port.name` / `.clone()` / `.to_string()` changes.

**Considered alternatives:**
- Faction lookup by name in RON (e.g. `faction: "Spain"`): rejected for now — `Faction` is still an enum with a small fixed set, RON's enum syntax (`faction: Spain`) is fine. Will revisit in Step 4 when we add a `FactionRegistry`.
- Putting all registries in one file: rejected — separate files cleanly delimit "this is what defines a port" vs "this is what defines a good", makes diffs readable.

**Verification:**
- `cargo build --workspace --tests --examples` clean.
- `cargo test --workspace` 94 tests pass.
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `bench_trade` identical calibration verdict ("5 bankrupt ships" baseline preserved).

**Next action:** User confirmation, then commit as "Step 2b: extract ports into RON". After that, Step 3 — port demographics + crew on ships (the first behavior change).

---

## 2026-05-23 — Step 3.a: port demographics genesis

**Goal:** Stand up the per-port sailor pool data and its monthly dynamics (growth, maturation, mortality) without touching ships yet. The pool evolves in the background; bench_trade reports it so we can calibrate before any ship draws from it.

**Implementation:**
- New `crates/sim-core/src/pop.rs`: `PortCategory` (EuropeanHub / CaribbeanEntrepot / SmallColonial / PirateHaven), `PortDemographics { seasoned, unseasoned, category }`, `tick_monthly()`. Faction-based seed and growth multipliers per crewing-plan §4.5.
- `Port` extended with `category: PortCategory` (mandatory RON field). `PortRecord` likewise. All 36 ports categorized in `ports.ron`.
- `World` gains `demographics: Vec<PortDemographics>` parallel to `markets`/`ports`. Monthly tick (gated on month transition) updates every port.
- `bench_trade` prints a sailor-pool summary block grouped by category.

**Design decisions:**
- **Deterministic rounded expectations**, not stochastic sampling, for v1. A given seed reproduces identical pool evolution — invaluable for calibration. Cost: small pools (e.g. pirate havens, ~30 sailors) see step-function behavior when `rate × N < 0.5` rounds to zero. Documented as expected; stochastic mode is a Phase-4 refinement when relevant.
- **Spain seed multiplier of 0.5** is what makes the Casa de Contratación bottleneck visible in the seed data (Spanish ports start with half the sailors of equivalent English ports). Cross-check: Cartagena's 7-port CaribbeanEntrepot total of 1882 is right for a mixed-faction sample.
- **`PortCategory` lives in `pop.rs`**, not `port.rs`. Keeps the demographics concern isolated; `Port` simply references it as data. If we later add other category-driven systems (defense, taxation), they each get their own module owning their own enum.
- **Pirate haven faction `Faction::Pirate` mult is 0.3**, but seeds (15, 25) before mult give (5, 8). The test `monthly_tick_pirate_haven_does_not_grow` is the honest claim — decay needs stochastic sampling that we're not adding yet.

**Considered alternatives:**
- `f32` pools displayed as integers: rejected — exposing fractional sailors in diagnostics is confusing, and the visible step-function behavior is a fair signal that v1 deterministic mode has limits.
- Per-port RNG seeded from port index: rejected — adds reproducibility complexity (cross-process determinism) without much modeling value at v1 pool sizes.
- Embed `PortDemographics` directly in `Port`: rejected — `Port` is read-mostly static data; demographics mutates every month. Same pattern as `PortMarket` already established.

**Verification:**
- `cargo test --workspace`: **101 passed** (was 94; +6 new pop tests, +1 existing relaxed).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `bench_trade`: identical Phase-2 calibration verdict ("⚠ 5 bankrupt ship(s)"). New sailor-pool block reports:
  - EuropeanHub: 4 ports, 22,066 total sailors
  - CaribbeanEntrepot: 7 ports, 1,882 total
  - SmallColonial: 23 ports, 1,346 total
  - PirateHaven: 4 ports, 74 total

**Next action:** Step 3.b — `Ship.crew_alive`, `ShipState::Hiring`, and the daily recruitment loop drawing from these pools.

---

## 2026-05-24 — Step 3.b: Hiring state + crew on ships

**Goal:** Decouple shipyard build from crewing. A new hull leaves the yard with **no sailors** and sits in `ShipState::Hiring`, drawing from the local `PortDemographics` pool until it reaches a minimum crew. Only then does it transition to `Docked` and become visible to the AI.

**Implementation:**
- `ShipStats` got two derived helpers: `crew_typical() = crew as u16` and `crew_min() = ceil(crew * 0.4).max(2)`. No RON-schema change in 3.b; per-type minimums can be added later if calibration demands it.
- `ShipState` gains a `Hiring` variant (doc-comment references crewing-plan §3). All exhaustive matches updated: `bench_trade` shows `"hiring"`; viz renders `HIRING (crew n/typical)` and counts hiring ships as docked for the lobby panel.
- `Ship.crew_alive: u16`. `Ship::new` defaults to `stats.crew_typical()` (back-compat for seed ships & tests). `Ship::freshly_built` sets `state: Hiring, crew_alive: 0`.
- `World.last_hire_day: u16`. New daily hiring tick in `World::tick` (gated on `date.day_of_year` transition, **before** path/AI work). For each `Hiring` ship: draw up to `HIRE_PER_DAY = 5` sailors from its owner port (seasoned-first, then unseasoned). Transition to `Docked` when `crew_alive >= crew_min()`.

**Design decisions:**
- **World-level hiring pass, not AI-driven (Option A).** Keeps `&mut demographics` out of `ShipBtContext` and fits the "AI is read-only" direction of Step 5's pipeline refactor. The AI never observes `Hiring` — by the time a ship is `Docked`, it's already crewed.
- **Crew helpers derived, not stored.** Avoids touching ship_types.ron in 3.b. `crew_min = ceil(crew * 0.4).max(2)` mirrors crewing-plan §2's "skeleton crew" rule of thumb.
- **`Ship::new` stays fully-crewed.** Tests and AI integration tests already construct ships ad-hoc; auto-Hiring them would have broken dozens of tests for no behavioral benefit. Only the *shipyard* path produces empty hulls — that's where the design intent lives.
- **Flat 5 sailors/day, no faction multiplier yet.** 3.b is plumbing; faction-fill-rate, demand pressure, and sign-on bounties land in 3.c.
- **Seasoned-first draw.** Crewing-plan §5 — ships prefer experienced hands; unseasoned only when seasoned dries up. Simpler than weighted sampling and matches historical hiring priority.

**Considered alternatives:**
- Hourly hiring with fractional accumulation: rejected. Once/day is simpler, matches the design doc's per-day rate, and avoids carrying a partial-sailor counter on each Hiring ship.
- Drain unseasoned first (to preserve veterans): rejected. Captains historically wanted seasoned hands; "skim the cream" matches both the design and intuition.
- Make `crew_min` an explicit RON field: deferred. Derive-and-tune now; promote to data if calibration needs per-type tuning.

**Verification:**
- `cargo build --workspace --tests --examples`: clean after adding `ShipState::Hiring` arms in viz and bench_trade.
- `cargo test --workspace`: **102 passed** (+1 new shipyard test: `freshly_built_ship_starts_hiring_with_no_crew`).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `bench_trade`: **2 bankrupt ships** (was 5). Legitimate behavior change — shipyard-built ships now spend their first few days hiring rather than immediately incurring debt by sailing, slightly damping the bankruptcy rate. Equilibrium mispricing metrics unchanged in shape. The "5 bankrupt" baseline is retired; "2 bankrupt" is the new Phase-3 baseline for Step 3.b.

**Next action:** User review, then commit as "Step 3.b: Hiring state + crew on ships". After that, Step 3.c (wages, morale, discharge).

---

## 2026-05-24 — Step 3.c.1: provisions burn & effective speed scale with crew_alive

**Goal:** Make the two "physical" crew effects from crewing-plan §7 actually do something — provisions burn (§7.2) and effective speed (§7.1) — without yet touching wages or morale.

**Implementation:**
- `Ship::daily_provision_burn(&self) -> f32` returns `crew_alive * 0.0018 tons/day`. `tick_resources` and `provisions_days_remaining` switched to this; the original `ShipStats::daily_provision_consumption` is retained as a "design burn" reference but is no longer called by the live tick.
- `Ship::crew_speed_multiplier(&self, stats) -> f32` implements the piecewise curve: below `crew_min` → 0.0; at `crew_min` → 0.60; at 0.6 of `crew_typical` → 0.84; at `crew_typical` → 1.00; overcrew capped at 1.00. Multiplied into `effective_speed`.
- New unit tests: `crew_speed_multiplier_piecewise`, `provision_burn_scales_with_crew_alive`.

**Design decisions:**
- **Implement the §7.1 annotation, not the formula.** The spec text contradicts itself: the formula `0.6 + 0.4 * (ratio - min_ratio) / (0.6 - min_ratio)` yields 1.0 at ratio=0.6, but the annotation says "60% → 84%". The annotation is internally consistent (continuous, monotone 60→84→100); the formula's other branch starts at 0.84 which only meets the first branch if the first branch ends at 0.84. So we lerp 0.60→0.84 over `[min_ratio, 0.6]` and 0.84→1.00 over `[0.6, 1.0]`. Calibration (3.d) can revisit.
- **Keep `ShipStats::daily_provision_consumption`** rather than delete. Other callers (e.g., `equilibrium.rs`, AI planning lookahead) may want the design-spec burn rate for forecasting "if I had a full crew, how long would my provisions last?". The clear method name (`Ship::daily_provision_burn` vs `ShipStats::daily_provision_consumption`) keeps the distinction visible.

**Considered alternatives:**
- Delete the ShipStats helper entirely: rejected — `crates/sim-core/src/equilibrium.rs` doesn't currently call it, but the AI's voyage-cost estimate at `estimated_voyage_days` uses `speed_typical * 0.55`; if a future planner needs design-burn for ETA-based provisioning checks, the helper is the right thing to call. Marginal API surface, real conceptual distinction.
- Make crew curve discontinuous (spec formula literally): rejected — produces a speed jump (sudden drop from 1.0 to 0.84) at exactly the band boundary, which is calibration-hostile.

**Verification:**
- `cargo test --workspace`: **104 passed** (+2 new in ship::tests).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `bench_trade`: identical Phase-3 baseline (2 bankrupt). Expected: seeded ships start fully crewed, so the crew multiplier is exactly 1.0 and burn rate is unchanged. Shipyard-built ships sail at 60% speed with `crew_min` crew until further hiring lands in 3.c.2 — small enough volume in the 60-day window not to move the verdict.

**Next action:** 3.c.2 — wages, sign-on bounty, discharge (with paid wages flowing into the destination port's `PortMarket.silver` per user direction).

---

## 2026-05-24 — Refactor: extract World::tick cadences (audit follow-up)

**Goal:** Honor the top finding of `planning/code-health-audit.md` — split `World::tick`'s growing per-cadence blocks into named methods before Step 3.c.2 (discharge) and 3.c.3 (hourly morale tick) add more weight.

**Implementation (mechanical, no semantic change):**
- `World::tick` shrunk from ~220 lines to ~15: computes `month` + `pathfind_stats`, then dispatches to:
  - `tick_monthly(month)` — markets, pop dynamics, profit snapshot, shipyard build decisions, snapshot reset. Early-return when `month == self.last_market_month`.
  - `tick_daily_hiring()` — Hiring-state ships drain sailors from local pool. Early-return when `day_of_year == self.last_hire_day`.
  - `tick_hourly_ai_and_physics(month, pathfind_stats)` — builds `PathfindContext`; per-ship AI tick + resource consumption + position advance with land-collision sweep.
- Ordering preserved exactly: monthly → daily → hourly → `advance_hours(1)`.

**Design decisions:**
- **Three private methods, not three pub methods.** External callers always tick the whole hour. Exposing them invites surprising states (e.g., a caller running `tick_monthly` without the matching `advance_hours`). If a future test or scenario needs partial control, we can promote on demand.
- **`pathfind_stats` constructed in `tick()`, passed in.** The PathfindContext also lives there. Building it inside `tick_hourly_ai_and_physics` would have been fine too; keeping construction in `tick()` keeps the per-method responsibilities cleaner ("the hourly method does the per-ship work, given the contexts").
- **Constants stay local to their cadence.** `HIRE_PER_DAY` stays inside `tick_daily_hiring` as a `const`; no need to lift it to module scope until 3.c.2 wants to share with discharge logic.

**Verification:**
- `cargo build --workspace --tests --examples`: clean.
- `cargo test --workspace`: **104 passed**, identical to pre-refactor.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `bench_trade`: bit-identical output (2 bankrupt, same equilibrium deltas).

**Audit status after this commit:**
- Finding #1 (World::tick) — **done**.
- Finding #2 (`ShipBtContext::execute_action`) — deferred to Step 5 opening commit, per audit recommendation.
- Finding #3 (`ShipAI::tick` 8-arg sig) — free with Step 5; no standalone work.

**Next action:** Step 3.c.2 — wages accrual + sign-on bounty + discharge on dock (discharged wages flow into the port's `PortMarket.silver`).
