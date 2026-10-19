# Beyond All Reason AI: Proposed Per-Layer Training Workflow

**Status:** Proposed training design and engineering handoff; no experiments or engine integration have been performed for this document.  
**Date:** 2026-10-19  
**Companion:** [BAR Hierarchical AI Architecture and Training Roadmap](<Hierarchical-Brains.md>)  
**Audience:** The agent or engineer implementing the training system.

## 0. How to use these two documents

The architecture roadmap owns the responsibility boundaries, mission/report protocol, observation legality, and decision ownership. This companion proposes **how to obtain, improve, compare, and promote the implementations of those responsibilities**. Read architecture §§2–6 for interfaces, §8 for controller scope, and §§9–10 for milestones and regression scenarios.

The intended result is a bot whose layers cooperate: Operations discovers a useful objective, requests a missing capability, receives a feasible delivery offer, assigns the resulting force, and revises the plan when a subordinate reports a problem. Training six isolated networks to earn unrelated scores would not establish that result.

Working assumptions:

- An existing battle simulator and evolutionary MLP trainer may be reusable, but their code, physics, and update procedure have not been inspected here.
- Training can be offline; deployment initially uses fixed, versioned policies. Online weight updates during live games are a later experiment.
- Human supervision consists of a few reviewed engagements per campaign, accumulating over time.
- Exact network sizes, decision frequencies, learning rates, sample budgets, and training durations must come from measured pilots.

Recommendations below are project-specific hypotheses. Framework capabilities are supported by the primary references in §11; they do not establish which algorithm will perform best in BAR.

## 1. Recommended division of work

**Build a functional scripted hierarchy, then replace individual judgments with learning when the replacement earns its cost.** Use supervised learning where labels are meaningful, PPO as the first gradient-based policy experiment, and the existing evolutionary trainer as a serious comparison. Keep geometry, legality, ownership, and resource accounting in ordinary code.

| Component                    | First working implementation                                                  | Proposed learning path                                                          | What would justify learning it?                                                  |
| ---------------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| Unit execution / reflexes    | Engine commands plus event-driven rules                                       | Optional imitation, then short micro RL or evolution                            | Better execution of the same squad task within the command/CPU budget            |
| Squad                        | Deterministic role-conditioned formation, targeting, and movement controllers | Optional teacher warm-start, then goal-conditioned PPO; compare evolution       | Better fulfillment of squad assignments across roles and terrain                 |
| Platoon                      | Candidate generator plus heuristic selection of complete coordinated plans    | First main PPO-versus-evolution experiment; add clear supervised corrections    | Better mission outcomes and useful objections with identical squads              |
| Operations                   | Sector/opportunity candidates plus scripted allocation and requests           | Goal-conditioned candidate selection, imitation where useful, then PPO          | Better objective/force selection and response to reports with identical children |
| Quartermaster                | Feasibility planner, streaming-resource forecast, and scheduler               | Supervised forecasts; optional learned ranking/priorities or parameter tuning   | Better on-time delivery of useful capabilities than the planner alone            |
| Strategic Director           | Scripted macro priorities, budget envelopes, and tech choices                 | Optional imitation, then full-game PPO or another measured policy-search method | Better match outcomes with the same Operations and Quartermaster                 |
| Reports / outcome estimators | Typed facts, rules, conservative estimates                                    | Separate supervised predictors with calibration                                 | Predictions improve parent decisions on unseen scenarios                         |
| Human-feedback evaluator     | Review dataset and diagnostic reports                                         | Optional preference/rating model after validation                               | Predicts mission-conditioned human judgments and improves real outcomes          |

**First learning target:** the platoon selector if existing deterministic squad controllers already execute useful tactics. Otherwise establish competent squads first. This is consistent with architecture M3–M4: competence at the lower interface is required; a learned lower interface is not.

The first integrated target remains **Operations → Platoon → Squad with supplied forces and scripted Strategy/Quartermaster**. Do not make a trained economy a prerequisite for demonstrating useful tactical and operational learning.

## 2. The shared training pipeline

### 2.1 Build one scenario runner with interchangeable controllers

Every module experiment should choose a scenario, instantiate a controller bundle, execute decisions, and save outcomes through the same runner. Swapping a heuristic selector for an evolved or PPO selector must preserve observation semantics, available actions, command execution, and evaluation.

The runner needs:

1. Seeded resets, scenario families, and a reliable replay/debug trace. State restoration and branching are valuable but must be verified rather than assumed available.
2. Separate actor-visible observations and authoritative evaluation state. Log their provenance; the latter must not leak into runtime inputs through candidates, masks, forecasts, or observation normalization. Outcome labels may use authoritative state as described in §4.2.
3. A versioned mission/action/report adapter, legality checks, command ownership, and command scheduling.
4. Independent outcome measurement and a decomposed reward ledger.
5. Frozen controller bundles for reproducible training and evaluation.
6. Engine-validation runs for simulator-trained behavior. Fidelity must cover the decisions being learned, not merely visual resemblance.

Use the custom simulator for cheap encounters only where engine comparisons support it. Start by testing whether **tactic rankings** agree: for example, whether uphill kiting beats a flat withdrawal in the relevant weapon matchup. A simulator that reverses that ranking supplies misleading training even if its average casualty count looks plausible. Treat wider randomization as a robustness tool, not a substitute for missing mechanics.

### 2.2 Start with fixed decision intervals per trained layer

Expose each initial learning environment through the Gymnasium interface [1]:

```text
reset(seed=..., options={"scenario": ...}) -> observation, info
step(action) -> next_observation, reward, terminated, truncated, info
```

One `step` advances **one decision interval for the controller being trained**. All child controllers and engine frames execute inside it. A platoon action may persist through many squad updates. A Strategy action may persist through many Operations updates. Each layer can have a different interval.

Initially, retain a plan until it expires, completes, or is replaced by a meaningful action. Include a valid `CONTINUE` option. Hard engine events and deterministic reflexes may act between policy decisions; record those interventions. Do not silently create variable-duration policy transitions while telling the trainer that every step has equal duration.

A later event-driven implementation needs duration-aware returns. If the reference discount is `gamma` per `tau` simulation seconds and a transition lasts `delta_t`, use:

```text
transition_discount = gamma ** (delta_t / tau)
return_target = discounted_reward_during_interval
                + transition_discount * next_value   # if bootstrapping is valid
```

The return and advantage calculations, including any eligibility-trace time convention, must agree with that timing. Merely putting `delta_t` in an observation or `info` does not change stock PPO's scalar-discount calculations. Fixed intervals are the recommended first implementation.

Set discount horizons in **game time and mission purpose**, then translate them into per-step values. Reusing the same `gamma` at every tier gives radically different effective horizons.

Distinguish actual task termination from external rollout/time cutoffs. An explicit hold deadline can be part of the task; an engineering timeout is a truncation. Preserve the final observation needed for bootstrapping, including with vectorized autoreset wrappers [2]. Decide explicitly whether a mission revision begins a new training episode or continues the same interaction; keep original and revised objectives in the trace.

### 2.3 Make candidate actions learnable and reviewable

For Platoon and Operations, start with a bounded set of **complete meaningful alternatives**. Each candidate contains its parameters, affected squads/platoons, proposed effect, and identifying metadata. Selection returns one candidate index; execution uses the stored candidate payload.

