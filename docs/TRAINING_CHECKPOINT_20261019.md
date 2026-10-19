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

## Remaining before a reconnaissance training release

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
