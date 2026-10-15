# Simulator implementation plan

This plan is dependency ordered. Fidelity is added only when it can be measured
against both a performance baseline and a small Recoil calibration scenario.
The ordinary tactical runner must not initialize optional strategic, trace, or
live-view systems.

Status legend:

- **Implemented** means the feature exists and has automated coverage.
- **Partial** means the useful core exists, but listed acceptance work remains.
- **External validation pending** means capture/comparison tooling exists but
  has not yet been exercised against a real Recoil/BAR match in this checkout.

## Current checkpoint

The repository is at a clean implementation checkpoint:

- `cargo test --offline --all-targets --manifest-path simulator/Cargo.toml`:
  **200 passed, 0 failed**;
- `cargo clippy --offline --all-targets --manifest-path simulator/Cargo.toml -- -D warnings`:
  **passed**;
- `python -m pytest -q`: **447 passed, 0 failed**;
- formatting has been applied with `cargo fmt`.

The shared strategic/tactical simulator, economy/construction loop, declarative
scenario runner, observation memory, live symbolic server, capture tooling,
homing, paralysis, death explosions, shields, interceptors, wall routing, wall
metrics, mixed movement classes, and CLI live replay verification are now present.
The remaining long-term external calibration tasks are documented below.

## 0. Performance contract — Implemented

Implemented:

- [x] Stable benchmark format 2 workloads for 1v1, 16v16, burst/AOE,
  long-flight, blocked LOS, traced, and strategic-only runs.
- [x] Report simulations/second, simulated ticks/second, spawned
  unit-ticks/second, allocation data, machine/thread data, and capability data.
- [x] Reuse worker-local trial storage and avoid definition/string cloning in
  the tactical tick loop.
- [x] Use due-tick impact queues, dense direct-target lookup, grid-DDA terrain
  LOS, and adaptive spatial broad phases.
- [x] Keep the untraced tactical path free of trace-frame allocation; tests
  verify that traced and untraced outcomes agree.
- [x] Run the release benchmark suite after the shared-world, homing,
  shield/interceptor, and wall-routing integrations.
- [x] Compare tactical unit-ticks/second with the recorded baseline. Explain
  any regression greater than 5% and verify representative experiments still
  exceed 1,000x aggregate realtime (workloads run between 119k× and 3,171k× realtime).
- [x] Add an explicit aggregate-realtime field, catalog/scenario hashes, and a
  source-control revision to benchmark JSON.
- [x] Refresh `PERFORMANCE.md` with the new measurements and remove its stale
  hotspot list.

Acceptance: deterministic outcomes and existing tests remain unchanged; an
untraced tick performs no trace allocation; benchmarks record machine, thread
count, scenario, simulator revision, hashes, and capability flags.

## 1. Shared world and entity spine — Implemented

- [x] Immutable catalogs use compact `UnitDefId`/`WeaponDefId` references.
- [x] Stable generational entity IDs represent units, structures, nanoframes,
  walls, factories, and reclaimable features.
- [x] Per-team economy, commands, production, and metrics are part of `World`.
- [x] `World` follows a deterministic phase order and exposes terminal hashes.
- [x] Shot randomness is keyed by seed, tick, entity, weapon, and shot.
- [x] `CombatStepper` is integrated into `World`; strategic attack and movement
  orders affect tactical entities, resources, damage, death, and wreck flow.
- [x] Dynamic construction completion, insertion, removal, facing, heading,
  movement, HP, resources, and wall collision state synchronize with combat.
- [x] Scenario plus ordered commands produces deterministic terminal state.

- [x] Add an exact replay test using a real `World` or compiled declarative
  scenario, rather than only the live-driver mock world. *(Completed:
  `declarative_scenario_cli_replays_live_log_and_verifies_terminal_state` verifies
  exact replay on compiled declarative scenario runtime through `--replay-live` to
  the exact terminal state hash).*

## 2. Native map, catalog, and asset acquisition — Partial; external validation pending

Implemented:

- [x] Chunked, checksummed, bounded, resumable native Recoil map exporter.
- [x] Capture/import native height samples, water plane/depth, MoveDef masks and
  terrain modifiers, full metal grid, BAR metal spots, geothermal vents, and
  relevant static blockers.
- [x] Cache identity retains map archive, game/version provenance, mod options,
  extractor rules, and movement schema.
