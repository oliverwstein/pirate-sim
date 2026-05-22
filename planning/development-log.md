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
