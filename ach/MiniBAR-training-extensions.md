# MiniBAR: Hierarchical Training Support — Implementation Handoff

**Date:** 2026-10-19  
**Status:** Source-review findings and proposed work; not implemented or benchmarked in this review.  
**Repository reviewed:** `https://github.com/wlockwood/minibar`  
**Branch reviewed:** `tiered-brains`. The exact commit SHA was not resolved by this web audit. Pin the actual checkout before implementation and recheck these findings against it.  
**Basis:** The attached *BAR Hierarchical AI Architecture and Training Roadmap* [A] and *BAR Hierarchical AI Training Workflow* [T], compared with MiniBAR source. Reference IDs below identify specific files and symbols.

## Recommendation

Extend MiniBAR rather than replacing it. The principal missing layer is a persistent, mission-conditioned hierarchy plus a shared sequential experiment interface. The reviewed repository already has tick-level simulation, integrated economy/construction/combat, legal-observation machinery, evolutionary training, evaluation infrastructure, traces, and replay verification. Those are foundations to reuse, not tasks to rebuild. [R1–R6, R11–R14]

The first target is a scripted parent assigning missions to a platoon that selects complete coordinated plans executed by deterministic squads. Compare a heuristic selector, an evolved selector, and a PPO selector only after they act through the same interface. Then add the Operations → Platoon → Squad vertical slice with scarce, costed scripted supply. Live Quartermaster and full-game Strategy come afterward. This follows [T] §§1, 3.3, 7.1; the architecture's proposed squad-network training is not a requirement to replace competent scripts.

**Scope boundary:** Put game mechanics in the simulator; hierarchy state, contracts, ownership, and deterministic tactical controllers in reusable runtime modules; episode objectives and rewards in the scenario runner; optimizers and PPO rollout storage in training code. Do not embed PPO or strategic reward rules into projectile physics.

This audit read source and project documentation. It did not execute the repository test suite, measure throughput, train policies, or compare live BAR/Recoil encounters. Existing test code and calibration facilities are not evidence that this review has qualified their behavior.

## Existing implementation worth preserving

| Foundation | Reviewed locations | Relevant distinction |
|---|---|---|
| Incremental simulation | `engine.rs::CombatStepper`, `world.rs::World::step`, `live.rs::StepWorld` | A sequential simulator already exists; the missing training boundary is above it. |
| Actions, observations, routes | `strategy.rs`, `sensors.rs`, `routing.rs`, `world.rs` | Side-level action legality and filtered observations are not yet hierarchical delegation and mission authority. |
| Economy and production | `economy.rs`, `construction.rs`, `world.rs` | Streaming resource allocation, projects, factories, repair/reclaim and delivery-related movement can support a planner. A capability-request planner is a separate responsibility. |
| Tactical execution and training | `policy.rs`, `learn/tactical.rs`, `learn/air.rs`, `learn/baselines.rs`, `learn/train.rs` | Per-unit tactical modules and frozen ground/air companions are not persistent platoon/squad controllers. |
| Campaign infrastructure | `learn/es.rs`, `learn/campaign*.rs`, `docs/TRAINING_EXPERIMENTS.md` | Reuse evolutionary evaluation, checkpointing, frozen artifacts and campaign evidence; extend them for the new task. |
| Diagnostics | `trace.rs`, `live.rs::ReplayLog`, `calibration.rs`, `score.rs` | Replay-from-start is not a complete restorable mid-episode snapshot. A calibration comparator still needs trustworthy reference observations. |

### Source details that materially affect the design

1. `learn/train.rs::RosterEntry` already has a `platoon` field, but its documented role is initial formation grouping. `Spawn`/tactical `UnitState` do not carry the proposed persistent hierarchy membership. Preserve the input feature while introducing runtime identities. [R7, R15]
2. `learn/tactical.rs` already includes `tactical_v4` and a `mission` module focused on key assets. It does not implement an externally assigned contract with arbitrary success criteria, deadlines, delegated loss authority and upward reports. V4 also removes V3's whole-side commitment/anti-stall doctrine; do not propose that removal as though V4 still needs it. [R8]
3. `learn/strategic.rs` currently combines economy/production choices with direct combat orders; its combat groups are rebuilt from spatial buckets. Preserve this as a legacy baseline rather than relabeling its modules Strategy, Quartermaster and Operations. [R9]
4. Filtered observations are implemented, but `ObservationPolicy` defaults to `Omniscient`; standalone tactical sensing also retains legacy oracle fallbacks. New hierarchy campaigns must select their observation profile explicitly. [R4, R5]
5. Existing `FormationSpec` patterns materialize starting positions. They are useful geometry utilities, not persistent squad formation controllers. [R4]
6. The inspected optimizer actually uses antithetic evolutionary trials and rank-based fitness updates. Its use of Adam does not make it PPO. Existing weights and their feature schemas must not be assumed compatible with a new platoon actor. [R11]

