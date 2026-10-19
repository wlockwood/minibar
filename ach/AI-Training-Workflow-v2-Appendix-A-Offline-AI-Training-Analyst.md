# Appendix A — Offline AI Training Analyst and Experiment-Driven Review

**Companion document:** [AI-Training-Workflow-v2.md](AI-Training-Workflow-v2.md)  
**Status:** Actionable implementation recommendation; proposed design, not a claim that the interfaces below already exist in MiniBAR.  
**Scope:** External agent-assisted diagnosis, scenario generation, counterfactual validation, supervised teaching, and training-quality auditing.  
**Architectural placement:** An **offline development service**, not a seventh in-game brain and not part of the simulation, policy-inference, or PPO update loop.  
**Adoption:** This appendix is standalone. It does not supersede or require edits to the companion document.

## A.1. Recommendation and rationale

**Build a separate, tool-using AI Training Analyst that reviews a selected subset of training episodes, diagnoses concrete weaknesses, requests bounded experiments, and produces versioned, evidence-backed development artifacts.** Its highest-value output is **not a score for a battle**. It is an actionable answer to: *What failed, at which boundary, how could we test that explanation, and what should change if the explanation holds?*

The initial implementation should consist of ordinary MiniBAR telemetry and command-line execution, plus a small **external Python review coordinator**. MiniBAR must **not** call an AI API, know which model reviews results, construct prompts, or accept AI-authored game commands during normal training. Agent/model access belongs exclusively to the external coordinator.

This extends, rather than replaces, the companion workflow:

| Companion section | Appendix contribution |
|---|---|
| §2, instrumentation and manifests | Decision-level evidence contract and trace provenance |
| §3, campaigns | Post-campaign selection, review, validation, and follow-up cycle |
| §4, formation-first squad learning | Diagnosing `CHARGE` / `HOLD` / `ORBIT` / `ENCIRCLE` / `KITE` selection separately from their frozen execution |
| §§6–7, hierarchy and rewards | Responsibility attribution, plan audits, and reward/proxy failure detection |
| §9, command/report loop | Auditing objection quality, mission revisions, requests, and overrides |
| §10, human review and corrections | Optional pretrained agent reviewer that proposes hypotheses and labels, with validation and human oversight |
| §§11–12, integration and promotion | Concrete delivery milestones, regression artifacts, and acceptance criteria |

**Recommended start:** implement the evidence/export boundary and run a reviewer manually after the first formation-selection benchmark. Do not build a custom preference/reward model, autonomous retraining service, database, or distributed agent platform initially.

## A.2. Ownership and system boundary

```text
 EXISTING TRAINING SYSTEM                           EXTERNAL REVIEW SYSTEM

 MiniBAR simulator ──neutral trace/results files──► Review coordinator
       ▲                                             ├─ selects episodes/decisions
       │                                             ├─ prepares evidence packets
       │                                             ├─ invokes AI reviewer
       │                                             └─ checks structured findings
       │                                                        │
       │                                                        ▼
       └── normal CLI / scenario-runner commands ─── Validation worker
                                                                │
                                        measured comparisons / confirmed defects
                                                                │
                                                                ▼
                                              Versioned findings and artifacts
                                              ├─ regression scenarios
                                              ├─ trusted correction candidates
                                              ├─ missing-action/feature requests
                                              ├─ simulator/engine bug reports
                                              └─ proposed campaign changes
                                                                │
                                                                ▼
                                            Engineer or explicit promotion gate
                                            (then ordinary training/evaluation)
```

| Component | Owns | Must not own |
|---|---|---|
| **MiniBAR / simulator bridge** | Deterministic scenarios, legal observations, action execution, outcomes, generic instrumentation, reproducible reruns | AI-provider integration, prompt construction, reviewer judgments, data-label promotion |
| **Existing trainer** | PPO/evolution, checkpoints, reward bookkeeping, scenario evaluation, campaign manifests | Deciding that a reviewer opinion is ground truth |
| **Review coordinator** | Sampling, packet assembly, budgets, model invocation, schema checking, finding persistence | In-game tactical decisions or direct edits to released policies |
| **AI reviewer** | Explanation, hypotheses, proposed alternatives, requests for tests | Claiming unrun tests passed, unilateral reward changes, authoritative mechanical facts without evidence |
| **Validation worker** | Allow-listed reruns, paired comparisons, fidelity tests, artifact lineage, measured statistics | Arbitrary execution of agent-authored code with unrestricted permissions |
| **Engineer / promotion process** | Accept/reject fixes and labels; evaluate real mission outcomes; release or roll back compatible bundles | Equating a persuasive explanation with a demonstrated policy improvement |

