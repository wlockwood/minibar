# Audited training experiments

Run these commands from the repository root. Python needs only its standard
library. Each experiment captures one committed source checkpoint and creates
fresh optimizers in a separate directory. The source trainer can keep running.

```powershell
cargo build --release --target-dir target/training-quality-final
python scripts/training_experiments.py prepare examples/campaign/quality/experiment.json --out data/experiments/my-quality-pilot
python -u scripts/training_experiments.py run data/experiments/my-quality-pilot
python -u scripts/training_experiments.py compare data/experiments/my-quality-pilot
Invoke-Item data/experiments/my-quality-pilot/report.html
```

The supplied manifest runs five treatments with three training seeds each:
legacy count reward and v1 sensors; no count bonus with v1; no count bonus with
v2; v2 with half-sized mutations; and v2 training selected modules only. Every
pilot completes one generation with normal simulation horizons. Validation uses
four independent layouts and two simulator seeds per layout. Final comparisons
use 24 new layout bundles against the configured opponents and captured champion.
This is a preliminary screen, not evidence of long-run improvement.

`run` resumes unfinished pilots, preserving optimizer state and cached work.
It prints progress every 30 seconds and saves `progress.json`, `runs.json`,
per-run `stderr.log` and `pilot-results.json`. To stop a pilot cooperatively,
create `arms/ARM/seed-SEED/run/STOP`; remove that file before resuming. Active
evaluations finish and cache their results before the coordinator stops. Failed
and stopped runs remain visible. Total generation targets include completed
generations. Change experimental treatments only by preparing a new directory.

## Read the evidence

`assessment.json` and `report.html` include the original reference, captured
champion, every completed pilot champion, paired gains, stage/faction/map/side
breakdowns, combat outcomes, production value, losses and reward components.
All final scores use one common profile with no production bonus. V1 strategic
brains receive a lossless v2 upgrade before this comparison. No winner is deployed.

Mean bundle fitness is the mean of each layout's score. Pooled fitness first
combines tactical scenario/opponent tallies across layouts, then applies the
worst-quarter penalty. These can differ. Opponent roster entries have equal
weight. The reported layout bootstrap interval is conditional on the evaluated
models and scenarios. It does not measure training-run uncertainty. A separate
table reports variation across training seeds and seed-paired treatment gains;
three seeds are too few for a firm ranking. Do not count battles within a layout
as independent replications. The Zed holdout does not select champions.

Comparison inputs are frozen by content: executable, config dependencies,
candidate brains and opponent brains. Cache entries include identities and report
hashes. Live champion replacement cannot change the models used by a comparison.
Prepared experiment input hashes cover the common comparison profile as well as
every training arm. Altered inputs fail validation. Resume always requires the
same training executable. If a separate final evaluator is used, its path and
SHA256 are explicitly recorded as `evaluation_simulator` and
`evaluation_simulator_sha256`; this does not permit changing a pilot backend.

## Opt in to the new training behavior

`examples/campaign/quality/campaign.json` provides a complete profile. Use a new
output directory and the newly built executable. An existing campaign retains its
legacy defaults and must resume with its original executable and dependencies.

Strategic stage configuration accepts:

```json
"reward": {
  "production_mode": "none",
  "production_weight": 0.02,
  "production_reference_value": 100.0,
  "production_cap": null
}
```

`count` preserves the old per-completed-unit bonus. `none` removes it. `value`
rewards completed production resource value divided by the reference value.
Queue submissions, cancellations, and unfinished products earn no production
bonus. Resource value is metal plus energy divided by 60. Every strategic trial
records the seven weighted objective terms and sufficient statistics. Rescoring
existing trajectories is a diagnostic, not a retrained policy; reports missing
the necessary statistics require native reevaluation. Reclaims and unfinished
construction losses are separated from completed-entity combat losses.

Each evaluation plan accepts `layouts` (default 1) and `seeds`. Layouts change
starting geometry; seeds repeat simulator randomness within a geometry. Their
product must fit the native stage seed band. The single-layout default preserves
the previous evaluation behavior.

At campaign level, `frontier_limit: 8` retains a bounded diagnostic set of
alternatives with different stage/faction tradeoffs. These brains are stored under
`frontier/TRACK/` and indexed in each track's `frontier.json`. They are
independent of the official champion. Missing challenger exports are rebuilt from
the committed checkpoint on resume; altered immutable exports are rejected. `max_faction_regression: 0.05` protects every
stage/faction against its fixed reference and fresh paired reference evaluations.
Each track can specify `reference_brain`; otherwise its starting brain is the
reference. Tactical scenarios must all declare `faction` when these features are
enabled. Missing metadata fails before creating a training checkpoint. Local
integration of returned branch champions applies the same faction protection.

## Strategic v2

```powershell
target/training-quality-final/release/visigoth-simulator.exe --upgrade-brain data/brains/from_replays/strategic/baseline.json --brain-kind strategic_v2 --brain-out data/brains/my-v2.json
```

The command refuses an existing output and writes a SHA256 upgrade receipt. It
appends zero-weight production inputs, preserving the original network's choices.
V2 adds energy cost, build time, ground damage/range, speed, disclosed enemy role
fractions and relative damage/range. Anonymous radar and hidden enemies do not
provide identified composition. Set strategic stage `brain_kind` consistently;
v1 remains loadable. Training moments are never transferred between schemas.

For a custom read-only comparison, run `scripts/training_compare.py MANIFEST
--out DIRECTORY`. The manifest specifies `simulator`, `campaign`, `track`,
`reference` label, `brains` entries with `label` and `path`, `layouts`, `seeds`,
`seed_base`, and optional `opponents` entries with `label` and `brain`.

The evaluation choices follow work on [reliable RL comparisons](https://arxiv.org/abs/2108.13264),
[environment diversity](https://arxiv.org/abs/1912.01588), and
[retaining diverse solutions](https://arxiv.org/abs/1504.04909). Simulator reward
gains still require longer independent runs and native BAR qualification.
