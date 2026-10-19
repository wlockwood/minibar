# Campaign verification, 2026-10-17

## Current readiness build

The updated release passed **376 Rust tests** (356 library and 20 others),
including 11 native campaign CLI tests, and **32 focused Python tests**. These
cover initial/partial-generation/holdout stops, exact cached resume, missing
limit values, accepted and rejected fresh promotions, unchanged-incumbent cache
reuse, worker input drift checks, and relocated worker addressing.

A retained one-generation trial used both tracks, two candidates per local and
exploratory island, normal episode horizons, one seed per training/fixed
validation/holdout plan, and **four fresh paired promotion seeds**. Total observed
wall time was **146.02 seconds** with four threads. A five-second cooperative
budget returned after 11.80 seconds, having finished its active work and committed
initial validation; resume completed in 134.23 seconds. This illustrates that
the time limit bounds new work launches rather than interrupting a simulation.

**Neither track retained a new champion.** Two tactical challengers cleared the
fixed-validation filter but failed their fresh paired checks:

| Challenger | Fixed validation | Fresh mean gain over incumbent | Pairs won | Decision |
| --- | ---: | ---: | ---: | --- |
| Local center | -0.346210 | -0.147551 | 0/4 | Rejected |
| Exploratory sample | -0.235894 | -0.047364 | 3/4 | Rejected |

The second candidate's larger loss outweighed its three wins. Its fixed-suite
score matches the winner retained by the earlier trial below. The new gate exposes
that weak evidence instead of reporting the fixed-suite gain as stronger play.
No strategic candidate cleared the fixed-validation gain filter.

Retained tactical validation/holdout were -0.374616/-0.806060. Strategic
validation/holdout were 0.380555/-0.269446 under the corrected asset reward.
These are preliminary simulation scores, not BAR strength measurements. The
trial's fixed-suite coverage is smaller than the normal profile's four seeds.
Its single generation does not establish long-run learning or optimal settings.

Independent same-condition replay of the previous strategic champion's 30
validation trials confirmed the placement correction: **91 -> 8 rejected orders,
72 -> 74 completed projects, and 143 -> 145 produced units**. Twenty-seven world
state hashes stayed identical. The policy now defers locations rejected by its
own build orders and tries alternatives; hidden occupant identity stays unknown.
Some initial/retried hidden-site rejections remain visible. Asset fitness now
counts each surviving non-feature entity once, with HP weighting, preventing
incidental storage from doubling constructor/factory value. Capability metrics
still overlap for sensors and reports. Previous strategic scores use a different
policy and reward and must not be compared numerically with this build.

Current evidence:

- [Bounded config](../../data/brains/campaign-readiness-20261017/campaign.json),
  [stop/resume receipt](../../data/brains/campaign-readiness-20261017/receipt.json),
  [summary](../../data/brains/campaign-readiness-20261017/run/summary.json), and
  [history](../../data/brains/campaign-readiness-20261017/run/history.jsonl).
- [Independent placement rerun receipt](../../data/brains/campaign-readiness-20261017/placement_recheck_receipt.json);
  full stage reports are beside it. Completed campaign work queues are empty.
- [Current portable normal bundle](../../data/packages/replay-informed-readiness.zip)
  and [smoke bundle](../../data/packages/campaign-smoke-readiness.zip), with
  [manifest verification](../../data/packages/readiness-package-manifests.json)
  covering 25 and 15 shipped files respectively, in directories and ZIPs.
- [Extracted smoke training/resume receipts](../../data/packages/relocated-readiness-smoke/verification_receipts.json):
  two generations, then resume to three, using the packaged executable/assets.
- [Relocated normal-profile worker receipts](../../data/packages/relocated-readiness-worker/verification_receipts.json):
  a job with an unavailable original coordinator path was solved using the
  bundle's config override, transferred back and accepted. The coordinator
  advanced to the next initial-validation stage. This manual probe deliberately
  stops waiting there; no process remains active.

All current artifacts use executable SHA256
`285b725b869741057e6e8153314aeda596b019b5eab8385d7363298b7b2c0aad`.
Package checks ran on one Windows host. Multi-host scheduling and cross-machine
floating-point equivalence remain untested. A changed executable intentionally
refuses old checkpoints; start a fresh run for this build.

## Earlier baseline trial

The following records the previous executable, before the placement/reward fixes
and fresh promotion gate. Its artifacts are retained as historical evidence.

The native release campaign completed two generations across tactical and
strategic tracks, each with a local and exploratory island. The bounded trial
used the normal three-stage episode horizons and scenario files, but only two
candidates per island and one seed for training, validation and holdout.
Validation ran every generation. Total observed wall time was **190.66 seconds**
with four threads. This is a small verification trial, not a benchmark or a
measurement of native BAR strength.

| Track | Initial validation | Retained validation | Initial Zed holdout | Retained Zed holdout |
| --- | ---: | ---: | ---: | ---: |
| Tactical | -0.374616 | -0.235894 | -0.806060 | -0.606889 |
| Strategic | 0.499516 | 0.525783 | -0.143632 | 0.010866 |

Each track's best checkpoint came from generation one. The tactical local center
improved first, then an exploratory sampled candidate improved it again; the
strategic improvement also came from an exploratory sampled candidate. This
exercises preserving useful larger mutations rather than keeping only the
updated optimizer center. Two generations did not reach the default restart
patience; integration tests separately force and verify module restarts.

The strategic champion's validation stages completed 72 construction projects
and produced 143 units across 30 trials. The objective stage still recorded 91
placement rejections; easy and mixed stages recorded none. Mixed tactical
validation has substantial timeouts. These limitations remain visible in reports
and should be addressed before treating a long run as a strength evaluation.
Scores have different definitions between tracks and are not comparable across
the table's rows. One-seed holdout gains are preliminary.

Evidence in this checkout:

- [Exact bounded config](../../data/brains/campaign-validation-20261017/campaign.json),
  [command receipt](../../data/brains/campaign-validation-20261017/receipt.json),
  [summary](../../data/brains/campaign-validation-20261017/run/summary.json) and
  [history](../../data/brains/campaign-validation-20261017/run/history.jsonl).
- Full checkpoints, initial/champion/island brains and stage reports are under
  `data/brains/campaign-validation-20261017/run`. Completed work queues are empty.
- [Baseline holdout receipts](../../data/brains/campaign-validation-20261017/holdout_baseline_receipts.json)
  record the independent initial-brain evaluations on the same holdout conditions.
- [Relocated-package receipts](../../data/packages/relocated-campaign-smoke/verification_receipts.json)
  record training two generations, resuming to three, and a standalone relocated
  worker whose result was accepted by the original source coordinator.

The package and worker tests ran on one Windows host. No multi-host scheduler,
cross-platform binary compatibility or cross-machine floating-point equivalence
was tested. The packaged executable must match the coordinator executable.

Verification passed: **361 Rust tests** (346 library plus 15 others), **32 focused
Python tests**, release build, scoped formatting, whitespace checks, package SHA256
manifests and local documentation links. Six native CLI integration tests cover
exact uninterrupted/resumed/external-worker equivalence, relocated config paths,
changed inputs, corrupted optimizer settings and stale holdout removal.
