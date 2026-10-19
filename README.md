# MiniBAR

MiniBAR is a fast, approximate simulator of small [Beyond All Reason](https://www.beyondallreason.info/)
(BAR) battles, together with the tools to train and inspect small neural-network
"brains" that command units inside it. It exists to support one larger goal: an
AI for BAR that is built from several small, readable brains rather than one
opaque one.

It runs on its own. BAR does not need to be installed or running.

## What we are doing

We are building a game-playing AI for BAR called the **Adaptive Command
Hierarchy (ACH)**. Instead of one network trying to play the whole game, ACH
splits the job into levels of command, much like an army does: a strategic level
decides what to build and where to fight, middle levels turn that into missions
for groups of units, and the lowest level decides how a group stands and shoots.

MiniBAR is where the lower levels are built and tested today. It currently
trains two kinds of brain:

- a **tactical** brain that steers combat units: whom to shoot, where to stand,
  when to fall back;
- a **strategic** brain that makes economy and production decisions and picks
  which enemy objective each group of units goes after.

The rest of the hierarchy is a design, not yet code. The design lives in
[ach/](ach/) and [Tiered Brains](docs/Tiered%20Brains.md).

## Why

Three reasons shape everything in this repository.

1. **Training needs millions of fights.** A real BAR match is far too slow to
   learn from. A fast approximate simulator can play many thousands of small
   battles per minute, so a brain can improve by trial and error overnight.
2. **We want to understand the result.** Each brain is a handful of tiny
   networks with plain-named inputs and outputs (`distance` in, `advance` out).
   You can watch a unit decide and see which input drove the decision, rather
   than trusting a black box.
3. **The hierarchy is a bet that has to be tested.** Whether splitting command
   into levels beats one big brain is an open question. A cheap simulator lets
   us run that experiment and keep the answer honest.

**A caution.** MiniBAR is an approximation. Results are useful for comparing
tactics and for training, but they do not establish how a tactic will perform in
a full BAR match. Unit statistics come from BAR's own data, but matching numbers
is not the same as matching behaviour. [Mechanics](docs/MECHANICS.md) and
[Deliberate limits](docs/REFERENCE.md#deliberate-limits) say exactly what is and
is not modelled.

## How it works

```
 BAR unit and map data ──►  simulator  ◄── scenarios (generated fresh each generation)
 (catalog files)            (Rust)
                               │
              many simulated battles, each scored
                               ▼
                 training: try small random changes to a brain,
                 keep the ones that score better (evolution strategies)
                               │
                               ▼
                      champion brain (a JSON file)
                               │
              ┌────────────────┼─────────────────┐
              ▼                ▼                 ▼
        council viewer    explain report    dashboard / video
     (watch it decide)   (what each module  (is training
                          actually does)     making progress?)
```

1. **Simulate.** The Rust simulator plays a battle between two sides. Each side
   runs a **policy**: either a hand-written baseline (`charge`, `kite`, `orbit`
   and others) or a learned brain.
2. **Score.** Each battle gives the learning side a score that rewards winning,
   rewards trading damage well, and counts a win against a stronger force for
   more than a win against a weaker one.
3. **Train.** Evolution strategies make slightly altered copies of a brain, score
   them all on the same battles, and move the brain toward the copies that did
   best. No gradients are needed. Training starts from a hand-written brain that
   already fights sensibly.
4. **Check.** A new brain replaces the current best (the **champion**) only if it
   also wins on separate fights that training never sees.
5. **Inspect.** Viewers replay any battle and show what each small network
   was thinking at each moment.

Unfamiliar words (brain, council, campaign, island and so on) are defined in the
[glossary](docs/GLOSSARY.md).

## Try it

You need a current stable **Rust** toolchain (edition 2024) and **Python 3**
with no extra packages. From the repository root, in PowerShell:

```powershell
cargo build --release
./target/release/visigoth-simulator.exe --catalog examples/pawn-hello.json --unit armpw --left kite --right charge --runs 1 --viewer duel.html
Start-Process ./duel.html
```

This opens a replay of two Pawns, one keeping its distance and one advancing.
On Linux or macOS, drop the `.exe` and open `duel.html` in your browser.

The [getting-started guide](docs/GETTING_STARTED.md) continues from here: it
trains a short demo brain, opens the council viewer to watch it decide, runs a
longer curriculum, and lists other commands to try.

## Where things are

| Folder | What is in it |
| --- | --- |
| [src/](src/) | The simulator (Rust): [src/engine.rs](src/engine.rs) runs battles, [src/learn/](src/learn/) holds the brains, scoring and training. |
| [scripts/](scripts/) | Python tools (standard library only): the viewer server, long-run trainers, dashboards, catalog builders, the BAR unit database. |
| [viewer/](viewer/) | The browser pages: council viewer, battle viewer and training dashboard template. |
| [examples/](examples/) | Unit catalogs, scenarios, training configs and the [campaign](examples/campaign/README.md) files. |
| [data/](data/README.md) | Reference inputs (catalogs, maps, authored starting brains) and, once you train, your outputs. |
| [docs/](docs/) | Guides, findings and design notes; see below. |
| [ach/](ach/) | The Adaptive Command Hierarchy design documents. |

## Where to go next

**I want to run things**
- [Getting started](docs/GETTING_STARTED.md): watch, train, inspect.
- [Reference](docs/REFERENCE.md): every command, the simulation model, scoring, limits.
- [Glossary](docs/GLOSSARY.md): the vocabulary.

**I want to train seriously**
- [Campaign guide](examples/campaign/README.md) and [training contract](docs/TRAINING_CAMPAIGNS.md): long, resumable runs.
- [Training experiments](docs/TRAINING_EXPERIMENTS.md): audited rewards and controlled comparisons.
- [Training branches](docs/TRAINING_BRANCHES.md): continue on another computer and import the results.
- [Structural training](docs/STRUCTURAL_TRAINING.md): let champions change their own network shape.
- [Progress dashboard](docs/TRAINING_DASHBOARD.md) and [cloud video](docs/CLOUD_VIDEO.md): see whether training is working.

**I want to know what the simulator models**
- [Mechanics](docs/MECHANICS.md) and [sensors](docs/BAR_SENSORS.md): line of sight, radar, jamming, cloak, strategic weapons.
- [Wrecks](docs/WRECKS.md): metal left behind by dead units.
- [Unit database](docs/UNIT_DB.md): BAR unit facts, also served to AI agents by the `bar-units` MCP server.
- [Map pipeline](MAP_PIPELINE.md) and [performance](PERFORMANCE.md).

**I want to know what we have learned**
- [Findings](docs/FINDINGS.md): what each experiment showed, newest first.
- [Curriculum review](docs/CAMPAIGN_CURRICULUM_REVIEW.md) and the [latest checkpoint](docs/TRAINING_CHECKPOINT_20261019.md).

**I want to know where this is going**
- [Hierarchical Brains v2](ach/Hierarchical-Brains-v2.md) and [Training Workflow v2](ach/AI-Training-Workflow-v2.md): the ACH architecture and how to train it.
- [Tiered Brains](docs/Tiered%20Brains.md): an earlier layered design, with a list of BAR facts it got wrong.
- [Platoon layer](docs/PLATOON_LAYER_PLAN.md), [objectives](docs/OBJECTIVES_PLAN.md), [replay imitation](docs/REPLAY_IMITATION_PLAN.md), [sensor research](docs/MLP_SENSOR_RESEARCH.md) and [Planetary Annihilation's AI](docs/PA_AI_REFERENCE.md) as prior art.
- [Implementation plan](IMPLEMENTATION_PLAN.md), [strategic design](STRATEGIC_DESIGN.md) and [placement design](DESIGN_PLACEMENT_OCCUPANCY.md): simulator roadmap.

**I want to change the code**
- [AGENTS.md](AGENTS.md): conventions for contributors and AI agents, including how to look up BAR unit facts.
- [Viewer protocol](viewer/README.md): the contract between simulator and viewer.
- Check the build with `cargo test --release` and `python scripts/test_evolve.py`.

## Status

This is a research project in progress. The simulator, training and viewers
work. The `tiered-brains` branch deliberately starts without earlier trained
brains or training output, so a new campaign begins from fresh inputs; the
guides that mention specific saved runs describe an earlier checkpoint. See the
[data policy](data/README.md).
