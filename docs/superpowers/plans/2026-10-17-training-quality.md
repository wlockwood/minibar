# Training Quality Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement and measure the five approved improvements without changing the active campaign or treating higher reward as proof of stronger play.

**Architecture:** Keep scoring, sampling, brain-schema upgrades, challenger retention and experiment orchestration in separate modules. Native evaluations provide one authoritative report contract for campaigns, workers and Python comparison tools. New experiments initialize independent optimizers from captured champion weights.

**Tech Stack:** Existing Rust/serde/rayon simulator; Python 3.10+ standard-library tooling and unittest; static HTML reports. No new runtime dependency is required.

**Spec:** `docs/superpowers/specs/2026-10-17-training-quality-design.md` (approved 2026-10-17).

## Global Constraints

- Do not stop, rewrite, upgrade, or acquire the exclusive training lock of the active `data/brains/campaign` run. Leave its executable and inputs available.
- New experiments use separate, explicitly named output directories and a new executable under `target/training-quality/release`.
- Missing new configuration fields preserve the old brain, fitness and single-layout evaluation behavior. New quality profiles opt in explicitly.
- Never relax executable/input identity checks to resume an old run.
- Reset optimizer moments and random streams for every independent experiment.
- Keep the 30-second console heartbeat and per-generation summaries.
- Zed remains outside model selection.
- Do not automatically deploy the experiment winner.
- Preserve unrelated work in this shared, dirty checkout. Record starting hashes of touched files. Do not run repository-wide formatting or stage unrelated paths.
- Use the existing execution checkout: the training implementation and fixtures are currently untracked within a larger dirty repository, so a clean HEAD worktree would omit the required source. Inspect repository guidance before changing this decision.
- Checkpoint boundaries are concern-specific and path-limited. Commit only if the relevant files can be staged without absorbing earlier unrelated edits; otherwise retain the verified changes and report the boundary explicitly.

## Review Focus

1. Reclaimed entities, cancelled products and unfinished nanoframes must not become combat kills or completed production value (Task 1).
2. A requested layout count of 1 must preserve legacy seed behavior; larger counts must create genuinely distinct starts without overlapping stage seed bands (Task 2).
3. Upgraded brains must preserve decisions with arbitrarily valued new sensors, not just with zero sensor values (Task 3).
4. A crash after immutable-object writes but before state commit must leave only unreferenced evidence, never an advanced frontier or changed official champion (Task 4).
5. A live source checkpoint changing during experiment preparation must not combine weights, reports or provenance from different committed snapshots (Task 6).

## Task 1: Explicit strategic reward components and trial telemetry

**Files:** Create `src/learn/strategic_reward.rs` and `src/learn/strategic_telemetry.rs`; modify `src/learn/mod.rs`, `src/learn/strategic.rs`; test the new modules and `tests/from_replays_cli.rs`.

**Interfaces:**
- `strategic_reward::RewardConfig`: serde configuration with production mode `count|none|value`, weight 0.02, reference value 100 and optional cap; implements `Default` and `validate() -> Result<(), String>`.
- `strategic_reward::RewardStats`: sufficient statistics for the seven existing score terms and completed production value.
- `strategic_reward::score(&RewardStats, &RewardConfig) -> Result<RewardComponents, String>`; `RewardComponents::total() -> f64` uses legacy addition order.
- `strategic_telemetry::TrialLedger::new(&World) -> Self`, `observe(&mut self, &World)`, and `summary(&self, &World, u8) -> TrialTelemetry` retain entity metadata needed after destruction.
- `strategic::TrainConfig.reward` defaults to legacy; trial reports add reward components and telemetry without removing old fields.

- [ ] Add failing unit cases: `legacy_components_preserve_score` checks the old formula and component sum; `equal_counts_unequal_values` uses two products totaling 100 versus 300 resource-value units and expects value bonuses 0.02 versus 0.06 at reference 100; `none_and_cap` expects zero in none mode and exactly 0.1 for a count bonus capped at 0.1. Reject negative/nonfinite controls and zero reference values.
- [ ] Add real-world fixtures proving one `FactoryReleased` earns one completion, queue/cancel earns zero, initial builders earn zero, reclaim is reported separately from combat destruction, destroyed completed entities are counted once, features/nanoframes are excluded, and unresolved economic trials do not report wins.
- [ ] Run `cargo test --lib strategic_reward` and `cargo test --lib strategic_telemetry`; confirm the new behavior is absent before implementation.
- [ ] Implement the reward module and ledger. Use `WorldEvent::FactoryReleased`, `Reclaimed`, `EntityDestroyed`, completion state and cached entity definitions; do not add hidden information to policy observations. Route strategic rollout scoring through the new module, preserving legacy arithmetic and episode horizon.
- [ ] Run focused tests and `cargo test --test from_replays_cli`; inspect a native strategic evaluation JSON to confirm each trial has the seven terms, production count/value, terminal outcome and separate reclaimed/combat losses.
- [ ] Record the verified Task 1 checkpoint and concern-specific file list.

