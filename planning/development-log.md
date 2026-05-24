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

---

## Step 5.a — Extract `execute_action` arms into named methods (2026-05-22)

**Scope:** Pure cosmetic refactor per `planning/code-health-audit.md` §2. The 286-line `match` in `BtContext::execute_action` was extracted into 8 named methods on `ShipBtContext`. Behavior identical at the bench. Opens the door for 5.b (collapse `ShipAI::tick` to take a context) and 5.c (introduce `ShipCommand::Steer` + Resolution Phase) to land arm-by-arm in reviewable diffs.

**Changes (`crates/sim-core/src/ai.rs`):**
- The `impl BtContext for ShipBtContext` block's `execute_action` is now a one-line `match` dispatcher to `self.act_sail()`, `self.act_resupply()`, …, `self.act_divert_to_port()`.
- 8 new methods on `impl<'a> ShipBtContext<'a>`: `act_sail`, `act_resupply`, `act_careen`, `act_undock`, `act_choose_destination`, `act_sell_all`, `act_buy_best`, `act_divert_to_port`. Each carries its original body verbatim (incl. comments) — no logic changes.
- Secondary helper extractions suggested by the audit (`arrive_at_destination_harbor` inside `act_sail`; `outfit_draw_if_home` + `tramping_credit_if_needed` inside `act_buy_best`) are **deferred**: they would help readability further but are not needed for 5.b/5.c, and the diff is already large at 561-line +/-.

**Verification:**
- `cargo build --workspace --tests --examples` clean (first try — no borrow-checker drama).
- `cargo test --workspace`: 116 passing.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `bench_trade -- 365`: fleet P/L **+846,992 pesos — identical** to pre-5.a.

**Atlantic-fleet calibration research** (background agent completed during 5.a): preserved in `~/.copilot/session-state/<id>/files/atlantic-fleet-numbers-1650-1720.md`. **Key takeaway for Step 10 calibration: target ~500 ships baseline for Caribbean basin ~1680, scaling to 800–1000 by 1720.** Current bench runs ~25 ships at 365 days — order-of-magnitude undershoot to be addressed in Step 10.

**Next:** 5.b — promote `ShipBtContext` to a per-tick type owned by `World::tick_hourly_ai_and_physics`; reduce `ShipAI::tick` 8-arg signature to `pub fn tick(&mut self, ctx: &mut ShipBtContext<'_>)`. Drops the two `Option<&mut …>` legacy slots; test scaffolding migrates to real empty markets/goods.

---

## Step 5.b — Split NavState; collapse `ShipAI::tick` signature (2026-05-22)

**Scope:** Audit §3 — but with a richer split than the audit proposed. Per discussion, the *captain's intent* belongs to the AI while the *ship's commitments to the world* belong to the Ship. This survives a future captain swap (player/scripted captains in Phase 4) without losing in-flight nav state.

