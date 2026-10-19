# Getting started

A hands-on tour of MiniBAR: watch a simulated fight, train a small neural
policy, and watch it make decisions. Each step builds on the one before, so run
them in order. If you have not yet read what MiniBAR is for, start with the
[README](../README.md).

Unfamiliar words (brain, campaign, island, champion and so on) are defined in
the [glossary](GLOSSARY.md). Every command runs from the **repository root**.

## 1. Watch a fight

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

## 2. Train a policy and watch it think

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
See the [campaign guide](../examples/campaign/README.md) for bounded checks, portable
packages and standalone evaluation workers.
Use the [portable branch workflow](TRAINING_BRANCHES.md) to start another
computer from current champions and import its checkpoints and locally assessed
best brains.
For auditable rewards, independent layouts and controlled training comparisons,
see the [training experiment guide](TRAINING_EXPERIMENTS.md).

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
[Overnight](REFERENCE.md#overnight) for stopping, resuming and limits.

To inspect the saved champion later without continuing training:

```powershell
python scripts/watch.py --catalog $cat --dir data/brains/evolve-demo
```

## 3. Other things to try

These commands use `$sim` and `$cat` from the setup above. The evaluation and
explanation commands use the brain created by the short training run.

| Goal | Command |
| --- | --- |
| Compare tactics over both starting sides | `& $sim --catalog $cat --unit armpw --left kite --right charge --paired --runs 10000` |
| Evaluate a trained brain on fresh seeds | `& $sim --catalog $cat --evaluate data/brains/demo/brain.best.json --train-config data/brains/demo/train_config.json` |
| Write a readable explanation of a brain | `& $sim --catalog $cat --explain data/brains/demo/brain.best.json --train-config data/brains/demo/train_config.json --explain-out demo.md` |
| Check the build | `cargo test --release` and `python scripts/test_evolve.py` |
| Look up or validate BAR unit stats | `python scripts/unit_db.py show cordoom`, or the `bar-units` MCP server ([unit database](UNIT_DB.md)) |

For maps, heatmaps, interactive two-player scenarios, exact replay, calibration
and benchmarks, see the [reference guide](REFERENCE.md).
It also explains how to capture real unit/map data using the separate Visigoth
bridge checkout.

## What is not here yet

The council viewer already lets you choose scenarios, opponents, seeds and
sides, control playback, and inspect decisions. You do not need a new command
for each battle.

The remaining setup would fit a local launcher: choose a catalog and saved
training run, start or resume training, show progress, and open evaluation or
explanation results. Today those steps still use the commands above; the viewer
cannot start or manage training itself. That launcher is the clearest next
improvement to make this workflow easier.
