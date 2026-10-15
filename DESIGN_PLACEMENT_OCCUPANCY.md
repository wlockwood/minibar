# Design: Placement and Layered Occupancy (Milestone 3)

## 1. Overview

The placement and occupancy subsystem replaces simple boolean obstruction grids with a counted, layered spatial representation and deterministic building placement rules. It operates on Recoil engine's 16-elmo building grid and supports factories, nanoframes, walls, resource nodes (metal extractors and geothermal plants), and upcoming pathfinding.

The subsystem is partitioned into two focused modules:
- `occupancy.rs`: Owns spatial occupancy layers, reference-safe counted cell contributions, and independent topology epochs.
- `placement.rs`: Owns compass build orientation (`BuildFacing`), 16-elmo grid snapping, footprint/yardmap rotations, and structured validation against `CompiledMap` terrain and `OccupancyGrid`.

---

## 2. 16-Elmo Build-Grid Snapping & Footprint Rotation

### Parity-Aware Snapping Rule
Recoil engine (`CGameHelper::Pos2BuildPos`) snaps buildings to a 16-elmo build grid with coordinates aligned to cell boundaries or cell centers based on footprint dimension parity:
- **Even footprint dimension** ($W \equiv 0 \pmod 2$): The building's center falls on a 16-elmo grid line ($k \cdot 16.0$).
  $$\text{snap}(x, W) = \left\lfloor \frac{x + 8.0}{16.0} \right\rfloor \cdot 16.0$$
- **Odd footprint dimension** ($W \equiv 1 \pmod 2$): The building's center falls on a half-cell offset ($k \cdot 16.0 + 8.0$).
  $$\text{snap}(x, W) = \left\lfloor \frac{x}{16.0} \right\rfloor \cdot 16.0 + 8.0$$

In all cases, the bounding box edges $[x - W \cdot 8, x + W \cdot 8]$ align exactly to integer multiples of 16 elmos, avoiding fractional cell splits.

### Compass BuildFacing and Rotation
Facings follow Recoil's `FacingMap` enum:
- `South = 0` (+Z facing, base catalog orientation)
- `East = 1` (+X facing)
- `North = 2` (-Z facing)
- `West = 3` (-X facing)

Rotations of rectangular footprints and yardmaps:
- Footprint $[W, D]$ rotates to $[W, D]$ for South/North, and $[D, W]$ for East/West.
- Yardmaps rotate per Recoil's transformation matrix:
  - **South**: $(x, z) \mapsto (x, z)$
  - **North**: $(x, z) \mapsto (W - 1 - x, D - 1 - z)$
  - **East**: $(x, z) \mapsto (z, W - 1 - x)$
  - **West**: $(x, z) \mapsto (D - 1 - z, x)$

---

## 3. Layer Semantics & Reference Safety

The occupancy grid tracks five distinct, orthogonal layers:
1. **Movement Blocking**: Counted (`movement_count: u16`). A cell is blocked to ground movement if and only if `movement_count > 0`.
2. **Fire Blocking & Height**: Continuous (`max_fire_height: f32`). Effective cell fire-blocking height is the maximum height across all active fire-blocking contributors in that cell.
3. **Construction Reservation**: Counted (`reservation_count: u16`). Soft reservation for pending construction projects.
4. **Factory Yard**: Counted (`factory_yard_count: u16`). Dedicated zone for factory pads and roll-out corridors.
5. **Resource Sites**: Discrete index mapping for geothermal vents (`geothermal_reservations`) and metal spots (`metal_spot_reservations`), plus circular exclusion zones for continuous metal fields (`continuous_extractors`).

### Reference-Safe Overlapping Contributions
A single cell can contain contributions from multiple occupants (e.g. adjacent walls, multiple reservations, or stacked structures). Each cell retains a `Vec<CellContribution>` tagged with the contributor's `OccupantId`.
- Adding an occupant increments layer counts and recomputes the cell's maximum fire height.
- Removing an occupant (`remove_occupant(id)`) subtracts only that occupant's contributions. If another occupant also blocked movement in that cell, `movement_count` decreases but remains $> 0$, keeping the cell blocked without corrupting the surviving occupant's state.

### Nanoframe & Wall Height Updates
Walls grow linearly during construction from height $0.0$ to full height. Calling `update_fire_height(occupant_id, new_height)` modifies the fire height of the occupant's contributions in-place and recomputes cell maximum heights without removing, re-inserting, or touching movement counts, reservations, or other occupants.

---

## 4. Decoupled Topology Epochs

Pathfinding and line-of-sight algorithms cache routing graphs and visibility lines. Invalidating these caches on every minor reservation or tick is computationally prohibitive. `OccupancyGrid` decouples its revision counters:

- `movement_epoch`: Increments **only** when a cell transitions between blocked (`count == 0 \to 1`) and unblocked (`count \to 0`). Adding a redundant blocker to an already-blocked cell or removing one of several blockers does *not* increment `movement_epoch`.
- `fire_epoch`: Increments **only** when the effective maximum fire-blocking height of a cell changes. Height updates below the cell's current maximum (or changes to non-fire layers) do *not* increment `fire_epoch`.
- `reservation_epoch`: Increments when construction reservations change.
- `yard_epoch`: Increments when factory yard reservations change.
- `resource_epoch`: Increments when metal or geothermal site claims change.
- `global_epoch`: Increments on any modification, providing backward compatibility with single-epoch consumers.

---

## 5. Deliberate Approximations

1. **Prism and Cylinder Collisions**: Buildings are represented as axis-aligned bounding boxes on the 16-elmo grid. Fine-grained 3D polygonal collision meshes are deliberately approximated as vertical prisms.
2. **Preconvolved Metal & Spot Coexistence**: Extraction radius queries on continuous metal maps use compile-time quantized convolution for $O(1)$ yield lookups. When discrete metal spots are present, discrete spot claims take precedence.
3. **Corridor Clearance Queries**: BFS corridor connectivity checks evaluate axis-aligned footprint bounding boxes moving through open cells, exposing the necessary topological queries for pathfinding without embedding full A* replanners into the occupancy module.
