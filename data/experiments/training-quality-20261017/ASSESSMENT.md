# MiniBAR training assessment

The strategic brain has improved in audited simulator fitness. Tactical improvement
is unproven, and this study does not establish stronger native BAR play.

This study captured committed generation 15. A final check found the same active
champions: tactical retained from generation 5, strategic from generation 15. The
original campaign and its optimizer were preserved. For legacy continuation, use
`data/brains/campaign` and its original executable/configuration. New experiments
capture its validated champions into separate runs; they do not merge optimizers.

## Current brain versus the original reference

All numbers below use the same no-production-bonus profile, 24 paired layouts,
one simulator seed per layout and two equally weighted opponent roster entries.
The intervals resample layouts, conditional on these models and scenarios.

| Track | Original reference | Captured champion | Champion gain | Exploratory 95% interval |
| --- | ---: | ---: | ---: | --- |
| Strategic | 0.211270 | 0.355111 | +0.143841 | +0.131797 to +0.154698 |
| Tactical | -0.487344 | -0.488639 | -0.001295 | -0.036185 to +0.033603 |

These are mean layout-bundle fitness scores. Tactical fitness pooled across
layouts is separately reported: original -0.460118, champion -0.460967. Pooling
happens before the nonlinear worst-quarter penalty.

The strategic gain survives removing the production-count bonus. It includes
better economy/assets and objective-stage fitness, with a faction tradeoff:
Armada objective fitness fell from 0.448286 to 0.268696 in the captured champion.
Tactical Armada easy-case fitness also fell, from -0.428131 to -0.609846 on these
samples. Aggregate improvement therefore needs faction-level scrutiny.

There were no terminal strategic wins in the evaluated models. Combat-enabled
objective episodes mostly ended unresolved; the improved pilot had one loss in
576 objective trials. Asset/reward improvements do not establish combat wins.

## All 15 full-horizon pilots completed

Five arms used three distinct training seeds each, fresh optimizers, one complete
generation and population 2 per island. Tactical horizons were 25/45/60 seconds;
strategic horizons were 2700/5400/9000 ticks. Validation used four layouts with two
simulator seeds each. No episode budgets were shortened in the full matrix.

| Arm | Strategic mean across 3 seeds | Sample SD | Accepted improvement |
| --- | ---: | ---: | --- |
| Legacy count reward, v1 | 0.363437 | 0.014421 | 1 of 3 seeds |
| No count bonus, v1 | 0.363437 | 0.014421 | 1 of 3 seeds |
| No count bonus, v2 | 0.355111 | 0 | 0 of 3 seeds |
| V2, smaller mutations | 0.355111 | 0 | 0 of 3 seeds |
| V2, selected modules | 0.355111 | 0 | 0 of 3 seeds |

Both v1 arms selected the same improved strategic weights for seed 2026101702.
That model scored 0.380089, a +0.024977 gain over the captured champion, winning
22 of 24 paired layout bundles (exploratory interval +0.015970 to +0.034204).
It recovered Armada objective fitness to 0.447979, while its overall objective
stage was below the specialist champion. It improved easy and mixed stages.
Removing the count bonus preserved the accepted results in this screen.

Every tactical pilot retained the starting champion. V2, smaller mutations and
module masking produced no accepted champion improvement in this single
generation. Their optimizer centers still differ; three seeds and one generation
are insufficient to rank their longer-run learning potential. Layout confidence
intervals must not be mistaken for independent training-run uncertainty.

## Recommendation

Keep the existing campaign as the reference. Use the no-count reward for clear
new comparisons, with the new faction guard and multiple layouts enabled. Treat
the balanced v1 pilot as a candidate for confirmation, not an automatically
published replacement. Its weights are in
`arms/no-count-v1/seed-2026101702/run/tracks/strategic/brain.best.json`.

The next useful confirmation compares legacy-v1, no-count-v1 and no-count-v2 over
at least ten generations and more independent seeds, with a larger population
held equal across arms. Reserve fresh final layout seeds and retain explicit
opponents. Investigate tactical faction tradeoffs before treating aggregate score
as progress. Smaller mutations and module masks can follow once reward/schema
comparisons have a clearer signal. No confirmation run or deployment was started.

## Implementation and verification

Implemented explicit reward components and completed-production telemetry;
independent layouts, paired comparisons and optional faction protection; bounded
resumable challenger retention; lossless strategic_v2 upgrades and disclosed
production sensors; and independent experiment orchestration with frozen inputs,
verified caches and separate training-seed summaries.

397 Rust tests and 91 Python tests passed. All 15 smoke pilots and all 15 full
pilots completed. The final release passed relocated-package, cooperative
stop/resume, reversed external-worker completion, checkpoint equivalence and
missing-report reconstruction probes. Scope-specific formatting and whitespace
checks passed. Read-only review found no remaining substantive issues. Windows
status-write sharing collisions now retry atomically; persistent failures preserve
the prior complete document and report cancellation/draining progress.

Pilot training stayed on its pinned executable. Final evaluation used a separately
recorded corrected executable. Intervening fixes concern diagnostic loss accounting,
frontier exports/role bookkeeping and malformed-metadata rejection; the fully
labelled pilot count/none objectives and scored trajectories are unchanged. Backend
identity checks were preserved. See the exact hashes in `experiment.json`,
`final-build.json`, `input-hashes.json` and `verification.json`.

See [the full interactive evidence tables](report.html), [machine-readable results](assessment.json)
and the [operator guide](../../../docs/TRAINING_EXPERIMENTS.md).

The evaluation design draws on [uncertainty in RL comparisons](https://arxiv.org/abs/2108.13264),
[diverse evaluation environments](https://arxiv.org/abs/1912.01588),
[retaining diverse solutions](https://arxiv.org/abs/1504.04909), and
[evolution strategies](https://arxiv.org/abs/1703.03864). These motivate experimental
controls; the measured MiniBAR results above remain the evidence for this project.