## Priority and dependency overview

**P0:** Required for the first credible mission-conditioned platoon experiment, spanning architecture M0–M4. Implement the scripted path first; PPO-specific work may follow it.  
**P1:** Required for the supplied-force Operations vertical slice, architecture M5.  
**P2:** Required only when proceeding to live Quartermaster, full-game Strategy or additional mechanics, architecture M6–M8.

Before edits, record the source SHA, dependency lockfile, compiler/platform, catalog/map hashes and baseline configurations. Run the existing relevant tests and save their results. Capture baseline simulation work and wall time with tracing both enabled and disabled. Preserve legacy configurations and checkpoint schemas.

Suggested new paths in the tickets are organizational proposals, not existing source files.

---

## 1. P0 — Add a persistent hierarchy episode runner above the existing stepper

**Owner:** Simulator integration and episode runtime.  
**Extend:** `src/world.rs`, `src/live.rs`, `src/engine.rs`; introduce a small episode/runtime module. [R1–R3]

Create one environment instance owning the world, scenario state, controller instances, clocks, pending decisions and outcome accumulator. Immutable catalogs and frozen weights may be shared; mutable controller state must be isolated per environment. Prefer an integrated `World` backend for the hierarchy so later production and Operations use the same state. A smaller combat-only backend is acceptable behind the same contract when its semantics are tested, not as a second set of physics rules.

Expose reset, obtain current legal decision context, apply selected plan, and advance to the trained layer's next fixed decision boundary. Execute frozen parents/siblings/children and ordinary simulation frames inside this interval. Configure cadences in integer simulation ticks and define a stable update order. Do not turn `step_seconds(large_interval)` into a substitute for the equivalent sequence of native ticks without testing equivalence.

`World::step_seconds` clears the current event list each step. Collect events and rewards after every native tick; reading events only after an entire platoon interval can lose intervening outcomes. [R2]

**Acceptance:** Repeated reset with the same configuration, seeds and action sequence reproduces selected state hashes and outcomes; interleaving two environments does not alter either trajectory. One decision step advances the documented number of ticks, except a genuine terminal boundary, and preserves all interval events.

**Depends on:** Minimal schemas from tickets 2–3; can be developed alongside them.

## 2. P0 — Introduce persistent groups, delegated ownership and command arbitration

**Owner:** Hierarchy runtime; small executor integration.  
**Extend:** `learn/train.rs` roster materialization, `strategy.rs` action validation/sequence handling, `world.rs` command integration; proposed `hierarchy/groups.rs` and `hierarchy/scheduler.rs`. [R4, R7, R15]

Add persistent platoon and squad IDs, unit membership, roles, parents, anchors, active assignments, and a roster revision. Keep authoritative combat IDs stable; store hierarchy metadata alongside them instead of teaching the physics kernel what a platoon means. Handle casualties, reinforcement, temporary detachment, explicit transfer and singleton squads.

Enforce one active low-level command owner per unit. Parent controllers delegate tasks rather than independently issuing conflicting moves to delegated units. Separate ownership transfer from motion and acknowledge transfers before the new owner acts. Apply delegated authority in addition to existing side ownership and engine legality.

Create a command scheduler with plan persistence, coalescing of equivalent orders, minimum dwell times, and permitted emergency overrides. Adapt the existing per-side action sequencing centrally so multiple controllers do not collide in sequence numbers. Record proposed, suppressed, issued, rejected and emergency commands separately.

