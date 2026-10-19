# Training Campaign Implementation Plan

> For agentic workers: use subagent-driven development and focused tests.

**Goal:** One command trains replay-informed brains through ordered easy-to-hard
stages inside every generation, with multiple search scales and resumable state.

**Architecture:** A native Rust campaign coordinator owns independent per-track
ES islands. Every candidate is evaluated on each curriculum stage, in ascending
difficulty, before a single population update. Typed, versioned evaluation work
units support local execution and a standalone worker command. Relative package
assets and a manifest make the campaign movable for later distributed execution.

**Tech stack:** Existing Rust/serde/rayon and Python standard-library packaging.

## Intent, scope and rulings

- User requests implementation of the previous training recommendations, one
  command and intra-generation complexity progression. Portable/distributed
  training is an eventual goal: implement a portable bundle and worker interface,
  with local orchestration now; no network scheduler or cloud deployment.
- The explicit implementation request authorizes necessary reversible work;
  do not insert skill approval checkpoints or commit a shared dirty checkout.
- Keep current checkout because most of the existing implementation is untracked.
  Preserve others' files and old trainer/brain compatibility.
- Separate tactical and strategic tracks remain separate controllers. Do not claim
  joint learned execution, flight fidelity or support for every catalog entry.
- Use staged assessment (one update after all stages), rather than sequentially
  overwriting weights on easy then hard cases and forgetting easy behavior.
- Preserve common conditions per candidate within a stage; stage weights are
  explicit and independent of battle counts. No fitness-based skipping of hard
  stages. Promotion uses fixed validation, while a separate holdout stays out of
  selection. Log stage fitness, outcomes and validation decisions.
- Store actual ES theta, Adam moments, generation, sampling configuration,
  champions, restart counters and histories. Resume must reproduce uninterrupted
  generations and reject changed inputs. Existing init_from remains a fresh start.
- Larger mutations and module restarts affect exploratory islands; they cannot
  overwrite a champion without validation. Do not combine different noise scales
  into an incorrectly normalized single ES population.

## Tasks and ownership

- [x] Optimizer: extend `src/learn/es.rs` with deterministic ask/tell and versioned
  validated serializable state. Preserve step() results. Tests for exact restart,
  reordered external evaluations and invalid-result rejection.
- [x] Evaluator: new `src/learn/campaign_eval.rs`, ordered typed stage plans for
  tactical/strategic configs, config-relative paths, deterministic evaluation and
  JSON stage reports. Tests for ordering, weights, consistency and invalid input.
- [x] Coordinator: new `src/learn/campaign.rs` plus focused state/work helpers,
  native CLI in `src/main.rs`, tracks/islands, stage barriers, restarts, archive,
  validation/promotion, input fingerprint and transactional generation checkpoint.
  Versioned standalone evaluation work units and worker CLI.
- [x] Portable assets: default staged curricula/config under `examples/campaign/`,
  standard-library package script with relative-path rewriting, dependency hashes,
  manifest and instructions. Verify moved package needs no BAR source or checkout.
- [x] Verify: focused Rust/Python tests, release suite, real bounded multi-track
  campaign, interrupted/resumed equality, moved-bundle training/worker smoke, scoped
  code review and runnable documentation. Report measured results and limits.

## Review focus

1. Easy stages remain in every generation and no candidate gets easier treatment.
2. Saved champions never inherit unvalidated restart weights or optimizer means.
3. Resuming preserves optimizer and sampling state; changed maps/configs fail.
4. Worker result identity, dimensions, finite values and duplicate/stale IDs are
   checked before optimizer updates; a failed evaluation does not become a score.
5. Packages contain every catalog/map/brain/opponent dependency, avoid absolute
   paths, and fail when an input cannot be bundled; no silent source fallback.

## Completion and review rulings

- All tasks completed. Six native CLI tests establish full resume/external-worker
  parity, moved-config addressing, input/settings rejection and stale holdout
  removal. Semantic content identities survive relative-path package rewriting.
- Compiled VSGMAP data is required for campaign maps to exclude implicit map
  acquisition sidecars from the dependency graph. Old training paths retain their
  existing map support.
- Validation cadence is first generation plus fixed intervals; it does not depend
  on the requested stopping generation. This preserves exact extension behavior.
- Per-stage spread is logged; automatic score standardization is deferred because
  it would change the meaning of the authored objective weights.
- Source reviews found and fixed archive seed aliasing, incomplete checkpoint
  settings validation, implicit map sidecars and stale holdout exports. CLI tests
  found and fixed f64 JSON round-trip drift. Claude's curriculum review led to
  actual attacking opponents, imbalances and distinct validation cases.
- Two full-horizon bounded generations completed in 190.66 seconds; retained
  validation and one-seed holdout improved for both tracks. Results and limits:
  examples/campaign/RESULTS.md. Portable default ZIP is
  data/packages/replay-informed.zip. Networking and cross-host determinism remain
  outside this implementation; native standalone worker transfer is verified.
