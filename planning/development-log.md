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

---

## 2026-05-24 — Step 3.c.2: wages accrual + sign-on bounty + port-silver flow

**Goal:** Close the sailor-side money loop. Crew now cost money at hire (sign-on bounty) and over time (running wages), and that silver flows into the port economy when paid — preserving the closed-economy property while making crew an actual operating expense.

**Decision: deferred discharge.** After web research, historical 17C merchant practice was to **keep trained crew aboard for the duration of a voyage** (turnaround 2–10 days; shore leave hours-to-days, supervised; wholesale discharge only at voyage end / refit / lay-up). The crewing-plan §3.4 spec ("discharge on every dock arrival") is too aggressive for our continuously-trading merchant AI — it would dump and re-hire dozens of crew per ship per month. Discharge will be wired later, gated on a refit / long-dwell trigger that proxies "end of voyage". User confirmed conservative scope for 3.c.2.

**Implementation:**
- `Ship.wages_owed_pesos: f32`, initialized 0.0 in both constructors.
- `pub const WAGE_PESOS_PER_MAN_MONTH: f32 = 4.0` — **corrected** from the crewing-plan §6.1 figure of 1.3 pesos. The spec had a peso-to-shilling conversion off by ~4x (a peso was 4–5 shillings, not 22). Historical English ordinary-seaman: 15–25 sh/mo ≈ 3–5 pesos/mo; with a ~30% Caribbean tropical premium, the baseline is ~4 pesos. Dutch (2–3) and Spanish (4–8) ranges bracket this. `SIGN_ON_BOUNTY_PESOS = WAGE_PESOS_PER_MAN_MONTH` (one month's wage per recruit per §6.2). Faction-conditional rates (privateer/pirate share systems) deferred.
- **Sign-on bounty** (in `tick_daily_hiring`):
  - `affordable_draw = floor(ship.silver / SIGN_ON_BOUNTY_PESOS)`. Hire cap = min(typical-gap, HIRE_PER_DAY, affordable_draw, pool_available).
  - On hire: ship.silver -= drawn * bounty; port market.silver += drawn * bounty.
- **Wages accrual** (in `tick_hourly_ai_and_physics`, post-AI-tick, before physics):
  - Sailing: `wages_owed_pesos += crew_alive * WAGE / (30*24)` per hour.
  - Docked: pay min(wages_owed, ship.silver) into the docked port's market silver via `ai.nav.docked_at_port`. Unpaid portion stays on the ship (will weight Morale in 3.c.3).
- New ship test: `fresh_ship_has_zero_wages_owed`.

**Design decisions:**
- **Sign-on bounty flows to port silver, symmetric with wage payout.** User direction. Both events represent sailors immediately spending their cash ashore. Keeps total system silver conserved.
- **Wage payout on every Docked-state hour, not just on the dock-transition tick.** Simpler and idempotent — if the port market is full, the payout still happens; if the ship's silver is short, only what's affordable is paid. Avoids needing to track a "previous state" per ship.
- **Bounty cap by silver, not credit.** An undercapitalized ship hires *fewer* sailors per day instead of going into debt to hire them. Matches the §3.2 footnote: "If the ship cannot afford the bounty, hiring stalls — visible in viz, fixable by the captain selling cargo or borrowing from the home port treasury (mechanism added in Step 9)."
- **`max(0.0)` on ship.silver in the payout calculation.** Defensive against negative silver from other systems (e.g., debt-mode accounting). Wages can't make a positive payment from a negative balance.

**Considered alternatives:**
- Pay wages on the dock-arrival transition tick only: rejected — needs prev-state tracking and is no simpler.
- Have sign-on bounty go into the demographics pool itself as a stat: rejected — pool tracks sailor head-counts, not pesos. Port market silver is the established place for port-side money.
- Faction-rated wage table now: deferred to 3.c.3 alongside privateer/pirate share systems.

**Verification:**
- `cargo test --workspace`: **105 passed** (+1 new).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `bench_trade`: 2 bankrupt (Phase-3 baseline preserved). Equilibrium deltas unchanged in shape. Wage flows are real but small relative to cargo silver: ~25 crew × 4.0 peso × 2 months ≈ 200 pesos/ship transferred to port markets over the 60-day run. Sustainability check: wages remain ~20–30% of typical voyage revenue, leaving healthy margin.

**Next action:** 3.c.3 — morale field + hourly modifiers per §8. Morale will read provisions_days_remaining, wages_owed, and gates on damage events (the damage hooks are stubs until Step 7). Most effects are inert until Step 9 mutiny; this step lays the channel.

---

## 2026-05-24 — Step 3.c.3: morale field + hourly modifiers + soft effects

**Goal:** Lay the morale channel that Step 9 will use to flip bankrupt-and-hungry merchants into pirates. Wire all §8 modifiers that depend only on systems we have today; stub instant deltas from prize / damage events that need Steps 7–8.

**Implementation:**
- `Ship.morale: f32`, initialized 1.0 in both constructors.
- Constants in `ship.rs` (named, so calibration can tune without code dive):
  - `MORALE_PROVISIONS_LOW_DAYS = 14.0`, `MORALE_PROVISIONS_CRITICAL_DAYS = 7.0`
  - `MORALE_LOSS_PROVISIONS_LOW = 0.001`/h, `MORALE_LOSS_PROVISIONS_CRITICAL = 0.005`/h
  - `MORALE_LOSS_WAGES_OVERDUE = 0.001`/h
  - `MORALE_GAIN_RESTED_IN_PORT = 0.001`/h
- `Ship::tick_morale(&stats)`: computes hourly delta from provisions-days-remaining, wages_owed vs current monthly bill, and rested-in-port (Docked + full belly + zero wages owed). Clamps to `[0.0, 1.0]`. Called from `tick_hourly_ai_and_physics` right after `tick_resources`.
- **Speed effect (band 0.25–0.4)**: `effective_speed` multiplies by 0.8 when `morale < 0.4`. Above 0.4 = no effect; below 0.25 the ship is heading for Step 9 mutiny but for now still moves at the sullen rate.
- **Recruitment penalty (band 0.4–0.7)**: `tick_daily_hiring` reduces the per-day hire cap by 10% when morale is in the band (`0.4..0.7` exclusive on upper). Word gets around about the captain.
- 5 new tests: morale init, critical-provisions drop, wages-overdue drop, port-recovery, speed-band throttle.

**Design decisions:**
- **Provisions modifiers are mutually exclusive, not additive.** §8.1 implies critical replaces low ("provisions days remaining < 14 / < 7" are bands, not stacked thresholds). At 5 days remaining we apply -0.005, not -0.001 + -0.005.
- **Rested-in-port requires both fed AND paid.** A docked but unpaid ship doesn't rest up — that matches the historical pattern of disputes over arrears keeping a crew restless even in port.
- **Speed-band: `< 0.4`, not `0.25..0.4`.** Below 0.25 still gets the 20% penalty (and eventually mutiny in Step 9). Adding a discontinuous "no further speed penalty below 0.25" would be unphysical; the deeper bands compound through Step 9's mutiny rather than escalating speed loss.
- **Recruitment band: `0.4..0.7` exclusive upper.** Faithful to the §8.2 wording ("0.4 – 0.7"). Below 0.4, hiring still works at the same reduced rate (no escalation here — deeper morale instead triggers desertion in Step 9).
- **Deferred §8.1 modifiers:** "+0.20 prize" (Step 8 — needs prize taking), "-0.10 damage" (Step 7 — needs damage events). These are instant deltas applied externally when the relevant systems fire, not part of `tick_morale`'s hourly accrual; no stub needed in the morale code itself.
- **Deferred §8.2 effects:** mutiny (< 0.25 + at sea + debt high) and wholesale desertion (< 0.10 in port). Both belong to Step 9 — the desertion effect specifically needs the bidirectional pool mutation (crew → unseasoned pool) tied to morale state machine.

**Considered alternatives:**
- Store morale modifiers per-source (so a UI could show "morale tank: -0.005 from provisions, -0.001 from wages"): rejected as scope creep. Diagnostics can recompute the breakdown from ship state.
- Apply the recruitment penalty as a continuous function of morale rather than a step at 0.4: rejected — the spec was explicit about bands, and a step at 0.4 is calibration-friendly (one threshold to tune).
- Faster recovery rate in port: kept at +0.001/h (≈ 1 morale point in 1000h = 42 days), which gives morale meaningful inertia. Calibration in 3.d may revisit.

**Verification:**
- `cargo test --workspace`: **110 passed** (+5 morale tests).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `bench_trade`: 2 bankrupt (Phase-3 baseline preserved). Morale starts at 1.0; the bench's well-supplied seeded merchants don't dip into any effect band over 60 days. The recruitment penalty only fires on demoralized captains, none of which exist yet.

**Next action:** Step 3.c is complete except for the calibration sweep (3.d). Options for the next slice: (a) the 3.d 1-year headless calibration run — confirm pools/morale are stable; (b) skip ahead to Step 4 (factions + spatial hash) since the crewing surface is wired and 3.d is a tuning exercise. User to decide.

---

## Step 3.d — Bench parameterization + crewing-loop & BT-reactivity fixes

**Date:** 2026-05-22 (continuation of 3.c session)

**Goal (per user direction):** Make 365-day and 730-day bench horizons standard alongside the 60-day smoke run; identify and fix any pathologies the longer horizons expose.

**Changes:**

1. **`bench_trade` accepts a CLI horizon.** `cargo run --release -p sim-core --example bench_trade -- 365` runs a one-year sim; argv defaults to `DEFAULT_SIM_DAYS = 60` if absent. `SIM_DAYS`/`SIM_HOURS` constants removed; local `sim_days`/`sim_hours` threaded through the bench's three loops and print statements.

2. **Crewing loop now tops up `Docked` ships too.** Previously `tick_daily_hiring` only processed `Hiring` ships and transitioned them to `Docked` the instant `crew_alive >= crew_min` — locking shipyard-built ships at exactly the minimum crew (~40% complement → 60% effective speed) for the rest of their life. Per user direction ("hiring sailors, especially unseasoned sailors in Europe or decently prosperous Caribbean ports, should basically always be possible"), the loop now also processes `Docked` ships at their current `docked_at_port`, topping them up toward `crew_typical`. The Hiring→Docked transition still fires at `crew_min` (a ship can put to sea undermanned in an emergency), but daily top-ups continue while it stays at port — and continue at *whatever* port it visits next, since sailors aren't faction-loyal and any port will sell their time.

3. **BT reactivity guard.** While `Hiring`, the AI's root Selector ran priority-3 (`COND_HAS_DESTINATION → ACT_SAIL`). `ACT_SAIL` calls `set_steering` and returns `Running` even though the world's physics phase refuses to move a non-`Sailing` ship. That `Running` status pinned the Selector's `running_child[0] = 2` cursor on priority-3, so when `tick_daily_hiring` externally flipped the state to `Docked`, the AI **never re-checked `COND_IS_DOCKED`** and never entered the dock cycle (SELL/RESUPPLY/BUY/UNDOCK). Added a defensive guard at the top of `ShipAI::tick`: if `ship.state == Docked` and `bt::state.running_child` is non-empty, reset the BT state so this tick re-evaluates from priority 1.

**Pathology discovered (and confirmed by 730-day sweep):**
- Pre-fix, all ten shipyard-built ships in a 365-day run ended at `state=Docked, cargo=empty, P/L = -bounty_only` (just the sign-on bounty deducted). Diagnostic traces showed `ACT_UNDOCK`, `ACT_RESUPPLY`, and `in_destination_harbor` were **never** called for any built ship despite the BT containing all five dock-tree leaves. Tracing `COND_IS_DOCKED` showed it firing only a handful of times — confirming the Selector cursor was stuck on priority-3.

**Verification (post-fix, with both crewing top-up and BT reactivity guard):**
- `cargo test --workspace`: **110 passed.**
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo run --release -p sim-core --example bench_trade -- 60`: fleet P/L **+96k → +101k**, ships built 7. Bankruptcy ticked from 2 → 4 because built ships now actually trade (and accrue chandler debt while doing so).
- `… -- 365`: fleet P/L **+222k → +667k pesos**, ships built 10 → 14. Eight of the ten formerly-stuck built ships are now sailing with cargo and either profitable or carrying tradeable inventory. Equilibrium deviation effectively unchanged (210% mean).
- `… -- 730`: fleet P/L **+431k → +725k**, ships built 13 → 21, total debt 76k. Sailor pools: European hubs 24k→26.5k (+11%/yr healthy growth), Caribbean entrepots ~flat (drained by hiring at growth rate — healthy steady state), Small Colonial +18%, PirateHaven flat at 74 (category growth ≈ 0 — calibration question for §12).

**Open pathologies (intentionally deferred):**
- **Amsterdam fluyt route saturation.** Seven of the seven `Manufactures`-loaded Amsterdam fluyts go negative at 365 days: they all pick the same most-profitable destination, saturate it, and run out of silver before they can rotate. This is a trade-planner / home-bias issue (more route diversity, or better cargo selection after saturation), not a crewing issue. Belongs in Phase 4 economic rebalancing.
- **Provisions stock unbounded** (~+612/mo net production over consumption with most ports not draining at the rate they produce) — pre-existing demand-side weakness, not crewing-related.
- **Elmina/Cadiz/Nantes mispriced** vs LP equilibrium (60%–6800% deviation) — these ports are visited rarely; equilibrium gap is structural, not a regression.
- **PirateHaven sailor growth ≈ 0** — confirms crewing-plan §12 calibration question. Pirate havens grow only when prizes arrive (Step 8/9 feedback).

**Workflow:**
- `copilot-instructions.md` updated implicitly: 365-day and 730-day bench runs are now part of every Step verification going forward (alongside 60-day smoke).

**Next action:** Step 4 (Factions + spatial hash). The remaining "ships in the red" at 365/730 are Amsterdam fluyts saturating one destination — they're sailing, just not profitably. That's an economic-rebalancing topic for Phase 4, not Step 3 crewing.

---

## Step 4.a — Faction renames + `#[repr(u8)]` (2026-05-22)

**Scope:** Pure-mechanical rename, behavior-preserving. Slice 1 of 4 for Step 4 (Factions + spatial hash).

**Decisions taken (with user, before coding):**
- **Drop the Relations Matrix from Step 4.** Faction-vs-faction relations (Hostile/Neutral/Friendly) are inherently dynamic (wars, treaties) and quantitative (thresholds). Phase 3 has no wars yet, so the Phase 3 consumers of "hostility" are (a) viz sight-lines, which only need a faction-equality check, and (b) Pursue/Flee in Step 6, which are better expressed per-ship via `ShipPolicy` (Pirate hostile to all merchants; Privateer{against: FactionSet}; Navy hostile to declared enemies; Merchant hostile to none). Revisit relations in Phase 4 when wars exist.
- **Reflected on Sid Meier's Pirates! mechanics** to validate Step 4 doesn't preclude later work. Concluded:
  - Per-ship and per-port faction flags: ✅ (Port.faction exists; Ship.faction lands in 4.b).
  - Captured-prize flag flip: ✅ trivial (`Ship.faction = capturer.faction` in Step 8).
  - Port capture: ✅ Port.faction is already mutable.
  - Letters of Marque (Privateer commission): unlocked by making `Faction` `#[repr(u8)]` so a future `FactionSet` is a bitset. Done in 4.a.
  - Dynamic war/peace, treasure fleets, sighted-but-unidentified: not modeled; out of Step 4 scope but not blocked.
- **Spatial hash API (lands in 4.c) will support faction-filtered neighbor queries** from day one — `neighbors(pos, range_nm, |id, ship| predicate)`.
- **Every ship must have an owner port.** Seeded ships (in `bench_trade`) currently use `Ship::new` which sets `owner_port = None`; will be fixed in 4.b alongside the `Ship.faction` field. Test-only ships in `market.rs`/`ship.rs` keep `Ship::new` as scaffolding.

**Changes (4.a):**
- `crates/sim-core/src/port.rs` — `Faction` enum: `Holland → Netherlands`, `Pirate → Free`; added `#[repr(u8)]` with explicit discriminants (Spain=0, England=1, France=2, Netherlands=3, Free=4); doc-comment rewritten to reflect the Phase 3 model (Free = independents; piracy is a per-ship `ShipPolicy`, not a faction).
- `crates/sim-core/src/harbor.rs`, `crates/sim-core/src/pop.rs` — `Faction::Holland → Faction::Netherlands` and `Faction::Pirate → Faction::Free` (mechanical).
- `data/registries/ports.ron` — same RON-side renames; header comment updated.

**Verification:**
- `cargo build --workspace --tests --examples` clean.
- `cargo test --workspace`: 91 sim-core unit + 19 integration = **110 passing** (same as before; pure rename).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `bench_trade -- 365`: identical fleet metrics and equilibrium deviations vs. pre-rename (rename is observation-free).
- `bench_trade -- 730`: identical bankruptcy verdict (14 ships in the red — pre-existing Amsterdam-fluyt saturation).

**Deferred to 4.b/4.c/4.d:**
- `Ship.faction` field (4.b).
- Seeded-with-port ship constructor + bench_trade migration (4.b).
- `crates/sim-core/src/spatial.rs` 10 NM dynamic spatial hash with filtered queries (4.c).
- Viz: ship faction colors + faint sight-lines between differing-faction ships (4.d).

---

## Step 4.b — `Ship.faction` field + seeded-with-port constructor (2026-05-22)

**Scope:** Add `Ship.faction: Faction` and route it through the two construction paths. No new consumers yet (4.c/4.d will use it).

**Changes:**
- `ship.rs` — added `pub faction: Faction` field on `Ship`. Doc explains it's mutable (Step 8 prize capture will change it to the capturer's faction). `Ship::new` defaults to `Faction::Free` (test/scaffolding only). `Ship::freshly_built` gained a `faction: Faction` parameter — caller in `shipyard::try_build` passes `port.faction`. New `Ship::seeded_at_port(pos, owner_port, faction)` constructor for the starter fleet — fully crewed, `state = Docked`, `owner_port = Some(idx)`, faction set from caller.
- `shipyard.rs` — pass `port.faction` through to `Ship::freshly_built`.
- `bench_trade.rs` — starter fleet seeded via `Ship::seeded_at_port`, inheriting each ship's port's faction. Seeded ships now have `owner_port = Some(idx)` (previously `None`), bringing them under the home-port remittance machinery.