Important requirements:

- Give the actor features describing **each candidate**, plus the relevant situation and mission. A slot number alone cannot mean "flank" in one state and "withdraw" in another without sufficient information.
- Save the exact candidate list, order, features, and action mask with each decision. Reconstructing candidates later from changed code is unsafe.
- Include continuation and an appropriate fallback. There must be at least one valid action, even if the only possible result is reporting a blocker.
- Mask genuinely invalid actions: absent units, illegal prerequisites, or prohibited actions. Keep legal expensive or risky plans available when authority permits them.
- Separate stable candidate IDs from temporary tensor slots. Store corrections against the candidate description/ID and its version, not just `action = 3`.
- Measure **candidate coverage**: can a competent reviewer find an adequate action in the list? Selection learning cannot invent a missing maneuver.
- Keep candidate generation fixed during an algorithm comparison. Expanding it is a separate experiment.

Start with a small set of coordinated templates; do not enumerate every combination of squad tasks. If one-step mission edits create coordination problems, add explicit coordinated-operation candidates before introducing a much larger action space.

A useful optional actor is:

```text
logit(candidate) = heuristic_prior(candidate, observation)
                   + learned_correction(candidate, observation)
```

The prior must be finite for legal actions, consistently scaled, and included in the distribution used for **sampling, recorded log probabilities, PPO updates, imitation, and deployment**. It must not be a hidden post-processing override. Record any deterministic execution fallback separately. Initialize the correction near zero if the intent is to start from the prior, then verify that actual sampled behavior matches that intent.

SB3 PPO supports discrete actions and dictionary observations; SB3-Contrib MaskablePPO adds invalid-action masking [3–4]. A candidate-aware scorer or heuristic residual may require a custom policy. The established PPO optimizer can still be retained.

### 2.4 Keep training data in distinct datasets

| Dataset                    | Contents                                                                                                             | Appropriate use                                                       |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| Current-policy rollouts    | Observations, candidates/masks, sampled actions, log probabilities, values, rewards, boundaries                      | PPO updates for that collecting policy                                |
| Demonstrations/corrections | Actor-legal observations, available candidates, preferred action, confidence and provenance                          | Supervised policy training                                            |
| Forecast examples          | Information available at prediction time, proposed plan/mission, controller versions, observed outcome and censoring | Success, loss, ETA, and feasibility prediction                        |
| Human reviews              | Mission context, clip/decision references, ratings/preferences, tags, confidence                                     | Diagnostics, regression cases, optional evaluator training            |
| Evaluation episodes        | Independent scenarios and complete outcome records                                                                   | Model selection on validation; final reporting on a separate test set |

Past trajectories can train supervised models when their targets remain meaningful. They are not automatically valid current-policy PPO experience. Keep rollouts separate from demonstrations even when both describe the same fight.

Each campaign manifest should bind the engine/mod and simulator versions, map/scenario generator, schema versions, actor/critic checkpoints, child and opponent versions, action generator, heuristic prior, reward configuration, normalizers, random seeds, trainer configuration, and deployment command settings. Hash configurations and save enough state to resume or reproduce the run.

### 2.5 Use mission-aware objectives and independent constraints

For each module, define objective outcomes first, shaping second, and diagnostic metrics separately. A possible decomposition is:

```text
reward = mission_effect_and_timing
         - mission_conditioned_resource_cost
         + validated_progress_shaping
         - avoidable_disruption
```

This is a design template, not a universal reward formula. Choose term scales from scenario consequences and inspect each term's contribution. A large number of tiny per-frame rewards must not overwhelm a decisive terminal outcome.

- Keep command legality, exclusive ownership, and explicit prohibitions in the executor. A reward penalty is insufficient to enforce them.
- `maxExpectedLossMetal` is a risk allowance, not a guarantee of realized casualties. Enforce deterministic prohibitions where possible; evaluate uncertain loss-budget violations explicitly.
- Treat an authorized sacrifice differently from an unnecessary loss. Use paired scenarios with identical troops and terrain but different mission intent and authority.
- Avoid automatic bonuses for reports, requests, spending, survival, or plan changes. Their value comes from consequences.
- If potential-based shaping is used, match its discount to the transition duration and handle terminal potential correctly. Test that cycling cannot accumulate profit.
- Keep a sparse outcome/win signal and report its performance separately from shaped return. Do not select a release solely because its training reward rose.

## 3. Training workflow for each controller

### 3.1 Unit execution and optional learned micro

**Dependency setup:** scripted squad orders, fixed mission authority, accurate engine mechanics, and a command scheduler. This component receives local orders; it does not select new strategic objectives.

**Workflow:**

1. Implement deterministic movement/target execution and a small set of event-driven reflexes. Measure failed execution, path cancellation, ability timing, and commands per unit.
2. Build isolated challenges only for shortcomings that matter: projectile avoidance, precision kiting, target execution, obstruction, ability timing, or self-destruct/wreck trade-offs. Verify each mechanic in BAR before rewarding it.
3. Where a teacher is reliable, generate observation–action examples from rules or short controlled searches. Keep future outcomes out of actor features; validate that a correction is justified by information observable before the event.
4. Compare the reflex with a small shared learned policy using short-horizon RL or evolutionary search. Keep the squad policy and encounters fixed. Condition the learned policy on unit mechanics and delegated risk.
5. Test the winner inside full squad tasks. An individually successful dodge can still break screening, obstruct allies, cancel fire, or lose the objective.

**Curriculum:** one mechanic and unit regime → varied geometry and elevation → varied weapons/cooldowns → nearby allies and collateral risk → mission-dependent authority → squad integration. Use separate action heads or policies only when action semantics or mobility differ enough to justify them.

**Training signal:** useful damage/ability effect and avoided damage relative to the assigned task; relevant costs include lost firing time, broken positioning, friendly damage, and command disruption. For self-destruct, evaluate actual delay, explosion effect, friendly exposure, and wreck outcome. Do not teach "near death means self-destruct" as an unconditional rule.

**Promotion gate:** repeatable improvement over the reflex baseline on unseen encounters and in squad missions, with measured inference, command, and pathfinding cost. If that gate fails, keep the rules and continue training higher layers.

### 3.2 Squad controller

**Dependency setup:** frozen unit execution/reflexes, a scripted platoon parent, and scripted/frozen sibling squads. Reuse architecture §8.2's observations; add no global information merely to make training easier.

**First policy boundary:** either select a coherent squad posture/formation candidate or adjust a few bounded parameters of a deterministic controller. Start with a shared role-conditioned policy where actions are compatible. Aircraft or substantially different ability regimes may need separate heads.

**Maneuver library as the action space.** Make the tactics that already work (encircle, orbit, kite, screen, pin-and-flank, hold-anchor, withdraw) named, parameterized deterministic controllers, and let the squad policy choose a maneuver and set its bounded parameters (radius, angular rate, trigger distance, side, duration). Do not have the policy emit free-form destinations. Give the maneuver choice a heuristic prior in the logits using the §2.3 form, so the untrained policy behaves like the scripted selector. The existing unit-level `template()` council in `src/learn/tactical.rs` (kite in `spacing`, gap-seeking in `encircle`) is the precedent: training starts from "fights well" and refines. These maneuvers are strong baselines. The learned policy has to beat them, not merely reproduce them (see the promotion gate).

