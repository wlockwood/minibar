# Wrecks, reclaim, and metal left behind

MiniBAR keeps supported destroyed units as neutral wrecks. Wrecks can obstruct
movement and fire, and damage can turn a wreck into a smaller heap when its
catalog defines that next stage. A wreck is no longer part of either army:
it cannot fight, provide sensors, or keep a side alive.

## Try a fresh catalog

The catalog must contain corpse definitions. To derive a small catalog from
your local BAR checkout and record a duel, run from the MiniBAR directory:

```powershell
$env:BAR_ROOT = 'D:\code\BAR\Beyond-All-Reason'
python scripts/make_catalog.py --out data/wreck-demo-catalog.json armpw corthud
cargo run --release -- --catalog data/wreck-demo-catalog.json --unit armpw --left kite --right charge --runs 1 --viewer wreck-duel.html
```

The output catalog is derived from source, not a game runtime export. A duel
only leaves a visible wreck if a unit dies and its exported corpse exists.
Use the same refreshed catalog with `--train` or `scripts/watch.py` to observe
wrecks during training battles. Recordings keep metadata for features that
appear after the first frame. Brown diamonds distinguish neutral wrecks from
red enemies; right-click enemy selection does not select a neutral wreck.

Old catalogs remain loadable, but missing `corpse` data means no wreck appears.
Re-export the catalog with the updated source exporter; loading it in a newer
simulator does not add missing definitions. Avoid `make_catalog.py --base`
when refreshing existing entries: that option preserves those entries verbatim.
Replay-derived catalogs need regeneration with the updated
`scripts/prepare_from_replays.py` as well. A legacy heap name alone does not
provide its health, shape or metal; refreshed catalogs include the next-stage
definition inline.

## Read the result

Battle results report `wreckage.metal_by_source_side` in physical arena-side
order, followed by `total_metal`. These are metal units still available in
living, reclaimable wrecks or heaps. A heap contributes its own remaining metal,
not its original wreck's metal. The corresponding side score is
`wreck_metal_left`.

With one surviving army, `winner_reclaim_potential` is all remaining neutral
wreck metal and `loser_metal_conceded` is the portion originating from the losing
army. These fields express an opportunity left on the field, not ownership,
territory control, or successful recovery. They are absent for a draw or timeout.
For example, 30 metal from the winning army and 70 from the losing army would
mean 100 potential metal and 70 conceded metal. This is an illustrative example,
not a BAR unit statistic.

The standalone tactical simulator does not collect that metal automatically.
The strategic world can credit resources when a builder actually completes
reclaim. Remaining wreck metal and recovered metal are separate quantities.
Wreck diagnostics do not change default training fitness, army investment, or
damage credit. Metal here is not converted into metal-plus-energy investment.

Training assessments and watch summaries present source armies as learner then
opponent, even when the learner started on arena side 1. Campaign dashboards
show mean metal left per battle for each army. Old reports without these counters
show unavailable; mixing old and new evidence also leaves that mean unavailable.
Old recordings still load, with missing per-unit reclaim metal defaulting to zero.

## Approximation limits

The catalog records wreck health, reclaim values, blocking flags, and optional
next-stage definitions. Source exports apply BAR's default wreck/heap health and
metal standardization through the shared unit database code. Commander and effigy
exceptions retain their source values. Collision geometry is reduced to a horizontal radius
and vertical interval, rather than the full BAR model. Missing legacy geometry
falls back to the destroyed unit's dimensions. Tactical ground units use bounded
local routes around blocking neutral features; strategic worlds update their
occupancy-grid routes as features appear and disappear. This does not reproduce
Recoil's complete pathfinding, crushing, or debris physics.

Fire blocking uses the simulator's collision approximation. Ordinary unguided
shots use a predicted intersection chosen when the shot launches; a wreck that
appears after launch is not guaranteed to intercept that already scheduled shot.
Guided projectile handling follows its separate simulated flight path. Treat
these as modeled approximations, not a claim of full BAR/Recoil parity.
