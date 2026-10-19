# Evolving Brain Structure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Evolve activation functions, active sensors, and active connections from existing champions, and make that transformation understandable in the campaign dashboard.

**Architecture:** Keep named modules, layer sizes, and output contracts stable. Store structural masks and hidden-neuron activations in each brain; each campaign island owns its full brain and optimizer. Structural proposals inherit champion parameters, train independently, and use the existing promotion checks.

**Tech Stack:** Rust, serde/serde_json, existing evolution-strategy trainer, Python standard library, standalone HTML/JavaScript/SVG dashboard.

**Spec:** [Approved design](../specs/2026-10-18-evolving-brain-structure-design.md)

## Global Constraints

- Outputs retain tanh and their existing meaning and range.
- Leaky ReLU (fixed negative slope 0.01).
- Keep layer widths, module names, output names, and available sensor order fixed for each brain kind.
- Default behavior remains existing weight-only training.
- Holdout remains a report, never a selection signal.
- Preserve existing campaign directories and independent branch state.
- Preserve dormant weights; exclude disabled parameters from perturbations, updates, decay, and regularization.
- Do not guess the authoritative champion by file modification time.
- Do not rewrite historical exported artifacts.
- Keep the standard-library-only generator and self-contained offline dashboard.
- No new product dependencies. Work in the existing dirty checkout only with explicitly scoped edits; do not stage unrelated or previously untracked files wholesale. Commits require a reviewable diff of this concern.

## Review Focus

1. Legacy brain loading must preserve numerical behavior and not invent historical structural events (Tasks 1, 5).
2. Disabled weights must remain unchanged even with existing optimizer moments and nonzero decay/L1 (Task 2).
3. Stopping after proposal creation must not repeat or lose an edit on resume (Task 3).
4. A rejected structural experiment must leave the incumbent and immutable reference unchanged (Task 3).
5. Portable imports must preserve complete structure/history and reject altered or stale artifacts (Task 4).

## File responsibilities

- Create `src/learn/structure.rs`: activation definitions, structural metadata validation, connectivity, parameter activity.
- Create `src/learn/structural_search.rs`: deterministic legal mutations and mutation descriptions.
- Modify `src/learn/mlp.rs`: execute and trace structural brains.
- Modify `src/learn/brain.rs`: versioned persistence and full-brain parameter activity.
- Modify `src/learn/es.rs`: masked parameter search preserving dormant state.
- Modify `src/learn/campaign_io.rs`: opt-in structural-search settings and validation.
- Modify `src/learn/campaign.rs`: island-owned brains, mutation lifecycle, checkpoints, snapshots, lineage.
- Modify `src/learn/mod.rs`: module registration.
- Modify `src/learn/explain.rs`, `src/learn/watch.rs`, `viewer/brain_watch.html`, `watch-objectives.html`: explain actual structural inference.
- Modify `scripts/training_branch.py`, `scripts/branch_transfer.py`, and `scripts/branch_selection.py` only where integration requires structural settings, validation, or preserved snapshots.
- Modify `scripts/campaign_dashboard.py` and `viewer/campaign_dashboard.template.html`: extract, compare, and visualize structure and events.
- Extend existing focused Rust/Python tests; create `scripts/test_structural_dashboard.py` if structural fixtures make the existing dashboard test file unwieldy.
- Add `examples/campaign/structural-search.json` and `docs/STRUCTURAL_TRAINING.md`: runnable configuration and champion-derived branch workflow.

## Task 1: Executable, self-describing structural brains

**Interfaces**