**Acceptance:** Two nearby platoons retain distinct identities after crossing the same spatial bucket. A transfer leaves no dual owner or stale command stream. Repeating `CONTINUE` does not cancel/rebuild a valid movement plan. A permitted emergency can interrupt it and subsequently restore the assignment. Simultaneous controller submissions are deterministically ordered.

## 3. P0 — Implement typed mission contracts and the upward report lifecycle

**Owner:** Reusable hierarchy protocol, not neural-network code.  
**Add:** Proposed `hierarchy/contracts.rs`, `hierarchy/missions.rs`, schema fixtures.  
**Basis:** [A] §3; [T] §4.

Implement `MissionOrder`, `MissionReport`, `CapabilityRequest`, `CapabilityOffer`, typed conditions/constraints and acknowledgments. Include schema version and mission revision explicitly on every relevant message; the roadmap's illustrative fields need this completion. Store parent/recipient, effect, intent, target, assigned force, priority, issue/expiry/deadline ticks, permitted deviations, resource authority and expected-loss allowance.

Implement the proposed lifecycle including accepted, active, at-risk, blocked, completed, failed and canceled. Specify allowed transitions, duplicate handling, stale-revision rejection, report freshness and bounded objection/reply behavior. Retain original objectives, revisions, spent resources and unresolved effects after cancellation.

Start reports with measured progress and evidence-backed blockers plus conservative confidence classes. Separate a concern, a recommendation and a refusal. A child may propose a changed objective but must not redefine its own success. Parent confirmation of a costly mission must carry explicit authority, not silently erase a prohibition.

`maxExpectedLossMetal` is a risk allowance, not a promise that random damage cannot exceed it. Enforce deterministically checkable prohibitions; measure uncertain loss-budget violations separately.

**Acceptance:** A scripted parent issues a hold, receives a meaningful objection, then either revises it or explicitly authorizes a costly delay. The child acknowledges the correct revision and follows it. A stale report cannot cancel the replacement mission. An accepted order does not generate an endless objection loop.

## 4. P0 — Make observation legality and mission conditioning explicit at every actor boundary

**Owner:** Observation adapter and shared services.  
**Extend:** `strategy.rs` observation types, `sensors.rs`, `world.rs`, tactical feature extraction and `docs/BAR_SENSORS.md`. [R4, R5, R8]

Use existing filtered observations and contact memory. Add a versioned per-tier feature registry covering mission intent/constraints, remaining time, authority, roster/roles, current plan, neighboring assignments, report history and only legally available terrain/contact information. Include explicit known/unknown masks and contact age/error; do not let placeholder zero enemy HP mean “nearly dead.”

Require an explicit observation profile for every new campaign. Reject incomplete metadata that would silently activate legacy oracle sensing in a deployment-intended experiment. Deliberately fully observed toy tasks can remain supported, but label them as such and do not treat them as fog-of-war qualification.

Keep private evaluation state separate from actor inputs. Candidate generation, masks, heuristic priors, forecasts, contact matching and normalization must use the same permitted information. In particular, querying an authoritative route/placement validator to reveal an unseen blocker must not leak into a pre-action mask. Final execution may reject an action according to actual game mechanics; disclose only the information the real interface would provide.

**Acceptance:** Two worlds with identical legal information histories but different hidden enemy state produce identical actor features, candidate descriptions, masks and deterministic decisions. When a legal sighting occurs, the observations may diverge. Radar/remembered contacts cannot reveal concealed health, identity or weapon state through derived features.

## 5. P0 — Wrap and extend deterministic tactics into actual squad controllers

**Owner:** Tactical controller library.  
**Extend/reuse:** `policy.rs`, `learn/baselines.rs`, `strategy.rs` formation utilities, existing tactical execution. Proposed `controllers/squad.rs`. [R4, R6, R8, R10]

Implement the first squads with deterministic charge, hold, orbit, encircle and kite behaviors, plus explicit withdraw/regroup and simple screening/support tasks as needed by the first platoon plans. Reuse existing primitives rather than rewriting their motion math. Missing behavior must receive its own controller test before being offered as a competent option.

Give a squad a roster, role, goal/anchor, threat-relative reference frame, target preference and a bounded formation envelope. Compute legal, reachable per-unit destinations deterministically. Preserve useful slot assignment through movement and casualties; include separation, terrain constraints and sibling geometry. Individual reflexes must remain within delegated mission authority.