Start with **one general-purpose reviewing agent**. The later roles described in §A.6 are tasks for that agent, not separate mandatory processes or neural networks.

## A.3. Minimum MiniBAR-facing interface

### A.3.1. Prefer files and an existing CLI over a network service

The external coordinator needs to **read episode artifacts** and **launch ordinary simulator experiments**. It does not need a simulator-resident API server, callbacks, message bus, or synchronous review interface. Reuse existing trace/viewer/replay/scenario features where the actual MiniBAR code supports them; audit the current repository before relying on particular CLI flags. The commands and filenames in this appendix are **proposed interface conventions**, not verified existing MiniBAR commands.

Preserve normal high-throughput training. Use configurable trace levels:

- **`summary` (default):** episode ID, seed, version manifest, scenario, mission outcomes, aggregate costs, errors, and reason for termination.
- **`decisions` (selected episodes or sampled campaigns):** legal controller observations, available choices, selected/accepted/executed actions, lifecycle transitions, mission reports, and concise local context.
- **`diagnostic` (on demand):** detailed simulation events and mechanics needed to reproduce a suspect result.

Do **not** store full world state every frame for every training run. Select episodes first where possible, then reproducibly rerun them with higher instrumentation. Measure disk and runtime overhead at every trace level.

### A.3.2. Evidence artifacts

A minimally useful review packet should reference or contain:

| Artifact | Required information | Why |
|---|---|---|
| `manifest.json` | Scenario/map/unit/simulator versions; seeds; parent/child/opponent bundles; policy/feature/candidate/reward/scheduler versions | A conclusion must apply to a specific experiment condition |
| `episode.json` | End conditions, explicit mission success/failure, timing, resources and losses, important phase/event markers | Enables triage without parsing full traces |
| `decisions.jsonl` | Decision-time actor-legal inputs, mission identity/authority, legal candidates/masks, sampled/requested action, actual acceptance and execution | Supports fair critique and action-level supervised examples |
| `events.jsonl` (optional initially) | Target loss, path failure, switch deferral, emergency interruption, unit losses, report/acknowledgment, objective transitions | Distinguishes the selector's choice from downstream execution |
| `review_packet.json` | Selected window, links/IDs for evidence, the review question, separate outcome data, available test capabilities | Gives the agent a compact, bounded task |
| `finding.json` | Diagnosis, evidence references, uncertainty, responsible boundary, proposed test, status and eventual validation | Makes review actionable and machine-checkable |

A campaign may use other layouts, but maintain these **logical fields** and immutable links to source artifacts. Store actual raw observations/features where affordable; a later recomputation with changed normalization or candidate generation is not equivalent to the decision the controller actually saw.

### A.3.3. The decision record must distinguish intent, selection, and execution

Example **proposed** record (illustrative fields and values):

```json
{
  "schema_version": 1,
  "episode_id": "episode_1842",
  "decision_id": "squad_2_d081",
  "frame": 1840,
  "controller_id": "squad_2",
  "controller_layer": "squad",
  "bundle_id": "bundle_sha256_or_version",
  "mission": {
    "id": "hold_pass_04",
    "version": 2,
    "type": "HOLD",
    "deadline_frame": 4500,
    "authorized_loss_metal": 600,
    "target_region_id": "north_pass"
  },
  "actor_observation": {
    "feature_schema": "squad_observation_v3",
    "values_ref": "observations/episode_1842/squad_2_d081.json",
    "visibility_provenance": "actor_legal"
  },
  "candidates": [
    {"id": "charge_v1", "mode": "CHARGE", "legal": true},
    {"id": "hold_v1", "mode": "HOLD", "legal": true},
    {"id": "orbit_v1", "mode": "ORBIT", "legal": true},
    {"id": "encircle_v1", "mode": "ENCIRCLE", "legal": true},
    {"id": "kite_v1", "mode": "KITE", "legal": true}
  ],
  "mode_before": "HOLD",
  "mode_age_frames": 480,
  "selected_candidate_id": "kite_v1",
  "accepted_candidate_id": "kite_v1",
  "effective_mode_after": "KITE",
  "intervention": null,
  "issued_order_count": 3
}
```