## Task 2: Explicit layout sampling and a shared native evaluation command

**Files:** Create `src/learn/campaign_sampling.rs`; modify `src/learn/mod.rs`, `campaign_eval.rs`, `campaign_work.rs`, `campaign_promotion.rs`, `campaign_console.rs`, `train.rs`, `strategic.rs`, and `src/main.rs`; extend `tests/campaign_cli.rs` and `tests/learn_cli.rs`.

**Interfaces:**
- `EvaluationPlan.layouts: usize`, serde default 1; `seeds` remains simulator seeds per layout.
- `campaign_sampling::sample_seed(base: u64, layout: usize, seed: usize, layouts: usize, seeds: usize) -> Result<u64, String>` uses checked contiguous blocks and rejects more than 1,000,000 slots per stage.
- `PreparedPlan::evaluate_plan(&Brain, seed_base: u64) -> Result<PlanReport, String>` returns existing stage aggregates plus explicit per-layout reports in stage details.
- Add `--evaluate-campaign BRAIN --campaign-config CONFIG --campaign-track NAME --campaign-plan validation|holdout --eval-seed-base N [--eval-layouts N] [--eval-seeds N] [--against-brain BRAIN]`. It prints one JSON `PlanReport` on stdout; overrides are validated and included in receipt metadata. `--against-brain` replaces the fixed opponent for that call and requires matching brain kind.
- `ScenarioSpec.faction: Option<String>` adds explicit reporting metadata; legacy scenarios need not invent a faction. Quality scenarios require metadata when faction protection is enabled.

- [ ] Add failing cases: `layout_one_matches_legacy`, `separate_layouts_change_starts`, `candidate_incumbent_share_all_samples`, `sample_bands_do_not_overlap`, and `sample_limits_reject_overflow`. Test stage-band boundary 1,000,000 accepted and 1,000,001 rejected, plus u64 overflow.
- [ ] Add an asymmetric tactical fixture demonstrating pooled-row robust fitness differs from mean per-layout fitness. Assert both named outputs independently using hand-calculated row scores. Verify strategic mean and dimension summaries against trial records.
- [ ] Run `cargo test --lib campaign_sampling` and the new focused CLI cases; observe failures before changing evaluation.
- [ ] Implement layout loops. For layouts=1, dispatch through the legacy code path, including its old strategic jitter/seed behavior. For layouts>1, separate layout jitter from combat randomness and expose both identities. Pool tactical tallies before worst-quarter scoring; never average robust scores and label that result as pooled fitness.
- [ ] Route campaign jobs, fresh promotion comparisons and standalone evaluation through the same sampling helpers. Keep job/input fingerprints sensitive to layouts and seed controls; update progress/report counts to distinguish jobs, layouts and simulations.
- [ ] Implement the native command and metadata, including a matching-kind explicit opponent. Test missing tracks/plans, mismatched opponents and malformed overrides before simulations or output mutation.
- [ ] Run `cargo test --test campaign_cli` and `cargo test --test learn_cli`. Compare native direct, coordinator and worker reports for the same two-layout fixture.
- [ ] Record the verified Task 2 checkpoint.

## Task 3: Strategic v2 inputs and behavior-preserving upgrade

**Files:** Create `src/learn/strategic_sensors.rs`; modify `src/learn/mod.rs`, `strategic.rs`, `campaign_eval.rs`, `brain.rs` only where needed, and `src/main.rs`; extend `tests/from_replays_cli.rs`.