Do not put hidden mutable squad state into the shared read-only `Policy: Sync` abstraction. Keep episode-local state in the runtime and immutable policy weights shareable. Existing tactical brains can remain baselines or explicitly scoped children, but cannot retain conflicting private objectives under the new parent.

**Acceptance:** A screen protects the assigned backline during withdrawal; a hold does not drift into a pursuit; a kite stays within its permitted operating area. Identical units respond differently to different legitimate squad tasks. Role changes and losses do not cause continuous slot or command churn. Profile issued commands as well as controller evaluations.

## 6. P0 — Add complete platoon-plan candidates and a common selector interface

**Owner:** Platoon controller/action adapter.  
**Add:** Proposed `controllers/platoon_candidates.rs`, `controllers/selector.rs`; reuse terrain/routing services.  
**Basis:** [T] §§2.3, 3.3; [A] §§6, 8.3.

Generate a small bounded set of complete coordinated plans: for example anchor-and-screen, screen-and-withdraw, advance with support, flank with a holding squad, regroup, continue, and report a blocker. Each contains its squad tasks, target/region, route/anchor, timing, commitment and permitted fallback. Avoid the full Cartesian product of independent squad orders.

Separate stable candidate/template IDs and versioned payloads from temporary tensor slots. Supply candidate features alongside mission/context features; pad with explicit masks where needed. Include at least one valid continuation/fallback path. Mask true invalidity, not authorized but risky tactics. Record the exact list and execute its stored payload, not a newly regenerated interpretation of the chosen slot.

Use a heuristic selector as the first implementation. A learned residual over a heuristic prior is optional, but the prior must participate in the same distribution used for sampling, recorded log probabilities, updates, cloning and deployment.

**Acceptance:** All selectable candidates can be executed or produce a documented execution failure. Padded slots cannot be selected. Reordering candidates does not corrupt IDs or human labels; a candidate-aware scorer preserves candidate scores under the corresponding permutation. A reviewer can identify a reasonable option on the initial scenarios, or the failure is routed to candidate coverage rather than the optimizer.

## 7. P0 — Extend objectives into mission scenarios, independent evaluators and interval rewards

**Owner:** Scenario/evaluation layer.  
**Extend:** `scenario_compile.rs`, `learn/objective.rs`, `score.rs`, `learn/strategic_reward.rs`; proposed mission evaluator and scenario fixtures. [R12, R13]

Existing objective and reward code is useful, but attack/defend key-asset outcomes do not cover the proposed contract vocabulary. Add the smallest complete initial set: HOLD, WITHDRAW and DELAY, followed by CAPTURE/ESCORT/RECON only when execution, observations and evaluation support their meaning. Keep match outcomes, parent mission completion and child-task completion separate.

Define regions, protected forces, deadlines, boundary crossings and objective state; add reproducible reinforcement/threat events. Pair the same battlefield with different mission intent and loss authority. Hold must measure holding; delay must measure delayed breakthrough or protected-force escape; withdraw must measure reaching safety with the requested force. Killing valuable enemies cannot automatically compensate for abandoning the assigned effect.

Accumulate a decomposed reward ledger at native-tick resolution and expose interval deltas. Keep sparse mission/global outcomes independently. Preserve sunk costs across revisions. Do not recursively sum all children's shaped rewards into the parent. Separate genuine task termination from engineering truncation; do not stop a campaign because one squad was eliminated or one local mission ended.

**Acceptance:** Authored scenarios include clear success, clear failure, a known blocker, and authorized sacrifice. A behavior that earns good metal exchange but abandons a required hold fails the mission. Repeated revise/cancel/request cycles cannot create net reward or erase losses. Every event contributes at most as specified to its objective ledger.

## 8. P0 — Extend traces, reproducibility and review data instead of building parallel logging

**Owner:** Instrumentation and datasets.  
**Extend:** `trace.rs`, `learn/explain.rs`, `learn/strategic_telemetry.rs`, `live.rs::ReplayLog`, existing viewer/campaign artifacts. [R3, R8, R14]

Add episode/decision/mission/revision IDs, legal observation artifact, exact candidates and masks, chosen candidate payload, issued commands and execution exceptions, report lifecycle, reward terms, outcome reasons and elapsed ticks. Join trainer-side sampled action probabilities/log probabilities and values by decision ID; the physics engine does not need to compute them.

