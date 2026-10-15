# Closing the simulation loop

The strategic simulator should model economy as resource streams and
construction as work streams. It should not reproduce Recoil internally. Walls
are ordinary constructible entities whose footprint, height, and blocking flags
make them wall-like.

## Staged architecture

1. Load the full reachable catalog into immutable shared storage. Runtime units
   carry a compact definition ID instead of cloning definitions. Keep map and
   catalog data in `Arc`s shared by every parallel trial.
2. Add `economy.rs` for team storage, capacity, income, upkeep, spending,
   stalls, and overflow. Add `construction.rs` for nanoframes, mobile builder
   tasks, assistance, and factory queues.
3. Add `placement.rs` and a dynamic occupancy grid. Static map data needs
   8- or 16-elmo cells; the current coarse 32-by-32 sample cannot represent wall
   gaps, factory yards, footprints, slopes, or movement classes.
4. Add movement-class navigation and obstacle-versioned path caches. Creating
   or destroying a wall changes an obstacle epoch; cached routes are keyed by
   movement class, goal, and epoch.
5. Add features/reclaim and BAR-specific economy systems only after golden
   traces establish their behavior.

Suggested modules are `economy`, `construction`, `placement`, `pathing`,
`spatial`, `strategy`, and `feature`. `engine.rs` remains the deterministic
orchestrator.

## Construction and factories

A construction order advances through queued, travelling, waiting for site,
constructing, and complete states. A nanoframe is a physical, attackable entity.
Builders in range contribute work:

```text
progress demand = total build power / build time * dt
metal demand    = metal cost  * progress demand
energy demand   = energy cost * progress demand
```

The limiting available resource scales applied progress. Resources stream in;
the full price is not reserved when the command is issued. Equal-priority jobs
receive deterministic pro-rata allocation, with project ID resolving rounding.

Factories use a persistent queue and advance through idle, starting item,
constructing at pad, releasing unit, and exit-blocked states. A completed unit
must clear the yard before the next factory interaction that requires that
space. Queue operations need append, prepend, replace, cancel, and repeat.

The first wall implementation uses the same construction state as every other
building. From nanoframe creation onward it occupies its footprint, has height
scaled by progress, blocks routing, and participates in direct-fire obstruction.
Those two nanoframe assumptions must be calibrated against Recoil.

## Deterministic tick order

1. Build observations from the prior completed tick.
2. Run stateful side strategies on their configured cadence and validate actions
   in stable `(side, sequence)` order.
3. Advance movement, facing, builder approach, and factory exits.
4. Create legal nanoframes and update occupancy.
5. Gather income, upkeep, construction, repair, and weapon-resource demands.
6. Allocate resources and apply construction work simultaneously.
7. Advance aiming/firing and resolve scheduled impacts.
8. Resolve simultaneous damage, deaths, wrecks, and occupancy changes.
9. Finalize completions for activation on the next tick, then record metrics.

Parallelism belongs across trials, never inside a trial. Random draws should
eventually be counter-based on seed, tick, entity, weapon, and shot serial so a
new unrelated random event cannot perturb existing experiments.

## Data contract additions

The existing bridge already exports costs, build time/power/range/options,
factory/builder flags, resource production/storage/upkeep, footprint, water/geo
constraints, map wind/tidal values, and metal spots. Retain all of those in the
Rust catalog.

The closed loop additionally needs yardmaps; blocking/collision flags; movement
class, slope, water, and crush rules; build/assist/repair permissions; extractor
radius; activation behavior; weapon energy and stockpile costs; corpse/resource
data; exact build-square sizes; geo positions; mod options; and starting
resources. Script-controlled nano points, factory door/yard timing, gadget
economies, wind time series, and map placement vetoes require runtime traces or
asset preprocessing rather than guesses.

## Metrics and calibration

Keep a metric vector rather than one universal score: outcome, army/builder/
factory/economy value, integrated income, generated/spent/overflow resources,
stalled build-power-seconds, idle factory/builder time, milestone times,
production throughput, incomplete sunk cost, and map control.

Wall experiments additionally record path time added, first breach time, shots
and damage intercepted, leaks, friendly movement/fire obstruction, survival
time, and exchange around the breach.

Calibrate in a ladder: one abundant-resource factory build; a travelling mobile
builder; assistance and builder death; isolated metal/energy stalls; storage;
individual generators/extractors; queue and blocked exit; footprint and site
constraints; then single walls, gaps, continuous lines, fire-over/fire-into,
and splash-behind-wall cases. Finally compare a complete opening and a raided
expansion against versioned headless-Recoil traces.