**Squad-local polar frame.** The squad policy's observations and actions use a frame owned by the squad, not map coordinates.

- **Origin and axis.** Origin at the squad anchor (centroid, or the key unit when one exists). Polar axis along the bearing to the order's reference: the primary threat group, the assigned objective, or the platoon axis. Fix the choice of reference for the duration of an order and record it with the order. A reference such as a threat centroid that moves as units die makes the frame jitter and the policy chase its own coordinates.
- **Observations.** Every entity (enemies, siblings, allies, terrain features, the objective) is expressed as range `r` and bearing `θ` in this frame, plus radial and tangential velocity. Encode `θ` as `(cos θ, sin θ)`, not as a raw angle, so there is no wrap discontinuity. Express `r` in units of the squad's own weapon range, and speeds in range-per-second. Keep absolute unit constants as separate features for what range-normalization hides: turn radius over range, splash radius over range, minimum range, and reload time.
- **Terrain without map coordinates.** Replace absolute position with ego-polar ray features: pathable distance, clearance, slope or elevation delta, and line-of-fire quality at a fixed set of bearings and ranges, plus range to the map edge along each ray. Chokepoints and high ground still reach the policy, as geometry relative to the squad.
- **Actions.** Express them in the same frame: target radius as a multiple of weapon range, angular rate or bearing offset, and a handedness choice (clockwise, counter-clockwise, or "toward the larger gap"). Ordinary code converts these into world waypoints and does the legality, pathing, and obstruction checks. This is where the maneuver library fits naturally. An orbit is constant `r` with a nonzero angular rate. A kite is a radial excursion keyed to reload phase. An encirclement is spacing the squad's members evenly in `θ` at a fixed `r`.
- **Handedness.** Left and right are the same problem. Either canonicalize (flip so the larger gap or the friendly flank is always counter-clockwise) or train with random reflection augmentation. Verify the choice with the mirror test below.
- **Where the frame is fragile.** The angle is undefined as `r → 0`: clamp `r`, and hold the previous bearing when the reference is closer than a small floor. Rotating the frame to track a moving reference puts the reference's own velocity into the tangential term, so include the reference's velocity in the frame explicitly. Fog and stale contacts shift the origin, so carry a staleness feature for each entity.
- **Where world coordinates stay.** Pathfinding, legality, and the command scheduler are world-frame code. The policy never sees a map coordinate, so it cannot memorize a map.

Hard requirements, tested with unit tests on the observation builder and the action decoder:

1. **Rigid-motion invariance.** Translate and rotate an entire scenario (map, units, objective) by a random rigid motion. The observation vector must be identical, up to floating-point tolerance, wherever the transform does not change the terrain.
2. **Mirror consistency.** Reflect the scenario. The observation must be the reflection under the canonicalization you chose, and the decoded action must reflect to the mirror world action.
3. **Round trip.** Decode of an action followed by observation of the result must reproduce the intended `(r, θ)` offset when the path is unobstructed.

The frame does not guarantee transfer by itself. It removes one nuisance, which is where on the map a fight happens. Transfer to other weapons, terrain, and forces is still shown by held-out tests.

**Workflow:**

1. Establish deterministic role controllers that can carry out explicit subgoals: screen, provide standoff fire, hold an anchor, reconnoiter, escort, or withdraw. Implement them in the polar frame, as the maneuver library above, with their parameters exposed. Verify the frame tests before any training.
2. Measure the library before training anything. For each held-out scenario record every fixed maneuver's outcome, and form a **best-of-library oracle** (the best fixed maneuver and parameter setting per scenario, found by sweeping). This is the number to beat, and it shows where the library has headroom.
3. Generate demonstrations from these controllers where they are competent. Include recovery states produced by imperfect positioning, not only clean textbook engagements.
4. Optionally warm-start by behavioral cloning [6], or initialize the prior-plus-correction actor with the correction near zero. Validate task performance after cloning; low classification loss is not proof of competent control.
5. Run goal-conditioned PPO with fixed siblings/reflexes and mission-aware rewards. Retain the heuristic/evolutionary comparison if these controllers already exist. Run the ablation matrix below with identical observations, actions, scenarios, and budgets.
6. Rebalance scenario sampling by role and failure class. Add harder interactions after basic subgoal competence, while continuing to sample earlier cases. Include opponents built to defeat each maneuver, so that choosing a maneuver correctly matters.
7. Collect a small number of corrections on states visited by the current policy. This uses the motivation of iterative imitation/DAgger [8]; a few corrections are not a claim to implement its full procedure or guarantees.
8. Integrate with frozen platoons and test the real distribution of assignments. Retrain on those assignments if the scripted parent omitted important cases.

**Ablation matrix** (all arms share observations, maneuver library, execution, reward, scenarios, and simulated-time budget):

| Arm                                           | Question it answers                                                             |
| --------------------------------------------- | ------------------------------------------------------------------------------- |
| Prior only (scripted maneuver selector)       | What the library and its selector already achieve                               |
| Best-of-library oracle                        | The ceiling of choosing a fixed maneuver well, per scenario                     |
| Learned from scratch (no prior)               | Whether the prior helps or traps the optimizer in its basin                     |
| Prior plus learned correction                 | Whether learning adds anything beyond the baked-in tactics                      |
| Prior plus correction, prior weight annealed  | Whether the final policy still needs the prior or has absorbed it               |
| Same arms in world-frame observations/actions | Whether the polar frame improves transfer and sample efficiency, or costs skill |

The last row is the direct test of the polar-frame suggestion. Compare held-out maps and weapon sets, not training scenarios, and report sample efficiency (simulated time to reach a fixed score) as well as final score.

**Curriculum:** single-role task → terrain and weapon variation → sibling support geometry → asymmetric force matchups → fog/stale contacts → degraded rosters and changed orders. Include negative transfer tests: an elevation or spacing choice useful for one weapon can be harmful for another. Add **frame-transfer tests**: train on some maps, orientations, and weapon sets and evaluate on others, including rotated and mirrored copies of training maps and maps with different edge and choke geometry.

**Training signal:** attainment of the assigned subgoal; mission-conditioned losses and useful contribution; secondary penalties for obstruction, pointless motion, and repeated order cancellation. Judge screening by protection and delay, not by its own damage dealt. Judge recon by useful observations and mission effect, not ground traversed.

**Promotion gate:** stronger than the scripted controller **and than the best-of-library oracle** on held-out subgoals, with no serious role regression. Beating only the average maneuver, or only the single best fixed maneuver, does not count. Require evidence the gain is learned and not inherited:

- **Prior plus correction beats prior only** under matched budget, with a confidence interval across independent training seeds.
- **Measured departure from the prior.** Report how often the policy overrides the prior's choice and the outcome difference on those decisions. A policy that never departs has only reproduced the baseline.
- **Counter-maneuver cases.** The policy switches or retunes correctly against opponents built to exploit each maneuver (for example, a kiter against slower units, an orbiter against a turret). Orbit is slippery, not strong: it must also lose when it should.
- **Parameter generalization.** Radius, timing, and handedness choices are fitted to the weapon and terrain, not copied from one training setting.
- **Frame-transfer tests pass:** held-out maps, rotated and mirrored copies, and unseen weapon sets score within a stated tolerance of the in-distribution results, and the world-frame arm does not beat the polar arm on held-out sets.