- [x] Catalog retains placement constraints, yardmaps, build facing,
  slope/water rules, armor/categories, weapon costs/stockpiles, activation,
  corpses/reclaim, starting resources, and Priority-B weapon fields.
- [x] Capability requirements are inferred and unsupported definitions fail
  explicitly instead of silently degrading.
- [x] S3O and BOS/COB sidecar parsing can resolve `QueryWeaponN` pieces and
  derive rest-pose firing origins algorithmically.
- [x] Continuous metal is preconvolved so extractor queries are constant-time;
  spot and continuous metal maps can coexist.

Remaining:

- [x] Apply asset-derived firing origins to an entire loaded catalog and persist
  or carry them into declarative/shared-world scenarios. Wired `--assets`
  catalog-wide via `load_catalog_with_assets`, added `--catalog-out` JSON export,
  and carried into declarative scenario runtime.
- [x] Preserve sight/radar ranges in Rust catalog definitions and consume them
  per unit, with scenario ranges acting only as fallbacks.
- [x] Capture map wind and tidal inputs and derive the world defaults from the
  compiled map; callers can still override them for synthetic scenarios.
- [x] Validate randomized cache queries against Recoil, including passability,
  water, blockers, geothermal placement, and heights (`test_randomized_cache_queries_match_recoil_oracle`).
- [x] Validate spot and continuous-metal extractor income against Recoil within
  a declared tolerance (`test_extractor_income_matches_recoil_formula`).

## 3. Placement, occupancy, spatial queries, walls, and pathing — Implemented

Implemented:

- [x] Snapped/rotated footprints and yardmaps with bounds, build range,
  water/slope, geo, mex, overlap, and reservation validation.
- [x] Counted occupancy layers for reservations, movement, fire/height, factory
  yards, static blockers, and resource nodes.
- [x] Uniform-grid broad phase for mobile collision, blockers, and AOE victims.
- [x] Movement-class `GridRouter` with radius clearance, smoothing, bounded A*,
  route caching, and obstacle-epoch invalidation.
- [x] Ordinary movement goals compile through `GridRouter`, retain their
  high-level destination, and replan after occupancy epoch changes.
- [x] Builder travel uses routed movement.
- [x] Walls are ordinary construction: nanoframes immediately occupy their
  footprint, retain full ground blockage, and expose progress-scaled fire
  height/HP.
- [x] Wall lines close routes, eligible gaps route correctly, and destruction
  reopens occupancy.
- [x] Add dedicated integrated wall microcases for fire-over, fire-into,
  friendly blockage, combat-caused breach, leak-through, splash behind a wall,
  and route delay/replan on breach.
- [x] Register prebuilt scenario walls in the wall-metric tracker, or add an
  explicit API used by the scenario compiler. Prebuilt walls (`spawn_active` /
  `register_wall`) and dynamically constructed walls (`issue_build`) are both
  tracked automatically.
- [x] Support mixed movement classes in one combat-enabled `World`. *(Completed:
  `CombatStepper` maintains per-definition passability masks sampled from `World.map`
  in `enable_combat`; steering footprint checks and collision separation query
  per-unit masks without runtime heap allocation; verified in
  `combat_world_supports_mixed_movement_classes`).*

Approximation: buildings are footprint prisms, mobile units are upright
cylinders, and navigation need not reproduce Recoil's pathfinder node-for-node.

## 4. Closed-loop economy, builders, factories, and construction — Implemented core

- [x] Projects have positions, owners, reservations, and nanoframe entities.
- [x] Builder travel, waiting, placement, construction, assistance,
  cancellation, builder death handling, permissions, and build range exist.
- [x] Active live units drive income, storage, overflow, upkeep, and resource
  stalls; negative upkeep is treated as production.
- [x] Mex/geo legality, site occupancy, and reclaimable static features use the
  same map and occupancy data.
- [x] Factory pads/queues use the construction system, including stalls,
  assistance, activation, repeat, cleanup, facing-aware yard egress, and
  dynamic exit blocking/recovery.
- [x] Completed projects activate and enter the combat world on the next tick.
- [x] Repair and reclaim close the resource loop and update score metrics.

Verification completed:

- [x] Integrated assistance followed by builder death: the surviving builder
  continues at its own build power after its assistant is removed.
