# Resource-balanced training implementation plan

> **For agentic workers:** Use superpowers:executing-plans to implement the integration tasks and superpowers:dispatching-parallel-agents for the independent simulator and generator tasks. Track steps below.

**Goal:** Train preserved champions on diverse resource-balanced armies with separate ground and air policies and honest promotion evidence.

**Architecture:** Extend the seeded matchup generator with explicit families and anchored comparisons. Keep flight mechanics and air policy separate from ground movement and policy, then route them together only during mixed fights. Campaign stages protect families, retain candidates and use the same explicit opponent policy throughout evaluation.

**Tech Stack:** Rust, serde, Rayon; Python standard-library catalog and dashboard tools.

**Spec:** `docs/superpowers/specs/2026-10-18-resource-balanced-training-design.md`

## Global constraints

- Use metal + energy / 60 and report actual integer-army imbalance.
- Preserve the completed campaign, frozen executable, champions and optimizer state.
- Keep independent air and ground input contracts, weights, structure, optimizer state and promotion.
- Candidate/incumbent comparisons use identical generated fights and mirrored sides.
- Unsupported capabilities must fail explicitly; do not claim native BAR fidelity.
- Keep training visible in a terminal; use 32 logical processors for successor pilots.
- Preserve unrelated changes in this shared dirty checkout; no automatic commits.

## Review focus

- A swarm exceeding a unit cap must report its remaining imbalance or fail explicitly.
- Flight must bypass blocked ground while ground units retain collision/path rules.
- Mixed-domain validation must keep the companion brain frozen and identified.
- Fresh generated compositions may change row names across layouts: pooling must preserve rows rather than silently zip unequal identities.
- Adam recentering must reset its correction clock while preserving future sampling streams.

### Task 1: Resource-balanced composition generation and catalog

Files: `src/learn/matchups.rs`, source-backed catalog/preview scripts and tests.
Interfaces: `generate_records(config, catalog, seed)` returns scenarios and realized investment metadata; `generate` remains the scenario-only wrapper. Aircraft use `UnitDef.flight`.

- [x] Write and run failing arithmetic, nearest-count, cap, anchor and family coverage tests.
- [x] Implement resource packing, anchored and budgeted families, static/air pools and deterministic metadata.
- [x] Export expanded local BAR definitions with source identities and explicit capability checks.
- [x] Run generator and exporter tests, including Mammoth/40 Pawns and Starlight/Banisher.

### Task 2: Flight mechanics and separate air policy

Files: `src/model.rs`, `src/engine.rs`, `src/policy.rs`, `src/learn/air.rs`, tactical dispatch and combat tests.
Interfaces: `UnitDef.flight: Option<FlightDef>`; air kind `tactical-air-v1`; `AirPolicy` and `DomainPolicy` route independently learned policies.

- [x] Write and run failing terrain-bypass, turning, altitude targeting, bomb/AA and policy separation tests.
- [x] Implement bounded flight, targeting/projectile geometry and the independent air policy.
- [x] Verify inherited ground behavior and both air/air and mixed air/ground fixtures.

### Task 3: Optimizer reset correction

Files: `src/learn/es.rs`, `src/learn/campaign.rs::recentred`.
Interfaces: additive optional checkpoint `adam_updates`, absent means legacy generation; recenter sets zero without resetting noise generation.

- [x] Write and run failing reset-clock and resume tests.
- [x] Implement separate correction clock and validate serialized state.
- [x] Run ES and campaign recenter tests.

### Task 4: Evaluation, curriculum preview and mixed control integration

Files: `src/learn/train.rs`, `src/learn/campaign_eval.rs`, `src/main.rs`, `src/learn/mod.rs`, CLI tests.
Interfaces: training config optionally names a frozen companion brain; native preview uses the same seed/generator as evaluation. Evaluation rows expose families and actual investments.

- [x] Test generated layouts with differing row identities, mixed policy routing and explicit preview metadata.
- [x] Add companion loading/identity, air seed initialization and preview command.
- [x] Pool generated rows without positional identity assumptions and carry family results through reports.
- [x] Align configured opponents across train, screen and promotion; reject accidental changing archive use for the successor.

### Task 5: Campaign recovery, retention and dashboard

Files: `src/learn/campaign.rs`, campaign configuration, `scripts/campaign_dashboard.py`, HTML template and tests.

- [x] Test stagnation recovery, evaluated candidate retention and family reporting.
- [x] Add bounded champion reanchoring and preserve rejected candidate weights/lineage.
- [x] Surface each brain's family coverage, outcomes and investment gaps; lay-review the changed display.
- [x] Produce successor configs with family stages, ratio-discounted scoring, gentle structural refinement and unchanged reference anchors.

### Task 6: Integrated verification and pilot packaging

Files: successor artifacts and user documentation.

- [x] Run focused Rust/Python tests, formatting and source capability checks.
- [x] Independently review integrated changes and address material findings.
- [x] Generate preview records and watchable combat samples.
- [x] Run bounded fresh-seed pilots without touching the completed campaign; report evidence and limitations.
- [x] Package the validated successor with executable/config identities and a visible 32-thread launcher.

## Execution record

The user explicitly requested implementation of the approved design. Proceed in this session without another approval checkpoint. Independent generator, flight and optimizer tasks are delegated under the parallel-agent skill; integration stays with the primary agent.


Implementation verification:

- 480 Rust tests and 156 Python tests pass; formatting and diff checks pass.
- Both final pilots completed three generations; Windows PowerShell completed-run resume preserves their full checkpoint bytes.
- Ground promoted a candidate at generation 2 in both pilots. These short, differently scoped pilots verify the pipeline and do not establish sustained or native BAR improvement.
- Final main checkpoint is 10,621,854 bytes. Full receipt/cache evidence remains available; compaction preserves exactly the prior pilot champions and optimizer states.
- Contact starts were tightened to 200–320 after replay inspection found stationary fights beyond both armies' sight. Actual ground and air samples contain decisions and shots.
- Fixed-wing turn rates use the source speed/turn-radius approximation. Air has independent inputs, weights, structure and promotion; companions remain frozen for reproducible comparisons.
- Production family stages target near equal investment; lightly weighted advantage/disadvantage stages retain unequal-force coverage.
- Verification receipt: `data/branches/diverse-training-20261018/verification.json`. Production has not been started; holdout remains unused.