The same weights must behave differently under legitimate changes of objective, risk allowance, or sibling needs. Test unseen compositions within the supported feature/action domain; truly new mechanics may require extending that domain.

### 3.3 Platoon commander

**Dependency setup:** frozen competent squads, scripted Operations, and fixed terrain/path/candidate services. Supplied combined-arms rosters avoid dependence on a trained economy.

**First policy boundary:** choose one complete coordinated plan from a bounded candidate list. A plan should specify squad roles, target/sector, approach or retreat geometry, timing/commitment, and persistence where relevant. Choosing unrelated individual squad commands is a later expansion.

**Workflow:**

1. Create mission-conditioned heuristic plan scores and record their baseline outcomes. Include `CONTINUE`, regroup/withdraw choices permitted by authority, and a report/blocker path.
2. Build paired tasks: the same local state may require preserving a valuable force, holding a position, or delaying at an authorized cost. Verify the actor actually receives the distinguishing contract fields.
3. Optionally clone clear teacher decisions, or initialize a heuristic-plus-correction actor. Use the identical starting prior in PPO and evolutionary variants.
4. Train the main selector comparison: heuristic, evolved MLP, and PPO actor, all using the same candidate plans and frozen squads. Sample stochastic actions for PPO training; evaluate the intended deployment selection rule.
5. Add incomplete intelligence, reinforcement timing, multiple routes, changing local threats, and missing counters. Train the report/objection loop using §4 rather than giving a free bonus for saying "impossible."
6. Add a few clear human action corrections between training phases. If supervised updates materially change the actor, discard obsolete PPO rollouts and collect fresh experience.
7. Integrate frozen Platoon with Operations. Compare local mission results and the effect on neighboring/global objectives; then retrain on actual operational assignments.

**Curriculum:** capture/hold/withdraw → screen-and-retreat and flank/support coordination → delay/escort/raid/recon → uncertain enemy strength → reinforcement/abort decisions → impossible or strategically costly missions. Expand the mission vocabulary only when execution and evaluation support it.

**Training signal:** externally measured contract success, timely progress, authorized cost, protected assets, and coordination. Use delay duration or survival of the force being protected for a delay mission. Do not let a favorable trade compensate automatically for abandoning the pass.

**Promotion gate:** held-out improvement over heuristic selection; goal-dependent plan changes; useful "cannot accomplish without X" reports; and a demonstrated response to revised or confirmed orders. Evaluate tactics separately from execution failures. A broken kite controller should trigger a controller fix, not a label that kiting is always a bad plan.

### 3.4 Operations commander

**Dependency setup:** frozen platoons, scripted strategic objectives/budget authority, deterministic opportunity services, and a scripted Quartermaster that offers real costs, travel/assembly delays, and refusals. Supplied capabilities must have scarcity and opportunity cost; otherwise requesting everything becomes a free solution.

**First policy boundary:** select an operational candidate: assign/revise a mission, coordinate several assignments, commit a reserve, scout, request capability, accept an offer, or continue. Begin with mission edits plus a few coordinated templates rather than an unbounded simultaneous assignment problem.

**Workflow:**

1. Establish a scripted allocation/request baseline and a reduced multi-sector campaign. Separate opportunity generation from learned valuation.
2. Supply candidates from terrain, intelligence, capability inventory, and mission state. Measure coverage of useful high-ground, recon, and special-operation choices.
3. Warm-start from sensible scripted allocations where available. A short-horizon search can label candidates when affordable, but its horizon and model assumptions must be recorded.
4. Train goal-conditioned candidate selection against frozen platoons and varied enemy strategies. Include unresolved uncertainty; do not let training-only knowledge choose suspiciously perfect scouting targets.
5. Train responses to subordinate reports: reinforce, revise objective/deadline, redirect another force, authorize sacrifice within authority, or escalate to Strategy.
6. Run capability-request scenarios before connecting live production. Vary available offers, lead times, resource limits, and alternative conventional solutions.
7. Replace scripted supply with frozen Quartermaster and scripted authority. Retrain Operations against actual ETA errors, assembly limits, delivery losses, and unavailable tech.

**Curriculum:** one objective and reserve → competing sectors → time-sensitive coordination → meaningful recon → high-ground opportunity cost → missing capabilities/spec-ops → objections and shocks → live delivery. Include cases where the attractive ridge should be ignored and where a conventional attack or deferral beats spec-ops.

**Training signal:** valuable regional effects and their timing, avoided front collapse, useful employment of reserves, and resource costs attributable to operational commitments. Candidate request value should come from delivered effects and downstream outcomes, not from the count of requests accepted. Information gain alone is insufficient if it scouts irrelevant areas.

**Promotion gate:** improved campaign outcomes with the same platoons/supply; successful coordination; calibrated use of reserves and requests; and productive adaptation to objections. Ablate reports and capability requests to establish that these interfaces contribute.

**Required demonstration:** from a held-out situation, recognize an installation or terrain opportunity, identify missing capability, compare available responses, submit a deadline-aware request, and employ the supplied force. Repeat with costs changed so a different response becomes preferable.

### 3.5 Quartermaster

**Dependency setup:** verified economy/build mechanics, capability catalog, budget arbitration, constructor/factory ownership, travel/placement services, and scripted request streams. A simulator used here must represent streaming resource use, shared build power, prerequisites, and delivery delay sufficiently well for scheduling decisions.

**First policy boundary:** keep feasibility, reservations, accounting, and execution deterministic. Let a learned component rank feasible composition/schedule candidates, set bounded priority weights, or predict uncertain duration. Do not start with unconstrained neural build queues.

**Workflow:**

1. Implement a forecast and rolling scheduler that produces feasible offers, reserves authorized resources without double spending, and revises ETA/blockers after shocks.
2. Benchmark simple rules against stronger planning/search on small problems. A short-horizon or approximate solver is acceptable; label its solutions as planner outputs, not proven optima.
3. Generate supervised examples for completion time, delivery time, bottleneck prediction, and ranking available schedules. Separate model error from uncertainty caused by future shocks or changed priorities.
4. Train any useful forecast model independently. Deploy it only after interval/calibration checks; keep a conservative fallback when outside the training domain.
5. If the planner still leaves substantial decision value, compare learned candidate ranking/priorities with hand-tuned priorities. Use supervised planner imitation first where useful; use RL for the trade-offs that remain sequential and uncertain. Evolution can tune a small priority/threshold vector without evolving the scheduler itself.
6. Train against diverse request streams, deadlines, income shocks, damaged factories, builder travel, threatened construction sites, and emergency reallocations. Add tactical risk through frozen Operations/scenario services.
7. Validate real compositions in mission rollouts. A capability-vector match does not prove a force can accomplish the requested effect. Feed failures back into the catalog, offer generator, or estimator as appropriate.
8. Couple to Operations with fixed strategic envelopes; then test both against a changing scripted Strategy before joint adaptation.

