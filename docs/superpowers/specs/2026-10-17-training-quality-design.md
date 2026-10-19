# MiniBAR training quality improvements

Status: approved by the user on 2026-10-17; implementation planning in progress.

## Intent and approved scope

The user requested implementation of all five recommendations from the campaign
review: audit strategic rewards, broaden evaluation and faction reporting,
preserve balanced challengers, improve production inputs, and run controlled
training experiments. Success means demonstrating what changes behavior and
generalization, rather than merely raising the existing aggregate score.

The generation-5 review is recorded in
`target/campaign-review/assessment.json`. Tactical improvement survived eight
fresh layout bundles, with 246 to 264 wins out of 528 battles per brain.
Strategic improvement survived eight seed bundles, but 88% of its aggregate
fitness gain was the arithmetic contribution of the unit-production bonus.
Armada regressed in every strategic stage. These measurements motivate the
changes; they are not acceptance thresholds for future experiments.

This is a coordinated change to evaluation, scoring, selection, brain inputs,
and experiment tooling. Deliver it in the five independently testable increments
below. Retain the existing evolution strategy and the bounded T1 ground roster.

## Preservation and compatibility

- Do not stop, rewrite, upgrade, or acquire the exclusive training lock of the
  active `data/brains/campaign` run. Leave its executable and inputs available.
- Capture one authoritative state-file byte snapshot for new experiment inputs.
  Read only atomically committed state, validate its structure, and record its
  SHA256 and backend identity. Extract champions from that snapshot rather than
  rereading changing derived exports. This does not export live optimizer state.
- New experiments use separate, explicitly named output directories and a new
  executable under `target/training-quality/release`. Existing packages remain
  intact. Never relax executable/input identity checks to resume an old run.
- Missing new configuration fields preserve the old brain, fitness and
  single-layout evaluation behavior. New quality profiles opt in explicitly.
- Reset optimizer moments and random streams for every independent experiment.
  Reusing champion weights is initialization, not checkpoint continuation.
- Cross-reward or cross-schema champions are compared only after reevaluation
  under one declared comparison profile and executable. Old reported scores
  cannot select a new champion.
- Keep the 30-second console heartbeat and per-generation summaries. Extend
  existing report/package/dashboard consumers to tolerate and display new data.

## 1. Strategic reward audit

Move strategic scoring into a focused, testable module. Serialize each weighted
component and its sum in every strategic trial. Preserve the legacy formula
exactly when the legacy profile is selected:

1. Surviving asset-value difference, divided by 1000.
2. Own surviving asset value, multiplied by 0.2/1000.
3. Metal-income integral, multiplied by 0.02 and divided by episode duration.
4. Production bonus, historically 0.02 per produced unit.
5. Energy overflow penalty, multiplied by 0.001/duration.
6. Metal overflow penalty, multiplied by 0.1/duration.
7. Stalled build-power penalty, multiplied by 0.0001/duration.

Add explicit production modes `count`, `none`, and `value`. `count` uses the
produced-unit count; `value` uses the sum of completed factory products'
`resource_value()` divided by `production_reference_value` (default 100).
Apply `production_weight` (legacy 0.02); optionally cap the resulting nonnegative
production bonus with `production_cap`. Default cap is absent. Validate finite,
nonnegative weights/caps and a finite positive reference value. The new audited
profile selects `none`; capped-count and value modes remain available for
controlled comparisons. Do not silently change defaults in existing curricula.

Production value counts actual completed products once, including produced
builders. Queue submissions, cancelled products and initial entities do not earn
production value. Record survival metrics separately; production remains
production even if that unit later dies.

Report stage/faction/map/side summaries for fitness and its components, production
count/value, surviving own/enemy assets, army/economy value, resource stalls,
construction rejections, and completed projects. Also report completed-entity
value lost on each side: count each completed, non-feature entity's confirmed
death once, excluding unfinished nanoframes. Report the terminal outcome as
`win`, `loss`, `mutual_elimination`, or `unresolved`, based on surviving
non-feature entities; these are diagnostic outcomes, not a new fitness term.
Keep the full episode horizon and identify combat-disabled stages explicitly.

The audit tool can rescore recorded trials under another production mode when
the required sufficient statistics are present. Older reports lacking production
value must be reevaluated for value-mode comparisons, not filled with guesses.
Label rescoring as evaluation of existing trajectories, not retraining evidence.

## 2. Multiple layouts and faction-aware evaluation

Add `layouts` to evaluation plans, default 1. Existing `seeds` retains its meaning
as simulator seeds per layout. For tactical plans, build a separate arena for
each layout instead of treating repeated simulations on one arena as independent
layouts. Strategic evaluation also exposes explicit layout versus simulator seed
identity; start-position jitter is controlled by layout identity and combat
randomness by simulator seed identity. Legacy layouts=1 retains legacy results.

