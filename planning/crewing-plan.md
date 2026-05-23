# Crewing Plan: Sailors, Recruitment, and Effectiveness

> **Scope.** This document specifies the complete crewing system for
> pirate-sim: how a port grows and loses sailors, how a ship hires and
> discharges them, how crew size affects performance, and how morale
> connects all of it back to provisions, wages, and (eventually) combat.
>
> **Status.** Design — to be implemented across Phase 3 Steps 3, 6, 9.
> Step 3 lands the foundational state and the recruit/discharge loop;
> Step 6 introduces ShipPolicy (pirate vs merchant) which gates some
> recruitment behavior; Step 9 closes the morale → mutiny → piracy loop.
>
> **Calibration anchor.** All numerical defaults draw from
> `planning/research/sailor-recruitment.md` §7. Where that document
> gives a range, we pick a Caribbean-leaning midpoint and flag it for
> the Step 10 calibration pass.

---

## 1. Conceptual model

A **sailor is a person**, not a consumable. Sailors live in ports, get
hired onto ships, sail somewhere, get paid (or don't), and either come
back to a port or die. The simulation tracks them as a population, not
a resource, with the following first-class concepts:

- **A port has a labor pool.** Two tiers: *seasoned* (low mortality,
  high effectiveness) and *unseasoned* (high mortality, lower
  effectiveness, matures into seasoned over time). The pool grows
  organically (apprenticeships, fishing → deep-sea conversion) and
  transiently (ships arriving discharge a few sailors who linger).

- **A ship has a crew, not a complement.** `Ship.crew_alive: u16` is
  the actual head-count, distinct from `ShipStats.crew_typical`. A
  ship may sail with zero crew if it's drifting in harbor; it may
  not put to sea below `crew_min`.

- **Building a ship does not crew it.** This is the critical
  user-driven design decision: `shipyard::try_build` produces a hull
  with `crew_alive = 0`, parked at the launching port. The ship's AI
  then enters a *Hiring* state and draws from the port pool over some
  number of days. Only when crewed does it undock and become useful.

- **Discharge on dock.** When a ship docks, it returns most of its
  crew to the port pool (preserving the seasoned/unseasoned split it
  had aboard). It keeps a small skeleton crew (`crew_min / 2`, the
  "officers and watchkeepers") who stay with the ship between voyages.
  This makes ports feel alive — sailors visibly circulate.

- **Morale binds it all.** Crew morale rises with full bellies,
  paid wages, and prize money; falls with starvation, unpaid wages,
  battle damage, and long voyages. Below a threshold, the crew strikes
  its colors — merchants surrender, pirates mutiny against the
  captain, naval crews desert in port.

---

## 2. Data shapes

```rust
// crates/sim-core/src/pop.rs (NEW in Step 3)

/// Coarse category that drives a port's organic growth rate and the
/// fraction of transient sailors it retains. Calibrated per
/// sailor-recruitment.md §7.1.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize)]
pub enum PortCategory {
    EuropeanHub,        // Seville, Amsterdam, London, Bristol, Nantes
    CaribbeanEntrepot,  // Port Royal, Havana, Cartagena, Curacao
    SmallColonial,      // Bridgetown, Boston, Charleston, San Juan
    PirateHaven,        // Tortuga, Nassau, Petit-Goave
}

pub struct PortDemographics {
    /// Skilled, low-mortality sailors. Drawn first by recruiters.
    pub seasoned: u32,
    /// Recently-arrived or freshly-converted; high mortality,
    /// matures into seasoned over months.
    pub unseasoned: u32,
    pub category: PortCategory,
}
```

```rust
// crates/sim-core/src/ship.rs (Step 3 additions)

pub struct Ship {
    // ... existing fields ...
    pub crew_alive: u16,        // actual head-count
    pub crew_seasoned: u16,     // of crew_alive, how many are seasoned
    pub crew_morale: f32,       // 0.0 (mutinous) – 1.0 (eager)
    pub wages_owed_pesos: f32,  // accumulated unpaid wages
}

pub struct ShipStats {
    // ... existing fields, rename `crew: u32` to `crew_typical: u16` ...
    pub crew_min: u16,       // absolute minimum to put to sea
    pub crew_typical: u16,   // design complement; provisions/speed sized for this
    pub crew_max: u16,       // overcrewed (boarding parties, etc)
}
```

```rust
// New ship lifecycle state added to ShipState

pub enum ShipState {
    Sailing,
    Docked,
    Hiring,    // NEW: at a port, drawing from pool day by day
}
```

> **On `crew_seasoned` as a single integer**, not a vector of records:
> the seasoned/unseasoned split is the only categorical attribute we
> care about per-sailor for v1. Individual identity, name, skills, and
> origin are explicitly out of scope. If we later want named officers,
> they get their own slotmap on top of this aggregate count.

---

## 3. Lifecycle: build → hire → sail → discharge

The full lifecycle of a ship's crew, with explicit phase boundaries:

```
┌──────────┐ try_build       ┌──────────┐ tick (daily)  ┌──────────┐
│  (none)  │ ──────────────► │  Hiring  │ ─────────────►│  Docked  │
└──────────┘                 │ at port  │  crew_alive   │ + small  │
                             │ crew=0   │  ≥ crew_min   │ skeleton │
                             └──────────┘               └─────┬────┘
                                                              │ AI: undock
                                                              ▼
                             ┌──────────┐ tick (hourly) ┌──────────┐
                             │  Docked  │◄──────────────│ Sailing  │
                             │ + return │  dock at port │ + attrition
                             │ crew to  │               │ + wages  │
                             │  pool    │               │ + morale │
                             └──────────┘               └──────────┘
```

### 3.1 Build phase

`shipyard::try_build` produces `Ship { crew_alive: 0, state: Hiring, … }`.
**The port's sailor pool is NOT debited here.** The hull is parked at
the launching port awaiting a crew. The shipyard's role ends with the
hull; sailors are a separate market.

> *Rationale.* Conflating build-and-crew makes shipbuilding artificially
> expensive at small ports (Bermuda might happily build a sloop but
> lack the men to sail it for weeks). It also hides the historically
> interesting case of half-crewed prize ships and Spanish hulks
> rotting at the dock for want of hands.

### 3.2 Hiring phase

Each day a ship in `Hiring` state attempts to draw sailors from the
docked port's `PortDemographics`. Per tick:

```
desired = crew_typical - crew_alive
draw_per_day = recruitment_rate(faction, port_category, demand_pressure)
actually_drawn = min(desired, draw_per_day, pool_available)
```

Drawn sailors come **seasoned-first** (depletes seasoned pool); only if
seasoned is exhausted does the ship accept unseasoned hands.
`recruitment_rate` is the per-day rate from §5 below.

The ship pays a **sign-on bounty** (~1 month's wages per sailor, debited
from `Ship.silver`). If the ship cannot afford the bounty, hiring
stalls — visible in viz, fixable by the captain selling cargo or
borrowing from the home port treasury (mechanism added in Step 9).

Hiring continues until `crew_alive ≥ crew_min`. The AI may then choose
to undock immediately (if cargo is bought and route planned) or to
continue hiring up to `crew_typical` (better effective speed, more
provisions burn — explicit ROI calc).

### 3.3 Sailing phase

Per hour at sea:

- **Provisions burn** = `crew_alive * 0.0018 tons/day / 24`, exactly as
  today but driven by `crew_alive` not `stats.crew_typical`. An
  undercrewed ship costs less to feed; an overcrewed one runs out
  faster.

- **Wages accrue** = `crew_alive * 24 * wage_rate_per_man_hour`, added
  to `wages_owed_pesos`. Wage rate depends on faction and policy
  (see §6).

- **Attrition** = scurvy & accident at sea. ~5%/month for unseasoned,
  ~1%/month for seasoned, sampled per hour as `bernoulli(rate/720)`.
  Losses come from unseasoned first. Tropical mortality (yellow
  fever) is modeled separately when *in Caribbean port*, see §4.

- **Morale tick** = function of `provisions_days_remaining`, `wages_owed`,
  `damage`, `time_since_dock`. See §7.

### 3.4 Dock phase

On arrival:

1. **Pay accumulated wages** from `Ship.silver` into the port's
   economy (small "wages_paid" stat for diagnostics). If the ship
   can't pay, the unpaid portion stays in `wages_owed_pesos` and
   tanks morale further; the crew may strike (refuse to sail again).

2. **Discharge** all but a skeleton crew of `max(crew_min / 2, 2)`.
   Discharged sailors return to the port pool — seasoned to seasoned,
   unseasoned to unseasoned. **Plus** any unseasoned discharged here
   begin a "seasoning" check on subsequent monthly ticks (§4).

3. The ship sits with its skeleton crew until the AI decides on a
   new voyage and transitions back to `Hiring` (or sails immediately
   with the skeleton if it's just a short coastal hop and AI deems
   it acceptable — rare).

---

## 4. Port sailor pool dynamics

The pool is updated on the **monthly tick**. Five mechanisms:

### 4.1 Organic growth (apprenticeship + conversion)

Per `PortCategory`, from research §7.1, converted to monthly:

| Category | Sailors/month into pool |
|---|---|
| EuropeanHub | 40–170 (uniform random) |
| CaribbeanEntrepot | 2–5 |
| SmallColonial | 0.5–1.5 |
| PirateHaven | 0 (organic), grows only via arrivals |

New growth lands in the **unseasoned** pool. (Apprentice boys aren't
seasoned the day they finish their indenture.)

### 4.2 Transient supply from arrivals

Already modeled implicitly via discharge in §3.4. Explicit version:
each ship docking adds `1d4 + (ship_size_class)` unseasoned sailors to
the local pool *in addition to* the discharged crew, representing
sailors who jump ship for a better berth or escape an abusive captain.
This is the dominant pool source for small Caribbean ports.

### 4.3 Maturation: unseasoned → seasoned

Each month, `~3%` of unseasoned sailors mature into seasoned. (Implies
~30-month average exposure to fully season; matches Rodger 1986's
3–5 years apprenticeship + 1–2 years deep-sea before "able seaman"
rating.)

```
matured = rng.bernoulli_count(unseasoned, 0.03)
unseasoned -= matured
seasoned += matured
```

### 4.4 Mortality

Per research §7.5, monthly mortality:

- **Unseasoned in Caribbean port**: 2–3%/month ("seasoning" yellow
  fever)
- **Unseasoned in European port**: 0.5–1%/month
- **Seasoned anywhere**: 0.4–0.7%/month

Implemented as `unseasoned -= bernoulli_count(unseasoned, rate)` etc.
The Caribbean penalty is the key driver — it's why Caribbean
populations of sailors *never accumulate* and remain dominated by
transient flow.

### 4.5 Faction multiplier on growth

Per research §7.3, faction culture affects how readily a port grows
its pool. Multipliers on §4.1 base rates:

| Faction | Multiplier | Why |
|---|---|---|
| England | 1.00 | reference |
| France | 0.90 | corsair culture concentrates the few in St Malo |
| Netherlands | 1.20 | cosmopolitan, recruits foreigners |
| Spain | 0.50 | Casa de Contratación bottleneck |
| Free | 0.30 | only attractive when share system pays |

---

## 5. Recruitment rate (`recruitment_rate`)

How many sailors a single ship can draw per day from a port's pool.
This is the *hiring-side* rate; the pool-side cap is just `min(draw,
pool_seasoned + pool_unseasoned)`.

Base rate per faction (from research §7.3, converted to sailors/day):

| Faction | Peacetime rate (sailors/ship/day) |
|---|---|
| England | 5 (familiar) / 3 (Caribbean) |
| France | 3 |
| Netherlands | 4 |
| Spain | 1.5 |
| Free / Pirate | 4 (high when reputation good; see ShipPolicy gate) |

Multipliers stacked on the base:

- **Demand pressure**: if `N` ships at this port are simultaneously in
  `Hiring` state, divide rate by `(1 + 0.5 * (N-1))`. Multiple ships
  compete for the same dock-side tavern.

- **Pay multiplier** (Step 9): if the captain offers above-market
  wages, multiply rate by `1.5`. Below-market: `0.5`.

- **Pirate-haven bonus**: if `port.category == PirateHaven` AND
  `ship.policy == Pirate`, rate doubles. The share system attracts.

- **Wrong-faction penalty**: an English ship trying to crew at a
  Spanish port: rate × 0.25 (only desperate locals will sign on
  with the foreigners).

War surges (research §7.3 wartime column) deferred to Phase 4 with the
faction-relations system.

---

## 6. Wages and pay

### 6.1 Base monthly wage by rating

From research §1.1: ordinary seaman ~20 shillings/month ≈ **1 peso/month**
(rough conversion: 1 peso ≈ 4 shillings 6 pence). Phase 3 v1 uses:

| Type | Monthly wage (pesos/man) |
|---|---|
| Merchant ship, peacetime | 1.0 |
| Merchant ship, Caribbean tropical premium | 1.3 |
| Privateer (share system) | 0.0 base; share on prize |
| Pirate (share system) | 0.0 base; equal share on prize |
| Navy (deferred) | 1.0 (back-pay system, accumulates) |

Wages accrue per-tick on `Ship.wages_owed_pesos`. The ship pays out
on dock (§3.4). Pirates pay the share system at the moment of prize
distribution (Step 8/9).

### 6.2 Sign-on bounty

One month's wage per recruit, paid at hire (§3.2). Forms a recruitment
cost that scales with crew size. This makes large hulls expensive to
crew, even if the pool exists — historically why a 300-ton merchantman
took 2–3 weeks to fit out vs days for a 60-ton sloop.

---

## 7. Crew effectiveness curves

Crew size affects two things directly: **effective speed** and
**provisions burn**. Other crew-driven effects (gunnery rate of fire,
boarding combat power) are introduced in Step 7 and 8 respectively.

### 7.1 Effective speed

```
ratio = crew_alive / crew_typical
speed_multiplier = piecewise(ratio):
    ratio < crew_min/crew_typical : 0.0    (cannot sail)
    ratio in [min/typical, 0.6]   : 0.6 + 0.4 * (ratio - min_ratio) / (0.6 - min_ratio)
                                            (linear ramp from 60% → ~84%)
    ratio in [0.6, 1.0]           : 0.84 + 0.16 * (ratio - 0.6) / 0.4
                                            (84% → 100%)
    ratio > 1.0                   : 1.0     (no overcrew speed bonus)
```

Plot: a 60%-crewed sloop sails at 84% of design speed (slow tacks,
short-handed sail handling); a fully-crewed sloop hits 100%;
overcrewing helps boarding, not speed.

### 7.2 Provisions burn

Linear in `crew_alive`:
```
daily_burn_tons = crew_alive * 0.0018
```
This is the existing formula with the source replaced. Overcrewed
ships go through provisions fast — historically, the limiting factor
on long pirate voyages with prize crews.

### 7.3 Seasoned bonus (deferred to combat)

`crew_seasoned / crew_alive` is the *quality multiplier* on gunnery
rate of fire and boarding combat power. Doesn't affect speed or
provisions — those are head-count effects. Hooked up in Step 7.

---

## 8. Morale: the connecting layer

Morale is the channel through which bad logistics turn into mutiny.
It's a single `f32` in `[0.0, 1.0]` on each ship.

### 8.1 Modifiers (hourly tick)

| Source | Effect |
|---|---|
| Provisions days remaining < 14 | -0.001 / hour |
| Provisions days remaining < 7 | -0.005 / hour |
| `wages_owed > 2 * monthly_wage_total` | -0.001 / hour |
| Successful prize taken (Step 8) | +0.20 instant |
| Hull damage taken (Step 7) | -0.10 instant per damage event |
| Time in port with full belly + paid wages | +0.001 / hour (rests up) |

### 8.2 Effects

| Morale band | Effect |
|---|---|
| > 0.7 | Default. No effect. |
| 0.4 – 0.7 | -10% recruitment rate (word gets around) |
| 0.25 – 0.4 | -20% effective speed (sullen crew, slow trim) |
| < 0.25 + at sea + debt high | **MUTINY**: triggers `ShipPolicy::Pirate` conversion (Step 9) |
| < 0.10 + in port | crew deserts wholesale, returns to pool, ship reverts to skeleton |

The mutiny trigger is the Step 9 endgame: a bankrupt, hungry, unpaid
merchant turns pirate. The other bands provide a soft gradient so
captains have early warning.

---

## 9. Faction & policy notes (cross-references)

These items belong to other plan documents but are listed here for
completeness of the crewing surface:

- **`Ship.faction: FactionId`** (Step 4) gates §5 wrong-faction
  penalty and §4.5 growth multiplier.
- **`Ship.policy: ShipPolicy`** (Step 6) gates the pirate-haven hiring
  bonus and the share-system wage path.
- **Combat (Steps 7–8)** consume crew via `DamageEvent.crew_killed`,
  feeding the mutiny loop in Step 9.

---

## 10. Implementation slicing within Step 3

Step 3 of `phase-3-plan.md` is the foundation. Within it:

1. **3.a — Data shapes & port pool genesis.**
   - Add `pop.rs` with `PortDemographics`, `PortCategory`.
   - Extend `Port` with a `category: PortCategory` field (in RON).
   - Seed each port's pool with category-appropriate initial values
     (Europeans: 2000–8000 seasoned; Caribbean: 50–300; pirate
     haven: 0–50).
   - Add monthly tick: growth + maturation + mortality (§4).
   - bench_trade prints per-port pool totals.
   - **No behavior change to ships yet**; pool just evolves in
     background.

2. **3.b — Crew on ships, hiring loop.**
   - Add `Ship.crew_alive`, `crew_seasoned`, `crew_morale`,
     `wages_owed_pesos`.
   - Rename `ShipStats.crew` → `crew_typical`; add `crew_min`,
     `crew_max` to RON (with backfill).
   - Add `ShipState::Hiring`.
   - `shipyard::try_build` produces `crew_alive = 0, state = Hiring`.
   - AI `dock_tree` gets a `Hiring` branch that pulls from
     `PortDemographics` per §3.2, §5.
   - AI undocks only when `crew_alive >= crew_min`.
   - **Wages and morale not yet wired** — Step 3.c.

3. **3.c — Wages, morale, discharge.**
   - Wages accrue per §6, paid on dock per §3.4.
   - Discharge returns crew to pool (skeleton retained).
   - Morale ticks per §8.1 with stubs for damage/prize events
     (those land in Steps 7–8).
   - Provisions burn switches to `crew_alive` (§7.2).
   - `effective_speed` adds the crew curve (§7.1).
   - bench_trade prints crew columns + morale histogram.

4. **3.d — Calibration sweep.**
   - 1-year headless run. Verify the Caribbean pool stays in a
     2x band around its seed; no port's pool collapses or
     explodes; ships average ~80–100% crew at sea; bankruptcy
     rate doesn't worsen vs Step 2 baseline.

After 3.d ships green, we move on to Step 4 (factions + spatial hash).

---

## 11. Explicit non-goals for Phase 3

To keep Step 3 from sprawling, the following are **out of scope**
until Phase 4 or later:

- Named individual sailors with names, skills, biographies.
- Captain as a distinct entity from the ship.
- Specialist roles (gunner, surgeon, carpenter, navigator) with
  effects beyond raw head-count.
- Letters of Marque as data with issuance dates and expiry.
- Impressment events (press-gang sweeps a port pool).
- Post-war demobilization shocks.
- Slave trade as a sailor source (cargo-side modeled; sailor-side not).
- Crew nationality as a per-sailor attribute that affects
  cross-faction hiring rates beyond the flat multiplier in §4.5.

---

## 12. Open calibration questions (for Step 3.d sweep)

1. Is the 3%/month maturation rate too slow? Historical
   apprenticeship was 3–5 years (60 months), so 1.7% would be a
   purer fit. Test: does 3% inflate the seasoned pool unrealistically?

2. The wrong-faction recruitment penalty of 0.25× — is it too harsh?
   In practice, Curaçao routinely crewed mixed nationalities. Maybe
   PortCategory should override (CaribbeanEntrepot ignores faction
   match entirely).

3. The skeleton crew of `crew_min / 2` — too generous? A docked
   merchantman historically often had only the master and a boy
   aboard. Test: does skeleton size affect anything we care about?

4. The morale recovery rate of 0.001/hour in port — at this rate full
   recovery from 0.25 to 1.0 takes 31 days. Probably too slow given
   typical 1–2 week port stays.

These are tuning knobs, not architectural decisions. Land defaults in
Step 3.c; tune in 3.d and again in Step 10.