- `structure::Activation`: serde snake-case `Tanh`, `Softsign`, `LeakyRelu`; `apply(self, value: f32) -> f32`.
- `structure::MlpStructure { inputs: Vec<bool>, connections: Vec<Vec<Vec<bool>>>, hidden: Vec<Vec<Activation>> }`.
- `MlpStructure::dense(sizes: &[usize]) -> Self`; `validate(&self, sizes: &[usize]) -> Result<(), String>`; `parameter_activity(&self, sizes: &[usize]) -> Vec<bool>`; `outputs_reachable(&self, sizes: &[usize]) -> bool`.
- `Mlp::structure(&self) -> &MlpStructure`; `set_structure(&mut self, structure: MlpStructure) -> Result<(), MlpError>`.
- `Brain::parameter_activity(&self, trainable: &[bool]) -> Vec<bool>`, in exactly the same order as `flatten`.
- Module JSON gains `structure`; new writes use brain version 2. Version 1 loads dense/tanh defaults, version 2 requires complete valid metadata. Reject unsupported versions and nonfinite parameters.

- [ ] Add tests: legacy forward result is bitwise unchanged; `Softsign.apply(3.0) == 0.75`; `LeakyRelu.apply(-5.0) == -0.05`; outputs use tanh; disabled input and edge contribute zero; dormant weights survive round trip; trace matches forward; wrong metadata shapes and unknown activations fail.
- [ ] Run `cargo test --lib learn::mlp` and `cargo test --lib learn::brain` with `--target-dir target/structural-verification`; confirm new tests fail for missing behavior.
- [ ] Implement metadata, serialization, inference, and trace. Preserve legacy accumulation order; skip disabled terms rather than changing how enabled sums accumulate. Keep allocation-free normal inference.
- [ ] Test connectivity on multi-output and zero-hidden-layer networks. Every output must have a path from at least one enabled input for structural mutations; legacy loading still accepts existing dense brains with arbitrary weights, including zeros.
- [ ] Run the focused suites plus structure tests. Verify tactical and strategic policy callers keep their existing named input/output contracts.
- [ ] Review the scoped diff; checkpoint the task without staging unrelated files.

## Task 2: Deterministic edits and masked weight training

**Interfaces**

- `structural_search::StructuralSearchConfig { every: u64, activations: Vec<Activation>, activation_weight: f64, input_weight: f64, connection_weight: f64 }`; serde rejects unknown fields, defaults activation choices to all three, cadence to 10, and each frequency to 1.
- `StructuralEdit`: tagged enum describing `Activation { module, layer, neuron, before, after }`, `Input { module, input, before, after }`, or `Connection { module, layer, source, destination, before, after }`.
- `mutate(parent: &Brain, trainable: &[bool], config: &StructuralSearchConfig, seed: u64) -> Result<(Brain, StructuralEdit), String>`.
- `EvolutionStrategy::ask_masked(&self, active: &[bool]) -> Result<Vec<Vec<f32>>, String>` and `tell_masked(&mut self, scores: &[f64], active: &[bool]) -> Result<GenerationStats, String>`. Existing `ask`/`tell` retain their behavior.

- [ ] Add tests that an edit changes exactly one legal setting, copies all parameters exactly, respects frozen modules, never disconnects an output, and is deterministic for a given seed. Reject invalid frequencies, empty trainable sets, and configurations with no possible effective edit.
- [ ] Add optimizer tests with inactive parameters, nonzero old moments, decay, and L1: inactive theta and moments remain exactly unchanged across ask/tell; all-active masked calls match existing calls; bad mask lengths do not mutate state.
- [ ] Run structural-search and optimizer tests to establish failure, then implement both interfaces. Enumerate legal effective edits before weighted selection; select only among mutation categories with available edits. Consume a fixed full-length noise stream so masking does not unpredictably shift other parameters.
- [ ] Run `cargo test --target-dir target/structural-verification --lib learn::es` and the new structural-search tests. Check active-mask offsets against hand-built multilayer brains, including biases and disabled first-layer sensor columns.
- [ ] Review the scoped diff and checkpoint the task.

## Task 3: Champion-derived structural campaign lifecycle

**Interfaces**

