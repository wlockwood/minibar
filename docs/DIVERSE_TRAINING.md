# Diverse ground and air training

The latest prepared successor is `data/branches/elite-range-training-20261019`.
It starts from snapshots of the saved ground and air champions in
`surround-training-20261018` and imports the five preserved air breakthroughs.
The older campaigns keep their original inputs.

```powershell
.\data\branches\elite-range-training-20261019\Train.ps1 -Profile production -Threads 32
# In a second terminal, after training creates runs/production:
python scripts/campaign_dashboard.py data/branches/elite-range-training-20261019/runs/production --out data/branches/elite-range-training-20261019/production-dashboard.html --watch 20
```

The dashboard shows automatic elite retests separately from the optional
"diagnostic frontier". The pool keeps up to eight promising brains per track,
including specialists that fail a protected fight family. Every five generations,
one is selected by oldest last-retest time and challenged against the current
champion on fresh seeds, through the same screening and paired promotion checks.
Retests do not guarantee promotion or weaken the family protections. Old imported
fitness values are never used. Each retained brain's origin, fresh observations,
last test and latest decision are shown. Complete evaluated candidates and their
paired evidence remain in the run's `candidates` directory.

Prepare a new portable experiment with source-backed units and separate ground
and air brains. Preparation copies preserved weights and a validated executable;
it does not run training or change the completed 200-generation campaign.

```powershell
cargo build --release
python scripts/prepare_diverse_campaign.py --sim target/release/visigoth-simulator.exe --out data/branches/elite-range-training-20261019
```

The destination must be new. Use another directory name for another experiment.
The default BAR source directory is `D:/code/BAR/Beyond-All-Reason`; override it
with `--bar-root`. The package includes the binary and all runtime JSON inputs,
so it can be relocated without the source repository. `receipt.json` records
file hashes, source checkpoints and the independent initialization contract.

When preparing without explicit checkpoint overrides, the ground initial weights
are the independently confirmed cohesion replacement candidate from `data/experiments/tactical-plateau-20261018/cohesion-candidate.json`.
The default generation-70 champion is the ground reference; preparation verifies
its weights against the preserved campaign state. Neither becomes a fabricated
historical promotion. By default, air starts from a native `tactical-air-v1` template unless
`--air-brain PATH` supplies an existing air checkpoint. Ground and air optimizer
states start fresh. Each domain keeps the other domain's initial brain frozen
as its companion throughout comparisons.

Preview the native seeded compositions and realized investment:

```powershell
$taskPackage = 'data/branches/elite-range-training-20261019'
& "$taskPackage/bin/visigoth-simulator.exe" --catalog "$taskPackage/catalog/catalog.json" --preview-curriculum "$taskPackage/stages/production/ground-heavy_swarm.json" --seed 17
```

Each record includes compositions, counts, requested and realized resource values,
objective investment, family, residual gap, unit cap and supply assumptions. A
resource value is metal plus energy divided by 60. Equal investment does not
promise equal combat strength. Mammoth/40-Pawn and Starlight/Banisher are explicit
curated pairs; Ragnarok/30-Mammoth preserves expensive structure coverage.
Other families include mixed ground, mobile/turret, air/air and bomber/AA fights.
The main families request near equal investment (98–102% before integer packing).
Production adds separate advantage/disadvantage exercises with lower stage weights.
Starting front gaps vary using each fight's actual units and objective assets:
the short end is the largest LOS, and the far end is 1.5 times the longest
offensive weapon range. If LOS is larger, the interval collapses at LOS. Air
also gets a runway floor of 4 times (acceleration distance to cruise + turn
diameter); the far end is at least 1.5 times that floor. This gives slow-turning
fighters and bombers room to approach. Preview and dashboard records expose
the sampled gap, bounds and movement derivation. Jitter preserves that sampled
front gap and the arena fits the complete formations.

Mobile policies search along their current heading when there are no contacts,
turning toward the known map centre near its edge. This reveals no enemy truth.
The simulation adds a conservative approach-time allowance to the combat budget,
assuming one side might hold position. Estimates are capped at one hour and
flagged in records; physically unreachable static matchups are flagged too.
The extra Solar Collector supplies a non-combat attack/defend objective; combat
turrets remain part of the combat packing budget.