Required correctness rules:

1. The actor input must be **exactly the information available at decision time**, including masks and candidate descriptors. Persist raw input values or an immutable reference, not only a loose summary.
2. A request to switch, an accepted switch, a deferred/canceled switch, and the effective maneuver are separate facts. Record mode age, assignment/version changes, scheduler dwell and emergency overrides where they influence behavior.
3. Use stable semantic candidate IDs **and** preserve the decision-time ordering/slot mapping consumed by the policy; names alone do not reconstruct PPO actions.
4. The mission contract must convey intent, authority, deadline, and important prohibitions. An apparently costly `HOLD` or `DELAY` can be correct when sacrifice was authorized.
5. Post-outcome truth, undiscovered enemy positions, and hidden random state belong to a **separate evaluation/debug channel**. They may support outcome scoring and fidelity diagnosis, but cannot silently become action-time teacher inputs.
6. Capture parent and child versions so the reviewer can distinguish a poor order from poor fulfillment of a sound order. Do not infer ownership solely from the visible unit behavior.

For initial deployment, allow a compact subset of features if it faithfully reconstructs the actual selected action and its justification; expand when a real diagnosis requires it.

## A.4. External coordinator and tool surface

Use a small Python package or directory **outside MiniBAR's simulation core**, for example:

```text
review_pipeline/
  config/                  # reviewer settings and budgets
  schemas/                 # packet, finding, experiment, label schemas
  select.py                # reproducible review sampling and deduplication
  packet.py                # observation/outcome separation and context assembly
  review.py                # external model/provider adapter
  validate.py              # allow-listed simulator and comparison requests
  route.py                 # findings -> tests, labels, issue reports, scenarios
  report.py                # campaign summary and measured reviewer utility
  tests/                   # schema, leakage, replay, and validator contract tests
reviews/
  <campaign_id>/
    queue.jsonl
    packets/
    findings/
    validations/
    accepted_artifacts/
```

An implementing agent can rename files to match the existing project organization; the key requirement is **separation of responsibilities**. Local files are sufficient for the first version; add a database only when scale or concurrent access makes it worthwhile.

### A.4.1. Minimum external operations

| Operation | Reads/does | Permission |
|---|---|---|
| `list_episodes(campaign)` | Summaries and metadata | Read-only |
| `get_review_packet(episode, decision?, window?)` | Curated actor-view evidence and separated outcomes | Read-only |
| `inspect_decision(decision_id)` | Full legal observation, choices, selection, execution and ancestry | Read-only |
| `request_comparison(scenario, alternatives, seeds, budget)` | Create a declarative experiment request | Proposal only |
| `run_comparison(request_id)` | Launch permitted simulator commands, capture results | Validator only |
| `save_finding(finding)` | Validate and store an AI-authored diagnosis | Write review artifact only |
| `export_accepted_artifacts(campaign)` | Prepare explicitly approved regressions/corrections for normal trainers | Approval-gated |

Keep **reviewer-facing tools declarative**. An agent requests `compare HOLD against KITE on this scenario with paired seeds`; the validator decides whether that comparison is supported, legal, within budget, and reproducible. Do not expose unrestricted shell execution as the production experiment API. A developer-operated coding agent may use a shell while building the system, but routine automatic review should use the restricted operations above.

Illustrative **external** CLI (not MiniBAR CLI):

```bash
python -m review_pipeline.select --campaign campaigns/c42 --budget 12
python -m review_pipeline.packet --queue reviews/c42/queue.jsonl
python -m review_pipeline.review --campaign c42
python -m review_pipeline.validate --campaign c42 --max-sim-runs 200
python -m review_pipeline.report --campaign c42
```

