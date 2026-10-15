# Visigoth tactical simulator

This Rust crate is a deliberately approximate laboratory for tactical ideas. It
does not try to replace Recoil or predict an entire BAR match. It runs small,
repeatable interactions quickly enough to compare movement policies over many
seeds.

The current vertical slice models:

- 30 Hz facing, acceleration, braking, turn rate, turn-in-place behavior, and
  forward/reverse selection, radius-aware terrain clearance, and circular unit
  collision;
- independent turret traverse where BAR supplies `turretspeedx/y`;
- damage **per projectile**, reload, burst size, intra-burst interval, projectile
  count, projectile travel time, angular error, splash falloff, and optional
  damage-over-time duration;
- 3-D direct-fire tests from cached S3O firing origins against terrain and
  simplified vertical unit/wall volumes; one tiny global projectile clearance
  replaces per-projectile collision geometry;
- Recoil-style moving-target lead: nominal flight-time prediction, periodically
  imperfect velocity estimation, `predictBoost`, and `leadLimit`;
- movement-class grid routing over compiled-map passability and layered dynamic
  occupancy, with unit-radius clearance, bounded A*, smoothing, and
  obstacle-epoch cache invalidation;
- pluggable, read-only `Policy` implementations and built-in charge, hold, kite,
  and orbit baselines;
- simultaneous per-frame damage, explicit overkill accounting, value exchanged,
  damage dealt/received, hit counts, survival value, outcome, and duration;
- compact due-tick impact scheduling plus an adaptive uniform broad phase for
  dense blocker and impact-victim queries;
- deterministic seeds and Rayon-parallel batches with reusable per-worker
  trial state and counter-keyed shot randomness;
- a compact generational strategic world for live units, structures,
  nanoframes, factories, walls, and reclaimable features;
- closed-loop income/storage/upkeep, travel/build/assist/repair/reclaim,
  factory queues/stalls/release, wall occupancy, and next-tick activation;
- deterministic strategy actions, resource-conserving strategic score vectors,
  bounded online/paired experiment aggregation, and exact terminal hashes;
- opt-in exact command replay plus a loopback-only HTTP/WebSocket symbolic live
  viewer; neither is initialized by ordinary headless tactical runs;
- versioned Recoil calibration fixtures with exact and statistical tolerances.

`run_parallel_results` retains every trial for a custom scorer;
`run_parallel` returns a compact aggregate with win rates, Wilson 95% intervals,
and mean metric vectors. Both preserve seed order regardless of worker count.
`run_paired_parallel` runs both policies on both sides for every seed and
normalizes output slots back to policy A/B, cancelling fixed side bias.

For performance regression work, `--benchmark-suite --benchmark-runs 10000`
runs fixed 1v1, 16v16, burst/AOE, long-flight, blocked-LOS, traced, and
strategic-only shapes. It reports simulated ticks and spawned unit-ticks in
addition to raw throughput, along with target and capability metadata. The
current baseline and audit are in `PERFORMANCE.md`; the dependency-ordered
roadmap is in `IMPLEMENTATION_PLAN.md`.

Paper DPS is derived for reporting only. The engine never applies DPS directly.
Consequently, burst and damage-over-time weapons with the same reported DPS can
produce different outcomes.

## Unit data

The bridge now adds an optional `simulation` object to every existing catalog
entry. It is generated from the `UnitDefs` and `WeaponDefs` loaded by the actual
match, so it follows balance and engine versions. Existing Python code ignores
the extra object.

Install the updated bridge once, then save a versioned catalog from the next
match and run a batch:

```powershell
python install.py
python run.py --catalog-out data/simulator-catalog.json
cargo run --release --manifest-path simulator/Cargo.toml -- \
  --catalog data/simulator-catalog.json --unit armpw --runs 10000 --paired --left kite --right charge
```

Engine-reference calibration is captured over the same bridge. See
[`docs/CALIBRATION_CAPTURE.md`](../docs/CALIBRATION_CAPTURE.md) for the bounded
movement, weapon-state/damage, and obstruction microcase protocol and the
generated `CalibrationFixture`/`MeasuredObservations` artifacts.

