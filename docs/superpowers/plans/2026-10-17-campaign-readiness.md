# Campaign readiness improvements

**Goal:** Make long training runs stoppable, informative and less likely to
promote a lucky candidate or learn around a simulation placement defect.

**Authority:** User explicitly requests selecting the next improvements and
implementing them. Continue through concrete fixes and validation without an
additional approval gate. Preserve the shared dirty checkout and existing runs.

**Scope:** Three bounded improvements to the existing campaign, not a new
distributed service or full game-fidelity project.

- [x] Diagnose and fix repeated economic construction-placement rejections from
  the retained objective-stage trials. Preserve perspective-limited observations;
  prove the specific cause with a regression test and rerun the affected cases.
- [x] Stop at evaluation-job/stage boundaries, including initialization and
  holdout, while caching completed results and preserving exact resume. Make
  summary/progress files reflect committed champions and current work.
- [x] Require a candidate that wins the fixed validation suite to also beat the
  incumbent on shared fresh promotion seeds, without using holdout for selection.
  Save the paired evidence. This is a noise guard, not a significance claim.
- [x] Add meaningful interruption/resume, cached-worker and promotion tests; run
  release verification, bounded full-horizon trials, and update portable bundles
  and documentation with current hashes and measured limitations.

## Ownership

Codex owns `campaign.rs`, readiness integration tests, docs, verification and
package generation. A helper owns promotion utilities, config/work protocol
extensions and their tests. Claude owns the placement diagnosis/fix in
`strategic.rs`; changes to `world.rs` require coordination before editing. Another
helper reviews package/work integrity without editing these paths.

## Decisions

- Preserve the champion and optimizer checkpoint on a partial generation. Cache
  complete work units; resume repeats the deterministic generation using them.
- Promotion challenges use the validation scenario family with new seed/layout
  samples, one seed per paired evaluation, and no archive opponents. Candidate
  and incumbent face identical conditions. Holdout remains outside selection.
- Keep old artifacts; a changed executable cannot resume their optimizer states.
  Export a new package rather than overwrite the previous release bundle.
- Count surviving replacement value once per strategic entity; overlapping role
  capabilities remain available as separate sensors/report metrics.
- Recheck standalone worker inputs before publishing or reusing results. Share
  unchanged-incumbent promotion evaluations without sharing across changed brains.

## Verification recorded during implementation

- A regression first failed because an exhausted startup budget still performed
  initial validation. It passes after job/stage cancellation was implemented.
- Ten native CLI tests pass in development mode, including exact partial-job
  resume and accepted/rejected external promotion with incumbent cache reuse.
- Twenty-two campaign library checks and thirty-two focused Python checks pass.
- Independent read-only source review found no remaining issue in the stop,
  promotion or shared-incumbent cache integration. Final release/package and
  full-horizon evidence follows after Claude's strategic changes are complete.

Final evidence is in `examples/campaign/RESULTS.md`: 376 release Rust tests and
32 Python tests pass; independent placement rerun reproduces 91->8 rejections,
72->74 projects and 143->145 units. One full-horizon generation completed after
a timed stop/resume in 146.02s. Both tactical fixed-suite winners lost the fresh
promotion gate; no champion was replaced. New package manifests verified;
extracted smoke trained/resumed 2->3 generations, and a relocated normal-profile
worker result advanced the original coordinator. One Windows host only.
