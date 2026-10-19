# Replay-informed training campaign

See [measured verification results](RESULTS.md) and the
[training contract](../../docs/TRAINING_CAMPAIGNS.md) for details.

For audited rewards, faction guards, strategic_v2 and independent experiments,
use the [training experiment guide](../../docs/TRAINING_EXPERIMENTS.md).

For training elsewhere from current champions and importing the result, use the
[portable branch workflow](../../docs/TRAINING_BRANCHES.md).

For the updated detection model and a fresh sensor-correct campaign profile,
see [BAR sensors](../../docs/BAR_SENSORS.md). Existing packages retain their
original executable and inputs.

From the repository root, build the release simulator and start both tracks:

```powershell
cargo build --release
target/release/visigoth-simulator.exe --train-campaign examples/campaign/replay-informed.json --train-out data/brains/campaign --threads 4
```

Resume the same directory with `--resume`. `--generations N` sets the total target
generation, including generations already completed:

```powershell
target/release/visigoth-simulator.exe --train-campaign examples/campaign/replay-informed.json --train-out data/brains/campaign --generations 250 --resume
```

To stop cooperatively, create `STOP` in the run directory:

```powershell
New-Item -ItemType File -Path data/brains/campaign/STOP -Force
```

Alternatively, add `--max-seconds 1800` to bound one invocation. Its budget starts
at invocation startup, including setup. STOP and the time budget are checked
before each stage and job launch, including initial validation and final holdout.
Already running simulations finish and cache their results, so this is not a
hard wall-clock deadline. A partial generation does not commit optimizer or
champion changes; `--resume` reuses its matching completed job files. Keep those
files and remove `STOP` before resuming:

```powershell
Remove-Item -LiteralPath data/brains/campaign/STOP
```

Read `TRAIN_OUT/progress.json` for live status, track, plan, generation/island
label, stage name/index, current-stage job totals/completed/cached/waiting counts,
elapsed seconds, and stop reason. The stage index is zero-based; counts describe
the current stage, not the whole campaign. Updates are atomic at stage start and
each completed, cached, or queued job. `summary.json` refreshes after committed
checkpoints and therefore can lag the active stage. For example:

```powershell
Get-Content data/brains/campaign/progress.json -Raw | ConvertFrom-Json
```

The simulator also prints a console heartbeat every 30 seconds, including while
an evaluation is still running. It shows the current generation/track/island and
stage, completed/cached/waiting evaluations, elapsed time, time since the last
completion, and retained champion validation scores. After two newly computed
evaluations, it estimates evaluations per minute and remaining time for the
current stage; cached results do not inflate the rate. Stage costs vary, so this
is not a whole-campaign ETA. Each committed generation prints its duration,
per-island candidate fitness statistics, validation scores (when scheduled),
promotions, fresh promotion decisions, and module restarts. Console reports go
to stderr; the final JSON summary stays on stdout. Generation durations on resume
measure the current invocation's work, including reuse of cached results.

For a bounded orchestration check, use `examples/campaign/smoke.json`. It runs two
generations with one population-two island per track and very short simulations.
Its scores are an execution check, not evidence of stronger play. Strategic
smoke stages last 300/600/900 ticks and can end with partially built projects;
they do not demonstrate completed production or combat. The normal easy stage
lasts 2700 ticks so every faction can finish its initial construction.
Smoke explicitly disables the fresh promotion challenge (`promotion_seeds: 0`);
use the normal profile for the full promotion check.

Every candidate runs the three stages in ascending difficulty before its island
updates once. Stage weights are 0.25 easy, 0.35 mixed, and 0.40 objectives; weights
do not grow with a stage's number of battles. Easy cases remain in every
generation. The default search has an independent population-32 local island
with sigma 0.04 and population-eight exploratory island with sigma 0.16. These
represent approximately 80/20 of candidate evaluations; the harder stages take
more simulation time. Only the exploratory island is marked for restarts.

The normal profile is a long run: its 40 candidates per track and two training
seeds require 6,880 tactical battles plus 2,400 strategic rollouts per generation,
before archive opponents, validation, promotion checks, and final holdout. At 200
generations that is at least 1,856,000 simulations. Measure early generations on
the target machine before choosing a run duration; the short smoke run cannot
predict the normal profile's wall time.

The tactical track progresses from duels/pairs through mixed ranges and roles to
attacking and defending factories. All mobile combat types use the `bot2` mask.
Easy and mixed training include controlled three-versus-five and five-versus-three
fights to exercise retreat and commitment. Objective opponents include `strike`,
which explicitly attacks enemy assets, alongside `charge` and `kite`; `guard`
is excluded because it holds when a defend scenario gives it no asset of its own.
The strategic track progresses from short synthetic construction through larger
production economies to growth and combat on a synthetic map and Center Command.
Default stages include Armada, Cortex, and Legion. Smoke stages cycle through
the three factions while keeping the evaluation count small.

