# Plan: Period-Correct Navigation (storms + replanning + DR error)

## Core thesis

A ship has two positions: `true_position` (physics — what we have today)
and `estimated_position` (what the captain *thinks* his position is).
Plans are computed against the estimate, not truth. The estimate drifts
from truth via dead-reckoning error each tick; **periodic fixes** snap
it back. Storms multiply the error rate. The system's behavioural
richness comes for free from the gap between estimate and truth — a
ship that thinks it's 100 NM west of its true position will literally
sail in the wrong direction until a noon sight or a landmark sighting
corrects it.

This naturally unifies storms, replanning, and historical realism into
one system. None of the parts are individually hard; they're hard
*to design well together*, which is why we do them together.

## Mental model: the bug is the feature

The hardest thing in modern navigation is not getting lost. The hardest
thing in 1680 navigation was knowing where you were, full stop. Modern
implementations of "wind blew me off course" feel artificial because
the AI always knows the truth. Here, the AI has only the estimate, and
the loop is:

```
plan against estimate  →  follow heading toward (estimated)
   waypoint  →  physics moves true_position  →  estimate drifts via
   DR error  →  occasional fix collapses some of the gap  →  if the
   gap was big, AI replans
```

That's the entire system in one paragraph. Everything below is detail.

## Scope (what's IN this feature)

1. `estimated_position` per ship; per-tick DR error walk; deterministic
   per-ship RNG seed.
2. **Noon sight** fix (resets latitude only) once per simulated day.
3. **Landmark fix** (resets both axes) when true_position is within
   sighting range of any port-in-the-port-list and the line of sight
   to that port's coastline is unobstructed.
4. **Latitude sailing routing**: a planner pre-pass that, for long
   voyages, decomposes the destination into "drop to destination
   latitude, then run along it". Navmesh handles each leg.
5. Storms (`WeatherEvent`s): spawn-drift-decay, with seeded RNG.
   Localized wind perturbation. Inside a storm, DR error rate is
   multiplied.
6. **Replan triggers**: stale plan + off-track, DR error spike, storm
   intersects route, post-fix correction big enough to invalidate
   current waypoint chain.
7. **Heave-to** action in the BT for ships caught deep in a storm.
8. Visualization: storm overlays, estimated-vs-true position diagnostic
   on selected ship, replan-counter HUD.

## Scope (what's explicitly OUT, and that's fine for "sound")

- Realistic storm physics (stylized radial gust + tangential rotation).
- Sounding-line fixes (depth-bottom-type-based position inference).
- Per-historical-route preferences (Spanish treasure routes etc.).
- Multi-day storms with eyewall/path complexity.
- Visual pilotage along familiar coasts (we'll lump this into
  "landmark fix" since the simulation is at strategic scale).
- Charts having their own errors (the navmesh is treated as ground
  truth for routing purposes).
- Damage/sinking from storms.

## Architecture

### New types

- `weather/events.rs`:
  - `pub struct WeatherEvent { kind, center, radius_nm, intensity, age_hours, lifetime_hours, drift }`
  - `enum EventKind { TropicalStorm, Squall, Calm }`
  - `pub fn step(events: &mut Vec<WeatherEvent>, date: SimDate, rng: &mut SmallRng)`
  - `pub fn perturb(base: WindVector, pos: Position, events: &[WeatherEvent]) -> WindVector`
  - `pub fn dr_error_multiplier(pos: Position, events: &[WeatherEvent]) -> f32`

- `nav/dr.rs`:
  - `pub struct DrModel { pub seed: u64, pub rng_state: u64 }`
  - `pub fn tick_error(estimate: &mut Position, true_pos: Position, heading_deg: f32, speed_kt: f32, multiplier: f32, rng: &mut SmallRng)`
  - Lat error rate ~0.05 NM/h cruising, ~0.15 NM/h in storm.
  - Lon error rate ~0.15 NM/h cruising, ~0.45 NM/h in storm.
  - Both implemented as Gaussian per-hour noise with no decay (longitude
    accumulates; latitude is reset by noon sights).

