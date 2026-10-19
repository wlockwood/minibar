# Training checkpoint: elite retests, range curriculum and reconnaissance

Git tag: `minibar-training-elite-range-recon-wip-20261019`.

This checkpoint preserves the current source, training documentation and small
durable campaign results. Reconnaissance support is still in progress.

## Completed before reconnaissance work

- Bounded elite pools with scheduled fresh retests and normal promotion gates.
- Unit-derived starting distances, aircraft runway, approach-time allowance and
  sensor-limited movement when no opponent is visible.
- Scripted surround opponent, reciprocal heavy/swarm exercises, frozen learned
  opponents and firing-participation diagnostics.
- Dashboard elite/range reporting and expansion-state preservation.
- Prepared `data/branches/elite-range-training-20261019` from saved ground/air
  champions and five air elites. Its binary and sealed inputs predate the
  reconnaissance changes below. Production was not started by this work.

## Reconnaissance implementation at this checkpoint

- Optional, deterministic sensor-support mix: neither side, one side, both sides
  in equal thirds. The latter two strata supply radar on about two thirds of
  generated fights. Both exported scout aircraft carry radar.
- Source-backed radar towers, scout aircraft and static jammer pools.
- Real, destructible support units with costs included in battle scoring and
  separately recorded alongside combat/objective investment.
- Fixed sensor-limited scouting missions for support aircraft; combat aircraft
  retain their separate learned controller.
- Auxiliary support survival does not prevent main-force elimination.
- Focused native checks cover seeded proportions, side swaps, support costs,
  elimination, radar-enabled fire, jamming suppression and visual scout recovery.

## Pending at the original WIP tag

- Check pending-projectile cleanup after main-force elimination: surviving units
  should not keep emitting new shots at support while existing impacts drain.
- Review travel allowance so fast auxiliary scouts cannot shorten the approach
  time required by slow main forces.
- Add dashboard support-distribution, provider and resource-imbalance displays.
- Complete configuration/coverage edge-case checks and final browser validation.
- Build and qualify a fresh successor package, preserving the existing packages'
  executable/input identities and champion snapshots.

Generated binaries, recordings, optimizer states and scratch verification logs
follow the repository's existing ignore rules. Unrelated cloud-video files are
excluded from this checkpoint.

## Verification at checkpoint

- `cargo test --all-targets --target-dir target/surround-verification`: 500 passed.
- Python script tests: 163 passed; unrelated in-progress `test_cloud_video.py`
  excluded.
- Rust formatting and staged whitespace checks passed.

These checks cover the current source; reconnaissance release checks listed above
remain pending.

## Verified follow-up

Tag: `minibar-training-radar-scouts-20261019`.

The follow-up completes the pending projectile-draining, main-force travel-time,
configuration, dashboard and package work above. Independent source review found
no concrete Important/Critical defects. Verification passed 503 Rust tests and
164 Python tests (the unrelated cloud-video test module remains excluded).

`data/branches/recon-training-20261019` preserves the initial champions and five
air elites from the preceding elite-range package. Its 55 sealed inputs were
hash-checked. The Windows launcher initialized the pilot at generation zero;
this was an initialization check, not a completed training pilot. Production
training was not started and holdout was not evaluated.

Read-only fresh validation covered every ground and air stage using one layout,
one seed and seed base `8000000060000`. Every stage recorded equal counts of
neither-side, one-side and both-side assigned support. These counts describe
assigned assets; actual radar contact varies during battle. All support costs
were checked against total starting investment. Browser verification displayed
support/range/time metadata and five imported elites without JavaScript errors,
and preserved expanded tables on resize.

Run the new package in a visible terminal with:

```powershell
.\data\branches\recon-training-20261019\Train.ps1 -Profile production -Threads 32
```

See [DIVERSE_TRAINING.md](DIVERSE_TRAINING.md) for the matching dashboard command.