**Curriculum:** feasible single request → competing deadlines → metal/energy/build-power bottlenecks → tech prerequisites and assembly/travel → income shocks → raids/emergency defense → full request/offer/revision loop.

**Training signal:** capability effects delivered at the required place and useful time, plus longer-term economic value under the assigned priorities. Count costs through prerequisites, production, assembly, and delivery. Late units can have much less value than on-time units. Avoid double counting a growing economy and all future output as independent bonuses.

**Promotion gate:** equal or better on-time useful delivery than the strong scripted scheduler across varied demand, with correct budget ownership, honest infeasibility handling, and calibrated ETAs. A model that reduces idle time while delaying critical AA does not pass.

**Recommended default:** a capable planner may remain the released implementation. Learned forecasting can be valuable even when no Quartermaster policy network is needed.

### 3.6 Strategic Director

**Dependency setup:** frozen, independently competent Operations and Quartermaster; fixed lower tactical bundle; legal global summaries; and a diversified opponent pool. Strategy should not be trained to compensate for a still-broken production loop.

**First policy boundary:** select a manageable package of macro commitments or edit one commitment: broad objectives, spending envelopes, tech investment, defensive risk posture, or a major requisition. Deterministic arbitration must convert those choices into consistent budgets.

**Workflow:**

1. Establish a full-game scripted macro baseline and verify that its authority reaches actual production and missions.
2. Generate macro demonstrations from that teacher. Human replay commands require explicit interpretation into this action vocabulary; do not assume build commands already label strategic intent.
3. Optionally warm-start the actor, then train with frozen lower layers. Start with fewer strategic choices and broaden only after those choices affect outcomes reliably.
4. Train on openings and diverse mid/late-game snapshots. Track snapshot provenance and evaluate full games separately: success from curated favorable resets does not prove sound openings or long-term strategy.
5. Use win/loss or another explicitly defined full-game result as the principal outcome. Test shaping against actual match performance. Choose the discount horizon to retain relevant long-term consequences.
6. Add opponents specializing in economic pressure, air transitions, artillery, stealth, or late-game threats. Rare crises need deliberate scenario sampling rather than reliance on spontaneous occurrence.
7. Include competing requests and honest negative offers. Teach Strategy when to fund an expensive operational plan, defer it, choose an alternative, or accept a local loss for global gain.
8. Compare the learned director with the identical lower bundle under scripted Strategy. Only after stable improvement should selected lower layers adapt.

**Curriculum:** constrained macro choices → broader openings/economies → hidden enemy plans → tech/counter transitions → competing operational investment → rare late-game crises → complete games and broader maps.

**Training signal:** match outcome and validated strategic progress. Army size, resource stock, and map control are diagnostics or carefully tested shaping, not sufficient goals by themselves. Penalizing match duration indiscriminately can teach premature attacks or surrender.

**Promotion gate:** better match outcomes across held-out opponents/maps with uncertainty reported, no extra observation privileges, reasonable catastrophic-risk handling, and preserved tactical/economic competence. A rise in economic score alone does not pass.

**Fallback if full-game RL is too expensive:** retain scripted Strategy with learned Operations/Platoon and a capable Quartermaster. This remains a useful integrated bot and a platform for further learning.

## 4. Train the command/report loop explicitly

Reports should combine measured facts, independently checked blockers, optional forecasts, and a proposed response. Avoid making the whole report an unconstrained string emitted by the policy.

### 4.1 Separate four different learned functions

| Function                      | Question answered                                         | Training target                                             | Runtime role                             |
| ----------------------------- | --------------------------------------------------------- | ----------------------------------------------------------- | ---------------------------------------- |
| Actor                         | Which available action should I choose?                   | PPO/evolutionary return or a supervised preferred action    | Selects plans/commitments                |
| PPO critic                    | What return should this policy expect from here?          | Returns under the configured reward and continuation policy | Primarily training machinery             |
| Outcome/feasibility predictor | Can this mission/plan succeed, with what loss and ETA?    | Measured success, loss, duration, and constraints           | Can support reports and parent decisions |
| Human-feedback evaluator      | Would the reviewer prefer this behavior for this mission? | Ratings or pairwise preferences                             | Optional training/evaluation aid         |

These functions may share some encodings later, but their targets are different. A high critic value is not automatically a calibrated success probability. An evaluator is not another command tier.

### 4.2 Build forecast training data from controlled execution

Start with factual reports and conservative heuristic forecasts. If learned prediction is justified:

1. Log the actor-legal observation, mission and version, proposed plan, current force, deadlines/authority, and controller bundle at prediction time.
2. Run the plan under a specified continuation policy; measure outcome, mission-conditioned loss, and completion/delivery time. Label repeated stochastic runs separately when available.
3. Include difficult and failing cases. Data collected only from plans a strong policy accepts will not cover everything the parent might ask about.
4. Mark cancellations, mission revisions, censoring, and execution failures. An aborted mission is not evidence of how long successful completion would have taken. Do not fit ETA only to successful easy missions and silently apply it everywhere.
5. Fit small supervised models first. Success classification, loss prediction, and conditional time estimates may need separate heads. Use scenario-grouped validation and compare against simple heuristics/base rates.
6. Check probability calibration and Brier/log loss where probabilities are reported; inspect ETA interval coverage and bias by mission, map, role, and force strength. If data are insufficient, report coarse confidence or `UNKNOWN`.
7. Revalidate after child policies, mechanics, or action semantics change. State explicitly which continuation policy a forecast assumes.

Authoritative outcomes can supply training labels without being actor inputs. However, a teacher must not demand that an actor diagnose a **currently hidden** enemy or blocker with certainty. Separate physical impossibility, evidence-supported risk, and uncertainty that requires scouting.

### 4.3 Use matched objection scenarios

Train both sides of the exchange with a small matrix:

| Situation                            | Expected subordinate behavior                                | Expected parent behavior                                                  |
| ------------------------------------ | ------------------------------------------------------------ | ------------------------------------------------------------------------- |
| Feasible routine mission             | Execute, report meaningful progress                          | Allow execution without needless reassignment                             |
| Known hard blocker                   | Report the specific blocker; offer an available alternative  | Revise, supply means, or choose another objective                         |
| Local cost exceeds current authority | Report estimated cost and request revision/authorization     | Compare global value; revise or explicitly authorize within its authority |
| Authorized costly delay              | Report risk honestly and pursue the approved delay           | Preserve intent; assess the protected force/global effect                 |
| Uncertain enemy strength             | Express uncertainty; request/recommend recon when useful     | Weigh information value against time and scout cost                       |
| New local opportunity                | Propose it without silently abandoning a critical assignment | Accept, defer, or reject using wider priorities                           |

The same battlefield state should appear with different legitimate contracts. This tests whether the bot learned intent rather than a fixed self-preservation rule.

Keep report generation factual where possible. A small decision head may choose to raise a concern, request a capability, or propose an alternative. The parent then makes the commitment decision; do not let the child rewrite its own success condition.

**Do not reward objection itself.** Measure whether reports arrive early enough to change outcomes, identify useful causes, and improve the parent's decisions. Track missed warnings and false alarms, but avoid punishing every conservative report so strongly that the bot hides uncertainty. Keep the costs and original outcome of revised/canceled missions visible; otherwise refusing hard assignments can become an easy reward exploit.

