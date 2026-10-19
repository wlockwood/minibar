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

- [ ] Write and run failing arithmetic, nearest-count, cap, anchor and family coverage tests.
- [ ] Implement resource packing, anchored and budgeted families, static/air pools and deterministic metadata.
- [ ] Export expanded local BAR definitions with source identities and explicit capability checks.
- [ ] Run generator and exporter tests, including Mammoth/40 Pawns and Starlight/Banisher.

### Task 2: Flight mechanics and separate air policy

Files: `src/model.rs`, `src/engine.rs`, `src/policy.rs`, `src/learn/air.rs`, tactical dispatch and combat tests.
Interfaces: `UnitDef.flight: Option<FlightDef>`; air kind `tactical-air-v1`; `AirPolicy` and `DomainPolicy` route independently learned policies.

- [ ] Write and run failing terrain-bypass, turning, altitude targeting, bomb/AA and policy separation tests.
- [ ] Implement bounded flight, targeting/projectile geometry and the independent air policy.
- [ ] Verify inherited ground behavior and both air/air and mixed air/ground fixtures.

### Task 3: Optimizer reset correction

Files: `src/learn/es.rs`, `src/learn/campaign.rs::recentred`.
Interfaces: additive optional checkpoint `adam_updates`, absent means legacy generation; recenter sets zero without resetting noise generation.

- [ ] Write and run failing reset-clock and resume tests.
- [ ] Implement separate correction clock and validate serialized state.
- [ ] Run ES and campaign recenter tests.

### Task 4: Evaluation, curriculum preview and mixed control integration

Files: `src/learn/train.rs`, `src/learn/campaign_eval.rs`, `src/main.rs`, `src/learn/mod.rs`, CLI tests.
Interfaces: training config optionally names a frozen companion brain; native preview uses the same seed/generator as evaluation. Evaluation rows expose families and actual investments.

- [ ] Test generated layouts with differing row identities, mixed policy routing and explicit preview metadata.
- [ ] Add companion loading/identity, air seed initialization and preview command.
- [ ] Pool generated rows without positional identity assumptions and carry family results through reports.
- [ ] Align configured opponents across train, screen and promotion; reject accidental changing archive use for the successor.

### Task 5: Campaign recovery, retention and dashboard

Files: `src/learn/campaign.rs`, campaign configuration, `scripts/campaign_dashboard.py`, HTML template and tests.

- [ ] Test stagnation recovery, evaluated candidate retention and family reporting.
- [ ] Add bounded champion reanchoring and preserve rejected candidate weights/lineage.
- [ ] Surface each brain's family coverage, outcomes and investment gaps; lay-review the changed display.
- [ ] Produce successor configs with family stages, ratio-discounted scoring, gentle structural refinement and unchanged reference anchors.

### Task 6: Integrated verification and pilot packaging

Files: successor artifacts and user documentation.

- [ ] Run focused Rust/Python tests, formatting and source capability checks.
- [ ] Independently review integrated changes and address material findings.
- [ ] Generate preview records and watchable combat samples.
- [ ] Run bounded fresh-seed pilots without touching the completed campaign; report evidence and limitations.
- [ ] Package the validated successor with executable/config identities and a visible 32-thread launcher.

## Execution record

The user explicitly requested implementation of the approved design. Proceed in this session without another approval checkpoint. Independent generator, flight and optimizer tasks are delegated under the parallel-agent skill; integration stays with the primary agent.
