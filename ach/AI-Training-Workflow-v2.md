# Beyond All Reason AI: Training Workflow — V2

**Status:** Proposed training and evaluation workflow; an implementation handoff, not a claim of verified BAR mechanics or measured learning gains.  
**Revision:** 2026-10-19 — matches the formation-first architecture revision.  
**Architecture companion:** [Hierarchical-Brains-v2.md](Hierarchical-Brains-v2.md). Section references below refer to its numbered headings.  
**Scope:** All six responsibility boundaries, the command/report loop, shared training infrastructure, periodic human review, and promotion into an integrated bot.

The architecture defines who decides, which information they may use, and how they communicate. This companion defines how to develop, train, compare, and promote those components. Read the architecture's §§2–6 for ownership, interfaces, observations, and action semantics rather than recreating them from this document.

**Revision basis:** The supplied V2 architecture and the prior training discussion available in this conversation. The original training companion's full text was not supplied, so the change summary below describes substantive design changes rather than a verified line-by-line diff.

## 1. What changes in V2

The central change is to make **squad formation selection a small, optional learning problem above a strong deterministic execution library**. Squad learning is no longer a prerequisite for learning the higher hierarchy.

| Architecture requirement                                          | Matching training decision                                                                                                                                                       |
| ----------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Fixed `CHARGE`, `HOLD`, `ORBIT`, `ENCIRCLE`, and `KITE` executors | Reproduce and version the existing formulas, presets, targeting rules, and reflexes before training. Freeze them during selector comparisons.                                    |
| Learn which mode to use, retain, or change                        | Begin with a discrete selector. Exclude per-unit waypoints, learned spacing, orbit direction, and continuous movement vectors from its initial action space.                     |
| Formations retain execution state                                 | Test idempotent reselection, stable slots, orbit direction, dwell rules, interruption, and recovery. Include mode age and execution status in the observation.                   |
| Missions and movement modes have different meanings               | Use paired scenarios with identical forces and different goals. A `HOLD` mission may require kiting or orbiting inside its allowed region.                                       |
| Platoon owns combined-arms coordination                           | Train squad choices within assigned sectors, support relationships, timing, and authority. Train platoons with competent frozen heuristic or validated learned squads.           |
| Learning must justify its cost                                    | Compare fixed modes, a strong mission-aware heuristic selector, evolutionary selection, and PPO selection with identical execution. Retain the heuristic when it is adequate.    |
| Five combat modes do not cover every role or lifecycle state      | Keep transit, regroup, withdrawal, no-target handling, and support-role behavior scripted and observable. Add specialist behavior only for demonstrated gaps.                    |
| Learned geometry and per-unit micro are optional extensions       | Require a separate experiment and promotion gate for preset selection, parameter refinement, composite modes, and learned execution. None blocks Platoon or Operations training. |

Retain the earlier training choices: mission-conditioned policies; fixed scripted baselines; PPO versus evolution as an experiment; supervised warm-starts and corrections; lightweight human review; calibrated feasibility/ETA prediction; and frozen-component integration before joint adaptation.

### 1.1 Assessment of the revised architecture

The narrower squad boundary is a useful starting point because it isolates **judgment** from **execution** and makes failures easier to attribute. Its value still depends on reproducing the reported strong formation results and finding enough selection headroom to justify a learned selector.

Four details need explicit training contracts:

1. **Persistence includes the assignment identity.** Keeping `KITE` while receiving a different target or sector is not necessarily continuing the same maneuver. Mission, assignment, target, and preset revisions need deliberate state-transition rules.
2. **Selection, accepted mode, and actual execution are separate facts.** A scheduler may defer a switch, or an emergency may temporarily interrupt it. Log these transitions rather than reporting only the requested label.
3. **Report quality is a prediction problem as well as a control problem.** A truthful objection should not be rewarded merely because an objection was emitted. Measure whether it predicts a real blocker and whether the parent's response improves outcomes.
4. **The initial five-mode experiment should have a stopping rule.** If a credible comparison finds little useful gain, freeze the heuristic and spend the next training budget on platoon coordination and operational decisions.

These refine the workflow without changing decision ownership or requiring another runtime brain.

## 2. Training structure and minimum infrastructure

### 2.1 Keep three kinds of learning distinct

| Learning task                 | Example target                                                    | Data and method                                                                              | Deployment role                                               |
| ----------------------------- | ----------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| **Action selection**          | Choose a formation, platoon plan, operation, or spending priority | Heuristic demonstrations, clear human corrections, PPO trajectories, or evolutionary fitness | Acting policy, if it passes its gate                          |
| **Outcome prediction**        | Mission success, remaining hold time, losses, delivery ETA        | Versioned outcome records; supervised classification/regression and calibration              | Supports reports and planning; may remain heuristic initially |
| **Human judgment prediction** | Which of two executions better serves the stated objective        | Explicit comparisons or ratings; optional preference/evaluator model                         | Training assistance after validation; not a new command tier  |

The PPO critic estimates future training return under the collecting policy. It is not automatically a calibrated success predictor, delivery forecaster, or model of human judgment. Keep those meanings separate even if a later implementation shares encoders.

### 2.2 Start with an instrumented deterministic system

Before optimizing weights, provide:

- A reproducible scenario runner and an observation-legal engine/simulator bridge.
- The versioned order/report/capability contracts from architecture §3, including acknowledgments, cancellation, revisions, unit ownership, and budget ownership.
- Stable squad executors, role scripts, a mission-aware selector, and a competent scripted platoon parent.
- A scheduler that records accepted commands, deferred switches, coalesced orders, interrupts, and execution failures.
- Named task metrics independent of the scalar reward or fitness.
- A way to restore small scenarios or replay from their initial conditions; arbitrary snapshot branching is optional until supported reliably.

