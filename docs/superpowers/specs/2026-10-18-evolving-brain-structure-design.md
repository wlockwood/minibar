# Evolving champion brain structure

Date: 2026-10-18
Status: approved and implemented; see docs/STRUCTURAL_TRAINING.md.

## Intended outcome

Train descendants of existing validated champions while allowing their activation
functions, active inputs, and active connections to change. Show those changes
over time in the campaign dashboard in language understandable without an AI
background. Run an isolated lay-review of the resulting dashboard and correct
verified comprehension problems.

Existing champion weights are the starting point. A structural experiment is not
automatically an improvement: replacing the champion still requires the existing
validation and promotion checks. Keep the original reference brain for regression
comparisons. Preserve existing campaign directories and independent branch state.

## Approach and alternatives

Recommended: evolve small structural edits between periods of ordinary weight
training. Each search island owns a complete brain structure and its optimizer.
An island is an independent search branch within a campaign. One branch continues
weight training without structural edits; other configured branches explore
structural descendants of the current champion.

Alternative: assign a fixed activation/input/connection configuration to every
branch at startup. This is simpler but cannot produce continuing transformations.

Alternative: arbitrary graph evolution, including adding neurons, layers, cycles,
and new sensor definitions. This expands the search substantially and changes
the execution model. It is outside this change.

## Meaning of the three kinds of change

1. Activation: a hidden neuron's response function can change between tanh,
   Softsign, and Leaky ReLU (fixed negative slope 0.01). Store the choice per
   hidden neuron. Outputs retain tanh and their existing meaning and range.
2. Inputs: each module can enable or disable individual named inputs from the
   sensor list already provided to that module by its brain kind. Disabled inputs
   contribute zero. This does not invent sensors or route another module's
   inputs into a module that cannot currently obtain them.
3. Connections: enable or disable individual existing edges between adjacent
   layers. A disabled edge contributes zero regardless of its stored weight.
   Retain its weight so reenabling an edge can recover its prior setting.

Keep layer widths, module names, output names, and available sensor order fixed
for each brain kind. The resulting graph remains a feed-forward network. This
first version changes which existing paths are active; it does not grow neurons
or create skip connections. Biases remain trainable.

An enabled input is available to the network. It is only connected to an output
if enabled edges provide a path. The dashboard must distinguish these facts;
neither proves the input has a meaningful effect on battle decisions.

## Saved brains and execution

Introduce a versioned brain format that records input enable flags, connection
enable flags, and hidden-neuron activations. Accept legacy version-1 brains as
all inputs enabled, all connections enabled, and tanh everywhere. Loading and
evaluating a legacy brain must reproduce its existing calculations exactly.

Validate array shapes, allowed activations, finite weights/biases, and mask
contents. Reject malformed metadata instead of silently replacing it with
defaults. Newly written brains carry all information needed to execute them.

Implement the same rules in normal inference, explanation traces, and browser
brain-watch calculations. A saved brain, worker evaluation, and viewer must
agree about which signals and edges contribute. Update both maintained
brain-watch entry points; do not rewrite historical exported artifacts.

Node identity is module name, layer index, and neuron index; sensor identity is
its stored name. This stays stable because widths and ordering do not mutate.
Compare edges by their endpoint identities, never by truncating arrays with zip.

## Structural search and inheritance

Add opt-in per-island structural-search configuration with mutation cadence,
activation choices, and relative frequencies for activation/input/edge changes.
Default behavior remains existing weight-only training. Configuration validates
positive cadence, finite nonnegative frequencies with a positive total, and at
least one legal mutation in the selected trainable modules.

At each configured structural boundary, start a proposal from the track's
current champion and make one effective structural edit in a trainable module.
Choose deterministically from legal edits using campaign seed, track, island,
and generation. An edit must actually change its target. Honor frozen modules.
Keep at least one enabled sensor-to-output path for each module output.

Copy all weights and biases from the parent. Do not randomize unaffected
parameters. A function change or disabled path can change behavior immediately;
inheritance does not promise identical behavior after an edit.

Start fresh optimizer moments for the structural proposal, then train its weights
for the configured interval. Preserve the champion independently throughout.
At the next structural boundary, explore another descendant of the then-current
champion. Successful changes therefore accumulate through champion lineage.
Experiments may fail without replacing or discarding the champion.

Keep full-shaped parameter vectors to avoid remapping weights when masks change.
Exclude disabled edges and edges from disabled sensors from optimizer updates,
weight decay, and regularization. Preserve their dormant values. The structural
metadata must be part of the island's identity, not an implicit runtime default.