**Interfaces:**
- `strategic::KIND_V2`, `specs_for_kind(kind: &str, hidden: usize) -> Result<Vec<ModuleSpec>, BrainError>`, and `template_for_kind(kind: &str, hidden: usize) -> Result<Brain, BrainError>` retain the existing v1 entry points.
- `strategic::upgrade_v1_to_v2(&Brain) -> Result<Brain, BrainError>` maps old input columns by name and appends zero columns; other layers/biases stay unchanged.
- `strategic_sensors::production_inputs_v2(...) -> Vec<f32>` consumes the existing 12 inputs, candidate definition, and perspective-filtered identified contacts. Keep world-private data outside this interface; define its compact contact-feature struct in this module.
- `strategic::TrainConfig.brain_kind` defaults to `strategic_v1`; PreparedPlan and opponent templates select that kind explicitly.
- Add `--upgrade-brain INPUT --brain-kind strategic_v2 --brain-out OUTPUT`, refusing an existing output. Write a sibling provenance receipt with source/output SHA256 and kind/version data. Add `strategic_v2` to `--seed-brain`.

- [ ] Add failing v1/v2 tests with exact old-column preservation and arbitrary nonzero new sensor inputs. Check output scores and chosen actions, then deterministic small-rollout hashes for v1 versus upgraded v2.
- [ ] Add sensor cases for unarmed units, mixed weapon legality, energy/build/range/speed variation, missing identified enemies and visible composition changes. A hidden enemy change must not change inputs. Check spec references 1000/1000/100/500/100 and every finite bound.
- [ ] Run focused tests to observe failure, then implement schema dispatch, normalization, legal direct-ground DPS, named-column upgrade and CLI provenance. Keep hidden width 8 in quality profiles and arbitrary supported widths in upgrades.
- [ ] Ensure campaign archives and explicit comparison opponents reject mixed kinds. Prove old `strategic_v1` fixture files still evaluate and upgraded brains can be reevaluated under the audited reward.
- [ ] Run `cargo test --test from_replays_cli` and focused strategic tests; record the verified Task 3 checkpoint.

## Task 4: Durable diagnostic frontier and faction protection

**Files:** Create `src/learn/campaign_frontier.rs`; modify `src/learn/mod.rs`, `campaign.rs`, `campaign_io.rs`, `campaign_promotion.rs`, `campaign_console.rs`, and `scripts/campaign_dashboard.py`; extend `tests/campaign_cli.rs` and `scripts/test_campaign_dashboard.py`.

**Interfaces:**
- `FrontierConfig { limit: usize, initial_dimension_slack: f64 }`, default limit 8 and slack 0.05 when enabled; legacy configuration leaves retention disabled.
- `FrontierState { version: u32, ... }` starts at version 1, owns retained weights/reports/provenance and implements `consider(...) -> Result<FrontierDecision, String>` with validated stage/faction dimensions.
- `TrackState.frontier: Option<FrontierState>` defaults absent. Keep the existing outer checkpoint format; version the new subsystem explicitly. Derived immutable objects are named by SHA256.
- `TrackConfig.reference_brain: Option<PathBuf>` defaults absent; quality experiments populate it with the captured source's original reference, upgraded for matching kind. Evaluate and checkpoint a separate reference brain/report for frontier eligibility and faction protection; do not replace the warm-start optimizer base or relabel its existing initial report. Resolve/package/hash the reference dependency like other brain inputs.
- Add optional campaign `max_faction_regression`, default absent, quality value 0.05. Apply initial-reference protection on each required stage/faction in both fixed and fresh promotion comparisons.

- [ ] Add failing cases for domination, nondomination, duplicate brains, deterministic ordering, missing required faction dimensions and a candidate that improves mean score while failing one faction.
- [ ] Add the generation-5-shaped tradeoff fixture: candidate scores easy=0.289555, mixed=0.511358, objectives=0.607017, versus specialist 0.159253/0.330292/0.809754. With suitable per-faction reference data, assert official rejection can coexist with diagnostic retention and unchanged official champion hash.
- [ ] Pin pruning: dimensions sorted by stage then faction; constant dimensions contribute zero distance; prefer unused dimension-extreme representatives in that order until capacity, then maximize minimum normalized Euclidean distance. Prefer higher aggregate fitness then ascending hash on ties. Persist bounds and decisions, and test input-order invariance and limit overflow.
- [ ] Run focused frontier tests to see them fail; implement nondomination, initial-reference eligibility and bounded deterministic retention. Preserve evaluated sampled brain payloads before job cleanup.
- [ ] Write immutable frontier brains/reports before checkpoint commit. Add crash fixtures for missing derived exports and unreferenced precommit objects; reconstruct from authoritative checkpoint payloads and never adopt unreferenced objects. Partial generation retains committed frontier unchanged.
- [ ] Extend uninterrupted/resumed/external-worker equivalence tests to include frontier state and faction guard evidence. Update summary, console and dashboard with official-versus-diagnostic labels and optional-field compatibility.
- [ ] Run `cargo test --test campaign_cli` and `python -m unittest discover -s scripts -p test_campaign_dashboard.py`; record the verified Task 4 checkpoint.