Run the bounded plumbing pilots in a visible PowerShell console:

```powershell
& "$taskPackage/Train.ps1" -Profile pilot -Threads 32
& "$taskPackage/Train.ps1" -Profile cohesion -Threads 32
```

Both profiles run three generations with population four and 30-second combat budgets plus approach allowance.
The cohesion profile changes only the ground cohesion module. Pilots omit holdout
plans entirely and use independent seeds. Their purpose is verifying training,
checkpointing, reports and resuming. They do not establish improved learning.

The production profile has 200 generations, separate protected family stages,
population 32 for the weight-only island, and population 24 with gentle noise
for structural refinement. Training and validation use the same fight distribution
and the fixed charge/kite/hold/surround roster plus a frozen learned opponent for
each domain. Frozen opponent files are copied from the supplied initial brains,
hashed in the package receipt, and never follow later promotions. The explicit
`swarm_heavy` family trains 40 Pawns against one Mammoth; `heavy_swarm` retains
the reverse. Validation uses two layouts; fresh paired
screening and 12-seed promotion checks retain stage protection. The archive is
disabled, so the opponent roster does not change during the comparison. Production
tests one assembled module-transfer candidate every five generations, under the
same promotion checks, and reanchors stagnant champion followers after 30 generations.
Generation history keeps two saved matchup examples per stage and full coverage
counts. Complete evaluated evidence remains in candidate receipts and cached work.
Production
prepares a distinct untouched holdout distribution with a 120-second combat budget
plus approach allowance; training and validation use the same rule and resolved
fights end early. Running
production evaluates that holdout according to the native campaign lifecycle.
After reviewing pilot results, start it with:

```powershell
& "$taskPackage/Train.ps1" -Profile production -Threads 32
```

Monitor the pilot (use `runs/production` after starting production):

```powershell
python scripts/campaign_dashboard.py "$taskPackage/runs/pilot" --out "$taskPackage/pilot-dashboard.html" --watch 20
```

The prepared verification artifacts include `watch-ground.html`, `watch-air.html`
and their native composition previews when generated during validation. Air's
decision panel displays its own flight signals.

The dashboard's **Are units able to fire?** panel reports both learner and opponent
for each validation stage. Participation is the number of armed units that fired
at least once, summed over battles, divided by armed units in those battles.
Friendly, terrain and supply obstruction rates use aimed, ready weapon checks as
their denominator. An obstructed weapon is checked again each simulation tick,
so this measures sustained blockage rather than a percentage of bullets lost.
The recording viewer shows full-battle participation and obstruction at the end.
Older results without these counters show unavailable rather than zero blockage.

The `surround` script assigns spaced firing positions on staggered rings around
visible targets, approaches far-side positions along an arc, and leaves overflow
units outside the firing ring until slots free up. Static units still obey their
movement limits. This is a benchmark opponent, not a hard-coded reward or a
guarantee that surrounding is the right tactic in every matchup.

Prepare a new package for these changes: existing packages contain their own
frozen executable and configuration. Updating repository scripts does not change
a running campaign. To keep current ground and air progress, pass their saved
`brain.best.json` files using `--candidate` and `--air-brain`; optimizer moments
start fresh because the evaluation distribution has changed. Use a new `--out`
directory and seed. Never edit an existing package's receipt to bypass its hashes.

Each profile saves its own state under `runs/PROFILE`. The launcher checks package
hashes before each invocation. Stop at a cooperative boundary and resume with the
same executable and inputs:

```powershell
& "$taskPackage/Train.ps1" -Profile pilot -Stop
& "$taskPackage/Train.ps1" -Profile pilot -Resume -Threads 32
```

Active work finishes and caches before stopping. `-MaxSeconds 60` sets a cooperative
time budget and may overrun while an active job completes. Altered inputs require
a new package. The finite supply is one million metal and energy per side with
no resupply; it is ammunition supply and is separate from unit investment. Outcome
stakes explicitly use a zero victory floor, so investment advantages remain
accounted for in wins and losses.

Read `catalog/manifest.json` for source hashes, rejected mechanics and model
approximations. Successful loading, combat fixtures and short pilots provide
bounded simulator evidence. They do not establish native BAR equivalence or
improvement over preserved champions on the expanded distribution.
