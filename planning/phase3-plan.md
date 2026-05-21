# Phase 3: Encounters, Combat, and Piracy

> **Goal of Phase 3:** make ships *discover each other*, and have that
> discovery matter. Phase 2 ships move past each other invisibly; in
> Phase 3 a merchant sees a pirate on the horizon and runs, a navy
> frigate gives chase, two captains agree to convoy, and a broke
> captain hauls down his honest colors and turns predator.
>
> This makes **piracy** in *pirate-sim* finally exist as a system, not
> a name. It also closes the loop on Phase 2: bankruptcy stops being
> an inert endpoint and becomes a recruitment channel for the pirate
> fleet — historically the dominant one in the Caribbean.

## Where we are at the end of Phase 2

- **Economic skeleton:** 9 goods, port markets with stockpiles and
  dynamic pricing, monthly production recipes, Europe-gateway sinks.
- **Ships:** 5 type catalog (Sloop / Brigantine / Bark / Fluyt / Ship)
  with per-yard specialization, ROI-based shipyard build/retire,
  home-port capital settlement, chandler/freight credit (`Ship.debt`).
- **AI:** trader behavior tree picks profitable (good, destination)
  pairs, takes chandler credit when broke, takes tramping credit when
  it has hold space but no cash; settles debt before dividends.
- **Visualization:** click-to-select ports and ships; per-ship
  inspector panel; market panel; commodity flow diagnostics in bench.
- **What's missing for a living world:** ships ignore each other. A
  Spanish galleon and an English merchant cross within yards of each
  other and notice nothing. There is no piracy. There is no combat.
  There are no factions. The economy is a peaceful clockwork.

## The thesis

> **Identity, encounter, consequence.** Every ship belongs to a
> faction. When two ships come within sight, they recognize each
> other and act on what they see. The simulation gains the events
> that make the rest of the genre — combat, fear, escort, smuggling,
> piracy — possible.

## What "sound" means for Phase 3

The system is sound if all of these hold in a 90-day demo:

1. **Hunt:** A pirate sloop spawned at Tortuga hunts and captures a
   merchant within ~30 days on average.
2. **Flee:** A merchant carrying valuable cargo diverts toward a
   friendly port when it spots a known pirate within sight range.
3. **Loot:** Captured cargo and silver transfer to the victor. The
   prize hull is either sunk, burned, or (rarely) added to the
   pirate's flotilla. The pirate later sells the loot at a haven at
   a discount.
4. **Recruitment:** Some bankrupt merchants turn pirate. The pirate
   fleet sustains itself organically from economic distress.
5. **Suppression:** Navy patrols on a route visibly reduce piracy
   there; unpatrolled lanes degrade in throughput.
6. **Trade law:** Faction + port + good determines trade legality.
   Ports refuse trade with hostile flags; smuggling at non-home
   ports becomes a real (profitable, risky) activity.

## Scope (IN)

| # | Item | Notes |
|---|---|---|
| 1 | **Factions** (English, French, Spanish, Dutch, Pirate, Neutral) with a static relations matrix; flag fields on `Ship` and `Port` | Identity is the prerequisite for all of the below. Static seed data only — AI factions don't make strategic decisions yet. |
| 2 | **Proximity / encounter system** | Spatial hash over ship positions; pairs within sight range emit an `Encounter` event. Sight range ~12 NM clear, less in bad weather. Re-evaluated every ~4 sim hours to match BT cadence. |
| 3 | **Identification** | At Phase 3 start: perfect ID (you always know what flag and what type). Fog-of-war ID is a Phase 4 elaboration. |
| 4 | **Lightweight combat** | Two ships in range exchange broadsides; hull/crew/cargo damage; outcome = capture, strike colors, sink, or break-off. Resolved over a few combat ticks, not single-tick. |
| 5 | **Pirate AI variant** | Predator BT: patrol shipping lane → identify prey → close → engage → loot → return to haven to fence and provision. |
| 6 | **Bankruptcy → piracy transition** | A ship that hits `MAX_SHIP_DEBT` and stays there for N consecutive port arrivals (or whose crew morale collapses) probabilistically strikes its colors and switches to pirate AI. |
| 7 | **Pirate haven port subtype** | Tortuga, Petit-Goâve, Nassau, etc. Accepts any flag; no Navigation Acts; fences goods at a discount; provides cheap provisioning and a place to careen out of sight. |
| 8 | **Trade-law Rhai hook** | `check_trade_legality(ship, port, good, date) -> Legality`. Minimal Navigation-Acts ruleset (1660 / 1663). Ports may refuse, accept normally, or accept as smuggled (price penalty + seizure risk). |
| 9 | **Convoy & escort intent** | Merchants on a route may pair up and travel in formation for safety; navy ships can be tasked to shadow a high-value merchant. Defense as emergent strategy, not central planning. |