Validation uses separate stage files with four fixed seeds. Tactical validation
changes counts, compositions, separation, and jitter; strategic validation changes
starting metal/energy and jitter on the same map families. These cases stay out
of training. The separate Zed holdout changes faction pairings and mission rosters,
uses two seeds, and does not select champions. Training uses two seeds. Zed is a
map-transfer check for authored initial brains, not evidence of human-play
generalization; it must be excluded from any future imitation-training data before
serving as a holdout for an imitation-initialized brain.
Normal-profile promotion additionally compares the proposed champion with the
incumbent on four fresh paired seeds from the validation scenario family. This
uses a separate seed band, not the fixed validation seeds or Zed holdout.
Rebalanced tactical validation (optional): in the 2026-10-18 recovery run, 20 of 33
validation cells (scenario x opponent) were saturated, i.e. >= 95% one outcome for
the starting brain (many 100% wins, some always-lost or always-timeout), so they could
not rank brains. `stages/tactical-validation-v2-*.json` adjust force ratios so that
about a third of cells are saturated and the starting, champion and latest-island
brains separate more (measured with `--evaluate-campaign` on 16 fresh seed bases; rows
for the passive `kite`/`orbit`/`strike` opponents stay mostly saturated by design).
`replay-informed-tactical-v2-validation.json` is `replay-informed-tactical.json` pointing at
them. The original files are unchanged because running campaigns and packages hash them;
a new campaign needs the v2 files from its start, since changing validation mid-run
breaks the champion's stored scores. Re-measure after changing units or the engine.

Tactical and strategic brains remain separate controllers; this campaign does
not jointly train their execution or turn the strategic force controller into
the learned tactical council.

The inputs are the bounded 22-unit baseline catalog and authored baseline brains
under `data/brains/from_replays`, together with its five saved map caches. These
are prerequisites: packaging and training fail if a referenced input is missing.
Current-source simulation fields are approximations, not complete runtime BAR
exports. Center Command and Zed retain **32 by 32 engine-captured samples**; the
64-elmo simulation raster adds no captured terrain detail. Resource sampling,
terrain leveling, and unsupported mechanics are documented in the source asset
manifest. Flight, underwater combat, carriers, and gadget-backed damage are
outside this campaign.

Campaign stages require compiled `.vsgmap` caches. Compile map JSON first; raw map
JSON can silently acquire runtime settings from adjacent acquisition sidecars,
so native campaigns reject it. Every shipped stage already uses a map cache.

To make a portable package after building the current executable:

```powershell
python scripts/package_training.py --config examples/campaign/replay-informed.json --sim target/release/visigoth-simulator.exe --out data/packages/my-training-package --zip
```

The destination must not already exist. The package recursively includes stage
configs, catalog, baseline/opponent brains, maps, and the native executable. Paths
are rewritten relative to each copied config. A SHA256 manifest records both
shipped and original dependency hashes, and `README.md` inside the package gives
training, resume, and standalone-worker commands. Move or unzip the complete
directory; it needs neither BAR source nor the original checkout. The executable
still requires its build operating system and architecture.

The verified current Windows bundle is
[replay-informed-readiness.zip](../../data/packages/replay-informed-readiness.zip).
Earlier packages and trial directories are retained as historical evidence.
This build changes policy behavior and strategic fitness; start a fresh run
directory rather than resuming a checkpoint made by the earlier executable.

The worker interface accepts coordinator-generated jobs:

```powershell
target/release/visigoth-simulator.exe --campaign-worker JOB.json --job-out RESULT.json --campaign-config examples/campaign/replay-informed.json
```

Transfer the job and all assets it references together. This provides a portable
evaluation interface; it does not run a network scheduler or merge arbitrary
worker results into an active campaign.

For manual workers, start the coordinator with `--external`. It writes pending
jobs under `TRAIN_OUT/work/jobs` and returns `waiting_for_workers`. Run each job
with the identical executable and matching campaign assets, writing its result
to `TRAIN_OUT/work/results/JOB_ID.json`. A relocated bundle supplies its local
campaign path through `--campaign-config`. Transfer results back to the
coordinator, then rerun it with `--resume --external`; repeat as subsequent
stages produce jobs. Completed stages cannot bypass the generation barrier.

Resume preserves optimizer state and seed streams. Keep the executable, stage
configs, catalog, maps, and starting brains unchanged. Training draws new
generation seed ranges; validation and holdout keep separate fixed ranges. A
different seed or changed dependency starts a new experiment, while increasing
the total generation target is permitted.
