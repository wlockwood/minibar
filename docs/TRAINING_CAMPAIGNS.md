# Training campaigns

The [campaign guide](../examples/campaign/README.md) starts with the runnable
command, package export and worker workflow. This document explains the training
contract and saved artifacts.

## One generation

Each track owns independent evolution-strategy populations, called islands. The
default local island has 32 candidates at noise standard deviation 0.04; the
exploratory island has eight at 0.16. These are experimental defaults.

Within each island's generation, all candidates first complete the easy stage,
then the mixed stage, then the objective stage. A barrier prevents advancing
until every candidate has a valid result for the current stage. Both strong and
weak candidates complete every stage, using identical scenarios and seed ranges.
The optimizer updates **once**, using the normalized weighted average of those
stage scores. Hard stages cannot erase easy stages from the objective, and a
stage with more battles does not automatically receive more weight.
History records each stage's population mean, standard deviation, extrema and
number of distinct scores. A flat stage provides no ranking signal; a high-variance
stage can dominate selection despite a smaller explicit weight. These diagnostics
support adjusting a curriculum; scores are not automatically standardized.

The tactical and strategic tracks train separate brains. Their stage scores have
different meanings and are never combined into one fitness value. This does not
yet connect the learned tactical controller to strategic execution.

Strategic fitness values each surviving non-feature entity once, scaled by its
remaining HP. Role capability metrics can overlap (for example, a constructor
with energy storage) without doubling that asset's reward. Strategic policies
also defer build locations after their own `PlacementRejected` orders, with
increasing retry delays, using no hidden occupant identity. The rollout feeds
`StrategicPolicy::observe_decisions` after each world step; callers using the
policy outside that rollout must feed the same own-order feedback.

## Exploration and selection

Training includes the fixed opponents plus a deterministically selected member
of the champion archive. Historical selection mixes the seed before indexing to
avoid repeatedly selecting the same slot. The archive is bounded by `archive_size`.

On the first generation and each `validation_every` generations thereafter,
evaluate both the updated optimizer center and the best sampled candidate on the
fixed validation suite. A candidate replaces the champion only when its aggregate
fitness improves by more than `min_gain` and no stage falls more than
`max_stage_regression` below either the current or initial champion. That fixed
suite is the first filter. With the default `promotion_seeds: 4`, a surviving
candidate and the incumbent then face four matching, single-seed validation plans
in a separate generation-specific seed range. Promotion also requires mean gain
above `min_gain`, a strict majority of positive pairs, and no excessive stage-mean
regression against the incumbent. Both brains receive identical conditions and
no archived opponents in this challenge. History retains every challenge's paired
reports, including rejected challengers. `promotion_seeds: 0` explicitly disables
this extra gate for the short smoke profile; the maximum is 64.
Unchanged-incumbent results are cached across challengers within a generation;
replacing the champion changes its work identity and requires new results.

Two optional settings tighten this (both off by default, so existing configs and
checkpoints behave as before):

- `promotion_min_t` (default `0`): also require the paired t statistic of the
  per-seed gains (mean gain over its standard error) to reach this value. Every
  report records `t_statistic`. Four pairs give a noisy estimate: a 2026-10-17
  simulation of the 4-pair rule accepted 21% of candidates that were no better
  and 30% of ones that were, so for cheap tracks raise `promotion_seeds` (up to
  64) and set `promotion_min_t` near 2.
- `recentre_on_promotion` (default `false`): when an island's sampled candidate
  becomes champion, restart that island's search at it with cleared optimizer
  moments and the same generation count (fresh noise). Without it an island keeps
  searching around its own centre, so a lucky sample is selected but never
  followed.

These guards protect basic skills while allowing gains on difficult stages. They
are conservative selection rules; `promotion_min_t` is a threshold on a small-sample
t statistic, not a calibrated confidence interval.
Challenge seeds change each generation; all challenges within that generation
share them. Increase evaluation coverage for consequential strength comparisons.

An exploratory island with no champion progress for `restart_after` generations
restarts one trainable module from random weights at `restart_scale`, keeping the
other modules from the champion. Its optimizer moments reset and its noise stream
changes. The champion itself remains unchanged until a candidate passes validation.

The holdout plan runs after the requested generations and never selects a
champion. Reusing that result to tune future experiments would make it a validation
set; reserve other maps/seeds when making a final strength comparison.

## Continuing a run

`--generations N` is a total target, not an additional count. A generation's
validation schedule depends on its number, not where a command stops. This keeps
one run to generation 100 equivalent to a run to 40 followed by resume to 100.

The authoritative `state.json` contains optimizer centers, Adam moments, noise
configuration, generation numbers, restart/progress counters, champion/archive
brains and histories. JSON parsing uses exact floating-point round trips.
Each completed generation commits atomically; derived brain/report exports are
regenerated on resume. A directory lock prevents two coordinators writing the
same run. Creating `STOP` in the output directory or setting `--max-seconds N`
stops before the next stage or individual job starts, including initial validation,
promotion and holdout. Remove `STOP` before continuing. The budget starts at
invocation, including setup. Already running jobs finish and cache their results,
so this is a cooperative limit and can overrun by those active jobs. A partial
generation does not advance optimizer or champion state.

An interrupted generation reuses matching completed work results and recomputes
missing work. The optimizer advances only when every required stage succeeds.
Changing curriculum, catalog, maps, baseline weights, seed or executable refuses
resume. Increasing the generation target is allowed. Content identities replace
dependency paths, so relocating a complete package preserves input identity.
Campaign maps must be compiled VSGMAP data, avoiding implicit acquisition sidecars.

## Files and workers

| Path under the run directory | Contents |
| --- | --- |
| `state.json` | Complete resumable campaign state |
| `summary.json` | Running, waiting, stopped or completed status and champion metrics |
| `progress.json` | Current plan/stage, job counts, elapsed seconds and stop reason |
| `history.jsonl` | Ordered per-generation, island and stage scores, updates and promotions |
| `tracks/NAME/brain.best.json` | Validated champion |
| `tracks/NAME/ISLAND/brain.latest.json` | Latest optimizer center for that island |
| `tracks/NAME/validation.best.json` | Champion's full stage validation reports |
| `tracks/NAME/holdout.json` | Final holdout report; removed when extending the run |
| `work/jobs` and `work/results` | Pending or completed work for an uncommitted generation |

With `--external`, the coordinator emits jobs and returns `waiting_for_workers`.
Workers consume one versioned job and write one matching result. The coordinator
accepts results by job identity and candidate index, regardless of arrival order,
and checks protocol, inputs, executable, stage metadata and finite scores. Finished
work files are removed after checkpoint commit to bound disk growth.
Workers recheck input identity before publishing results, refusing an accidentally
edited bundle. The coordinator also checks input identity before checkpointing
initial validation, generations and holdout.

`progress.json` updates atomically when a stage begins and when a job completes,
reuses a cached result or queues external work. Counts refer to the current stage;
`jobs_cached` is included in `jobs_completed`. `summary.json` refreshes after each
committed checkpoint and on exit. Progress is observational and does not alter
the reproducible checkpoint/history state.

The native worker and portable bundle provide the execution contract for later
distributed scheduling. Today, job/result transfer and coordinator reinvocation
are manual. There is no network service, remote authentication or automatic host
discovery. Work fingerprints detect accidental mismatch for trusted workers;
package SHA256 manifests identify the shipped files. Executables must match and
require the operating system/architecture for which they were built.