### 4.4 Test feedback as an integrated capability

Compare matched bundles with full reports, factual status only, and no upward reports. Separately test a parent that receives reports but ignores them. This distinguishes a useful predictor from a useful command loop.

Measure mission/global outcome, avoidable losses, response latency, unresolved blockers, repeat objections after an explicit answer, and command churn. Test stale reports, delayed reinforcements, rejected requests, and mission-version mismatches. A negative report should lead to a bounded lifecycle transition, not an endless negotiation loop.

## 5. Human review and supervised learning

### 5.1 Use a few reviews per campaign for high-value corrections

Begin with a lightweight review packet rather than a learned reward model. Each campaign should propose a small mixed queue: ordinary play, clear failures, surprising wins, and suspicious behavior such as inactivity or repeated plan changes. Keep an unbiased random sample as part of the queue; reviewing only spectacular failures distorts the dataset.

For each chosen engagement, show:

- The mission, intent, authority, deadline, roster, policy bundle, and relevant neighboring assignments.
- A clip/trace with enough lead-in and aftermath to judge the action's purpose.
- The observation and candidate plans available at the decision, separated from any omniscient post-match view.
- Automatically computed outcomes and costs.

Ask for one or two important annotations, not labels on every frame:

```yaml
scenario_id: ridge_delay_variant_17
mission_id: hold_pass_04
mission_version: 2
decision_id: platoon_03_decision_81
judgment: clear_mistake        # questionable / uncertain are also valid
effectiveness: 2              # optional, anchored 1–5 scale
efficiency: 3
sanity: 2
reason_tags: [unsupported_pursuit, abandoned_assignment]
preferred_candidate_id: screen_withdraw_v2  # optional; must have been available
comment: Raiders should screen the ranged squad until it clears the pass.
```

Include the actor observation/candidate artifact and schema versions in the actual record. The example above is an annotation shape, not a complete storage specification.

Define the dimensions consistently:

| Dimension     | Review question                                                                        | Automatic information to accompany it                                  |
| ------------- | -------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| Effectiveness | Did it achieve the assigned effect at the useful time?                                 | Explicit success, progress, deadline, protected objective              |
| Efficiency    | Was the time/resource expenditure justified by that effect?                            | Losses, delivery/mission time, resource cost, participation            |
| Sanity        | Were decisions coherent with its information, authority, and coordination obligations? | Churn, unsupported separation, stale information, execution exceptions |

Allow "unusual but effective," "insufficient information," and low-confidence feedback. Broad grades are useful diagnostics; they do not identify a correct action for every decision in the match.

### 5.2 Route feedback before training on it

| Review finding                                                | First destination                                        |
| ------------------------------------------------------------- | -------------------------------------------------------- |
| Good plan selected; movement/ability execution failed         | Reflex/controller/scheduler fix plus regression scenario |
| No adequate plan was offered                                  | Candidate generator or action vocabulary                 |
| Decision required information absent from the observation     | Observation/belief/interface investigation               |
| Bad choice among adequate represented plans                   | Supervised actor correction                              |
| Parent assigned a locally impossible or globally poor mission | Parent policy/forecast/request loop                      |
| Bot exploited a scoring loophole                              | Reward or outcome-definition revision                    |
| Reviewer dislikes the style but outcome trade-off is unclear  | Diagnostic tag or comparison request; no forced target   |

A preferred action that was missing from the candidate set is a proposal to extend that set. It is not a valid class label for the old policy interface.

### 5.3 Add supervised corrections in controlled phases

1. Accumulate trusted observation–candidate–preferred-action examples across campaigns. Group splits by whole encounter/scenario family, not nearby frames.
2. Train with behavioral cloning on the available valid actions, weighting uncertain or ambiguous labels less strongly. Support multiple acceptable actions when appropriate rather than inventing a uniquely correct answer.
3. Mix new examples with earlier corrections and behavior worth preserving. Check that fixing one repeated mistake does not erase another skill.
4. Evaluate actual mission outcomes and held-out corrections before resuming automated optimization.
5. Collect fresh PPO rollouts after significant actor changes. An evolved candidate may also receive supervised refinement, but compare and retain it based on reevaluated fitness.

Combining a PPO loss with a weighted imitation loss inside training is a valid later design, but is custom work. Start with explicit alternating phases so the contribution of supervision is observable. Ensure the cloning distribution includes the same masks and heuristic prior as the deployed actor; do not assume every off-the-shelf BC helper handles a custom masked policy unchanged.

### 5.4 Consider a learned evaluator only after labels accumulate

Prefer paired alternatives from the same starting situation when practical: A / B / roughly equal / cannot judge. If state restoration works, branch a decision into alternative plans with matched continuation settings and enough repeated seeds to avoid presenting one lucky outcome as proof.

Train a small mission-conditioned model on structured behavior features before considering raw replay video. Include the information needed to recognize the complaint: objective progress, contribution, exposure, support geometry, timing, losses, and plan changes. Keep subjective dimensions separate unless there is a justified rule for combining them.

Before its predictions influence training, require:

- Better held-out agreement than simple outcome/feature baselines, with label uncertainty and class balance reported.
- Splits by complete fights/scenario families; reserve independent reviews for evaluating the evaluator.
- Checks for shortcuts such as clip length or scenario identity. Different mission horizons and early termination can leak outcomes into preference learning.
- Inspection of disagreements, uncertain regions, and behavior the optimizer discovers to exploit its score.
- A measured downstream gain when its influence is added; objective outcomes and contract adherence remain independent checks.

Freeze the evaluator during a PPO collection/update campaign or evolutionary comparison batch. After changing its scoring, start a new reward version and reevaluate comparisons. Do not silently mix candidates scored under different evaluators.

The `imitation` library supplies behavioral cloning and preference-learning components, including an SB3 PPO example [6–7]. Its example uses synthetic preferences; BAR clip selection, human annotation, custom candidate/mask adapters, and validation still require project work. A handful of reviews per campaign is useful project steering, not evidence that a general tactical evaluator is already reliable.

## 6. Compare PPO, evolution, and hybrids without changing the question

The terminology matters: an MLP is the policy representation; evolution and PPO are ways to improve it; backpropagation computes gradients of a defined loss. Your evolutionary policy search can solve a reward-driven control problem without gradient updates. The simulator need not be differentiable for policy-gradient PPO [5].

### 6.1 First controlled selector experiment

Use Platoon as the first comparison unless the existing simulator only supports another well-defined boundary.

| Variant                                  | Purpose                                                                            |
| ---------------------------------------- | ---------------------------------------------------------------------------------- |
| Fixed heuristic selector                 | Establish what the action structure and deterministic execution already accomplish |
| MLP trained through evolution            | Measure the benefit of policy learning with the current approach                   |
| Compatible MLP actor trained through PPO | Measure gradient-based policy learning under matched task conditions               |
| Optional warm-started versions           | Measure whether a shared heuristic/teacher initialization helps either method      |

Hold missions, candidate generation, observations, priors, squad execution, reward/outcome definitions, training distribution, and validation scenarios constant. PPO can have its training-only critic; count its cost rather than requiring evolution to have one.

