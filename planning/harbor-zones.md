# Harbor Zones Plan

## Motivation

Pathfinding to a literal port coordinate forces ships to thread tiny river
channels and coastal slivers — fragile against grid resolution and clearance
choices. Worse, ports inland from open water (Philadelphia up the Delaware,
Albany up the Hudson) are unreachable without exquisitely accurate land masks.

Trade-sim conventions (Anno 1404/1800, Patrician, Port Royale) treat each port
as owning a **harbor zone**: a region of sea cells around the port where ships
are considered "in port" for purposes of docking, trade, and combat. Pathfinding
targets the zone, not the literal point; arrival triggers when a ship enters
the zone.

This decouples three concerns that are currently tangled:

1. **Routing clearance** (how much sea margin A* requires for safety).
2. **Reachability** (can a ship physically arrive at this port?).
3. **Docking** (when does the ship transition from sailing to docked?).

## Design

### Per-port harbor radius

Add a field to each port:

```rust
pub struct Port {
    // ...existing fields...
    pub harbor_radius_nm: f32,  // default 8.0
}
```

Defaults: 8 NM is enough that a port with an open coast (Bridgetown, Tobago)
gets a comfortable docking annulus, while small enough that adjacent ports on
the same coast won't overlap meaningfully. Override per-port for special cases
(Philadelphia ≈ 80 NM down the Delaware to Cape May, Albany ≈ 130 NM down the
Hudson, New Orleans ≈ 100 NM from the river mouth).

Data lives in `data/ports.json` (or wherever ports are defined now); keep
loading code defaulting to 8.0 if the field is missing for backwards compat.

### Harbor zone (computed at world load)

Per port, compute the set of sea cells comprising its harbor zone:

1. Find the **harbor anchor cell**: the nearest sea cell to the port position
   (using existing `nearest_sea_cell`).
2. **BFS from anchor** through 8-connected sea cells, with two stopping rules:
   - Cell distance from the *port position* exceeds `harbor_radius_nm`.
   - BFS frontier is exhausted (small enclosed inlets terminate naturally).
3. Result: a `Vec<(u32, u32)>` of cells, plus a hashed set for O(1) "is in
   zone" tests.

Why BFS instead of "all cells within radius"? An isthmus or peninsula can place
land within the radius but still on the wrong side of the coast — those cells
must not be considered part of the harbor. BFS through sea-only respects
connectivity.

Storage:

```rust
pub struct Harbor {
    pub port_index: usize,
    pub anchor: Position,         // best deep-water approach point
    pub cells: HashSet<(u32, u32)>,
    pub bbox: (Position, Position),  // for cheap pre-check
}

pub struct HarborMap {
    pub harbors: Vec<Harbor>,
    // Reverse index: cell -> port_index, for fast "what harbor am I in?" lookup.
    pub cell_to_port: HashMap<(u32, u32), usize>,
}
```

`HarborMap` lives on `World` next to the land map and is built once at
`World::load`.

### Pathfinding integration

`find_path(ctx, start, port_index)` (or an overload taking a `Harbor`):

- Goal predicate becomes `|cell| harbor.cells.contains(cell)` instead of
  `cell == goal_cell`.
- Heuristic uses distance to `harbor.anchor` (admissible — the anchor is the
  closest cell of the goal set in the typical case; for guaranteed admissibility
  we can use distance to the bounding-box edge if needed).
- A* terminates the moment any harbor cell is popped — that cell becomes the
  end of the planned path.
- Final waypoint = the popped harbor cell, smoothed back along the path as
  before.

This eliminates the "ship can't enter a 1-cell-wide channel because clearance
is 1" failure mode for ports because the *zone itself* is the destination, and
any cell in the zone counts.

### Arrival logic

`NavState::compute_heading` (or world tick) checks: is the ship's current
position inside its destination's harbor zone? If yes, transition to
`Docked` (or whatever the docking state machine wants). No more
`pos.distance(literal_port) < ARRIVAL_NM` check — that goes away.

Side effect: ships will visibly stop short of the literal port marker, which
is fine and matches reality (ships anchor in the harbor; they don't sail onto
the dock). The viz can render the harbor zone as a faint disk so this reads
correctly.

### Combat / interception (forward-looking)

Pirates probably should not attack ships inside friendly harbor zones (forts,
guard ships, no-fight rule). Ships entering a hostile zone might be challenged.
This is out of scope for now but the data model supports it: `cell_to_port`
already gives us "is this position in someone's harbor."

## Visualization

- Render each harbor zone as a translucent colored disk (color by port faction).
- Optionally outline the zone (BFS boundary cells).
- Useful for debugging: misshapen zones immediately reveal bad data (port
  position on land, port too far from sea, etc.).

## Migration steps

1. Add `harbor_radius_nm` field to `Port` with a default.
2. Build `HarborMap` in `World::load` after `LandMap` is loaded.
3. Update `find_path` to take a goal harbor (or a goal predicate) instead of
   a goal `Position`.
4. Update callers in `ai.rs` (`assign_destination`) to pass the harbor for the
   chosen port.
5. Replace `ARRIVAL_NM`-based arrival in `nav.rs` / `world.rs` with a
   "is in destination harbor zone" check.
6. Drop the `goal_anchor` workaround in `pathfind.rs` (no longer needed —
   the zone itself absorbs the role of the snapped anchor).
7. Update viz to draw harbor zones.
8. Tune per-port radii in port data for ports that need it (Philly, Albany,
   New Orleans, etc.).

## Testing

- Unit test: small synthetic map, port on a 1-cell-wide channel, verify A*
  succeeds against the harbor zone where it would fail against the literal
  port cell.
- Unit test: peninsula case — a port with land on three sides, verify the
  harbor zone is on the seaward side only (BFS-connected from sea), not
  bleeding through the peninsula.
- Integration: rerun `examples/diag_nav.rs` and verify all major routes
  including Philadelphia and New York succeed.

## Out of scope (future)

- Multi-zone harbors (e.g. a port with two valid approaches separated by an
  island).
- Tide / depth-aware harbor zones (deep-draft ships can't anchor everywhere).
- Pirate "no-go" zones around enemy harbors.
