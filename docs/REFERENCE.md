# MiniBAR reference

Start with the [README](../README.md) for the demo and training workflow.
This guide covers less common commands and simulation details.

- [Duels, traces and heatmaps](#duels-traces-and-heatmaps)
- [Maps and strategic scenarios](#maps-and-strategic-scenarios)
- [Unit data and BAR integration](#unit-data-and-bar-integration)
- [Learned policies](#learned-policies-a-council-of-small-perceptrons)
- [Calibration](#calibration)
- [Performance and development](#performance-and-development)
- [Simulation model](#simulation-model) and [limits](#deliberate-limits)

All commands run from the **repository root**, even though this file is in
`docs/`. Examples use PowerShell:

```powershell
$sim = "./target/release/visigoth-simulator.exe"
$cat = "examples/pawn-hello.json"
```

On Linux/macOS, use `./target/release/visigoth-simulator` and invoke it directly
instead of using PowerShell's `& $sim`. Python scripts use only the standard
library. For the complete flag list, run `& $sim --help`,
`python scripts/watch.py --help`, or `python scripts/evolve.py --help`.

## Duels, traces and heatmaps

```powershell
& $sim --catalog $cat --unit armpw --left kite --right charge --paired --runs 10000
& $sim --catalog $cat --unit armpw --left orbit --right hold --distance 400 --seed 7
& $sim --catalog $cat --unit armpw --runs 1 --trace duel.json
& $sim --catalog $cat --unit armpw --runs 1 --aggregate-viewer heat.html --aggregate-runs 250
```

Policies are `charge`, `hold`, `kite`, `orbit`, or `brain:PATH/TO/brain.json`.
Paired batches run both seatings for every seed to cancel fixed side bias.

What the four built-in tactics do (all fire at the nearest enemy whenever it is
in range):

- `charge`: run straight at the enemy at full speed.
- `hold`: stand still and shoot.
- `kite`: back away while reloading, stop to shoot when the enemy is in range,
  and close in only when it has left range.
- `orbit`: circle-strafe. Every tick it moves sideways (perpendicular to the
  line to its target) at full speed, pushes outward if closer than 0.75 of its
  weapon range, and otherwise drifts inward at only 0.2 of its sideways speed.
  It therefore holds roughly 0.75 range from the target while never standing
  still, so a projectile weapon without perfect lead misses it more often.

`orbit` is slippery, not strong. Measured on 1v1 Pawn duels (240 apart, 3000
paired seeds), each row is the left tactic:

| left vs right | left wins | left loses | notes |
| --- | ---: | ---: | --- |
| `orbit` vs `hold` | 100% | 0% | strafing beats standing still |
| `orbit` vs `charge` | 51% | 45% | a coin flip |
| `orbit` vs `kite` | 0% | 100% | `kite` wins, but takes about 58 s |
| `kite` vs `hold` | 100% | 0% | |
| `kite` vs `charge` | 47% | 50% | |

Two consequences for training. Beating `orbit` is slow (about 58 s of a 60 s
battle cap), so use a longer `max_time` (the overnight config uses 90 s). And
because `orbit` barely pursues, a unit that simply keeps its distance is never
caught: a council trained from scratch against `orbit` alone learns exactly
that, scores zero damage exchanged, and never wins. Warm-started from a council
that already knows how to fight `kite` and `charge`, the same training beat
`orbit` in 1v1, 3v3 and 5v3 within 60 generations (see `evolve.py`'s focus
child below).
Results are JSON; throughput goes to stderr. Metrics include win rates with
Wilson 95% intervals, damage, survival value and overkill.

`--viewer duel.html` writes an animated representative fight; `--trace duel.json`
writes its trace for tooling. `--trace-every N` sets the sampling interval in
ticks. Rendering is a separate run from the batch. The aggregate viewer reduces
sampled runs to occupancy, aim-point and death heatmaps; its memory is bounded
by `--heat-cols` times `--heat-rows`.

## Maps and strategic scenarios

Compile once for repeated batches:

```powershell
& $sim --compile-map examples/river-crossing.map_source.json --map-out river.vsgmap
& $sim --catalog $cat --map river.vsgmap --movement-class BOT --unit armpw --runs 10000
```

`--map` also accepts map-source or bridge hello JSON and compiles it in memory.
Use `--movement-class BOT` (or another available class) if the catalog entry
has no movement class. `--map-cell` sets compilation resolution.

Strategic scenarios add economy, construction, factories and walls. Each needs
a map; these examples work with the bundled catalog:

```powershell
& $sim --catalog $cat --run-scenario examples/wall-defense.json --map examples/flat-pawn-map.json
& $sim --catalog $cat --run-scenario examples/economy-race.json --map examples/flat-pawn-map.json --strategic-only
```

| Scenario | Purpose |
| --- | --- |
| `examples/complete-opening.json` | Constructor builds extractors, solar panels and a factory |
| `examples/economy-race.json` | Expansion race between two sides |
| `examples/wall-defense.json` | Walls and rocket-unit defense |
| `examples/pawn-raid.json` | Pawn flank raid alongside an economy opening |
| `examples/pawn-envelopment.json` | Pawn envelopment formation |
| `examples/river-two-player.json` | River crossing for interactive play |

Offline runs print metrics and a terminal state hash. `--strategic-only` skips
the tactical stepper. `--los-range`, `--radar-range` and `--radar-resolution`
control observations. Radar resolution now specifies smooth position-error
magnitude, default 96 elmos. See [BAR sensor rules](BAR_SENSORS.md) for stealth,
cloak, jamming, building ghosts, refreshed catalogs and the new training profile.

### Live play and exact replay

```powershell
& $sim --catalog $cat --run-scenario examples/river-two-player.json --map examples/river-crossing.map_source.json --serve 127.0.0.1:0 --live-log session.json
```

Open the printed `http://127.0.0.1:PORT` in one or two tabs. Set each tab's
**Control** selector to Blue (side 0) or Red (side 1); each receives its own fog
of war and can command only its own side. The server allocates command sequences
and validates orders, including build previews, queueing, assistance and
cancellation.

Press **Enter in the terminal** to stop and save the log, or supply
`--live-max-ticks N` for a cap. The scenario's offline duration does not limit a
live session. Verify the log against the same initial scenario:

```powershell
& $sim --catalog $cat --run-scenario examples/river-two-player.json --map examples/river-crossing.map_source.json --replay-live session.json
```

Replay fails if the terminal tick or authoritative state hash differs. Live
servers bind to loopback; ordinary headless runs never open a socket.
`--human-side 0` on an offline trace records protocol events for a future driver;
it does not make that replay interactive.

## Unit data and BAR integration

The bundled `examples/pawn-hello.json` is a reference catalog for demos, not a
current balance export. For real experiments, use a versioned catalog from the
Visigoth bridge: its `simulation` fields come from the match's loaded `UnitDefs`
and `WeaponDefs`.

**Capture tools belong to the separate Visigoth checkout.** They are not shipped
here. In that checkout, install the bridge with `python install.py`, then capture
with `python run.py --catalog-out data/simulator-catalog.json` while connected to
BAR. Return here and set `$cat` to the captured file's absolute path.

For native maps, start the desired map in BAR as a full-view spectator and run
`python -m bridge.native_map capture --out-dir data/maps --minibar-cell-size 64`
from the Visigoth checkout. Follow its `docs/NATIVE_MAP_CAPTURE.md` for capture
prerequisites. Pass the resulting map source here with `--map`, or compile it
once to `.vsgmap`. MiniBAR reads captured data, not BAR map archives directly.

The cache retains heights, water depth, named movement passability, resources
and static feature footprints as available in the input. Canonical input
preserves native corner heights and provenance; older canonical files can
recover provenance from a sibling acquisition file. Continuous metal is
preconvolved so extractor-site queries are constant-time. Use
`World::new_with_static_features` to materialize captured blockers as neutral
entities with counted occupancy. See [MAP_PIPELINE.md](../MAP_PIPELINE.md) for
the acquisition contract and metal-map caveats.

For source checkouts of BAR, add `--assets PATH_TO_BAR_SOURCE`. Catalog loading
then resolves `model_name` and `script_name`, parses the S3O piece hierarchy,
finds the piece returned by BOS `QueryWeaponN` (including the legacy primary /
secondary / tertiary names), and caches its projectile emit point. According to
the S3O convention, pieces with zero or one vertex emit at the piece origin and
pieces with at least two vertices emit at their first vertex. Asset processing
happens once and is absent from the simulation hot loop.

If `--assets` is omitted, weapons fall back to the unit collision midpoint.
That is useful for smoke tests, but geometry-sensitive experiments should use
derived origins. Script-controlled alternating barrels or pose-dependent piece
translations remain explicit future work; ambiguous source fails during asset
loading rather than selecting an arbitrary piece.

## Learned policies: a council of small perceptrons

`src/learn/` trains tactical policies that stay readable. Instead of one opaque
network, a **brain** is a _council_ of tiny perceptrons, each with one named
responsibility, labelled inputs and a labelled output. A fixed, hand-written
formula combines their outputs, so every behaviour is attributable to exactly
one module output.

The current wiring is `tactical_v3` (six modules plus a small fixed doctrine):

| module            | owns                                                                      | inputs (distances are in the unit's own weapon ranges)                                                                              | output              |
| ----------------- | ------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| `target_selector` | which of the 3 nearest enemies to shoot (one shared scorer run per enemy) | distance, enemy HP, enemy range, enemy threat (paper DPS ratio), enemy value, is-nearest                                            | `score`             |
| `spacing`         | closing on or backing away from the target                                | distance, own reload, own HP, target range/HP, whether it is approaching, friend/enemy DPS ratio pair, own range rank                | `advance` (−1…+1)   |
| `strafe`          | circling the target                                                       | distance, own reload, approach, target range, own HP, friend/enemy DPS ratio pair, own speed rank                                   | `orbit` (−1…+1)     |
| `cohesion`        | drifting toward or away from friendly units                               | ally-centroid and nearest-ally distance, own HP, friend/enemy count ratio, distance                                                  | `regroup` (−1…+1)   |
| `engagement`      | fight or flight                                                           | friend/enemy ratio pairs for count, HP, DPS, value, speed and short/mid/long-range DPS, own HP, speed/range/HP ranks, platoon power ratio pair | `retreat` (−1…+1)   |
| `encircle`        | formation around the enemy force                                          | bearing around the enemy relative to its line of advance (front alignment, side), angular gap to the nearest friend each way, ring distance | `bloom` (−1…+1)     |

**Force inputs.** Following the approach published for Supreme Commander 2 and
visible in Planetary Annihilation's AI vocabulary, the council reads _ratios_
between the friendly and enemy force within three weapon ranges of the unit.
Each statistic enters twice, as `ours_over_theirs_X` and `theirs_over_ours_X`,
both clamped to 0..1, so the balance is visible whichever side is ahead.
Weapon-range bands are relative to the unit's own range, so "they outrange me"
is visible. The three `*_rank` inputs compare the unit's speed, range and
maximum HP with the best friendly combatant, which lets one council serve a
mixed roster.

`fight = advance·toward + orbit·tangent + regroup·toward_allies + gate·bloom·around_enemy`;
`flee = clamp(retreat, 0, 1)·(1 − commit)`; `move = (1−flee)·fight + flee·away_from_enemy_force`;
`throttle = min(|move|, 1)`; a result under 0.05 means stand still and shoot.

**Doctrine: a platoon that is ahead fights.** `commit` measures how clearly the
_whole side_ (not just this unit's neighbourhood) outguns the enemy side: the
ratio of `DPS × HP` between the sides, 0 at 1.5× and 1 at 2.5× (5 against 3
identical units is 2.8×). Committed platoons do not retreat, and a unit that is
out of contact (more than about 0.8 weapon ranges from its target) is pulled back
toward it: `pull = commit · clamp((distance − 0.8) / 0.4, 0, 1)`, applied as
`move = (1−pull)·move + pull·toward_target`. This is fixed code, not learned,
because a brain that has found a way to score by running must not be able to
leave units idle while their side is winning; it was added after exactly that was
observed (5 council units fleeing 3, two of them parked out of the fight).
`gate = 0.2 + 0.8·commit` scales `bloom`, so encirclement strengthens as the
platoon gets further ahead.

**Doctrine: never sit out a fight your fellows are in.** The pull above also runs at
full strength (as if `commit` were 1) whenever any friendly combatant is already within
weapon range of the unit's target, whatever the platoon balance. It only brings back
units that are out of contact (over about 0.8 ranges away); it does not stop a unit from
keeping its range, and `retreat` still works as before. Measured on replayed battles
(`scripts/flee_report.py`), units more than 1.5 ranges from every enemy while a friend
was engaged fell from 7.0% to 5.1% of unit-time, with the win rate unchanged to slightly
better (72% to 73%).

**Doctrine: objectives.** With a key unit on the other side (see "Generated platoon
matchups and objectives") an attacker moves for it with at least 40% of its movement
(all of it with full commitment) and shoots it once within 0.9 of its range.

**Encirclement.** `encircle` sees the unit's bearing θ around the enemy force
relative to the enemy's line of advance (its mean velocity, or the side facing us
if it is standing still), as `cos θ` (1 = directly in front, −1 = behind) and
`sin θ`, plus the angular gaps to its nearest friendly neighbour counter-clockwise
and clockwise, as fractions of a full circle. Sliding toward the larger gap
spreads friends into an even ring; sliding away from the front puts them on the
flanks and rear. Against a stationary enemy the authored version spans about 50%
of the circle on average, against about 15% with `encircle` silenced.

**The authored starting council.** Training starts (`"start": "template"`, the
default) from a hand-written council, `tactical::template`, rather than random
weights. Each rule is a few weights on named inputs: prefer near, wounded and
dangerous targets; close to about 0.85 weapon range; kite (back away while the
gun reloads); retreat only when the platoon is hopelessly outgunned; slide toward
the larger gap and away from the enemy's front; `strafe` and `cohesion` silent for
training to discover. Without any training it wins 5v3 against `charge`, `kite`
and `orbit` essentially every time (48, 48 and 47 of 48 on the Pawn). A random
start (`"start": "random"`) is still available but must discover fighting from
nothing, and tends to settle on running away. `init_from` takes precedence over
both.
Exact definitions live in `src/learn/tactical.rs` and are stored by name in
every weights file, which also records each module's role and every weight as a
labelled matrix. Hidden layers are small (default 6 units) on purpose.
Brains trained under the earlier `tactical_v1` or `tactical_v2` wiring are
rejected with a clear error, because their modules or inputs differ; retrain them. The reasoning behind this
design, and what was verified about Planetary Annihilation's own networks versus
what is still unknown, is in [docs/PA_AI_REFERENCE.md](PA_AI_REFERENCE.md).

```powershell
# train (CONFIG.json, or `default` for 1v1 / 3v3 / 3v5 underdog / 5v3 favourite of --unit)
& $sim --catalog $cat --unit armpw `
  --train examples/train-tactical.json --train-out data/brains/pawn-skirmish

# read what it learned
& $sim --catalog $cat --unit armpw `
  --explain data/brains/pawn-skirmish/brain.best.json `
  --train-config data/brains/pawn-skirmish/train_config.json `
  --explain-out data/brains/pawn-skirmish/report.md

# use it anywhere a policy is accepted (duels, traces, the HTML viewer)
& $sim --catalog $cat --left brain:data/brains/pawn-skirmish/brain.best.json --right kite --paired
```

**Objective** (`src/learn/objective.rs`, declared in the config's `objective`). Per
battle, from the learner's side:
`fitness = win · outcome · stake + exchange · exchange_term`. `outcome` is
+1 for a win and −1 for a loss; a timeout (both sides alive at the time limit)
is `+timeout_underdog` (default 0.3) when the learner was the underdog, so
surviving a stronger force is a partial win, and `−timeout_penalty` (0.3)
otherwise, so stalling against an equal or weaker force is a partial loss; a
mutual wipe-out is 0. `stake` is the opponent-to-learner starting-value
ratio (capped, raised to `underdog_exponent`) for a win, so beating a stronger
force pays more than beating an equal one, but never less than `win_stake_floor`
(default 1), so a favourite's win is not discounted; for a loss it is the inverse,
so losing to a weaker force costs more than losing as the underdog. The default
`timeout_penalty` is 0.6, which makes stalling against a force you outnumber
about as bad as losing a fair fight. Training maximises not the plain mean but a
**robust score**: `(1 − worst_case_weight)·mean + worst_case_weight·mean of the
worst quarter` of the (scenario, opponent) rows (default weight 0.3), so one
scenario, such as a favourite fight, cannot be traded away for the rest. The exchange
term is the HP-weighted fraction of total starting value the opponent lost
minus what the learner lost, which keeps a signal alive before the first kill.
A **key-unit term** is added to that: units whose definition name is listed in
the config's `key_units` (for example a commander) are flagged on both sides, and
`key_unit_bonus` (default 0.5) rewards the fraction of the opponent's key units
destroyed while `key_unit_penalty` (0.5) charges the fraction of ours lost. This
is the fix for a failure documented in Supreme Commander 2's networks: a score
that only counts the tactical trade teaches units to flee a commander whose death
explosion kills its attackers. It has no effect when `key_units` is empty, and
mutual kills cancel exactly when the two weights are equal.
Metrics stay separate in all reports (win/draw/loss, Wilson intervals, mean
exchange, and win rate specifically in underdog battles).

**Training** is antithetic evolution strategies with rank-shaped fitness and
Adam (`src/learn/es.rs`): gradient-free, deterministic for a given seed, and
parallel across candidates. Every scenario is built twice (learner on side 0 and
side 1) because swapping policies alone would hand an underdog the larger army
half the time. Within a generation all candidates face identical layouts and
seeds; layouts are re-jittered every generation; `brain.best.json` is chosen on
a fixed, un-jittered validation set with a disjoint seed range.

Useful knobs: `train_modules` trains only the named modules and freezes the rest
(add one responsibility at a time and watch its effect); `es.l1` shrinks unused
weights to exactly zero; `init_from` continues from a saved brain; an
`opponents` entry of `brain:PATH` fights an earlier brain. `key_units` flags unit types as key units for the objective's key-unit term.

**Reading the report.** For each module it gives the output's distribution, a
ranked table of how far each input _actually_ moves the output over the range
that input takes in real battles ("swing" and direction, or "ignored"),
one-input-at-a-time sweeps, what each hidden unit reads and pushes, and
complete worked decisions with every named input and output. An input that
never varies in the training scenarios (for example enemy range when every
enemy is a Pawn) is reported as ignored, because nothing could have been
learned from it: use mixed rosters to give `target_selector` real work.

### Running for hours: `scripts/evolve.py`

```powershell
python scripts/evolve.py --catalog $cat --config examples/train-tactical.json --out data/brains/evolve-demo --watch-port 8765
```

**Pawns against Thugs.** `examples/pawn-thug.json` adds Core's Thug (`corthud`, a
T1 plasma bot: 1100 HP, 140 metal, speed 45, one 380-range ballistic cannon doing
104 damage every 1.73 s) to the sample catalog. Its numbers come from BAR's unit
definition and script with the exporter's own conversions, but it is a hand
derivation and not a bridge export (map gravity is assumed to be 130, Recoil's
default). `examples/train-thugs.json` trains Pawns against an equal *metal* worth
of Thugs: 3 against 1, 5 against 2, 8 against 3 and 13 against 5. Start it from a
good Pawn brain:

```powershell
python scripts/evolve.py --catalog examples/pawn-thug.json --config examples/train-thugs.json `
  --out data/brains/thugs --seed-brain data/brains/overnight/final/champion.json
```

Each **round** trains `--children` runs (default 4, sharing the available cores)
from the current champion for `--generations-per-round` generations. Child 0 keeps the incumbent
settings as a control; the others get small random tweaks to `sigma`, learning
rate, `l1`, population, seeds per battle, layout jitter, or train only one or two
modules. The champion and every child are then scored on the same fresh seeds
and layouts against the baselines plus the latest champions, and a child is
promoted only if it beats the champion by `--min-gain`. Promoted champions join
a hall of fame that later children also train against, which guards against
overfitting the three baselines. Only optimiser settings are tweaked; the
objective, scenarios and baselines never change, so scores stay comparable.

**Focus child.** After each round the loop records which baseline the best
candidate scores worst against. In the next round the `--focus-children` children
after the control (default 1) train *only* against that baseline (plus hall-of-fame
opponents) instead of the whole set. Because every candidate is still judged on the
full benchmark, a specialist that forgets the other baselines does not get
promoted, but one that fixes the weakness does. Specializing on `orbit` took a
council's `orbit` win rate from 0% to 78% while its `charge` result held and its
`kite` result fell from 91% to 75%, a net gain the full benchmark accepts.

#### Overnight

```powershell
python scripts/evolve.py --catalog $cat --config examples/train-overnight.json --out data/brains/overnight --watch-port 8765
```

`examples/train-overnight.json` is the example config with a 90 s battle cap and
an extra mild underdog scenario (2v3). It runs until you stop it. Practicalities:

- On Windows the script asks the system not to sleep while it runs; turn the
  display off freely, but do not close the lid on a laptop unless its power plan
  allows it.
- Everything printed is also appended to `evolve.log` in the output directory,
  including each round's per-baseline win rates and the next focus.
- A round that fails outright is retried after 30 s; `--max-failures` (default 5)
  consecutive failures stop the run. Individual failed children are skipped.
- `--max-hours H` stops starting new rounds after `H` hours; it never interrupts a
  round in progress.
- **Stopping.** A file named `STOP` in the output directory is checked between
  rounds, so the round in progress finishes first (rounds can take 5 to 25
  minutes). A file named `STOP_NOW` abandons the round in progress immediately and
  saves what was already promoted. `Ctrl+C` behaves like `STOP_NOW`. Delete either
  file before resuming.
- **Cost.** Selection sees only score, so an unrestrained run drifts toward
  expensive settings: one night's population grew from 64 to 108 and rounds from 5
  to 22 minutes while quality had plateaued. Population, seeds per battle and
  layout jitter are therefore capped (80, 4, 50).
- **Seeding.** `--seed-brain PATH` starts a new run from an existing brain: it
  becomes the first champion (children must beat it) and a hall-of-fame opponent.
- It is safe to stop and resume at any time. The saved champion always reflects the
  best candidate found so far.
- **What is kept when it ends.** Every promoted champion is saved permanently in
  `hall_of_fame/` and `champion.json` always holds the newest, so nothing is lost
  even if the machine loses power. When the run ends normally (`Ctrl+C`, a `STOP`
  file, `--max-hours` or `--rounds`), it also writes `final/`: the champion, a
  measured `report.md` of what each module does, a standalone `watch.html` replay
  file, and `summary.json` (rounds, promotions, simulated hours). If the process
  was killed abruptly (closing the console window, power loss), run
  `python scripts/evolve.py --catalog $cat --config ... --out data/brains/overnight --finalize-only`
  to write `final/` from the saved state. Prefer the `STOP` file for a clean stop
  of a window you cannot reach.

The command above writes to `data/brains/evolve-demo/`; without `--out`, the
default is `data/brains/evolve-<name>/`. Outputs include `champion.json` (always the
current best), `hall_of_fame/`, `history.jsonl` (every candidate's score, win /
timeout / loss rates, underdog win rate and tweaks per round), `state.json`, and
each round's configs and logs. Stop with `Ctrl+C` (the unfinished round is
discarded) or by creating a file named `STOP` in that directory (stops after the
round); run the same command again to resume. If you used a `STOP` file, remove
it before resuming.

### Generated platoon matchups and objectives
Training on a few fixed scenarios lets a council memorise them. A config with a
`matchups` section draws a **fresh batch of random fights every generation** instead
(`src/learn/matchups.rs`; examples: `examples/train-t1-platoons.json`,
`examples/train-t1-objectives.json`):

- Each side fields one platoon, or two that start as separate groups. A platoon is
  **all bots or all vehicles** (hovercraft count as vehicles), never both.
- Platoon types are the squad roles Planetary Annihilation's AI uses (scout, raid, brawl,
  line, artillery), each listing which units can fill it. Half of the platoons are
  **mixed** (several units from the type's pool); the rest are **pure** (one unit type).
- Fights are balanced across **underdog / even / overdog by metal** (the enemy has
  1.25-2x, 0.9-1.1x or 0.5-0.8x the learner's metal). Metal is a poor strength measure
  (24 Fleas against 4 Rockos), so each fight's name also carries the **power ratio**
  (`p0.62`: health x damage per second, the measure the commit doctrine uses).
- `attack_share` / `defend_share` add **objective fights**: an immobile target
  (`targets`, e.g. `armsolar`, `armlab`) sits behind its owner's platoons. Destroying
  the enemy's target wins an attack however many units remain; losing your own loses a
  defence; holding it to the time limit wins one (`src/learn/objective.rs`). The
  opponents `strike` (go for the target) and `guard` (stay near your own target) take
  objectives seriously; the other baselines ignore them.
- Validation, `--explain` and the viewer use one fixed batch; `--evaluate` draws a new
  batch per `--eval-seed-base`. `evolve.py --eval-matchups N` sets how many fights the
  promotion benchmark uses (bigger is steadier; round scores on small batches are noise).

Catalogs for these units come from the game's own definition files, with no running
game: `python scripts/make_catalog.py --base examples/pawn-thug.json --out
examples/t1-catalog.json armpw corthud ...` (`scripts/bar_lua.py` reads the Lua). It
applies the bridge's conversions; assumptions not checked against an export are listed in
the script (map gravity 130, time-averaged missile speed, shared-script yaw instant).

To compare brains fairly, use one large fresh batch and break the results down by
class, power, role, domain, mixed or pure, objective and opponent:

```powershell
python scripts/analyze_matchups.py --config data/brains/t1/base_config.json `
  --catalog examples/t1-catalog.json --exe $sim `
  --brain old=data/brains/overnight/final/champion.json --brain new=data/brains/t1/champion.json
```

### Watching it: the council viewer

`viewer/brain_watch.html` replays battles at an adjustable speed (0.02×–8×,
with presets, pause, single-frame stepping and a scrub bar) next to a graph of
the perceptron that is making the selected unit's decision. Nodes are labelled
with the named inputs and outputs; blue solid edges push a neuron up and red
dotted edges push it down, with thickness showing each edge's actual
contribution (weight × input) this instant. Hover or click a node for its live
formula, switch between the modules, click a council unit to follow it, and
drag any input slider to ask what the module would do *if* that input were
different (the battle pauses; "Return to live" resumes). Hidden activations and
outputs are recomputed in the page from the recorded inputs and the weights in
the brain file, so the graph shows exactly what ran.

```powershell
# follow an evolving champion live: new promotions reload the page on their own
python scripts/watch.py --catalog $cat --dir data/brains/evolve-demo
# or one fixed brain
python scripts/watch.py --catalog $cat --brain data/brains/x/brain.best.json --train-config data/brains/x/train_config.json
# or a standalone file with every scenario x opponent embedded (~7 MB), no server
& $sim --catalog $cat --watch data/brains/x/brain.best.json --train-config data/brains/x/train_config.json --watch-out watch.html
```

The server binds to 127.0.0.1 and simulates battles on demand, so any scenario,
opponent, seed, and side is available; "auto-next" starts the next seed when a
battle ends. Deep links work in both modes, for example
`?battle=4&t=9&pause=1&mod=1&unit=2&node=1,2`.

Limits: tactical micro only (flat arenas, no terrain or LOS inputs, one weapon's
reload and range). The strategic `Strategy`
trait is the intended second consumer of `mlp`/`brain`/`es`.

## Calibration

```powershell
& $sim --compare-calibration FIXTURE.json --observations MEASURED.json --report report.json
```

This compares measurements from Recoil against versioned exact and statistical
tolerances. Replace the uppercase placeholders with actual files. The capture
request template is `examples/calibration/recoil-microcases.request.json`;
engine-side setup is documented in the separate Visigoth checkout's
`docs/CALIBRATION_CAPTURE.md`.

## Performance and development

```powershell
cargo test --release
python scripts/test_evolve.py
& $sim --catalog $cat --unit armpw --benchmark-suite --benchmark-runs 10000 --seed 1
```

The benchmark covers duels, formations, burst/AOE, long-flight projectiles,
blocked terrain LOS, traced output and strategic ticks. It reports simulated
ticks and spawned unit-ticks as well as throughput and capability metadata.
[PERFORMANCE.md](../PERFORMANCE.md) records the baseline and audit;
[IMPLEMENTATION_PLAN.md](../IMPLEMENTATION_PLAN.md) contains the roadmap.

Rayon uses all cores by default. In PowerShell, set `$env:RAYON_NUM_THREADS = "4"`
to limit a process and its children; `Remove-Item Env:RAYON_NUM_THREADS` clears
that override. `evolve.py` assigns thread budgets to its parallel children.

| Path | Responsibility |
| --- | --- |
| `src/engine.rs`, `src/model.rs` | Tactical simulation |
| `src/world.rs`, `src/strategy.rs` | Strategic state and actions |
| `src/map.rs`, `src/routing.rs`, `src/occupancy.rs` | Terrain, navigation and placement |
| `src/learn/` | Learned policies, training, evaluation and explanation |
| `scripts/` | Training orchestration and council viewer server |
| `viewer/` | Live/historical viewer and council viewer |
| `examples/`, `tests/` | Input fixtures and CLI integration tests |
| `data/` | Generated outputs; currently not ignored by Git |

The [shared viewer contract](../viewer/README.md) documents live/historical
bootstrap, frames, provenance and review APIs. Recorded observations are
rendered directly rather than advanced through simulator physics.

## Simulation model

This Rust crate is a deliberately approximate laboratory for tactical ideas. It
does not try to replace Recoil or predict an entire BAR match. It runs small,
repeatable interactions quickly enough to compare movement policies over many
seeds.

The current vertical slice models:

- 30 Hz facing, acceleration, braking, turn rate, turn-in-place behavior, and
  forward/reverse selection, radius-aware terrain clearance, and circular unit
  collision;
- independent turret traverse where BAR supplies `turretspeedx/y`;
- damage **per projectile**, reload, burst size, intra-burst interval, projectile
  count, projectile travel time, angular error, splash falloff, and optional
  damage-over-time duration;
- 3-D direct-fire tests from cached S3O firing origins against terrain and
  simplified vertical unit/wall volumes; one tiny global projectile clearance
  replaces per-projectile collision geometry;
- Recoil-style moving-target lead: nominal flight-time prediction, periodically
  imperfect velocity estimation, `predictBoost`, and `leadLimit`;
- movement-class grid routing over compiled-map passability and layered dynamic
  occupancy, with unit-radius clearance, bounded A\*, smoothing, and
  obstacle-epoch cache invalidation;
- pluggable, read-only `Policy` implementations and built-in charge, hold, kite,
  and orbit baselines;
- simultaneous per-frame damage, explicit overkill accounting, value exchanged,
  damage dealt/received, hit counts, survival value, outcome, and duration;
- BAR's adaptive flanking damage: remembered attack direction, recovery over
  time, and increased damage when attackers spread around a target;
- compact due-tick impact scheduling plus an adaptive uniform broad phase for
  dense blocker and impact-victim queries;
- deterministic seeds and Rayon-parallel batches with reusable per-worker
  trial state and counter-keyed shot randomness;
- a compact generational strategic world for live units, structures,
  nanoframes, factories, walls, and reclaimable features;
- closed-loop income/storage/upkeep, travel/build/assist/repair/reclaim,
  factory queues/stalls/release, wall occupancy, and next-tick activation;
- deterministic strategy actions, resource-conserving strategic score vectors,
  bounded online/paired experiment aggregation, and exact terminal hashes;
- opt-in exact command replay plus a loopback-only HTTP/WebSocket symbolic live
  viewer; neither is initialized by ordinary headless tactical runs;
- versioned Recoil calibration fixtures with exact and statistical tolerances.

`run_parallel_results` retains every trial for a custom scorer;
`run_parallel` returns a compact aggregate with win rates, Wilson 95% intervals,
and mean metric vectors. Both preserve seed order regardless of worker count.
`run_paired_parallel` runs both policies on both sides for every seed and
normalizes output slots back to policy A/B, cancelling fixed side bias.

For performance regression work, `--benchmark-suite --benchmark-runs 10000`
runs fixed 1v1, 16v16, burst/AOE, long-flight, blocked-LOS, traced, and
strategic-only shapes. It reports simulated ticks and spawned unit-ticks in
addition to raw throughput, along with target and capability metadata. The
current baseline and audit are in [PERFORMANCE.md](../PERFORMANCE.md); the dependency-ordered
roadmap is in [IMPLEMENTATION_PLAN.md](../IMPLEMENTATION_PLAN.md).

Paper DPS is derived for reporting only. The engine never applies DPS directly.
Consequently, burst and damage-over-time weapons with the same reported DPS can
produce different outcomes.

### Flanking damage

Flanking defaults to BAR's mode 1, with 1x to 2x incoming damage. Each target
remembers an attack direction independently of its hull heading. Incoming hits
turn that direction using accumulated mobility; hits from other angles then
deal more damage. This applies after armour-table and splash scaling, including
EMP damage, before health, stun, overkill, and score accounting. Shields absorb
the unmodified weapon damage before flanking is considered. The attacker's
position when the hit resolves supplies the direction, including for shots
whose shooter has since died.

Existing catalogs receive these defaults automatically. A unit catalog entry
can supply `"flanking":{"mode":0}` to disable the mechanic, or override
`min_damage`, `max_damage`, `mobility_add`, and `direction` (an object with `x`
and `z`). Direct `UnitDef` JSON uses `strategic.flanking`. Modes 2 and 3 use
unit-relative directions; mode 2 adapts and mode 3 keeps fixed armour facing.
Mobility starts at `mobility_add * 1000` and increases by `mobility_add` every
15 simulation frames (BAR defaults: 10 initially and 0.01 per half second).
MiniBAR projects attack directions onto the horizontal plane and shares the
15-frame update phase across units; Recoil uses 3-D directions and staggered
unit updates. The reference rules are BAR's `gamedata/modrules.lua` and Recoil's
`CUnit::GetFlankingDamageBonus` / `CUnit::DoDamage` in `rts/Sim/Units/Unit.cpp`.

## Deliberate limits

This is an executable approximation, not a claim of engine parity. Strategic
attack orders, construction, walls, deaths, wrecks, resources, and factories
share one deterministic world with the tactical stepper. Unguided gravity
weapons solve low or enforced-high analytical arcs, test terrain and cylinder
blockers along the curved path, and schedule only their due impact—there is no
per-frame projectile allocation. Channeled beams, homing, shields, paralysis,
and interception are also modelled.

The deliberate limits are fixed-speed ballistic flight (no weapon acceleration,
bounce, cratering, or tactic-selected toggleable high trajectory), simplified
projectile radii, and symbolic rather than visual physics. Each mechanism still
needs a small Recoil calibration fixture before it should be treated as engine
parity.
