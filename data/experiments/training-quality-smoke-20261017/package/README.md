# Portable MiniBAR campaign

This bundle contains its campaign, stage configs, catalogs, brains, maps and native
simulator. BAR source and the original checkout are not needed. Use the same OS
and architecture as this executable: win32 / AMD64.

From this directory in PowerShell:

```powershell
& './bin/visigoth-simulator.exe' --train-campaign './campaign.json' --train-out './runs/default' --threads 4
& './bin/visigoth-simulator.exe' --train-campaign './campaign.json' --train-out './runs/default' --resume
& './bin/visigoth-simulator.exe' --campaign-worker './jobs/job.json' --job-out './results/result.json' --campaign-config './campaign.json'
```

The worker consumes a coordinator-produced job. Its referenced files must also be
transferred; this package provides runtime assets, not a network scheduler. Native
campaign stages require compiled `.vsgmap` maps, avoiding implicit acquisition
sidecars. To use manual workers, start the coordinator with `--external`; it
returns `waiting_for_workers` and writes jobs under `runs/default/work/jobs`.
Write each result to `runs/default/work/results/JOB_ID.json`, transfer it back,
then rerun the coordinator with `--resume --external`. Repeat for later stages.
Use `--campaign-config` to address the local campaign after relocating a job.

Resume requires the same executable and dependency contents. Training seed ranges
change per generation; validation and holdout use separate fixed seed ranges.
The normal profile also requires fresh paired promotion checks before replacing
its champion. These checks use a separate generation-specific seed range.
Changing a seed/config/input requires a new experiment; the total generation
target may increase. Very short smoke stages can stop with partial construction.

Read RUN_DIRECTORY/progress.json for current stage/job counts and elapsed time;
summary.json reports committed champions. To stop cooperatively, create a STOP
file in the run directory or add --max-seconds N. Checks occur before stages and
job launches, including initial validation and holdout. Active jobs finish/cache,
so the time budget can overrun. Partial generations keep their completed work;
remove STOP and resume with the same inputs to reuse it exactly.

The SHA256 manifest covers every shipped file except the manifest itself. Source and
packaged hashes differ when JSON paths are rewritten. Generated results belong
under `runs/`; package input files must stay unchanged when resuming.