Compare both **simulation work consumed** and **wall-clock/hardware cost**. Count simulated game seconds or engine frames as well as decisions and complete encounters; battle count alone favors methods that end encounters early. Include evaluation, population trials, teacher generation, and hyperparameter search in the cost report, distinguished from final training cost.

Use matched validation encounters and multiple independent training runs as resources permit. Start with a modest seed pilot to estimate variance, then choose additional repetitions from the effect size that would matter. A single lucky checkpoint is insufficient evidence of algorithm superiority. Match deployment behavior too: PPO may train stochastically but deploy with `argmax`; test that deployed form.

### 6.2 Integration requirement for PPO

Evolution can operate through `candidate weights -> whole-encounter fitness`. PPO needs trajectories: decisions, sampled action probabilities, rewards, boundaries, and value estimates. Wrapping a final scalar score in Gymnasium without exposing the decisions is not sufficient for the proposed sequential selector.

PPO already alternates rollout collection and gradient updates [5]. There is no requirement to backpropagate every engine frame, after every action, or during deployed play.

### 6.3 Add hybrids only to solve a demonstrated problem

Recommended order:

1. **Shared teacher/heuristic start, then one optimizer.** Establish clean PPO and evolutionary baselines.
2. **Evolution followed by PPO refinement.** Use compatible observations/actions, a sensible stochastic distribution, an initialized critic, and fresh rollouts. Compare before/after and against the same extra budget spent on evolution.
3. **Gradients for the actor; evolution for a small outer parameter set.** Examples include controller priorities or bounded spacing settings. Changing them changes the child behavior, so retrain/evaluate against a versioned environment.
4. **Population members with occasional gradient refinement.** Attempt only if diversity or local refinement has a measured purpose. Track actor, critic, optimizer, normalizer, and rollout ownership per member.

After mutation, policy replacement, significant supervised refinement, or substantial child/environment changes, collect fresh PPO experience and review which critic/optimizer state remains meaningful. Do not repeatedly apply PPO to a shared archive of arbitrary old battles. Supervised datasets have different reuse rules.

EvoTorch is an optional replacement for maintaining a custom evolutionary training loop: it supports neural-network evaluation through scalar fitness [9]. Retain a working custom trainer if replacing it would delay the experiment without a concrete benefit.

## 7. Integrate and adapt the hierarchy in stages

### 7.1 Training order mapped to the architecture milestones

| Architecture stage            | Training deliverable                                                                                    | Gate before moving on                                                        |
| ----------------------------- | ------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| M0: mechanics/runner          | Inventory existing trainer; benchmark simulation; compare relevant engine mechanics and tactic rankings | Reproducible, legal observation/action/replay loop                           |
| M1: contracts/instrumentation | Mission lifecycle, reports, ownership, request/offer, reward and trace schemas                          | Scripted objection, revision/override, cancellation, and completion work     |
| M2: deterministic baselines   | Usable reflexes, squads, platoon plans, sector allocator, production scheduler                          | Baseline outcome and runtime measurements with zero learning                 |
| M3: squad competence          | Train squads only where scripts leave a meaningful gap; optional micro experiments                      | Lower controllers fulfill varied goals reliably                              |
| M4: platoon selection         | Heuristic/evolution/PPO comparison; reports; initial corrections                                        | Better held-out missions and appropriate intent-dependent behavior           |
| M5: Operations vertical slice | Multi-sector training with prebuilt forces and costed scripted supply                                   | Useful objectives, coordination, requests, and responses to objections       |
| M6: real Quartermaster        | Forecast/scheduler validation, optional learned priorities, live delivery loop                          | Requested effect survives authorization → production → delivery → employment |
| M7: Strategy                  | Full-game teacher/baseline, then measured strategic learning                                            | Better match results with a fixed lower bundle                               |
| M8: coordinated adaptation    | Selective unfreezing, opponent pool, rare-event replay, team/scaling extensions                         | Integration gains without unacceptable regressions/runtime cost              |

Quartermaster development can proceed alongside tactical learning after contracts stabilize. If the current squads and simulator are already competent, begin M4 learning promptly while closing gaps in instrumentation. Do not rebuild existing working pieces merely to follow the row order.

### 7.2 Freeze first, then alternate adaptation

For every new parent/child pairing:

1. Integrate with both frozen and inspect real request/order/report distributions.
2. Repair interface mismatches and missing candidates before training around them.
3. Train the parent against a frozen competent child bundle.
4. Freeze the improved parent and adapt the child to the resulting assignment distribution if needed, preserving earlier tasks in training.
5. Evaluate the combined bundle and each affected module against its prior baseline.
6. Promote the bundle together or roll it back together.

Do not update every layer on every rollout initially. Child changes make the parent's environment change; parent changes alter the child's goal distribution. Shared encoders or normalizers also count as changes even if one policy head is nominally frozen.

If independent cooperating squads/platoons eventually learn simultaneously, investigate parameter sharing and centralized training with explicit multi-agent support. A single platoon policy controlling many game units is still one learning agent. Do not introduce a multi-agent framework merely because the army is large.

### 7.3 Train rare events without forgetting routine play

Maintain a mix of ordinary episodes, targeted failure scenarios, and rare-crisis snapshots. Track results separately by source; a hand-picked curriculum win rate is not the expected live-game win rate. Retain earlier scenario families during fine-tuning and test both specialist gains and routine regressions.

Use frozen past checkpoints, varied scripts, and targeted exploiters as an opponent pool. Keep final test opponents separate from the adaptive training league. Expand to allied/team play after ownership, budgets, and the core hierarchy are stable.

### 7.4 Assign credit at the layer making the decision

Train each actor against its own decision sequence and externally measured objective. A squad receives feedback about its assigned contribution; the platoon about the combined mission; Operations about the campaign effects of assignments and requests; Strategy about the match. The same event can inform several objectives, but do not recursively add every child's shaped return into the parent's reward. That makes arbitrary shaping and the number of children change strategic value.

Use mission, request, offer, and objective IDs to connect delayed effects to earlier commitments. A request's cost begins before delivery; its benefit may arrive after several subsequent decisions. Set rollout/return horizons accordingly. Mission cancellations and revisions must not erase sunk costs or unresolved effects.

Initially, frozen children make parent credit assignment more tractable. When attribution is unclear, run matched scenarios changing one assignment/controller while holding others fixed where practical. Keep these diagnostic comparisons distinct from claims of perfect causal attribution. Simultaneous multi-layer learning with centralized critics is a later experiment, not required for this workflow.

## 8. Campaign operation and promotion

### 8.1 A repeatable campaign cycle

1. **Specify:** choose one module/bundle, the weakness being addressed, primary outcome, constraints, training budget, and the baseline it must beat.
2. **Freeze:** record schemas, reward, candidates, priors, child/opponent bundles, and normalizers. Choose grouped training/validation/test partitions.
3. **Check:** run a short smoke rollout; inspect observations, sampled action probabilities/masks, reward terms, boundaries, and command execution.
4. **Train:** collect fresh rollouts or evolutionary evaluations, run scheduled updates, and checkpoint with costs and metrics.
5. **Validate:** use a fixed validation distribution for checkpoint selection; inspect failures and a small mixed human-review queue.
6. **Route feedback:** fix software/data/candidate problems at their owner; add trusted corrections or training scenarios under a new version where needed.
7. **Evaluate:** compare the chosen candidate against its baseline in held-out scenarios, integration tests, and engine runs.
8. **Promote or retain baseline:** save the complete controller bundle and its evidence. A failed experiment may still yield useful diagnostics; it need not replace a stronger script.