Validate the custom simulator against actual BAR for the mechanics used by the first experiments: effective range, projectile travel, firing obstruction, movement/pathing, targeting, elevation, and order semantics. Extend fidelity checks as cloaking, economy, wrecks, abilities, shields, and nuclear defense enter the curriculum. Version both the game data and simulator assumptions.

A deterministic controller is closed-loop code. Its use of pathfinding, accessible-arc searches, and per-unit corrections still has runtime cost and can still contain bugs.

### 2.3 Version one experiment as a complete bundle

An experiment manifest should identify:

| Area                | Required provenance                                                                                                       |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| World and mechanics | Engine/game/map/unit-data versions; custom simulator revision and known approximations                                    |
| Interfaces          | Observation/action/order/report schemas; feature units; normalization statistics; candidate generator and masks           |
| Execution           | Formation formulas and presets; lifecycle rules; targeting/reflexes; scheduler; fallback scripts                          |
| Hierarchy           | Parent, sibling, child, and opponent policy versions; assigned authority; communication delays                            |
| Learning            | Algorithm/configuration; heuristic prior; initial actor; training seed; reward terms; training budget                     |
| Evaluation          | Scenario split/version; objective definitions; primary metric; minimum useful gain; regression tolerances; runtime budget |

Any changed executor, preset, reward, normalization, or heuristic prior creates a new experiment condition. Do not attribute a movement-code improvement to a better optimizer. When changing a bundle, rerun its matched baselines.

## 3. Common campaign workflow

A **campaign** is a bounded experiment on a named decision problem with a fixed bundle, training budget, evaluation plan, and promotion rule. One campaign should answer a concrete question such as “does learned mode selection improve escort completion over the heuristic with these executors?”

### 3.1 Before training

1. Define the owner, mission family, observations, available actions, authority, and episode lifecycle.
2. Select a competent baseline and confirm that the task is achievable by at least one suitable controller. Include intentionally infeasible cases as a separately scored reporting task.
3. Freeze parents/children/services not under study. Generate training, development, and sealed test scenario families.
4. Declare the primary outcome, meaningful improvement, permitted regression, resource budget, and analysis method before inspecting the final test results.
5. Run the baselines and inspect a small set of traces. Fix interface or execution defects before collecting expensive training data.

Split by scenario family, map geometry, composition/matchup, and related initial-state ancestry. New random seeds of a familiar arena test stochastic reliability; they do not alone demonstrate generalization. Keep all branches and neighboring clips from one saved engagement in the same data partition.

### 3.2 While training

Collect trajectories or candidate fitness under the fixed bundle. Evaluate checkpoints on the development suite; retain the strongest baseline and previous promoted checkpoint. Keep easy/common cases in the curriculum as harder scenarios are added.

Log simulated frames or game-seconds, battles completed, policy decisions, training updates, wall-clock time, hardware, and peak memory. Count demonstrations, branch rollouts, failed candidates, hyperparameter trials, and selection of initial weights in the cost of the relevant experimental pipeline.

Do not tune the curriculum, reward, or checkpoint against the sealed test set. A scenario examined during diagnosis becomes a development/regression example; replace its role in the next sealed test when necessary.

### 3.3 After training

1. Compare the selected checkpoint against the strongest matched baseline on held-out scenarios and engine validation cases.
2. Report results by mission, role, matchup, terrain, and failure type as well as aggregate performance.
3. Estimate uncertainty across independent training seeds and evaluation scenarios; use paired comparisons where possible. Resample at the independent match/scenario level rather than treating every decision as independent evidence.
4. Review a small mix of ordinary behavior, failures, surprising successes, and suspicious traces.
5. Promote, retain the baseline, or run a specific follow-up experiment. Preserve the manifest, checkpoint, traces, outcomes, annotations, and reason for the decision.

Choose replication and evaluation size from pilot variability and the practical gain being sought. A single successful training seed or a narrow win-rate difference is insufficient evidence of superiority.

## 4. Squad workflow: formation selection with fixed execution

This is the largest V2 revision. Architecture §§6.2 and 8.2 define the formation responsibilities; this section supplies the experimental sequence.

### 4.1 S0 — Reproduce execution and lifecycle behavior

Reuse the existing tested formulas where available. Produce one frozen version of all five modes with role/stat-conditioned presets, fixed targeting and reflexes, parent constraints, and the same scheduler used by every selector.

Verify the following before training:

- Selecting the active mode again preserves state; an explicit assignment revision resets only the state it actually invalidates.
- Orbit direction and slot/arc assignments remain stable when no material change requires reassignment.
- Cancellation, target loss, blocked paths, unavailable firing positions, and permitted emergency interrupts have documented outcomes.
- Transit, regroup, withdrawal, and role-specific scripts have explicit lifecycle ownership. Logs identify when they take control and when selection resumes.
- A requested switch, a deferred switch, an effective maneuver, and a failed maneuver are distinguishable in reports.
- Hard constraints are enforced by code. Costly legal actions remain available within the parent's authority.

Do not force a nonexistent target into `CHARGE` or silently interpret every blocker as `HOLD`. A valid fallback may complete a task, or it may merely keep the squad safe while reporting that the task is blocked.

### 4.2 S1 — Establish selection baselines and headroom

Evaluate these on matched scenarios:

| Baseline                                            | What it establishes                                                                                                          |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| Each fixed applicable mode                          | Where each executor succeeds or fails without switching                                                                      |
| Simple role/matchup rules                           | Whether basic range and mobility knowledge explains most of the benefit                                                      |
| Strong mission-aware heuristic selector             | The actual baseline learning must improve, including persistence, support, and goal constraints                              |
| Best fixed mode chosen with hindsight per encounter | Offline diagnostic of variation across modes; not a deployable selector, optimal switching policy, or guaranteed upper bound |

Reproduce the reported preferences for encircle in tested brawler/raider cases, kite under exploitable range advantage, and orbit as a possible fallback. Treat these as hypotheses for those encounter families. Preserve charge and hold and actively test situations that favor them.

Use repeated matched encounters to separate performance variation from genuine choice value. If the heuristic already makes reliable choices and failures mainly come from missing behaviors or bad execution, stop expanding the selector experiment and address the responsible component.

### 4.3 S2 — Define the smallest selector

**Initial recommendation:** five categorical outputs, one per mode. Choosing the current mode means continue, so a sixth `CONTINUE` output is unnecessary initially. This is compatible with the architecture's continue semantics while avoiding duplicate actions with identical effects. If an explicit `CONTINUE` is later introduced, document how it differs from selecting the active mode and how it behaves without an active maneuver.

Retain executor state outside the network. Include enough state in observations to make good choices: current mode and age; assignment/target/anchor; goal and authority; role/stat/health summaries; effective range and mobility relationships; accessible retreat and approach space; distinct threat groups; sibling support; firing participation; progress; and blockers. Hidden executor state that changes transition behavior must have an adequate observable summary or a justified history model.

Derive mode distances, spacing, widths, direction, slot selection, targeting, and reflex thresholds from the frozen presets and assignment. The initial network does not output them.

Use action masks for unavailable or prohibited modes: missing prerequisites, incompatibility with an explicitly constrained parent assignment, or a documented lifecycle restriction. Dwell locks are part of that protocol; expose their remaining duration and emergency exceptions. A poor range matchup is usually a soft preference, not proof that an action is invalid. Ensure every selection state has a valid action; the lifecycle controller handles states where no combat mode applies.

Prefer a compact shared role/stat/mission-conditioned feed-forward model first. Keep engineer, repair, logistics, aircraft, and special-ability scripts where the five combat modes do not represent their jobs. A responsibility or role name does not imply a separate MLP.

### 4.4 S3 — Optional supervised initialization

Choose one initial comparison design and keep it consistent across evolution and PPO:

- Clone trusted heuristic choices into the selector, including continue decisions and appropriate switches.
- Or use a finite heuristic preference plus a learned correction to action logits, with a zero/small initial correction.
- Or train without a prior as a separate ablation when its additional cost is justified.

For a residual actor, define the behavior distribution explicitly:

\[
\pi(a\mid o)=\operatorname{softmax}\bigl(h(o)+f\_\theta(o)\bigr)\_a
\]

Apply the same legal-action mask before sampling and before calculating training probabilities. Freeze and version `h`, its scale/temperature, and normalization. PPO must record and recompute probabilities from the entire actual sampling distribution. Adding a heuristic or hysteresis transformation after selecting the action must not be hidden from the transition contract.

Clear human mode corrections are useful examples. Branch rollouts from a saved state can supply comparisons or teacher candidates, but one favorable future does not establish the correct action for every indistinguishable observation. Repeat stochastic alternatives and account for hidden-state uncertainty before treating a branch result as a trusted label.

Balance correction examples so long stretches of unchanged modes do not bury rare useful switches. Preserve representative ordinary states as well as mistakes. Validate imitation on complete unseen engagements.

### 4.5 S4 — Compare learned selection methods

Keep the same model family, observations, initial prior, action semantics, cadence, fixed execution, reward/fitness components, and scenario distribution where the algorithms permit. Compare:

1. Strong mission-aware heuristic selection.
2. Evolutionary selection using the current trainer.
3. PPO selection using a standard implementation and masks where needed.

Use both a fixed simulation-work budget and a fixed wall-clock budget, reported separately. Derive evolutionary fitness and PPO reward from the same declared task return, horizon, timing convention, and cost terms rather than giving either method a more favorable objective. PPO and evolution need not have identical internal update counts. Apply comparable tuning budgets and include warm-start costs. Evaluate deployed deterministic selection initially; test controlled stochastic deployment as a separately declared condition if useful.

Start with fixed selection intervals, distinct from executor updates and engine-order issuance. Select/retain a mode, run the frozen closed-loop executor for the interval, aggregate its consequences, and return the next observation. Dwell restrictions, permitted emergencies, and parent cancellations remain observable environment behavior (§8).

### 4.6 S5 — Curriculum and promotion

| Stage           | Additions                                                                               | Required lesson                                                  |
| --------------- | --------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| Basic matchups  | Applicable homogeneous combat squads; flat open terrain; force/range/mobility variation | Different modes suit different matchups                          |
| Mission pairs   | Same physical start under destroy, hold, delay, escort/screen, or withdraw goals        | Goal changes behavior with the same weights                      |
| Persistence     | Moving threats; temporary exposure; mode age; multi-phase encounters                    | Retain useful maneuvers and switch when circumstances justify it |
| Geometry limits | Obstacles, partial arcs, choke access, retreat room, verified weapon-specific elevation | Recognize executor opportunity and limitations                   |
| Cooperation     | Scripted sibling squads, screens, synchronized commitment, protected support            | Respect platoon coordination and authority                       |
| Uncertainty     | Lost contacts, multiple threat groups, incomplete identities, varied opponents          | Use legal observations and honest uncertainty                    |
| Transfer        | Held-out terrain/compositions and real-engine cases                                     | Retain useful behavior beyond familiar arenas                    |