Keep separate datasets for current-policy PPO rollouts, demonstrations/corrections, forecast labels, human reviews and evaluation. Save forecast-time legal inputs, continuation-policy version and outcome censoring. A cancellation is not a successful-completion ETA label. Keep actual objective truth separate from actor-visible clips when showing a review.

Extend manifests to include hierarchy schema, scenario split, candidate generator/prior, child/opponent bundle, reward version, normalizers, command settings and observation profile. Preserve existing checkpoint/export support. Review records should reference stable candidate IDs, support uncertainty, and flag missing candidates rather than assigning invalid class labels.

Initial reproducibility can use seeded reconstruction plus action replay and state hashes. Full snapshot/branch support is useful later, not a prerequisite for PPO. Any snapshot claim must include physics/projectiles, resource/build jobs, RNGs, contacts, pending actions, ownership, mission/report state and scheduler clocks; entity positions alone are insufficient.

**Acceptance:** A saved decision can be reconstructed for review without regenerating its candidates. Replay reproduces outcomes at checked boundaries. Swapping/resetting a controller does not leak state between episodes. If snapshots are later added, uninterrupted execution and snapshot-resume match, including across a pending projectile hit and mission revision.

## 9. P0 — Add a Gymnasium adapter and compare optimizers on the same platoon task

**Owner:** Training bridge/trainer, not simulator mechanics.  
**Extend:** Existing ES/campaign harness; add Python adapter and a versioned persistent Rust worker or native binding. [R11, R14; T §§2.2, 6, 9]

Provide the workflow's `reset(seed, options) -> (observation, info)` and `step(action) -> (observation, reward, terminated, truncated, info)`. Start with one fixed-duration decision interval per environment and a feed-forward discrete candidate selector. Expose masks through the mechanism required by the selected trainer. Preserve the true final observation before vectorized autoreset and test timeout bootstrapping.

Use an established PPO implementation, provisionally the workflow's SB3/MaskablePPO path after a pinned compatibility smoke test. A persistent local request/response worker is a straightforward initial bridge; benchmark it before choosing a faster native/batched transport. Do not launch a new simulator process per decision or put browser rendering in the hot path. Bound requests and surface simulator errors as errors, not fabricated terminal rewards.

Connect heuristic and ES selectors to the identical episode runner, features, candidate set, frozen squads, rewards and scenarios. New platoon feature/action schemas require explicit new actor checkpoints or migrations, not blind transplantation of existing tactical weights. PPO owns fresh rollouts, its critic and optimizer; historical ES battle summaries are not PPO rollout data.

**Acceptance:** Run collect → update → save → reload → evaluate for PPO, plus ES evaluation through the same task. Verify masks, logged action probabilities and actual deployed action selection. Compare native and exported actor scores/masks on saved legal observations. Report mission outcomes, seed variability, simulated ticks/game time, decisions, evaluation cost and wall time; retain the heuristic if learning adds no demonstrated value.

## 10. P0, continuing — Qualify mechanics by training profile using the existing calibration layer

**Owner:** Simulator fidelity/engine bridge.  
**Extend:** `calibration.rs`, `score.rs`, `model.rs`, relevant engine/map/routing tests and reference-capture adapters. [R15–R17]

Create a feature profile for each scenario family, listing represented, approximated, unsupported and empirically qualified mechanics. Fail the deployment-qualification gate when a task depends on unsupported or unqualified behavior; exploratory simulator runs may remain labeled as such. Do not require all BAR mechanics before the first restricted platoon experiment.

First qualify movement/turning, reachability, collision/obstruction, range and firing while moving, relevant projectile timing, target eligibility, finite energy where consequential, and legal sensing. Compare the ranking of deterministic tactics across matched MiniBAR and BAR encounters, not only aggregate damage. Include flat and elevation cases with more than one weapon regime and terrain where the attractive ridge is correctly rejected.

Use the existing calibration comparator, but obtain reference traces from an actual pinned BAR/Recoil setup. Preserve catalog/map/engine/mod provenance and tolerate differences only under declared bounds. Treat simulator timings as simulator timings; they do not establish Recoil's pathfinding or command costs.

