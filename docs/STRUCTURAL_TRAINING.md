# Watch champions change their structure

Start a new training branch from a completed campaign's champions:

```powershell
cargo build --release --target-dir target/structural-verification
$sim = 'target/structural-verification/release/visigoth-simulator.exe'
python scripts/training_branch.py export --config examples/campaign/structural-search.json --sim $sim --from-run data/brains/campaign --out data/branches/structural-20261018 --structural-every 10 --revalidate-backend
```

Replace `data/brains/campaign` with the source run you want to inherit, and choose
a fresh output directory. Export after that source trainer exits; export takes
a lock to obtain a stable champion snapshot. A run directory contains
`state.json`. You can instead use `--best-dir PATH` to select champions from a
best-brain registry containing `current.json`.

Export copies the champions' complete weights and structure into a separate
branch with fresh optimizer state and random seeds. It preserves the source.
`--revalidate-backend` reevaluates the copied champion weights if the executable
has changed; it does not reuse old fitness measurements or resume the old
optimizer with a new executable. Evidence is saved in `qualification.json`.
That evaluation can take as long as a full validation pass.

Train the new branch and watch its dashboard:

```powershell
& .\data\branches\structural-20261018\Train.ps1 -Threads 4 -MaxSeconds 3600
python scripts/campaign_dashboard.py data/branches/structural-20261018 --watch 20
```

Open `data/branches/structural-20261018/dashboard.html` in a browser. The generator
keeps refreshing that file every 20 seconds. Use another terminal to watch while
training runs. Rerun `Train.ps1` to resume the same branch. Keep the branch's
executable and input files unchanged when resuming.

## What can change

Each brain contains named **modules**, each responsible for one decision. A
module receives named battle facts, passes them through hidden neurons (small
calculations), and produces an instruction. Each neuron combines weighted
incoming signals and a bias (its starting preference).

The structural explorer changes one setting at a time:

- **Activation:** change a hidden neuron's response to tanh, Softsign, or Leaky
  ReLU. tanh is a smooth dial between -1 and +1; Softsign approaches those limits
  more gradually; Leaky ReLU passes positive values through and scales negative
  values to 1%. Final outputs retain tanh and their existing interpretation.
- **Input:** enable or disable one sensor already available to that module.
  Disabled sensors contribute zero. Training does not invent new sensors or add
  sensors from other modules.
- **Connection:** enable or disable one existing connection between adjacent
  layers. Disabled connections contribute zero, but retain their previous weight
  for possible reuse.

The number of neurons and layers stays fixed. There are no backward or skip
connections. Every mutated module output retains at least one path from an
enabled sensor. A path existing does not prove that the sensor matters in a
battle: its weights might be zero, tiny, or canceled by other signals.

## How progress accumulates

An **island** is an independent search branch inside a campaign. One island keeps
training weights normally; another explores structure. In the supplied example,
`local` trains weights and `wide` explores structure.

The structural island proposes an edit in generation 1 and then every ten
generations (1, 11, 21, ...). Each proposal inherits the champion current at that
moment, including all its weights and biases. Changing an activation or disabling
a path can immediately change behavior, despite inheriting the weights.

The proposal trains its own weights until the next interval. Dormant weights
are preserved rather than perturbed or decayed. Structural edits reset that
island's optimizer moments; they do not reset the campaign's generation count.

An experiment replaces the champion only if it passes the existing validation
and promotion checks, including configured stage/faction protection and fresh
paired evaluations. Otherwise the champion survives. Later proposals inherit
accepted changes, allowing successful transformations to accumulate. Holdout
tests report generalization; they never select champions.

## Reading the dashboard

The campaign summary always describes the current accepted champion. The brain
selector changes the network diagram and comparison, without changing training.

Choose a module, a recorded brain, and either its parent or the starting champion
as the comparison. Orange highlights structural changes. Changed disabled edges
remain visible even when other disabled connections are hidden. Input labels
remain visible when disabled. Hidden nodes show their activation function.

The timeline distinguishes proposals still training, evaluated experiments that
have not been promoted, accepted experiments, and experiments replaced before
evaluation. Generation counts training rounds; attempt counts structural
proposals in that island. Proposed snapshots show the edit before its subsequent
weight training. Accepted snapshots contain the trained weights.

A promoted brain can include both an edit and weight training, so a promotion
does not establish that the edit alone caused its improvement. Parent means the
champion an experiment inherited from, or the incumbent an accepted brain
replaced. Weight drift compares matching enabled parameters separately from
structural differences. Historical runs without recorded lineage show their
available endpoints and explicitly say that structural history was not recorded.

## Configuration reference

The example config starts from baseline brains when run directly. Use the export
command above to replace those baselines with your selected current champions.

An island opts into structural search with:

```json
"structural_search": {
  "every": 10,
  "activations": ["tanh", "softsign", "leaky_relu"],
  "activation_weight": 1.0,
  "input_weight": 1.0,
  "connection_weight": 1.0
}
```

The three weights control relative mutation-category frequencies. A zero disables
that category; remaining legal categories are sampled proportionally. At least
one frequency must be positive. `every` must be positive, activation choices
must be unique, and selected trainable modules must permit a legal effective
edit. Frozen modules are preserved. Keep a weight-only island in the track.
Structural search and the older random-module `restart` option cannot be enabled
on the same island.

`--structural-every N` configures an explorer during export while retaining an
ordinary weight-training island. Omit that flag to use the config as written.
Omitting `structural_search` entirely preserves ordinary weight-only behavior.

Version-1 brains load as dense networks with tanh everywhere. New saves use
version 2 and include all input masks, connection masks, and hidden activations.
Checkpoints and portable returns include snapshots, lineage, and optimizer state.
Older executables cannot run version-2 brains. Use the packaged executable for
that branch; import never merges optimizers or trusts remote fitness.
