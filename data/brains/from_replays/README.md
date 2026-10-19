# Replay-informed MiniBAR brains

This package contains runnable starting brains, map conversions, three-faction
training curricula, and short training trials. The starting weights are authored
from [unit and human-match research](../../../docs/MLP_SENSOR_RESEARCH.md).
They are not fitted to replay actions. Training uses MiniBAR rollouts and its
evolution strategy.

From the repository root in PowerShell:

```powershell
cargo build --release
python scripts/prepare_from_replays.py --sim target/release/visigoth-simulator.exe
python scripts/run_from_replays.py --catalog data/replay/catalog-simulable.json --generations 3 --threads 4 --run-name experiment-02
```

Use a fresh run name each time. The supplied `baseline-v1` run is retained for
inspection. Omitting `--catalog` uses the smaller, 22-unit package catalog.
`--setup-only` exports starting brains and configurations without training.
The scripts require local BAR source and captured replays only when preparing
the assets again; the saved catalogs and compiled maps can be used directly.

## Brains and execution

| File | Brain | Decisions |
| --- | --- | --- |
| `tactical/baseline.json` | `tactical_v4`, seven small MLPs | Target, spacing, strafe, cohesion, engagement, encirclement, mission drive |
| `strategic/baseline.json` | `strategic_v1`, three small MLPs | Economic investment, factory production, group mission target |

Each brain JSON names its modules, inputs, outputs, layer sizes, and trainable
weights. Sensor functions calculate the input values; MLPs score choices or
produce movement weights. Hard legality checks remain outside the networks.
All modules can be evolved; `train_modules` can restrict training to named
modules. Older `tactical_v3` brains retain their existing schema and wiring.

The tactical policy evaluates per-unit movement with local and whole-platoon
statistics, allows overlapping weapon capabilities, excludes targets that no
weapon can damage, and considers a strategic objective beyond the nearest three
enemies. It runs in the combat training harness, which exposes both sides.
The strategic policy runs in MiniBAR's economic `World`, reads filtered enemy
observations, and scores construction, production, and objectives for local
combat groups. It retains ongoing builder/factory work. Economic sensors include
income/spending balance, unmet demand, storage and overflow.

These are separate trainable baselines. Strategic rollouts execute combat through
the existing World machinery; they do not yet call the learned tactical brain.
The initial curricula use supported T1 ground/economy roles from Armada, Cortex,
and Legion. Loading the expanded catalog does not train every included unit.
Emergency air defense, independent per-weapon target allocation, full technology
progression and joint training of the two levels remain follow-up work.

## Maps and simulation

`maps/center-command.vsgmap` and `maps/zed-remake.vsgmap` are compiled from real
32-by-32 replay terrain, movement and metal captures. Three synthetic maps
(512, 1024 and 2048 elmos) provide deterministic fixtures. `manifest.json` and
the map provenance sidecars record source hashes, starts, capture spacing and
approximations. The 64-elmo simulation grid does not increase capture detail.

Economy and construction were already implemented in MiniBAR: resource income,
storage, streamed construction spending, shortages, builder travel/assistance,
factory production, repair and reclaim. The strategic curriculum uses them.
Its default match lasts 180 simulated seconds, with decisions once per second,
seeded start jitter and both starting sides tested. Starting resources and a
base income are curriculum settings, not a simulated commander.

Terrain leveling is not simulated. The prepared catalog disables the source
`level_ground` flag so coarse interpolated slopes can pass the retained slope
and water checks. Expanded catalogs are copied to `catalog.expanded.json` with
this adjustment recorded in `catalog.expanded.provenance.json`; their originals
are preserved. Metal integration is approximate. Wind, tidal energy, geothermal
vents, map features and complete game options were not captured. These ground
trials do not validate aircraft, ships, gadgets or native BAR match strength.

Claude's [catalog support audit](catalog_audit/SUPPORT_AUDIT.md) separates
represented mechanics, approximations, missing runtime behavior and converter
gaps. The 354-unit catalog includes entries that need further work before use in
training, including commanders, interceptors, shields and metal makers. The
initial T1 curricula exclude those roles. Solar damage scaling and wreck/death
effects remain approximations even for the bounded roster.
The companion [sensor gap review](catalog_audit/CURRICULUM_SENSOR_GAPS.md)
prioritizes per-weapon targets, threat-domain candidates, air-response fixtures
and economic assets at risk for the next curriculum.

## Continue training or evaluate

Continue from a checkpoint by copying the corresponding training config and
setting `init_from` to that checkpoint (absolute paths are accepted). Relative
paths resolve from the configuration directory; existing working-directory
paths remain accepted for older tactical configs.

```powershell
$sim = '.\target\release\visigoth-simulator.exe'
$assets = 'data/brains/from_replays'
& $sim --catalog "$assets/catalog.expanded.json" --evaluate-strategy "$assets/strategic/baseline-v1/brain.best.json" --train-config "$assets/strategic/baseline-v1/train_config.json" --eval-seeds 2 --eval-seed-base 3000000000
& $sim --catalog "$assets/catalog.expanded.json" --evaluate "$assets/tactical/baseline-v1/brain.best.json" --train-config "$assets/tactical/large_scale_512.json" --eval-seeds 2 --eval-seed-base 3000000000
```

`tactical/large_scale_128.json` runs 64 versus 64 units for each faction.
`tactical/large_scale_512.json` runs 256 versus 256 in four groups per side on
the synthetic 2048 map.
`tactical/center-command.json` and `tactical/zed-remake.json` select real-map
combat arenas. Increase rosters, seeds, duration and populations for broader
experiments after checking map capacity. An incompatible movement class or
illegal formation is rejected instead of silently using a flat arena.

Each training directory saves initial, best and latest brains, its resolved
configuration, summary, generation history and command output. `runs/<name>/`
contains command times, input/checkpoint hashes and additional evaluations.
The best checkpoint is retained by validation fitness, so it can differ from
the latest generation. See [RESULTS.md](RESULTS.md) for measured results and
their limits.