Reward subgoal achievement and timely contribution. Interpret losses through authority and objective. Screening may be excellent despite little personal damage. Do not reward a formation label, geometric circle, movement, or survival independently of the assigned goal. Treat switch count as a diagnostic first; if adding a switching penalty, verify that it does not suppress necessary recovery.

**Promote only if** the learned selector gives a repeatable, practically useful improvement over the strongest matched heuristic, passes mission/authority and persistence regressions, and fits runtime and command budgets. Report selector inference, executor geometry, pathfinding, and issued orders separately. The trained policy must change its choices when the legitimate mission changes.

**If it does not pass:** retain the heuristic and continue to Platoon and Operations. If it passes but a specific weakness remains, test one expansion at a time: bounded preset choice, a parameter refinement, an additional/composite mode, or specialist execution. Rebaseline after each execution change. Learned per-unit movement remains a separate experiment (§5).

## 5. Unit execution and optional micro workflow

**Start:** Engine-order translation, pathing fallback, target execution, local safety, and ordinary ability rules are deterministic. Enforce unit ownership, protected-unit rules, and parent loss authority in the execution layer.

**When to train:** A repeated, measurable execution gap remains after fixing the scripts and validating the relevant game mechanics. Examples include projectile dodging, a specific ability's timing, or a self-destruct/wreck-denial tradeoff. Select one gap per experiment.

**Environment:** Small engine-accurate encounters with a fixed squad parent and fixed selector. Vary legal projectile observations, weapon timing, terrain/obstruction, local allies, health, friendly-fire risk, and mission risk tolerance. Validate self-destruct delay, explosion effects, and wreck behavior directly before using their predictions as rewards or teacher labels.

**Method:** Begin with a scripted baseline and trusted local demonstrations where available. Compare short-horizon RL or evolution using the same action boundary. Share parameters across compatible units and batch inference when worthwhile. Learn sparse eventful corrections, target/ability choices, or bounded control adjustments rather than replacing the entire movement stack at once.

**Signal and evaluation:** Measure the chosen gap: damage avoided at comparable useful damage/mission progress, ability effect, collateral losses, or verified denial value. Also measure order count, path disruption, latency, and performance under different parent missions. A dodge that abandons an authorized critical task can be a tactical failure despite preventing damage.

**Gate:** Keep the deterministic baseline unless the learned extension gives repeatable gains within execution and command budgets. When replacing a child used by other policies, run frozen integration regressions and collect fresh training experience before adapting its parent.

## 6. Higher-layer workflows

### 6.1 Platoon: learn combined-arms plans under mission contracts

**Frozen surroundings:** Scripted Operations parent; deterministic shared services; competent frozen squads using the heuristic or a previously promoted learned selector. Learned squads are optional.

**Action boundary:** Select a bounded complete tactical-plan candidate: route/staging, roles and sectors, target priorities, support relationships, commitment timing, retreat/regroup stages, and required mode constraints. The candidate generator computes legal geometry. Avoid independent choices that can assign incompatible roles to the same squad. Include maintain-plan, permitted revision, and escalation outcomes explicitly.

**Sequence:**

1. Demonstrate a scripted mission executor for capture, hold, delay, destroy/raid, escort/support, recon, and withdrawal as the available role scripts permit.
2. Train on short premade-force scenarios with frozen competent squads. Start with a few mission types; expand routes, unit mixes, deadlines, reinforcement timing, and losses allowed.
3. Use behavioral cloning for trusted complete-plan choices, then compare goal-conditioned PPO with the existing evolutionary method if affordable. Preserve the same candidate semantics and prior.
4. Add paired legitimate missions with identical forces, including costly authorized delay, preservation, and withdrawal. Include hard-impossible cases separately from merely unattractive orders.
5. Train/report calibrated feasibility and predicted cost where data justify them (§9). Exercise parent revision, confirmation, cancellation, and reassignment.

**Reward and metrics:** Mission effect, deadline, support contribution, controllable authorized losses, and asset preservation when ordered. Track unnecessary replanning and abandoned support relationships. Do not make kill/loss ratio the universal objective or reward requests/refusals merely for occurring.

**Gate:** Held-out mission performance improves on the scripted planner; at least two different legitimate goals produce meaningfully different plans with the same weights. Squads remain coordinated, and a useful “cannot accomplish without X” report changes the parent's decision appropriately. Protect every earlier formation/persistence regression.

### 6.2 Operations: learn objectives, allocations, and capability needs

**Frozen surroundings:** Scripted Strategy supplies intent and authorization. Frozen platoons execute missions. A dummy Quartermaster offers feasible forces with explicit costs, assembly/delivery ETAs, capacity limits, and occasional failures.

**Action boundary:** Rank/select candidate objectives and operations; assign available forces and reserves; coordinate deadlines; request reconnaissance or capabilities; revise/cancel plans. Ownership and budget services reject double allocation independently of learned scores. Keep candidate generation observation-legal.

**Sequence:** Begin with prebuilt forces in reduced multi-sector maps. Establish nearest-threat and stronger value/force-aware allocation baselines. Warm-start from trusted allocations if useful, then train candidate selection with goal-conditioned RL or evolution. Add conflicting objectives, reserve commitments, changing intelligence, platoon objections, and deadline-aware supply offers before live production.

Include high-ground opportunities that are valuable, inaccessible, exposed, irrelevant, or suited to different weapons. Include installations where infiltration, siege, conventional attack, recon-first, or deferral is preferable. Specialized missions should emerge as alternatives in the operation set; do not reward requesting spec-ops specifically.