Add `--viewer duel.html` for a sampled, standalone top-down replay (or
`--trace duel.json` for tooling). Rendering is a separate representative run,
so the parallel batch remains on the allocation-free headless path. The viewer
uses BAR strategic icons when `--assets` can resolve them, otherwise stable
geometric symbols, and draws weapon intent/lead as lines rather than projectiles.
`--human-side 0` enables deterministic move/attack command events for a future
live driver; it does not pause or mutate the offline replay.

`--aggregate-viewer heat.html --aggregate-runs 250` reduces sampled runs to
fixed-size occupancy, aim-point, and death heatmaps. The aggregate memory cost
is bounded by `--heat-cols` × `--heat-rows`; generating it is an explicit
visualization workload and does not alter the normal batch implementation.

Compile terrain and resources from canonical native capture output once, then
reuse the compact cache in tactical runs:

```powershell
cargo run --release --manifest-path simulator/Cargo.toml -- --compile-map data/map.map_source.json --map-out data/map.vsgmap
cargo run --release --manifest-path simulator/Cargo.toml -- --catalog data/hello.json --map data/map.vsgmap --unit armpw --runs 10000
```

`--map` also accepts a canonical `.map_source.json` or bridge hello JSON and
compiles it once in memory before a duel, declarative scenario, or live server
starts. That makes small hand-authored maps convenient without changing what a
tick sees: height, water depth, named movement passability, metal spots or a
continuous metal field, geothermal vents, and static blockers all enter the
same immutable `CompiledMap`. For example, the checked-in wall scenario can be
started directly from its resource-bearing map description:

```powershell
cargo run --release --manifest-path simulator/Cargo.toml -- `
  --catalog simulator/examples/pawn-hello.json `
  --run-scenario simulator/examples/wall-defense.json `
  --map simulator/examples/flat-pawn-map.json --serve 127.0.0.1:0