The provider adapter is replaceable. Its interface should accept a bounded packet and return a schema-validated response. Make input/output retention and retry behavior explicit, and keep provider credentials out of simulation artifacts.

## A.5. Review procedure and evidence standard

### A.5.1. Sample useful cases, not every battle

After a completed checkpoint/campaign, choose a bounded review queue combining:

- **Uniform random episodes:** protects against the bias of reviewing only dramatic losses.
- **Clear mission failures:** abandonment, missed deadlines, inappropriate losses, or failure to report a known blocker.
- **Surprising successes:** identify effective behaviors worth preserving, teaching, or converting into new tactics.
- **Anomalies:** oscillating formation changes, path failures, long inactivity, contradictory orders, strange reward/outcome disagreement.
- **Integration failures:** parent/child disagreement, unproductive capability requests, stale feasibility reports, failed handoffs.

A practical **pilot budget** is 8–12 episodes per campaign, with 1–3 focal decisions each; this is a tunable project budget, **not** a claim about statistical sufficiency. Save selection reasons and probabilities where known. Deduplicate near-identical failures so one issue does not consume the whole budget. Keep episode/scenario ancestry available to prevent train/test contamination.

### A.5.2. Two-pass review to prevent hindsight cheating

1. **Decision-time critique:** the agent sees only the actor's legal observation, mission, available candidates, selected/accepted/executing behavior, preceding legal history, and relevant authority. It identifies whether the decision is defensible *without seeing the later outcome*.
2. **Outcome diagnosis:** show subsequent events and labeled evaluation truth. The agent checks whether its hypothesis matches the actual execution and result, including downstream causes and uncertain enemy behavior.
3. **Propose an experiment:** prefer the smallest comparison distinguishing the plausible explanations. If no supported test exists, record an unresolved hypothesis rather than a correction label.

The reviewer must not conclude that a choice was foolish **only because it lost**, claim it should have known a hidden enemy was present, or assign blame to the squad when the platoon ordered an incompatible task.

### A.5.3. Classify findings by the component that could fix them

| Finding class | Example | First destination |
|---|---|---|
| `bad_action_selection` | Selected `KITE` when a mission-compatible `HOLD` was available and demonstrably preferable | Squad/platoon selector experiment; possible supervised correction |
| `execution_or_scheduler_defect` | Good formation chosen, but repeated movement-order cancellation prevents firing | Deterministic executor/scheduler issue and regression |
| `missing_action_candidate` | No legal coordinated screening-withdrawal plan exists | Candidate generator, tactics library, or plan vocabulary |
| `observation_or_belief_gap` | Controller lacks known range/uncertainty information needed for a defensible decision | Feature/belief contract investigation |
| `parent_intent_or_authority_error` | Parent orders unsupported push or changes a sacrificial hold without revising authority | Parent policy and mission protocol |
| `report_or_forecast_error` | Platoon knows a hard blocker but fails to report, or ETA is systematically optimistic | Feedback lifecycle or separately calibrated predictor |
| `reward_or_metric_exploit` | Excellent scalar score despite abandoned objective | Reward/metric definition and evaluation tests |
| `simulator_fidelity_issue` | MiniBAR ranks an approach favorably because weapon/terrain behavior differs from BAR | Simulator/bridge mechanics investigation |
| `uncertain` | No legal candidate comparison, insufficient context, or stochastic result | Request more evidence; no training label |

Review the **entire command chain** where available: Strategy intent → Operations mission → Platoon plan → Squad mode → unit execution → report. Diagnoses should identify the **earliest supported actionable defect**, not merely the lowest visible failure.

### A.5.4. Required finding format

Example **hypothesis**, deliberately not marked verified:

```json
{
  "schema_version": 1,
  "finding_id": "c42_f17",
  "episode_id": "episode_1842",
  "decision_ids": ["squad_2_d081"],
  "finding_class": "bad_action_selection",
  "responsible_boundary": "squad_selector",
  "claim": "The selected KITE mode may abandon the assigned HOLD region too early.",
  "evidence_refs": ["episode_1842/squad_2_d081", "episode_1842/mission_result"],
  "uncertainty": "medium",
  "alternative_candidate_id": "hold_v1",
  "proposed_validation": {
    "kind": "paired_initial_state_comparison",
    "scenario_ref": "scenario_1842",
    "choices": ["kite_v1", "hold_v1"],
    "seeds": 20,
    "primary_outcome": "seconds_holding_region"
  },
  "status": "hypothesis"
}
```