**Signal:** Time-weighted value of meaningful objectives, coordinated mission effects, prevented front failures, useful information, and timely capabilities. Charge the opportunity cost of tied-up forces/resources where the task definition supports it. Evaluate missed opportunities, useless requests, churn, reserve misuse, and response to truthful or mistaken subordinate reports.

**Gate:** It outperforms a credible sector-allocation baseline in unfamiliar multi-front scenarios and recognizes important terrain/capability needs that simple allocation misses. Removing upward feedback or capability requests should expose their measured contribution when those mechanisms are claimed to help. Complete the Operations → Platoon → Squad vertical slice before requiring a learned economy.

### 6.3 Quartermaster: solve scheduling first, learn priorities selectively

**Frozen surroundings:** Validated economic mechanics and legality; scripted Strategy authorization; randomized capability demand from scripted Operations. Use a capability catalog and verified production/assembly/delivery model.

**Start:** A deterministic forecast/scheduler generates feasible compositions, prerequisites, jobs, builder/factory assignments, and delivery alternatives. Treat the Quartermaster as a peer of Operations, with explicit arbitration and reservations. It does not receive authority to spend the same budget twice.

**Sequence:**

1. Benchmark baseline feasibility, forecast accuracy, and on-time delivery without enemies.
2. Add scarce build power, streaming metal/energy limits, constructor travel, competing requests, technology choices, interruptions, and deadlines.
3. Train separate ETA/cost predictors only where the scheduler's estimates have consequential gaps. Score delivery to the actual assembly/handoff point, not just factory completion.
4. If scheduling priorities remain weak, imitate an optimizer/strong teacher, or learn to rank feasible composition and job-priority candidates. Keep placement legality, prerequisite checks, resource reservations, and accounting deterministic.
5. Introduce economic shocks, raids, damaged facilities, lost territory, urgent defense, canceled requests, and live Operations feedback. Reconcile outstanding reservations and revised delivery offers.

**Signal:** Mission-relevant capability delivered before its useful deadline, useful economic investment, appropriate responsiveness, and truthful forecast revisions. Track stalls, idle time, travel, vulnerability, waste, and delivery shortfall as separate costs. A purposeful short stall may be preferable to missing an emergency defense; metal spent is not itself success.

**Gate:** Meets or improves a strong rule-based scheduler on varied held-out demand without invalid commitments; gives honest alternatives for impossible requests and calibrated delivery estimates. Retain deterministic priorities if learning adds no practical gain. Integrate actual force assembly, readiness, ownership transfer, and mission assignment before crediting an end-to-end delivery success.

### 6.4 Strategy: learn long-horizon commitments after the lower loop works

**Frozen surroundings:** Competent Operations and Quartermaster, with frozen lower controllers and services. Scripted substitutes remain acceptable when their limitations are explicit.

**Action boundary:** Choose broad priorities, resource envelopes, tech/economic commitments, major request authorization, doctrine, and risk tolerance. Lower tiers retain their execution responsibilities.

**Sequence:** Start from a scripted macro baseline and a reduced candidate set. Use replay-informed imitation only when high-level intent and the player's legal information can be reconstructed; raw unit commands are not reliable strategic labels. Compare full-game learning against the fixed macro baseline using identical subordinates. Introduce map/economy variation, hidden enemy plans, tech transitions, operational requests, and late-game risks progressively.

Use targeted snapshot/scenario resets for rare crises such as unusual tech, artillery, shields, and nuclear saturation. Include enough remaining context and aftermath to judge the strategic commitment, and retain ordinary full-match evaluation so crisis training does not dominate doctrine.

**Signal:** Team victory or defined long-horizon strategic outcome is primary. Any shaping for economy, army, or territory needs an ablation against actual game outcomes. Do not pay the policy to accumulate assets indefinitely or authorize requests indiscriminately.

**Gate:** Repeatable improvement over the macro baseline on held-out maps/opponents with identical lower controllers, observation privileges, and resources. Report uncertainty, rare-crisis outcomes, poor-matchup regressions, runtime, and response to real Operations/Quartermaster feedback. Escalate to more complex self-play or joint training only after this comparison is trustworthy.

## 7. Rewards, fitness, and responsibility

Each layer's reward must reflect the mission and authority it actually receives. Log primary task outcomes, hard constraints, soft costs, and diagnostics separately, even when a learner ultimately needs a scalar objective.

| Layer          | Primary outcome                                         | Common misleading proxy                                     |
| -------------- | ------------------------------------------------------- | ----------------------------------------------------------- |
| Squad          | Assigned subgoal and useful support                     | Damage, survival, motion, or a preferred mode label         |
| Platoon        | Mission effect within timing/authority                  | Favorable trades while abandoning the objective             |
| Operations     | Valuable theater outcomes and suitable force allocation | Capturing irrelevant sectors or requesting every capability |
| Quartermaster  | Useful, timely, feasible delivery and investment        | Spending quickly or minimizing stalls at any cost           |
| Strategy       | Full-match strategic success                            | Maximizing stockpiles, nominal army value, or match length  |
| Optional micro | A specific useful execution improvement                 | Damage avoidance that undermines the mission                |

Enforce hard constraints through the authority/legality layer rather than assuming a finite penalty prevents violations. Distinguish expected-loss authorization from actual realized losses: a stochastic outcome above an expectation is not automatically proof that an order was unauthorized. Explicit protected-unit or geographic prohibitions remain hard constraints.

Start with terminal task outcomes plus modest, justified intermediate signals. If using potential shaping for duration `d` measured in a declared base time unit, the corresponding term is `gamma^d * Phi(next) - Phi(current)`. Define terminal potential consistently and preserve bootstrap treatment at external truncations. Do not claim shaping is policy-invariant after adding arbitrary bonuses, changing terminal treatment, or ignoring elapsed time.