```

For repeated batches, compile the source once to `.vsgmap`; the regular trial
path never parses JSON or recompiles terrain.

To use a real BAR map, start that map in BAR as a full-view spectator and run
`python -m bridge.native_map capture --out-dir data/maps --minibar-cell-size 64`.
It asks Recoil for the installed map rather than parsing its archive, then
writes a compact height/passability/resource map that can be passed directly to
`--map`. Details and the capture prerequisites are in
[the native-map guide](../docs/NATIVE_MAP_CAPTURE.md).

The CLI also accepts bridge hello JSON. Canonical assembler output embeds
engine/game/map identity, options, extractor rules, and MoveDef schema in the
versioned cache; older canonical files can recover that provenance from an
auto-discovered sibling acquisition file. Exact native corner heights are
preserved only by the canonical input.

The cache stores quantized heights, water depth through the retained water
plane, named MoveDef passability masks, metal spots, continuous metal fields,
geothermal vents, and removable static feature footprints. Continuous metal is
preconvolved at compile time so an extractor-site query is constant-time. Use
`World::new_with_static_features` when a strategic trial should materialize
captured blockers as neutral feature entities and counted occupancy. See
`MAP_PIPELINE.md` for the acquisition contract and metal-map caveats.

The strategic `World` connects deterministic resource income/storage/stalls,
streamed nanoframe construction, builder travel and assistance, factory queues,
repair, reclaim, and release blocking. Walls become physical while under
construction and expose progress-scaled line-of-fire height. Snapped/rotated
placement, layered counted occupancy, and movement-class routing cover map-side
legality and builder navigation. Stateful strategies can drive this loop in
bounded parallel, side-swapped experiments and emit the same metric vector used
by online analysis. Filtered observations support terrain- and blocker-aware
LOS, anonymous quantized radar contacts, and optional deterministic per-side
last-seen memory. Memory preserves the contact's last disclosure level and is
cleared when its remembered position is observed to be empty.

`live::LiveDriver` applies commands on authoritative ticks and produces an exact
replay log. `live_server::start_world_live_server` exposes that driver only on a
loopback HTTP/WebSocket endpoint, serving the shared top-down symbolic viewer.
Starting the server is explicit, so normal headless experiments pay no socket or
viewer cost.

The same maintained, dependency-free asset is available at
`simulator/viewer/viewer.html` for read-only simulator-trace and recorded BAR
trajectory playback. Its versioned `ViewerFrame` protocol is independent of
`World`; historical observations are rendered directly and never advanced
through MiniBar physics, combat, economy, targeting, or pathfinding. See
`simulator/viewer/README.md` for the bootstrap/frame API, provenance and
recorded-versus-derived conventions, radar privacy guarantees, and Python
integration contract.

For a declarative scenario, add `--serve 127.0.0.1:0 --live-log session.json`.
The viewer receives terrain height/water/passability, resource sites, health,
construction progress, routes, firing intent, and authoritative build previews;
it submits queue, assist, cancellation, and activation commands through the same
validated action path as strategies. Replay the resulting interaction against an
identically initialized scenario with `--replay-live session.json`; the command
fails if its terminal tick or authoritative state hash differs.
Use the live **Control** selector to switch a browser between Blue/side 0 and
Red/side 1. The selected side determines both its fog-of-war observation and
the only side it may command, so two people can play the same small scenario
from opposite sides. Human command sequences are allocated by the server and
cannot collide with scripted strategies or another local browser.

For a ready interactive resource-map session, use
`examples/river-two-player.json` with
`examples/river-crossing.map_source.json`; open the printed loopback address in
one or two browser tabs and set each tab's **Control** selector to its side.
The declared scenario duration remains the offline-test horizon; live sessions
run until the operator stops them unless `--live-max-ticks N` supplies a cap.

For source checkouts of BAR, add `--assets PATH_TO_BAR_SOURCE`. Catalog loading
then resolves `model_name` and `script_name`, parses the S3O piece hierarchy,
finds the piece returned by BOS `QueryWeaponN` (including the legacy primary /
secondary / tertiary names), and caches its projectile emit point. According to
the S3O convention, pieces with zero or one vertex emit at the piece origin and
pieces with at least two vertices emit at their first vertex. Asset processing
happens once and is absent from the simulation hot loop.

If `--assets` is omitted, weapons fall back to the unit collision midpoint.
That is useful for smoke tests, but geometry-sensitive experiments should use
derived origins. Script-controlled alternating barrels or pose-dependent piece
translations remain explicit future work; ambiguous source fails during asset
loading rather than selecting an arbitrary piece.

For a smoke test, `examples/pawn-hello.json` contains a clearly labelled
reference Pawn profile. It is not a substitute for regenerating data from the
installed BAR version when comparing real tactics.

The JSON result keeps metrics separate. Do not optimize a single composite
score until the experiment declares its weights; otherwise a policy can appear
better merely by learning the scorer.

## Performance contract

Run the fixed workload suite before and after mechanics changes:

```powershell
cargo build --release --manifest-path simulator/Cargo.toml
simulator/target/release/visigoth-simulator.exe --catalog simulator/examples/pawn-hello.json --unit armpw --benchmark-suite --benchmark-runs 10000 --seed 1
```

It covers a duel, 16v16 formation, burst/AOE, long-flight projectiles, blocked
terrain LOS, traced output, and a 16v16 strategic tick loop. [PERFORMANCE.md](PERFORMANCE.md)
records the current baseline and audit; [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md)
is the dependency-ordered roadmap.

## Deliberate limits

This is an executable approximation, not a claim of engine parity. Strategic
attack orders, construction, walls, deaths, wrecks, resources, and factories
share one deterministic world with the tactical stepper. Unguided gravity
weapons solve low or enforced-high analytical arcs, test terrain and cylinder
blockers along the curved path, and schedule only their due impact—there is no
per-frame projectile allocation. Channeled beams, homing, shields, paralysis,
and interception are also modelled.

The deliberate limits are fixed-speed ballistic flight (no weapon acceleration,
bounce, cratering, or tactic-selected toggleable high trajectory), simplified
projectile radii, and symbolic rather than visual physics. Each mechanism still
needs a small Recoil calibration fixture before it should be treated as engine
parity.
