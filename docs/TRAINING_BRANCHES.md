# Training on another computer

The [current-best Windows ZIP](../data/packages/training-current-best-20261017.zip)
is ready to copy. It contains the simulator, both validated champion brains,
normal training curriculum, maps, catalogs, hashes and PowerShell launchers.
The other computer needs the same operating system/architecture as the included
executable. Rust, Python and a BAR checkout are unnecessary on that computer.
The exporting/importing computer uses Python 3.10 or newer.

Each export starts an **independent branch**: current champion weights become
fresh optimizer centers, and new campaign/island random seeds explore different
work. Existing Adam moments and candidate centers stay with their original run.
Shared validation conditions deliberately remain comparable. Copying one export
to two machines and running both repeats the same branch; export another package
to get another random branch. `--seed N` deliberately makes an export repeatable
from the same source checkpoint/config.

## Export

From the repository root, using the current validated run:

```powershell
python scripts/training_branch.py export --from-run data/brains/campaign-readiness-20261017/run --out data/packages/next-training-branch
```

This creates a folder and sibling `.zip`; both destinations must be unused.
`--config` defaults to `examples/campaign/replay-informed.json`, and `--sim` to
`target/release/visigoth-simulator.exe`. You can supply another campaign or matching
executable. The source run must be stopped, with validated champions for every
target track, and use that exact executable. Export does not change its inputs,
checkpoint or brains. The native binary validates the packaged configuration
before publication. The printed branch ID and seeds are also in `branch.json`.

After importing and assessing results, export from the best-brain registry:

```powershell
python scripts/training_branch.py export --best-dir data/brains/current-best --out data/packages/another-training-branch
```

Keep the original export folder or ZIP locally: import uses its manifest to check
that returned results used exactly the shipped inputs and executable.

## Train and return

Unzip the whole package elsewhere. From its folder, run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\Train.ps1 -Threads 4 -MaxSeconds 28800
```

Repeat the command to resume. `-Generations N` sets a total target; otherwise the
campaign target applies. With `-MaxSeconds 0` or no budget, training continues
until that target. The budget is cooperative: already running jobs finish and
cache results. Creating `run/STOP` stops subsequent launches; remove it before
resuming. `run/progress.json` reports the current stage and job counts;
`run/summary.json` reports committed champions. All candidates complete easy,
mixed and objective stages within each generation as in local campaigns.

Once the trainer has exited:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\Return-Results.ps1
```

This creates a sibling `PACKAGE-NAME-results.zip`. Use `-Out PATH.zip` to choose
another unused destination. The return script checks input hashes and locks
against active training while snapshotting. It includes the complete portable
inputs, authoritative checkpoint, optimizer state, champion/archive/history and
partial-generation work files. Reports and brain exports are regenerated from
the checkpoint on import. STOP markers, locks and unrelated files are excluded.
The original remote run remains resumable.

## Import and select champions

Copy the result ZIP back, then run from the original checkout:

```powershell
python scripts/training_branch.py import RETURN.zip --package data/packages/next-training-branch --out data/brains/branches --against-run data/brains/campaign-readiness-20261017/run --best-dir data/brains/current-best
```

Use the export that actually produced `RETURN.zip` in `--package`; an original
ZIP works too. For the prebuilt current-best package linked above, use
`--package data/packages/training-current-best-20261017` instead.

Import checks ZIP paths, hashes, original input identity and executable identity,
then asks the native binary to validate the checkpoint and cached work. The
returned branch is saved under `OUT/BRANCH_ID/SNAPSHOT_HASH`, with its own
`campaign.json`, executable, assets and `run` directory. It can continue locally
using its `Train.ps1`. The original local training run stays intact.

Duplicate detection includes **both committed state and partial work**. A later
return with the same generation but more completed jobs becomes another saved
snapshot. An identical return reports `duplicate`, including after you have
continued its earlier import locally; it does not overwrite that progress.
Divergent snapshots remain separate. The import receipt is `import.json`.

With `--best-dir`, selection proceeds as follows:

1. For a changed returned champion, re-evaluate it, the local incumbent and the
   initial reference on the **same packaged validation plan**. Remote fitness
   claims do not select the winner. Require aggregate gain and protect every
   stage against excessive regression below incumbent and initial reference.
2. Compare a surviving candidate and incumbent on at least four fresh paired
   seeds, requiring mean improvement, a strict majority of pair wins, and stage
   protection. Import uses at least four pairs even for a smoke campaign whose
   normal training promotion check is disabled. Holdout never selects.
3. Publish accepted brain weights into immutable objects referenced by
   `BEST_DIR/current.json`. Keep incumbents for rejected candidates. Save full
   fixed/fresh evidence under the imported branch's `assessment` directory.

`--against-run` supplies the initial incumbents and reference brains when creating
the registry; stop that run first. Once the registry exists, its current brains
supply the incumbents. Identical brains need no reassessment. Repeating an import
does not reroll conditions or overwrite an existing completed assessment.

Omit `--best-dir` to import a resumable branch without selecting champions.
Accepted weights seed future exports via `--best-dir`; existing campaign
optimizer states are never averaged or replaced. If reassessment fails, the
validated branch remains imported and the same command can retry assessment.

These are conservative selection guards, not a significance test. The package
hashes support trusted machines and detect accidental mismatch; they are not
remote authentication. A real multi-host scheduler and cross-machine floating
point equivalence remain outside this workflow.

## Verification

Focused tests exercise the actual release executable and Windows PowerShell,
including training, complete return/import, continued native state equality
after relocation, extra cached work with unchanged state, duplicate protection,
active-run locks, malformed archives, corrupted checkpoints/caches, registry
snapshot consistency, and local rejection of inflated remote fitness claims.

The [normal-package transfer receipt](../data/branches/transfer-proof-20261017/receipt.json)
records extraction of the shipped 31-file package, PowerShell startup at a tiny
budget, return, native import, unchanged-champion registry bootstrap and duplicate
import. It deliberately does not run a normal-profile training generation. The
focused fixtures separately train and resume complete generations. All probes
ran on one Windows host; the destination computer has not yet been qualified.

```powershell
python -m unittest discover -s scripts -p test_training_branch.py -v
```

See [campaign behavior](TRAINING_CAMPAIGNS.md) and
[prior measured campaign results](../examples/campaign/RESULTS.md) for simulation
scope and training limitations.