Use counterfactual diagnostics to localize faults: replay the plan with a competent scripted child; compare candidate availability; inspect whether the actor had relevant information; verify that the selected command executed; and check whether the parent supplied a realistic mission. These are debugging/evaluation tools, not proof of causal blame from one rollout.

## 8. Decision timing and trainer integration

### 8.1 Initial PPO integration

Use Gymnasium for the environment boundary and Stable-Baselines3 PPO for the first gradient-based comparison. Use SB3-Contrib MaskablePPO when legal/lifecycle masks are needed. Keep the current evolutionary trainer; evaluate EvoTorch if maintaining that trainer is itself a burden. See the official references in §13.

For squads, one environment step initially covers one **fixed selection interval**, with many executor/engine updates inside it. For platoons or higher tiers, use a fixed interval appropriate to that tier. Do not inherit the same discount or rollout length mechanically across time scales.

Define and test:

- `reset(seed, ...)` returns the initial legal observation and diagnostics.
- `step(action)` returns the next observation, interval reward, `terminated`, `truncated`, and diagnostics.
- Actual task success/failure or expiry of a deadline that defines the task is terminal. An external collection cutoff is a truncation, with appropriate remaining-value handling.
- Actor observations, normalization, masks, priors, and likelihood evaluation agree in collection, optimization, evaluation, and deployment.

SB3 PPO supports discrete actions and dictionary observations; arbitrary dictionary actions are not its standard action interface. Bounded whole-plan indices are a practical starting point for higher tiers. Candidate descriptors must be present in the observation, and padded/absent candidates masked. A candidate index has meaning only with its decision-time candidate list and version.

Use mask-aware evaluation with MaskablePPO. Do not assume it automatically supplies recurrence, centralized multi-agent critics, or the custom residual actor. Start with compact histories/features and fixed peers; add a framework/customization only for a measured requirement.

### 8.2 Persistence, intervention, and variable duration

Prefer masks that expose an active dwell restriction over secretly replacing a requested switch. Nevertheless, legitimate downstream interventions can occur. Record **sampled action, accepted action, actual mode, duration, override reason, and executor outcome**. PPO's stored probability describes the sampled action; changing the logged action to a fallback without changing its likelihood is incorrect. Keep the wrapper's transition semantics fixed and visible.

Deterministic emergency handling can occur inside a fixed interval without forcing an early policy decision. End the episode early for a true terminal outcome as required. Later, if event-driven reselection or interruptible options produce nonterminal transitions with different elapsed durations, explicitly implement time-aware return and advantage calculations.

For base-unit discount `gamma` and duration `d`, bootstrapping uses `gamma^d`; accumulated within-option rewards must use compatible timing. Potential shaping, advantage recursion, and bootstrap handling need the same convention. Logging duration in `info` does not make stock fixed-step PPO apply these changes. Treat this as a trainer extension or choose a suitable implementation, with independently checked synthetic timing cases before large runs.

### 8.3 Supervision, evolution, and PPO bookkeeping

Keep PPO rollouts, supervised examples, preference labels, and evolutionary evaluation records as separate datasets. Historical battles are valuable for diagnostics and supervised predictors; they are not an unlimited on-policy PPO replay buffer.

For a first hybrid, use separate stages: optional imitation/evolutionary initialization, then fresh PPO collection and refinement. After actor mutations, substantial supervised updates, or changes to children, reward, priors, masks, or observation semantics, discard incompatible rollouts and collect a fresh batch. Reassess critic, optimizer, and normalization compatibility rather than blindly copying all state.

Combining a PPO loss and an imitation loss is a custom training design, not a stock framework promise. If needed, declare its strength and schedule, validate preservation of trusted behavior and combat gains, and retain pure PPO/evolution baselines. Avoid a population/PPO hybrid until it addresses a measured failure of simpler methods.

## 9. Train and evaluate the feedback loop

### 9.1 Protocol competence precedes forecast learning

Exercise the architecture §3 lifecycle with scripts first: issue → acknowledge → execute → report risk/blocker → revise/confirm/cancel → finish. Verify that stale orders cannot regain ownership, a revised order has an explicit identity, requests consume or release the right reservations, and neither side can thrash the other with repeated equivalent messages.

Use paired tests for:

- An achievable task, a costly achievable task, a risky task, and a hard-impossible task.
- The same damaged platoon ordered to preserve itself versus execute an authorized delay.
- A missing capability, feasible delivery before the deadline, delivery too late, and a useful alternative mission.
- A truthful report, stale estimate, false confidence, delayed acknowledgment, parent override, and cancellation during execution.

Reports, recommendations, and refusals have different meanings. Confirmation changes authority or intent where applicable; it cannot create a nonexistent path, weapon, or prerequisite. A subordinate should pursue the best feasible authorized effect and report residual shortfall rather than promise an impossible result.

### 9.2 Prediction datasets and calibration

Start with coarse feasibility classes and conservative ETA ranges. Add learned prediction only where better estimates change decisions.

| Prediction             | Example supervised target                                          | Important qualification                                                                        |
| ---------------------- | ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| Mission feasibility    | Success under this mission, authority, and frozen execution policy | Include mission revision/cancellation and policy version; success depends on what happens next |
| Remaining holding time | Time until a defined loss of control                               | Record right-censoring if the episode ends before control is lost                              |
| Loss/cost              | Realized losses/resources over a specified mission horizon         | Incomplete or revised missions are not interchangeable with completed ones                     |
| Delivery ETA           | Time to assembled-ready handoff                                    | Track cancellations, shortages, and undelivered requests separately                            |
| Execution quality      | Range/support/progress failure under a chosen mode                 | Use verified telemetry rather than mode labels as ground truth                                 |