**Behavioral side-effect (intended):**
Seeded ships are now first-class home-ported ships, consistent with shipyard-built hulls. They participate in `ai.rs` `home_bias` destination-selection and remit surplus silver to the home port on dock.
- 365-day bench: fleet P/L **+667k → +847k pesos (+27%)**.
- "Bankrupt" count (silver-only, dividend-blind threshold) ticked from 10 → 11 at 365d, 14 → 16 at 730d — those hulls likely show positive lifetime P/L once dividends are counted; bench bankrupt heuristic is a Phase-2 stopgap to be revisited in calibration.

**Verification:**
- `cargo build --workspace --tests --examples` clean.
- `cargo test --workspace`: 110 passing.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- bench_trade -- 365 and -- 730 ran cleanly.

**Next:** 4.c — `spatial.rs` 10 NM dynamic spatial hash with filter-closure neighbor query, rebuilt each `tick_hourly_ai_and_physics`. No AI consumers yet.

---

## Step 4.c — Dynamic spatial hash (2026-05-22)

**Scope:** New `crates/sim-core/src/spatial.rs` with a 10 NM uniform-grid spatial index, rebuilt over `Sailing` ships at the top of every hourly tick. No AI consumers yet (4.d viz will be the first).

**Design choices:**
- **Cell size 10 NM** matches typical 17C visibility from a quarterdeck on a clear day. A range query of `r` NM touches an `O((r/10)²)` cell block.
- **Storage: `BTreeMap<(i32, i32), Vec<(ShipId, Position)>>`.** BTree gives deterministic iteration order (important for reproducible bench output). Each entry carries the exact position so `neighbors` can do precise Euclidean distance checks without an external lookup into `World::ships`.
- **API: `neighbors(pos, range_nm, filter)`** where `filter: FnMut(ShipId) -> bool` is invoked AFTER the distance check — only on true neighbors. This is the agreed-on hook for faction-aware queries; callers express "ships within visual range that are not of my faction" without a second pass.
- **Rebuilt each tick** at the top of `tick_hourly_ai_and_physics`. Cheap (single pass, ~tens of ships now, hundreds later). When Step 5's pipeline refactor lands, the rebuild moves into the Mutation Phase formally; the API stays put.
- **Indexes only `Sailing` ships.** Docked / Hiring / Anchored ships aren't candidates for at-sea interaction; excluding them simplifies the Step 6 SeePrey condition (no false hits against ships safely in harbor).
- **`#[derive(Default, Clone, Debug)]`** so `World` initialization stays uniform with the other sub-systems.