## Task 5: Reproducible read-only comparison and reward audit tooling

**Files:** Create `scripts/training_compare.py`, `scripts/training_report.py`, `scripts/test_training_compare.py`; modify `scripts/branch_selection.py` to share the authoritative comparison contract without changing old import defaults.

**Interfaces:**
- `training_compare.compare(manifest: Path, out: Path) -> dict` validates brain/config/executable identities, invokes the native command, saves raw reports and returns paired summaries. Fixed roster entries name explicit opponent brains/configs.
- `training_compare.rescore(report: dict, reward: dict) -> dict` requires sufficient reward statistics and rejects unsupported older reports for value mode.
- `training_compare.summarize_pairs(candidate: list, reference: list, bootstrap_seed: int) -> dict` performs 20,000 paired bundle resamples and returns named point estimates, intervals and pair counts.
- `training_report.render(assessment: dict) -> str` produces a standalone HTML report with stage/faction/component tables, source hashes and limitations; no network assets.

- [ ] Add tests with literal arithmetic: gains [1,1,1,1] have interval [1,1]; identical pairs yield zero gain and all ties; deterministic seed reproduces a nonconstant bootstrap. Assert a mixed reward, opponent, layout or brain-kind comparison is refused before publication.
- [ ] Test rescore count/none/value/cap values from Task 1; older missing production-value data must cause a reevaluation-needed error. Tests distinguish pooled robust tactical results from mean bundle results and never bootstrap individual battles as independent models.
- [ ] Run `python -m unittest discover -s scripts -p test_training_compare.py`; confirm failures, implement tools, then rerun against real tiny native tactical and strategic fixtures. Upgrade v1 weights explicitly before common-v2 comparison.
- [ ] Exercise named fixed opponents and missing faction metadata. Report unavailable dimensions rather than inferring labels from scenario names. Escape arbitrary labels when generating HTML.
- [ ] Reevaluate captured reference/champions on a small development-layout set to verify end-to-end output. Keep final 24-layout seeds unused until Task 7.
- [ ] Record the verified Task 5 checkpoint.

## Task 6: Independent experiment matrix and quality profiles

**Files:** Create `scripts/training_experiments.py`, `scripts/test_training_experiments.py`, `examples/campaign/quality/experiment.json`, `quality/campaign.json` and its stage configs; modify `scripts/package_training.py`, `scripts/test_package_training.py`, `scripts/training_branch.py`, `scripts/test_training_branch.py`, and `scripts/training_report.py` as needed.

**Interfaces:**
- `training_experiments.snapshot_source(run: Path, out: Path) -> dict` reads one state byte payload without acquiring the live lock, validates it, writes that exact payload and derives champions/provenance from it once.
- `training_experiments.prepare(manifest: Path, out: Path) -> dict` creates immutable arm configs and seeds. `run(out: Path) -> dict` executes/resumes the declared matrix; `compare(out: Path) -> dict` uses Task 5. CLI subcommands match these names.
- Experiment directory contains `experiment.json`, `source/`, `arms/ARM/seed-SEED/`, `comparisons/`, `progress.json` and `report.html`; existing output is refused except for explicit resume.
- Add five arms exactly as approved: legacy/v1; no-count/v1; no-count/v2; no-count/v2 with sigma 0.02/0.08; no-count/v2 with tactical engagement/mission and strategic production trainable.

