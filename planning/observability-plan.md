# Observability Plan: Event Journals → SQLite

**Status:** Deferred. Captured here for future implementation. Interim approach is per-entity telemetry fields on factions/ports/ships (see end of doc).

## Motivation

Bench programs currently learn about the run by ad-hoc walks over `world.ships` and `world.ports`, computing summary statistics inline. This has two problems:

1. **Anticipation tax** — every new question (per-faction P/L, stranded ports, crown revenue by month) requires editing the bench. Hard to ask "wait, but what about X?" without round-tripping through code.
2. **Agent friction** — an AI agent investigating run behavior can't easily slice the data; it has to read every bench and trust the aggregation logic.

The fix is a structured event log that the sim emits during the run, dumped to a queryable form (SQLite) at the end. Any question the data supports can then be answered with SQL after the fact, with no sim changes.

## Architecture

### Crate layout

- **`sim-core`** stays pure. No `rusqlite`. It owns:
  - `events.rs` module with one flat `#[derive(Copy, Clone, Debug)]` struct per event category.
  - `WorldJournals` struct grouping `Vec<TradeEvent>`, `Vec<LifecycleEvent>`, etc.
  - `EventMask` bitflag; per-category bit. Default `EventMask::NONE` so headless runs never accumulate.
- **`sim-analytics`** (new tiny crate, depends on `sim-core` + `rusqlite`): the `dump_to_sqlite(&WorldJournals, &Path)` writer. Bench programs depend on it. `sim-core` does not. Alternative: keep the dumper as a `dev-dependency` of `bench_trade.rs`; promote to a crate when a second consumer appears.

### Memory shape: SoA, not enum

Do **not** use a single `Vec<SimEvent>` enum. Enums size every element to the largest variant; that wastes RAM and ruins cache locality when most events are small. Use Struct-of-Arrays: one `Vec` per event category.

```rust
#[derive(Copy, Clone, Debug)]
pub struct TradeEvent {
    pub sim_minute: u64,
    pub port: PortId,
    pub ship: ShipId,
    pub flag: Faction,
    pub good: GoodId,
    pub side: TradeSide,       // Buy | Sell
    pub tons: f32,
    pub base_price: Pesos,
    pub duty_paid: Pesos,
}

pub struct WorldJournals {
    pub trades: Vec<TradeEvent>,
    pub lifecycles: Vec<LifecycleEvent>,
    pub hazards: Vec<HazardEvent>,
    // future: combats, policy denials, etc.
}
```

Constraints:
- Every struct includes `sim_minute: u64` (canonical time unit) so categories can be `UNION ALL`'d into a chronological timeline.
- ID-only fields. No `String`, no `Vec`, no `Box`. Use `ShipId`, `PortId`, `GoodId`, `Faction` (one byte).
- Add `const _: () = assert!(std::mem::size_of::<TradeEvent>() <= 64);` per struct to keep them cache-friendly.

### What stays / what goes

**Stays untouched** (simulation state, not telemetry — sim logic reads these):
- `World::last_month_avg_profit` (shipyard reads this).
- `PortMarket::stockpile`, `treasury`, `crown_silver`.
- All inventory, cash, queue, and price state.

**Migrated to events** (purely observational):
- `HazardCounters` → `HazardEvent` rows.
- Bench's ad-hoc per-faction P/L walks → derived in SQL from `LifecycleEvent` + `TradeEvent`.

### EventMask

Bitflag on `World`. Each push site checks its category's bit before pushing. Headless runs default to `NONE`. Bench programs set whichever bits they want before ticking. Prevents the undrained-events memory leak by making the default behavior a no-op.

### Parallelism

Each Rayon thread in the AI phase holds a thread-local `WorldJournals`. At end of phase, sort each per-thread `Vec` by `(sim_minute, ship_id)` then `extend()` into the world's journals. Mirrors the existing `ShipCommand` flatten pattern at `crates/sim-core/src/world.rs:795`.

### SQLite output

One table per `Vec`. A `timeline` view stitches them:

```sql
CREATE VIEW timeline AS
  SELECT sim_minute, 'trade'  AS kind, ship, port, ... FROM trade_event
  UNION ALL
  SELECT sim_minute, 'combat' AS kind, attacker AS ship, NULL, ... FROM combat_event
  ORDER BY sim_minute;
```

Sample queries an agent would write post-hoc:

```sql
SELECT flag, COUNT(*) AS ships_built FROM lifecycle_event
WHERE kind = 'built' GROUP BY flag;

SELECT port, SUM(base_price * tons) AS volume FROM trade_event
WHERE sim_minute BETWEEN ?1 AND ?2 GROUP BY port ORDER BY volume DESC;

SELECT ship, port FROM lifecycle_event WHERE kind = 'sunk';
```

## Phase 1 deliverables (when we revive this)

1. `sim-core::events`: `TradeEvent`, `LifecycleEvent`, `HazardEvent` + `EventMask` + size asserts.
2. `World::journals: WorldJournals` + `event_mask: EventMask` (default `NONE`).
3. Push sites at ~10–15 obvious points: `market::{buy, sell}`, auction settlement in `world::clear_one_good`, `shipyard::try_build`, ship sinking/scuttling, hazard rolls. Each gated on the mask bit.
4. Migrate `HazardCounters` to `HazardEvent`.
5. AI-phase thread-local journals + deterministic sort+flatten by `(sim_minute, ship_id)`.
6. New crate `sim-analytics` with `rusqlite`; `dump_to_sqlite` + `timeline` view.
7. `bench_trade` sets the mask, runs, calls `sim_analytics::dump_to_sqlite("target/bench_trade.sqlite")`, prints `wrote N events in M tables to <path>`.
8. Regression test: `EventMask::NONE` → all journal vecs stay length 0 after a full tick.

## Strictures (mandatory)

1. **`sim-core` must stay free of `rusqlite`.** The DB writer lives in `sim-analytics` or as a dev-dep of the bench.
2. **Watch enum sizing — but we sidestep this entirely by using SoA.** If an enum is ever introduced, assert each variant size.
3. **Do not delete simulation state masquerading as telemetry.** `last_month_avg_profit`, `crown_silver`, `stockpile`, etc. are read by sim logic and must remain on `World` / `PortMarket`.

## Open design questions

- SQLite vs CSV: SQLite chosen for queryability. CSV remains an option if `rusqlite` ends up being inconvenient.
- Mask granularity: start coarse (one bit per category). Add finer bits (e.g., `ECONOMY_VERBOSE` for per-transaction events) if volume becomes a problem.
- Auto-clear policy: `World::tick` does not auto-clear journals. Consumers must `dump_to_sqlite` then `world.journals.clear_all()` for long-running aquarium mode.

---

## Interim approach (chosen for now)

Rather than build the full event system immediately, we are going to start with **per-entity telemetry fields** on the key actors:

- **Faction**: aggregate counters that belong to the polity (e.g., crown revenue accrued, ships built, ships lost).
- **Port**: per-port aggregates (e.g., lifetime trade volume, lifetime duties collected, count of distinct flags that have docked).
- **Ship**: per-ship aggregates (e.g., lifetime profit, voyages completed, last refit time).

This keeps the surface area small, avoids the cross-cutting refactor of adding a journal layer, and gives benches enough to answer the immediate questions (per-faction P/L, stranded ports) without needing a queryable database. When the questions outgrow per-entity counters, return to this plan and implement the event journal layer.