If an evaluation failure is used to alter the design, that case becomes a regression/development case. Refresh the untouched final test set periodically rather than repeatedly tuning against it while calling it held out.

### 8.2 Promotion evidence

Use architecture §10's regression scenarios; add the matched-goal, request-cost, and report ablations specified here. The release report should include:

- Primary mission/campaign/match outcomes by scenario family and by training run, with appropriate confidence intervals or paired uncertainty estimates.
- Material regressions, catastrophic failures, objective abandonment, and worst-case/low-percentile behavior. Do not hide a failed mission type inside an overall average.
- Observed hard-constraint, ownership, and legality violations; executor invariants must remain enforced regardless of reward.
- Report calibration, useful warning lead time, requests that led to valuable effects, and production/delivery accuracy.
- Command churn, pathfinding work, inference latency, simulation frame-time impact, throughput, and total training cost.
- Simulator-versus-engine differences and performance on unseen maps/compositions/opponents within the supported domain.

Define practical improvement and allowed regression thresholds before the campaign, using baseline variance and game consequences. There is no universal episode count or success percentage that proves readiness for every layer.

### 8.3 Deployment parity and rollback

Export the actor together with feature definitions, normalization state, candidate generation/prior versions, action masks, and command/persistence settings. Freeze running statistics for evaluation and deployment. Replay recorded legal observations through the training and deployed implementations and compare candidate scores, masks, and selected actions within a specified tolerance.

Use the real deployment cadence and deterministic/stochastic selection rule in acceptance runs. The critic, human-feedback evaluator, and optimizer need not ship unless a defined runtime feature uses them. A feasibility model does need to ship if reports depend on it.

Retain the previous complete bundle and a scripted fallback. Rollback must restore compatible schemas, predictors, and normalization as well as actor weights.

## 9. Framework choices and implementation boundaries

| Need                                       | Proposed tool                             | Boundary / caveat                                                                                                                  |
| ------------------------------------------ | ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| Environment adapter                        | Gymnasium [1–2]                           | Own the simulator/engine bridge and correct task boundaries; SB3 vector environments have wrapper conventions that must be checked |
| Neural networks and supervised losses      | PyTorch through the chosen trainer        | Own feature/model design; preserve actor observation legality                                                                      |
| First policy-gradient trainer              | Stable-Baselines3 PPO [3]                 | Use established rollout/loss/update machinery; custom policies are still project code                                              |
| Discrete candidate legality                | SB3-Contrib MaskablePPO [4]               | Use mask-aware training and evaluation; subprocess masks must be supplied by the environment                                       |
| Behavioral cloning / preference components | `imitation` [6–7]                         | Supply BAR labels, review workflow, and adapters; not a ready-made replay reviewer                                                 |
| Evolutionary policy search                 | Existing trainer, optionally EvoTorch [9] | Reuse the same task definition and count population evaluation costs                                                               |

Keep the initial actor feed-forward with mission state, trends, and short history if sufficient. Stock MaskablePPO does not support recurrent policies or continuous `Box` actions; standard SB3 PPO does not offer dictionary actions [3–4]. Masking plus recurrence, arbitrary structured actions, mixed continuous/discrete choices, variable-duration PPO, or a privileged centralized critic are **additional integration work**, not capabilities obtained by combining checkboxes.

Avoid package-version promises from an evolving documentation URL. Pin a compatible stable environment after verifying the needed API behavior. Run one short end-to-end collection/update/save/load/evaluate cycle before committing to the stack.

The adapter, observations, candidate plans, task metrics, and review tooling are good targets for AI-assisted development. Keep training configurations explicit and inspectable; use the established trainer for its supported algorithm rather than asking an agent to invent PPO's update loop.

## 10. Concrete next-agent assignment

Proceed in this order:

1. **Inspect the existing code.** Identify how weights change, whether tactical decisions can be stepped/logged, simulator fidelity, existing controllers, language boundaries, and measured throughput. Do not call the trainer PPO or evolution solely from its function names.
2. **Write the minimal adapter/contract specification.** Resolve observation legality, mission encoding, candidate features/masks, persistence, reward/boundary semantics, and trace schema. Preserve the architecture's ownership boundaries.
3. **Make the baseline observable.** Reproduce at least a few clear mission successes/failures, a known blocker, an objection/revision, and an authorized costly delay without learning.
4. **Run the smallest discriminating pilot.** Prefer platoon heuristic/evolution/PPO with fixed squads. If the bridge exposes only battle fitness, budget the trajectory adapter explicitly; continue the evolutionary baseline while building it.
5. **Return measurements and a recommendation.** Report task performance, training variance, simulation work, wall time/hardware, command/runtime cost, and specific failure modes. Choose the next method from those results.
6. **Expand to Operations and the request loop.** Demonstrate terrain/spec-ops/capability choices under realistic costs before full economic or strategic RL.

Required handoff artifacts: a versioned campaign configuration, controller-bundle manifest, scenario/split manifest, observation/action/report schemas, reward ledger, representative decision traces, review/correction dataset format, comparison report, and deployment/rollback procedure.

Decisions still requiring code or engine evidence include the simulator bridge, reliable state restoration, supported mission/action coverage, inference/export format, actual training throughput, useful network encoders, and whether any learned micro, scheduler priorities, or reward evaluator earn their cost. These are explicit experimental decisions, not missing invitations to train every layer at once.

## 11. Primary references

Framework documentation was checked while preparing this proposal. These sources establish interfaces and methods; the BAR-specific workflow and promotion criteria above are proposed applications.

1. **Gymnasium environment contract:** [Env API](https://gymnasium.farama.org/api/env/).
2. **Episode boundary semantics:** [Farama: Terminated/Truncated Step API](https://farama.org/Gymnasium-Terminated-Truncated-Step-API).
3. **PPO implementation and supported spaces:** [Stable-Baselines3 PPO](https://stable-baselines3.readthedocs.io/en/master/modules/ppo.html).
4. **Action masking, evaluation requirements, and limitations:** [SB3-Contrib MaskablePPO](https://sb3-contrib.readthedocs.io/en/master/modules/ppo_mask.html).
5. **PPO algorithm:** Schulman et al., [Proximal Policy Optimization Algorithms](https://arxiv.org/abs/1707.06347).
6. **Supervised policy training:** [`imitation`: Behavioral Cloning](https://imitation.readthedocs.io/en/stable/algorithms/bc.html).
7. **Preference reward learning and PPO example:** [`imitation`: Preference Comparisons](https://imitation.readthedocs.io/en/latest/algorithms/preference_comparisons.html).
8. **Iterative imitation and distribution shift:** Ross, Gordon, and Bagnell, [A Reduction of Imitation Learning and Structured Prediction to No-Regret Online Learning](https://arxiv.org/abs/1011.0686).
9. **Scalar-fitness neural policy search:** [EvoTorch: Neuroevolution](https://docs.evotorch.ai/latest/user_guide/neuroevolution/).