The actual schema should reject missing source IDs, non-existent candidate IDs, ambiguous scenario versions, unsupported experiment types, unbounded seed requests, or claims that a test succeeded without a stored validation result. Store the full explanation separately if desired; the structured result is what the pipeline processes.

## A.6. High-value reviewer jobs

Start with the first two, then add others as matching simulator and hierarchy capabilities become available.

| Job | Reviewer question | Concrete output | Preconditions |
|---|---|---|---|
| **Tactical after-action analyst** | Was the chosen formation/plan appropriate for this mission and visible state? What component caused the miss? | Localized finding and discriminating test | Decision/mission/actual-action trace |
| **Adversarial scenario designer** | What nearby scenario variant reveals whether this competence is robust? | Seeded parameterized scenario family, expected distinction, legal constraints | Scenario generator and development suite |
| **Simulator fidelity auditor** | Do MiniBAR and actual BAR disagree on mechanics or tactical rankings that training relies on? | Minimal reproducible matchup, mismatch measurements, ranked fix | Matched controlled encounters in both systems |
| **Tactical teacher** | Can a good existing candidate be chosen more consistently than the current policy? | Candidate-level correction **proposal**, later evidence-backed label | Actual legal decision candidates and reproducible comparisons |
| **Hierarchy/reward auditor** | Was the cause a parent assignment, child failure, missing report, or exploitable objective? | Interface/reward issue and paired mission/report regression | Mission and report ancestry |
| **Operations/Strategy planning teacher** | Would a different fully specified, authorized objective/resource plan work better? | Candidate operation or plan proposal with costs and assumptions | Candidate generation, legal world summary, credible cost/ETA model |
| **Tactic-discovery analyst** | Does a recurring successful pattern justify a new deterministic maneuver? | Narrow maneuver specification and separate benchmark proposal | Stable baseline and evidence that current vocabulary is insufficient |

**Specific squad-first recommendation:** compare the five frozen combat modes and the mission-aware heuristic first. Ask the reviewer to distinguish **wrong mode choice**, **correct mode but poor executor**, **missing support/authority**, and **no applicable mode**. Do not let it invent learned waypoint actions or replace the five-mode boundary during this initial benchmark.

For Operations and Strategy, an agent may propose complex plans, but proposals should be translated into the actual typed candidate/mission vocabulary, authorized budgets, legal observations, and constraints. A natural-language plan alone is not a valid policy label. Proposed operations must be simulated or explicitly remain unverified.

## A.7. Validation: from plausible story to measured evidence

### A.7.1. Support three grades of comparison

**Grade 1 — Same-initial-state paired scenarios (MVP).** Rerun the same scenario family and seeds with one declared controller/action strategy changed. Freeze the rest of the controller bundle, scenario generator and reward definitions. This is a useful **policy or tactic comparison**, not necessarily a counterfactual replay of one historical decision.

**Grade 2 — Decision-level counterfactual branch (later).** Restore or deterministically replay to the exact actor-visible decision state, intervene on one available legal choice, then continue under a declared, fixed parent/child/opponent policy bundle. Record all other differences and limitations. Implement only if MiniBAR supports reliable restoration or intervention replay; ordinary command-log playback alone does **not** prove arbitrary mid-episode branching exists.

**Grade 3 — Actual BAR/Recoil validation.** Reproduce the critical encounter in the real game/engine, using a matched observation/action/goal definition as closely as possible. Compare both measured outcomes and **tactic rankings**. Engine runs are essential for claims that depend on projectile physics, elevation, firing obstruction, movement/pathing, ability mechanics, or command behavior.

Do not describe a Grade 1 test as if it were Grade 2, or extrapolate a MiniBAR-only result into verified BAR behavior.

### A.7.2. Comparison contract