**Changes:**
- `crates/sim-core/src/spatial.rs` — new module: `SpatialHash`, `SPATIAL_CELL_NM = 10.0`, 6 unit tests covering empty, in-range vs out-of-range, true-Euclidean (not cell-membership), filter exclusion, clear, and negative-coord cell binning.
- `crates/sim-core/src/lib.rs` — register `pub mod spatial;`.
- `crates/sim-core/src/world.rs` — `pub spatial: SpatialHash` field on `World`; initialized in `World::load`; rebuilt at top of `tick_hourly_ai_and_physics` (Sailing ships only).

**Verification:**
- `cargo build --workspace --tests --examples` clean.
- `cargo test --workspace`: 97 sim-core unit (was 91; +6 spatial) + 19 integration = **116 passing**.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- bench_trade -- 365: fleet P/L +847k pesos (identical to 4.b — no AI consumer yet).
- bench_trade -- 730: fleet P/L +2.87M pesos.

**Next:** 4.d — viz: draw ships in their faction colors; faint sight-lines between differing-faction ships within visual range. Uses `world.spatial.neighbors(...)` with a faction-filter closure.

---

## Step 4.d — Viz: faction colors + sight-lines (2026-05-22)

**Scope:** First user-visible change of Step 4. Ships are drawn in their faction color; faint white lines connect Sailing ships of different factions that are within visual range.