Use checked, non-overlapping seed ranges. A stage's existing 1,000,000-seed band
reserves `layouts * seeds` slots; reject counts that exceed it or overflow u64.
Each layout receives a contiguous simulator-seed block. Log both identities.
Generation, initial validation, promotion and holdout retain separate outer seed
bands, and comparison brains always receive identical conditions.

Store per-layout reports. Overall tactical fitness pools row tallies across all
layouts before applying the existing worst-quarter aggregation. Strategic fitness
remains the mean of its trials. Also expose the mean of per-layout tactical
scores as a separately named diagnostic; do not confuse these two aggregates.

Add a read-only comparison runner with paired layout/seed bundles and an explicit
roster of fixed opponents. Report per-stage, faction, scenario, opponent, map and
side outcomes where applicable; unsupported dimensions are marked unavailable.
Use faction metadata in new tactical scenarios rather than guessing from names.

Report paired deltas, wins/ties/losses between bundle scores, and exploratory
95% percentile bootstrap intervals using 20,000 paired bundle resamples with a
recorded bootstrap seed. Label intervals conditional on the sampled scenarios
and trained models. Across independent training runs, report run-level results
and uncertainty separately; individual battles are not independent training runs.

The new quality validation profile uses 4 layouts and 2 simulator seeds per
layout. Final comparisons use 24 fresh layouts and 1 simulator seed per layout.
Keep evaluation configuration and all source hashes in each comparison receipt.
Zed remains outside model selection. The previously inspected review seeds are
development evidence and cannot be presented as untouched final holdout.

## 3. Retain useful challengers and expose balance

Keep the original reference brain separate from the optimizer's starting brain.
New branches may initialize optimizers from a captured specialist champion while
using the source run's original reference for diagnostic eligibility and optional
faction protection. Record and reevaluate both under the new comparison profile;
upgrading to v2 applies to both when required. Without an explicit reference,
legacy runs continue using their existing initial brain. Official champion stage
guards retain their existing semantics.

Keep the existing official champion's stage-protection and fresh-promotion rules.
Add an independent bounded diagnostic frontier for evaluated challengers. Retain
actual weights and full fixed-validation reports for the starting reference,
official champion and nondominated challenger alternatives. Never retain only a
score when the corresponding weights will be deleted during work cleanup.

Frontier dimensions are stage-by-faction fitness values. Larger is better in
every dimension; domination requires no worse in all dimensions and strictly
better in at least one. Use deterministic comparisons, SHA256 brain identities,
deduplication and deterministic tie-breaking. Admit diagnostic challengers only
when they meet initial-reference protection in every available dimension; they
need not meet every current specialist-champion score. Default frontier limit is
8 alternatives, excluding the separately retained reference and champion. Above
the limit, preserve each dimension's best representative where capacity permits,
then use deterministic normalized nearest-neighbor distance to retain diversity;
ties use aggregate fitness then brain hash. Persist the normalization bounds and
truncation decisions so pruning is reviewable.

The frontier is resumable state with an explicit version. Its immutable brain
objects, reports and generation/island provenance must exist before checkpoint
commit. A partial generation cannot advance the committed frontier. Exports are
reconstructible; stop/resume/local/worker equivalence includes this state.

An optional quality selection guard additionally protects each stage/faction
against the initial reference with maximum regression 0.05. Existing stage and
fresh-pair guards continue to apply. Missing required faction information causes
an explicit error in this mode. Frontier retention never publishes a new
`current-best`, changes the official champion, or inserts an untested opponent
into training. Read-only comparison explicitly chooses alternatives to assess.

## 4. Versioned production sensors

Introduce `strategic_v2`; retain `strategic_v1` loading, policy execution,
templates and evaluation. Add a strategic `brain_kind` config field defaulting
to `strategic_v1`. New quality configs opt into v2. Fixed and archived opponent
brains must have matching kind; changes to templates are explicit inputs to
comparison provenance.

V2 keeps the existing investment and mission interfaces and appends these
production inputs after the existing 12 inputs:

- Candidate energy cost, build time, direct ground damage per second, maximum
  direct-ground-weapon range, and top movement speed.
- Disclosed enemy fractions that are mobile combat units, static combat units,
  builders, and factories.
- Candidate range and ground-DPS advantage relative to disclosed combat enemies.

Use current catalog mechanics for legal targets and ground DPS; exclude paralyzer,
interceptor and unsupported damage modes from the direct-damage estimate. Zero
range/DPS for unarmed products is valid. Unknown or unidentifiable radar contacts
do not contribute invented composition values. All enemy features use the
existing perspective-filtered observations; hidden catalog identities cannot leak.

