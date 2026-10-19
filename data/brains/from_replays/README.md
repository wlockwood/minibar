# Reference inputs for training tests and examples

This directory retains reference assets on the `tiered-brains` branch:

- `catalog.json` and `catalog.sensors.json`: unit definitions.
- `maps/`: compiled maps and their source/provenance files.
- `manifest.json`: source asset provenance.
- `tactical/baseline.json` and `strategic/baseline.json`: authored initial
  weights, as described in the original package; these are not trained champions.
- Scenario and training configuration JSON used by the examples.

Previous learned brains, runs, trial outputs, scores, logs and diagnostics have
been removed from this branch. They remain in Git at the
`minibar-training-radar-scouts-20261019` checkpoint and in the ignored local archive
described in [data/README.md](../../README.md).

For a runnable workflow, see the [repository README](../../../README.md).