Do not label every canceled mission a failure or a revised one a success for the original order. Avoid reporting a forecast as calibrated beyond the mission/role/policy distribution on which it was tested. Resample or explicitly qualify estimates after major controller replacement.

Measure probability calibration and a proper scoring rule when predicting probabilities; interval coverage and errors for ETA/losses; false optimism and excessive pessimism separately. Compare against simple constant or rule-based forecasts. Test whether using the report improves parent decisions through controlled feedback ablations, rather than promoting a predictor only because its average error decreased.

An objection is not a rewardable end in itself. Do not let a controller learn to avoid difficult missions through automatic refusal, or a parent learn to ignore every report because easy cases dominate training.

## 10. Human review and supervised learning

Keep reviews lightweight and cumulative. A practical starting budget is a few engagements per campaign, with one or two annotated moments per engagement; this is a workflow choice, not a claim that a few labels can train a general evaluator.

### 10.1 What the reviewer sees and records

Show the assigned objective, authority/constraints, the information actually available to the actor, candidate plans or modes, selected and executing behavior, relevant reports, and enough aftermath to assess the purpose. Keep an explicitly labeled omniscient debugging view separate from actor-view policy annotations.

Record scenario/mission/decision identity, exact policy/bundle versions, timestamp, judgment type, reason, confidence, and any preferred alternative. Optional overall ratings use these meanings:

| Rating        | Question                                                                                  |
| ------------- | ----------------------------------------------------------------------------------------- |
| Effectiveness | Did it accomplish the assigned effect in time?                                            |
| Efficiency    | Was the expenditure of units, resources, and time justified by that effect?               |
| Sanity        | Was its behavior coherent with its information, assignment, and coordination constraints? |

Allow “unusual but effective,” ties, and “cannot judge.” Efficiency must not penalize an authorized sacrifice merely because it was expensive.

### 10.2 Route feedback to the responsible component

| Review finding                                            | Appropriate next action                                                                      |
| --------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Wrong mode among represented legal choices                | Trusted supervised mode correction or paired alternative rollout                             |
| Good mode, broken movement/slots/range correction         | Fix and version the executor; rerun matched baselines                                        |
| Useful behavior absent from candidate/action library      | Candidate/service or narrowly scoped behavior experiment                                     |
| Missing legal information/history                         | Observation or state-summary change, followed by new baselines                               |
| Unsupported pursuit violates the combined-arms assignment | Inspect platoon plan, squad authority, and lifecycle enforcement before blaming the selector |
| Honest local choice undermines the global objective       | Inspect parent intent/reward/authority and train mission pairs                               |
| Exploit of a reward or learned evaluator                  | Fix the evaluation/reward condition and rerun affected comparisons                           |
| Unclear cause                                             | Diagnostic tag or request for a comparison, without inventing a correct action               |

Collect ordinary states, failures, unexpected successes, and traces with blockers/churn; do not review only the worst losses. Corrections drawn from the current policy's visited states can supplement earlier demonstrations and reveal distribution shift.

Use direct observation–action corrections first. If applying a separate supervised phase, verify its effects and collect fresh PPO experience afterward. Do not treat every decision in a poorly rated match as a bad action.

### 10.3 Optional learned evaluator

Accumulate comparisons between objective-conditioned clips, preferably branches from a common initial situation when available. Keep entire related encounters together in dataset splits. Begin with compact telemetry features and a small evaluator, with held-out human judgments reserved for testing.

Only give its predictions training influence after it predicts unseen judgments usefully and survives checks for exploitable blind spots. Keep objective battle/mission outcomes independent. Freeze the evaluator and weighting during a comparison batch or evolutionary generation; reevaluate candidates after changing it. The evaluator and reward version belong in the campaign manifest.

The `imitation` library provides behavioral cloning and preference-learning components; the BAR trace viewer, annotation store, actor-view reconstruction, and correction semantics remain project work. A broad rating dataset can remain valuable for diagnosis even if no learned reward model is ever promoted.

## 11. Integration and campaign order

This table maps directly to architecture §9. It is dependency order, not a calendar or a promise of compute cost.

| Milestone                      | Training/development campaign                                                                                                      | Exit evidence                                                                                                           |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| **M0** Mechanics and runner    | Reproduce small actual-engine encounters and reported fixed-formation baselines; measure custom-simulator agreement and throughput | Reproducible legal observations, commands, outcomes, and replay traces; documented fidelity limits                      |
| **M1** Contracts               | Exercise complete scripted order/report/objection/revision and resource/ownership lifecycles                                       | No conflicting ownership or hidden-information leakage in the defined tests                                             |
| **M2** Deterministic baselines | Freeze formations, presets, targeting/reflexes, support scripts, heuristic selector, mission executor, and economy scheduler       | Playable zero-learning tasks; baseline outcomes, persistence, blockers, and runtime costs recorded                      |
| **M3** Squad selection         | S0–S5: headroom, optional imitation, evolution/PPO comparison with identical executors                                             | Competent heuristic retained, or a learned selector promoted for useful held-out gains; no learned geometry requirement |
| **M4** Platoon                 | Mission-conditioned combined-arms plans with frozen squads and realistic/impossible assignments                                    | Goal-dependent coordination, mission success, and useful upward reports                                                 |
| **M5** Operations              | Multi-sector planning with supplied forces and a constrained dummy Quartermaster                                                   | Meaningful terrain/capability decisions and adaptive report handling; first full vertical slice                         |
| **M6** Quartermaster           | Production/assembly/delivery sandboxes, forecast calibration, optional priority learning, live integration                         | Intelligence → request → authorization → production → handoff → assigned mission → reported outcome                     |
| **M7** Strategy                | Macro teacher, full-game comparison, targeted rare-crisis curricula                                                                | Full-game improvement over identical-subordinate macro baselines                                                        |
| **M8** Joint adaptation        | Frozen integration regressions, then limited selective unfreezing and broader opponent/team formats                                | Integrated gains without losing prior competence or exceeding runtime budgets                                           |