## Scope (OUT — deferred to Phase 4+)

- **Populations and labor.** Still abstracted into per-port prosperity.
- **Buildings as entities.** Still single-recipe per port.
- **AI-faction strategic decisions.** No mid-sim war declarations,
  no fleet deployments, no policy changes. Relations matrix is read-only.
- **Player command UI.** Still fully autonomous; player observes.
- **Storms coupled to markets.** Storms remain navigational only.
- **Diplomacy.** No relations drift, no treaties, no peace conferences.
- **Reputation / wanted lists.** Pirates are pirates; no fame system
  yet (good Phase 4 layer once player intervention exists).
- **Multi-ship engagements / line-of-battle tactics.** Combat is
  pairwise. Fleet battles are Phase 5.

## Architecture

### New data shapes

```rust
// crates/sim-core/src/faction.rs (NEW)

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct FactionId(pub u8);

pub struct Faction {
    pub id: FactionId,
    pub name: &'static str,
    pub kind: FactionKind,
    pub primary_color: (u8, u8, u8),
}

pub enum FactionKind { Metropolitan, Colonial, Pirate, Neutral }

pub enum Stance { Friendly, Neutral, Hostile, NoQuarter }

pub struct FactionRegistry {
    factions: Vec<Faction>,
    relations: Vec<Vec<Stance>>,   // square matrix indexed by FactionId
}
```

```rust
// crates/sim-core/src/encounter.rs (NEW)

pub struct Encounter {
    pub a: usize,             // ship index
    pub b: usize,
    pub range_nm: f32,
    pub a_recognizes_b: bool, // for future fog-of-war
}

pub struct EncounterSystem {
    grid: SpatialHash,        // existing or new
    cooldowns: HashMap<(usize, usize), Tick>,  // don't re-encounter every tick
}

impl EncounterSystem {
    pub fn step(&mut self, world: &World) -> Vec<Encounter>;
}
```

```rust
// crates/sim-core/src/combat.rs (NEW)

pub struct CombatState {
    pub attacker: usize,
    pub defender: usize,
    pub range_nm: f32,
    pub ticks_engaged: u32,
}

pub enum CombatOutcome {
    Captured { winner: usize, loser: usize },
    Sunk(usize),
    BrokeOff(usize),
    Ongoing,
}

pub fn tick_combat(state: &mut CombatState, world: &mut World) -> CombatOutcome;
```

### Ship/Port changes

```rust
// ship.rs
pub struct Ship {
    // ... existing
    pub faction: FactionId,        // NEW
    pub hull_damage: f32,          // 0 = pristine, 100 = sinking
    pub crew_alive: u16,           // NEW (currently implicit)
    pub gunnery: f32,              // 0.0–1.0; pirate veterans > navy > merchant
    pub morale: f32,               // 0.0–1.0; below 0.2, mutiny / strike colors
}

// port.rs
pub struct Port {
    // ... existing
    pub faction: FactionId,        // NEW
    pub haven: bool,               // NEW — pirate fence + safe haven
}
```

### Combat model (v1, deliberately simple)

Each combat tick (~15 sim minutes):

1. **Range and maneuver.** Both ships try to close or break off based
   on speed advantage + wind. The faster/handier ship dictates range.
2. **Gunnery exchange.** Each ship rolls damage =
   `broadside_weight × gunnery × range_falloff × random(0.5..1.5)`.
   Damage applied to hull, crew, cargo proportionally.
3. **Outcome check.**
   - Hull >= 100 → sunk (cargo lost with ship)
   - Crew <= 30% → strike colors → captured
   - Morale <= 20% → strike colors → captured
   - Either side decides to break off (significant speed advantage
     and damage taking too much) → flee
4. **Loot transfer.** On capture: all cargo + silver moves from
   loser to winner. Hull either added to winner's flotilla (rare,
   only if winner has crew to spare) or burned.

### Pirate BT outline