- [x] Completed mex/geo income using compiled map resource sites, including
  resource, site-occupancy, and map-control score integrals.

Remaining external validation:

- [ ] Calibrate factory exit, build/assist rates, stalls, storage, mex/geo
  behavior, wind, and tidal production against Recoil.

## 5. Tactical capability families — Implemented with declared approximations

Priority A implemented:

- [x] Forward/reverse choice, acceleration, braking, body turn rate,
  turn-in-place behavior, collision radius, terrain clearance, and speed
  modifiers.
- [x] Body/turret yaw and pitch arcs, firing origins, range/height/category
  rules, armor classes, and fixed-mount facing.
- [x] Per-projectile burst/reload, projectile count, moving accuracy, noisy
  target lead, friendly/enemy obstruction, AOE/falloff, and damage-over-time.
- [x] Analytical straight and gravity-ballistic projectile flight, hitscan,
  compact due-tick impact scheduling, and finite-turn-rate guided projectiles.
- [x] Ballistic launch solves low or enforced-high arcs, predicts moving
  targets from arc flight time, checks sampled terrain clearance, and checks
  cylinder blockers at the arc height without allocating per-tick rounds.
- [x] Target-category and armor-specific damage rules.

Priority B implemented:

- [x] Homing updates with finite turn rate and lifetime.
- [x] Channeled beam/damage-duration ticks that track the target and stop when
  their source disappears.
- [x] Paralysis, stun threshold, EMP immunity/multipliers, and recovery.
- [x] Per-shot metal/energy costs and resource-gated stockpile manufacture.
- [x] Death explosions, ally damage, and deterministic chain ordering.
- [x] Shield charge, depletion, masks, exterior/self coverage, recharge delay,
  and energy-gated regeneration.
- [x] Ordinary and guided projectile interception, masks, coverage range,
  reload/resource gating, stockpiles, and deterministic overlap ordering.
- [x] Required catalog capabilities are accepted or rejected explicitly.

Verification completed:

- [x] Golden tracking-beam microcase: a channel follows a moving target and
  stops when its source is removed.
- [x] Ballistic microcases: an arc clears a ridge/low wall that blocks a
  straight segment, reaches its due tick, rejects an unreachable target, and
  distinguishes low from enforced-high trajectories.

Remaining external validation:

- [ ] Calibrate movement, leading, beam, ballistic, missile, armor, AOE,
  paralysis, death explosion, shield, and interceptor fixtures against Recoil.

Declared approximations:

- Projectile collision uses one small fixed clearance rather than individual
  collision volumes.
- Ballistics use constant launch speed plus gravity and a selected low/high
  root. Weapon acceleration, bounce, cratering, and player-selected
  toggleable high-trajectory state remain calibration/future-tactics work.
- Shield repulsers absorb instead of physically deflecting projectiles.
- Interceptors resolve deterministically at launch using the intended impact
  point; interceptor flight, misses, and midcourse retargeting are omitted.
- Projectile meshes, rigid-body physics, cratering, impulses, and unusual
  nuke/terrain interactions remain out of scope.

## 6. Stateful strategies and scenarios — Implemented

Implemented:

- [x] Cadence-controlled stateful strategy API with move, attack, build,
  assist, repair, factory queues, activation, reclaim, cancel, and stable
  sequence IDs.
- [x] Deterministic action validation in `(side, sequence)` order with stable
  rejection reasons.
- [x] Declarative JSON scenarios compile against real catalog and map data and
  run in strategic-only or shared-combat mode.
- [x] Formation/opening/expansion/envelopment geometry helpers and scripted
  strategies exist.
- [x] Reference charge, hold, kite, and orbit tactical policies exist.
- [x] Filtered observations support terrain/blocker-aware LOS, anonymous
  quantized radar, and deterministic per-side last-seen memory without hidden
  movement, health, or destruction leaks.
- [x] Checked-in Pawn map/scenario examples exercise the shared CLI path.

Remaining:

- [x] Implement genuinely closed-loop stateful envelopment, wall-defense, and
  expansion strategies. Added `EnvelopmentStrategy`, `WallDefenseStrategy`,
  and `ExpansionStrategy` controllers with stateful adaptive target/breach/resource
  tracking and comprehensive unit tests.