Quartermaster sandbox work may proceed independently after contracts stabilize. Swap M5/M6 development emphasis if an existing economy controller makes that practical. Preserve the explicit capability and handoff interfaces.

During integration, freeze both adjacent components first. Replay actual parent-generated orders into child evaluations and actual child reports into parent evaluations. Expand the training distribution to cover the newly observed boundary cases before unfreezing. Change one owner at a time where practical, keep compatible checkpoint bundles, and retain a rollback baseline.

An opponent pool should include scripted tactics, previous checkpoints, and targeted counters; keep opponent selection/version fixed for controlled comparisons. Team games require separate ally-information, ownership, communication, and coordination tests. A successful 1v1 vertical slice does not by itself validate those interfaces.

## 12. Promotion checklist and first implementing-agent assignment

### 12.1 Common release gates

Every promoted component must have evidence for:

1. **Correct semantics:** legal observations; stable action/mission identities; explicit authority; reproducible lifecycle and fallback behavior.
2. **Useful task gain:** meaningful held-out improvement over a credible matched baseline, or explicit retention of the simpler baseline.
3. **Goal conditioning:** changed legitimate assignments produce appropriate changes with the same weights.
4. **Robustness:** mission/role/matchup breakdowns, adverse scenarios, independent seeds, and relevant engine validation.
5. **Honest feedback:** blockers and forecasts carry useful information; parent responses improve the command loop where claimed.
6. **Runtime fit:** inference, deterministic execution, pathfinding, order count, and frame-time spikes fit measured budgets.
7. **Replaceability:** the module can be swapped for its scripted fallback under the same contracts; the compatible bundle and rollback path are preserved.

The regression suite must include architecture §10: formation selection, persistence/recovery, mission-versus-mode, complementary squad behavior, terrain, spec-ops alternatives, sacrificial delay and objections, missing counters, economy shocks, fog-of-war traps, coordinated capture, wreck/elevation cases when mechanics are validated, rare tech/nuclear crises at the relevant stage, and module replacement.

Do not demand a nuclear-defense test before a tiny squad experiment. Declare which tests apply to each milestone, and require the full relevant suite before claiming integrated competence.

### 12.2 First implementing-agent assignment

Start with a **formation-selection benchmark and a scripted mission/report vertical slice**, not six simultaneous training jobs:

1. Inventory the current simulator, evolutionary trainer, formation formulas, and existing encounters. Reproduce their results without changing execution.
2. Specify the initial squad selection/lifecycle contract, including five-mode idempotent selection, assignment revisions, dwell, interruption, fallback, and report fields.
3. Build a small mission-paired scenario suite with legal observations, frozen scripted parents/siblings, fixed-mode and mission-aware heuristic baselines, and separate selector/executor/command profiling.
4. Validate the required mechanics and tactic rankings against small actual BAR encounters. Record mismatches and decide which comparisons remain credible in the lightweight simulator.
5. Measure headroom. If warranted, run a small evolutionary versus PPO selector pilot with a shared initialization and fixed execution. Report learning curves against simulation work and time, mission-specific outcomes, uncertainty, and engineering cost.
6. Return a promote/retain decision and one concrete next campaign. Advance to platoon mission coordination with whichever squad controller is strongest and justified.

Required handoff artifacts are the versioned contracts and experiment manifest, scenario split, baseline report, traces/review examples, measured fidelity/runtime limits, and the selected compatible controller bundle. Network widths, exact cadence, hyperparameters, compute estimates, and learning superiority remain empirical decisions.

## 13. Tooling references and verification boundary

The following primary documentation was checked on 2026-10-19 for the framework capabilities described here. Pin mutually compatible releases during implementation; moving documentation URLs do not identify a reproducible software environment.

- [Gymnasium environment contract](https://gymnasium.farama.org/api/env/) — reset/step interface and termination/truncation semantics.
- [Stable-Baselines3 PPO](https://stable-baselines3.readthedocs.io/en/master/modules/ppo.html) — standard PPO, supported spaces, rollout configuration, and discount parameters.
- [SB3-Contrib MaskablePPO](https://sb3-contrib.readthedocs.io/en/master/modules/ppo_mask.html) — invalid-action masking and mask-aware evaluation. Check its supported features before combining extensions.
- [PPO algorithm reference](https://spinningup.openai.com/en/latest/algorithms/ppo.html) — collection/update structure and the on-policy relationship; clipping is not a guaranteed bound on performance or every probability change.
- [Behavioral cloning in imitation](https://imitation.readthedocs.io/en/latest/algorithms/bc.html) and [preference comparisons](https://imitation.readthedocs.io/en/latest/algorithms/preference_comparisons.html) — supervised policy and preference-learning components.
- [EvoTorch neuroevolution](https://docs.evotorch.ai/latest/user_guide/neuroevolution/) — optional tooling for evolutionary neural-policy optimization.

The architecture is the source for project responsibility boundaries and reported formation preferences. This companion's curricula, staging choices, action encoding, and promotion procedures are proposed engineering judgments. No project code, training runs, BAR API implementation, simulator fidelity, or claimed formation performance was inspected or experimentally verified for this document.