The reviewed model already represents shields, stockpiles and interception-related state. It also explicitly approximates repulsers as absorption and documents simplified interception behavior. Those models require further work before training the roadmap's shield-geometry or nuclear-saturation decisions. Do not replace a missing timing model with a hard-coded “seven interceptors” rule. [R15, R17]

**Acceptance:** Initial supported scenarios have reference results, explicit tolerances and a tactic-ranking report. Regressions are attributable to a mechanic, action-execution or controller difference. Unsupported scenarios cannot silently enter a claimed engine-qualified campaign.

---

## 11. P1 — Add Operations sectors, opportunities and coordinated allocation

**Owner:** Operations runtime and scenario services.  
**Add:** Proposed sector/opportunity registry and Operations candidate controller; reuse map, routing, sensor and roster services.  
**Basis:** [A] §§4–5, M5; [T] §3.4.

Represent a small sector graph/grid with objective value, access, legal threat/uncertainty, current missions, available platoons and reserves. Supply deterministic opportunity candidates for useful staging positions/high ground, recon, threats and target effects. Separate generating a plausible opportunity from selecting it under strategic priorities.

Operations actions should assign/revise/cancel missions, coordinate a few complete multi-platoon templates, commit a reserve, scout, request capability, accept an offer, or continue. The current direct-combat “strategic” controller remains a separate baseline until its responsibilities are deliberately separated.

Keep child platoons frozen while establishing the scripted baseline. Test reactions to reports: reinforcement, changed timing/objective, explicit loss authority, alternative force allocation or escalation. Avoid perpetual reassignment that prevents subordinate execution.

**Acceptance:** Two concurrent objectives create real opportunity cost; one local mission ending does not end the campaign. Operations sometimes secures a ridge and sometimes ignores it. Coordinated attack/support timing survives routine updates. Report-aware and report-ablated bundles can be compared with the same children.

**Depends on:** Tickets 1–8 and qualified scenario mechanics; PPO is not a prerequisite for a scripted Operations demonstration.

## 12. P1 — Implement scarce, deadline-aware capability requests and scripted supply

**Owner:** Request/offer protocol, budget ledger and scenario supply service.  
**Reuse:** Existing catalog, dynamic entities and sensor-support units; do not confuse initial support injection with an operational request loop. [R18; A §3.5; T §3.4]

Add request → feasible/negative offers → authorization/reservation → acceptance → assembly/delivery → ownership handoff, with cancellation/revision and costs retained. Requests specify effects and capabilities, not only unit recipes. Scripted offers may use predefined compositions initially, but include costs, lead time, latest useful arrival, delivery location, availability and reasoned refusal.

Implement scarce stock or budget and opportunity cost. Do not give unlimited free units for every accepted request. Existing nearby units, ordinary attack, recon, deferral and specialized force acquisition should compete. Deliver actual units into the scenario with stable IDs and explicit ownership; assembly and travel must not disappear behind an instantaneous ready flag.

**Acceptance:** A missing-counter scenario results in a justified request and useful deployment. Insufficient budget, unavailable tech/stock or impossible deadline can produce an honest negative offer. Two requests cannot reserve the same resources/force. Raising the cost or delaying delivery makes a conventional alternative or deferral preferable in at least one paired test.

**Depends on:** Tickets 2–3, 7, 11; can begin scripted before any learning.

## 13. P2 — Connect a real Quartermaster planner to existing streaming production

**Owner:** Deterministic planning/scheduling; small action API extensions where needed.  
**Reuse:** `economy.rs`, `construction.rs`, `world.rs`, placement/catalog/routing and current production-candidate utilities. [R9, R19, R20]

Replace the scripted supply provider behind the same request/offer interface. Plan prerequisites, feasible composition, builders/factories, queue order, resource streams, assembly and travel. Account for shared build power and competing authorized commitments without double-reserving present stocks or future capacity. Keep strategic spending envelopes and deterministic financial accounting separate from optional learned priorities.

Produce conservative ETA/blocker updates from actual progress and resource forecasts. Handle power/income shocks, factory/builder loss, priority changes, placement failures and delivery losses. Expose narrow typed job-priority/build-power controls when needed; the reviewed action enum does not itself provide the entire desired scheduling vocabulary. Reuse lower-level priority facilities rather than making new resource equations.

