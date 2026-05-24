# Phase 4 — Combat Realism: Ordnance Supply Side, Repair, Sub-Tick Combat

**Theme:** Make combat have *bite*. The pieces consumption depends on (ordnance production + repair) and the pieces that make engagements feel real (sub-tick resolution + forts) are the missing links between the deterministic combat math we already have and the political/strategic layer Phase 5 will add.

**What we already have (Step 7/8 — don't rebuild):**
- `Ship::hull_integrity` + `Ship::rigging_integrity`, sinking at `hull ≤ 0`.
- `combat::compute_broadside_damage(cannons, range)` — deterministic falloff from 1.0× point-blank to 0.3× at 0.5 NM.
- `combat::broadside_supply_cost(cannons)` — every broadside debits `POWDER_TONS_PER_GUN` + `SHOT_TONS_PER_GUN` per gun from the attacker's cargo; if either is short, the command is silently dropped at resolution.
- Boarding action with deterministic casualty math and prize/burn outcome.
- Storm/foundering/fire hazards reducing hull integrity hourly (`weather/hazards.rs`).
- Careening at port restores `hull_fouling` and `teredo_damage` (but **not** combat damage — see §3 below).

**What's missing (this is Phase 4):**
1. **Ordnance supply side.** Consumption is wired; nobody produces gunpowder or shot. Ships start with seeded cargo and can never refill once empty.
2. **Repair at port.** Hull and rigging integrity degrade monotonically over a ship's lifetime — careen recovers fouling/teredo but never combat damage. Long-lived merchants accumulate scars they can never shed; eventually any sustained piracy environment grinds the fleet to nothing.
3. **Sub-tick combat resolution.** Hourly granularity is too coarse for multi-broadside engagements, fort vs. ship duels, and the reload cadence that makes a sloop's small magazine economically *meaningful*.
4. **Forts.** No stationary defenders at all today. Without them, a major port and a tiny anchorage look identical to a pirate.

**Deferred to Phase 5+:** Relations matrix + Letters of Marque, war declaration cycle + demobilization shocks, port Behavior Trees (defense doctrine + bounties). Phase 4 prepares the substrate; Phase 5 plugs in the political driver.

---

## 0. Scope contract

| | In | Out |
|---|---|---|
| **Ordnance supply** | Production at 4 European hubs; AI top-up at port departure | New industry abstraction (FW-1); ordnance grades (FW-3) |
| **Repair at port** | Hull + rig restoration over time at port, paid in silver; tracks pesos/HP rate | Dry-dock vs. afloat repair distinction; partial overhauls; insurance |
| **Sub-tick combat** | 5-minute resolution inside the engagement-locked hour; reload model; disengagement rule | Sub-tick AI (FW-4); fleet maneuver doctrine (FW-5); weather-in-combat (FW-6) |
| **Forts** | Stationary batteries at major ports; range, fire rate, port-pool ordnance, deterministic fire | Damage to structures; siege; garrison morale; counter-battery |
| **AI cadence** | Hourly AI sets engagement intent; sub-tick loop resolves | Reactive in-combat AI (no mid-exchange retargeting) |
| **Calibration** | Extend `bench_trade` for ordnance flow + repair spend; new `bench_combat` for sub-tick stability | Equilibrium LP including ordnance (Phase 5) |

The choice of these four before the political work is deliberate: it gives Phase 5 a *real* combat substrate to drive, instead of trying to design relations on top of placeholder combat.

---

## 1. Ordnance supply side (do this first)

### 1.1 Goods (no schema changes)

`ids::GUNPOWDER` and `ids::CANNON_SHOT` are already in `GoodsRegistry::starter()`, already pricable through `PortMarket`, already consumed by `combat::broadside_supply_cost`. What changes: ports start *producing* them.

### 1.2 Production — *simple v1*

Extend a handful of existing European-hub recipes (`PortArchetype`) to produce gunpowder and shot:

- `EuropeanLondon`: +powder, +shot (Royal Powder Mills + Royal Foundries)
- `EuropeanAmsterdam`: +powder, +shot (Dutch arsenals; the Republic was the dominant 17C powder exporter)
- `EuropeanCadiz`: +powder, +shot (Casa de la Contratación arsenal; supplied the *flota*)
- `EuropeanNantes`: +powder only (modest)

**Deliberate v1 simplification.** We're folding ordnance into the existing recipe primitive (`monthly_outputs` / `monthly_inputs`) rather than introducing an "industry" or "facility" abstraction. The vision is that production systems eventually get encapsulated — a port can compose a sugar mill + a powder mill + a foundry + a shipyard, each with its own inputs/outputs/maintenance/labor. Phase 4 explicitly does **not** build that. Future-work hook:

> **FW-1 (Phase 6+):** Encapsulate production into composable industries per port. `ProductionRecipe` becomes one of many `Industry` instances on a port; ports load their industry set from data. This is the level at which "Spanish silver minting at Potosí" or "Boston shipyard expansion" become first-class.

Calibration numbers:
- Historical baseline: Amsterdam exported ~3000–5000 *barrels* of powder/year to the colonies c. 1680 (one barrel ≈ 50 kg ≈ 0.05 t). That's 150–250 t/yr ≈ 12–20 t/month.
- In-sim: a sloop carries 4 t powder; 6th-rate carries ~20 t. With current `POWDER_TONS_PER_GUN = 0.01`, an 8-gun sloop spends 0.08 t per broadside — magazine ≈ 50 broadsides. Add the future fort draw (~1 t/month per major fort, ~20 forts) → ~20 t/month structural draw + ~30–40 t/month combat draw. Total ~50–60 t/month system-wide. Sized accordingly: 4 hubs × ~15 t/month each = 60 t/month gross.

### 1.3 AI demand — top-up at port

Add a thin layer to the existing port-departure routine: top up ordnance from the local market before sailing if (a) the ship is below its desired loadout and (b) local market has stock at a reasonable price.

```rust
fn desired_ordnance_loadout(ship: &Ship, stats: &ShipStats) -> (f32, f32) {
    let scale = if ship.policy == ShipPolicy::Pirate { 0.5 } else { 0.1 };
    let cap = stats.cargo_capacity_tons * scale;
    let powder = cap * 0.5;
    let shot = powder; // 1:1 weight ratio in v1
    (powder, shot)
}
```

**Does not** trigger dedicated arsenal-run voyages in v1 — that's behavior we want to *observe emerge* from the trade planner as ordnance arbitrage opportunities appear (drained Caribbean ports → high sell price → trade routes form). If bench shows ordnance stuck at arsenal hubs with no distribution after 730 days, add an explicit "ferry ordnance" trade signal in v2 (FW-2).

### 1.4 Acceptance

Extend `bench_trade 730`:
- Annualized ordnance flow per producer hub > 0; per non-producer hub net negative (someone's buying).
- After 730 days: ≥ 70% of Caribbean ports have non-zero powder stock at last sample.
- Average ship powder cargo > 0.5 t across the fleet at end-of-run (i.e., ships are restocking, not just burning through seeded cargo).

New unit tests:
- `european_hub_produces_powder_monthly` (recipe output verified)
- `port_departure_tops_up_ordnance_from_market`
- `pirate_desired_loadout_exceeds_merchant_loadout`

---

## 2. Repair at port (do this second — small, decoupled)

### 2.1 Problem

`hull_integrity` and `rigging_integrity` are monotonically decreasing over a ship's lifetime: storms whittle them, combat carves chunks, and there is *no* recovery path. Careening (the existing `tick_careen`) cleans biofouling and rolls back teredo damage but doesn't touch combat scars. A merchant that survives a single pirate encounter at 60% hull carries that scar to the grave. Over long horizons this means: the fleet's average hull condition trends to zero, ships sink from storms at increasing rates, and shipyards are the only source of fresh integrity. The economy can't reach steady state at the historical fleet size.

### 2.2 Model

At a docked port, a ship's hull and rigging recover at a per-hour rate, debiting silver from `ship.silver` to the port's market. Rates and prices:

| Resource | Rate (HP/hr at port) | Cost (pesos/HP) | Rationale |
|---|---:|---:|---|
| Rigging | 1.0 | 1.5 | Cordage + canvas + a few days' rigger labor; cheap by ton, fast. |
| Hull | 0.3 | 6.0 | Carpentry on oak plank with iron fastenings; slow, expensive. Historical ratio holds: a 6-month overhaul cost ~30% of build price for a Royal Navy 4th-rate. |

A full 100-HP rebuild for a sloop hull = ~333 hours ≈ 14 days at port, ~600 pesos. A typical battle-scarred merchant at 70% hull recovers in 100 hours ≈ 4 days, ~180 pesos. Both line up with historical refit cycles (3–6 weeks at port, single-digit-percent of voyage revenue).

Repair only happens if the ship has the silver. If silver is insufficient, the ship pays what it can and the rest is recorded as **drydock debt** (`Ship::debt += unpaid`). This composes with the existing wage-debt mechanism — bankruptcy threshold and shipyard recovery already handle accumulated debt.

### 2.3 Materials (optional, deferred)

V1: repair is silver-only. The market sells "the carpenter's labor" abstractly. We do **not** require the port to have actual `NAVAL_STORES` / `MANUFACTURES` stock — that's a coupling we want eventually but not now (FW-8). Trade-off: a port can repair ships even when it's economically destitute. Acceptable for v1 because real ports were never destitute on this axis; carpenters survived even in depressed years.

### 2.4 AI behavior

The existing "docked at port" routine ticks careen and pays wages. Add a third sub-step: tick repair. Ships always repair when docked (no opt-in flag); this is the merchant's standard maintenance behavior. Captains who want to skip repair (e.g., desperate pirate fleeing with a hot hold) will simply leave port faster — repair runs only while docked.

### 2.5 Acceptance

- `bench_trade 730`: average fleet hull integrity at end-of-run > 60% (currently trending down toward 0).
- Per-ship lifetime hull integrity time series shows the characteristic sawtooth: voyage attrition + occasional combat damage, then port restoration.
- New unit tests:
  - `docked_ship_recovers_rigging_at_expected_rate`
  - `docked_ship_pays_silver_for_hull_repair`
  - `insufficient_silver_creates_debt_not_partial_repair_freebie`
  - `repair_does_not_exceed_max`

---

## 3. Sub-tick combat (do this third)

### 3.1 Time model

Inside the existing hourly tick, when at least two hostile ships (or a hostile ship + a fort) are within **engagement range** (0.5 NM, the existing `combat::CANNON_RANGE_NM`), run a **5-minute sub-tick loop** (12 sub-ticks per hour) for the duration of the hour. Outside-combat ships skip the sub-tick loop entirely — only engaged participants pay the cost.

The hour-level AI sets engagement *intent* (issue `FireBroadside` / `AttemptBoard` once); the sub-tick loop converts that intent into a 12-step exchange where reload, ordnance debit, hit application, and disengagement get checked at each step.

### 3.2 Tick architecture

Extend `tick_hourly_ai_and_physics` (already split into AI / Resolution / Mutation+Physics phases by A1):

```
AI Phase                  (existing — hourly cadence; sets intent)
Resolution Phase          (existing — steer, dock; combat commands now
                           translate to "engagement participation")
Sub-tick combat loop:                                  ← NEW
    snapshot engaged participants (ships + forts)
    for sub_tick in 0..12:                             // 5-min steps
        for each participant in deterministic order:
            if next_fire_at <= current_minute and target in range and ordnance ok:
                fire → damage + ordnance debit (via existing combat::* helpers)
                next_fire_at = current_minute + reload_minutes(participant)
        check disengagement: any ship > 2 NM from all hostiles for full sub-tick → drop
Mutation/Physics Phase    (existing — non-engaged ships; mutiny; weather; wages)
```

**Determinism:** sub-tick loop iterates a fixed snapshot of participant IDs; per-participant RNG state advances in that order.

**Performance:** at any tick we expect ≤ ~20 engaged participants out of 480+ ships. 20 participants × 12 sub-ticks × cheap pair-wise work = negligible vs. the hourly economic + AI work.

**Re-use existing combat math:** `compute_broadside_damage` and `broadside_supply_cost` stay as-is. The sub-tick loop calls them up to 12× per engaged ship per hour instead of once.

### 3.3 Reload model

Each ship/fort gets a `next_fire_at_minute: f32` (sub-tick precision). On firing:

```
next_fire_at = now + reload_minutes(participant)
```

`reload_minutes` for ships ≈ `1.5 * (2.0 - seasoned_ratio())` — i.e., 1.5 min for fully-seasoned crews, 3.0 min for fully green. **Here is where the A2 `crew_seasoned` field finally bites:** seasoned crews fire twice as fast. Forts have a flat 2.0 min reload (land-based gun crews historically less practiced).

### 3.4 Engagement / disengagement

A ship enters engagement when its hourly AI emits a `FireBroadside` or `AttemptBoard` against a nearby target, or when a fort's range circle is breached by a hostile.

A ship can **disengage** by reaching > 2 NM separation from all hostile contacts for a full sub-tick. This is the breakaway condition: a pursuer must commit to closing or accept the loss of contact.

### 3.5 Forts

New `Fort` struct on `Port`:

```rust
pub struct Fort {
    pub guns: u16,
    pub range_nm: f32,
    pub powder_pool: f32,
    pub shot_pool: f32,
    pub next_fire_at_minute: f32,
}
```

Forts seeded at major ports based on historical garrison data (rough tiers):
- **Tier 1 — major batteries** (24+ guns, 1.5 NM): Cartagena, Havana, Port Royal, Cadiz, Veracruz
- **Tier 2 — modest forts** (8–16 guns, 1.0 NM): most colonial capitals
- **No fort:** pirate havens, tiny ports

Fort ammunition refills from the port's market on a monthly tick (port "buys" from itself at market price; in v1 booked as a state expenditure, no economic record). Long-term FW-9: ordnance budget as a faction-level expense line.

**Hostility rule (interim).** Until the relations matrix lands in Phase 5, "hostile" = `ShipPolicy::Pirate` OR (ship faction differs from port faction AND ship faction is not `Free`). This is a single function call swappable for a relations-matrix lookup later — no other refactor needed.

### 3.6 Acceptance

New benchmark `bench_combat`:
- Seed a known scenario: 1 pirate sloop attacks 1 trader near 1 major-fort port.
- 100 trials with varying RNG seeds.
- Distributional checks:
  - Pirate engagement-success rate (sloop wins) drops measurably with fort presence.
  - Average rounds expended per engagement ≈ 30–80 (sanity).
  - Average engagement duration: 15–45 minutes sim time.
  - No infinite loops; every trial terminates within 60 minutes sim time.

Existing tests (`tests/combat.rs`, 8 tests) must continue to pass — they exercise the resolution phase, not the sub-tick loop. The adaptation is small: issue a combat command, then *advance one full hour* (which now includes 12 sub-ticks) and check the post-state. Damage totals should be ≥ what the old single-broadside-per-tick model produced.

`bench_trade 730` regression check:
- Bankrupt count should not increase meaningfully (target: ≤ 95, vs. post-A3 baseline 86). If it spikes, we're killing too many ships in combat — tune damage tables or sub-tick reload rate.
- Pirate-lost count is allowed to rise (forts now contribute to pirate attrition — this is the *point*).
- Average fleet hull integrity should hold steady above 60% (combo of §2 repair and reasonable damage per encounter).

---

## 4. Out-of-scope / parking lot

Carrying these forward as Phase 5 / FW-N items:

- **FW-1:** Composable industries (encapsulate `ProductionRecipe` into per-port industry sets).
- **FW-2:** Dedicated arsenal-run trade signal (if v1 emergent distribution proves insufficient).
- **FW-3:** Ordnance grades (round / grape / chain) — different damage profiles, different production.
- **FW-4:** Sub-tick AI (in-engagement retargeting, retreat decisions). Currently AI is set at hour boundary.
- **FW-5:** Fleet maneuver doctrine (line-ahead, weather gauge) — sub-tick combat treats each ship as independent in v1.
- **FW-6:** Weather effects on combat (rough seas reduce hit rate, wind affects who has the gauge).
- **FW-7 (Phase 5):** Relations matrix + LoM + war cycle + port BT (defense doctrine, bounties).
- **FW-8:** Repair consumes `NAVAL_STORES` / `MANUFACTURES` instead of pure silver.
- **FW-9:** Fort ordnance as a faction-level budget line, not a silent draw.

---

## 5. Sequence of work

1. **1.1–1.3 (ordnance production + AI top-up)** — recipe additions, top-up logic. Tests + small `bench_trade` extension.
2. **2.1–2.4 (repair at port)** — hull/rig recovery while docked, silver debit, debt path. Tests + check `bench_trade 730` average hull integrity climbs.
3. **3.1–3.4 (sub-tick combat for ships)** — engagement detection, sub-tick loop, reload model, disengagement. New unit tests; existing combat tests adapted.
4. **3.5 (forts)** — `Fort` struct, seed data, integration into sub-tick loop.
5. **3.6 (calibration)** — `bench_combat`, regression check against `bench_trade 730`, tune damage tables / reload times if needed.
6. **Development-log entry** + `phase-4-postmortem.md` skeleton (so we have a place to drop issues during the work).

Each step ends with `cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace` green, plus the relevant bench.

