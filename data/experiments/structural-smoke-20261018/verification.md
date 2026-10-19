# Structural evolution verification — 2026-10-18

Implemented activation changes per hidden neuron, input masks, and connection
masks in version-2 brains. Version-1 brains load with dense connections and tanh.
Final outputs remain tanh. Structural proposals inherit champion parameters and
train in independently checkpointed islands under existing promotion checks.

The source for this smoke test was the generation-29 checkpoint in
`data/brains/campaign`. Its checksum before and after the test was
`9c53638c9cee2f5c0f31a45b9ee7ca93d0fa3a706b277f94cfa05f4183aeb962`.
Both tactical and strategic champion weights and their original references were
copied into a separate package and requalified with the new release executable.

The two smoke generations used deliberately shortened scenarios and a promotion
threshold that preserves the incumbents. Tactical proposals included activation
and input changes; strategic proposals included connection changes. The return
archive imported with identical state, and its repeat was recognized as a
duplicate. `verification.json` records the main assertions. This demonstrates
feature operation, not an improvement in playing strength or a physical
second-computer test.

## Checks

- Full Rust suite: 447 tests passed, followed by a passing additional structural
  external-worker equivalence test covering partial proposals and relocation.
- Release build and `cargo fmt --check` passed.
- Python: 32 dashboard/viewer tests, 21 branch tests against the release binary,
  and 8 packaging tests passed.
- Both maintained brain-watch pages passed executable Node fixtures for masks,
  Softsign, Leaky ReLU, and legacy defaults.
- Chrome rendered synthetic, real, and legacy dashboards. Snapshot selection,
  parent/initial comparison, themes, and mobile rendering were checked. Mobile
  width was 430 CSS pixels with no page-wide overflow; the diagram itself pans
  horizontally to keep labels readable.
- Scoped Git diff and explicit whitespace checks, including untracked files,
  passed. The original shared checkout changes were not staged or committed.
- Strict repository-wide Clippy remains failing on existing lint debt, including
  engine argument counts, sensor-test initialization style, and world casts.
  Newly introduced campaign lint findings were corrected. This is not a clean
  Clippy result.

## Lay-review

Three fresh, isolated reviewers inspected only rendered screenshots and visible
dashboard text, without source, prior conversation, or other reviewers' reports.

Pass 1 found that an experiment selection could be mistaken for the headline
champion scores. It also found unclear disabled-edge visibility, signed movement
outputs, completion/stall wording, and weight-drift percentages. Clarified the
selector scope, comparison exceptions, output meaning, and score explanations.

Pass 2 understood the read-only controls but wanted clearer track, parent, and
search-branch meanings. It also challenged the synthetic lineage. Added track
definitions, parent semantics, center/sample definitions, and moved long network
explanations into an expandable section.

Pass 3 identified a remaining real inconsistency: manually generated structural
champion records did not match synthetic performance promotions. Replaced that
independent sequence with snapshots generated from the same promotion history;
a regression test now asserts exact correspondence. Activation explanations have
a descriptive expandable label. The transformation table opens by default;
selecting an experiment selects its edited module when available.

Suggestions to infer a learned tactic from a removed sensor were not treated as
proof of behavior. That requires battle observations or controlled comparisons;
the page explicitly limits its claims to structure. Horizontal diagram scrolling
on narrow screens remains a documented readability tradeoff.

## Independent code review

The reviewer found dormant-weight updates in standalone strategic training and
insufficient structure binding for weight-only checkpoints. Regression tests
reproduced both before fixing them. Also tightened inactive optimizer values and
typed promotion history validation. The reviewer checked the remediations and
found no remaining important defect in those fixes. Native export now preserves
the original regression reference, rather than replacing it with the warm-start
champion. Its regression test also failed before the fix and passed afterward.
