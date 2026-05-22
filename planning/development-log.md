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