- `nav/fixes.rs`:
  - `pub fn try_noon_sight(estimate: &mut Position, true_pos: Position, date: SimDate, last_sight_day: &mut u16, weather_clear: bool) -> bool`
    Resets `estimate.y = true_pos.y + N(0, 0.5)` once per day if not
    cloudy. Returns whether a fix occurred.
  - `pub fn try_landmark_fix(estimate: &mut Position, true_pos: Position, ports: &[Port], land: &LandMap, max_range_nm: f32) -> Option<usize>`
    If any port within `max_range_nm` of true_pos AND line-of-sight to
    its coast cell, snap estimate to true_pos + N(0, 1.0); return port
    index for diagnostic.

### Modified types

- `NavState`:
  - `pub estimated_position: Position`
  - `pub dr: DrModel`
  - `pub last_noon_sight_day: u16`
  - `pub last_fix_age_hours: u32`
  - `pub plan_age_hours: u32`
  - `pub fn invalidate_plan(&mut self)`

- `WeatherSystem`:
  - `pub events: Vec<WeatherEvent>`
  - `pub event_rng: SmallRng`
  - `pub fn wind_at(&self, pos, month) -> WindVector` overlays events
    on top of climatology.

- `World`:
  - `tick()`: in order — step weather events, then for each ship:
    apply DR error walk, attempt noon sight + landmark fix, run AI
    (which may invalidate the plan and replan), then run physics
    against true_position.

- `PathfindContext`:
  - Add `events: &'a [WeatherEvent]` (optional). Edge cost in the
    navmesh route is multiplied by `1 + sum(intensity * (1 - dist/r))`
    over overlapping events; routing detours around active storms
    naturally.

- `pathfind::find_path_to_harbor`:
  - New flag: `prefer_latitude_sailing: bool`. When true and the
    great-circle (straight-line) distance is greater than
    `LAT_SAIL_MIN_NM` (e.g., 600 NM), insert an intermediate waypoint
    at the destination's latitude *roughly under the start's
    longitude*, then route legs A→intermediate and intermediate→dest.
    The navmesh handles land avoidance on each leg.
  - The "roughly under" is critical: we don't want to drop straight
    down through hostile waters. Pick the longitude that's closer to
    the destination by `LAT_SAIL_DROP_FRACTION` (e.g., 0.6) of the
    east-west span, biased toward the wind's downwind direction.

- `ai.rs` (`ShipAI::tick`):
  - Pre-BT: increment plan_age, run `check_replan_triggers`. Triggers:
    1. `nav.path` is empty AND a destination is set.
    2. `plan_age_hours > MAX_PLAN_AGE_HOURS` (default 72).
    3. Distance from `estimated_position` to next waypoint exceeds
       `OFF_TRACK_NM` (default 30).
    4. A storm overlaps the next 200 NM of the planned path.
    5. A landmark fix this tick moved the estimate by more than
       `FIX_THRESHOLD_NM` (default 30).
  - Replanning calls `find_path_to_harbor` with
    `prefer_latitude_sailing = great_circle_dist > LAT_SAIL_MIN_NM`.
  - If currently inside a TropicalStorm with intensity > 0.6, the BT
    selects `RIDE_STORM`: `compute_heading` returns wind-driven
    direction; ship just drifts and waits.

### Sim-viz

- `draw_storms`: translucent red gradient circles for storms; gray for
  calms.
- For the **selected/demo** ship: draw a small marker at
  `estimated_position` connected to `true_position` by a thin line.
  Show DR error magnitude in the HUD.
- HUD: append "weather: N storms active | replans: M".

### Determinism

Everything seeded:
- `WeatherSystem::event_rng` from a `weather_seed` config.
- Per-ship `dr.rng` from `(world_seed XOR ship_id)`.
- Tests can pin any of these.

## File-level diff outline

