# Diverse tactical successor

This is a new independent experiment. Ground starts from the supplied checkpoint
(the confirmed cohesion replacement candidate by default); the preserved
generation-70 champion is the reference. Exact sources are listed in receipt.json.
The air brain has its own explicit initialization and weights. Opposite-domain
companions remain frozen at package initialization. No optimizer state is inherited.

Run a short plumbing pilot in a visible PowerShell console:

```powershell
./Train.ps1 -Profile pilot -Threads 32
./Train.ps1 -Profile cohesion -Threads 32
```

These 3-generation pilots use 30 seconds of combat plus approach allowance to test the workflow. They cannot establish
learning improvement. They contain no holdout plan. After reviewing pilot evidence,
the production profile is available with `./Train.ps1 -Profile production -Threads 32`.
Production is 200 generations with a 120-second combat budget plus approach
allowance (resolved fights end early), a protected 40-Pawns-versus-Mammoth family alongside the reverse, and
charge/kite/hold/surround scripts plus a frozen learned opponent per domain.
The frozen opponents are copies of the initial brains and never follow promotions.
Both training and validation use this roster.

Starting front gaps vary from the largest participating LOS to 1.5 times the
longest offensive weapon range. If LOS exceeds that far end, the interval
collapses at LOS. Aircraft receive a conservative runway floor of
4 * (cruise acceleration distance + turn diameter); their far end is also at
least 1.5 times this floor. Travel allowance budgets for an opponent holding
position. The configured 30/120 seconds remains available as combat time after
that estimated approach. Preview records include all bounds and capped or
unreachable-contact flags. Fixed historical configurations remain unchanged.

Up to eight promising elites are retained per track. Every five production
generations (every pilot generation), one gets a fresh challenge through the
same screening, paired tests and promotion checks. Standalone imported elite
brains are frozen in this package and their old scores are not reused. Full
candidate receipts remain under runs/PROFILE/candidates. The dashboard shows
pool identities, provenance, recent retest gains and rejection reasons.

Reconnaissance support cycles through neither side, one side and both sides
in equal thirds, with seed-dependent rotation. On covered sides, radar towers,
radar-equipped scout aircraft or both are selected. Jammers vary independently
(one-third chance per side). Assets use source-backed sensor ranges, can be
shot down/destroyed and never grant omniscient vision. Scout aircraft fly a
fixed area sweep; combat aircraft keep their separate learned air controller.
Support investment is separate from the near-equal main combat armies, while
all costs and damage count in scoring. Surviving support cannot prevent a win
after the main force is eliminated. Actual coverage may disappear as assets die
or leave range, and jamming does not block visual scouting.

Production includes a separate untouched holdout plan;
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
