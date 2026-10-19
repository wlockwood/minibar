# Continue evolving the champions

The next campaign is ready in `data/branches/tactical-recovery-20261018`.
Its tactical brain is exactly the champion from generation 20 of
`data/branches/structural-20261018`, whose checkpoint contains 46 completed
generations. This starts fresh optimizers and random streams from learned
weights; it does not discard the learned brain. The old run stays intact.

Start it in your PowerShell terminal from the repository root:

```powershell
.\data\branches\tactical-recovery-20261018\Train.ps1 -Threads 32
```

The launcher now defaults to all available logical processors. On this machine
that is 32. Repeat the command to resume after stopping. The terminal prints
progress and generation summaries. No trainer was left running during setup;
the preflight committed zero training generations.

To keep the dashboard current, use a second terminal:

```powershell
python scripts/campaign_dashboard.py data/branches/tactical-recovery-20261018 --watch 20
```

Open `data/branches/tactical-recovery-20261018/dashboard.html`. It initially
shows a waiting state until the trainer completes its initial validation.

On Windows, the dashboard closes checkpoint files before parsing them. The
launcher also retries transient checkpoint-save access errors up to five times,
resuming the same executable, committed optimizer and cached work. Retries share
the original time budget. If updating an already running dashboard process,
stop it with Ctrl+C and run its command again to load the reader fix. A failed
save does not require starting the brain over.

## What changed

| Change | Result |
| --- | --- |
| Continuous champion refinement | The `local` weight-only branch adopts the champion's complete weights and structure at the next generation boundary. Optimizer moments reset on adoption, while the generation counter and noise stream continue. |
| Longer structural trials | `wide` trains each structural proposal for 30 generations before trying another change. Activation functions, inputs and connections can still change. |
| Strategic signal investigation | Nearby perturbations produced identical detailed outcomes on easy and mixed scenarios in the probe; objectives still distinguished some candidates. This supports a local plateau, not a claim that strategic tasks are solved. Rewards were left unchanged. |
| Separate screening and acceptance | A positive improvement on a rotating paired screen can advance, even below the old fixed aggregate score. The final minimum gain is 0.001. |
| Decision explanations | New generation records and the dashboard distinguish screening failures, insufficient fresh gains, failed majority/evidence checks, and stage/faction regressions. Old records retain their original, less detailed history. |
| Fresh tactical retest | The current local centre and champion were compared on 96 paired batches, with two fresh layouts per batch and all validation scenarios/opponents. No holdout was used. |
| Promotion gate repair | Candidate and champion share a fresh screening batch. A separate set of 12 fresh paired batches decides promotion, requiring sufficient gain, a majority of wins, stage protection and a configured paired evidence threshold of 2. Fixed audit stage floors remain protected. |
| Tactical focus | Only tactical trains. The strategic champion is preserved in `parked/strategic.json` and the original checkpoint. Subset imports preserve other champion registry entries. |

Training and validation each use two layouts. Fixed validation results remain
the comparable audit shown in the charts; the screening and final promotion
conditions use separate seed bands. A candidate that wins fresh tests can
have a lower fixed audit score. The holdout never selects champions. Paired
acceptance guards are not a formal guarantee of statistical significance.

The campaign uses a new executable, so its inherited tactical champion was
reevaluated with the packaged executable and curriculum. See the package's
`qualification.json` for the report and both executable identities.

## Diagnostic results

The current tactical centre's mean gain over the champion was **0.01367**, with
a standard error of **0.00820** across the 96 paired batches. It won **52** and
lost **44** batches. Stage mean gains were easy **+0.01031**, mixed **−0.05043**,
and objectives **+0.07187**. This is a promising but uncertain aggregate lead
with a mixed-stage tradeoff, so the champion was retained.

Detailed diagnostic reports are under:

- `.quality-work/campaign-recovery/diagnostics/summary.json` and the individual paired reports.
- `.quality-work/campaign-recovery/strategic/summary.json` and the centre/perturbation reports.

The tactical benchmark used the previous campaign's packaged executable to
measure its own checkpoint. The strategic probe compared the centre with three
Gaussian perturbations of standard deviation 0.04 under identical conditions.
Neither diagnostic changed training state or selected a replacement champion.

Verification logs and screenshots are in `.quality-work/campaign-recovery/`.
The recovery changes passed the Rust suite, portable branch tests and dashboard
tests. A separate code review checked checkpoint inheritance and export/import
compatibility. An isolated lay review of the rendered dashboard led to clearer
candidate labels, explicit opponent labels, corrected threshold precision and
a distinction between training scores and fixed validation charts.