```
crates/sim-core/src/
├── nav.rs                  (existing) — extend NavState, add helpers
├── nav/
│   ├── dr.rs               NEW — DR error model
│   └── fixes.rs            NEW — noon sight + landmark fix
├── weather/
│   ├── mod.rs              add events vec + rng + wind_at overlay
│   └── events.rs           NEW — WeatherEvent + step + perturb
├── pathfind.rs             event-aware edge cost; latitude-sail flag
├── ai.rs                   pre-BT replan check; RIDE_STORM action
├── world.rs                tick order: events → DR walk → fixes → AI
└── lib.rs                  expose new modules
```

```
crates/sim-viz/src/main.rs  draw_storms + estimate marker + HUD lines
```

## Implementation order (each step ships independently green)

1. **Estimate plumbing**: add `estimated_position` to NavState,
   initialize to `true_position`, expose to AI. AI uses estimate for
   heading-target and arrival checks. NO error model yet — at this
   stage estimate==truth always, so behavior is unchanged. Verify
   bench still 992/992.

2. **DR error model** (no fixes yet): add per-tick noise to estimate.
   Bench will start failing routes because ships drift off and the
   harbor-arrival check (estimate-distance-to-anchor) succeeds while
   true position is hundreds of NM off. *This is exposed-but-broken
   on purpose for the next step.*

3. **Noon sight + landmark fix**: latitude is now self-correcting
   daily; longitude reset on landfall. With these in, integration
   tests show: ships from Barbados to Jamaica (a short, landmark-rich
   trip) arrive normally. Ships to remote destinations like Bermuda
   may wander; that motivates step 4.

4. **Latitude sailing routing**: the planner's longitude-uncertainty
   compensation. Long routes now decompose into "go to parallel,
   then run along it". Bermuda from Caribbean → land at the right
   latitude band, run east → Bermuda fix. Add test for a long
   route's path having a clear two-leg structure.

5. **Storms (no replanning yet)**: spawn-drift-decay machinery.
   Wind overlay. DR error multiplier. Visual rendering. Bench
   re-run: routes still complete because storms don't yet block
   plans, just slow ships.

6. **Replan triggers** including storm-on-route detection. Now
   storms cause real route changes. RIDE_STORM heave-to action.

7. **Polish**: HUD, replan counter, deterministic seeded scenarios.

## Sanity checks / "sound" criteria

The feature is "sound" if all of these hold in a 60-day demo run with
3 ships and a fixed seed:

- Every ship completes its assigned voyages (no permanently stuck).
- Average estimate-vs-truth error stays below ~150 NM under normal
  conditions; spikes to ~300 NM during/after storms; collapses on
  landmark fixes.
- Replan counter is non-zero (replans actually happen) but bounded
  (no replan thrashing — should be < 5 replans per voyage).
- A ship deliberately spawned upwind of a known landmass with a
  destination on the other side correctly latitude-sails around.
- Two demo runs with the same `weather_seed` produce identical
  storm spawns; different seeds produce different ones.

## Open questions

These should be answered (with your input) before step 4, not now:

1. **Latitude sailing decision**: should we always do it for routes
   over `LAT_SAIL_MIN_NM`, or have the AI decide based on its DR
   error magnitude? *Default: always, for sailing-tradition realism.*
2. **Cloudy days**: do we model "no noon sight today" as a random
   ~20% per-day chance, or tie it to local wind/storm conditions?
   *Default: 20% flat for v1, refine later.*
3. **Fix accuracy**: is N(0, 0.5 NM) latitude noise / N(0, 1.0 NM)
   landmark noise good, or do you want it tighter/looser? *Default
   numbers come from the research doc; I think they're right.*
4. **Storm seasonality**: hard-code "tropical storms only spawn
   June–November"? *Default: yes; squalls year-round.*
5. **Ship-AI awareness of its own error**: does the AI *know* its
   DR estimate is uncertain (and pad routes accordingly), or does
   it treat the estimate as truth? *Default: treats as truth — that's
   period-realistic; the gap is what creates the interesting
   behaviours.*