Use `x/(x+reference)` for nonnegative scalar normalization: energy cost reference
1000, build time 1000 catalog build-work units, DPS 100 damage/second, range 500
elmos, and speed 100 elmos/second. Fractions lie in [0,1]. Advantage is
`(candidate-enemy)/(candidate+enemy+1)` in [-1,1], using mean values across
identified combat enemies; with none, output 0. Clamp/validate finite inputs.

Provide an explicit v1-to-v2 weights upgrade. Match existing input columns by
name, append zero-weight columns, and preserve other layers and biases. This
preserves pretraining decisions for the same observations while enabling new
sensors to learn. Record source/output hashes and schema versions. Do not migrate
Adam state, mutate the source brain, or automatically overwrite an installed best.

## 5. Controlled experiments and operator workflow

Add a manifest-driven experiment runner using existing native campaigns and
portable dependency packaging. It produces one clearly named experiment directory
with an index, generated immutable configs, source snapshots, comparison reports,
per-arm/per-training-seed runs and a readable HTML report. Avoid creating more
ambiguous top-level `campaign-*` directories under `data/brains`.

Provide these five initial arms, each warm-starting from the same captured
champions and using the same broadened evaluation protocol:

1. Legacy reward and v1 inputs, both modules and mutation settings unchanged.
2. No production-count bonus; otherwise identical to arm 1.
3. No production-count bonus and behavior-preserving upgraded v2 inputs.
4. Arm 3 with both islands' sigma halved (0.02 local, 0.08 wide).
5. Arm 3 with tactical `engagement`/`mission` and strategic `production` modules
   trainable; freeze the remaining modules.

Use three independent training seeds per arm, paired across arms. Initial weights,
candidate counts, training episode counts, seed/layout distributions, stage
horizons, validation schedule, thread allocation, and final comparison budget are
identical except for each arm's named treatment. Capture any failure or stopped
run; never discard it from the report or silently restart with another seed.

Pilot profile: one complete generation per arm/seed, population 2 per island,
one training layout and simulator seed, full normal episode horizons, and all
three stages/factions. This is an orchestration and initial-signal screen, not a
claim of optimal settings or sustained learning. Run all 15 pilots; report
wall time and completed work. Do not call a partial generation a completed pilot.
The runner supports larger user-specified generation counts and candidate budgets
for confirmation without changing the experimental definitions.

Compare all arms under one audited, no-production-bonus profile, after upgrading
v1 brains to v2 with zero new-input weights for evaluation. Report legacy score
components as diagnostics only. Keep the original starting reference, the captured
champion, and every arm's outcome visible. Do not select across incomparable
scores or automatically deploy the experiment winner.

The initial implementation runs meaningful unit/CLI/worker/package tests, a smoke
matrix, and the full-horizon 15-run pilot matrix. Final 24-layout comparisons are
run for each distinct resulting champion; identical brains reuse evaluations by
content identity. Estimate remaining work from measured completed pilots and
surface progress throughout; no silent overnight job or recurring schedule.

## Verification and delivery

- Prove legacy reward component sums match legacy scores, count/value/cap modes
  cannot reward queue submissions, and recorded components support exact rescoring.
- Prove equal-count unequal-value production is distinguishable and reported
  combat/economy outcomes cannot masquerade as each other.
- Prove multiple layouts actually change starts, comparison brains share starts,
  seed bands cannot collide, and both tactical aggregation modes are labeled.
- Prove faction regression is visible and optional guard rejects it; rejected
  official candidates can survive as diagnostic alternatives without promotion.
- Prove frontier bounds/deduplication/determinism, interrupted checkpoint recovery,
  worker reuse and resumed versus uninterrupted state equivalence.
- Prove old v1 files still load and upgrading preserves choices and deterministic
  rollouts; new sensors react to range, energy cost, roles and disclosed composition
  without revealing hidden enemies. Validate missing/degenerate inputs.
- Prove every experiment arm differs only in its declared treatment, frozen
  modules stay frozen, duplicate brain evaluations reuse valid results, and
  failed/stopped runs remain in reports.
- Run Rust and Python suites, scoped formatting/whitespace checks, release build
  in its isolated target directory, relocatable package/worker probes and visible
  report inspection. Report existing unrelated failures explicitly.
- Provide exact output paths, launch/resume/compare commands, observed pilot
  outcomes, remaining uncertainty, and the recommended confirmation experiment.

## Research basis

The implementation uses measured MiniBAR failure modes to choose changes; these
papers inform evaluation and experimentation, not claims of BAR strength:

- https://arxiv.org/abs/2108.13264 — uncertainty and reliable RL comparisons.
- https://arxiv.org/abs/1912.01588 — diverse environments and generalization.
- https://arxiv.org/abs/1504.04909 — retaining diverse high-performing solutions.
- https://arxiv.org/abs/1703.03864 — evolution strategies as a viable optimizer.

Joint tactical/strategic execution, imitation learning, air/naval rosters,
distributed scheduling and native BAR qualification are outside this change.
