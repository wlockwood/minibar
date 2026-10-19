# Replay informed baseline brains implementation plan

> For agentic workers: use the implementation and verification skills task by task.

**Goal:** Save runnable, trainable tactical and strategic baseline brains in
`data/brains/from_replays`, convert real local test-map data, and run a measured
short training trial with reproducible results.

**Architecture:** Preserve existing tactical_v3 brains. Add a research-informed
tactical_v4 with objective/capability inputs and learned mission drive, and a
strategic_v1 controller that uses the existing economic/construction World and
generic evolution strategy. Share versioned compiled maps across trials. Author
baseline weights from the research; do not describe them as imitation-trained.

**Tech stack:** Existing Rust/serde/rayon runtime and Python asset preparation.

**Spec:** [Sensor research](../../MLP_SENSOR_RESEARCH.md) and the user's explicit
implementation request, including short training after assets and simulation work.

## Constraints and rulings

- Work in the current dirty checkout: the training code and research are largely
  untracked, so a clean worktree would omit the user's actual working baseline.
  Preserve concurrent edits and do not commit unrelated files.
- The user explicitly requested implementation of the researched design. Continue
  through implementation and trials without inserting another approval checkpoint.
- Economy/construction already exist; extend telemetry/controller integration,
  rather than replacing resource allocation or construction state machines.
- No native BAR gameplay/balance changes. Do not write inside the installed game.
- Every brain's labeled inputs must actually be consumed by its runnable policy.
  Unsupported game domains remain disclosed; no invented air/naval parity.
- Unknown enemy state is not zero; strategic enemies come from filtered observations.
- Record map acquisition fidelity, version/source hashes and approximation limits.

## Review focus

- Old tactical_v3 files still load and preserve their existing wiring.
- Objective/capability sensors affect actual choices, rather than decorative JSON.
- Busy builders and factories are not reset every decision tick.
- Requested resource demand remains visible when actual spending is starved.
- Train/evaluation seeds differ; report baseline, latest and retained best separately.

## Task 1 Tactical brain and mapped combat arenas

Files: `src/learn/tactical.rs`, `src/learn/train.rs`, focused tests.

- [x] Add regression tests for preserved v3 loading, v4 objective preference,
  incompatible-target rejection and changed mission weights changing behavior.
- [x] Add tactical_v4 specs/template, compatibility dispatch in TacticalPolicy,
  objective/relative-speed/weapon-eligibility sensors and a learned mission module.
- [x] Add `TrainConfig.brain_kind` defaulting to tactical_v3. New configs select v4.
- [x] Support optional compiled map, movement class and cell size in training;
  materialize terrain outside battle rollouts and place formations legally.
- [x] Run focused tactical/training tests including map-backed scenarios.

## Task 2 Trainable strategic controller

Files: new `src/learn/strategic.rs` and optionally a focused helper module. Parent
owns registration and CLI changes in `src/learn/mod.rs` and `src/main.rs`.

Interfaces: `template(hidden: usize) -> Result<Brain, BrainError>`;
`TrainConfig::load(&Path)`; `train(&TrainConfig, &UnitCatalog, &Path)` returning a
serializable summary; controller accepts validated strategic_v1 Brain.

- [x] Test demand/expense distinction, legal construction, persistent builder work,
  factory production, fog-limited targeting and save/reload/train compatibility.
- [x] Implement named economy-investment, production and mission candidate scorers
  using actual World economy/construction/commands, with exact own telemetry and
  observation-filtered enemies. Do not query hidden occupancy as an input oracle.
- [x] Reuse EvolutionStrategy; evaluate mirrored sides and held-out seeds; save
  initial/best/latest brains, history, resolved config and before/after metrics.
- [x] Exercise the real economic model, completed construction and production.

## Task 3 Assets and reproducible package

Files: `scripts/prepare_from_replays.py`, tests, `data/brains/from_replays/**`.

- [x] Prefer existing runtime-resolved catalog/map captures; otherwise convert local
  replay grids or map archives with explicitly labeled approximate passability.
- [x] Provide at least two real test maps plus a deterministic small fixture.
- [x] Save source/acquisition hashes, movement assumptions and starts; compile caches.
- [x] Generate all three factions' supported baseline curricula/catalog data, retaining
  economic build options and strategic fields rather than flattening into combat DPS.
- [x] Export tactical/strategic templates, run configs and repeatable commands.

## Task 4 CLI, trial and report

Files: `src/main.rs`, `src/learn/mod.rs`, CLI tests, package README/results.

- [x] Expose template export and strategic train/evaluate commands without breaking
  the existing tactical train/evaluate/watch paths.
- [x] Run formatting, relevant Rust/Python tests, and full release tests.
- [x] Run at least three generations per runnable baseline kind with bounded initial
  populations, fixed validation scenarios/seeds and wall-time measurements.
- [x] Record generation histories, checkpoint hashes and baseline/best/latest results.
- [x] Document scale-test commands separately from the initial bounded smoke trial;
  distinguish approximate simulator improvements from full BAR performance.