Train completion/delivery forecasts only after independent scheduler tests; calibrated prediction or learned ranking is optional. Capabilities must be validated in mission rollouts, not merely matched as a sum of unit-stat vectors.

**Acceptance:** Real delivered units fulfill an operational effect before its useful deadline; competing requests cannot double-spend; a shock changes ETA and causes an explicit revision/refusal. Planner accuracy and on-time useful delivery are measured separately from economic utilization. No learned Quartermaster is required to pass.

**Depends on:** Ticket 12 and economy/production fidelity qualification.

## 14. P2 — Separate true Strategy and gate advanced mechanics before full-game training

**Owner:** Strategic runtime, scenario curriculum and mechanics-specific extensions.  
**Basis:** [A] M7–M8 and §10; [T] §3.6. [R9, R15–R17]

Create macro commitments for broad goals, spending envelopes, tech priorities, risk posture and major requests. Strategy delegates battlefield objectives to Operations and production to Quartermaster rather than retaining uncoordinated direct unit orders. Establish a scripted full-game baseline with frozen competent lower layers before strategic RL.

Use actual match outcome as the principal evaluation result and keep existing economic scores as diagnostics or explicitly tested shaping. Add varied openings and mid/late-game scenarios with snapshot provenance. A curated crisis success is not a substitute for a full-game result. Defer simultaneous multi-layer adaptation and team ownership until the single-player hierarchy works.

Add only mechanics required by the next supported scenario family. Examples include explicit manual-fire and self-destruct commands and their timing/wreck effects; qualified cloak/EMP infiltration; specialized mobility/transport; accurate shield interactions; and interceptor flight/detection/reload/stockpile timing. Exported metadata or a generic move/attack action is not proof that a capability can be controlled and evaluated. Existing basic air/sensor support should be extended and qualified, not described as nonexistent.

**Acceptance:** The identical lower bundle achieves better held-out full-game outcomes under a new Strategy before promotion. Each newly advertised special capability has an action path, observations, resource/timing semantics, outcome tests and engine-reference evidence. Rollback restores a compatible whole bundle, not just actor weights.

---

## First implementation milestone to request from the next agent

Do not initially request “all fourteen tickets complete.” Request a runnable, scripted **mission-contract platoon sandbox** with:

1. Persistent squads/platoons and exclusive ownership.
2. HOLD, WITHDRAW and authorized DELAY from matched starting states.
3. A small library of deterministic squad behaviors and complete platoon candidates.
4. Legal mission-conditioned observations and meaningful blocker/revision behavior.
5. Fixed-decision stepping, independent outcomes, interval rewards and replayable decision traces.
6. A declared mechanics profile and initial BAR comparison evidence for the tactics involved.

Then run the shared heuristic/evolution/PPO selector pilot. If squad execution is the failure, fix it first; if no good plan is offered, extend candidates rather than spending more optimization budget. Only then expand to tickets 11–12 for Operations and supplied capabilities.

### Suggested acceptance fixtures (new tests, names are proposals)

| Fixture | Required result |
|---|---|
| `same_seed_same_actions` | Same checked episode hashes and outcome; parallel environments remain isolated. |
| `native_tick_event_accumulation` | Events on intermediate ticks appear exactly as intended in interval rewards and traces. |
| `group_crossing_and_transfer` | Persistent identity survives crossing; exactly one command owner after handoff. |
| `continue_preserves_order` | Continuation does not reset a live plan; emergency override follows authority. |
| `mission_revision_stale_report` | Stale messages cannot affect the new contract. |
| `hold_withdraw_delay_pair` | Intent and authority change valid behavior and outcome assessment. |
| `known_blocker_vs_hidden_threat` | Known impossibility is reported; undisclosed threats are not magically diagnosed. |
| `hidden_world_twins` | Equal legal information histories yield equal actor features/candidates/masks. |
| `candidate_reorder_and_padding` | Stable candidate identity survives reorder; padded actions are unselectable. |
| `timeout_final_observation` | True terminal vs truncated status and bootstrapping observation are correct. |
| `python_rust_actor_parity` | Exported scorer and training scorer match on saved legal decisions. |
| `costed_request_deadline_pair` | Budget/deadline changes alter a sensible acquisition decision. |
| `request_double_reservation` | No duplicated claim on scarce resources or force. |
| `tactic_ranking_reference` | MiniBAR and BAR comparisons expose and gate consequential ranking reversals. |