Per-island checkpoint state contains its complete base brain, optimizer state,
structural attempt counter, parent identity, and pending edit. The optimizer
belongs to that exact structure. Ordinary weight updates resume exactly; a
structural change explicitly resets optimizer state. Existing weight-only state
can resume through a validated legacy loading path, without editing its files
in place as part of starting a new structural branch.

## Selection, identity, and portable branches

Run existing training, validation, stage/faction protection, and fresh paired
promotion comparisons for structural candidates. Preserve the initial reference.
Holdout remains a report, never a selection signal. Do not give a structural
candidate an automatic promotion or claim superiority from training fitness.

Include complete structural metadata in brain hashes, job identities, exported
snapshots, archive opponents, and imported candidates. Reject stale work from a
different backend or structure. Keep strict input/output contracts while
allowing legal activation and mask differences in champion/frontier validation.

Warm-start a new campaign branch from an explicitly selected current run or
best-brain registry using the existing branch workflow. Record exact parent
hashes and source provenance. Do not guess the authoritative champion by file
mtime, merge optimizer state, or silently overwrite/resume an active campaign
under the changed executable. Provide a runnable example and command to create
the new structural branch from the user's selected champion source.

## History and dashboard

Save an initial brain snapshot and immutable snapshots at structural proposals
and accepted promotions. Record parent and child brain hashes, generation,
track/island, edited module/target, old/new setting, and eventual outcome.
Reference snapshots by identity in committed history. Commit history and
checkpoint state consistently so interrupted generations do not become completed
events in the dashboard. Include snapshots in portable return packages.

Extend scripts/campaign_dashboard.py and its HTML template, retaining the
self-contained offline page and standard-library-only generator.

The new section answers: "What does this brain look at, how is it connected,
and what changed since its parent?"

- Select track, module, and recorded snapshot. Default to the current champion.
- Draw named inputs, hidden neurons labeled by activation, and named outputs.
  Show disabled sensors distinctly; show disabled edges on demand to limit clutter.
- Show active/available input counts, enabled/possible edge counts, and activation
  counts. Explain activation as "how a neuron turns its calculated number into
  an outgoing signal." Label the counts as structural, not importance scores.
- Compare with the parent or starting champion. Highlight added/removed active
  paths and changed activations using labels and line styles as well as color.
- Provide a chronological list: "Disabled reload input in spacing", "Changed
  spacing neuron 2 from tanh to Softsign", or "Enabled connection from health
  to spacing neuron 3". State whether each experiment is training, evaluated
  without promotion, or accepted as champion. Distinguish generation numbers
  from structural-attempt numbers.
- Keep performance charts adjacent to the history. Explain that an accepted
  brain may include both a structural edit and subsequent weight training; do
  not attribute all improvement causally to that edit.
- Compare weight drift only across matching enabled parameter identities, and
  report structural changes separately. Do not describe a weight's sign or
  magnitude as proven behavioral importance.
- Legacy runs show their initial/current structure and "Structural history was
  not recorded for this run." Never fabricate a historical sequence from two
  endpoints. Synthetic demo data is visibly labeled.

The dashboard remains read-only. Watching a run does not change its training.

## Verification and review

Focused Rust checks cover legacy calculation parity, serialization round trips,
invalid metadata, each activation, disabled sensor/edge behavior, matching normal
and traced inference, mutation legality, frozen modules, and weight inheritance.

Campaign checks cover deterministic edits, structurally distinct work identity,
pause/resume equivalence, optimizer ownership/reset, preservation of rejected
champions, and a short campaign that records a real structural proposal without
requiring that proposal to improve fitness. Verify portable export/import of
structural brains and history with existing branch integrity checks.

Python checks cover legacy dashboard loading, structural snapshots/diffs,
missing history, history/checkpoint consistency, and safe embedded JSON. Render
legacy, synthetic evolution, and real smoke-run dashboards. Inspect the rendered
graph, controls, labels, and change timeline in a browser if a browser tool is
available; report any visual verification limitation explicitly.

Use lay-review with an isolated reviewer who sees only the rendered dashboard
or screenshots and cannot inspect source, docs, prior chat, or this spec. Use a
software-literate player unfamiliar with neural networks. Require a comprehension
check and ranked, specific confusion findings. Verify findings against source,
fix genuine problems, and use a fresh reviewer if substantive wording changes
need a second pass. Report rejected findings and remaining limitations honestly.

## Approval boundary

This contract was approved before implementation. The implementation plan was
subsequently approved for execution in this session. Existing champions and
running campaign state were preserved; a separate short branch was used for
verification.