- `IslandConfig` gains optional `structural_search: Option<StructuralSearchConfig>`. Omission preserves current behavior. Reject simultaneous structural search and legacy random-module restart on the same island; require at least one weight-only island in an enabled track.
- Island checkpoint gains optional full `base` brain, `structural_attempts`, `last_structural_generation`, `parent_sha256`, and pending `StructuralAttempt`; legacy missing fields resolve to the track base only after validation.
- `StructuralAttempt` stores edit, proposal hash, parent hash, start generation, and outcome. Outcome labels are `training`, `evaluated_without_promotion`, `accepted`, or `superseded_without_evaluation` (for an interval that ends before validation).
- Track checkpoint gains content-addressed brain snapshots and structural history. Event records refer to immutable snapshot hashes and identify generation, track, island, and attempt.

- [ ] Extend CLI fixtures: proposal at generation 1, next at `1 + every`, frozen modules unchanged, rejected proposal preserves incumbent/reference, interrupted run equals uninterrupted state/history, and a weight-only run retains its existing decisions.
- [ ] Add negative checkpoint tests for mismatched structure/activity, changed config, bad snapshot hashes, and invalid attempt counters. Retain strict backend/input identity checks; never bypass them to resume an old executable's run.
- [ ] Run new CLI cases to establish failure. Implement island-owned candidate construction, parameter masks, deterministic seed derivation from campaign/track/island/generation, and proposal creation from the champion current at that boundary.
- [ ] Reset optimizer moments and use a deterministic distinct optimizer seed on structural proposals. Preserve the global campaign generation and keep local optimizer counters explicitly separate. Do not apply a newly promoted structure to another island's old optimizer.
- [ ] Integrate existing fixed validation, paired promotion, faction/stage protection, archive/frontier, and recentring logic with full structural brains. A promoted sample becomes its complete brain, not parameters applied to the old track base.
- [ ] Snapshot initial/proposed/promoted brains and checkpoint events atomically with generation state. Export per-island latest brains from their own base. Interrupted pending generation work must regenerate the same job identities.
- [ ] Run `cargo test --target-dir target/structural-verification --test campaign_cli` plus focused campaign unit tests. Compare a short resume run's parsed state and history with its uninterrupted counterpart.
- [ ] Review the scoped diff and checkpoint the task.

## Task 4: Portable inheritance and accurate brain explanations

**Interfaces**

- Add `--structural-every N` to `scripts/training_branch.py export`, configuring a structural island derived from each selected champion while keeping a weight-only island. Require positive N; the existing mutually exclusive `--from-run`/`--best-dir` remains the champion selector.
- Export/import and work protocols carry full version-2 brains and content-addressed snapshots. Retain existing hash/provenance and native validation paths.
- Browser forward calculations accept module `structure`; legacy absent structure means dense/tanh. Trace views identify disabled inputs/edges and actual neuron activations.

- [ ] Add branch tests: selected champion parameters inherited exactly; immutable source directories unchanged; structural options survive relocation; returned snapshots included; tampered structure/history fails identity checks; repeated identical returns remain duplicates.
- [ ] Run the new Python cases, implement the export option and any required snapshot/native-validation adjustments, then rerun `python -m unittest discover -s scripts -p 'test_training_branch.py'` and related branch-selection suites.
- [ ] Add a tiny browser calculation fixture with each activation and disabled paths. Update maintained brain-watch code and textual explanations to agree with native inference. Verify fixture values against native traces, including legacy brains.
- [ ] Run a short native-backed export/train/return/import probe in a new `data/experiments/structural-smoke-*` directory. Preserve source champions, do not merge optimizer state, and do not claim a physical second-machine test.
- [ ] Review the scoped diff and checkpoint the task.

## Task 5: Transformation dashboard and lay-review

**Interfaces**