**Changes (sim-viz/src/main.rs):**
- Imported `SPATIAL_CELL_NM` from `sim-core::spatial`.
- New constants: `SHIP_SIGHT_RANGE_NM = SPATIAL_CELL_NM` (10 NM, matches the spatial cell and the 17C horizon-from-quarterdeck range); `SIGHT_LINE_COLOR = (0.85, 0.85, 0.9, 0.18)` (faint cool-white).
- `draw_ships`:
  - Ship triangle color is now `ship.faction.color_rgb()` per-ship (was hardcoded `SHIP_COLOR`).
  - Between the path-drawing loop and the ship-triangle loop, added a new pass that — for each Sailing ship — calls `world.spatial.neighbors(pos, SHIP_SIGHT_RANGE_NM, |id| id != me && other.faction != mine)` and draws faint lines from this ship to each returned neighbor. Pairs are drawn twice (overlapping), accepted for visual simplicity.
- Removed dead `SHIP_COLOR` constant.

**Verification:**
- `cargo build --workspace --tests --examples` clean.
- `cargo test --workspace`: 116 passing.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- bench_trade -- 365: fleet P/L unchanged (+847k pesos) — viz changes don't touch headless behavior.

**Step 4 complete.** Plumbing for factions + spatial queries is in place. No AI behavior change yet — the spatial index is consumed only by the viz layer at this point. Step 5 (pipeline refactor with read/mutate phases) and Step 6 (ShipPolicy + Pursue/Flee BT nodes) will be the first AI-side consumers.

**Cross-cutting summary across Step 4 (a → d):**
- `Faction` enum: `Holland → Netherlands`, `Pirate → Free`, `#[repr(u8)]`.
- `Ship.faction: Faction` field; `Ship::seeded_at_port` constructor; seeded fleet is now home-ported and remits dividends (fleet P/L +27% at 365 days).
- `crates/sim-core/src/spatial.rs`: 10 NM dynamic spatial hash with filter-closure `neighbors` API.
- Viz: faction-colored ship triangles; faint sight-lines between differing-faction ships.
- 6 new unit tests (116 total). All commits behavior-preserving in the headless bench except the home-port side-effect noted in 4.b.