```
Selector:
  // Priority 1: docked at haven → fence + refit + recruit
  Sequence: [IsDocked, SellAllLoot, ResupplyAndCareen, BuyProvisionsOnly, Undock]

  // Priority 2: in combat → keep fighting
  Sequence: [InCombat, Engage]

  // Priority 3: prey visible → close and attack
  Sequence: [SeePrey, Pursue, Engage]

  // Priority 4: low provisions or hold full → return to haven
  Sequence: [NeedsHavenRun, NavigateToHaven]

  // Priority 5: patrol shipping lane
  Sequence: [PickLane, Patrol]
```

### Bankruptcy → piracy transition

A trader becomes pirate when **all** hold:

1. `ship.debt >= 0.9 × MAX_SHIP_DEBT` (effectively maxed out)
2. Has been so for `>= 60 sim days` of failed recovery attempts
3. Currently at a port that is NOT its home port (captain can't be
   bailed out by his owners)
4. RNG roll (per-month) clears a configurable threshold (default 30%)

When it fires:
- `ship.faction = FACTION_PIRATE`
- `ship.owner_port = None`
- `ship.debt = 0` (creditors lose; that's the historical reality)
- AI swapped to pirate BT
- Crew morale boosted (better-than-starving prospects)
- A faint event log entry is emitted for the bench output:
  `"Sloop 7 (ex-Boston) strikes colors at Petit-Goâve."`

### Trade-law hook

```rust
// market.rs additions

pub enum Legality { Legal, Smuggled, Forbidden }

pub trait TradeLawHook {
    fn check(&self,
             ship: &Ship,
             port: &Port,
             good: GoodId,
             date: Date) -> Legality;
}

pub struct DefaultLaw;
impl TradeLawHook for DefaultLaw {
    fn check(...) -> Legality {
        // Rhai-evaluated when scripting is wired; hardcoded v1.
        // Built-in: Navigation Acts after 1660, hostile-faction bans,
        // haven free-for-all.
    }
}
```

`PortMarket::try_buy` / `try_sell` consults the hook. `Smuggled`
trades go through with a price penalty (factor takes a cut) and add
a per-month "discovery roll" — if discovered, cargo confiscated and
ship gets a fine added to `debt`.

### Encounter visualization

- A thin colored line drawn between two ships within sight range,
  with color encoding the relationship (green = friendly, yellow =
  neutral, red = hostile).
- Combat: thicker red line + small puff sprites; ships' hull damage
  shown as a fading-red overlay on the ship triangle.
- Pirate ships drawn with a distinct color (e.g., black/dark red)
  and a small ⚑ marker.

## File-level diff outline

```
crates/sim-core/src/
├── faction.rs          NEW
├── encounter.rs        NEW
├── combat.rs           NEW
├── ai.rs               → pirate BT variant; navy patrol variant
├── ship.rs             + faction, hull_damage, crew, gunnery, morale
├── port.rs             + faction, haven
├── market.rs           + Legality, TradeLawHook
├── world.rs            owns FactionRegistry + EncounterSystem
└── lib.rs              re-export

data/
├── factions.ron        NEW
├── ports.ron           + faction & haven flags
└── trade_laws.ron      NEW  (or scripts/trade_laws/*.rhai when scripted)

crates/sim-viz/src/main.rs
└── encounter lines, pirate styling, combat puffs, faction colors
```

## Implementation order (each step ships green)

1. **Faction registry + flag fields.** Pure plumbing. No behavior
   change. Ports and ships get faction IDs from seed data. Viz
   gains a faction-color overlay on port markers. Bench unchanged.

2. **Proximity / encounter detection.** Spatial hash + `Encounter`
   events. Visualization draws lines between paired ships. Still no
   behavior change — but you can *see* the meetings happen for the
   first time, which is qualitatively striking.

3. **Pirate BT variant + minimal combat.** Seed one pirate sloop at
   Tortuga; let it hunt anything not flagged Pirate within sight.
   Capture = take cargo + silver, sink the victim. Wins the day:
   first kill makes a Caribbean of 10 merchants feel suddenly
   inhabited.

4. **Pirate haven mechanics.** Tortuga/Petit-Goâve/Nassau accept
   the pirate flag, fence cargo at a discount, sell provisions.
   Pirate now has a complete loop.

5. **Bankruptcy → piracy.** Pirate fleet now sustained organically
   from the economic system. The economic pressure of Phase 2 has a
   visible outlet.

6. **Trade-law hook + Navigation Acts.** Foreign-flag ships refused
   at most ports; smuggling at non-home ports becomes profitable but
   risky; smuggled cargo has a confiscation roll.

7. **Navy patrols.** Royal Navy frigates assigned to lanes
   (Jamaica–Bristol, Charles Town–London) intercept identified
   pirates. Adds the suppression side of the loop.

8. **Convoy/escort intent.** Merchants on a high-piracy route may
   pair up; navy can be tasked to shadow a single high-value
   merchant. Emergent defense.

9. **Calibration pass.** Run a 90-day demo. Tune sight range,
   combat lethality, bankruptcy-to-piracy rate, smuggling discount,
   patrol density, until:
   - Pirate fleet stabilizes around 5–15% of total ship count
   - Average merchant survives 4–8 voyages before incident
   - High-piracy lanes show 30–60% throughput reduction
   - At least one navy interdiction per 30 days on a patrolled lane

## Open questions to resolve before step 1

1. **Faction granularity:** one nation per metropolitan power (England,
   France, Spain, Netherlands), or separate Royal vs. colonial sub-factions
   (English Crown vs. Massachusetts Bay vs. Jamaica)? **Recommended:
   one per power for v1**, sub-factions in Phase 4 when politics matters.

2. **Pirate as one faction or many crews:** is "Pirate" a monolithic
   faction or do individual pirate ships have their own captains
   with separate reputations? **Recommended: monolithic for v1**;
   crew identity is a Phase 4 reputation system.

3. **Combat lethality dial:** how often does combat sink a ship vs.
   capture vs. break off? **Recommended target distribution:** 10%
   sunk, 50% captured, 40% broke off. (Adjust during step 9.)

4. **Sight range model:** flat 12 NM, or scaled by weather/visibility/
   night-day? **Recommended: flat for v1**, weather coupling in Phase 4.

5. **Pirate haven economic effect:** does fencing have an explicit
   discount (e.g., 60% of port price), or is the haven just an
   ordinary market with relaxed laws? **Recommended: explicit
   discount** — keeps the gameplay choice clean.

6. **Captured-hull policy:** added to winner's flotilla (more
   ships), sold (cash), or burned? **Recommended: 70% burned, 25%
   sold at haven for partial value, 5% added** (and only if the
   pirate already has enough crew to spare).