- [ ] Add a controlled file-read race test: swap source state after the captured read, then assert state hash, both champions, original reference brains and generation all originate from the first payload. Do not use a live-run exclusive lock and do not combine repeated state reads. Assert generated arms retain original references separately from champion optimizer initialization.
- [ ] Add manifest tests asserting three independent training seeds per arm, paired across arms; shared starting behavior and work budgets; fresh optimizer state; arm 4 only changes sigma and arm 5 only changes masks; all non-treatment config differences are limited to names, paths and lineage.
- [ ] Test failures/stops remain in reports, uncommitted generations remain incomplete, source/output directories are protected, and retries preserve arm/seed identity rather than rerolling randomness.
- [ ] Run `python -m unittest discover -s scripts -p test_training_experiments.py` to confirm missing functionality, then implement manifest generation, execution, resume and receipts. Report matrix progress at least every 30 seconds even when a child emits no output.
- [ ] Set quality validation to 4 layouts x 2 simulator seeds. Pilot uses one full generation, population 2 per island, one training layout x one simulator seed, full horizons, all stages/factions and the same evaluation roster. Derive explicit distinct generation/training/development/final seed bands and record them in the manifest.
- [ ] Keep branch transfer's strict old backend checks. The new experiment workflow records source backend versus new evaluation backend as a deliberate weights-only transition; it never imports old optimizer/cache state or bypasses old checks. Test old package and branch flows unchanged.
- [ ] Run packaging, branch-transfer and experiment unit suites; run a tiny all-arm smoke matrix. Inspect the rendered HTML for treatment names, incomplete results and the distinction between legacy diagnostics and audited comparison scores.
- [ ] Record the verified Task 6 checkpoint.

## Task 7: Integrated release, full-horizon pilots and delivery

**Files:** Update `docs/TRAINING_CAMPAIGNS.md`, create `docs/TRAINING_EXPERIMENTS.md`, and add a short link in `examples/campaign/README.md`. Store evidence beneath one experiment directory under `data/experiments/training-quality-20261017/` (choose an unused suffix if necessary).

- [ ] Run `cargo test` and `python -m unittest discover -s scripts -p "test_*.py"`. Record unrelated failures by name; do not silently omit failed suites. Run scoped `rustfmt --check` and whitespace checks for changed files.
- [ ] Build `cargo build --release --target-dir target/training-quality`. Keep the old locked release executable intact. Run release native comparison, schema-upgrade, cooperative-stop/resume and relocated package/worker probes against this binary.
- [ ] Prepare the 15-run pilot matrix from one current committed champion snapshot, retaining the starting reference and captured champions. Run every arm/seed to one committed full-horizon generation. Record elapsed time, evaluations and any failure; surface progress and estimate remaining time from actual completed runs.
- [ ] Use the same audited profile to compare each distinct resulting brain, reference and captured champion on 24 fresh layouts x one simulator seed. Content-addressed cache keys include the entire comparison profile, roster, seeds, brain and executable; prove a changed input invalidates reuse. Reuse exact duplicates only.
- [ ] Generate and visually inspect `report.html`: paired gains, faction/stage outcomes, reward decomposition, run-level variability, costs, failures, and conditional uncertainty must be visible. Do not call one-generation pilots proof of optimal settings or long-run strength.
- [ ] Run one fresh code review over the final changes; address material findings and rerun the checks affected by any fixes. Do not change training behavior based on final evaluation and then reuse those same samples as untouched holdout.
- [ ] Document runnable prepare/run/resume/compare commands, output paths, actual results and a measured follow-up budget. Confirm the original run inputs/executable were not modified; its state may legitimately have advanced while training continued.
- [ ] Deliver the implementation, verified executable path, report and any remaining failures/limitations. Leave active experiment and best-brain selection explicit; do not deploy the winner or schedule further training.

## Execution and coverage review

Tasks 1–4 implement scoring, sampling, schema and retention respectively. Task 5
provides the shared evaluation/audit evidence. Task 6 defines the controlled
experiments and portability; Task 7 runs and validates the approved deliverable.
All five Review Focus cases are assigned tests. Every cross-task type and command
is defined above before its consumer. The spec's seed counts, sigma values,
normalization references, frontier limit, bootstrap count and pilot size remain
unchanged.

Recommended execution: native implementation in this session, followed by one
independent review. Most changes share evaluation/report contracts, so doing the
implementation in one context should reduce integration overhead. User review of
this plan and selection of execution method precede implementation.

## Execution result (2026-10-18)

Implementation and bounded experimental study completed. All 15 full-horizon pilots and 15 smoke pilots committed; final comparisons use 24 paired layouts and both opponent entries. All 397 Rust tests and 91 Python tests pass, with final release relocation/worker/recovery probes and browser inspection. The current campaign was preserved and no winner deployed. See [measured assessment](../../../data/experiments/training-quality-20261017/ASSESSMENT.md) and [operator guide](../../TRAINING_EXPERIMENTS.md). The execution ledger records shared-checkout rulings, pinned pilot versus final evaluator provenance, concurrency adjustment, and additional recovery/Windows status-write regressions.
