# Independent MiniBAR training branch

Unzip the complete folder on another Windows computer with the same architecture
as the shipped executable. Rust, Python and a BAR checkout are not needed there.

From this folder, in PowerShell:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\Train.ps1 -Threads 4 -MaxSeconds 28800

Repeat the command to resume. -Generations N sets a total target (default: the
campaign target). The time budget is cooperative: active jobs finish/cache.
Create run/STOP to stop before subsequent jobs; remove it before resuming.
Read run/progress.json for current stage/jobs and run/summary.json for champions.

After the trainer exits:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\Return-Results.ps1

Send the resulting sibling *-results.zip back to the original computer. There:

    python scripts/training_branch.py import RETURN.zip --package ORIGINAL_EXPORT --out data/brains/branches --against-run LOCAL_RUN --best-dir data/brains/current-best

The original export folder or ZIP is required to verify immutable inputs.
Import preserves the complete checkpoint and partial work in a separate branch.
It never combines optimizer states. Champion integration reevaluates both sides
locally, including fresh paired seeds; holdout does not select. Duplicate returns
do not overwrite local progress. Future exports can use --best-dir.

This branch starts from validated champion weights, with new optimizer/random
streams, so it explores different work. A repeated explicit seed intentionally
recreates the same branch. Keep all shipped input files unchanged. Manifest hashes
detect accidental edits for trusted computers; this is not remote authentication.
Numeric equivalence across physical computers has not yet been demonstrated.