7. **Bankruptcy-to-piracy probability:** the 30%/month figure is a
   guess. Should it scale with how easy honest trade is? **Probably
   yes**, but defer the dynamic tuning to step 9.

8. **Smuggling discovery model:** flat per-month roll, or scales
   with port size / patrol presence? **Recommended: scales with
   port faction's local "vigilance"**, which is just a static port
   property for now.

## What this plan deliberately does NOT do

- It does not introduce **strategic AI factions.** Wars, treaties,
  and policy changes do not happen during a Phase 3 run.
- It does not introduce **populations** or **pop-driven labor**.
  Ports continue to be a single prosperity multiplier.
- It does not introduce **building-level production** or shipyard
  upgrades.
- It does not introduce **player command** — the player observes.
- It does not introduce **reputation systems** or named individual
  captains.
- It does not introduce **multi-ship combat tactics** beyond
  pairwise engagements.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Pirates wipe out the merchant fleet faster than recruitment can replenish | Calibrate combat lethality + bankruptcy-to-piracy rate together in step 9; cap pirate population at a fixed fraction. |
| Combat tick rate makes engagements feel too slow/fast | Use 15 sim-min combat ticks but allow batch resolution when no player is watching that engagement. |
| Encounter system O(N²) blows up with many ships | Spatial hash with 20 NM cells. With ~100 ships in a 1000×1000 NM map, ~25 ships per cell typical — fine. |
| Trade-law Rhai integration is too heavy for v1 | Hardcode the Navigation Acts ruleset; reserve the Rhai hook as a `fn` boundary for later. |
| Bankruptcy-to-piracy makes broke ships immortal | Pirate ships can still be sunk in combat. The fleet self-balances. |
| Faction relations matrix gets out of date when politics changes | Acceptable for v1 — static seed data, hand-edited per scenario year. |

## Connection to navigation (deferred Phase 2 item)

A* navigation (per `planning/navigation-plan.md`) is still pending
from Phase 2. It's worth doing **before** Phase 3 starts in earnest
because:

- Combat happens on the open sea; ships fleeing toward a friendly
  port need to actually take a sensible route around islands, not
  pin into the coast.
- Pirates patrolling a lane need to follow the lane, not
  straight-line through Cuba.
- Convoy formation requires shared waypoints, not parallel straight
  lines.

Recommendation: land A* navigation as the **0th** step of Phase 3
prep, or treat it as the last item of Phase 2 polish.
