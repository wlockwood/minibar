# Diverse ground and air training

Prepare a new portable experiment with source-backed units and separate ground
and air brains. Preparation copies preserved weights and a validated executable;
it does not run training or change the completed 200-generation campaign.

```powershell
cargo build --release
python scripts/prepare_diverse_campaign.py --sim target/release/visigoth-simulator.exe --out data/branches/diverse-training-20261018
```

The destination must be new. Use another directory name for another experiment.
The default BAR source directory is `D:/code/BAR/Beyond-All-Reason`; override it
with `--bar-root`. The package includes the binary and all runtime JSON inputs,
so it can be relocated without the source repository. `receipt.json` records
file hashes, source checkpoints and the independent initialization contract.

The ground initial weights are the independently confirmed cohesion replacement
candidate from `data/experiments/tactical-plateau-20261018/cohesion-candidate.json`.
The unchanged generation-70 champion is the ground reference; preparation verifies
its weights against the preserved campaign state. Neither becomes a fabricated
historical promotion. Air starts from a native `tactical-air-v1` template unless
`--air-brain PATH` supplies an existing air checkpoint. Ground and air optimizer
states start fresh. Each domain keeps the other domain's initial brain frozen
as its companion throughout comparisons.

Preview the native seeded compositions and realized investment:

```powershell
$taskPackage = 'data/branches/diverse-training-20261018'
& "$taskPackage/bin/visigoth-simulator.exe" --catalog "$taskPackage/catalog/catalog.json" --preview-curriculum "$taskPackage/stages/production/ground-heavy_swarm.json" --seed 17
```

Each record includes compositions, counts, requested and realized resource values,
objective investment, family, residual gap, unit cap and supply assumptions. A
resource value is metal plus energy divided by 60. Equal investment does not
promise equal combat strength. Mammoth/40-Pawn and Starlight/Banisher are explicit
curated pairs; Ragnarok/30-Mammoth preserves expensive structure coverage.
Other families include mixed ground, mobile/turret, air/air and bomber/AA fights.
The extra Solar Collector supplies a non-combat attack/defend objective; combat
turrets remain part of the combat packing budget.

Run the bounded plumbing pilots in a visible PowerShell console:

```powershell
& "$taskPackage/Train.ps1" -Profile pilot -Threads 32
& "$taskPackage/Train.ps1" -Profile cohesion -Threads 32
```

Both profiles run three generations with population four and six-second fights.
The cohesion profile changes only the ground cohesion module. Pilots omit holdout
plans entirely and use independent seeds. Their purpose is verifying training,
checkpointing, reports and resuming. They do not establish improved learning.

The production profile has 200 generations, separate protected family stages,
population 32 for the weight-only island, and population 24 with gentle noise
for structural refinement. Training and validation use the same fight distribution
and scripted charge/kite/hold roster. Validation uses two layouts; fresh paired
screening and 12-seed promotion checks retain stage protection. The archive is
disabled, so the opponent roster does not change during the comparison. Production
prepares a distinct untouched holdout distribution with 24-second fights; running
production evaluates that holdout according to the native campaign lifecycle.
After reviewing pilot results, start it with:

```powershell
& "$taskPackage/Train.ps1" -Profile production -Threads 32
```

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