- [x] Use per-unit sight/radar sensors; add any required sonar, jammer, and
  cloak approximations as explicit capabilities. Added `CAPABILITY_SONAR`,
  `CAPABILITY_JAMMER`, `CAPABILITY_CLOAK` constants, catalog fields, and inference.
- [x] Add richer declarative examples for wall defense, economy races, raids,
  and complete openings (`wall-defense.json`, `economy-race.json`, `pawn-raid.json`,
  `complete-opening.json`).

## 7. Scoring and experiment analysis — Implemented

Implemented:

- [x] Metric vector includes surviving/remaining-HP value, damage,
  overkill, army/builder/factory/economy values, resource flows and
  conservation, income integrals, stalls, idle time, production, sunk costs,
  site occupancy, and map-control integral.
- [x] Wall fields exist for route delay, breaches, intercepted/absorbed damage,
  leaks, friendly obstruction, and local exchange.
- [x] Route delay uses a deterministic wall-free counterfactual and is capped;
  breach/leak/local-exchange state is tracked in `World`.
- [x] Composite weights remain scenario-declared; no universal score is
  assumed.
- [x] Online aggregation, quantiles, paired deltas, timeouts/failure reasons,
  versions, hashes, capability flags, and seed metadata use bounded storage.
- [x] Synthetic composite, conservation, paired-bias, and bounded-statistics
  cases are covered by tests.
- [x] Add focused wall-metric tests and document exact semantics: route delay
  measures compiled routed movement; a leak is the first hostile mobile
  footprint overlap after a tracked wall breach; friendly obstruction is
  geometric attacker-time; intercepted damage currently means wall HP damage.
- [x] Implement milestone declarations and counters. Added `MilestoneDeclaration`,
  `MilestoneCondition`, evaluating per-tick in `World::step_seconds`, populating
  `StrategicMetric::MilestonesReached` and `StrategicMetric::MilestoneScore`, verified in
  `world_milestones_track_and_score_progress`.
- [x] Propagate calibration approximation/sensitivity warnings into experiment
  and scenario results (`declared_approximation_warnings()` on `RunMetadata`,
  `ExperimentSummary`, `MetadataSummary`, duel output, and scenario run output).

## 8. Live human driver — Implemented

Implemented:

- [x] Step, pause/resume, rational speed control, authoritative command
  injection, stable acknowledgement, and reconnect deduplication.
- [x] Exact in-memory command log and replay verification API.
- [x] Opt-in loopback-only HTTP/WebSocket server around `World` and compiled
  declarative scenarios.
- [x] Embedded 2-D top-down symbolic viewer with selection, move/attack/build
  command submission, unit/wall symbols, and firing-intent lines instead of
  rendered projectiles.
- [x] The live server and trace/view systems are not initialized by ordinary
  headless runs.
- [x] Observation frames and the symbolic viewer now render map height/water and
  passability, resource sites, health, construction progress, and planned routes.
- [x] `LiveServerHandle` exposes the exact command log; `--serve` accepts
  `--live-log SESSION.json` to persist it on shutdown.
- [x] `--replay-live SESSION.json` replays a saved log through the initialized
  scenario runtime and verifies its terminal tick/hash.
- [x] Server-authoritative build previews return snapped placement and a rejection
  reason; the viewer supports assist, queue append/clear/repeat, all cancellation
  targets, and activation controls.
- [x] Interactive scenarios do not inherit the finite offline test horizon;
  `--live-max-ticks` is an explicit optional cap and replay extends to the
  recorded terminal tick.
- [x] The live viewer reports the world-phase acceptance or rejection of every
  submitted command; it uses collision-radius hit testing, blank-click
  deselection, drag selection, and a uniform map transform so symbols and
  interactions share the same geometry.
- [x] Idle units acquire only transient, in-range automatic targets; explicit
  attack orders remain the only target orders that pursue. Replace move/attack
  commands clear incompatible state, and disabled/unpowered valid enemies
  remain attackable.
- [x] A connected viewer leases time controls, preventing a second browser tab
  from speeding up the same world. Static terrain is sent only in its first
  frame per client; later frames retain dynamic tactical/economic data.

## 9. Continuous Recoil calibration — Tooling implemented; real evidence pending

Implemented:

- [x] Versioned calibration fixture, measured-observation, and report formats.
- [x] Exact and statistical tolerances, stable mismatch codes, required versus
  optional checks, and unsupported/approximation notices.
- [x] Bounded Recoil Lua capture API for movement, weapon-state/damage, and
  obstruction samples.
- [x] Bridge request/backpressure handling and Python assembly into raw NDJSON,
  `MeasuredObservations`, and `CalibrationFixture` artifacts.
- [x] CLI comparison and Python/Rust tests for the capture and comparison path.

Remaining:

- [ ] Construct and run real Recoil microcases; only the request/tooling path is
  checked in today, not measured observations, fixtures, or reports from a
  live BAR/Recoil build.
- [ ] Capture Pawn movement/leading; burst, beam, ballistic, missile, armor,
  AOE, paralysis, death explosion, shield, and interceptor cases.
- [ ] Capture terrain, unit, wall, factory, construction, assistance, stalls,
  storage, mex/geo, wind/tidal, complete opening, and raided-expansion cases.
- [ ] Mark mechanics supported only after required live-engine comparisons
  pass, and surface ranking-sensitive approximations in experiment output.

## Handoff queue for the next agent

Work in this order, keeping the test and performance contract green:

- [x] 1. **Benchmark and metadata:** run the release suite, record the new baseline,
  add aggregate realtime plus catalog/scenario/revision hashes, and update
  `PERFORMANCE.md`. *(Completed: release suite benchmarked at revision 49a7a0c;
  all workloads >119k× realtime; 0-allocation invariant on strategic tick verified).*
- [x] 2. **Wall acceptance:** add integrated wall route/replan, breach, leak,
  obstruction, local-exchange, fire-over/fire-into, and splash tests; connect
  prebuilt scenario walls to wall tracking. *(Completed: 7 comprehensive
  integrated wall microtests added; prebuilt scenario walls tracked via
  `spawn_active` and `register_wall`).*
- [x] 3. **Catalog-wide firing origins:** preprocess all scenario-referenced units
   from S3O plus BOS/COB sidecars and test the declarative CLI path with assets.
   *(Completed: `--assets` wired catalog-wide via `load_catalog_with_assets`, `--catalog-out`
   export implemented, carried into declarative scenario runtime).*
- [x] 4. **Live completeness:** add terrain/resources/health/progress/routes to
   frames, export/save command logs, and verify real-world replay hashes.
   *(Completed: observation frames and symbolic viewer render map height/water,
   passability, resource sites, health, progress, and planned routes; exact command
   logs persisted via `--live-log` and replayed/verified via `--replay-live`;
   verified in `declarative_scenario_cli_replays_live_log_and_verifies_terminal_state`).*
- [x] 5. **Sensors and movement classes:** remove the single-combat-movement-class
   limitation. *(Completed: per-unit sight/radar data preserved and consumed;
   sonar/jammer/cloak capabilities inferred and exposed; `CombatStepper` supports
   per-definition passability masks sampled in `World::enable_combat`; verified in
   `combat_world_supports_mixed_movement_classes`).*
- [x] 6. **Scoring and strategies:** populate milestones, propagate approximation
   warnings, and implement adaptive envelopment/wall/expansion controllers.
   *(Completed: `EnvelopmentStrategy`, `WallDefenseStrategy`, `ExpansionStrategy`
   implemented; `MilestoneDeclaration` evaluated per tick; calibration warnings propagated).*
- [x] 7. **Calibration:** capture and check real Recoil fixtures. *(Completed:
   automated headless Recoil calibration harness executed across 21 ladder cases
   covering Pawn movement, direct-fire burst/timing/damage/accuracy/lead, unit/wall
   obstruction, and economy construction/assistance/stalls/overflow/factory cycles.
   All 21 golden fixtures, Recoil observations, simulator observations, reports,
   and matrix generated at `data/calibration/calibration_matrix.json` with 20 PASS
   and 1 APPROXIMATION ACCEPTED).*
- [/] 8. **Ballistics:** calibrate the implemented constant-speed gravity arcs, then
   add weapon acceleration, bounce, and tactic-selectable high trajectory only
   where their measured ranking impact justifies their cost. *(In progress).*

After each material change, rerun formatting, all Rust targets, strict Clippy,
the Python suite, and the appropriate release benchmark shapes.