- `campaign_dashboard.summarize_structure(brain)` returns named modules, input enable/reachability, hidden activation labels, stable nodes/edges, structural counts, and brain identity.
- `campaign_dashboard.structure_diff(parent, child)` returns changed activations, input toggles, and edge toggles by named stable identities.
- Track dashboard model gains `structure_snapshots`, `structural_events`, `champion_snapshot`, and `structural_history_available`. Use committed checkpoint history for structural events; an ahead-of-checkpoint history append must not expose uncommitted structure.
- Weight drift compares matching enabled identities only, retaining legacy result shape for dense identical layouts.

- [ ] Add tests for legacy defaults, missing history, activation/input/edge diffs, shape mismatch without zip truncation, disabled-weight drift exclusion, ahead-of-checkpoint history, safely embedded malicious labels, and a malformed/missing snapshot shown as unavailable rather than invented.
- [ ] Run new Python tests to establish failure, implement extraction and identity-aware drift, then run all dashboard tests.
- [ ] Add offline SVG network visualization with track/module/snapshot selectors, parent/initial comparison, optional disabled edges, activation labels, and accessible line styles/labels. Present current champion by default; mark experimental snapshots clearly.
- [ ] Add a plain-language timeline and legend. Define activations, enabled versus connected inputs, and experiment outcomes. State that combined structural and weight training changes do not isolate the cause of improvement. Preserve existing performance views and read-only behavior.
- [ ] Extend `--demo` with visibly synthetic structural history including all three edit types, rejection, and promotion. Generate legacy, synthetic evolution, and native smoke dashboards in the new experiment directory.
- [ ] Inspect rendered controls, graph direction, Unicode, readable labels, snapshot comparison, mobile overflow, and light/dark themes using available browser tools. If no browser tools exist, explicitly report that visual validation limitation.
- [ ] Invoke lay-review with one isolated fresh agent, persona a software-literate strategy-game player unfamiliar with neural networks. Permit only the rendered dashboard/screenshots (no source, spec, docs, chat, or web). Require a comprehension check first, quoted/ranked confusion, control predictions, and what worked. User explicitly requested this delegation.
- [ ] Verify each finding against implementation, fix confirmed defects or unclear explanations, and use a fresh isolated reviewer after substantive UI copy changes. Report rejected findings with reasons. If only source or rendered text can be reviewed, label that narrower review honestly.
- [ ] Review the scoped diff and checkpoint the task.

## Task 6: Runnable workflow and completion verification

- [ ] Write `docs/STRUCTURAL_TRAINING.md` starting with export from an explicitly selected run/registry, training the new branch, generating/watching its dashboard, and recognizing accepted versus experimental changes. Explain available sensor limits and fixed neuron counts before advanced configuration details.
- [ ] Add a complete runnable structural-search example, using existing catalog/brain/stage paths and a local weight-only island plus a structural island. Check path resolution through native config loading.
- [ ] Run `cargo fmt --check`, `cargo check --target-dir target/structural-verification`, and relevant Rust learning/campaign/CLI tests. Broaden once to the full suite if the focused checks pass; distinguish preexisting failures from introduced ones.
- [ ] Run Python dashboard/branch tests and `git diff --check` on scoped tracked files; separately check newly added files for whitespace because untracked files are absent from normal git diffs.
- [ ] Review the final implementation against every approved spec section, including inactive optimizer state, snapshot packaging, legacy behavior, and holdout separation. Use requesting-code-review guidance if an independent code review is required by the execution workflow.
- [ ] Report implemented behavior, exact passing checks, lay-review findings/remediation, available artifacts, and any remaining visual or remote-machine limitations. Do not claim training produced a stronger champion merely because the feature passed its tests.

## Execution handoff

Recommended execution: native, implementing the tasks sequentially in this session.
The tasks share brain serialization, mask ordering, and checkpoint interfaces;
keeping implementation in one context reduces integration work. The requested
isolated lay-review remains a separate review of the resulting dashboard.

Approved for native execution and implemented on 2026-10-18. See `docs/STRUCTURAL_TRAINING.md` for the workflow and `data/experiments/structural-smoke-20261018/verification.md` for verification evidence. Existing champion state was preserved.
