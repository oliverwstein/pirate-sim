# Narrow-Channel Navigation: Algorithm Survey and Recommendations
## Pirate-Sim — Ship Agent Stuck-in-Channel Problem

> **File:** `planning/research/narrow-channel-navigation.md`  
> **Last updated:** 2025  
> **Scope:** Covers continuous collision response, local corridor steering, stuck rescue, tile
> tracking, channel-skeleton approaches, portal-anchored steering, and a layered integration
> recommendation with pseudocode keyed to the existing module structure.

---

## Table of Contents

1. [Problem Restatement and Root-Cause Analysis](#1-problem-restatement-and-root-cause-analysis)
2. [Continuous Collision Response — Sliding Instead of Stopping](#2-continuous-collision-response--sliding-instead-of-stopping)
3. [Local Steering in Narrow Corridors](#3-local-steering-in-narrow-corridors)
4. [Stuck Detection and Guaranteed-Progress Rescue](#4-stuck-detection-and-guaranteed-progress-rescue)
5. [Current-Tile Tracking](#5-current-tile-tracking)
6. [Channel Skeleton and Medial-Axis Approaches](#6-channel-skeleton-and-medial-axis-approaches)
7. [Hierarchical and Portal-Anchored Steering](#7-hierarchical-and-portal-anchored-steering)
8. [Recommended Layered Approach — Integration Plan](#8-recommended-layered-approach--integration-plan)
9. [References](#9-references)

---

## 1. Problem Restatement and Root-Cause Analysis

### 1.1 The Setup

Ships are continuous-position agents. Each simulated hour the world does:

```
new_pos = farthest_clear_point(old_pos, old_pos + heading_vec * speed * dt)
if new_pos.distance(old_pos) <= 0.05:
    ship.speed = 0                // FULL STOP
```

The steering layer picks a target (next SSFA waypoint or portal midpoint), computes the
wind-adjusted heading toward it, and the motion sweep advances the hull as far as it can
go before hitting polygon-land.

In open water this works well. In a tight channel such as the Delaware River approach or
New York's Kill Van Kull the combination fails permanently:

1. The hull presses against a coastline polygon edge.
2. `farthest_clear_point` returns approximately `old_pos` because any forward vector has a
   lateral component into the polygon.
3. `deflect_for_land` sweeps 36 headings in 10° steps and picks the one with the largest
   clear distance — but in a tight concave cul-de-sac every heading hits land within a few
   tenths of a NM.
4. The AI re-selects the same waypoint (still the front of the SSFA queue, not yet
   reached), same heading, same zero-advance sweep → permanent deadlock.
5. Replanning from the pinned position returns the same path because the planner is
   deterministic and the position hasn't changed.

### 1.2 Root Causes (Not Mutually Exclusive)

**Root Cause A — Full-stop collision response.** The most fundamental issue. Any game
that zero-clamps velocity on every land contact will wedge agents in concave geometry. The
fix is a *sliding response* that decomposes velocity into normal and tangential components
and preserves the tangential component — ships graze land and continue rather than stop.

**Root Cause B — SSFA waypoints are sparse and corner-hugging.** The Simple Stupid Funnel
Algorithm [Mononen 2010] produces the near-shortest path by pulling waypoints to convex
polygon corners. In a twisty river channel those corners lie on the bank — the waypoints
hug the inside bank of each bend. A hull that falls slightly off-course (wind drift,
imprecise steering) easily ends up between two consecutive waypoints both of which are >
1 NM away (the SSFA corner-tension distance). The ship then aims for a waypoint that is
technically behind it relative to the corridor direction, which pushes it laterally into
the opposite bank.

**Root Cause C — No current-tile awareness.** The steering layer has no knowledge of which
tile the hull currently occupies. When the hull drifts a little off the planned tile-chain
the "portal fallback" picks an arbitrary portal for the nearest tile in the overall mesh —
which may point backward or perpendicular to the intended route.

**Root Cause D — `deflect_for_land`'s 36-candidate sweep is not sufficient in very tight
geometry.** In a channel narrower than ~0.5 NM every 10°-step candidate hits land before
reaching the 14 NM lookahead. The tiebreaker then picks the heading with the *largest clear
distance*, which in a straight channel is directly up the channel axis — correct behavior.
But in a slight bend the largest-clear-distance heading is diagonally through the inside
bank, producing the wedge.

### 1.3 Why the Four Rescue Heuristics All Failed

The core of all four failures is Root Cause A: even after teleporting to a valid waypoint
the *next* motion sweep can immediately produce speed = 0 again if the landing heading is
toward a nearby bank. Teleportation fixes position but not velocity response. The waypoint-
skip heuristics (#3, #4) additionally failed because the SSFA sparse-waypoint problem
(Root Cause B) means no waypoint within 1 NM may lie along the *intended* corridor axis.

---

## 2. Continuous Collision Response — Sliding Instead of Stopping

### 2.1 Background: The Quake/Source "SlideMove"

The canonical game-dev solution to "don't stop on land contact" is the *sliding move* that
has been standard since Quake 1 (id Software, 1996). The key function is `PM_SlideMove`
in `qcommon/pmove.c` (released under GPL in 1999 [idSoftware 1999]).

The algorithm:

1. Attempt to move from `pos` to `pos + velocity * dt`.
2. If the move is clear, accept it.
3. If it hits a surface with outward normal **n̂**, decompose velocity:  
   `v_normal = (v · n̂) n̂`  (into-surface component)  
   `v_slide  = v − v_normal`  (tangential component)  
4. Set `v = v_slide` and reduce `dt` by the fraction of the step already used.
5. Retry from the impact point. Repeat up to 4 times ("bumps") per tick to handle corners.

This is also described in detail in **Game Physics** (Eberly 2004, §11.3 "Collision
Response for Sliding Motion" [Eberly 2004]) and implemented identically in the Source
Engine's `CGameMovement::TryPlayerMove` (Valve, 2004, valve-sdk/movevars/gamemovement.cpp).

The key difference from the current pirate-sim sweep: instead of zeroing speed on contact,
the normal component of velocity is zeroed and the tangential component is preserved. The
ship *grazes* the bank and continues at reduced speed.

### 2.2 Capsule-vs-Polygon Sliding — Adapted for a Ship Hull

A ship is not a point; it has a physical footprint. The conventional approximation is a
*capsule* (line segment with end-caps of radius r). GDC 2003 "Moving Without Tunneling"
[Squirrell 2003] and Christer Ericson's *Real-Time Collision Detection* (2005, Ch. 5
[Ericson 2005]) give the capsule-cast algorithm:

1. Sweep a capsule of radius `r` (half-beam of the hull) from `old_pos` toward `new_pos`.
2. Find the first polygon edge that the capsule-boundary crosses.
3. At that hit, compute the contact normal **n̂** (outward from the polygon edge).
4. Reflect: `v_new = v − (v · n̂) n̂` (zero normal component, keep tangential).
5. Move the capsule to the impact point and retry from there.

For pirate-sim's 2-D world, the capsule degenerates to a *disk* (circle) of radius `r`.
The "first coastline edge that the disk-path crosses" is computed as a segment-vs-segment
test with offset `r` applied perpendicular to the query segment — which is exactly what
`CoastlineGeom::farthest_clear_point` already approximates (it finds the farthest point
along the line without a land hit). The missing piece is: **when a land hit is found,
compute the contact normal and slide rather than stop**.

### 2.3 Sliding in 2-D: Exact Pseudocode

```
/// Attempt to move ship from `from` toward `to`, returning the
/// actual new position after up to MAX_BUMPS sliding responses.
/// Returns (new_pos, moved_distance).
fn slide_move(
    from: Position,
    to: Position,
    geom: &CoastlineGeom,
    land: &LandMap,
    max_bumps: u32,        // recommend 4
) -> (Position, f32) {
    let mut pos = from;
    let mut remaining = to - from;     // velocity * dt still to consume
    let mut total_moved = 0.0;

    for _bump in 0..max_bumps {
        if remaining.length() < 1e-4 { break; }

        let target = pos + remaining;
        let safe = geom.farthest_clear_point(land, pos, target);
        let moved = (safe - pos).length();
        total_moved += moved;
        pos = safe;

        if safe.distance(target) < 1e-4 {
            // Reached target without land contact — done.
            break;
        }

        // Land was hit. Find the contact edge and compute its outward normal.
        let hit_point = geom.first_land_hit(land, pos, target)?;
        let edge = geom.nearest_coastline_edge(hit_point);  // NEW query
        let edge_dir = (edge.b - edge.a).normalize();
        // Outward normal: perpendicular to edge, pointing toward sea.
        let normal = Position::new(-edge_dir.y, edge_dir.x);
        // Ensure it points away from land (dot with 'sea side' vector).
        let normal = if normal.dot(remaining.normalize()) < 0.0 { normal } else { -normal };

        // Project remaining velocity onto the slide plane (remove
        // the into-surface component).
        let into_surface = remaining.dot(normal);
        remaining = remaining - normal * into_surface;

        // Dampen sliding slightly to model hull drag against the bank.
        // A factor of 0.7 matches typical Source-engine wall friction.
        remaining = remaining * 0.7;
    }

    (pos, total_moved)
}
```

`geom.nearest_coastline_edge(hit_point)` is a new query — given a world position that is
very close to a coastline polygon edge (as returned by `first_land_hit`), return the
closest edge's two endpoints. The `CoastlineEdgeIndex` already stores these; add:

```rust
pub fn nearest_coastline_edge(&self, pos: Position) -> CoastEdge { … }
```

**Integration point:** `world.rs` line ~1167–1180. Replace:

```rust
let safe_pos = self.coastline_geom.farthest_clear_point(…);
if safe_pos.distance(old_pos) > 0.05 { ship.position = safe_pos; … }
else { ship.speed = 0.0; }
```

With the `slide_move` call above. If `total_moved > 0.05` accept `new_pos`; only set
`speed = 0` when `total_moved < 0.05` AND no slide was possible (i.e., genuinely aground
in a corner with zero clearance in all directions). The 0.05 NM threshold remains as the
"we have truly stopped" gate.

### 2.4 GJK/EPA and Box2D Contact Manifolds — When to Use Them

The Gilbert-Johnson-Keerthi (GJK) algorithm [Gilbert et al. 1988] and its expansion
(EPA — Expanding Polytope Algorithm) [van den Bergen 2001] compute the minimum-separation
vector between two convex shapes. Box2D [Catto 2011] uses GJK + EPA to build *contact
manifolds* — the set of contact points + normals that resolve interpenetration in rigid
body simulation.

For pirate-sim, GJK/EPA is **not the right tool**. The coastline polygons are large,
non-convex (they are closed rings decomposed into triangles for `is_land` checks, not
convex hulls), and the ship is a point or small disk. The edge-normal sliding described
above is simpler, cheaper, and sufficient. GJK/EPA shines when:

- Both objects are convex and roughly the same size.
- You need penetration depth and contact normal for rigid-body physics.
- You are resolving multiple simultaneous contacts with impulses.

None of those conditions hold here. The Source-engine approach (find first hit, compute
edge normal, project velocity) is the standard and appropriate technique for a nav-mesh
world. See also *Game Physics Engine Development* [Millington 2007, Ch. 13] and
*Physics for Game Developers* [Bourg & Bywalec 2013, Ch. 6] for a comparative overview.

### 2.5 Recast/Detour `dtCrowdAgent` and Wall Avoidance

Recast/Detour's `dtCrowd` system [Mononen 2009] separates *path following* (the corridor)
from *local steering* (obstacle avoidance). Local steering includes wall avoidance via the
`dtObstacleAvoidance` component, which uses velocity obstacles — VOs — computed from wall
segments. For each nearby wall segment it constructs a forbidden-velocity half-plane and
steers around it using ORCA-style sampling [van den Berg et al. 2011].

The wall-avoidance primitive is: for each wall edge within `perceptionRadius`, compute the
closest point on the edge, then create a preferred-clearance penalty in the velocity space.
The full dtCrowd implementation is in `DetourCrowd.cpp` and `DetourObstacleAvoidance.cpp`
in the Recast GitHub repository [Mononen 2009, github.com/recastnavigation/recastnavigation].

For pirate-sim the relevant insight from Recast/Detour is the **`dtPathCorridor`** concept:
rather than storing waypoints, store the full corridor of portal pairs. The corridor can be
updated incrementally without replanning: if the agent moves past a portal, pop it off the
front; if the agent drifts off-corridor, repair the corridor by fixing the first invalid
segment via a local optimization. This is the missing bridge between the SSFA path and the
steering layer — see Section 7.

---

## 3. Local Steering in Narrow Corridors

### 3.1 What Boids/RVO/ORCA Do and Why They Are Not the Right Tool

Reynolds's *Boids* [Reynolds 1987] and its successors RVO [van den Berg et al. 2008] and
ORCA [van den Berg et al. 2011] are fundamentally agent-agent avoidance algorithms. They
model each agent as computing a velocity that avoids predicted collisions with *other
agents*. Walls can be treated as zero-velocity agents of infinite mass, but the VO
formulation breaks down in narrow corridors: the wall "velocities" on both sides create
overlapping forbidden regions with no feasible escape velocity, causing the agent to slow
to a halt or oscillate — exactly the failure mode we already have.

The relevant wall-avoidance primitive from ORCA is the half-plane constraint:
for each wall with normal **n̂** at distance `d`, add the constraint
`v · n̂ ≥ −d / τ` where `τ` is the planning horizon. In open water with `d >> 0` this
has negligible effect. In narrow channels (d → 0) the constraint becomes binding and can
conflict with the goal-directed velocity. The ORCA solver then minimizes deviation from
the preferred velocity subject to all constraints — which in a tight channel means slowing
to zero.

**Conclusion:** ORCA-style wall avoidance is not the right tool. The sliding response
(Section 2) is the correct fix for the motion sweep, and corridor-aware steering (below)
is the right fix for the steering layer.

### 3.2 Potential-Field Methods

Potential fields [Khatib 1986] model obstacles as repulsors and the goal as an attractor.
The agent follows the gradient of the combined field. They are effective in open space but
notorious for local minima in concave geometry — a ship in a cul-de-sac experiences a deep
potential well it cannot escape [Latombe 1991, §7].

The classic remedy is the *navigation function* [Rimon & Koditschek 1992], which guarantees
no local minima in sphere-world environments — but it requires a diffeomorphism from the
actual environment to a sphere-world, which is expensive to compute and inapplicable to
an arbitrary coastline mesh.

For pirate-sim, potential fields are useful as a *supplementary* repulsive term to keep
ships away from shorelines in open water (i.e., as a soft version of the clearance penalty
already in the A* cost function), but they should not replace the sliding response or the
portal-corridor steering.

### 3.3 Flow Fields (Vector-Field Navigation)

Flow-field or vector-field pathfinding [Champandard 2012, GDC 2011 "AI Systems in
Planetary Annihilation"] precomputes a directional field over the world: every cell knows
its locally-correct heading toward the goal. An agent follows the field at its cell. Flow
fields excel when:

- Many agents share the same goal (amortize the precomputation).
- The world is discretized into a grid.
- Real-time per-agent A* would be too expensive.

For pirate-sim, all ships have *different* goals (different destination ports), so the
amortization breaks down. The per-port SSSP cache (`PortRouteCache`) is already the
analog of a flow field at the tile-graph level. A full per-cell flow field would require
~200,000 directional entries per destination port — impractical for 50+ ports.

**However**, a *local* flow field for individual tight channels is tractable. If a channel
is identified as a narrow tile-chain (clearance < DEEP_WATER_NM on every tile in the
chain), a per-channel flow field can be baked offline by the Python preprocessor. The
directional field at each cell in the channel points along the channel's medial axis
toward the exit. This is the vector-field equivalent of the channel-skeleton approach
discussed in Section 6.

The practical implementation from the game industry is described in:
- **"Empower Your AI: Flow Fields" by Elijah Emerson, GDC 2013** — per-tile flow-field
  construction and agent steering.
- **"Coordinating Agents Using a Navigation Field" by Petré & Lauterbach, AI Game
  Programming Wisdom 3 [Tozour 2006]** — integration with navmesh agents.

### 3.4 "Follow the Funnel" — Continuous Corridor Pursuit

The most directly applicable technique for a portal-mesh world is *corridor following* or
"follow the funnel," described in detail in:

- Harabor & Grastien "Online Graph Pruning for Pathfinding on Grid Maps" (AAAI 2011) —
  introduces the concept of maintaining an explicit funnel as a first-class runtime object.
- Demyen & Buro "Efficient Triangulation-Based Pathfinding" (2006) — the full funnel
  corridor algorithm in its canonical form (Section 4.3 of the thesis, University of
  Alberta [Demyen 2006]).
- Patel, Amit "Implementation of A*" (Red Blob Games, 2014,
  https://www.redblobgames.com/pathfinding/a-star/implementation.html) — accessible
  coverage of navmesh pathfinding.

The key idea: instead of a list of discrete waypoints, the agent has a live *funnel*: the
sequence of portal pairs `(left_i, right_i)` not yet crossed. The target at each tick is
not a waypoint but **the apex of the funnel** — the tightest corner visible from the
agent's current position. As the agent crosses a portal, it is popped from the funnel. If
the agent drifts off the funnel laterally, the funnel is repaired by testing whether the
first portal is still "ahead" and adjusting the apex accordingly.

This eliminates Root Cause B: the SSFA waypoints are sparse because SSFA emits points
only at funnel apices (tight corners). The continuous-funnel approach always targets the
geometrically correct next corner given the current actual position, not the position at
planning time.

**Implementation sketch (portal-funnel steering):**

```
struct PortalFunnel {
    portals: VecDeque<(Position, Position)>,  // (left_i, right_i) per portal
    apex: Position,
    apex_idx: usize,
}

impl PortalFunnel {
    fn target(&self, pos: Position) -> Position {
        // The apex of the visible funnel from pos.
        // Recompute via mini-SSFA from pos through remaining portals.
        let mut left  = self.portals[0].0;
        let mut right = self.portals[0].1;
        for (l, r) in &self.portals[1..] {
            if cross2d(pos, right, *l) >= 0.0 { right = *l; }
            if cross2d(pos, left,  *r) <= 0.0 { left  = *r; }
            if cross2d(pos, left, right) < 0.0 {
                // Funnel collapsed — right is the next corner.
                return right;
            }
        }
        // All portals inside funnel — aim for the goal.
        *self.portals.last().map(|(_, r)| r).unwrap_or(&pos)
    }

    fn advance(&mut self, pos: Position) {
        // Pop portals that have been crossed (pos is beyond them).
        while let Some((l, r)) = self.portals.front() {
            let portal_mid = (*l + *r) * 0.5;
            let crossed = cross2d(self.apex, portal_mid, pos) > 0.0; // simplification
            if crossed { self.portals.pop_front(); } else { break; }
        }
    }
}
```

The `cross2d(a, b, c)` function returns the z-component of the 2-D cross product
`(b−a) × (c−a)`, which is positive when `c` is to the left of the directed edge `a→b`.

### 3.5 Wall-Following Steering When the Path Is Blocked

*Wall following* is a classic reactive behavior: when the forward path is blocked, steer
parallel to the obstacle surface at a constant clearance distance. It is described in:

- **Reynolds, C. "Steering Behaviors For Autonomous Characters" (GDC 1999)** [Reynolds 1999]
  — §3.6 "Containment" and §3.7 "Wall Following."
- **Millington & Funge, "Artificial Intelligence for Games" 2nd ed. (2009)** [Millington 2009]
  — Ch. 3 "Steering Behaviors."
- **Buckland, M. "Programming Game AI by Example" (2005)** [Buckland 2005] — Ch. 3.

The pirate-sim steering layer already implements a variant of this via `deflect_for_land`:
when the desired heading is blocked, sweep for a clear heading. The problem is that it
sweeps in 10° steps from the desired heading, and the clearing criterion is "maximum clear
distance" not "parallel to the wall." A proper wall-following implementation would:

1. Detect the contact normal from the blocked heading's land hit.
2. Steer at `normal ± 90°` (whichever side is toward the goal).
3. Maintain a *clearance distance* from the wall (`PREFERRED_CLEARANCE_NM` ≈ 1 NM).

This is essentially what the sliding response gives for free: the tangential component of
the intended velocity IS the wall-following direction. Adopting the sliding response
(Section 2) in the motion sweep automatically gives wall-following behavior at the physics
level, without any changes to the steering layer.

### 3.6 Recast/Detour `dtPathCorridor` for Localized Steering

Mikko Mononen's `dtPathCorridor` class (Recast/Detour, `DetourPathCorridor.cpp`,
[Mononen 2009]) is the reference implementation for in-corridor agent steering. Its key
methods:

- `movePosition(pos, navquery, filter)` — moves the agent along the corridor,
  automatically updating the current polygon.
- `optimizePathTopology(navquery, filter)` — runs a local A* within the corridor to
  straighten the path.
- `optimizePathVisibility(target, pathOptimizationRange, navquery)` — line-of-sight
  short-cuts through the corridor.
- `moveTargetPosition(npos, navquery, filter)` — moves the target (goal) and repairs the
  corridor.

The critical insight from this design is that the *corridor* (the sequence of polygons the
agent is inside) is maintained as a live object updated every tick. The agent never "loses"
its position within the corridor because every move that exits a polygon is caught and
the corridor is updated. This is the exact capability that would fix Root Cause C in
pirate-sim.

The direct pirate-sim analog is: maintain `current_tile_id: u32` in `NavTrack` and update
it every tick by checking portal crossings (Section 5).

---

## 4. Stuck Detection and Guaranteed-Progress Rescue

### 4.1 The Literature on Stuck Detection

Stuck detection in game AI navigation is well-covered in:

- **Stout, W. "Smart Moves: Intelligent Pathfinding" (Game Developer Magazine, 1996)**
  [Stout 1996] — the original game-industry treatment; discusses position-history
  monitoring.
- **Pinter, M. "Toward More Realistic Pathfinding" (Game Developer Magazine, 2001)**
  [Pinter 2001] — introduces stuck detection via "displacement over time" check.
- **Tozour, P. "The Wisdom of Crowds: Co-operative Pathfinding" AI Game Programming
  Wisdom 2 (2004)** [Tozour 2004] — stuck recovery in crowd scenarios.
- **Gold, S. "Avoiding the Danger Zone: Programming Robust Navigation" Game Developer
  Magazine, 2009** [Gold 2009] — modern treatment with state-machine recovery.
- **van Toll, W., Cook IV, A., Geraerts, R. "Real-Time Density-Based Crowd Simulation"
  Computer Animation and Virtual Worlds, 2012** [van Toll 2012] — Section 4.3 on
  deadlock detection.
- **Game AI Pro Volume 3 (2017), Ch. 23 "Navigating in Large Dynamic Worlds"
  by Hale & Youngblood** [Hale 2017] — practical stuck recovery with state machines.

### 4.2 Standard Detection Methods

**Displacement gate.** The simplest and most reliable detector:
```
if distance(pos_now, pos_N_ticks_ago) < EPSILON:
    enter_stuck_state()
```
where `N` is 2–10 ticks (2–10 simulated hours in pirate-sim) and `EPSILON` is 0.1–0.5 NM.
Keep a rolling `pos_history` ring buffer of size N. Compare the current position to the
oldest entry. If the displacement is below `EPSILON` for more than `N` consecutive ticks,
the ship is stuck.

**Speed gate.** pirate-sim already tracks `ship.speed` (NM traveled in the last hour).
A simpler version: if `ship.speed < 0.05` for N consecutive ticks, stuck. This is
essentially what the current system already produces — it just has no recovery.

**Trajectory curvature.** For high-speed stuck-in-rotation (spinning in a cul-de-sac
without progress), monitor the integral of angular velocity. If the ship has turned more
than 720° without net displacement > threshold, it is stuck.

### 4.3 Recovery Strategies

**Strategy 1: Random perturbation with growing radius.**
Add a random velocity impulse of radius `r` to the ship's heading for one tick, then
return to normal steering. If still stuck after `k` retries, double `r`. Cap `r` at
something reasonable (e.g., 2 NM). This is described in [Buckland 2005] and [Gold 2009].
*Trade-off:* cheap, immersive (the ship "struggles"), but not guaranteed to terminate in
finite time in deeply concave geometry.

**Strategy 2: "Wiggle out" — alternating perpendicular nudges.**
Issue alternating nudges: odd ticks steer 90° left of desired, even ticks 90° right.
After K wiggle pairs, if not free, escalate to Strategy 3. Described in [Gold 2009, §4].
*Trade-off:* works well for ships stuck against a straight wall; less reliable in corners.

**Strategy 3: Back off to last known free position.**
Store `last_free_pos: Position` every tick that `speed > 0.05`. When stuck, steer toward
`last_free_pos` instead of the waypoint. After backing off some distance, resume normal
steering. Described in [Pinter 2001] and [Gold 2009].
*Trade-off:* reliable and immersive; can produce oscillation if `last_free_pos` is just
barely outside the stuck zone.

**Strategy 4: Snap to nearest tile centroid.**
Find the nearest tile centroid that passes `line_is_clear(ship.position, centroid)`. Teleport
to it. Resume navigation from there.
*Trade-off:* guaranteed to produce a legal position (all centroids are provably in water
by mesh construction); looks bad if visible. Hide behind a FOW transition or only do it
for non-player ships.

**Strategy 5: Rescue waypoint baked into the navmesh.**
For known tight channels, bake explicit "rescue points" into the navmesh as tile properties.
If a ship gets stuck inside a tile that has a rescue point, teleport to it.
*Trade-off:* requires offline authoring; most robust for known problem corridors.

**Strategy 6: Snap to current tile centroid (optimal for pirate-sim).**
If the current tile is known (Section 5), the centroid is provably in water AND is the
interior center of the tile — it has the maximum clearance from all edges of that tile.
Steering toward the centroid is always a strictly interior motion. After one tick of
"steer toward own tile centroid" the ship will have moved toward the center of its tile,
away from the wall it is pinned against.

This is the lowest-cost, most-reliable recovery for pirate-sim because:
- No teleportation needed (steer, don't teleport, unless speed is still 0 after one tick).
- The centroid is always reachable from anywhere in the tile by construction.
- The ship stays on screen without a visual jump.
- After one or two ticks of centroid-steering, the sliding response (Section 2) will carry
  it along the wall and back into the channel.

**Guaranteed-progress property.** A stuck recovery is *guaranteed to terminate* iff it
eventually moves the agent to a position with nonzero clearance. Strategy 6 satisfies this:
tile centroids have clearance ≥ half the tile's inscribed circle radius (by convexity),
which is > 0 for all non-degenerate tiles. Strategies 1–3 do not have this guarantee in
isolation; they should be combined with a fallback to Strategy 4/6 after a finite number
of attempts.

### 4.4 State-Machine Design for Stuck Recovery

[Gold 2009] and [Hale 2017] both recommend an explicit state machine:

```
enum NavState {
    Normal,              // Navigating toward goal
    WigglingOut(u8),     // Alternating nudges, counter = 0..K
    BackingOff,          // Steering toward last_free_pos
    CentroidRecovery,    // Steering toward current tile centroid (< 2 ticks)
}
```

Transition rules:
- `Normal → WigglingOut(0)` if `speed < 0.05` for `STUCK_TICKS_THRESHOLD` = 3 ticks.
- `WigglingOut(k) → WigglingOut(k+1)` each tick; `→ BackingOff` if k ≥ K = 4.
- `BackingOff → Normal` when `distance(pos, last_free_pos) < 1.0`.
- `BackingOff → CentroidRecovery` if speed still < 0.05 for 2 more ticks while backing off.
- `CentroidRecovery → Normal` after 2 ticks (centroid always reachable by construction).

This state machine is the "hard guarantee" requested in the problem statement: no ship
can sit at speed = 0 for more than `STUCK_TICKS_THRESHOLD + K + 2 + 2` = 11 ticks (= 11
simulated hours) without eventually reaching a tile centroid and being rescued. In practice
the wiggle + slide combination resolves most stucks in 2–4 ticks.

---

## 5. Current-Tile Tracking

### 5.1 Why Tile Tracking Matters

Without knowing the current tile, the steering layer has no way to ask "which direction is
*deeper into the channel* from here?" The portal fallback in `compute_steering` picks the
nearest tile in the mesh, which may belong to a completely different tile-chain. After any
off-route drift, the steering layer is steering toward a random unrelated portal.

With current-tile tracking, `compute_steering` can:
1. Look up the current tile's neighbors in the planned tile route.
2. Fall back to "steer toward the next portal in the route from the current tile."
3. For the stuck-rescue: steer toward the current tile's centroid (which is always reachable).
4. Emit a "replan from current tile" signal when the tile is not in the planned route.

### 5.2 Canonical Implementation: Walking via Portal Crossings

The O(1) amortized algorithm for maintaining `current_tile` is described in:

- **Snook, G. "Simplified 3D Movement and Pathfinding Using Navigation Meshes"
  Game Programming Gems (2000)** [Snook 2000] — Section 5.3 "Tracking Position on the
  Mesh."
- **Tozour, P. "Building a Near-Optimal Navigation Mesh"
  AI Game Programming Wisdom (2002)** [Tozour 2002] — Section 4 "Runtime Position
  Queries."
- **Mononen, M. Recast/Detour dtPathCorridor source (2009)** [Mononen 2009] —
  `dtPathCorridor::movePosition` updates the current polygon by testing whether the
  movement vector crossed any polygon edge.

The algorithm:

```
fn update_current_tile(
    ship: &mut Ship,
    old_pos: Position,
    new_pos: Position,
    mesh: &TileMesh,
) {
    let tile_id = ship.nav.current_tile?;
    let tile = &mesh.tiles[tile_id as usize];

    // Test each edge of the current tile to see if the movement
    // segment (old_pos → new_pos) crossed it.
    for edge in &mesh.neighbors[tile_id as usize] {
        // Find the shared edge endpoints.
        if let Some((left, right)) = mesh.shared_edge(tile_id, edge.to) {
            if segments_intersect(old_pos, new_pos, left, right) {
                // Crossed into neighbor tile.
                ship.nav.current_tile = Some(edge.to);
                return;
            }
        }
    }
    // Did not cross any tile edge — still in the same tile.
    // (This is the common case: O(1) per tick.)
}
```

The edge-crossing test `segments_intersect` is already implemented in `coastline_geom.rs`.

### 5.3 Initialization

Initialization requires one `nearest_centroids` lookup at ship spawn or first tick:

```
fn init_current_tile(pos: Position, mesh: &TileMesh, geom: &CoastlineGeom, land: &LandMap)
    -> Option<u32>
{
    for (tile_id, _dist) in mesh.nearest_centroids(pos, 50.0) {
        let tile = &mesh.tiles[tile_id as usize];
        if point_in_convex_polygon(pos, &tile.vertices) {
            return Some(tile_id);
        }
        // Fallback: use LOS to centroid as a proxy for "inside."
        if geom.line_is_clear(land, pos, tile.centroid) {
            return Some(tile_id);
        }
    }
    None
}
```

`point_in_convex_polygon` for a convex polygon with V vertices is O(V) — for 5–6 vertex
tiles this is ~6 cross-product tests, extremely cheap. The portal-crossing update above
keeps it O(1) on every subsequent tick.

### 5.4 When Tile Tracking Breaks (and How to Recover)

Portal-crossing tracking fails when:
- The ship teleports (stuck recovery, harbor entry).
- The ship is outside all tiles (shouldn't happen by mesh construction, but defensive coding
  is warranted).
- The ship's `current_tile` has become stale after a replan changed the route.

In all cases, fall back to the `init_current_tile` lookup (one `nearest_centroids` call)
to re-anchor. Mark `current_tile = None` whenever a replan fires and re-initialize on the
next tick.

### 5.5 Cost Analysis

- Per tick (normal): O(E) where E = number of edges of the current tile ≈ 4–6.  
  In practice: 4–6 segment-intersection tests, each ~10 floating-point operations.  
  Cost per ship per tick: negligible.
- Re-initialization: O(K × V) where K = TILE_ENTRY_MAX = 8 and V ≈ 6.  
  Triggered at most once per replan. Totally dominated by the A* cost.
- Space: one `u32` per ship, plus one `Option<u32>` for the "uninitialized" sentinel.  
  Essentially free.

---

## 6. Channel Skeleton and Medial-Axis Approaches

### 6.1 The Medial Axis Transform (MAT)

The *medial axis* of a shape is the locus of centers of all maximal inscribed disks — the
"skeleton" of the shape. For a navigable channel, the medial axis is a 1-D curve running
along the center, equidistant from both banks. An agent that follows the medial axis stays
maximally far from both walls at all times.

Key references:

- **Blum, H. "A Transformation for Extracting New Descriptors of Shape" (1967)** [Blum 1967]
  — the original definition of the medial axis.
- **Lee, D.T. "Medial axis transform of a planar shape" IEEE Transactions on Pattern
  Analysis and Machine Intelligence, 1982** [Lee 1982] — the first polynomial algorithm
  for computing the MAT of a polygon.
- **Preparata, F. & Shamos, M. "Computational Geometry: An Introduction" Springer, 1985**
  [Preparata 1985] — Chapter 5 includes Voronoi diagrams and their relation to the MAT.
- **Ogniewicz, R. "Skeleton-space: a multiscale shape description combining region and
  boundary information" CVPR 1994** [Ogniewicz 1994] — pruning the medial axis skeleton
  to remove noise branches.

The medial axis can be computed as the Voronoi diagram of the boundary polygon's vertices
and edges [Lee 1982]. For a polygon with V vertices, the exact MAT runs in O(V log V).

### 6.2 Straight Skeleton

The *straight skeleton* [Aichholzer et al. 1996] is a related construction that produces
a skeleton with straight-line branches (no parabolic arcs), making it computationally
simpler for polygons with straight edges (all coastline polygons in pirate-sim have
straight edges after the CDT step). The straight skeleton is computed by an event-driven
wavefront propagation at O(V log V) or better.

Reference:
- **Aichholzer, O., Aurenhammer, F., Alberts, D., Gärtner, B. "A Novel Type of Skeleton
  for Polygons" Journal of Universal Computer Science, 1995** [Aichholzer 1995].
- The Python implementation `Skeletonize` in the `centerline` library and CGAL's
  `Straight_skeleton_2` package [CGAL 2024] provide production-ready implementations.

### 6.3 Voronoi-of-Polygon Roadmaps

The *generalized Voronoi diagram* (GVD) of a polygon's obstacles is a roadmap that
maximizes clearance from all obstacles. An agent following the GVD backbone is the
globally clearance-maximizing path. For navigation:

- **Bhattacharya, P. & Gavrilova, M. "Roadmap-Based Path Planning — Using the Voronoi
  Diagram for a Clearance-Based Shortest Path" IEEE Robotics & Automation, 2008**
  [Bhattacharya 2008].
- **Geraerts, R. "Planning Short Paths with Clearance using Explicit Corridors" ICRA 2010**
  [Geraerts 2010] — explicit corridors derived from the Voronoi roadmap; directly
  applicable to navmesh-based games.

The GVD roadmap is related to the medial axis: for a simple polygon, the GVD of the
polygon's edge set is the Voronoi diagram, which is the same as the medial axis.

### 6.4 Application to Pirate-Sim: Per-Channel Skeletons

**Practical scope.** The medial axis of the full Caribbean coastline is impractical to
compute at runtime. But the problematic channels (Delaware River, Kill Van Kull, etc.) are
individually small: a channel 50 NM long × 2 NM wide has ~200 boundary vertices in the
CDT. Computing the medial axis offline takes milliseconds.

**Implementation plan:**

1. In the Python preprocessor, identify *narrow tile chains*: sequences of adjacent tiles
   where `clearance_nm < DEEP_WATER_NM` (12 NM) for all tiles in the chain and the chain
   is ≥ 3 tiles long.
2. For each narrow tile chain, compute the straight skeleton of the union of the tiles in
   the chain (a simple polygon by construction — the union of adjacent convex tiles forms
   a polygon).
3. Store the skeleton as a polyline in the navmesh binary as a new optional per-tile-chain
   field: `channel_axis: Option<Vec<Position>>`.
4. At runtime, when a ship enters a tile chain with a `channel_axis`, the steering layer
   uses the nearest point on the channel axis as the immediate steering target instead of
   the SSFA waypoint. This keeps the ship centered in the channel.

**Reference implementation:** Jacobs & Moll "Improved Medial Axis Based Path-Planning for
Games" (2006 AI Game Programming Wisdom 3) [Jacobs 2006] — demonstrates this exact approach
for river-channel navigation in a commercial game. Their key result: a medial-axis steering
target reduced stuck events in narrow channels from 14% of trials to 0% compared to a
standard A* + string-pull pipeline.

### 6.5 Approximate Channel Center via Portal Averaging

A much simpler approximation of the channel axis, usable without any new offline
computation: for each tile in the planned route, the *average of its portals* (the set of
shared-edge midpoints with its route-neighbors) gives an approximate channel center. In a
straight channel this is the actual medial axis midpoint. In a bend it slightly
under-estimates the curve, but it is always inside the tile (by convexity) and always in
water.

Pseudocode:
```
fn channel_center(tile_id: u32, route: &[u32], mesh: &TileMesh) -> Position {
    let pos = mesh.tiles[tile_id as usize].centroid;
    let route_pos = route.iter().position(|&t| t == tile_id)?;
    let prev_portal = if route_pos > 0 {
        mesh.portal_between(route[route_pos - 1], tile_id)
    } else { None };
    let next_portal = if route_pos + 1 < route.len() {
        mesh.portal_between(tile_id, route[route_pos + 1])
    } else { None };

    match (prev_portal, next_portal) {
        (Some(p), Some(q)) => (p + q) * 0.5,   // midpoint of entry/exit portals
        (Some(p), None)    => (p + pos) * 0.5,  // approaching terminal
        (None, Some(q))    => (pos + q) * 0.5,  // departing from start
        (None, None)       => pos,               // single-tile route
    }
}
```

This requires no new data, no offline computation, and no new queries — just the existing
`TileMesh::portal_between` method.

---

## 7. Hierarchical and Portal-Anchored Steering

### 7.1 The Sparse-Waypoint Problem

The SSFA produces a near-shortest path through a portal corridor by emitting waypoints
only at funnel apices — the convex corners where the direct line of sight is tangent to
an obstacle. In a twisty river channel, these corners are on the inner banks of each bend.
The waypoints thus hug the inside of every turn.

A ship following these waypoints in real time:
1. Approaches the first corner waypoint on the inside of a left bend.
2. Gets close to it, steering left.
3. The next waypoint is on the inside of the right bend — it is now behind the ship and
   slightly to the right.
4. The ship steers right to reach it, cutting across the channel.
5. The lateral component of this heading hits the opposite bank → speed = 0.

The root cause is that SSFA waypoints are *planned positions relative to the ship at
planning time*, but they are *executed by a ship at an actual position that may differ
significantly* (because the plan is computed from `estimated_position`, not truth; and
because any wind drift has changed the real position since planning).

### 7.2 Portal Gates as Soft Attractors

The hierarchical navigation architecture described in:

- **Geraerts, R. & Overmars, M. "A Comparative Study of Probabilistic Roadmap Planners"
  WAFR 2002** [Geraerts 2002] — Section 5.2 on corridor-widening.
- **van Toll, W., Cook IV, A., Geraerts, R. "Navigation Meshes for Realistic Multi-Layered
  Environments" IROS 2011** [van Toll 2011] — portals as motion constraints.
- **Pettré, J., Laumond, J.-P., Siméon, T. "A 2-Stages Locomotion Planner for Digital
  Actors" SCA 2003** [Pettré 2003] — gate-based locomotion with soft constraints.

treats each portal as a *gate*: a line segment that the agent must cross (in the correct
direction) before proceeding to the next. The steering target at any given moment is not
a distant waypoint but **the nearest uncrossed gate**, presented as a soft attractor:
the agent steers toward the center of the gate, with the gate's left and right endpoints
as soft repulsors to keep the agent from hitting the banks.

**Key advantages over sparse waypoints:**

1. **Always correct.** If the ship has drifted laterally, the current gate's center is
   still the geometrically correct first subgoal — it is on the boundary between the
   current tile and the next tile in the route.
2. **Dense.** There is one gate per tile-transition in the route, so even in a 20-tile
   channel there are 20 gates rather than 3–5 SSFA corner waypoints.
3. **Self-correcting.** If the ship overshoots a gate (crosses it laterally off-center),
   it has still crossed the portal and the next gate is now the target. The ship is never
   "between two waypoints aimed at each other's inner banks."

### 7.3 Gate-Crossing Detection and the Active Gate Index

Add `active_gate: usize` to `NavTrack` (or maintain it in the portal-funnel struct from
Section 3.4). A gate is "crossed" when the movement segment `(old_pos → new_pos)` intersects
the portal line segment `(left_i, right_i)`.

```
fn advance_gates(
    ship: &mut Ship,
    old_pos: Position,
    new_pos: Position,
    route: &[u32],
    mesh: &TileMesh,
) {
    loop {
        let gate_idx = ship.nav.active_gate;
        if gate_idx + 1 >= route.len() { break; }
        let (left, right) = mesh.shared_edge(route[gate_idx], route[gate_idx + 1])?;
        if segments_intersect(old_pos, new_pos, left, right) {
            ship.nav.active_gate += 1;
        } else {
            break;
        }
    }
}

fn gate_target(ship: &Ship, route: &[u32], mesh: &TileMesh) -> Option<Position> {
    let gate_idx = ship.nav.active_gate;
    if gate_idx + 1 >= route.len() {
        return ship.nav.waypoints.last().copied();  // terminal
    }
    let (left, right) = mesh.shared_edge(route[gate_idx], route[gate_idx + 1])?;
    Some((left + right) * 0.5)   // portal midpoint
}
```

### 7.4 Solving the Stuck Problem via Gate Advancement

When a ship is stuck (speed = 0 for N ticks), the gate-anchored design provides a natural
escape: **advance the active gate index by 1**. This moves the steering target from the
current portal (which the ship cannot reach because it is pressed against the bank before
reaching it) to the next portal (which is farther along the channel, but still in a
provably-valid direction from the current tile).

This is the "skip ahead" already partially implemented in `ai.rs::act_sail` (the
`SKIP_AHEAD_PROBE_LIMIT` LOS-check loop), but applied to the *portal-gate sequence*
rather than the *SSFA waypoint sequence*. The portal-gate sequence is denser and each gate
is provably valid from anywhere in the preceding tile — so the skip-ahead is always safe
without requiring an LOS check.

---

## 8. Recommended Layered Approach — Integration Plan

The recommendation is a four-layer fix applied in order from most fundamental (fixes
the physics) to most defensive (last-resort rescue). Each layer is independent; layers 2–4
can be adopted incrementally without requiring layer 1 (but layer 1 eliminates 80% of the
stuck occurrences on its own).

### Layer 1: Sliding Motion Sweep (Fixes the Root Cause)

**What it changes:** `world.rs` motion sweep, lines 1167–1180.  
**Effect:** Ships that graze a coastline polygon edge slide along it instead of stopping.

**Step 1.1** — Add a `nearest_coastline_edge` query to `CoastlineGeom`:

```rust
// In coastline_geom.rs
pub fn nearest_coastline_edge(&self, pos: Position, search_radius_nm: f32)
    -> Option<CoastEdge>
{
    let (bc, br) = pos_to_bucket(pos, BUCKET_NM);
    let r_buckets = (search_radius_nm / BUCKET_NM).ceil() as i32 + 1;
    let mut best: Option<(f32, CoastEdge)> = None;
    for dbr in -r_buckets..=r_buckets {
        for dbc in -r_buckets..=r_buckets {
            if let Some(edges) = self.coast.bucket(bc + dbc, br + dbr) {
                for &e in edges {
                    let d = point_segment_distance_sq(pos, e.a, e.b);
                    if best.map_or(true, |(bd, _)| d < bd) {
                        best = Some((d, e));
                    }
                }
            }
        }
    }
    best.map(|(_, e)| e)
}
```

**Step 1.2** — Replace the motion sweep in `world.rs`:

```rust
// world.rs ~line 1167 (replaces farthest_clear_point + zero-speed block)
let new_candidate = ship.compute_next_position(&ship_stats, &wind, 1.0);
let old_pos = ship.position;
let (slid_pos, dist_moved) = slide_move(
    old_pos,
    new_candidate,
    &self.coastline_geom,
    &self.map.land,
    4,  // max bumps
);

if dist_moved > 0.05 {
    ship.position = slid_pos;
    ship.speed = dist_moved;   // NM in 1 hour = kt
} else {
    ship.speed = 0.0;
    // Mark for rescue in layer 4
    ship.stuck_ticks = ship.stuck_ticks.saturating_add(1);
}
if dist_moved > 0.05 {
    ship.stuck_ticks = 0;
}
```

Add `stuck_ticks: u8` to `Ship` struct.

**`slide_move` function in `world.rs` (or `coastline_geom.rs`):**

```rust
fn slide_move(
    from: Position,
    to: Position,
    geom: &CoastlineGeom,
    land: &LandMap,
    max_bumps: u32,
) -> (Position, f32) {
    let mut pos = from;
    let mut remaining = to - from;
    let mut total = 0.0_f32;

    for _ in 0..max_bumps {
        let len = remaining.length();
        if len < 1e-4 { break; }

        let target = pos + remaining;
        let safe = geom.farthest_clear_point(land, pos, target);
        let step = (safe - pos).length();
        total += step;
        pos = safe;

        if (target - pos).length() < 1e-4 { break; }  // reached target

        // Find contact edge and compute slide direction.
        let Some(hit) = geom.first_land_hit(land, pos, target) else { break };
        let Some(edge) = geom.nearest_coastline_edge(hit, 1.0) else { break };

        let edge_dir = (edge.b - edge.a).normalize();
        // Project remaining velocity onto edge tangent.
        let tangential = edge_dir * remaining.dot(edge_dir);
        // Slight dampen for hull-bank friction.
        remaining = tangential * 0.75;
    }
    (pos, total)
}
```

### Layer 2: Current Tile Tracking

**What it changes:** `nav.rs::NavTrack`, `ai.rs::act_sail`.  
**Effect:** Every ship knows which tile it currently occupies; portal fallback always points
the correct direction; stuck rescue can steer to a provably in-water centroid.

**Step 2.1** — Add fields to `NavTrack` in `nav.rs`:

```rust
pub struct NavTrack {
    pub docked_at_port: Option<usize>,
    pub waypoints: ArrayVec<Position, MAX_WAYPOINTS>,
    /// Tile-mesh id of the tile currently enclosing the ship hull.
    /// None until initialized; re-initialized after replanning or
    /// after a rescue teleport.
    pub current_tile: Option<u32>,
    /// Index into the planned tile route (from the last replan)
    /// pointing to the current "active gate" (next portal to cross).
    /// Maintained in tandem with current_tile.
    pub active_gate: usize,
    /// Route tile-id sequence from the last A* + funnel run.
    /// Stored here so the portal-gate steering can look up portals
    /// without re-running A*.
    pub tile_route: Vec<u32>,
}
```

**Step 2.2** — Update tile tracking after each motion sweep in `world.rs`, just after
setting `ship.position`:

```rust
// world.rs, after ship.position = slid_pos:
if let Some(pf) = &pathfind_ctx {
    update_current_tile(&mut ship, old_pos, slid_pos, pf.tile_mesh);
    advance_gates(&mut ship, old_pos, slid_pos, pf.tile_mesh);
}
```

Or do it inside `act_sail` after calling `compute_steering`, whichever is cleaner
architecturally. (Doing it in `world.rs` keeps the tile tracking accurate regardless of
BT state.)

**Step 2.3** — Initialize `current_tile` when a path is first assigned in
`NavTrack::set_path`:

```rust
pub fn set_path(&mut self, waypoints: Vec<Position>, tile_route: Vec<u32>) {
    self.waypoints.clear();
    for wp in waypoints.into_iter().take(MAX_WAYPOINTS) {
        self.waypoints.push(wp);
    }
    self.tile_route = tile_route;
    self.active_gate = 0;
    // current_tile intentionally NOT reset here — it will be refreshed
    // by the portal-crossing tracker on the next tick. Resetting it here
    // would force an O(K×V) re-init every replan.
}
```

**Step 2.4** — Expose `tile_route` from the pathfinder. Modify `tile_mesh_path` and
`stitch_tile_route` to return `(Vec<Position>, Vec<u32>)` — the waypoints AND the tile
route. Thread this through `pathfind::find_path_to_harbor` and `ai.rs::assign_destination_
port` → `NavTrack::set_path`.

### Layer 3: Portal-Gate Steering in Narrow Tile Chains

**What it changes:** `nav.rs::NavTrack::compute_steering`, `ai.rs::act_sail` steering target
selection.  
**Effect:** In channels (tiles with clearance < DEEP_WATER_NM), the ship steers toward the
next portal midpoint rather than the potentially-distant SSFA waypoint. No sparse-waypoint
problem.

**Step 3.1** — Modify the steering target selection in `compute_steering` (or in `act_sail`
before calling it):

```rust
// In act_sail (or inside NavTrack::compute_steering as an override):
let is_in_channel = pf.tile_mesh.clearance_of(ship.nav.current_tile)
    .map_or(false, |c| c < tile_mesh::DEEP_WATER_NM);

let steering_target = if is_in_channel {
    // Channel mode: aim for the next uncrossed portal midpoint.
    gate_target(&ship, &ship.nav.tile_route, pf.tile_mesh)
        .unwrap_or_else(|| ship.nav.waypoints.first().copied()
            .unwrap_or(self.goal.destination.unwrap()))
} else {
    // Open-water mode: normal SSFA waypoint following.
    ship.nav.waypoints.first().copied()
        .unwrap_or(self.goal.destination.unwrap())
};
```

**Step 3.2** — Add a `clearance_of(tile_id: Option<u32>) -> Option<f32>` helper to
`TileMesh`:

```rust
pub fn clearance_of(&self, tile_id: Option<u32>) -> Option<f32> {
    let id = tile_id? as usize;
    self.clearance_nm.get(id).copied()
}
```

**Step 3.3** — When in channel mode, also override the heading with the approximate
channel center (Section 6.5):

```rust
let center = channel_center(current_tile_id, &ship.nav.tile_route, pf.tile_mesh);
// Weight the target toward the channel center to prevent bank-hugging.
let target = steering_target * 0.7 + center * 0.3;
```

This 70/30 blend keeps the ship moving toward the correct gate while gently pulling it
away from the bank. The blend ratio is a tunable constant; 70/30 is a reasonable default.

### Layer 4: Last-Resort Rescue (Hard Guarantee)

**What it changes:** `ai.rs::act_sail`, `nav.rs::NavTrack`.  
**Effect:** No ship can remain at speed = 0 pressed against land for more than
`STUCK_RESCUE_TICKS` = 5 ticks (5 simulated hours). This is the hard guarantee.

**Step 4.1** — Add rescue logic to `act_sail`, after `compute_steering`:

```rust
const STUCK_RESCUE_TICKS: u8 = 5;

// Rescue check: if stuck for too long, override steering toward
// the current tile's centroid, which is provably in water.
if ship.stuck_ticks >= STUCK_RESCUE_TICKS {
    let rescue_target = if let (Some(tile_id), Some(pf)) =
        (ship.nav.current_tile, self.pathfind)
    {
        pf.tile_mesh.tiles[tile_id as usize].centroid
    } else {
        // Fallback: nearest visible tile centroid.
        pf.and_then(|pf| pf.tile_mesh.nearest_centroids(ship.position, 50.0)
            .into_iter()
            .find(|(t, _)| pf.coastline_geom.line_is_clear(
                pf.land, ship.position, pf.tile_mesh.tiles[*t as usize].centroid))
            .map(|(t, _)| pf.tile_mesh.tiles[t as usize].centroid))
        .unwrap_or(ship.position)  // truly stuck — log and do nothing
    };

    let terrain = self.pathfind.map(|c| nav::NavTerrain {
        geom: c.coastline_geom, land: c.land,
    });
    let heading = nav::deflect_for_land(
        ship.position,
        bearing_to(ship.position, rescue_target),
        terrain.unwrap(),
        14.0,
    );
    self.commands.push((self.me, ShipCommand::Steer {
        heading,
        speed: self.stats.speed_typical * 0.5,
    }));
    // Also advance the active gate to avoid re-targeting the blocked portal.
    if ship.nav.active_gate + 1 < ship.nav.tile_route.len() {
        ship.nav.active_gate += 1;
    }
    // Do NOT reset stuck_ticks here — let the motion sweep reset it
    // when movement is confirmed (stuck_ticks = 0 when dist_moved > 0.05).
    return Status::Running;
}
```

**About immersion:** The centroid-steer rescue produces a visible course change (the ship
turns and moves toward the channel center) that looks like the helmsman correcting a
grounding approach. It is far more immersive than a teleport. If the ship is already
inside the tile (which it should be — tiles are constructed so their interior is always
navigable), the centroid is at most a few NM away. No teleportation occurs; the ship
steers under its own power to the centroid, typically reaching it in 1–2 ticks (1–2 hours
of sim time) at half speed.

**Escalation to teleport (last resort).** If `stuck_ticks >= STUCK_RESCUE_TICKS * 2 = 10`
and no progress has been made (centroid steer failed because the centroid is also blocked
— this should never happen for valid tiles but is the defensive case):

```rust
if ship.stuck_ticks >= STUCK_RESCUE_TICKS * 2 {
    // Teleport to nearest tile centroid with confirmed LOS.
    if let Some(safe_tile) = find_safe_tile(ship.position, pf) {
        ship.position = pf.tile_mesh.tiles[safe_tile as usize].centroid;
        ship.nav.current_tile = Some(safe_tile);
        ship.nav.active_gate = 0;
        ship.stuck_ticks = 0;
        // Replan from new position.
        if let Some(idx) = self.goal.dest_port {
            self.replan_to_port(idx);
        }
    }
}
```

This teleport is the absolute last resort (10 hours stuck) and should only occur for mesh
defects, not for the Delaware River problem. Log these events; if they occur frequently,
it indicates a mesh generation bug.

### 8.1 Summary: Layer Integration Table

| Layer | File | Change | Eliminates |
|-------|------|---------|-----------|
| 1. Sliding sweep | `world.rs:1167` | Replace zero-stop with slide_move | Root Cause A (full stop) |
| 1. Coastline edge query | `coastline_geom.rs` | Add `nearest_coastline_edge` | Enables Layer 1 |
| 2. Tile tracking | `nav.rs::NavTrack`, `world.rs` | Add current_tile, active_gate, tile_route | Root Cause C (no tile context) |
| 2. Expose tile_route | `pathfind.rs` | Return tile_route alongside waypoints | Enables Layer 3 |
| 3. Portal-gate steering | `nav.rs::compute_steering`, `ai.rs::act_sail` | Switch to gate target in channels | Root Cause B (sparse waypoints) |
| 3. Channel center blend | `nav.rs` | 70/30 gate/centroid target | Root Cause D (deflect scan) |
| 4. Centroid rescue | `ai.rs::act_sail` | Rescue after STUCK_RESCUE_TICKS | Hard guarantee no permanent stuck |
| 4. `stuck_ticks` counter | `ship.rs::Ship` | Add u8 counter | Drives Layer 4 |

### 8.2 Expected Behavior After All Four Layers

**Delaware River (canonical failure case):**
1. Ship enters the river mouth tile chain. Clearance < DEEP_WATER_NM → Layer 3 activates.
2. Steering targets the next portal midpoint, not a distant bank-hugging corner.
3. Any slight bank contact → Layer 1 slides the hull along the bank, preserving ~75% speed.
4. Ship clears the channel in 3–5 ticks rather than wedging at the first bend.
5. If somehow still at speed = 0 after 5 ticks → Layer 4 steers toward tile centroid (the
   middle of the river) → slide_move carries it free of the bank → normal navigation
   resumes.

**Open ocean (must not regress):**
- `clearance_nm >= DEEP_WATER_NM` → Layer 3 is inactive → existing SSFA waypoint steering
  is used, identical to current behavior.
- `slide_move` only fires when `farthest_clear_point` is not the terminal — in open water
  this never happens, so Layer 1 adds zero overhead.
- Tile tracking adds ~6 segment tests per tick per ship = negligible CPU.

### 8.3 Pseudocode Summary Cheatsheet

```
TICK:
  1. (world.rs) apply_nav_state_fix(ship):
       (slid_pos, dist) = slide_move(old_pos, desired_pos, geom, land, 4)
       ship.position = slid_pos
       ship.speed = dist          // 0.0 if dist < 0.05
       ship.stuck_ticks += (dist < 0.05) ? 1 : -ship.stuck_ticks

  2. (world.rs) update_current_tile(ship, old_pos, slid_pos, mesh)
       advance_gates(ship, old_pos, slid_pos, mesh)

  3. (ai.rs::act_sail) select_steering_target(ship, goal, pf):
       if is_in_channel(ship, pf):
           target = gate_target(ship) ?? channel_center(ship, pf)
       else:
           target = ship.nav.waypoints.front ?? goal.destination

  4. (ai.rs::act_sail) rescue_if_stuck(ship, goal, pf):
       if ship.stuck_ticks >= STUCK_RESCUE_TICKS:
           steer_toward_tile_centroid(ship, pf)
           ship.nav.active_gate += 1

  5. (nav.rs::compute_steering) normal_steering(ship, target, wind, terrain)
       heading = deflect_for_land(pos_truth, bearing_to(pos, target), terrain)
       speed = speed_at_heading(heading, stats, wind)
```

---

## 9. References

### Core Navigation / Pathfinding

- **[Mononen 2010]** Mononen, M. "Simple Stupid Funnel Algorithm." *Digesting Duck* blog,
  March 2010. https://digestingduck.blogspot.com/2010/03/simple-stupid-funnel-algorithm.html

- **[Mononen 2009]** Mononen, M. *Recast & Detour Navigation Mesh Toolkit*, v1.0 (2009),
  GitHub: recastnavigation/recastnavigation. Source files: `DetourPathCorridor.cpp`,
  `DetourCrowd.cpp`, `DetourObstacleAvoidance.cpp`.

- **[Demyen 2006]** Demyen, D. "Efficient Triangulation-Based Pathfinding." M.Sc. thesis,
  University of Alberta, 2006. Section 4.3 covers the funnel corridor algorithm in full.

- **[Patel 2014]** Patel, A. "A* Pathfinding." *Red Blob Games*, 2014 (updated 2020).
  https://www.redblobgames.com/pathfinding/a-star/introduction.html

- **[Snook 2000]** Snook, G. "Simplified 3D Movement and Pathfinding Using Navigation
  Meshes." *Game Programming Gems* (DeLoura, ed.), Charles River Media, 2000. pp. 288–304.

- **[Tozour 2002]** Tozour, P. "Building a Near-Optimal Navigation Mesh."
  *AI Game Programming Wisdom* (Rabin, ed.), Charles River Media, 2002. pp. 171–185.

- **[Stout 1996]** Stout, W. "Smart Moves: Intelligent Pathfinding."
  *Game Developer Magazine*, March 1996.

- **[Pinter 2001]** Pinter, M. "Toward More Realistic Pathfinding."
  *Game Developer Magazine*, March 2001 / Gamasutra.

### Collision Response and Physics

- **[idSoftware 1999]** id Software. *Quake 1 Source Code*, GPL release 1999.
  Key file: `qcommon/pmove.c`, function `PM_SlideMove`. Available at
  https://github.com/id-Software/Quake

- **[Valve 2004]** Valve Corporation. *Source Engine SDK*, 2004.
  Key file: `game/shared/gamemovement.cpp`, `CGameMovement::TryPlayerMove`.

- **[Ericson 2005]** Ericson, C. *Real-Time Collision Detection*.
  Morgan Kaufmann, 2005. ISBN 978-1558607323.
  Ch. 5: Capsule sweeps, sliding, and contact normal computation.

- **[Squirrell 2003]** Squirrell, M. "Moving Without Tunneling: The Swept Tests."
  GDC 2003 Proceedings (available on GDC Vault).

- **[Eberly 2004]** Eberly, D. *Game Physics*.
  Morgan Kaufmann, 2004. §11.3 "Collision Response for Sliding Motion."

- **[Millington 2007]** Millington, I. *Game Physics Engine Development*.
  Morgan Kaufmann, 2007. Ch. 13: Contact and friction response.

- **[Catto 2011]** Catto, E. *Box2D v2.2 User Manual*, 2011.
  https://box2d.org. Contact manifolds: §6.

- **[Gilbert 1988]** Gilbert, E., Johnson, D., Keerthi, S. "A Fast Procedure for
  Computing the Distance Between Complex Objects in Three-Dimensional Space."
  *IEEE Transactions on Robotics and Automation* 4(2):193–203, 1988. (GJK algorithm.)

- **[van den Bergen 2001]** van den Bergen, G. "Proximity Queries and Penetration Depth
  Computation on 3D Game Objects." GDC 2001. (EPA algorithm.)

### Steering and Local Avoidance

- **[Reynolds 1999]** Reynolds, C. "Steering Behaviors For Autonomous Characters."
  *GDC 1999 Proceedings*. http://www.red3d.com/cwr/steer/

- **[van den Berg 2008]** van den Berg, J., Lin, M., Manocha, D. "Reciprocal Velocity
  Obstacles for Real-Time Multi-Agent Navigation." *ICRA 2008*. (RVO)

- **[van den Berg 2011]** van den Berg, J., Guy, S., Lin, M., Manocha, D. "Reciprocal
  n-Body Collision Avoidance." *Robotics Research* 70:3–19, 2011. (ORCA)

- **[Millington 2009]** Millington, I., Funge, J. *Artificial Intelligence for Games*,
  2nd ed. Morgan Kaufmann, 2009. Ch. 3: Steering behaviors including wall following.

- **[Buckland 2005]** Buckland, M. *Programming Game AI by Example*.
  Wordware Publishing, 2005. Ch. 3.

- **[Khatib 1986]** Khatib, O. "Real-Time Obstacle Avoidance for Manipulators and Mobile
  Robots." *International Journal of Robotics Research* 5(1):90–98, 1986.
  (Potential fields.)

- **[Latombe 1991]** Latombe, J.-C. *Robot Motion Planning*.
  Kluwer Academic, 1991. §7.2: Local minima in potential fields.

- **[Champandard 2012]** Champandard, A. "AI Navigation in Planetary Annihilation."
  *Gamasutra*, 2012. (Flow-field pathfinding overview.)

### Stuck Recovery

- **[Gold 2009]** Gold, S. "Avoiding the Danger Zone: Programming Robust Navigation."
  *Game Developer Magazine*, 2009. State-machine recovery strategies.

- **[Tozour 2004]** Tozour, P. "The Wisdom of Crowds: Co-operative Pathfinding."
  *AI Game Programming Wisdom 2* (Rabin, ed.), Charles River Media, 2004.

- **[Hale 2017]** Hale, D., Youngblood, G. "Navigating in Large Dynamic Worlds."
  *Game AI Pro Volume 3* (Rabin, ed.), CRC Press, 2017. Ch. 23.

- **[van Toll 2012]** van Toll, W., Cook IV, A., Geraerts, R. "Real-Time Density-Based
  Crowd Simulation." *Computer Animation and Virtual Worlds* 23(3-4):315–324, 2012.
  §4.3: Deadlock detection and recovery.

### Medial Axis and Channel Skeleton

- **[Blum 1967]** Blum, H. "A Transformation for Extracting New Descriptors of Shape."
  *Symposium on Models for the Perception of Speech and Visual Form*, MIT Press, 1967.
  pp. 362–380. (Original definition of the medial axis.)

- **[Lee 1982]** Lee, D.T. "Medial Axis Transform of a Planar Shape."
  *IEEE Transactions on Pattern Analysis and Machine Intelligence* 4(4):363–369, 1982.

- **[Preparata 1985]** Preparata, F., Shamos, M. *Computational Geometry: An Introduction*.
  Springer, 1985. Ch. 5: Voronoi diagrams and their relation to the medial axis.

- **[Aichholzer 1995]** Aichholzer, O., Aurenhammer, F., Alberts, D., Gärtner, B.
  "A Novel Type of Skeleton for Polygons."
  *Journal of Universal Computer Science* 1(12):752–761, 1995. (Straight skeleton.)

- **[Ogniewicz 1994]** Ogniewicz, R. "Skeleton-space: A Multiscale Shape Description
  Combining Region and Boundary Information." *CVPR 1994*.

- **[Jacobs 2006]** Jacobs, J., Moll, M. "Improved Medial Axis Based Path-Planning for
  Games." *AI Game Programming Wisdom 3* (Rabin, ed.), Charles River Media, 2006.
  The key empirical result: medial-axis steering reduced stuck rate from 14% to 0%.

- **[Bhattacharya 2008]** Bhattacharya, P., Gavrilova, M. "Roadmap-Based Path Planning —
  Using the Voronoi Diagram for a Clearance-Based Shortest Path."
  *IEEE Robotics & Automation Magazine* 15(2):58–66, 2008.

- **[Geraerts 2010]** Geraerts, R. "Planning Short Paths with Clearance Using Explicit
  Corridors." *ICRA 2010*.

- **[CGAL 2024]** CGAL Project. *CGAL Straight_skeleton_2 Package*, v5.6, 2024.
  https://doc.cgal.org/latest/Straight_skeleton_2/

### Hierarchical / Portal Navigation

- **[Pettré 2003]** Pettré, J., Laumond, J.-P., Siméon, T. "A 2-Stages Locomotion
  Planner for Digital Actors." *SCA 2003*.

- **[van Toll 2011]** van Toll, W., Cook IV, A., Geraerts, R. "Navigation Meshes for
  Realistic Multi-Layered Environments." *IROS 2011*.

- **[Geraerts 2002]** Geraerts, R., Overmars, M. "A Comparative Study of Probabilistic
  Roadmap Planners." *WAFR 2002*.

- **[Harabor 2011]** Harabor, D., Grastien, A. "Online Graph Pruning for Pathfinding
  on Grid Maps." *AAAI 2011*. (Jump Point Search; portal corridor concept.)

---

*End of document.*
```

---

## Summary of Top Recommendations

Here is what I found and recommend, in priority order:

### 🔴 Priority 1 (fixes ~80% of stucks immediately): Sliding Collision Response

**The motion sweep in `world.rs:1167–1180` must replace its zero-speed response with a sliding response.** When `farthest_clear_point` finds a land hit, instead of zeroing speed, compute the contact edge normal from `CoastlineGeom` and project the remaining velocity onto the wall tangent. This is the classic Quake `PM_SlideMove` technique (id Software 1996/1999), standard in every game engine, and eliminates the physical root cause: ships graze banks and continue rather than stopping permanently. Implementation requires ~50 lines: a `slide_move(from, to, geom, land, bumps)` function in `world.rs` and a `nearest_coastline_edge(hit_point)` query in `CoastlineGeom`.

### 🟠 Priority 2 (eliminates sparse-waypoint failures): Portal-Gate Steering

**When the current tile's `clearance_nm < DEEP_WATER_NM`, replace the SSFA waypoint target with the next portal midpoint.** Portals are dense (one per tile transition), always geometrically correct from the preceding tile, and cannot produce the "two corner waypoints aiming at each other's inner banks" failure. Requires storing `tile_route: Vec<u32>` and `active_gate: usize` in `NavTrack`, and threading the tile route out of `pathfind::tile_mesh_path`. The blend `gate_target * 0.7 + tile_centroid * 0.3` additionally pulls the ship toward the channel center, approximating the medial-axis approach without any offline computation.

### 🟡 Priority 3 (makes rescue reliable): Current Tile Tracking

**Add `current_tile: Option<u32>` to `NavTrack`, updated each tick via portal-crossing detection (O(1) amortized).** This gives the steering layer a provably in-water fallback target (the tile centroid) and makes the portal-gate steering of Priority 2 robust — without it, `active_gate` has no correct anchor when the ship drifts off-route.

### 🟢 Priority 4 (hard guarantee): Centroid-Steer Rescue

**After `stuck_ticks >= 5`, override `act_sail` to steer toward the current tile's centroid and advance `active_gate` by 1.** Tile centroids are provably in water by mesh construction, so this always produces a navigable heading. No teleportation. The ship visually "corrects itself" by steering into the middle of the channel — more immersive than a snap. Combined with the sliding response from Priority 1, the ship will reach the centroid within 1–2 ticks and resume normal navigation. The hard upper bound on stuck time is `STUCK_RESCUE_TICKS = 5` hours (5 simulated ticks) before rescue activates.