Every validation request should specify:

- Immutable scenario and bundle IDs, action/candidate IDs, and observation/legality contract.
- Exactly what changes between A and B; keep executors, parent intent, and scheduler constant when testing a selector.
- Paired seeds/scenario ancestry, opponent versions, run budget, and stopping conditions.
- Primary mission-specific outcome (e.g., hold duration, mission completion, protected-force escape), with losses and command/runtime cost as supporting metrics.
- Whether the action was actually accepted and executed; a deferred or masked action is not a valid tested alternative.
- A report with actual run IDs, aggregate differences, variability/uncertainty, and important regressions by scenario family.

For noisy outcomes, run multiple paired encounters, estimate effect size and uncertainty, and inspect exceptions. A single favorable branch or convincing narrative is insufficient for a trusted label. A reviewer should be able to report **no supported winner**.

### A.7.3. Explicit evidence statuses

| Status | Meaning | Permitted use |
|---|---|---|
| `hypothesis` | Reasoned claim, no completed discriminating test | Queue for testing or engineering review only |
| `reproduced` | The observed failure is repeatable under documented conditions | Regression candidate; not yet proof that proposed fix works |
| `supported` | Controlled comparisons favor the proposed explanation or action within stated scope | Candidate correction or implementation proposal, subject to approval |
| `rejected` | Evidence contradicts the proposed fix or attribution | Preserve to improve reviewer and avoid repeat investigation |
| `inconclusive` | Weak effect, conflicting results, invalid comparison or unsupported branching | No policy label; request targeted follow-up |
| `accepted` | Human/release process approves a specified artifact for development/training | Enter ordinary pipeline with provenance |

`accepted` does **not** mean deployed or promoted. Policy promotion continues to use the companion's independent held-out and actual-engine gates. A validated local tactic may still regress full missions.

## A.8. Route outputs back to the right workflow

| Validated or reviewed artifact | Destination | Guardrail |
|---|---|---|
| Wrong represented action with convincing matched evidence | Versioned supervised action-example dataset (§§4.4, 10.2) | Label is an actual available legal action at that observation; include confidence, evidence, and branch assumptions |
| New difficult-but-legal scenario | Development curriculum / regression suite (§§3, 4.6, 11) | Keep related variants in the same split; never silently contaminate sealed tests |
| Deterministic execution failure | Executor/scheduler issue plus minimal reproduction (§§2.2, 4.1) | Fix code and rebaseline; do not train a network to compensate for a broken executor |
| Missing plan/mode, missing sensor, or faulty mission semantics | Architecture/interface proposal | Version contract and candidate generator; rerun baselines before crediting a learned model |
| Forecast/report fault | Supervised predictor or protocol test (§9) | Evaluate calibration and actual parent decision gain; objections themselves earn no automatic reward |
| Simulator/engine mismatch | Fidelity issue and cross-engine test (§2.2) | Quarantine affected training claims until ranking agreement is shown or limitation documented |
| Suspected reward hack | Reward/evaluation revision proposal (§7) | Freeze current evidence; new reward version and independent outcome regressions |
| New operational plan or formation maneuver | Narrow candidate/behavior experiment (§§4, 6) | Do not grow the deployed action space solely on agent suggestion |

**Keep four stores distinct:** on-policy PPO rollouts, supervised corrections/demonstrations, forecast/outcome data, and reviewer hypotheses/validation results. A finding is not an on-policy rollout, a critic target, or automatically a trusted teacher label. After changing an actor, executor, prior, masks, reward, or child policy, collect fresh PPO experience as prescribed in §8.3.

Avoid direct AI-authored reward updates in early versions. Reward changes must be explicit proposals, versioned, ablated, and checked against mission outcomes. Do not optimize a learned reviewer score until a separate evaluator has passed the companion's §10.3 gate.

## A.9. Automated curriculum and tactic discovery

Once the basic reviewer is useful, let it produce **declarative scenario variants**, not arbitrary simulation code. A scenario proposal should include a parent scenario, legal parameter edits, targeted failure hypothesis, target mission/role, and the behavior it is intended to distinguish.

Examples:

- The same range-advantaged `KITE` matchup with fast flankers arriving at varying times.
- A `HOLD` assignment with the same enemy and force but different loss authorization or deadline.
- A support squad whose protected artillery temporarily loses line of sight or retreat space.
- A platoon receiving a truthful blocker report at different times before a coordinated attack.
- An Operations request whose delivered anti-air capability arrives before versus after it can matter.

The scenario generator validates legal maps/units/mission parameters, prevents duplicates, tags scenario ancestry, and keeps all derivatives out of the sealed evaluation split used to measure generalization. Continue sampling ordinary cases to avoid overfitting to adversarial puzzles. Measure whether the curriculum improves held-out outcomes, not merely whether the policy learns the generated scenarios.

A reviewer may also nominate an **entirely new deterministic tactic** after recurring evidence shows the existing candidate library cannot express an effective response. Specify its preconditions, state/lifecycle, parameters, interaction with parent constraints, executable pseudocode, and measurable benchmark. Implement and test it as a separately versioned baseline; do not silently grant the learned selector a richer action space during PPO-versus-evolution comparison.

## A.10. One post-campaign review cycle

1. **Freeze the campaign record.** Collect immutable version manifests, summaries, policies, scenario definitions, and sampled traces. The trainer finishes its normal checkpoint/evaluation sequence without waiting for an AI call.
2. **Select a bounded queue.** Combine random cases, failures, successes, anomalies, and integration issues. Save the sampling policy and source IDs.
3. **Prepare actor-first packets.** Start with what the controller knew, its mission and available actions. Append outcome/debug evidence only in the separate diagnostic pass.
4. **Review and parse findings.** Invoke the external agent; validate its JSON and source references. Reject claims about actions that were absent/illegal, invented mechanics, or unsupported experiment types.
5. **Test high-value hypotheses.** Run budgeted paired encounters first; use exact branches only when supported, and actual BAR tests when mechanic fidelity matters. Record negative and inconclusive tests as well as successes.
6. **Route and approve.** Turn supported findings into candidate corrections, regression scenarios, or engineering issues. An engineer checks ambiguous/strategically significant cases; approval is explicit.
7. **Use the ordinary training pipeline.** Apply approved inputs in a new, versioned campaign. Run the companion's matched baselines, development tests, and held-out promotion gates. The analyst never promotes a checkpoint on its own.
8. **Measure reviewer utility.** Did this cycle find real defects, save diagnosis time, produce reusable tests, or improve independent task outcomes? Retain the simpler process if it does not justify its cost.

Store the actual **before/after experiment**, review cost, confirmed causes, accepted artifacts, and eventual model outcome. That allows assessment of the agent as a development tool, rather than assuming a large quantity of commentary equals value.

## A.11. Rollout plan and acceptance tests

These are dependency stages, not estimated durations. Implement alongside the companion's **M0–M2 instrumentation/baselines**, then broaden at M3–M8 as the hierarchy matures.

| Stage | Implementation deliverables | Exit tests |
|---|---|---|
| **R0 — Inventory** | List available MiniBAR trace, scenario, replay, paired evaluation, campaign, and viewer/inspection capabilities; identify gaps and versions | A human can reproduce at least one formation encounter and link outcome to the exact controller bundle |
| **R1 — Evidence contract** | Manifest/episode/decision schemas; actor/evaluator separation; optional decision tracing; IDs and provenance tests | Reconstruct selected legal action, mission authority, accepted/executed mode, and reason for any intervention in multiple seeded episodes; normal runs remain fast |
| **R2 — Manual analyst MVP** | Offline selector, packet builder, replaceable model adapter, validated `finding.json`, review summary | On a seeded queue, produce several source-grounded findings; reject invented candidate IDs and missing evidence; no simulator-side AI calls |
| **R3 — Controlled validator** | Allow-listed `request_comparison` and paired-run executor; status/evidence lifecycle; report generator | Same inputs reproduce measured comparisons; a known wrong-mode case can be investigated without changing executors; run budget enforced |
| **R4 — Feedback export** | Approval gate; accepted regression and correction formats; split/ancestry checks; training integration | At least one accepted case becomes a runnable regression; at least one trusted correction is consumable by the ordinary supervised workflow; no sealed-test leakage |
| **R5 — Wider audits** | Scenario design, simulator fidelity comparison, hierarchy/report review, optional Operations teacher | Demonstrate usefulness on a second failure class, and independently measure whether accepted changes improve held-out mission outcomes |

