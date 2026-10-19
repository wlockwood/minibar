# Portable training branches

User request: create a portable training package from current best, bring the
other computer's results back, and integrate them. User clarified that the other
computer should explore a separate branch. Proceed through implementation and
verification within this authorized scope; preserve existing runs and bundles.

The existing executable remains unchanged, preserving checkpoint compatibility.
Python on the exporting/importing computer builds on package_training.py; the
Windows destination needs only the shipped binary and PowerShell launchers.

- Export validated best brains from a stopped run or the local best registry.
  Start fresh optimizers from those champions using distinct root/island seeds.
  Include all normal curriculum assets, lineage, hashes and one-command launchers.
- Return a complete portable branch with its authoritative checkpoint and partial
  work cache. Lock against active training while snapshotting. Validate inputs
  before launching and returning. Never ship unrelated local files or STOP locks.
- Import against the original export's immutable manifest; reject bad hashes,
  mismatched identities, traversal, duplicate ZIP paths and unsafe file types.
  Validate a staged checkpoint with the unchanged native executable before commit.
  Keep branch checkpoints separate; duplicate returns are idempotent, divergent
  returns are preserved, and independent optimizer states are never combined.
- Optionally integrate champions into a best-brain registry. Re-evaluate incoming,
  incumbent and initial reference brains on identical validation plans using the
  existing native evaluation CLI. Require fixed-suite gain and stage protection,
  then four fresh paired checks with mean gain, strict majority and stage guards.
  Never select using holdout. Registry objects and assessment evidence persist;
  existing training run/optimizer state stays intact. Future exports can use it.

Files: scripts/training_branch.py (CLI/export/import), branch_transfer.py
(manifest/archive/lock primitives), branch_selection.py (native comparisons),
training_branch/Train.ps1 and Return-Results.ps1 (portable runtime), focused
Python tests, and docs/TRAINING_BRANCHES.md. Existing packager stays compatible.

Verification: first reproduce missing export/import behavior with failing tests;
test distinct seeds and warm-start champion weights, round-trip/import/duplicate
safety, archive corruption/traversal, active-run locking, promotion rejection and
acceptance, and native local-versus-relocated training. Exercise the actual
PowerShell launch/return workflow and native checkpoint validation. Build a real
current-best normal-profile ZIP, retaining all previous artifacts. Final source
review and documentation/hash checks before completion.

Progress:
- [x] Export and portable launchers
- [x] Return/import with complete resumable state and duplicate protection
- [x] Champion integration with fresh local comparisons
- [x] Tests, real round-trip proof, current-best ZIP and operator guide

Completion evidence: 16 branch tests and 32 existing focused Python tests pass;
all 376 release Rust tests pass. The ready current-best ZIP contains 31 verified
immutable files. Its local Windows PowerShell extract/start/return/import probe
and repeat import are recorded in data/branches/transfer-proof-20261017/receipt.json.
That normal-profile probe stopped before training; focused fixtures separately
trained and resumed complete generations. No second physical computer was tested.
The native executable is unchanged. Final review found no remaining material
issues after snapshot deduplication and registry-read consistency fixes.