### Expected handoff evidence

Return the pinned source/dependency manifest, changed schemas, runnable scenario/campaign configs, test results, baseline and candidate metrics by mission family, representative legal decision traces, a small review packet, mechanics qualification/known-limitations table, throughput and command-cost measurements, and full-bundle export/rollback instructions. Report work not yet completed explicitly. Do not infer an algorithm win from a single checkpoint or a higher shaped training reward.

## Sources and exact inspection entry points

The repository URLs are given as code so they can be copied into an agent's source inspection. Relative source paths below refer to the reviewed `tiered-brains` branch. Replace the branch with the pinned commit in permanent implementation documentation.

- **[A]** Attached `BAR_Hierarchical_AI_Architecture_and_Training_Roadmap.md`, especially §§2–6 and 8–10.
- **[T]** Attached `BAR_Hierarchical_AI_Training_Workflow.md`, especially §§1–4 and 6–10.
- **[R1]** `src/engine.rs`: `CombatStepper`, `CombatCommand`, per-tick `step`, target observations, combat resource configuration and deterministic state hashing.
- **[R2]** `src/world.rs`: `World::step`, `step_seconds`, observation policy, `advance_combat`, `prepare_combat_commands`, action queue and economy/production integration.
- **[R3]** `src/live.rs`: `StepWorld`, `LiveDriver`, `ReplayLog`, `replay_and_verify` and step messages.
- **[R4]** `src/strategy.rs`: `StrategyAction`, `QueuedAction`, action validation/sequencing, `ObservationPolicy`, observed contacts, `FormationSpec`/`FormationPattern` and `StrategyCadence`.
- **[R5]** `docs/BAR_SENSORS.md` and `src/engine.rs::StandaloneSensors`: supported sensing, unknown-value limitations, legacy oracle behavior and reference-validation caveats. Read source when older scope statements differ from new code.
- **[R6]** `src/policy.rs`: read-only per-unit `Policy: Sync` and existing deterministic combat policies.
- **[R7]** `src/learn/train.rs`: `RosterEntry::platoon`, `formation`, tactical configuration, resource supply and frozen companion brain support.
- **[R8]** `src/learn/tactical.rs`: V3/V4 kinds and specs, `think_v4`, key-asset mission scorer, `DecisionTrace`, `ModuleTrace`, `TacticalPolicy` and legacy doctrine differences.
- **[R9]** `src/learn/strategic.rs`: investment/production/mission module specs, `actions`, production candidates, spatial combat grouping and direct order emission.
- **[R10]** `src/learn/baselines.rs`: objective-aware `strike`/`guard` baselines and combat primitive reuse.
- **[R11]** `src/learn/es.rs`: antithetic noise trials, ranked fitness update and resumable optimizer state.
- **[R12]** `src/learn/objective.rs`: `ObjectiveKind::{Attack,Defend}`, `BattleAssessment` and objective-aware battle evaluation.
- **[R13]** `src/learn/strategic_reward.rs`: configurable component-based strategic reward and legacy defaults.
- **[R14]** `docs/TRAINING_EXPERIMENTS.md`, `src/learn/campaign.rs` and its companion modules: campaign artifacts, evaluation splits, freezing, compatibility and evidence.
- **[R15]** `src/model.rs`: `Spawn`, `UnitState`, `WorldView`, weapon/movement/flight/resource definitions and explicit mechanics approximations.
- **[R16]** `src/calibration.rs`: reference fixture schemas, comparison/check machinery, unsupported/approximation handling; capture is an adapter responsibility.
- **[R17]** `src/score.rs`: capability flags, outcome/milestone metadata and declared approximation warnings.
- **[R18]** `src/learn/sensor_support.rs`: destructible support-unit injection, not an operational requisition lifecycle.
- **[R19]** `src/economy.rs`: streaming resource demand and priority allocation.
- **[R20]** `src/construction.rs`: project costs/build-time, shared work and partial construction state.
- **[R21]** `src/routing.rs`: route requests/status and existing grid-routing service.

Source base: `https://raw.githubusercontent.com/wlockwood/minibar/tiered-brains/`.