**Required negative tests:**

- A bad-outcome episode where the original decision was rational from legal observations: reviewer must not automatically create a label.
- A `HOLD`/`DELAY` mission permitting sacrifice: reviewer must respect intent and loss authority.
- A decision where `KITE` was requested but dwell/scheduler kept `HOLD`: blame must reflect the actual action path.
- An appealing counterfactual action absent from or masked in the original candidate list: label must be rejected as an action correction and routed to action-vocabulary investigation.
- A simulator mismatch that reverses a tactic ranking: MiniBAR-only training conclusion must be qualified/quarantined.
- An identical replay reviewed twice: source IDs, deduplication, and evidence states remain consistent.
- A case where one paired seed favors an alternative but the aggregate is uncertain: status remains `inconclusive`.
- An adversarial scenario derived from a sealed test: export to training/development must be blocked.

Set practical numerical thresholds for review usefulness, overhead, and promotion **after measuring a small pilot**, not by asserting universal accuracy rates. Track (a) reviewer hypothesis confirmation/rejection rate, (b) fraction of findings resulting in a useful test or bug report, (c) simulator runs/agent cost per accepted artifact, (d) human review time, and most importantly (e) independently measured downstream mission gains and regressions.

## A.12. First implementation assignment for another agent

**Objective:** Deliver a minimal, external AI-assisted after-action review loop for MiniBAR formation selection, with enough evidence to distinguish selection error from execution/mission error and to validate one proposed alternative. **Do not modify the V2 training workflow, add AI calls to MiniBAR, or build a new policy trainer.**

**Work order:**

1. Audit the current MiniBAR code and CLI. Document what exists for trace export, scenario seeds, paired reruns, decision data, replay/intervention, and current training manifests. Do not assume snapshot branching or a specific CLI flag exists.
2. Design/version the smallest practical `manifest`, `episode`, `decision`, `review_packet`, `finding`, and `validation` schemas; explicitly define observation legality and how to link events to policy/bundle and mission versions.
3. Add only **generic, optional instrumentation** to MiniBAR where existing outputs lack the required facts. Keep the trace overhead measurable and avoid agent/provider-specific concepts in simulator code.
4. Implement the external Python queue/packet/review/validation shell with a model-provider adapter and strict JSON validation. Allow manual invocation first; the model must have read-only evidence access and may propose, not directly execute, comparisons.
5. Provide a bounded paired-scenario validation operation using the current supported MiniBAR execution interface; report whether exact decision branching is possible and defer it if not.
6. Demonstrate the full path on at least two scenarios: one suspected wrong formation choice and one good choice with bad downstream execution or parent instruction. Show that the findings are routed differently.
7. Export one reproducible regression scenario and, only if supported by controlled evidence and legal candidates, one candidate supervised correction. Record rejected/inconclusive hypotheses too.
8. Report instrumentation overhead, review/validation cost, reproducibility, known simulator fidelity limits, security assumptions, and remaining gaps. Propose the next narrow expansion only after the MVP proves useful.

**Required handoff:** external package and command examples; schema documentation with examples; test fixtures; a review queue and two worked findings with source references; validation run IDs and metrics; a regression artifact; optional supervised example with provenance; and a short implementation/evaluation report.

**Definition of done:** Given a completed MiniBAR formation campaign, an operator can invoke **one external command sequence** to select a small set of episodes, obtain grounded agent diagnoses, validate at least one proposed comparison, and produce an **approval-ready** issue, regression test, or legal supervised example. The simulator never invokes an AI service; the trainer remains unchanged until an explicitly accepted artifact is used in a new campaign.

---

### Recommendation in one sentence

Treat a capable AI agent as an **offline scientific collaborator**—one that proposes diagnoses, counterexamples, and experiments—while MiniBAR remains a neutral simulator and **measured outcomes, legal observations, and existing release gates** decide what the bot actually learns.