**Key restructuring:**
- `NavState` split into two `pub` structs:
  - `NavGoal { destination, dest_port }` — lives on `ShipAI.goal` (the captain's "where I want to go").
  - `NavTrack { waypoints, docked_at_port }` — lives on `Ship.nav` (path being followed, current mooring).
- `compute_steering` is now `NavTrack::compute_steering(&mut self, &mut NavGoal, pos, stats, wind, land)` — final-arrival clearing mutates both.
- `set_path` no longer auto-syncs `goal.destination` (it lived on a different struct). Callers (`assign_destination_port`, `replan_to_port`) explicitly update `goal.destination` to the path's terminal waypoint — preserves the prior semantic that arrival is judged against the harbor anchor, not the port's literal coordinate. (First bench run without this resync regressed P/L from +846,992 → +824,704; restoring it returned exact parity.)
- `DockAction` enum **moved** from `ai.rs` to `ship.rs` and renamed in spirit (it describes what the ship is *doing* at dock, not AI cognition). `Ship.dock_action: DockAction` is now a ship field. Imports must use `sim_core::ship::DockAction`.

**`ShipAI::tick` signature change:**
- Old: `pub fn tick(&mut self, ship, stats, wind, ports, harbors, pathfind: Option<&PathfindContext>, markets: Option<&mut [PortMarket]>, goods: Option<&GoodsRegistry>)` — 8 args, two `Option<>` legacy slots for market-less tests.
- New: `pub fn tick(&mut self, inputs: &mut ShipTickInputs<'_>)` where `ShipTickInputs { ship, stats, wind, ports, harbors, pathfind: Option<&PathfindContext>, markets: &mut [PortMarket], goods: &GoodsRegistry }`.
- `ShipBtContext` is now `pub` but still constructed inside `tick` from `ShipTickInputs` + `&mut self.{goal, rng_state}`. This is the "hybrid" shape — slightly short of the audit's "World owns and constructs the ctx" ideal, but `goal` and `rng_state` are genuinely AI state and don't want to live elsewhere. The practical pain (the two `Option<>` slots) is gone.
- Markets/goods are non-Option everywhere; the fallback paths inside `act_resupply`/`act_sell_all`/`act_buy_best` for "no market wired" become harmless `idx >= markets.len()` early-returns. Test scaffolding uses real empty `Vec<PortMarket>` + `GoodsRegistry::starter()` via new helpers `tick_ai` and `tick_ai_with_markets`.

**Caller migrations:**
- `World::tick_hourly_ai_and_physics`: builds one `ShipTickInputs` per ship per tick in a local block (still slot-by-slot — full ship-list double-buffering arrives in 5.c).
- World hiring/wage paths now read `ship.nav.docked_at_port` (not `ai.nav.docked_at_port`).
- `bench_trade.rs` + `viz/main.rs` seeded-fleet inits write `ship.nav.docked_at_port = Some(idx)` and use the new `sim_core::ship::DockAction` path.
- `diag_nav.rs` reads `world.ship_ais[id].goal` + `s.nav`.

**Test migrations:**
- 17 `ai.tick(…, None, None, None)` callsites bulk-converted to `tick_ai(&mut ai, &mut ship, &stats, &wind, &ports)`.
- `ai.dock_action` → `ship.dock_action`; `ai.nav.{destination,dest_port}` → `ai.goal.{destination,dest_port}`; `ai.nav.docked_at_port` → `ship.nav.docked_at_port`.
- Two `tick_until` predicate closures changed from `|_, a| ship.dock_action != X` (captures outer `ship`, conflicts with `tick_until`'s `&mut ship` borrow) to `|s, _| s.dock_action != X` (uses helper's `&Ship` parameter).

**Verification:**
- `cargo build --workspace --tests --examples` clean.
- `cargo test --workspace`: **116 passing**.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `bench_trade -- 365`: **+846,992 pesos** — bit-identical to 5.a baseline.
- `bench_trade -- 730`: **+2,869,818 pesos** — bit-identical.

**Deferred:** Pulling `goal`/`rng_state` off `ShipAI` so `World` can fully own/construct `ShipBtContext` (audit's ideal). Sub-helper extractions inside `act_sail` / `act_buy_best` from audit §2. Neither blocks 5.c.

**Next:** 5.c — introduce `ShipCommand::Steer { heading, commanded_speed }`. `act_sail` pushes a Steer command instead of calling `ship.set_steering`; a new Resolution Phase between AI and physics drains the queue. All other ship/market mutations stay in-place per `phase-3-plan.md` ("Steer as the only command initially").

---

## Step 5.c — `ShipCommand::Steer` + Resolution Phase (2026-05-22)

**Scope:** Plumb the Command/Event pipeline's intent half. Per `planning/phase-3-plan.md` §3 and Step 5, the AI now writes a `ShipCommand` to a buffer instead of mutating the ship directly; a new Resolution sub-step drains the buffer between AI and physics. This is the *minimum-viable* version of the cellular-automata shape — only `Steer` is introduced; per-tick buffer is drained immediately after each ship's AI tick (no cross-ship interactions yet). Sets up the data shape that Step 6+ (`FireBroadside`, `AttemptBoard`, `StrikeColors`) will extend.

**Changes:**
- New module `crates/sim-core/src/command.rs`:
  - `pub enum ShipCommand { Steer { heading: f32, speed: f32 } }`. Doc-comment notes the planned extensions for combat.
- `crates/sim-core/src/ai.rs`:
  - `use crate::command::ShipCommand; use crate::types::{Position, ShipId, WindVector};`
  - `ShipTickInputs` gains `pub me: ShipId` and `pub commands: &'a mut Vec<(ShipId, ShipCommand)>`.
  - `ShipBtContext` gains `me: ShipId` and `commands: &'a mut Vec<(ShipId, ShipCommand)>` (private — flow through `ShipTickInputs`).
  - `act_sail`: the `self.ship.set_steering(s.heading, s.speed)` call is replaced by `self.commands.push((self.me, ShipCommand::Steer { heading: s.heading, speed: s.speed }))`.
- `crates/sim-core/src/world.rs`:
  - `World` gains `pub commands: Vec<(ShipId, ShipCommand)>` (allocation reused across ticks).
  - `tick_hourly_ai_and_physics`: `self.commands.clear()` at the top; per-ship loop now passes `me: id` + `commands: &mut self.commands` in `ShipTickInputs`, then runs a Resolution sub-step that `drain(..)`s the buffer and applies any `Steer` to `self.ships[target]` via `set_steering`. Re-borrows `ship` after the drain (the drain takes `&mut self.ships`).
- `crates/sim-core/tests/ai_behavior.rs`:
  - New `apply_commands` helper (test-side Resolution Phase) applies `Steer` back to the ship.
  - New `dummy_id` helper mints a throwaway `ShipId` via a transient SlotMap.
  - Both `tick_ai` and `tick_ai_with_markets` now construct a local `Vec<(ShipId, ShipCommand)>`, pass it into `ShipTickInputs`, and apply it after `ai.tick` returns.

**Design notes:**
- The Resolution drain happens *per-ship* (immediately after each AI tick), not *per-tick* (after the full AI loop). This is the smallest deviation from pre-5.c semantics — physics still sees the freshly-issued steering on the same tick. Once Step 6 lands and ships need to read each other's pre-resolution states, this will become an after-loop drain.
- Commands are tagged with the issuing `ShipId`. For `Steer` the id always matches the issuer, but tagging now means combat commands (`FireBroadside(target)`, `AttemptBoard(target)`) will need an additional target field rather than a structural change.
- `World.commands` is a `Vec<(ShipId, ShipCommand)>` rather than a `SecondaryMap<ShipId, Vec<ShipCommand>>` because the typical case is one command per ship per tick; a flat Vec keeps the drain trivial and avoids a second allocation per ship.
- `apply_commands` lives in the test file rather than on `ShipCommand`/`World` because the production drain has knowledge of `self.ships` (target lookup) that doesn't generalize. A public `apply_steer` helper could be extracted in Step 6 once there's a second caller.

**Verification:**
- `cargo build --workspace --tests --examples` clean.
- `cargo test --workspace`: **116 passing**.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `bench_trade -- 365`: **+846,992 pesos** — bit-identical.
- `bench_trade -- 730`: **+2,869,818 pesos** — bit-identical.

**Next:** Step 6 — `Ship.policy: ShipPolicy`; `Pursue` / `Flee` BT nodes; `SeePrey` condition consults spatial hash + faction relations + policy; hardcoded pirate-sloop spawn near Tortuga. First visible Phase 3 behavior.

---

## 2026-05-22 (cont) — Nav-1+2+3: estimated_position + DR error + noon-sight/landmark fixes

**Plan reference:** `planning/navigation-plan.md` items Nav-1 (estimated_position), Nav-2 (DR error), Nav-3 (noon sight + landmark fix). Storms, latitude-sailing, replan triggers, and heave-to are deferred to Phase 4.

**Why now:** the 5.b NavGoal/NavTrack split made the captain's belief-state a natural place to hang a separate position estimate. Adding it now (before Phase 3 combat) means the chase BT in Step 6 sees prey through the navigator's eyes, not via omniscient truth.

**Changes:**
- `nav.rs`:
  - `NavGoal { estimated_position: Option<Position>, last_noon_sight_day: u16 }`.
  - Constants: `DR_ERROR_LAT_NM_PER_HOUR=0.05`, `DR_ERROR_LON_NM_PER_HOUR=0.15`, `LANDMARK_SIGHT_NM=20.0`, `NOON_SIGHT_NOISE_NM=0.5`, `LANDMARK_FIX_NOISE_NM=1.0`.
  - Helpers: `xorshift64`, `uniform01`, `gaussian` (Box-Muller), `apply_dr_error`, `try_noon_sight`, `try_landmark_fix`.
  - `compute_steering` now takes both `pos_estimate` and `pos_truth`:
    - Waypoint advance uses **estimate** (captain crosses off waypoints as he believes he passes them).
    - Final arrival check uses **truth** (a ship has arrived when its hull is at the harbor — prevents premature "arrival" when estimate drifts to the dest while truth is still mid-voyage).
    - Reactive `deflect_for_land` uses **truth** (lookouts see real breakers, not the captain's mental coastline) — this is the explicit "no accidental grounding" fix.
- `ai.rs`:
  - `ShipAI { prev_truth: Option<Position>, nav_rng_state: u64 }`. The nav RNG is independent of the destination-choice RNG so noise rolls don't perturb economic decisions.
  - Navigator pass at top of `tick`: lazy-init estimate from truth → advance by `truth - prev_truth` (DR plot) → add DR noise scaled by speed → noon sight (lat only, once/day) → landmark fix (only while `speed > 0.1`, so docked ships don't accumulate fix noise around a known dock).
  - `ShipBtContext::estimated_position()` helper; 9 BT/action call-sites replaced `self.ship.position` → `self.estimated_position()` (planning, conditions, destination choice).
- `world.rs`: passes `day_of_year` through `ShipTickInputs`.
- `tests/ai_behavior.rs`: helpers pass `day_of_year: 0` (disables noon sight in unit tests).

**Design notes:**
- **Why separate `nav_rng_state`:** without this, `gaussian()` calls advance the shared `rng_state`, shifting the sequence consumed by `act_choose_destination`. With even zero noise this caused destination divergence (verified: zero-noise nav with separated RNG is bit-identical +846,992 to baseline). The separation makes "tune the noise rates" a pure perturbation problem.
- **Why DR = motion-delta + noise:** the first iteration treated DR as pure noise; the captain's estimate didn't track the ship at all (estimate stayed near origin while truth sailed away). Real-world DR plot advances with each leg sailed; cumulative *errors* are the noise.
- **Why arrival uses truth:** premature arrival → BT clears the goal → picks new destination → ship never actually docks → bankruptcy. The truth check is the single line that keeps the economy from collapsing.
- **Why landmark fix gated on speed > 0.1:** at a dock, truth doesn't move but fix noise would jitter the estimate ±1 NM every tick (24/day) around a position the captain already knows perfectly. This was responsible for ~750k of the P/L regression in the un-gated version.

**Verification:**
- `cargo build --workspace --tests --examples`: clean.
- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: **116 pass** (97 + 19).
- `bench_trade -- 365`: **+1,451,474 pesos** (vs +846,992 baseline, +71%), 9 bankrupt (vs 11 baseline).
- `bench_trade -- 730`: **+4,552,932 pesos** (vs +2,869,818 baseline, +59%), 10 bankrupt. 730/365 ratio 3.13x, matches baseline 3.39x — confirms no compounding regression.

**Why P/L improves under uncertainty (diagnosed):**

The investigation took several iterations and the answer turned out to be two effects, one of them not really about nav.

**1. Real nav bug (fixed):** the first cut had P/L explode to +25.6M at 730d. Root cause was `in_destination_harbor` using the captain's *estimate*. DR drift could put the estimate inside the harbor polygon before the real hull was — triggering instant docking, profit settlement, and a new outbound voyage while the actual hull was still mid-ocean. Effectively a teleport-into-port loop. Fixed: harbor-zone check now uses truth (same principle as the final arrival check; the pilot/lookout recognizes the actual harbor on physical entry, not the captain's mental coastline).

**2. Residual +59% lift is the pre-existing home-bias feedback loop, exposed by nav perturbation.** Per-ship breakdown:

| Ship | Home | Baseline div. | Current div. |
|---|---|---|---|
| 0 | Bridgetown sloop | 1,236,403 | 2,654,275 |
| 5 | Havana sloop | 41,809 | 326,079 |
| 3 | Charleston sloop | 467,774 | 38,956 |
| (most others) | — | similar | similar |

Even at baseline (no nav changes at all), ship #0 alone earned 43% of total fleet dividends. The home-bias formula in `act_buy_best` (`bias = surplus_silver/200`, capped 200 pesos/ton) creates positive feedback: once a ship accumulates silver above the operating float, the bias dominates trade selection (a 200 peso/ton bonus is more than most genuine arbitrage margins), locking the ship into short home-and-back loops with the most-profitable nearby partner. Those loops compound the dividends rapidly.

Nav noise doesn't *break* anything — it simply perturbs which ships escape their early-trip inefficiencies and enter the lock-in. Different ships flip into runaway mode; some that were stuck in baseline now thrive, and one (#3) that was thriving in baseline now stalls. The fleet total grows because, on net, more ships make it into the bias-locked state.

**This is an economic-model issue, not a navigation issue.** It would surface under any sufficiently-large perturbation (different seed, different RNG ordering, etc.). Documented here so we know to address it during Step 10 calibration — likely candidates are (a) soft-cap the home bias, (b) make the bias kick in only above a much higher silver threshold, (c) decay surplus silver into the port treasury more aggressively, or (d) make the trade planner consider multi-hop routes.



**Dock-count diagnostic confirms.** Added `Ship.lifetime_dock_count` (incremented in `ship.dock()`) and a new `docks` column in the bench. Top earners:

| Ship | Docks (730d) | Cycle | Per-dock profit |
|---|---|---|---|
| #0 Bridgetown sloop | 309 | ~56 hr (~336 NM rt) | 8,600 |
| #1 Port Royal sloop | 579 | ~30 hr (~180 NM rt) | 660 |
| #6 Fort-Royal sloop | 126 | ~140 hr | 3,200 |
| #8 Amsterdam sloop | 316 | ~55 hr | 257 |

These are physically realistic trip rates (sloop ~6 kt, ~144 NM/day). Ships are NOT teleporting or re-docking artificially — they are doing many real round-trips on routes with large persistent price gaps. The bench's own "Top 10 mispriced cells" output corroborates: e.g. San Juan provisions priced 4.7 vs equilibrium 1.3 (253% over), Cayenne provisions 37.8 vs 14.4 (162% over). A sloop running provisions on a 180 NM hop at those margins genuinely earns ~660 pesos per cycle.

**Two distinct asks for Step 10 (filed):**
1. *Home-bias formula* (`act_buy_best`): the `surplus/200` cap-200 bias dominates real arbitrage once a ship clears its float, creating a winner-take-all dynamic where one ship per port locks into the most-profitable short loop. Consider raising the threshold or making the bias decay.
2. *Market equilibration*: prices in the calibration data sit 150–250% above LP equilibrium on multiple goods/ports, and persist under heavy trading because prod/cons restock between trades. Either tighten the restock rate, slow the price clamp, or strengthen the carrying-trade markup.

Neither belongs in Nav-1+2+3.

**Verified no auto-dock-on-zone-entry.** `in_destination_harbor` matches only the BT-chosen destination port, not any harbor in range. A ship sailing to Charleston that wind-drifts through Bridgetown'''s zone does NOT re-dock at Bridgetown. Docking requires BT intent (which set the destination via `assign_destination_port`) AND physical entry of that specific port'''s harbor zone.

**Truth-based waypoint advance considered and rejected.** Tried it as an alternative to estimate-based; caused a +109M runaway at 730d. Almost certainly ships orbiting a waypoint their estimate-biased heading prevents them reaching (truth waypoint check never fires → ship keeps sailing toward a moving target that never advances). Reverted. The design intent ("captain crosses off waypoints as he believes he passes them") stands and is documented inline.

**Decisions ratified in this slice:**
- Steering bearing: estimate (captain steers from where he thinks he is).
- Waypoint advance: estimate (he crosses off waypoints from his DR plot).
- Final arrival clear: truth (a ship has arrived when its hull is at the harbor).
- Harbor zone entry: truth (the pilot/lookout recognizes the actual harbor).
- Reactive land deflection: truth (lookouts see real breakers).
- Landmark fix: gated on `speed > 0.1` (no fix-noise jitter at the dock).
- nav_rng_state separate from rng_state (zero-noise nav is bit-identical to baseline).

**Next:** Step 6 — `Ship.policy: ShipPolicy`; `Pursue` / `Flee` BT nodes; `SeePrey` condition consults spatial hash + faction relations + policy; hardcoded pirate-sloop spawn near Tortuga. The chase BT will read `estimated_position()` for prey detection range, completing the loop.

---

## Step 6 — Pursue / Flee BT

**Scope shipped:**
- `ShipPolicy { Merchant, Pirate }` enum on `Ship`. Defaults to Merchant.
- `NavGoal { pursue_target: Option<ShipId>, flee_from: Option<ShipId> }` for cross-tick chase/flee state.
- Two new high-priority BT branches (above trade, below docked):
  - Sequence(IsSailingPirate, SeePrey, Pursue) — pirates chase richer or slower ships within 12 NM.
  - Sequence(IsSailingMerchant, SeeThreat, Flee) — merchants run from any pirate in 12 NM.
- `VISUAL_RANGE_NM = 12.0` (matches `nav::ARRIVAL_NM`); `PURSUE_BREAKOFF_NM = 24.0` for hysteresis (once locked, target sticks until it slips outside the wider band).
- New per-tick `ShipSnapshot { position, policy, faction, max_speed, cargo_capacity_tons }` map built alongside the existing spatial-hash rebuild. Lets one ship's AI inspect any other Sailing ship without taking a second borrow on `world.ships` (which is already mut-borrowed for the active ship).
- `World::spawn_pirate_sloop_at(name, seed)` helper. Bench and viz both seed pirates at **Tortuga / Petit-Goâve / Nassau**.
- 3 new tests in `tests/ai_behavior.rs`: pirate-sees-and-pursues; merchant-flees-when-pirate-in-range; pirate-ignores-other-pirate.

**Design choices:**
- Detection: pirate prey filter = non-pirate ship with `cargo_capacity > self || max_speed < self`. Since sloops are the smallest hull in the registry, "richer" catches every brig/bark/ship/fluyt; "slower" catches a laden sloop. Threat filter for merchants: any ship with `policy == Pirate`. Relations matrix deferred to Phase 4 (when wars exist).
- Pursue/flee never touches `goal.destination` — when the encounter ends, the merchant naturally resumes the original trade voyage.
- Sailing-only: docked pirates don't undock to chase; docked merchants can't run from the dock.
- Pirates use truth-position to steer toward prey (lookouts see real hulls on the horizon); merchants use estimate to locate nearest port (a lost merchant runs toward where he *thinks* safety is).
- `act_flee` adds a small evasive bias: if the nearest port lies within 30° of the threat bearing, sheer 90° off the threat instead of sailing straight at it.

**Verification:**
- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: **119 pass** (was 116; +3 Step 6 tests).
- `bench_trade -- 365`: **+1,339,944** (vs +1,451,474 baseline Nav-1+2+3); 10 bankrupt (vs 9). The three pirate sloops contribute +715k of their own P/L (trade activity at home haven when no prey is in range — pursue/flee is a high-priority *override*, but the BT falls through to trade when nothing's chaseable). Merchants in pirate-frequented waters take a small hit from flee detours.
- Per-ship table now visibly identifies the pirate sloops (rows 10–12: Tortuga +258k, Petit-Goâve +444k, Nassau +13k).

**Caveat:** Step 6 is movement-only — pirates "catch" prey but nothing happens (no combat). Step 7 will add `ShipCommand::FireBroadside` and a damage model.


## Step 7 — Gunnery & damage events (2026-05-22)

**Goal:** turn Step 6's pursue/flee chases into actual ship-to-ship combat. Pirates and merchants exchange broadsides when within range; hull and rigging damage accumulate. Sinking, boarding, and morale impact are deferred (Steps 8/9). All damage is deterministic for now — a calibration RNG can layer on in Step 10.

**What landed:**
- New `combat` module (`crates/sim-core/src/combat.rs`): single damage formula `compute_broadside_damage(cannons, range_nm) → (hull, rig)` with linear falloff from 1.0× at point-blank to 0.3× at `CANNON_RANGE_NM = 0.5 NM` (~1000 yd). Hull base 0.5/gun, rigging base 0.3/gun. Powder & shot cost 0.01 t/gun/broadside.
- New `ShipCommand::FireBroadside { target }` variant; emitted by `act_pursue` and `act_flee` whenever the target is within `CANNON_RANGE_NM` and the attacker has both `Gunpowder` and `Cannon Shot` in cargo. Resolved in the existing Resolution Phase by reading attacker cannons + position, deducting powder + shot from the attacker, and saturating-down hull/rigging on the target.
- `ShipStats` gains 3 fields: `cannons`, `hull_integrity_max`, `rigging_integrity_max` (RON-loaded; sloop 8/100/80, brigantine 12/130/100, bark 14/160/120, fluyt 12/180/130, ship 24/400/200).
- `Ship` gains 2 runtime fields: `hull_integrity`, `rigging_integrity`, init to stats max in both constructors. `Ship::effective_speed` now multiplies by `rigging_integrity / rigging_integrity_max` — knocking out a chaser's rigging is how a slower merchant breaks contact.
- Two new goods: `Gunpowder` (id 9) and `Cannon Shot` (id 10). Light monthly production at London (8t/12t), Amsterdam (6t/10t), and Cadiz (3t/5t) — period-correct: those were the major Atlantic powder mills and arsenals. No port consumption — only ships in combat burn them.
- `act_sell_all` now skips the magazine (powder/shot stay aboard across dock visits) and calls `replenish_ordnance` to top up to 4t each for pirates / 1t each for merchants. Built ships pick up their first magazine on the first dock cycle at their home port.
- Bench `bench_trade.rs`: 2 new columns `hull%` / `rig%`; seed merchants start with 1t+1t. Pirate spawn helper seeds 4t+4t at the haven port.
- New test file `tests/combat.rs` with 5 integration tests (formula falloff, rigging-speed coupling, supply-cost scaling, end-to-end FireBroadside damage, no-fire-without-supply). Existing `tests/ai_behavior.rs` updated to handle the new `ShipCommand` variant in its test-side resolver.

**Design choices:**
- **Deterministic damage, no RNG.** Per user preference: "simple version for now, fancier later". A crit/miss roll can layer over the falloff formula in Step 10's calibration pass without touching the wiring.
- **Both sides fire while in range.** Merchants don't drop their bowls when chased — `act_flee` also pushes FireBroadside. Captures the "armed merchantman" reality of the 1670–1720 Caribbean.
- **Cannons are a static stat, not a Good (yet).** Promoting cannons to a Good (so ports can build them up over time, ships can rearm at home, captured prizes inherit guns) is a future step. For Step 7 a fixed `stats.cannons` per ship type is the smallest change that makes combat meaningful.
- **Magazine stays aboard.** `act_sell_all` filters out powder/shot to keep the trade AI from accidentally disarming the ship between dock visits. Pirates top up to 4t/4t (deep magazine because hunting is full-time), merchants to 1t/1t (defensive ration).
- **`CANNON_RANGE_NM = 0.5`** (≈ 1000 yd). 0.25 NM was the "real" engagement distance in the period, but at 1-hour tick granularity a 0.25-NM window leaves only one tick to fire before the chase resolves; 0.5 NM gives the pursuer 2–3 ticks of actual gunnery. Tunable in Step 10.
- **Hull and rigging decouple cleanly.** Rigging immediately throttles speed (visible effect), hull is the bookkeeping that Step 8 will turn into sinking thresholds. Both saturate to 0 in Step 7 (a fully shot ship still floats — comes back into play at Step 8).

**Verification:**
- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: **130 pass** (was 119; +6 combat unit tests + 5 combat integration tests). Existing `ai_behavior` tests untouched.
- `bench_trade -- 365`: **+1,999,209** fleet P/L (vs Step 6 baseline +1,339,944), 3 bankrupt (was 3). Combat visible in per-ship table: ship 13 (Boston-built bark) hammered to **0% hull / 0% rig** by the Tortuga & Petit-Goâve pirates — still floats (Step 7 design: no sinking yet), but now creeps along under shredded canvas. Petit-Goâve sloop holds full 4t/4t magazine (resupplied each dock cycle); Tortuga sloop ran out and is back on trade behavior. Fleet P/L jump comes partly from new powder/shot trade flows and partly from merchants making more round-trips with armed ordnance reducing pirate effective harvest.

**Caveat / known follow-ups:**
- A hulled ship at 0% integrity is a zombie until Step 8 lands sinking + wreck removal.
- Built ships start unarmed and only acquire ordnance on the first dock cycle at their home European port — first-voyage outbound run is undefended. Acceptable for Step 7; Step 8/9 will tie magazine outfitting to the shipyard build phase.
- The damage scalars (0.5/0.3/gun, 0.5 NM range, 0.01 t/gun) are sketches. Step 10's calibration sweep will tune them against equilibrium pirate/merchant attrition rates.

---

### Post-Step-7 bugfix: ship ping-pong at provisions-dry havens (commit `52eb1cd`)

**Bug:** 730d bench showed ship 7 (London-seeded sloop) accumulating 2,785 docks (~12/day) once English Harbour's chandler stockpile depleted around day ~510.

**Root cause:** `tick_resupply_at_market` returns `done = true` whenever the port stockpile is empty (interpreted as "tried", not "got fed"). Combined with `act_divert_to_port` picking the geographically nearest port without regard to its actual stock, a low-provisions ship that arrived at a dry sugar island would: dock → resupply succeeds vacuously → undock still hungry → next tick's COND_IS_LOW_PROVISIONS fires again → divert chooses the same dry port → infinite loop. The two existing dock-loop tests didn't cover this path because they ran on empty test markets where divert wasn't gated on stock.

**Fix:** `act_divert_to_port` now filters port candidates by `markets[idx].stockpile.get(PROVISIONS) > 0.5` and returns `Status::Failure` if no qualifying port exists, letting the BT fall through to the normal sail-to-destination branch instead of looping back to the same dry harbor.

**Bench impact (730d):** ship 7 docks 2,785 → 37, P/L \$83k → \$197k; fleet total P/L +2.33M → +2.49M; bankrupt ships 7 → 4.

**Other 730d oddities noted for future investigation (not bugs blocking Step 5.a):**
- **Ship 6 Fort-Royal: +\$1.06M P/L** — 8× the next-best earner. Possibly exploiting a high-margin sugar-island-to-European-hub route on a sloop. Worth tracing in Step 10 calibration.
- **4 remaining bankruptcies** — mostly newly-built Amsterdam fluyts and a Cartagena/Nantes sloop. The shipyard may be over-producing fluyts relative to round-trip cadence. Calibration pass.
- **Equilibrium price LP deviation: mean 210%, max 6,851%** (Elmina Tobacco) — pre-existing structural mismatch between sim prices and analytical LP equilibrium; not a regression.
- **Tortuga pirate runs out of powder/shot mid-sim** — no Caribbean powder production yet. A future Caribbean import / raid-resupply line would let the haven sustain piracy.
- **Ship 13 (Boston-built bark) at 0% hull/0% rig still listed "sailing"** — expected; Step 8 will add sinking + wreck removal.

## Step 8 — Boarding, sinking, closest-approach combat gating (2026-05-22 cont., commit `834726f`)

Per `planning/phase-3-plan.md §8`. Adds the violence half of Phase 3: pirates can now actually take ships, and broadsides actually connect.

### The hourly-tick granularity problem

Discovered while designing the boarding range: with hourly ticks and ships moving 5–8 kt, two ships closing at 2+ kt of Δspeed pass through each other's combat envelope in a single tick. End-of-tick distance alone made the 0.5 NM cannon range a near-miss most of the time, and boarding's natural 0.05 NM (actual grapple distance) basically unreachable. Step 7 broadsides were almost certainly under-firing — we just didn't have ship-pair instrumentation to notice.

**Fix:** `combat::min_distance_over_tick(a_pos, a_vel, b_pos, b_vel)` — linear interpolation of relative position over the unit tick interval, returns the minimum `|r(t)|` for t ∈ [0, 1]. Now used as the range gate for both `FireBroadside` (Step 7 retroactive fix) and `AttemptBoard` (Step 8 native). `ShipSnapshot` grew `velocity` and `rigging_frac` so the AI can compute closest-approach and gate on rigging without re-borrowing the ships map.

### Boarding mechanics

- **Command:** `ShipCommand::AttemptBoard { target }`.
- **AI gate:** in `act_pursue`, pirates emit `AttemptBoard` after `Steer` + `FireBroadside` when target rigging < 30% of max (the "dismasted enough to grapple" condition) AND closest-approach this tick < 0.05 NM.
- **Resolution:** `combat::resolve_boarding` is a pure function — `force = crew * (1 + 0.5 * morale)`, larger force wins, tied force goes to defender (home-deck advantage). Casualty rates: 20% winner, 65% loser. Defender takes losses first; if attacker wins, half its surviving crew transfers as prize crew. Prize gets `policy = Pirate`, inherits attacker's `faction`, morale reset to 0.8, nav cleared. If the prize crew split would drop the attacker below `stats.crew_min()`, the prize is burned instead (`state = Sunk`).

### Sinking and Cleanup Phase

- `ShipState::Sunk` variant added. Set by Resolution when `hull_integrity <= 0` (broadside kill) or by the burn-prize path.
- New Cleanup Phase at the end of `tick_hourly_ai_and_physics` sweeps Sunk ships out of the SlotMap. SecondaryMap entries (`ship_ais`, `silver_at_month_start`) removed alongside. SlotMap generation bumps, so the `ShipId` becomes permanently invalid — no ghost references.

### Bench impact (730d)

- Fleet total P/L: +2.49M → **+3.41M** (+37%). All from broadsides actually firing via closest-approach — boarding doesn't transfer cargo so doesn't directly inject silver.
- Bankrupt: 4 → **7**. Long-tail merchants caught in pirate windows.
- **Combat ledger: 5 pirate(s) afloat (3 seeded + 2 captured), 0 sunk.** Step 8's first real signal — 2 successful boardings turned merchant prizes (Boston bark, Amsterdam fluyt) into pirates. The captured ships show 9% hull / 27% rig — signature of a long-running brawl.
- No outright sinkings yet — gunnery alone isn't lethal enough to hull-zero a merchant before boarding kicks in. Calibration (Step 10) will tune.
- 365d run shows +10% P/L vs pre-Step-8 (1.999M → 2.196M) — confirms the closest-approach gunnery effect compounds over horizon.

### Test coverage

141 tests pass (was 130). Added:
- 5 `combat` unit tests for `min_distance_over_tick` (stationary, crossing paths, passes-close-during-tick, clamping)
- 4 `combat` unit tests for `resolve_boarding` (force decides outcome, tie goes to defender, morale can flip, losses clamp to crew)
- 3 integration tests in `tests/combat.rs`: prize-taking (policy + faction + morale flip), Cleanup reaping a hull-zero ship, under-crewed pirate burns the prize

### Known follow-ups (logged, not blocking)

- Captured prizes still hold their original merchant cargo — Step 9 territory (the prize crew presumably sells it at the next haven). Worth a separate look once mutiny lands.
- Tortuga and Nassau pirates run their magazines down to 0.4 t over 730 days (started with 4 t each). No Caribbean powder production means they would starve out long-term. Step 10 calibration question: should boarders salvage powder/shot from prizes?
- Equilibrium LP deviation (max 6,851% at Elmina Tobacco) unchanged from pre-Step-8 — still a structural pricing question separate from combat.

## Step 9 — Bankruptcy → Piracy mutiny trigger (commit `c237181`)

Closes the Phase 2 economic loop into Phase 3 violence: a chronically bankrupt merchant at sea whose crew has finally given up flips `policy = Pirate` mid-voyage. The captain doesn't get a vote.

### Trigger condition

`policy == Merchant && state == Sailing && (debt + wages_owed_pesos) > MUTINY_DEBT_THRESHOLD && morale < MUTINY_MORALE_THRESHOLD`

- `MUTINY_DEBT_THRESHOLD = 1.5 × MAX_SHIP_DEBT = 7500 pesos`
- `MUTINY_MORALE_THRESHOLD = 0.25` (deep in the "sullen / mutinous" speed-throttle band)

The plan originally specified `debt > MAX_SHIP_DEBT * 1.5` only, but the chandler-credit system caps raw `debt` at `MAX_SHIP_DEBT = 5000`, so 7500 is unreachable from debt alone. Combining `debt + wages_owed_pesos` makes the threshold mean what it should: total financial distress the crew can feel. A bankrupt ship at the chandler ceiling needs another ~20 months of unpaid wages to cross over.

### Mutiny effects

- `policy = Pirate`. The new pirate captain re-plans next tick (NavGoal cleared, waypoints flushed).
- `debt = 0`, `wages_owed_pesos = 0`. Mutineers torch the books — debt to chandlers ashore and obligations to officers they just murdered are no longer their problem.
- `morale = MUTINY_POST_FLIP_MORALE = 0.55`. Not euphoria, just a reset.

### New morale wiring (the missing modifiers from the plan)

- **`MORALE_LOSS_DEBT_HEAVY = 0.0015/hr`** when `debt >= MAX_SHIP_DEBT`. Maxed-out chandler credit = crew knows the captain is sunk. Drops morale ~0.036/day at the cap.
- **`MORALE_GAIN_PRIZE_TAKEN = 0.30` one-shot** applied to the boarding attacker on a successful capture (Step 8 wiring). The "rises with prize money" half of the plan.

### Diagnostics

New `World.mutinies_total: u32` cumulative counter. Bench Combat ledger split into `seeded + captured + mutinied`. Total pirates afloat = sum.

### Bench impact (730d)

- Fleet P/L: **+3.48M** (vs +3.41M pre-Step-9 — small lift from heavy-debt morale loss compounding slightly into trade behavior, plus 1 fewer capture in the deterministic-seed roll).
- Bankrupt: **8** (vs 7 pre-Step-9 — heavy-debt morale loss now starts biting before bankruptcy lock-in).
- **Mutinies: 0**. The trigger is conservatively sized; bankrupt fluyts stay above the 0.25 morale floor because they dock often enough that the rested-in-port morale gain offsets the bleed. This is the realistic outcome — historical mutinies were tail events, not routine — but Step 10 will revisit calibration once we can run year-on-year fleets and see whether the long-tail rate should be higher.

### Tests (147 total, was 141)

6 new in `ship.rs`:
- `morale_drops_on_heavy_debt` — single-tick verification
- `mutiny_flips_indebted_low_morale_merchant_at_sea` — happy path, asserts policy + cleared debt + new morale
- `mutiny_does_not_trigger_when_docked` — state guard
- `mutiny_does_not_trigger_below_thresholds` — both gates fail-closed
- `mutiny_triggers_on_wages_pushing_total_over_threshold` — the debt+wages combination
- `mutiny_ignores_already_pirate_ships` — policy guard

### Known follow-up

- 730d shows zero mutinies. Either the threshold is too high in practice or the morale system doesn't drop bankrupt crews fast enough. The hypothesis is that frequent port docking + the rested-in-port morale gain (0.001/hr) cancels out the at-sea bleed. Step 10 calibration question: should the rested-in-port gain be gated on the ship being in funds (already half-gated on `wages_owed <= 0`)?
- Captured ships and mutinied ships both end up Pirate-policy, but with different starting conditions (captured: morale 0.8, fresh prize crew; mutinied: morale 0.55, full original crew minus officers — currently we don't model the officer kill). Eventually worth distinguishing.

## Step 10.a — Historical fleet scale-up (commit `a042931`)

The bench had been running on a hand-picked 13-ship fleet (10 merchants + 3 pirates) since Step 6. That was an order of magnitude under the historical baseline: per `planning/research/atlantic-fleet-numbers-1650-1720.md`, the Caribbean basin c. 1680 supported roughly **400–800 active hulls**, with a recommended simulation baseline of ~500. Step 10.a scales the bench up to that range so the calibration pass can run against realistic load.

### Design

`Ship::seeded_at_port_typed(pos, port, faction, ship_type, &stats, silver)` is a parameterized starter constructor — like `seeded_at_port`, but with explicit type/stats/silver so a single seed function can produce a mixed fleet.

`World::seed_historical_fleet(base_seed) -> Vec<ShipId>` iterates `world.ports` and, per `PortCategory`, picks a target count and a faction-appropriate ship-type mix:

| Category            | Ships/port | Typical mix                              |
|---------------------|-----------:|------------------------------------------|
| `EuropeanHub`       | 30         | fluyt-heavy with ships and brigs         |
| `CaribbeanEntrepot` | 25         | balanced merchant + a few warships       |
| `SmallColonial`     | 8          | mostly sloops + brigantines              |
| `PirateHaven`       | 6          | sloops/brigantines, policy=Pirate        |

Per-port RNG is `LCG(base_seed + port_idx)` so the same `base_seed` always produces the same fleet. Pirate-haven ships flip `policy = Pirate`, `faction = Free`, and start with 4/4 powder/shot instead of the merchant default of 1/1. Starting silver scales with cargo capacity (`max(1500, cap × 25)`), deliberately leaner than the shipyard's ~30/ton so the world doesn't start drowning in cash.

The bench replaces the manual ship list with a single `world.seed_historical_fleet(0xCAFE_1680)` call. The per-ship printer (unreadable at 500 ships) is gone, replaced with a `(faction, type)` aggregate table plus top-5 and bottom-5 P/L outliers. The Combat ledger is preserved and now reads e.g. `67 pirate(s) afloat (24 seeded + 3 captured + 40 mutinied)`.

### Results

Bench produces **503 ships** across 38 ports. Performance is linear in sim time:

| Sim days | Wall (release) |
|---------:|---------------:|
| 60       | 2.7 s          |
| 365      | 17 s           |
| 730      | 47 s           |

At 730d: fleet P/L +8.66M pesos, 81 bankrupt (16%), 67 pirates afloat (24 seeded + 3 captured + **40 mutinied**), 3 captured by combat, 3 lost. Step 9's mutiny trigger is now visibly firing — at the prior 33-ship scale it never crossed the threshold.

### Production sanity check

Before scaling, asked: does the world produce enough provisions to feed 500 ships? Calculation:

- Per-man burn: 0.0018 t/man/day (≈ 4 lbs/man/day dry food). This matches the 17C Royal Navy victualling allowance (1 lb biscuit + ~2 lb salt meat + peas/cheese; Davis, *Rise of the English Shipping Industry*). Beer/water (~1 gal/man/day = ~8 lbs) is not tracked as "provisions" in the goods catalog.
- Weighted average crew across our ship-type mix ≈ 25 (sloop 15, brigantine 22, bark 50, fluyt 25, ship 80).
- Fleet demand: 500 × 25 × 0.0018 × 365 ≈ **8,200 t/yr** at full activity.
- World production: **9,552 t/yr structural** (sum over all 38 markets of `recipe.monthly_outputs * prosperity * 12`). After port-population consumption (2,208 t/yr), net available to fleet: **7,344 t/yr**.

So fleet need is roughly the same order of magnitude as net production — about a 10% structural shortfall at full activity, well within the slack of ships sometimes sitting in port. The 16% bankruptcy rate is therefore **not a global provisions shortage** — it's a distribution/economics problem (fluyts over-built relative to demand for their slow-but-bulky niche; small colonial ports occasionally starving while European hubs are flush). That's a Step 10.b or later concern.

### Open questions for later calibration

- Fluyt over-supply: most bottom-5 P/L outliers are Amsterdam fluyts. Either the type mix in `EuropeanHub` is too fluyt-heavy or the shipyard's "build pencils" math is over-eager.
- Small-colonial provisioning: ports like Cumana, La Vela, and the Mosquito Coast outposts may sit far from any provisioning hub. Worth measuring per-port provisions inflow before tuning.
- The 40 mutinies are concentrated in a few Spanish flotas that get caught between bankrupt and unable-to-careen; the mutiny pipeline now works but the distribution may merit its own pass.

## Step 10.b — Non-combat attrition (commit `d0f4d46`)

Step 9 added a path for ships to die via mutiny → piracy, and Step 8 added boarding+sinking. But the historical record is dominated by *non-combat* losses — storms, foundering from teredo-rotted hulls, and fires aboard. Per Davis (1962) and Jarvis (1954), 17C peacetime English merchant shipping ran 1.5–2.5%/yr all-cause loss, with the cause mix roughly storms 50–65%, foundering 10–20%, fire 5–10%, piracy 5–10%. The bench's ~500-ship fleet should be losing 8–13 ships/yr to the elements. Before Step 10.b: zero non-combat losses.

### Module layout

All hazard logic lives in a new `crates/sim-core/src/weather/hazards.rs` (~360 LOC). `WeatherSystem` now holds a `HazardSystem` alongside the existing `WindGrid`. The hazard system owns:

- A deterministic `xorshift64` RNG state seeded at `WeatherSystem::load`, so the same bench seed produces the same attrition trace.
- `HazardCounters` for bench reporting (storms damaged vs sunk, foundered, fires sunk vs total).
- Two pure functions on `Ship`: `tick_environment` (teredo accumulation per hour, in tropical or open water) and `tick_age` (daily age bump).
- `roll_hazards(ship, pos, month) -> Vec<HazardEvent>` for the stochastic events.

The world tick calls `tick_environment` + `roll_hazards` per ship-hour (after `try_mutiny`, before the wages block) and applies any returned `HazardEvent` to the ship: hull damage, possible sink, magazine clear on fire. Sinking ships are reaped by the existing Step 8 Cleanup phase at end-of-tick — no new plumbing required. `tick_age` fires once per day from `tick_daily_hiring` which already gates on `day_of_year` transitions.

### Calibration

| Constant | Value | Source / rationale |
|---|---|---|
| `STORM_RATE_TROPICAL` | 2.5%/yr | Davis 1962 (Caribbean basin 3–6%/yr; ours is mid-low because foundering covers part of that range) |
| `STORM_RATE_OPEN` | 1.2%/yr | Davis 1962 (open Atlantic 1.5–3%/yr) |
| `HURRICANE_MONTH_MULTIPLIER` | 3.0 (Aug–Oct, tropical only) | NOAA HURDAT climatology: ~85% of Atlantic land-falling storms in this window |
| `STORM_CATASTROPHIC_FRACTION` | 40% | Storm "losses" historically were total losses; the other 60% are survivable damage events that bleed hull integrity for next time |
| `FIRE_RATE_SAILING` | 0.4%/yr | Davis et al.: fire = 5–10% of all losses, scales to ~0.1–0.2%/yr at sea |
| `FOUNDERING_RATE_AT_MAX_TEREDO` | 3%/yr at teredo=100 | Ramps from 0 at teredo=30 (research's "structurally suspect" threshold); multiplied by `max(1, age_days/3650)` |
| `TEREDO_RATE_TROPICAL_PER_HOUR` | 0.005 (≈ 43/yr) | Reaches the 80-point "structurally dangerous" mark in ~22 months, matching §1.3's 18–36 month figure |
| `TEREDO_RATE_OPEN_PER_HOUR` | 0.001 | Teredo navalis prefers warm salty water; northern routes are ~1/5 the tropical rate |
| `TROPICAL_Y_NM` | 450 (= 25°N in our origin frame) | Covers Caribbean + Gulf of Mexico, excludes Bermuda/Carolinas |

`Ship` gains two fields: `teredo_damage: f32` (0–100) and `age_days: u32`. All four constructors initialize them; `tick_careen` now reduces teredo at half the fouling rate (structural replanking is slower than scraping the bottom).

### Bench results (730 d, 503 ships)

```
Combat ledger:  63 pirate(s) afloat (24 seeded + 0 captured + 39 mutinied), 24 lost
Attrition:      16 storm sinkings (15 damage-only), 4 foundered, 2 fires (1 sunk)
```

- Non-combat losses: 21 over 2 years → **2.1%/yr** — squarely inside Davis's 1.5–2.5%/yr peacetime envelope.
- Cause mix: storms 81%, foundering 15%, fire 4%. Storms are a bit high vs research's 50–65%; foundering and fire are textbook.
- 15 damage-only storm events left ships limping but afloat — these compound, so over a 5-year run the same hulls would eventually sink to repeated storms even without a catastrophic roll.
- Bankruptcies fell from 81 → 67. Attrition is impartial: it disproportionately removes ships that were already in trouble (sat at sea undermanned, etc.), nudging the bankrupt count down indirectly.

### Determinism check

The same bench seed (`0xCAFE_1680` for fleet seeding, hardcoded constant for hazard RNG) produces an identical attrition trace across runs. Verified with two back-to-back invocations of `cargo run --release -p sim-core --example bench_trade -- 730`.

### Open follow-ups for later calibration

- Storm cause-share is 81% vs research's 50–65%. Could lower `STORM_RATE_TROPICAL` to 0.018 and raise foundering to compensate.
- `roll_hazards` doesn't yet know about combat-state: ships fleeing under all sail in heavy weather should have *higher* storm risk. Worth wiring in after Step 11.
- Fire rate doesn't yet scale with magazine load. The infrastructure is there (`ship.magazine_powder`); plug in when calibration calls for it.
- AI doesn't currently care about `teredo_damage` — careening is triggered by fouling alone. A captain who's never put in for a careen will quietly accumulate teredo until the foundering roll catches up. That's behaviorally accurate but means the AI is leaving free survival on the table.


---

## Step 11 — Prize handling rework + stochastic mutiny (`1ba214e`)

### Context

The 5-year bench after Step 10 surfaced a glaring problem: 256 pirates afloat at year 5 (60% of the active fleet), with 236 mutinies fueling almost all of the growth (captures: 0). Two structural defects were producing it.

1. **Every successful boarding flipped the prize to Pirate** (or burned it if the attacker couldn't spare crew). Historically pirates rarely kept prizes — they were after cargo and quick cash, not more hulls to maintain.
2. **The mutiny trigger was deterministic**: any Merchant at sea with debt > 1.5× MAX_SHIP_DEBT and morale < 0.25 flipped *on the first tick* conditions held. Historically (Rediker, Earle) mutinies were rare and took weeks of conspiring; the Golden Age saw maybe 1-2 significant mutinies per year basin-wide, not the 47/yr the bench was producing.

### Step 11.a — prize handling

Successful boardings now draw a weighted outcome from a deterministic per-world combat RNG (xorshift64, seeded at load):

| outcome | weight | effect |
|---|---|---|
| `take` | 5% | flip to Pirate (only if target hull > attacker × 1.2 AND attacker can spare a prize crew) |
| `sell` | 30% | cargo + hull bounty banked, prize sunk |
| `sink` | 50% | cargo stripped, hull dispatched |
| `release` | 15% | cargo stripped, target sails on with empty holds |

Cargo and 90% of silver are stripped on every non-`take` outcome. The world now tracks four counters (`prizes_taken/sold/sunk/released`) printed by the bench.

The size-gate is important: a 60-ton sloop pirate is not going to crew up a 200-ton fluyt as a prize even if the dice fall right. Without that, "take" became the universal outcome again for big targets.

### Step 11.b — stochastic mutiny

Two changes to `try_mutiny`:

- Threshold raised from 1.5× to **3× MAX_SHIP_DEBT** (5000 → 15000). With the chandler debt cap at MAX_SHIP_DEBT (=5000), the only way to reach the new threshold is also to carry ~10k pesos in unpaid wages — about a year of crew payroll. This filters out "annoyed" crews and keeps only "ruined and starving" crews.
- Per-hour roll: when all conditions hold, the crew flips with `p = 0.0002/hour`. A ship stuck in the mutiny zone for a full year has ~83% odds of flipping, but a typical 1-2 week distress window only fires 3-7% of the time. This models the "weeks of grumbling before the conspiracy fires" dynamic.

`try_mutiny` now takes a uniform sample as a parameter; the world calls it with `combat_rng_step(&mut self.combat_rng_state)` so the result is deterministic across runs but no longer a step function.

### Bench results (1825 d, ~500-ship Caribbean)

```
Combat ledger:  35 pirate(s) afloat (24 seeded + 0 captured + 13 mutinied), 60 lost
Prize outcomes: 0 taken, 1 sold, 1 sunk, 1 released
Attrition:      26 storm sinkings (39 damage-only), 33 foundered, 8 fires (5 sunk)
```

| metric | pre-Step-11 | post-Step-11 |
|---|---:|---:|
| pirates afloat @ 5yr | 256 | **35** |
| mutinies (5yr total) | 236 | **13** (2.6/yr) |
| captures | 0 | 0 |
| non-combat attrition rate | 2.2%/yr | 2.2%/yr (unchanged) |
| bankrupt ships | 55 | 86 |

The bankruptcy bump is the expected counterpart to the mutiny drop: ships that previously flipped to Pirate (and zeroed their books) now stay merchants and accumulate debt until they're written off. Same population of distressed hulls, different label.

### Observations

- Boardings remain rare in this bench (3 in 5 years). With the new outcome split that's *no* additional pirates from captures over the whole run — meaning the only growth path for the pirate fleet is mutiny, and we now have a sane mutiny rate. The fleet is stable around the seeded baseline.
- Captures will become more important once we add (i) navy patrols (Phase 4), which actually find pirates and force boardings, and (ii) intercept BTs that drive pirates toward known merchant lanes.
- The 5-year structural issues that aren't pirate-related — Tobacco chronic deficit (−564 t/yr), Cadiz Manufactures hoarding (9467% over LP equilibrium) — are still there. Those are LP/production calibration problems, not piracy problems.

### Test updates

- `pirate_boards_dismasted_merchant_and_takes_prize` → `..._and_resolves_prize`: now asserts the invariants that hold regardless of which weighted outcome the RNG picks (some prize counter incremented; cargo stripped; defender either gone, released-as-merchant, or flag-flipped).
- `under_crewed_pirate_burns_prize_instead_of_taking_it` → `..._cannot_take_prize`: under-crewed attackers can't satisfy the `can_spare_crew` gate, so the `take` outcome is unreachable for them; the test pins that invariant.
- Added `mutiny_skips_when_roll_above_probability`: with all deterministic conditions met but a roll just above the probability gate, no flip occurs.

158 tests pass.

## 2025-XX-XX — Phase 3 post-mortem cleanup (A1/A2/A3)

Three focused refactors against `planning/phase-3-postmortem.md`, batched into one cleanup pass before drafting `phase-4-plan.md`.

### A1 — Read-Compute-Write split in `tick_hourly_ai_and_physics` (`world.rs`)

**Problem (postmortem §1).** The hourly tick fused three concerns into one per-ship loop: (a) the ship's AI ran and pushed `ShipCommand`s, (b) those commands were drained and applied *for that ship only*, and (c) the ship's resource/morale/physics tick ran. Combat commands push tuples shaped `(me, cmd)` where `me` is the *issuing* ship — but the drain step's filter conflated "this loop's ship" with "this command's target", giving order-dependent first-strike privileges to whichever ship's index was iterated first.

**Fix.** Lifted the drain *out* of the per-ship loop. The hourly tick is now three phases iterating the same `ids` snapshot:

1. **AI Phase** — `ai.tick(...)` for each ship; pushes to `self.commands`. No cross-ship mutation.
2. **Resolution Phase** — drain `self.commands` once. Each tuple's first element is the attacker (no filtering needed). Steer / FireBroadside / AttemptBoard arms unchanged in mechanics, just relocated. Renamed local from `target` to `attacker` to match the tuple's semantics; fixed two stale `let attacker_id = id` references that should have been `let attacker_id = attacker`.
3. **Mutation/Physics Phase** — for each ship: tick_resources, tick_morale, try_mutiny, hazards, wages, swept-physics-with-land. Order matches the original loop.

**Determinism preserved** because the `ids` snapshot is collected once and reused across all three phases, and the per-ship RNG mixers (mutiny rolls share `self.combat_rng_state`; hazards use `self.weather.hazards`) read in the same order as before.

**Validation.** All tests pass (158 → still 158 here, A2's tests come next). `bench_trade 730` post-A1: bankrupt 99→90, lost pirates 18→11, attrition 13→10 storm sinkings. The improvement is consistent with eliminating the artificial first-strike — the previous "faster ship in the iteration always shoots first" effect was reliably winning trades for pirates that shouldn't have won them.

### A2 — `crew_seasoned: u16` on `Ship` (`ship.rs`)

**Problem (postmortem §2 / crewing-plan §7.3).** `crew_alive` tracked headcount only — there was no way to distinguish a freshly-hired greenhorn crew from one that had survived a season of boarding actions and storms. Combat doctrine (Phase 4 +) needs this as a multiplier on rate-of-fire and boarding effectiveness.

**Implementation.** New `u16` field with the invariant `crew_seasoned ≤ crew_alive`. Initialization:

- `Ship::new` and `seeded_at_port_typed` → fully seasoned (veteran crews seeded on world genesis).
- `freshly_built` (shipyard output) → 0 seasoned (hull-only).

New methods:

- `seasoned_ratio() -> f32` — for future combat modifiers.
- `apply_crew_losses(losses)` — integer pro-rata split between seasoned and unseasoned, rounds toward zero (slightly biases losses toward unseasoned; acceptable for v1 and avoids float-determinism concerns).
- `detach_prize_crew(amount) -> (alive, seasoned)` — pro-rata transfer when forming a prize crew.

Wired in:

- `tick_daily_hiring`: the hire loop already draws seasoned-first from the port pool; we now track the seasoned slice of `drawn` and credit it to `s.crew_seasoned` alongside `s.crew_alive`.
- Boarding casualty application: replaced raw `saturating_sub` with `apply_crew_losses`.
- "Take prize" branch: uses `detach_prize_crew` so the prize inherits its share of veterans.

Three new unit tests in `ship.rs`: ratio preservation under losses, saturation + invariant, prize-crew split. 158 → 161 tests pass.

### A3 — Multi-hop trade planning (`trade.rs`)

**Problem (postmortem §2 "Home Bias / Amsterdam Fluyt pathology").** `find_best_trade` greedily scored single legs `A → B` by `sell_price_B − buy_price_A − distance_cost`, with a `home_bias` bonus applied to the immediate destination. Result: Barbados sloops locked onto Barbados↔Martinique oscillations (high single-leg margin, no onward export at Martinique), and Amsterdam-bound fluyts saturated single corridors because every cash-laden ship saw "going home" as the best single leg.

**Fix.** Two-hop horizon (rolling — the AI still re-plans at every port, but it *scores* with a one-hop lookahead):

1. For each candidate first hop `A → B`, look up the best speculative onward leg `B → C` (excluding `C = A` to forbid immediate-bounce trades).
2. Score = `profit_AB + ONWARD_LOOKAHEAD_WEIGHT * min(profit_BC, profit_AB) + home_bonus`. Cap on `profit_BC` is critical (see calibration below).
3. `home_bonus` applies to the *circuit terminus* (either `dest_idx == home` OR `onward_terminus == home`) — not just the immediate first hop. This is what kills the "ship goes home for the home bonus" pathology: the bias now requires the ship to *end* the multi-leg circuit at home.
4. If there is no profitable onward leg from `B`, charge a `DEAD_END_PENALTY_PESOS_PER_TON` against the score.

API back-compat preserved — `TradePlan` still carries only the first hop. Reported `estimated_profit_per_ton` is the unbiased single-leg margin (score is internal; analytics want the raw number).

**Calibration story (worth recording).**

| Variant | bankrupt @ 730d | notes |
|---|---:|---|
| Pre-A3 (post-A1 baseline) | 90 | |
| `ONWARD_WEIGHT=0.5`, no cap | **162** | drained downstream ports forecast 300+ pesos/ton phantom sells; ships chased phantom circuits and over-committed |
| `ONWARD_WEIGHT=0.15`, cap onward ≤ first-leg profit | **86** | clamp prevents speculative future from out-voting the certain present |

Final constants: `ONWARD_LOOKAHEAD_WEIGHT = 0.15`, `DEAD_END_PENALTY_PESOS_PER_TON = 1.5`, `onward_used = min(onward_raw, first_leg_profit)`. The cap is the key insight: the lookahead is *real but speculative*; we don't trust it to override the immediate margin, only to break ties between equally-attractive immediate destinations.

New private helper `best_single_leg_excluding(origin, exclude, ...)` factored out so both `find_best_trade` (for the lookahead) and future callers (post-prize replan, scout/observer behaviors) can share the same single-leg scoring path. New test `prefers_working_circuit_over_dead_end` pins the dead-end-vs-circuit decision with explicit recipes (custom `ProductionRecipe` for full control, since archetype defaults had confounding cross-good stockpiles).

**Validation.** 162 tests pass (was 161; one new). Clippy clean. `bench_trade 730`: bankrupt 90 → 86 (∼5% drop, within noise but consistent). Prize activity up: from 0 taken / 1 sold / 1 sunk → 0 taken / 3 sold / 3 sunk (more ships now reach more ports). `bench_pathfind` unchanged (1406/1406 routes).

**Performance.** Multi-hop is O(N²M²) per `find_best_trade` call (60 ports × 11 goods → ~436k inner ops). Called only at port re-plan, not every tick — well within budget.

**Limitations recorded for Phase 4+.** The lookahead doesn't account for cargo-capacity differences between the first and second leg, doesn't model congestion (multiple ships converging on the same circuit), and treats all onward goods as fungible from the planner's perspective even though the ship will need to actually unload at B before reloading. None of these matter at the current scale; revisit if convoy behavior becomes a goal.

## Phase 4 — Combat Realism (in progress)

### §1 — Ordnance supply (`market.rs`, `ai.rs`)

**Premise from postmortem.** "Ordnance consumption is wired; nobody produces gunpowder or shot. Ships start with seeded cargo and can never refill once empty."

**What we found mid-implementation.** Step 7 had already done more than the plan credited: production was wired at three of four planned European hubs (London / Amsterdam / Cadiz), and the AI top-up at port was wired in `ai::act_sell_all → replenish_ordnance`. The actual gaps in §1 were small: Nantes had no powder recipe, and the magazine targets were flat (`(4,4)` pirate, `(1,1)` merchant) regardless of cannon count.

**Changes shipped.**

- `market.rs`: `PortArchetype::EuropeanNantes` recipe now produces 4 t/mo gunpowder (French royal foundries at Essonnes, est. 1664 by Colbert). Powder-only — French shot output largely went into army artillery, not naval export. Two regression-guard tests: every European hub produces powder; the three major arsenals also produce shot.

- `ai.rs`: replaced the flat magazine constants with `ordnance_target(ship, stats)` keyed on cannon count via `combat::broadside_supply_cost`. Targets expressed directly in "broadsides of reserve":
  - `MERCHANT_BROADSIDES_TARGET = 20`
  - `PIRATE_BROADSIDES_TARGET = 40`

  With `POWDER_TONS_PER_GUN = 0.01`, a 24-gun pirate ship now carries ~9.6 t powder + shot (was 4 t each), an 8-gun sloop merchant ~1.6 t each (was 1 t). Historically defensible: indiamen carried 5–10 broadsides of reserve, privateers 30+.

**Calibration.** `bench_trade 730 d`: bankrupt 86 → 80 (slight improvement), gunpowder annualized flow +252 t/yr, cannon shot +324 t/yr at producer hubs. Top-up loop reaches Caribbean ports without strangling cargo capacity (magazine is bounded by cannons, not by hold size).

**Decision recorded.** Considered the plan's `cap × scale × 0.5` formula (cargo-size-scaled) but chose cannons-scaled instead because consumption itself is cannon-keyed — the target should track the demand source directly. A fluyt with 12 guns now matches a frigate-converted-merchant of 12 guns regardless of cargo capacity, which is the right invariant.

### §2 — Repair at port (`ship.rs`, `ai.rs`)

**Course correction during planning.** The original plan framed both hull and rigging as "monotonically decaying state". Reviewing the code: hull degrades from storms + fires + combat (all in place); rigging degrades from combat *only*. That changed the model.

- **Rigging is a combat reserve, not a wear part.** Sails, cordage, spars come from bo's'n stores carried aboard. Historical port turnaround included re-rigging as a normal item — a yard could put a new spar up in a day. So rigging gets a one-shot full top-off on undock (like provisions), billed at 1.5 pesos/HP.
- **Hull is a wear part.** Storm damage + combat damage accumulate; only the carpenters can fix it. Ticks while docked at 0.3 HP/hr, billed at 6 pesos/HP, sized so a 100-HP rebuild ≈ 14 days at port for ~600 pesos (about 30% of a sloop's build cost, matching RN "great repair" line-items).

**New API on `Ship`.**

- `tick_repair_hull(stats) -> bool` — one hour of carpentry. Always applies the full HP delta (carpenters work regardless of payment); silver covers the bill if available, otherwise the shortfall accrues to `Ship::debt` (composes with existing wage / chandler debt: bankruptcy threshold, shipyard scrap, mutiny pressure already cover it).
- `top_off_rigging(stats) -> f32` — full one-shot restore, returns HP delta. Same silver/debt path. Returns 0.0 (no-op) when rigging already at max.

**Wiring.**

- `ai.rs::act_careen` now ticks both `tick_careen` (fouling + teredo) *and* `tick_repair_hull` in parallel; only returns Success when both complete. A battle-damaged ship stays at port until it's actually seaworthy — historically correct.
- `ai.rs::act_undock` calls `top_off_rigging` immediately before clearing the dock state.

**Calibration.** `bench_trade 730 d`: bankrupt 80 → 83 (+3). Marginal — current pre-§3 combat cadence is rare enough that most ships pay only a few pesos for the storm hits they took. The real test is after §3 when combat damage scales up. Avg fleet hull integrity target ≥ 60% can't yet be meaningfully checked.

**Future work captured (FW-8).** Repair currently silver-only. Eventually we want hull repair to consume `NAVAL_STORES` + `MANUFACTURES` from the port market and rigging top-off to consume `NAVAL_STORES`. Decoupled from §2 deliberately — kept the v1 path independent of market stock so we can validate the rate / cost / debt machinery first.

### §3 — Sub-tick combat (next up)

Pending; planning notes in `planning/phase-4-plan.md §3`.

### §3c-1 — Symmetric engagement (CA-style per-hour tactical judgment)

**Problem.** The initial §3c-1 implementation used a role-based design: the ship that opened fire became `Attacker`, the other `Defender`, and the engaged subtree branched on role. Two issues emerged: (1) the rigid role lock did not match the historical pattern where each captain re-evaluates fight/flee each hour from his own view, and (2) calibration regressed — `bench_trade 730` jumped from 85 baseline to 118 bankrupt, primarily because merchants tagged "Defender" were stuck in `FleeAndFire` even after the pirate had exhausted ordnance and was no longer a threat.

**Alternatives considered.**
- *Keep roles but add role-swap heuristics.* Rejected — adds complexity to preserve a label that does not earn its keep; the resulting logic would just be `should_fight`/`should_flee` reading role state.
- *Drop the engaged branch entirely and lean on `see_threat`/`see_prey`.* Rejected — that's what the old code effectively did (the engaged branches were dead code in practice), and it gave no way to express "I commit to staying in this fight even though you're momentarily out of range" or to gate disengage on a cooldown.

**Decision.** Symmetric engagement, no role enum. `engage(a, b)` is a mutual flip gated by a `disengaged_until_minute` cooldown. The engaged subtree is itself a Selector with priority-ordered conditions evaluated every hour from each ship's own snapshot: `should_disengage` → `should_fight` → `should_flee` → `hold`. A new `ShipCommand::Disengage { other }` clears both ships' `engaged_with` and stamps a 60-minute cooldown on both, preventing thrashing.

`ShipSnapshot` was extended with `hull_frac` and `cannons` so the heuristics can compare strengths from world view alone.

**Results.** Colosseum: scenarios 1/2/4/5 now end with `DEFENDER ESCAPED` at h65–h81 (merchants break off once pirates exhaust ordnance and pick `should_disengage`); scenario 3 still ends `TARGET SUNK` h12 (24-gun vs bark — expected). `bench_trade 730` = 100 bankrupt — above 85 baseline but well below the 118 role-based regression, within tolerance for §3c-1 (further calibration in §3.6).

**BT framework pitfall (caught mid-validation).** While validating, the engaged branches appeared to never re-evaluate — `COND_IS_ENGAGED` was checked once at hour 0 and then never again, even though `engage()` was firing correctly. Root cause: the Selector in `bt.rs` has memory via `state.running_child[depth]`. When a child returns `Status::Running`, that child index is *cached* and re-entered next tick, **skipping higher-priority siblings entirely** until the cached child returns Success/Failure. `act_pursue` and `act_flee` were returning `Running`, so the BT never climbed back up to re-check `IS_ENGAGED`.

**Fix.** `act_pursue` and `act_flee` now return `Status::Success` after pushing their intents. This forces a full top-down re-evaluation each tick — true CA-style judgment. Dock-tree actions that genuinely need multi-tick state (`act_resupply`, `act_careen`) keep `Running` deliberately. *This is the silent assumption to remember: in this codebase, any BT leaf that should be reconsidered every tick must return Success, not Running. Returning Running is a commitment to multi-tick state, not a polite "still working" signal.* The old role-based engaged branches were dead code masked by exactly this bug — `see_threat`/`see_prey` in the default subtree happened to produce roughly the right behavior, so no one noticed.

**Future work.** §3c-2 (Strike + prize), §3c-3 (boarding integration), §3d (forts), §3e (calibration sweep — likely needs to revisit `should_disengage` thresholds and the "outnumbered" rule once Phase 5's relations matrix replaces the ShipPolicy hostile-proxy).

### §3c-2 (minimal) — Strike colors + shared prize resolver

**Problem.** §3c-1 gave ships `should_disengage`, but a ship that is hopelessly beaten — torn rigging, half its crew dead, morale collapsed, counterpart faster — should *surrender* rather than try to break off into a hail of fire. The existing boarding-victory path already had a take/sell/sink/release prize roll, but it was bolted to `resolve_boarding`; surrender-without-boarding had no path through the world resolver.

**Alternatives considered.**
- *Full §3c-2 per the original plan (Follow BT branch + prize sails with victor to port + sells on arrival).* Rejected for this commit — meaningful scope creep (touches movement/physics, station-keeping, port-trigger sells), and we want to see Strike alone behave in bench_trade before investing in the follow voyage.
- *Reuse `Disengage` and have the world infer surrender from low morale.* Rejected — violates the principle that the AI decides intent and the world resolves it. Surrender is a distinct intent and deserves its own command.

**Decision.**
- New `ShipCommand::Strike { to: ShipId }`. Issuer = prize; `to` = victor.
- New `COND_SHOULD_STRIKE` + `ACT_STRIKE` in `ai.rs`. Priority *above* `should_disengage` in the engaged-subtree Selector — a hopelessly beaten ship surrenders cleanly rather than trying to break contact.
- `should_strike` fires when *both*: (a) `morale × hull_fraction < STRIKE_THRESHOLD` (0.15), and (b) cannot outrun the counterpart (own effective speed ≤ counterpart's, *or* own rigging is below the boarding threshold).
- New `STRIKE_THRESHOLD = 0.15` constant in `combat.rs`.
- Extracted the boarding-victory prize block (~140 lines) into `World::resolve_prize_action(victor, prize)`. The boarding path now just calls it; the new Strike resolution in the command-drain loop also calls it (deferred to after the drain to avoid double-borrowing `self`).
- The instant-despawn model for take/sell/sink/release is preserved — *no* follow voyage in this commit. Take still flips the prize to `Pirate` policy in-place; sell/sink set hull=0 → Sunk; release leaves the target afloat with cargo stripped.

**Results.**
- 173 tests pass, fmt + clippy clean.
- Colosseum: same 5 scenarios produce the same outcomes (4× DEFENDER ESCAPED, 1× TARGET SUNK) — none of them stress the strike condition cleanly. The `PRIZE SURRENDERED` verdict is wired and will fire when a scenario does trigger it.
- `bench_trade 730d`: **89 bankrupt** (down from 100 after §3c-1; baseline 85, tolerance ≤95). Prize ledger shows 74 prize events total (5 taken, 26 sold, 33 sunk, 10 released) — Strike is doing real work in the broader sim, letting beaten merchants surrender (preserving capital that would otherwise be lost to sinking).

**Future work.**
- §3c-2b: Follow BT branch + `follow_target` field + prize physically sails with victor and sells when victor reaches friendly port. Replaces the instant `take`/`sell` despawn with a real voyage.
- Calibration: 89 bankrupt is in tolerance but worth revisiting once §3d (forts) and §3e (calibration sweep) land — Strike may be too generous, and `STRIKE_THRESHOLD` 0.15 is unvalidated.
- Open question: should ships of certain factions (Navy, Privateer with Letter of Marque) be forbidden from striking? In Phase 5 (Relations Matrix) the surrender decision will gain a "honor of the flag" gate. For now any policy can strike.

### §3c-3 — Boarding as a first-class engaged-subtree choice

**Problem.** §3c-1's engaged subtree had a silent hole: a pirate engaged with a rigging-crippled merchant but out of powder would route through `should_fight` (false, requires ordnance) → `should_flee` (true, the fall-through) and *flee from helpless prey it could trivially board*. `act_pursue` already called `maybe_board` for pirates, so when `should_fight` did fire (ammo + prey crippled) boarding worked — but with an empty magazine the BT never reached `act_pursue` at all. The `should_disengage` rule had a `can_board` guard that prevented disengage in this case, but nothing then routed the ship into the board.

**Decision.** New `COND_SHOULD_BOARD` + `ACT_BOARD` in the engaged subtree. Priority is *above* `should_fight` (so a pirate facing crippled prey commits to the grapple even with ammo — fire-and-board was the historical combined-arms approach, and `act_board` still calls `maybe_fire_at` to soften the deck if magazine permits) and *above* `should_disengage` (so the disengage rule cannot itself preempt a viable board).

`should_board` gate: Pirate policy + engaged + counterpart visible in snapshots + counterpart rigging < `BOARDING_RIGGING_THRESHOLD` + own crew ≥ 2. Sets `goal.pursue_target = engaged_with` so `act_board` (which reuses the `act_pursue` steering machinery) steers at the right ship.

Final engaged-subtree priority: **strike → board → disengage → fight → flee → hold**.

**Results.**
- 174 tests pass (1 new regression test added: `engaged_pirate_with_no_powder_boards_crippled_prey`). The test sets a Pirate with empty Cargo + `engaged_with = merchant_id`, places a crippled-rigging merchant 0.1 NM away, and asserts that the BT emits `AttemptBoard` (and does *not* emit a southbound flee Steer).
- bench_trade 730d: **89 bankrupt** (unchanged from §3c-2). Prize ledger also unchanged at 74 events. The no-ammo-vs-crippled case is rare in this seed, so §3c-3 is correctness-only here — it doesn't shift the calibration metric. Worth re-checking after §3d (forts) and §3e (calibration sweep) bring more combat events into the run.
- Colosseum: same 5 outcomes (none of the scripted scenarios end with an out-of-ammo pirate facing crippled prey).

**Open question.** `act_board` currently still calls `maybe_fire_at` even when it has ordnance, which means a pirate with a healthy magazine and crippled prey will fire a softening broadside *and* attempt to board on the same tick. That's historically accurate (fire-and-board) but it does mean the boarding gate can fail (rigging may regenerate? no — actually it doesn't, but range may open if the target sails away after the broadside). In v1 we accept this — `maybe_board` re-gates `BOARDING_RANGE_NM` on the same tick using the *commanded* attacker velocity, so if the steer-toward-target keeps us in range, the board still fires.

**Future work.** §3d (forts), §3e (calibration sweep), §3c-2b (Follow voyage), Phase 5 (Relations Matrix + naval boarders).

---

### Phase 4 §3c-2b — Prize tow via destination mirroring

**Problem.** `resolve_prize_action`'s `sell` outcome was an instant-resolve: the prize despawned in place and the victor pocketed `cargo_silver + hull_bounty` immediately. That's a tolerable abstraction for sinkings but completely skips the period-canonical "prize sails to friendly port under a skeleton crew, sells, captor gets a share" arc — the one moment where pirate economics is most legible.

**Design pivot from earlier draft.** The original spec called for a `follow_target` field plus a Follow BT branch with station-keeping math. Discarded as overkill: the prize doesn't need to formation-sail with her captor, she just needs to end up at the same port. Settled on the much simpler rule: **prize copies owner's destination each tick, runs her own AI, sells on dock.**

**Implementation.**
- `Ship.prize_owner: Option<ShipId>` added near the engagement fields. `None` for normal ships and for `take` / `sink` / `release` outcomes.
- `combat::PRIZE_TOW_CREW_SPLIT = 0.20` (smaller than the 0.50 used for `take` — sailing only, no fighting).
- `World.prizes_in_tow` / `prizes_orphaned` counters added.
- `resolve_prize_action` sell branch split into `sell_tow` and `sell_instant`. Tow is chosen when the victor can spare `tow_crew` while staying ≥ `crew_min`. If not, the existing instant-sell behavior runs unchanged.
- Two new passes in `tick_hourly_ai_and_physics`:
  - **Pre-AI copy-owner-nav pass.** After the `ids` snapshot, build a `SecondaryMap<ShipId, (destination, dest_port)>` from every ship's current goal. Then for each ship with `prize_owner = Some(v)`: if `v` is gone, clear `prize_owner` + bump `prizes_orphaned`; else copy `v`'s goal into the prize's. Waypoints are cleared on change so `act_sail` re-routes.
  - **Post-physics pay-at-port pass.** Scan for `state == Docked && prize_owner.is_some()`. Pay the victor `cargo_silver + hull_bounty` (same formulae as instant-sell), bump morale, mark the prize Sunk so the Cleanup phase reaps her, increment `prizes_sold`.

**Orphan rule.** Settled on a universal "no rescue from beyond the grave": if the victor sinks, the prize's `prize_owner` is cleared and she continues with her last-known destination under her own (now-pirate, since the policy/faction were already flipped at capture) colors. No silver is paid. This is historically defensible (a prize crew with no captor would absolutely sell their own loot at the nearest friendly port — but we don't yet model that, so the loot just rides into oblivion when she eventually sinks or is engaged). Web-searched several period sources; the no-rescue rule was the cleanest invariant.

**Scope limit.** `take` outcome stays instant flip-to-pirate in place. The user's "share a destination" intent technically applies to all prize voyages, but `take` is much rarer (0.05 weight, gated on hull upgrade) and the existing flip-in-place gives the new pirate captain a free re-plan, which is fine. Revisit in Phase 5 if it becomes a bottleneck.

**Calibration shift.** 730d bench_trade 89 → **105 bankrupt** (~18% regression, well below the 118 ceiling that triggered the symmetric-engagement rewrite). The capital dip is real: a sell that used to pay out in tick T now pays in tick T + (voyage length), so victors run their wages owed up before the bonus lands. At end of run: 9 prizes still in tow, 1 orphaned, 2 instant-sold (crew-spare failure), 20 sunk, 8 released. Acceptable trade for the much richer narrative payoff.

**Combat test fixture updated.** `pirate_boards_dismasted_merchant_and_resolves_prize` now also counts `prizes_in_tow` toward total outcomes and branches on `prize_owner` to recognize the tow case (cargo intact, policy flipped, prize_owner set) as distinct from `take` (cargo intact, policy flipped, no prize_owner) and `release` (cargo stripped, original policy).

**Open question / future work.** The orphaning rule could be softened (prize crew "switches sides" to whichever faction's port is closest, sells under their own name) but that needs a port-affinity model that doesn't exist yet. Defer.

---

### Post-Phase-3 cleanup A2 — `crew_seasoned` invariant + tests

**Status going in.** The `crew_seasoned: u16` field was already on `Ship` from earlier work, alongside pro-rata `apply_crew_losses` / `detach_prize_crew` helpers and a `seasoned_ratio()` accessor. The hiring loop in `tick_daily_hiring` was already drawing seasoned-first from the port pool and incrementing `ship.crew_seasoned` in lockstep with `ship.crew_alive`. Combat call sites (`world.rs:1274, 1278`) use the pro-rata helpers. So the postmortem A2 work was almost entirely already-done — but had **no tests**, which meant any regression to the invariant `crew_seasoned <= crew_alive` would have gone silent.

**One deliberate departure from the postmortem spec.** Postmortem §3.2 said "Initialize to 0 on `Ship::new`; shipyard build initializes 0 (hull only)." The current code does the *opposite* for `Ship::new` and `Ship::seeded_at_port_typed`: they seed `crew_seasoned = crew_typical()` (100% seasoned). The in-source comments document the reasoning: "Seed-fleet ships are fully crewed (no Hiring loop) and are assumed to be veteran crews — bench fleets represent established merchant captains." Setting these to 0 would mean every ship at simulation start is staffed by complete greenhorns, which would dominate the bench_trade economy with mutinies and crew-induced sail-handling losses the moment crew modifiers land in Phase 5. Calibration-wise the current default is the right one; only `Ship::freshly_built` (shipyard hull launch) initializes to 0, which matches the postmortem intent for *new* hulls. Leaving as-is.

**What A2 added.**
- 5 unit tests in `ship.rs` covering: pro-rata casualties (`apply_crew_losses_pro_rata_preserves_invariant`), saturation at full crew loss (`apply_crew_losses_saturates_at_alive_and_zeroes_seasoned`), prize-crew detach pro-rata (`detach_prize_crew_returns_pro_rata_split`), `seasoned_ratio` on empty crew, and `seasoned_ratio` as a proper fraction.
- 1 integration test in `combat.rs` (`hiring_draws_seasoned_first_and_tracks_count`): hand-sets a port pool to `(3 seasoned, 100 unseasoned)`, plants a `state == Hiring` ship at that port with empty crew, advances 48 hourly ticks to cross a day boundary, and asserts that the ship's `crew_seasoned` ends up exactly equal to `min(crew_alive, 3)` — i.e., all seasoned hands were drawn before any unseasoned. Total tests: 174 → **180**.

**Test fixture gotcha worth recording.** `World::tick` advances 1 *hour*, not 1 day. The daily hiring path is gated on a day-of-year change (`if today == self.last_hire_day { return; }`), and `last_hire_day` is initialized to the world's load-time day. So the first 24-hour window often doesn't fire any hiring at all, because the day-of-year check is evaluated *before* `advance_hours(1)`. The test loops 48 ticks to guarantee a crossing. Other future hiring tests should follow the same pattern (or expose `tick_daily_hiring` as `pub(crate)` for direct invocation).

**No calibration impact.** This is a test-only change plus already-shipped helpers. bench_trade not re-run.

**Next.** A1 — clean three-phase split of `tick_hourly_ai_and_physics`.

---

## 2025-XX-XX — Phase 5 prep: diplomacy.md gaps filled

**Context.** Before drafting the Phase 5 plan (Relations Matrix + Letters of Marque + war cycle + port BTs), wanted to close the 9 research gaps flagged in `planning/research/diplomacy.md` lines 682–701. Most were Wikipedia rate-limit failures from the original research pass; a few were areas where secondary literature was vague on specific simulation-relevant numbers.

**What was done.** Background research agent (`diplomacy-gaps`) executed a focused pass and produced `planning/research/diplomacy-gaps.md` (~59KB). Of the 9 gaps:

- **Fully filled:** GAP 4 (Western Design fleet composition).
- **Substantially filled:** GAPs 1, 3, 6, 7 — including a detailed Modyford commission timeline (1664 anti-privateer proclamation → reversal within weeks; the 1668/1669/1670 commissions to Morgan; the Treaty of Madrid lag-failure timeline with specific weeks; the Morgan/Byndloss/Tortuga kickback arrangement post-1675).
- **Partially filled:** GAPs 2, 5, 8, 9 — best-available web sources exhausted; remaining detail would require book-level sources (Pares 1936, Haring 1910, Pestana 2017, Rediker 2004, Lane 1998, Calendars of State Papers Colonial).

**Simulation-actionable numbers extracted** (see `diplomacy-gaps.md` "CROSS-CUTTING SIMULATION PARAMETERS"):

- **Communication lag:** war declaration Europe→Jamaica central 8 wk, range 4–16 wk; treaty news similar.
- **Bond:** standard English privateer bond £1,500.
- **Prize split (Porto Bello, 1668):** ~10% Crown Admiralty Tenth + ~10% governor commission fee + ~5% admiral + ~75% owners/crew. (Residual: whether Modyford's 10% was *the* Admiralty Tenth diverted locally vs. an additional fee is still ambiguous — flag for design decision.)
- **Bribery rates:** small ops £20–100, major (ship + cargo) several thousand £, Kidd's failed cache £14,000.
- **Force sizes:** Western Design 38 ships / 8,000 troops; Morgan/Panama 30+ ships / ~2,000; Cartagena 1697 10+ naval / 1,850; typical buccaneer fleet 3–15 ships.

**Design implications already visible (to be folded into Phase 5 plan):**

1. **Communication lag is THE mechanic** that makes "No Peace Beyond the Line" emerge naturally — if treaty effects arrive in the Caribbean with a 4–16 week stochastic delay, the gap between metropolitan policy and local reality writes itself.
2. **Governor commission fees are separate from Crown Admiralty Tenth** — suggests Port struct needs both a faction-level "Crown share %" and a port-level "governor's cut %", configurable per-governor (Modyford = 10%, others lower).
3. **Modyford reversal pattern** (issuing anti-privateer order, then reversing under economic pressure) is the canonical example of *Port BT vs. Faction BT* conflict — supports modeling governor as a semi-autonomous agent with their own utility function (local revenue, personal wealth), not a deterministic relay of metropolitan policy.
4. **Byndloss/Tortuga kickback** justifies modeling commissions as a *tradeable* mechanic: when a governor's own faction is at peace, captains can still buy commissions from foreign governors at currently-warring factions, with intermediary fees.

**Next.** Draft `planning/phase-5-plan.md` — likely split into 5a (Static Relations + LoM substrate, unblocks Forts) and 5b (Dynamic diplomacy, Port BT, communication lag).

---

## 2026-05-24 — DOD/Perf Refactor #1: Fixed-point currency (`Pesos`)

**Context.** External code review (May 2026) flagged that the simulation
stored every silver/debt/wage balance as `f32`, with `cost > silver + 1e-4`
style guards papering over accumulated drift. Over 730 in-game days × thousands
of transactions, this can leak phantom pesos or reject valid trades.

**Decision.** Introduced `sim_core::money::Pesos`, an `i64`-backed fixed-point
type with centavo precision (1/100 peso). All *stored* balances now live in
`Pesos`; prices remain `f32` (they're computed fresh per call and never
accumulate). Conversion is one-way per transaction: float bill computed from
float price, rounded to centavos via `Pesos::from_pesos_f32` or
`Pesos::scale(f32)`, then exact integer arithmetic forever after.

**Alternatives considered:**
- *Whole pesos (i64)*: rejected — would round wage accrual (per-hour) and per-ton
  resupply costs.
- *Millipesos*: rejected — overkill for 17C economy.
- *Status quo with tighter epsilons*: rejected — the drift is fundamental, not
  a tolerance issue.

**Scope.** Converted: `Ship.{silver, starting_silver, lifetime_dividends, debt,
wages_owed_pesos}`, `PortMarket.silver`, `ShipType.build_silver`,
`BuildCost.silver`, `World.{silver_at_month_start, last_month_avg_profit}`,
and all currency constants (`STARTING_SILVER_PESOS`, `MAX_SHIP_DEBT`,
`MUTINY_DEBT_THRESHOLD`, `WAGE_PESOS_PER_MAN_MONTH`, `SIGN_ON_BOUNTY_PESOS`,
`HULL/RIGGING_REPAIR_COST_PESOS_PER_HP`, `INITIAL_PORT_SILVER_PESOS`,
`STARTING_SILVER_FLOOR`). `PortMarket.debt: Cargo` (commodity tons owed to
hinterland) intentionally left as-is. All `+ 1e-4` currency epsilons deleted.

**Validation.** 187 tests pass; clippy clean; `bench_trade` numbers
near-identical to baseline (79 → 80 bankrupt ships, well within rounding
noise — confirms economics unchanged). Pre-existing equilibrium calibration
gap (Tobacco/Manufactures price deltas of 1000s of %) is unrelated and
predates this work.

**Notes for next phases.**
- Pesos's `Serialize`/`Deserialize` is transparent on `i64` centavos. Any
  future save-game format will read/write integer cents.
- The `BuildCost.total() -> f32` mixes pesos and tons; kept as f32 since
  it's only used as a relative ROI score, with `silver` line converted via
  `as_pesos_f32()` at the boundary.
- Remaining DOD/perf items from the review (Cargo flat array, spatial hash
  flat-vec, PRNG → `rand_pcg`, NavTrack → ArrayVec) are queued in the
  session plan and will land as separate commits.

---

## 2026-05-24 — DOD/Perf Refactor #2: Cargo flat array

**Decision.** Replaced `Cargo`'s internal `Vec<(GoodId, f32)>` with a flat
`[f32; CARGO_SLOTS]` where `CARGO_SLOTS = 16` (room for the 11 starter goods
plus headroom). `Cargo` is now exactly 64 bytes — one cache line on
common x86_64 / aarch64 — and held inline on every ship and port.

**Wins.** `get`/`add` become O(1) indexed loads with no branch and no
heap allocation. Every `Ship` is fully inline (no pointer chase from
Ship into a heap-allocated cargo Vec). `iter()` is now deterministic by
`GoodId` ascending (previously by insertion order), which is a strict
improvement for reproducibility.

**Contract changes.** None visible to callers — the public API
(`new`, `get`, `add`, `remove`, `iter`, `is_empty`, `len`, `clear`,
`total_tons`) is unchanged. `iter` now yields slots with positive stock
only, in `GoodId` order; one test in `cargo.rs` was updated to assert
the new ordering (and a `size_of::<Cargo>() == 64` guard added so this
property doesn't regress).

**Out-of-range goods.** `GoodId.0 >= CARGO_SLOTS` panics in debug,
silently no-ops in release. The current registry is 11 wide so this is
unreachable in practice; if `goods.ron` ever grows past 16, bumping
`CARGO_SLOTS` is a single-constant change (and the cache-line guard
test will fail, which is the prompt to revisit the size choice).

**Validation.** 188 tests pass; clippy clean; `bench_trade` 82 vs 80
bankruptcies (within centavo-rounding noise from Phase 1 + new iteration
order influencing per-port settlement order).

---

## DOD Refactor — Phase 3: SpatialHash flat-vec (2026-05)

**Context.** External review flagged that `SpatialHash` used
`BTreeMap<(i32,i32), Vec<(ShipId, Position)>>`. BTrees are cache-unfriendly
(node-jumping on lookup) and the per-cell `Vec` adds another heap
indirection on every query. With ~500 ships querying every tick, this is
a hot path.

**Alternatives considered.**
1. *Lazy sort inside `neighbors(&mut self, …)`.* Simple but pushes a `&mut`
   requirement through every caller. AI ticks hold concurrent borrows on
   `World`; demanding `&mut spatial` broke compilation in `ai.rs`.
2. *Keep BTreeMap, swap inner `Vec` for `SmallVec`.* Half-measure that
   leaves the BTree pointer-chasing in place.
3. *Flat `Vec<Entry>` with explicit `finalize()`.* Chosen. One sort per
   tick, then all queries are `&self` binary searches over a contiguous
   buffer. Deterministic via a total order on `(cell, ship_id)`.

**Resolution.** `SpatialHash` is now `Vec<Entry { cell, id, pos }>`.
Build phase calls `clear()` → many `insert()` → exactly one `finalize()`.
Queries use `partition_point` to locate each of the 9 neighbour cells
in O(log n) and then scan the contiguous slice. A `debug_assert` in
`neighbors()` catches missing `finalize()` calls in dev.

The single integration point in `world.rs` adds one `self.spatial.finalize();`
call after the per-tick build loop. Test helpers in `ai_behavior.rs` were
updated to call `finalize()` after their inserts.

**Validation.** 189 tests pass; clippy clean; `bench_trade` 82
bankruptcies (identical to Phase 2 baseline); `bench_pathfind` 1406/1406
routes ok, 1.36 ms avg.

---

## DOD Refactor — Phase 4: PRNG → `rand_pcg` (2026-05)

**Context.** Four call sites (`ai.rs`, `nav.rs`, `weather/hazards.rs`,
`world.rs`) hand-rolled the same xorshift64 + 53-bit-mantissa-mixer
pattern, each as a private helper. Hand-rolled xorshift has documented
weak low bits; the multiplicative mixer was added as folk wisdom rather
than from a stated mathematical guarantee. Five separate copies also
made it impossible to swap algorithms without touching every site.

**Alternatives considered.**
1. *`rand` (full crate).* Brings in `OsRng`, distributions, and the
   getrandom platform shim — overkill for a deterministic sim that only
   needs uniform f32 and Gaussians.
2. *`fastrand`.* Tiny, but the algorithm is wyrand (a multiplier hash);
   designed for speed in tooling, not statistically scrutinized at the
   level PCG is.
3. *`rand_pcg::Pcg64Mcg` behind a thin newtype.* Chosen. Stable,
   documented determinism contract, ~5 ns per draw, and the only
   surface area we need (`next_u64` + uniform_f32 + Box-Muller).

**Resolution.** New `sim_rng::SimRng` newtype wraps `Pcg64Mcg`. Three
methods: `uniform_f32`, `gaussian` (Box-Muller), `next_u64`. Seeded via
`SeedableRng::seed_from_u64` so existing u64 seeds keep their public
meaning. All four sites converted: `ShipAI` now stores `rng` + `nav_rng`
as `SimRng`; `HazardSystem` stores `rng: SimRng`; `World.combat_rng:
SimRng` replaces the free `combat_rng_step(&mut u64)` helper (the
borrow-checker dance that motivated it is unaffected — `&mut self.combat_rng`
works inside drain loops the same way `&mut self.combat_rng_state` did).

Tests that depend on specific RNG outcomes (combat rolls at given seeds,
e.g. `pirate_in_cannon_range_damages_merchant`, prize-disposition tests)
were re-validated under the new generator; **all 192 tests pass** with
no golden-value adjustments needed — the affected tests check
qualitative invariants (cargo missing, hull damaged) rather than exact
roll values.

**Validation.** 192 tests pass; clippy clean; `bench_trade` 79
bankruptcies (vs 82 pre-Phase-4 — within noise; the equilibrium gap is
unchanged); `bench_pathfind` 1406/1406 routes ok, 1.42 ms avg.

---

## DOD Refactor — Phase 5: NavTrack waypoints as ArrayVec (2026-05)

**Context.** `NavTrack.waypoints` was a `VecDeque<Position>`. Every
`Ship` therefore held a heap allocation that resized and got dropped
constantly; for 500 ships doing periodic re-planning, the allocator
traffic alone is meaningful, and the pointer chase pulls cache lines
into `Ship` iteration loops (combat, weather, AI tick) that have no
business touching the waypoint buffer.

**Sizing the cap.** `bench_pathfind` exhaustively walks all 1406 ordered
port pairs in the historical 11-port set. Measured max path length is
37 waypoints. A 64-slot `ArrayVec<Position, 64>` is 64 × 8 bytes = 512
bytes (Position is two f32), with no extra indirection. Future routes
(e.g. trans-Atlantic dogleg around the Cape of Good Hope) get
generous headroom; a `debug_assert!` in `set_path` will fire if the
planner ever exceeds the cap in dev so we notice when it's time to raise.

**Alternatives considered.**
1. *`tinyvec` instead of `arrayvec`.* Spills to heap when capped —
   defeats the determinism-of-layout argument and re-introduces the
   allocator traffic we're trying to remove.
2. *`SmallVec<[Position; 32]>`.* Same spillover issue plus extra
   discriminant per slot.
3. *Pull `NavTrack` into a parallel ECS-style component.* The reviewer's
   alternative suggestion. Defers to the "skip ECS" decision; we can
   revisit if `Ship` itself ever grows uncomfortably wide.

**Resolution.** `waypoints: ArrayVec<Position, 64>` (constant exported
as `MAX_WAYPOINTS`). API stays nearly identical — `front()` becomes
`first()`, `pop_front()` becomes `remove(0)`. The O(n) front-pop is
trivial at n ≤ 64 and we typically remove at most a handful per tick.

**Validation.** 192 tests pass; clippy clean; `bench_trade` 79
bankruptcies (unchanged from Phase 4); `bench_pathfind` 1406/1406 ok,
1.37 ms avg. No path ever exceeded 64 waypoints in either benchmark.
