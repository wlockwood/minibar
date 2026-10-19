# Diverse tactical successor

This is a new independent experiment. The ground anchor is the confirmed cohesion
replacement candidate; its unchanged generation-70 champion is the reference.
The air brain has its own explicit initialization and weights. Opposite-domain
companions remain frozen at package initialization. No optimizer state is inherited.

Run a short plumbing pilot in a visible PowerShell console:

```powershell
./Train.ps1 -Profile pilot -Threads 32
./Train.ps1 -Profile cohesion -Threads 32
```

These 3-generation, 6-second pilots test the workflow. They cannot establish
learning improvement. They contain no holdout plan. After reviewing pilot evidence,
the production profile is available with `./Train.ps1 -Profile production -Threads 32`.
Production is 200 generations and includes a separate untouched holdout plan;
running production evaluates its holdout according to the native campaign lifecycle.

Stop cooperatively with `./Train.ps1 -Profile pilot -Stop`; active work finishes
and caches. Resume using `./Train.ps1 -Profile pilot -Resume -Threads 32`.
`-MaxSeconds 60` sets a cooperative time budget, not a hard process kill.
Each profile has its own `runs/PROFILE` state. Keep package inputs unchanged.
The launcher verifies all prepared file hashes before running or resuming.

Preview exactly the native seeded scenarios without training:

```powershell
./bin/visigoth-simulator.exe --catalog ./catalog/catalog.json --preview-curriculum ./stages/production/ground-heavy_swarm.json --seed 17
```

`catalog/manifest.json` records source paths, hashes and bounded mechanics.
The extra Solar Collector is a non-combat objective asset; combat turret investment
is packed in the combat budget. One million metal and energy per side is finite
ammunition supply; no resupply is modeled. See `receipt.json` for executable,
source-brain, checkpoint and package identities. Source/catalog acceptance and
plumbing pilots do not establish native BAR equivalence or champion improvement.
