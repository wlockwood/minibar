# MiniBAR

MiniBAR is a fast, approximate simulator for small Beyond All Reason battles.
Use it to compare tactics, train small neural policies, and watch why units make
their decisions. It also runs scripted economy and construction scenarios.

You can try it with the included data; BAR does not need to be running.
Results are useful for experiments, but do not establish how a tactic will
perform in a full BAR match.

[Sensor rules and training](docs/BAR_SENSORS.md) cover visual sight, radar drift,
stealth, cloak, jamming and remembered buildings.

Unfamiliar words (brain, campaign, island, champion and so on) are defined in the
[glossary](#glossary).

## Start here: watch a fight

You need a current stable **Rust** toolchain with edition 2024 support. The
training viewer also needs **Python 3**, with no extra packages.

Run these commands in **PowerShell, from this repository's root**:

```powershell
cargo build --release
$sim = "./target/release/visigoth-simulator.exe"
$cat = "examples/pawn-hello.json"
& $sim --catalog $cat --unit armpw --left kite --right charge --runs 1 --viewer duel.html
Start-Process ./duel.html
```

This opens a replay of two Pawns: one keeps its distance (`kite`), while the
other advances (`charge`). `armpw` is the Pawn's internal unit name. The terminal
also prints a JSON result. The bundled catalog contains reference data for demos;
use a fresh BAR export for balance experiments.

On Linux/macOS, use `./target/release/visigoth-simulator` for the executable,
invoke it directly with the same flags, and open `duel.html` in your browser.
The examples below use the PowerShell variables above; set them again if you
open a new terminal.

## Train a policy and watch it think

A saved **brain** is a set of six small neural modules that choose a target,
spacing, circling, grouping, retreat and a formation around the enemy. Training
starts from a hand-written council that already fights sensibly. Together these modules are the
**council** controlling a unit. The **council viewer** shows their inputs and
decisions alongside a battle replay.

The example trains Pawn forces in equal and uneven fights against `charge`,
`kite` and `orbit`. Training tries to improve a score based on battle outcomes
and the value of damage exchanged, accounting for relative force strength.

First, train a short demo run:

```powershell
& $sim --catalog $cat --train examples/train-tactical.json --generations 10 --train-out data/brains/demo
```

This writes the best brain and its scenario configuration into
`data/brains/demo/`. Ten generations are enough to try the workflow; omit
`--generations 10` to use the example's 200-generation training run.

Then open the viewer:

```powershell
python scripts/watch.py --catalog $cat --brain data/brains/demo/brain.best.json --train-config data/brains/demo/train_config.json
```

The script starts a local server and opens your browser. In the page:

1. Choose a **Scenario** and opponent, then play, pause, scrub or slow the replay.
2. Click a unit controlled by the council and select a module to inspect its decisions.
3. Hover or click a graph node to see its calculation. Move an input slider to
   try a hypothetical value; **Return to live** resumes the recorded battle.

Keep the terminal running while using this viewer. Press **Ctrl+C** to stop
the server. If the browser does not open, use the address printed in the terminal.

### Train through an expanding curriculum

To train the replay-informed tactical and strategic brains in one command:

```powershell
& $sim --train-campaign examples/campaign/replay-informed.json --train-out data/brains/campaign --threads 4
```

Each population generation runs easy cases, mixed forces/economies, then objectives
before updating weights. The campaign combines local and exploratory searches,
validates champions and saves full optimizer state. Add `--resume` to continue.
See the [campaign guide](examples/campaign/README.md) for bounded checks, portable
packages and standalone evaluation workers.
Use the [portable branch workflow](docs/TRAINING_BRANCHES.md) to start another
computer from current champions and import its checkpoints and locally assessed
best brains.
For auditable rewards, independent layouts and controlled training comparisons,
see the [training experiment guide](docs/TRAINING_EXPERIMENTS.md).

### Keep training while you watch

For a longer session, this one command starts repeated training rounds and
opens a viewer that follows the best policy found so far (the **champion**):

```powershell
python scripts/evolve.py --catalog $cat --config examples/train-tactical.json --out data/brains/evolve-demo --watch-port 8765
```

Each round compares new candidates with the champion on the same fresh test
battles. The page waits for the first champion, then refreshes when a candidate
scores enough better to replace it. **Ctrl+C** stops training and its viewer,
keeping the saved champion but discarding the unfinished round. Run the same
command to resume.

To train overnight, use the overnight config and leave the terminal open:

```powershell
python scripts/evolve.py --catalog $cat --config examples/train-overnight.json --out data/brains/overnight --watch-port 8765
```

Each round it finds the baseline the champion does worst against and dedicates a
candidate to it, while the full test battles stop it forgetting the others.
Progress is also written to `data/brains/overnight/evolve.log`. See
[Overnight](docs/REFERENCE.md#overnight) for stopping, resuming and limits.

To inspect the saved champion later without continuing training:

```powershell
python scripts/watch.py --catalog $cat --dir data/brains/evolve-demo
```

## Other things to try

These commands use `$sim` and `$cat` from the setup above. The evaluation and
explanation commands use the brain created by the short training run.

| Goal | Command |
| --- | --- |
| Compare tactics over both starting sides | `& $sim --catalog $cat --unit armpw --left kite --right charge --paired --runs 10000` |
| Evaluate a trained brain on fresh seeds | `& $sim --catalog $cat --evaluate data/brains/demo/brain.best.json --train-config data/brains/demo/train_config.json` |
| Write a readable explanation of a brain | `& $sim --catalog $cat --explain data/brains/demo/brain.best.json --train-config data/brains/demo/train_config.json --explain-out demo.md` |
| Check the build | `cargo test --release` and `python scripts/test_evolve.py` |

For maps, heatmaps, interactive two-player scenarios, exact replay, calibration
and benchmarks, see the [reference guide](docs/REFERENCE.md).
It also explains how to capture real unit/map data using the separate Visigoth
bridge checkout.

## What belongs in the viewer?

The council viewer already lets you choose scenarios, opponents, seeds and
sides, control playback, and inspect decisions. You do not need a new command
for each battle.

The remaining setup would fit a local launcher: choose a catalog and saved
training run, start or resume training, show progress, and open evaluation or
explanation results. Today those steps still use the commands above; the viewer
cannot start or manage training itself. That launcher is the clearest next
improvement to make this workflow easier.

## Glossary

Terms are grouped by topic, and a term in **bold** inside a definition has its
own entry. Where the code or a config file uses a different spelling, it is
shown in `code`. Options that start with `--` (such as `--train`) are
command-line flags of the simulator program (`$sim` in the commands above).

**How the pieces fit.** A **brain** is a small set of readable neural networks
that decides what units do. **Training** improves a brain by trial and error:
each **generation** it makes slightly altered copies (**candidates**), has each
play simulated battles and receive a score (**fitness**), and shifts the brain's
numbers toward the copies that scored best. A candidate that also does well on
separate **validation** fights can replace the **champion**, the best brain so
far. A **campaign** runs this for a long time on two independent **tracks**
(tactical and strategic). Each track is trained through several **stages** of
increasing difficulty by several **islands** that search in different ways.
`scripts/evolve.py` is a simpler alternative to a campaign with its own words
(**round**, **hall of fame**), explained under **Round**. Entries below are
ordered by topic, so some use a term that is defined further down.

### Policies and brains

- **Policy**: the rule that decides what each unit on a side does, moment to
  moment (see **Tick**). It is either scripted (a **baseline**) or learned (a
  **brain**). A side has one policy, which every one of its units runs on its
  own surroundings. Anywhere MiniBAR accepts a policy, `brain:PATH` loads a
  saved brain.
- **Baseline** (also *baseline opponent*): a fixed, hand-written policy used as a
  sparring partner and yardstick. Nothing about it is learned. The built-ins are
  `charge` (advance on the nearest enemy), `hold` (stand and shoot), `kite` (keep
  distance), `orbit` (circle the enemy), `strike` (drive at the enemy **key
  unit**) and `guard` (stay near your own key unit).
- **Perceptron**: a very small neural network: numbers go in, pass through a
  few *hidden units* (each one takes a weighted sum of the numbers before it
  and applies a response function), and one number comes out. By default that
  response is `tanh`, a smooth dial between -1 and 1, and every unit connects
  to every unit in the next layer. Structural training can change hidden
  response functions and enable or disable inputs and connections; final
  outputs still use `tanh`. The hidden layer is deliberately tiny (6 units
  by default) so a person can read it. In MiniBAR, "perceptron" and
  "multi-layer perceptron" (`Mlp` in the code) mean the same thing.
  See [evolving champion structure](docs/STRUCTURAL_TRAINING.md) to start from
  existing champions and watch their networks change in the dashboard.
- **Module**: one perceptron with one named job, such as `spacing` or
  `target_selector`. Its inputs and its output are labelled with plain names
  (for example `distance` in, `advance` out).
- **Council**: the modules plus the fixed, hand-written formula that combines
  their outputs into a movement and a target for a unit, so every behaviour
  traces back to exactly one module output. The formula contains no learned
  numbers: everything learned lives inside the modules. "Council" names the
  structure; a **brain** is a council with particular learned numbers.
- **Brain**: a council with its learned numbers (**weights**) and structure filled in, usually
  as a file: one self-describing JSON text file listing every module's weights
  (`brain.best.json`, `champion.json`). "A brain" can also mean the version of
  that file currently being trained or tested. There are two types. The
  *tactical* brain steers individual combat units (where to stand, whom to
  shoot, when to retreat) and has six modules (`target_selector`, `spacing`,
  `strafe`, `cohesion`, `engagement`, `encircle`). The *strategic* brain makes
  economy and production decisions (what to build, what factories produce, which
  enemy objective each group of combat units goes for) and has three modules
  (`investment`, `production`, `mission`). Each type is trained as its own
  **track**; the two are not yet connected, so the strategic brain does not
  command the tactical one.
- **Weights**: the numbers inside a module that training changes. Two brains
  with the same **wiring** differ only in their weights.
- **Wiring** (the `kind` field in a brain file): the exact list of modules,
  inputs and outputs a brain has. `tactical_v3` is the third numbered revision
  of the tactical wiring and `strategic_v2` the second of the strategic one. A
  brain can only be loaded under the wiring it was trained with; older wirings
  are rejected with an error and must be retrained (a strategic v1 brain can be
  upgraded to v2).
- **Doctrine**: fixed rules in code, outside the council, that training cannot
  change. For example, a platoon that is clearly ahead does not retreat. They
  exist because a brain that scores by running away must not be able to leave
  units idle while their side is winning: that was observed, so the rule was
  taken out of the learned part.
- **Commit**: a 0 to 1 number measuring how clearly a whole side is stronger
  than the enemy side. A side's strength is its total *HP* (health points)
  times its total *DPS* (damage per second). With our strength divided by
  theirs, commit is 0 up to 1.5, 1 from 2.5 upward, and ramps between. A
  committed platoon does not retreat, and a unit that has drifted away from the
  fight is pulled back toward its target; both are doctrine rules.
- **Template** (`"start": "template"`): the hand-written starting brain that
  training improves on, instead of beginning from random weights. It already
  fights sensibly. A campaign's starting brain (`baseline.json` in the shipped
  files, not to be confused with a **baseline** opponent) plays the same role:
  authored, not learned.
- **Personality**: *not a MiniBAR term.* It comes from Planetary Annihilation's
  AI, where a personality is a named bundle of AI settings (including an
  "aggression" scale on the AI's input ratios). It appears only in
  [the PA reference notes](docs/PA_AI_REFERENCE.md). MiniBAR has no personality
  setting; how a side behaves is chosen by its **policy**, that is, a **baseline**
  or a **brain**.

### Fights and scenarios

- **Tick**: one step of the simulation, 1/30 of a second. A 2700-tick scenario
  is 90 simulated seconds.
- **Learner**: the side whose brain is being trained or tested in a fight, as
  opposed to the opponent. Scores are always from the learner's point of view.
- **Faction**: one of BAR's three playable sides (Armada, Cortex, Legion), each
  with its own unit roster. Some campaign options check that a brain does not get
  worse for one faction while improving on another.
- **Roster**: the list of unit types, and how many of each, that a side starts a
  fight with.
- **Scenario**: a starting setup for one fight (map, forces and positions).
  The same scenario is normally run twice, with the learner on each side, so
  neither side gets a built-in advantage.
- **Platoon**: one group of units on a side, controlled together. A generated
  platoon is either all bots or all vehicles (hovercraft count as vehicles),
  and is *pure* (one unit type) or *mixed* (several).
- **Matchup**: a generated fight between platoons, drawn fresh each generation
  so a brain cannot memorise a fixed set of scenarios.
- **Underdog / even / overdog**: how the learner's army compares with the
  enemy's in *metal*, the in-game resource units are paid for with, so metal
  cost is a stand-in for army size. A generated underdog fight gives the enemy
  1.25 to 2 times the learner's metal, an even fight about the same, an overdog
  fight 0.5 to 0.8 times. Metal is a rough guide to strength (24 cheap units can
  lose to 4 expensive ones), so a generated fight's name also shows the
  *power ratio*, such as `p0.62`: the learner's strength divided by the
  enemy's, where strength is HP x DPS. `p0.62` means the learner has 62% of the
  enemy's strength.
- **Key unit**: a unit type flagged as the one that matters, such as a
  commander. Killing the enemy's is rewarded and losing your own is penalised.
- **Objective fight**: an attack-or-defend scenario around an immobile target
  (a building). Destroying the enemy's target wins an attack, losing your own
  loses a defence, and holding it until the time limit wins a defence.
- **Catalog**: the unit data file (`--catalog`): costs, health, speed, weapons.
  The bundled ones are small samples for demos; use a fresh BAR export for
  balance work.
- **Seed**: the number that fixes every random draw in a fight or a training
  run, so the same seed reproduces the same result. *Jitter* randomly shifts
  starting positions so a brain cannot rely on exact spots.

### Training a brain

- **Training**: improving a brain's weights by repeated trial: try variations,
  score them in simulated fights, and move toward the better ones. No
  gradients are used; the simulator is treated as a black box.
- **Evolution strategies** (**ES**): the training method. Each **generation**
  it makes a population of **candidates** by adding random noise to the current
  weights, scores them all, and moves the weights toward the noise that scored
  well. Noise comes in plus-and-minus pairs (*antithetic*: the same noise added
  and subtracted), only each candidate's rank in the scores is used (not the
  score itself), and the step is smoothed by *Adam*, a standard
  neural-network training rule that keeps each update steady; you can ignore it
  to follow the rest. The code is [src/learn/es.rs](src/learn/es.rs).
- **Candidate**: one noisy copy of the current weights, being tested.
- **Population**: the number of candidates per generation. It must be even
  because of the plus-and-minus pairing.
- **Sigma**: how large the random noise is: the typical size of the nudge added
  to each weight. A candidate at sigma 0.16 is nudged about four times as far as
  one at 0.04. Small sigma refines; large sigma explores.
- **Center**: the weights the search is currently built around; candidates are
  noisy copies of it. It moves a little each generation toward what scored well.
  The center is not the **champion**: the center is where the search stands, the
  champion is the best brain that has passed the checks below, and the best
  candidate of one generation is a third thing. The center often trails the
  champion or is a different brain altogether.
- **Generation**: one cycle of "make candidates, score them, update the
  center". The unit of progress for `--train` and for **campaigns**.
  `--generations N` is a total target, not an extra count.
- **Fitness**: the score a candidate gets; higher is better. This is the
  tactical score (the strategic track scores surviving economy, production and
  army value instead). For one battle, from the learner's side:
  `win weight x outcome x stake + exchange weight x exchange + key-unit term`.
  The weights default to 1 and 0.5. The *key-unit term* is 0.5 times the
  fraction of the enemy's **key units** destroyed, minus 0.5 times the fraction
  of ours lost, and is zero when no unit type is flagged as key.
  *Outcome* is +1 for a win, -1 for a loss, 0 for both sides wiped out on the
  same tick, and for a **timeout** +0.3 if the learner was the underdog or -0.6
  if not.
  *Stake* is how much the fight was worth, from the ratio of the two sides'
  starting value (roughly their cost): beating a force twice your size counts
  double a fair fight (capped at 4 times), losing to a weaker force costs
  more than losing to an equal one, and a favourite's win still counts as much
  as an even fight, never less.
  Tactical training ranks candidates by a *robust score* rather than the plain
  mean: 70% of the mean fitness across all (scenario, opponent) pairings plus
  30% of the mean of the worst quarter of those pairings. Each pairing is one
  *row*: one scenario against one opponent. The point is that no scenario can
  be traded away for the rest. Details are in
  [the reference](docs/REFERENCE.md#learned-policies-a-council-of-small-perceptrons).
- **Exchange**: the HP-weighted fraction of the combined starting value that
  the enemy lost, minus what you lost. It is the second term in fitness, and
  gives training a signal before the first kill.
- **Timeout**: a fight where both sides are still alive at the time limit.
  It scores as a partial win for an underdog and a partial loss otherwise, so
  stalling against a weaker force is punished.
- **Round**: used by `scripts/evolve.py`, the simpler way of training for hours
  (a **campaign** is the other, and the two are alternatives, not layers). A
  round starts several short training runs (*children*, each a few
  **generations** long) from the current champion, then judges the champion and
  every child on the same fresh battles. A child replaces the champion only if
  its score is higher by at least `--min-gain`, in the same score units as
  **fitness**. A *focus child* trains against
  only the opponent the champion does worst against. The *hall of fame* keeps
  every promoted champion, and later children train against them as well as the
  baselines (`charge`, `kite` and `orbit` in the example config) to avoid
  overfitting to those three.
- **Champion**: the best brain a run has accepted so far, saved as the run's
  result: `tracks/NAME/brain.best.json` in a campaign, `champion.json` for
  `evolve.py`. Only a challenger that passes **validation** and **promotion**
  replaces it. At the start the champion is the starting brain.
- **Challenger**: a brain put forward to replace the champion. It is not the
  same as a candidate: a candidate is a noisy copy tested during training, while
  a challenger is an island's center or its best candidate of that generation,
  once it is entered into validation.
- **Validation**: scoring a brain on a fixed set of scenarios and seeds that
  training never uses. The champion is scored the same way, so the two scores
  can be compared fairly. Because the same set is reused, a brain can slowly
  become tuned to it, which is one reason the extra head-to-head in
  **Promotion** and the **holdout** exist.
- **Promotion**: a challenger replacing the champion. In a campaign it is
  attempted every `validation_every` generations (5 in the shipped campaign).
  A challenger is promoted only if all three hold: its validation score beats the
  champion's by more than `min_gain` (0.005 in the shipped campaign, in fitness
  units); no **stage** falls more than `max_stage_regression` (0.15) below the
  champion's; and in a final head-to-head it beats the incumbent on a majority
  of 4 new seeds (`promotion_seeds`) that are different every generation. A
  "pair" there is the challenger's result and the incumbent's on the same
  seed. (`evolve.py` promotes by the simpler rule in **Round**.)
- **Holdout**: a final test on maps or rosters used for neither training nor
  validation. It never selects a champion; reusing its result to tune
  experiments would turn it into a validation set.
- **Overfitting**: a brain that scores well on training scenarios but worse on
  others. Validation, holdout and a changing `matchups` batch exist to catch it.

### Campaigns

- **Campaign**: one long, resumable, checkpointed training run described by a
  campaign file such as `examples/campaign/replay-informed.json`. It trains
  every **track** through every **stage** each generation, validates and
  promotes champions, and saves complete state so it can stop and continue.
  Started with `--train-campaign`. Not the same as a BAR game campaign.
- **Track**: the training of one brain type within a campaign. There are two:
  `tactical` and `strategic`. A track has its own starting brain, stages,
  **islands**, champion and archive, so a campaign finishes with one champion
  per track. Their scores mean different things and are never combined.
- **Stage**: one named group of scenarios in a track, with a difficulty number
  (which only orders the stages) and a weight (its share of the average). The
  default stages are `easy` (0.25), `mixed` (0.35) and `objectives` (0.40).
  Every candidate plays all stages, and the island updates *once* from their
  weighted average, so hard stages cannot erase easy skills.
- **Curriculum**: the ordered set of stages. "Expanding" means the harder stages
  are added to, not substituted for, the easy ones.
- **Island**: one independent ES search inside a track, with its own **center**,
  **population**, **sigma** and random noise. Each island's center starts as the
  track's starting brain and then moves on its own: an island never reads
  another island's center. What they share is the target. Every island is
  scored on the same stages and seeds, and all of them try to beat the one track
  **champion**: after each island's update, its center and its best candidate
  are validated as challengers. Having several islands lets one track search in
  different ways at once, for example carefully and boldly (see the two below);
  a lucky find on any island can become the champion. A track's islands are
  listed in the campaign file; the shipped campaign gives each track two,
  named `local` and `wide`.
- **Local island** (`local`): the island that refines. In the shipped campaign
  it has a large population (32) and small sigma (0.04), so its candidates stay
  close to its center and it makes steady, careful improvements. It is not set
  to restart.
- **Wide island** (`wide`): the island that explores. In the shipped campaign
  it has a small population (8) and large sigma (0.16), so its candidates land
  far from its center and it can find changes the local island would not
  reach, though most of what it tries is worse. It is the one set to
  **restart**. With these numbers the local island runs four times as many
  candidates, so about 80% of evaluations go to local and 20% to wide.
  Populations, sigmas and which islands restart are all settings in the
  campaign file, not fixed by the program.
- **Restart**: a reset for an island that has gone stale. If a restartable
  island has not had its own center or best candidate promoted to champion for
  `restart_after` generations (30 in the shipped campaign), its center is
  replaced by the champion's weights with one randomly chosen module overwritten
  by small random numbers (size `restart_scale`), and its Adam memory is
  cleared. The champion itself is unchanged; the restart only gives that island
  a fresh place to search from.
- **Archive**: a bounded list of a campaign's past champions that training
  occasionally fights, so a brain must still beat its own earlier versions.
  (`evolve.py`'s *hall of fame* does the same job.)
- **Frontier** (optional, `frontier_limit`): a small set of validated brains
  that were not made champion but trade off differently (better on some stage or
  faction, worse on another), kept for study. It never influences which brain
  becomes champion.
- **Reference brain**: the fixed brain a track's results are compared against
  when the optional faction-regression guard (`max_faction_regression`) and the
  frontier are enabled. By default it is the track's starting brain.
- **Checkpoint**: the state saved after each completed generation
  (`state.json`). A partial generation is never saved; resuming recomputes it,
  reusing finished work. `STOP` in the run directory, or `--max-seconds`, stops
  cleanly between jobs.
- **Package**: a self-contained folder or ZIP of a campaign (executable,
  configs, catalog, maps, brains and SHA256 hashes) that runs on another
  computer of the same operating system and architecture.
- **Branch**: a package exported from the current champions with new random
  seeds, so another machine explores different work. Its results are imported
  back and assessed locally before a brain is used.
- **Job**: one unit of simulation work: one candidate played through one stage.
- **Worker / external**: normally the campaign program (the *coordinator*) runs
  every job itself. With `--external` it instead writes the jobs as files and
  waits; a worker (`--campaign-worker`, possibly on another computer) runs one
  job and writes one result file. Transfer is manual today.

### Reading and watching

- **Council viewer**: the replay page (`viewer/brain_watch.html`, started with
  `scripts/watch.py`) that plays a battle next to a graph of the module making
  the selected unit's decision. Edges show each weight times its input at that
  instant; sliders ask what the module would do if an input were different.
- **Explain report** (`--explain`): a written report of what each module in a
  brain does, measured from real battles, including which inputs it actually
  ignores.
- **Heartbeat**: the status line the campaign prints every 30 seconds.
  `progress.json` holds the same live status in a file.

### Source data

- **BAR**: Beyond All Reason, the game being approximated.
- **Recoil**: the engine BAR runs on. Units, maps and distances (*elmos*,
  the engine's length unit) follow its conventions.
- **Visigoth**: the separate checkout and bridge that captures real unit and map
  data from BAR. MiniBAR reads what it exports.
- **VSGMAP** (`.vsgmap`): a compiled map cache. Campaigns accept only these,
  never raw map JSON.
- **Replay-informed**: the name of the shipped campaign
  (`examples/campaign/replay-informed.json`). Its unit catalog and authored
  starting brains were built from human game replays; training then improves
  them by simulated fights, not by copying the players.

## Further reading

- [Command and simulation reference](docs/REFERENCE.md) — advanced workflows,
  training details, scoring, supported mechanics and limits.
- [Performance](PERFORMANCE.md) — benchmarks and baseline measurements.
- [Implementation plan](IMPLEMENTATION_PLAN.md) — development roadmap.
- [Map pipeline](MAP_PIPELINE.md) — terrain and resource data formats.
- [Viewer protocol](viewer/README.md) — integration contract for live and historical views.
