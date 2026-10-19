# Beyond All Reason AI: Hierarchical Architecture and Training Roadmap

**Status:** Proposed design / research handoff — **not** an implementation specification  
**Audience:** An engineering or research agent tasked with investigating feasibility, designing training pipelines, and implementing the system  
**Primary source:** `BAR MLP argon-output.pdf` (28-page conversation), plus the subsequent architecture review and design discussion  
**Focus:** A practical, learnable, scalable command hierarchy for Beyond All Reason (BAR), including production, operational planning, combined-arms tactics, and bidirectional feedback  
**Revision:** 2026-10-19 — formation-first squad control: deterministic execution; learn mode selection before considering learned geometry.

---

## 0. Instructions for the next agent

Use this document as the **preferred conceptual starting point**, not as immutable architecture. Your assignment is to turn it into a verified, measurable implementation and training plan.

1. **Verify the engine and game assumptions** before committing to interfaces or training environments. In particular, investigate Recoil/BAR APIs, command semantics, observability, pathfinding, elevation/ballistics, radar and jammer behavior, build-power/economy mechanics, unit abilities, and nuclear interception.
2. **Investigate the best training approach for each controller**, including algorithms, network/input architectures, reward functions, curricula, simulator requirements, pretrained data, evaluation, and estimated compute costs. Do not assume every controller needs PPO or even a neural network. For squads, first compare heuristic and learned **selection among fixed deterministic formations**, not end-to-end learned movement.
3. **Design a working vertical slice first.** Prioritize a small Operations → Platoon → Squad system with scripted providers for everything else, then connect production and strategic planning.
4. **Keep decisions and interfaces separate from implementation.** A responsibility called a “brain” may be an MLP, a graph/spatial encoder with an MLP head, a rules engine, an optimizer, or a combination.
5. **Propose alternatives when evidence warrants them.** Flag assumptions, recommend experiments that distinguish options, and report actual benchmarks rather than repeating unverified timing, neuron-count, or training-cost claims.
6. **Return concrete artifacts:** a reviewed architecture and data contract; a proposed engine/simulator bridge; per-controller training specifications; a milestone plan with acceptance tests; profiling estimates; and a list of unresolved research questions.

The most important design requirement is **not simply stronger micro**. It is the ability to reason across levels: discover an opportunity, identify a missing capability, obtain or assemble that capability, carry out the mission, and revise the plan when circumstances change.

---

## 1. Core design principles

### 1.1 Clear ownership of decisions

Every meaningful decision should have one **default owner**, even if other controllers can propose it. Avoid both a giant Strategic AI micromanaging everything and a set of independent controllers that fight over the same units, resources, and objectives.

### 1.2 Bidirectional command, not blind obedience

A parent assigns **intent, objectives, constraints, and resources**. A subordinate chooses how to execute within its delegated authority and returns **progress, feasibility, predicted cost, blockers, and alternatives**. Subordinates may identify opportunities that were not anticipated from above.

A locally unattractive action may still be globally correct. A platoon may need to sacrifice itself to delay an attack. Consequently, the parent must communicate acceptable sacrifice and mission priority; the platoon must not optimize local metal efficiency to the exclusion of its mission.

### 1.3 Capability requests instead of hard-coded unit recipes

Operational commanders should usually ask for capabilities (“reconnoiter this sector,” “disable that artillery,” “hold this passage for 90 seconds”), not fixed lists of unit types. Production should translate requirements into feasible unit compositions, considering technology, opportunity cost, travel time, and delivery deadlines.

Predefined compositions are useful initial baselines, **not the final representation** of requirements.

### 1.4 Learned judgment, deterministic mechanics

Use learning for context-dependent decisions: priorities, commitments, tactics, candidate selection, trade-offs, and uncertainty. Use deterministic or optimization-based services where game rules and geometry can be calculated more reliably: legal actions, path feasibility, placement candidates, ballistic/coverage estimates, build prerequisites, and command scheduling.

**For squads, the initial learning problem is which formation to use and when to retain or change it.** Execute `CHARGE`, `HOLD`, `ORBIT`, `ENCIRCLE`, and `KITE` with simple, tested deterministic formulas. Do not ask a squad MLP to rediscover their geometry or output individual movement vectors. Keep the formation library fixed while evaluating selectors; add learned execution only for a demonstrated remaining weakness.

### 1.5 Trainable in isolation, composable in deployment

Each controller must support a stable goal/observation/action/status interface. Early training should provide competent scripted parents and children, synthetic mission assignments, and focused scenarios. Integration should use frozen components first, followed by cautious coordinated adaptation.

### 1.6 Honest partial observability

An acting policy must never receive enemy state the player could not know. Maintain confidence, timestamps, and predictions for contacts lost in fog of war. A centralized critic may use richer information *during training* if explicitly justified, but the deployed actor must remain observation-legal, including through all derived features.

### 1.7 Evaluation before optimization

Start with simple heuristics and independently measurable tasks. Only increase network complexity when experiments show a useful gain. Preserve independent metrics for effectiveness, efficiency, plausibility/sanity, robustness, inference cost, and command churn.

---

## 2. Proposed architecture

The following are **responsibility boundaries**. They are not a commitment to six entirely independent learned neural networks.

```text
                         ┌──────────────────────┐
                         │  STRATEGIC DIRECTOR  │
                         │  global goals/budgets │
                         └──────────┬───────────┘
                                    │
                     ┌──────────────┴──────────────┐
                     │                             │
             ┌───────▼────────┐          ┌────────▼────────┐
             │   OPERATIONS   │◄────────►│  QUARTERMASTER  │
             │ theaters, force│ requests │ factory/eco/    │
             │ missions, needs│ & ETAs   │ construction    │
             └───────┬────────┘          └─────────────────┘
                     │
             ┌───────▼────────┐
             │    PLATOON     │  one local mission;
             │   COMMANDERS   │  combined-arms coordination
             └───────┬────────┘
                     │
             ┌───────▼────────┐
             │     SQUAD      │  formation selection;
             │   CONTROLLERS  │  deterministic formation execution
             └───────┬────────┘
                     │
             ┌───────▼────────┐
             │  UNIT EXECUTION│  engine orders, safety reflexes,
             │   / MICRO      │  optional shared learned micro
             └────────────────┘

       Shared services for every layer:
       ─ observed world state / intelligence belief and uncertainty
       ─ terrain, LOS, mobility, weapon envelopes and threat analysis
       ─ capability catalog, production forecasts and action legality
       ─ path / placement candidate generation and command scheduler
       ─ event log, metrics, simulator/replay, scenario runner
```

**Important:** Operations and Quartermaster are *peers* below Strategy. The Quartermaster is not a battlefield chain-of-command tier. They exchange requests and feasibility information under strategic priorities and resource authorization. A dedicated arbitration/budget mechanism must prevent them from making conflicting commitments.

### 2.1 Responsibilities, inputs, outputs, training

| Component                     | Inputs, in addition to shared observations                                                                                                                          | Primary outputs                                                                                                                     | Upward feedback                                                                                                   | Training starting point                                                                                                                                 |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Strategic Director**        | Global economic/technological trends; territorial control; enemy-strategy beliefs; production availability; operational reports; strategic risks                    | Global priorities, objectives and doctrine; resource envelopes; tech commitments; approval/denial of major requisitions             | Terminal layer; logs chosen commitments and predicted consequences                                                | Full matches with competent frozen/scripted subordinates; imitation of a scripted macro baseline and RL/self-play later                                 |
| **Operations Commander**      | Strategic objectives and authority; sector maps; threat and uncertainty estimates; platoon readiness; reinforcements and ETAs; available capabilities               | Regional/theater objectives; platoon assignments; force allocation; reserves; coordinated timings; capability/recon requests        | Mission feasibility, front status, opportunities, capability deficits, projected sector outcomes                  | Multi-sector scenarios with prebuilt forces and scripted supply; goal-conditioned RL over discrete/candidate missions                                   |
| **Quartermaster**             | Resource/energy flows and reserves; build power; facilities; technologies; constructor locations; resource authorizations; delivery requirements                    | Production, construction and tech-job queues; allocation/priority of build power; feasible composition plans; assembly and delivery | Completion ETAs, budget deviations, stalls/bottlenecks, impossibilities, alternative compositions and costs       | Economy/production sandboxes with randomized goals; deterministic planner baseline; constrained scheduling + optional learned priorities                |
| **Platoon Commander**         | One mission contract; squad roster and capabilities; local terrain and threats; neighbor assignments; mission deadlines and loss allowance                          | Tactical plan; approach/retreat route; positions; engagement posture; per-squad tasks; reinforcement/abort requests                 | Progress, predicted costs, ability to meet mission, emerging threats/opportunities                                | Combined-arms sandbox scenarios; fixed mission types; scripted or pretrained squads; goal-conditioned RL                                                |
| **Squad Controller**          | Assigned subgoal/constraints; unit stats; effective range/mobility matchup; local terrain/contacts; platoon and sibling geometry; current mode and execution status | Select/continue a formation mode; deterministic presets generate spacing and movement; fixed targeting/ability rules initially      | Active mode, readiness, actual progress, range/support failures, blockers and alternatives; aggregated to platoon | Deterministic charge/hold/orbit/encircle/kite plus heuristic selector; optional shared role-conditioned MLP learns mode selection with executors frozen |
| **Unit Execution / Reflexes** | Unit mechanics, current orders, actual projectiles, nearby obstacles, HP and cooldowns                                                                              | Sparse engine commands; emergency dodging, target execution, local retreat or ability/self-destruct actions                         | Exceptions, unit status, order execution failures; aggregated                                                     | Deterministic baseline first; optional fast shared policy trained against specific micro challenges                                                     |

### 2.2 Typical update behavior

Do **not** hard-code an arbitrary per-layer frequency until profiling. Initial targets to measure:

- Strategy: slow periodic decisions (roughly every 5–20 seconds), plus interrupts for major events.
- Operations: periodic replanning (roughly every 1–5 seconds), plus interrupts for breakthroughs, loss of sectors, and important sightings.
- Quartermaster: regular resource scheduling (roughly every 0.5–2 seconds), plus job-completion or supply-shock events.
- Platoon: 1–5 Hz for tactical decision updates, event-triggered when the mission becomes infeasible.
- Squad: profile deterministic formation execution separately from formation selection. The existing 2–10 Hz range is an initial executor target to measure, not a required neural-inference rate. Select/retain a mode at a coarser fixed decision interval initially; permit deterministic emergency handling between decisions and investigate event-driven reselection later. Maintain mode state rather than restarting a maneuver at each update.
- Unit reflexes: event-driven or per simulation frame where required; not necessarily neural inference at every frame.

**Inference cadence and issued-order cadence are different.** Repeatedly canceling/reissuing pathfinding orders can be much worse than evaluating a policy frequently. Command scheduling should coalesce orders, enforce minimum dwell times, and permit immediate overrides for real emergencies.

---

## 3. The command/feedback protocol is a first-class feature

The document's most important design choice is the **two-way interface**. Implement and test it before serious multi-controller RL.

### 3.1 An order has five parts

1. **Desired effect / success condition:** what should change in the world; explicit termination conditions where possible.
2. **Intent:** why the effect matters, so a subordinate can choose sensible alternatives.
3. **Authority:** what deviations, tactical actions, resource commitments, and losses the subordinate may choose without permission.
4. **Constraints:** hard prohibitions, timing/deadline, geographic limits, protected units, exposure constraints, etc.
5. **Validity / lifecycle:** issue time, expiry, priority, cancellation and revision rules.

### 3.2 A subordinate report has five parts

1. **State and progress:** accepted, executing, at risk, blocked, completed, failed, canceled; measurable progress.
2. **Feasibility:** whether it can accomplish the mission and a calibrated success/confidence estimate where available.
3. **Cost and ETA:** predicted losses, resources needed, time to completion, and uncertainty.
4. **Blockers / missing capabilities:** why progress is threatened and what is needed.
5. **Alternatives / opportunities:** revised plan, request for reinforcement, worthwhile objective, or refusal with reason.

Predicted probabilities should be treated as **unreliable until calibrated**. Start with coarse confidence classes or heuristic estimates if necessary rather than making up percentages.

### 3.3 Illustrative data contract

Adapt this into a versioned typed schema (JSON Schema, Protobuf, Cap'n Proto, etc.). The specific fields and units are provisional.

```typescript
type MissionType =
  | "CAPTURE" | "HOLD" | "DELAY" | "RAID" | "DEFEND"
  | "RECON" | "ESCORT" | "INTERDICT" | "DESTROY"
  | "INFILTRATE" | "WITHDRAW" | "SUPPORT";

type MissionStatus =
  | "PROPOSED" | "ACCEPTED" | "ACTIVE" | "AT_RISK"
  | "BLOCKED" | "COMPLETED" | "FAILED" | "CANCELED";

interface MissionOrder {
  schemaVersion: number;
  missionId: string;
  parentId: string;
  recipientId: string;
  type: MissionType;
  targetRegionId?: string;
  targetEntityId?: string;       // Only if actor can legally know it
  intent: string | number[];     // Human-readable/debug or encoded intent
  successCriteria: TypedCondition[];
  priority: number;
  deadlineFrame?: number;
  expirationFrame?: number;
  resourceAuthorizationId?: string;
  maxExpectedLossMetal?: number;
  unitsToPreserve?: string[];
  permittedDeviations: string[];
  hardConstraints: TypedConstraint[];
  assignedUnitIds: string[];
}

interface MissionReport {
  schemaVersion: number;
  missionId: string;
  timestampFrame: number;
  status: MissionStatus;
  progress: number;
  feasibility: "HIGH" | "MEDIUM" | "LOW" | "UNKNOWN";
  etaFrames?: number;
  expectedLossMetal?: number;
  confidence?: number;
  blockers: TypedBlocker[];
  requestedCapabilities: CapabilityRequest[];
  alternativeMissionProposals: MissionProposal[];
  urgent: boolean;
}

interface CapabilityRequest {
  requestId: string;
  requesterId: string;
  targetRegionId?: string;
  effect: string;                 // e.g., "disable artillery"
  requiredCapabilities: Record<string, number>;
  optionalCapabilities?: Record<string, number>;
  latestUsefulDeliveryFrame?: number;
  authorizedBudgetId?: string;
  criticality: number;
}

interface CapabilityOffer {
  requestId: string;
  feasible: boolean;
  compositionCandidates: Array<{
    unitCounts: Record<string, number>;
    estimatedMetalCost: number;
    estimatedEnergyCost: number;
    deliveryEtaFrame: number;
    confidence: number;
  }>;
  blockers: TypedBlocker[];
}
```

**Separate these software contracts from policy tensors.** The agent can turn a typed order into a normalized feature vector or learned embedding, but the system must retain the explicit semantics for debugging, safety constraints, and independent testing.

### 3.4 Escalation, objections, and overrides

- A subordinate may **adapt locally** within delegated constraints without asking every frame.
- A subordinate should **report and propose alternatives** when a mission becomes very costly, impossible, inconsistent, or less valuable than a discovered opportunity.
- A parent may **confirm a costly order** if the loss has strategic value; confirmation may adjust the loss allowance or priority.
- **Hard impossibility** (no path, no required unit, invalid target, impossible prerequisite) returns a failure/blocker rather than pretending to execute.
- Immediate reflexive self-preservation may temporarily override movement where allowed, but must not silently replace a high-priority mission.
- Orders need versioning, acknowledgments, cooldowns/hysteresis, cancellation rules, and conflict resolution; otherwise the hierarchy can thrash or deadlock.
- Make the distinction between **feasibility report**, **recommendation**, and **refusal** explicit.

**Canonical scenario:** Operations orders a damaged platoon to hold a pass for 120 seconds. The platoon estimates it can hold for 40 seconds, reports insufficient anti-artillery support, and suggests a fallback. Operations either revises the order or explicitly approves a sacrificial delay because its main army needs 120 seconds to escape. The subordinate must optimize *the approved mission*, not a private “never lose metal” objective.

### 3.5 Requests need an economic arbitration path

A platoon requests reinforcement → Operations validates/aggregates the need → Strategy authorizes scarce resources and sets priorities as needed → Quartermaster returns feasible compositions and delivery dates → Operations chooses assignments and can revise its plans. Emergency handling may bypass routine cycles but should still consume an explicit budget or priority envelope.

Prevent multiple controllers from independently spending the same resources or issuing conflicting unit orders.

---

## 4. Who discovers opportunities, makes new platoons, and chooses terrain?

This is the main missing piece from the PDF's later three-tier proposal.

### 4.1 Decision ownership

| Decision                                                               | Default owner                                                                                              | Supporting roles                                                                                |
| ---------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| What broad economic/military objective matters most?                   | Strategy                                                                                                   | Operations and Quartermaster report needs and feasibility                                       |
| Which regions/fronts deserve forces and reserves?                      | Operations                                                                                                 | Strategy supplies priorities; world model supplies candidates                                   |
| Should we secure this ridge/choke/high ground?                         | Operations for theater objectives; Platoon for local tactical positions                                    | Terrain service proposes useful positions; Strategy can designate globally decisive terrain     |
| Do we need a spec-ops or reconnaissance mission?                       | Operations proposes; Strategy authorizes major investment                                                  | Intelligence identifies unknowns/opportunities; platoons propose targets                        |
| What *effect* is required from a new force?                            | Operations                                                                                                 | Platoons report capability gaps                                                                 |
| Which units can provide that effect, at what cost and when?            | Quartermaster/capability planner                                                                           | Operations supplies mission constraints; Strategy authorizes tech and budget                    |
| How should a combined-arms platoon accomplish an assignment?           | Platoon                                                                                                    | Squads report local constraints                                                                 |
| Where should different weapon/role squads position themselves?         | Platoon assigns sectors, support relationships and constraints; Squad computes local positions within them | Deterministic formation executor, terrain, weapon geometry and path services                    |
| Which charge/hold/orbit/encircle/kite mode should a squad use now?     | Squad selector, within the Platoon assignment and delegated authority                                      | Platoon coordinates commitments and may constrain modes; executor reports progress and blockers |
| How should a threatened individual unit dodge/withdraw/use an ability? | Unit execution or squad policy                                                                             | Platoon sets retreat risk tolerance and mission priority                                        |
| When/how much anti-nuke or shield capacity is worth buying?            | Strategy budgets risk; Operations identifies protected areas; Quartermaster provisions                     | A deterministic throughput/geometry estimator computes vulnerability                            |

### 4.2 Example: deciding to request a spec-ops platoon

1. Intelligence detects or suspects a valuable enemy installation, but direct attack lanes have high losses.
2. The **Operations** policy scores available options: frontal attack, siege, diversion, airborne attack, covert infiltration, reconnaissance first, or defer.
3. If a covert option has high expected value, Operations submits a **capability request**: e.g., reach the installation, disable it temporarily, and survive/escape if practical, before a useful deadline.
4. **Strategy** accepts, denies, or postpones the commitment based on economy, global timing, and competing priorities.
5. **Quartermaster** evaluates technology and production options, provides composition/ETA alternatives, and schedules an approved build.
6. **Operations** forms and assigns the force; **Platoon** designs the infiltration; **Squads/unit micro** handle movement, concealment, targeting, and ability timing.
7. If the mission is infeasible or unexpectedly valuable, feedback moves upward and can change the commitment.

No unique top-level **SpecOps brain** is required. Initially, give Operations a spec-ops/recon mission option and give the appropriate squads specialized policy heads/reflexes; add a specialist controller only if tests reveal a real need.

### 4.3 Example: securing high ground

1. A reusable **terrain opportunity service** evaluates candidate regions using elevation, weapon-specific range/ballistic effects, LOS/radar implications, approachability by unit class, chokepoints, cover, fortification opportunities, enemy fire envelopes, and connection to valuable objectives.
2. Operations selects/declines candidate terrain objectives in light of wider plans and available forces.
3. Platoons select approach routes, staging areas, fallbacks, artillery positions, and screening geometry.
4. Squads select a suitable formation mode and use its deterministic executor to make weapon- and mobility-specific adjustments within assigned sectors: indirect-fire units, short-range assault units, anti-air, scouts, and engineers may have different optimal positions. The selector need not calculate individual firing or movement positions.
5. Platoons report when the hill is impractical to occupy or no longer worth the expected cost.

Do **not** bake in “higher is always better.” Elevation can help one weapon system and hurt another, and accessibility/exposure may dominate. Verify actual BAR/Recoil mechanics, especially projectile trajectories, range and LOS, rather than trusting a simplistic height bonus.

### 4.4 Other opportunity services

Use candidate-generation services rather than adding a neural network per topic:

- **Recon/intelligence priorities:** likely enemy expansion, missing tech information, stale contacts, probable hidden defenses; value of information versus scout loss.
- **Wreck/reclaim opportunities:** recoverable metal, threat exposure, enemy access, and denial value.
- **Defensive coverage:** radar, jammer, anti-air, anti-nuke, shield, anti-artillery, and repair requirements.
- **Strike feasibility:** target value and time sensitivity, approach corridors, defensive saturation, range and cooldowns.
- **Reinforcement and reserves:** expected time-to-contact, composition gaps, capability redundancy, delivery and assembly locations.

Controllers evaluate proposed opportunities and allocate resources; the services compute mechanics and features.

---

## 5. Shared world model and sensor design

A neural policy should not directly consume every visible unit in an arbitrary array. Build a shared, **observation-legal**, versioned world-state layer that can emit appropriately shaped observations per tier.

### 5.1 Common world-state services

- Friendly unit catalog: location, orders, weapon/ability stats, health, composition, available builders/factories.
- Enemy contact/belief registry: visible observations, radar-only data, last-seen times, possible identity, uncertainty and threat estimates; no omniscient enemy unit data.
- Spatial maps: passability by unit class, elevation, LOS, radar, likely threat, frontlines/sector control, reclaim, candidate staging areas.
- Economics: metal/energy income, spending/pull, storage, build power, forecasted job completion, wind/tidal/environmental production as game-appropriate.
- Capability registry: what a composition can *do* (range bands, anti-air, stealth, burst damage, scouting, repair, transport, demolition, mobility, etc.), not just its nominal cost.
- Mission history and state: current orders, deadlines, previous decisions, observed progress, cooldowns and recent failures.
- Action feasibility: prerequisites, reachable regions, ownership of units/resources, and legal command masks.

### 5.2 Per-tier feature families

**Strategy:** global trend and risk summaries, economy and technology posture, enemy-strategy beliefs, map-sector control, available capability aggregates, operational requests, production forecasts, and strategic event history.

**Operations:** coarse sector graph/grid; known and suspected enemies; own force location/capabilities/readiness; local economic and terrain value; supply/reinforcement ETAs; uncertainty; current theater missions and priorities; candidate opportunities.

**Quartermaster:** actual resource stocks/flows; current and projected demand; production/build power; factory/constructor status and locations; tech prerequisites; per-job priorities; authorized requests, delivery deadlines, material and energy costs.

**Platoon:** mission vector and loss allowance; component squad roles/stats; nearby friendly/enemy/uncertain contacts; local threat/terrain features; weapons/range envelopes; relative locations of objectives, cover, retreat routes, neighbors, and reclaim.

**Squad:** assigned subgoal, authority and formation envelope; role/unit-stat features; effective range and mobility matchups; platoon-relative geometry and sibling support; legal threats/targets and contact uncertainty; local terrain, retreat room and accessible approach sectors; firing obstruction and useful firing participation; unit health distribution; current formation, time in mode, progress and recent blockers. Weapon cooldown/accuracy/trajectory, movement/slope limits and ability features should inform deterministic execution or compact selector features where useful. The selector consumes the consequences of these mechanics rather than reproducing them through learned per-unit geometry.

**Unit reflex:** very local targets/projectiles/obstacles and immediate status, not a global strategy observation.

### 5.3 Encoding recommendations

- Use normalized bounded ratios where natural, and log or signed-log transforms for values spanning early-to-late game scales. Scale distances/ranges relative to map dimensions, movement/range capability, or an appropriate local radius.
- Use relative coordinates to an objective, platoon anchor, and threat axis for transferable tactical geometry. Preserve absolute map features only where strategically meaningful.
- Use a small fixed regional map, pooled entity features (e.g., Deep Sets), or spatial/graph models as appropriate. Compare these experimentally; a pure flat MLP is not inherently optimal for variable force and sector counts.
- Encode contact recency and uncertainty explicitly. Never treat a last-known enemy position as guaranteed current truth.
- For a feed-forward policy, include useful derivatives and short-term trends (e.g., velocity, incoming damage, resource acceleration, recent enemy movement). Consider recurrence if history still materially improves performance.
- Include the **current assigned mission**, its constraints, and recent parent feedback in the observation; otherwise a learned controller cannot be meaningfully goal-conditioned.
- Use hard masks for truly invalid actions. Do not mask merely **undesirable** but legal sacrifices, tech choices, attacks, or retreats: those are judgment calls for the hierarchy.
- Start with justified feature groups, log their distributions, then use ablation, mutual usefulness, and robustness tests for feature selection. Do not apply a universal correlation threshold or insist on a specific sensor count.

### 5.4 Separation of world model and decision model

Most tiers can share intelligence, pathing, terrain analysis, and effect estimators without sharing policy weights. A shared world model is **not** an omniscient tactical oracle: every consumer must only receive fields that its command scope and game observability permit.

---

## 6. Action design and network policy boundaries

### 6.1 Prefer parameterized high-level options

Examples:

- Strategy: `COMMIT_TECH`, `PRIORITIZE_FRONT`, `AUTHORIZE_RAID`, `EXPAND_ECONOMY`, `DEFENSIVE_POSTURE` with budget/priority parameters.
- Operations: `ASSIGN_MISSION(platoon, mission, region, deadline)`, `REQUEST_CAPABILITY(effect, deadline)`, `RESERVE_FORCE`, `COORDINATE_ATTACK`, `SCOUT_REGION`.
- Quartermaster: `SCHEDULE_JOB`, `ADJUST_BUILD_POWER`, `SELECT_COMPOSITION`, `SET_PRODUCTION_PRIORITY`, `ASSIGN_CONSTRUCTOR`; exact placement generated and validated by separate services.
- Platoon: `ADVANCE`, `HOLD`, `CONTEST`, `FLANK`, `KITE`, `WITHDRAW`, `SPLIT`, `REGROUP`, with objective, route, formation/posture parameters.
- Squad: `SELECT_FORMATION(CHARGE / HOLD / ORBIT / ENCIRCLE / KITE)` or `CONTINUE`, within the assigned target/anchor/sector and mission authority. Initially derive width, spacing, engagement distance, orbit direction and other parameters from deterministic presets or the parent assignment. Bounded preset selection or parameter refinement is a later experiment, not the first action space.
- Unit: `DODGE`, `ATTACK`, `MOVE`, `RETREAT`, `SELF_DESTRUCT`, or ability execution, subject to valid-unit and safety constraints.

Policies may select **candidate** regions, routes, composition plans, and target entities supplied by deterministic or learned proposal systems. This is usually easier to train and validate than arbitrary continuous world-space coordinates.

### 6.2 Formation-first squad control: select the behavior, calculate the geometry

**Preferred initial architecture:** a compact formation selector above a fixed deterministic execution library. The selector may begin as a heuristic and later become a shared role-conditioned MLP. Its first responsibility is to choose the right mode for the current subgoal and local situation, including when to continue it. It is not initially a general movement, targeting, spacing, and ability network.

The project owner reports that very simple formation commands are already difficult to beat in the tested encounters. Treat that as the motivation for this narrower learning problem, not as a verified claim that these five modes are sufficient or optimal for every BAR situation. Preserve and reproduce those baselines before replacing them.

#### Initial deterministic formation library

Here, **formation** is shorthand for a complete tactical movement behavior, not just a static shape. Reuse the existing tested formulas where available; the descriptions below define intended responsibilities, not new verified engine-command implementations.

| Mode       | Deterministic execution responsibility                                                                                                                           | Situation-dependent choice left to the selector                                                       |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `CHARGE`   | Close toward the assigned target/contact region, respecting the movement envelope, spacing rules and unit-appropriate engagement distance.                       | Whether closing now serves the assignment and is worth the exposure.                                  |
| `HOLD`     | Maintain the assigned anchor or sector, with stable spacing, permitted firing and local corrective movement.                                                     | Whether preserving this position or delaying movement is preferable to advancing or giving ground.    |
| `ORBIT`    | Combine tangential movement around a target/reference with radial correction toward a preset fighting distance; preserve direction unless a change is justified. | Whether lateral motion is useful and compatible with terrain, support and mission constraints.        |
| `ENCIRCLE` | Assign accessible arc/ring positions around the target group and move toward them with spacing and range correction; retain stable assignments where practical.  | Whether gaining separated attack directions justifies travel, exposure and temporary loss of support. |
| `KITE`     | Maintain a favorable engagement-distance band, moving away from closing threats or restoring useful firing distance along permitted routes.                      | Whether the effective range/mobility matchup and retreat space make kiting useful for this subgoal.   |

These are closed-loop controllers: deterministic does **not** mean replaying a fixed movement sequence. Recalculate local corrections from current legal observations and execution state. Use shared terrain/path/weapon services, stable slot assignment, bounded candidate searches, and command scheduling; measure their cost rather than assuming formulas or group orders are free. An encircle controller may use an accessible arc instead of insisting on a full circle. A kite controller may exploit favorable relative elevation when the verified weapon geometry and permitted route support it; do not hard-code an unconditional uphill bonus.

Keep firing/target choice, ordinary spacing, and reflex thresholds scripted initially, within parent priorities. Supply deterministic transit, regroup, withdrawal and no-target/blocker handling outside the five-mode combat selector as required by the mission. Do not force an invalid combat mode just to keep a five-output interface, or let a hidden fallback go unreported.

#### Ownership, persistence and feedback

The **Platoon** still decides the combined-arms plan: squad roles, target priorities, assigned sectors/anchors, support relationships, coordinated commitment timing, and loss authority. The **Squad** chooses its mode within that assignment. Local formation selection must not silently turn screening into an unsupported pursuit or independently revise the platoon's mission. For a tightly synchronized maneuver, the parent may constrain or prescribe a mode; the squad still reports infeasibility through the normal protocol.

A squad decision records the mode, task/target/anchor reference, deterministic preset version and selection time. The executor retains slot assignments, orbit direction and other mode state. Selecting the same mode or `CONTINUE` must not reset that state. Minimum dwell time and switching hysteresis should allow maneuvers to develop, with immediate handling for cancellation, actual invalidity or permitted emergencies. Reports distinguish **selected** from **executing effectively**, including inability to maintain range, blocked approaches, lost support and insufficient progress.

A mission named `HOLD` does not require the `HOLD` movement mode: orbiting or kiting inside the allowed region may fulfill the mission better. Likewise, `CHARGE` can be a useful approach phase without being the right behavior for an entire engagement. Do not permanently ban either because it performs poorly in some deathmatch tests. The five labels are intentionally a small first action space; combinations such as kiting within an encircling sector can be added later only if they address a measured limitation.

#### Shared selectors before specialist heads

Use unit statistics, role and mission conditioning for a shared selector where action semantics are compatible. Preserve these role families:

- Bruiser/assault and protective screen
- Standoff skirmisher and artillery
- Raider/flanker/swarm
- Anti-air, shield, jammer and electronic warfare support
- Engineer, repair, reclaim and logistics
- Aircraft (possibly separate fixed-wing and gunship behaviors)
- Recon, infiltration and special operations

The five-mode combat library is a starting point for applicable combat squads, not a forced vocabulary for repair, reconnaissance, aircraft or every special ability. Keep role-appropriate scripts under the same mission/report interface; extend the library or add specialist heads only when evidence shows a missing behavior.

Squads may contain homogeneous unit types or compatible functional groups; do not insist on a rigid one-unit-type definition. Small elite groups and single-unit groups should fit the same mission/report protocol. Homogeneous, spatially coherent groups are a useful first test case; mixed groups need presets that respect their actual mobility and weapon differences rather than an unrepresentative average.

**Do not decide “seven separate MLPs” in advance.** Compare shared selectors and specialist heads only after testing whether learned mode selection improves on the heuristic selector. Keep deterministic execution as the default even when selection is learned. See §8.2 for the training and promotion gates.

### 6.3 Preserve selective per-unit learning

Start with deterministic squad formation execution and deterministic per-unit emergency reflexes. Preserve the option to use a parameter-shared per-unit policy for abilities, projectile dodging, focus-fire, precision kiting, or self-destruct timing, but test it as a separate extension after the fixed formation library and selector have established their baseline. Inference cost, pathfinding cost, and *actual command* cost must be measured separately.

A group policy may evaluate 20 units at once but issue only a few stable commands. A per-unit policy may evaluate frequently but issue a new order only when an action threshold is crossed. Do not conflate “one policy evaluation per unit” with “one new path per unit every few frames.”

### 6.4 Game-mechanics solvers remain independent

Use dedicated deterministic or approximate simulation tools to evaluate:

- Terrain access, travel-time/path candidates, LOS and weapon positioning
- Building placement, radar visibility and construction footprint legality
- Effective build throughput and resource forecasts
- Anti-nuke interception **throughput and stockpile**, not just circular coverage
- Shield charge/regeneration versus expected incoming projectiles
- Projectile impact timing, elevation/range effects and likely line of fire
- Reclaim value and enemy wreck-control risk
- Self-destruct delay, explosion geometry, wreck behavior and friendly-fire effects

The last items require game-accurate validation. Simplified fixed-formulas in the source PDF are illustrative, not verified BAR mechanics.

---

## 7. Training strategy: overall approach

### 7.1 Three tracks to develop in parallel

**Track A — Functional deterministic hierarchy:** Implement the interface and a playable rule-based baseline, including the five deterministic combat formations and a mission-aware heuristic selector. This proves that the decisions, action timing, and communication make sense *before* training becomes a confounding factor.

**Track B — Focused learned components:** Train one component at a time in small, reproducible environments, initially against scripted peers, then a diverse league of opponents. For squads, learn formation selection with execution frozen before considering parameter refinement or learned geometry.

**Track C — Measurement/fidelity:** Maintain separate held-out scenario suites, BAR-engine validation runs, replay/debug traces, profiling, and tests for simulator mismatch and illegal information leaks.

These tracks should share versioned observation, order, report, action, and metric schemas.

### 7.2 Candidate learning methods

Treat the choices below as **research candidates**, not decisions made by this document:

- **Supervised learning / behavioral cloning:** warm-start responses from scripted experts, generated optimal/near-optimal planning decisions, and carefully reconstructed human replay actions. Human replays often show *commands*, not labeled high-level intents; mission labels require inference or annotation.
- **PPO with GAE (actor-critic):** sensible baseline for goal-conditioned local combat and operations policies, provided the simulator can generate enough trajectories. Investigate alternatives where action spaces or rollout costs make PPO inefficient.
- **Hierarchical/options RL:** appropriate abstraction for actions lasting multiple seconds; termination conditions and the elapsed time between decisions should be included in returns. For variable-duration actions, investigate Semi-Markov discounting such as `gamma^(duration)`.
- **Centralized training, decentralized execution:** potentially valuable for cooperating squads/platoons; keep execution observations legal. Investigate parameter sharing, centralized critics, and multi-agent credit assignment only where needed.
- **Evolutionary policy search:** retain as a benchmark or complementary tool, particularly if an existing custom battle simulator and evolutionary training workflow are already functioning. It may be useful for small policies, strategic curricula, fitness search, architecture/reward search, or teacher generation. Compare sample efficiency and compute cost to gradient-based learning rather than assuming evolution or backpropagation is always superior.
- **Optimization and model-predictive planning:** attractive for deterministic resource/production scheduling and tactical geometry. Train a policy to choose priorities/candidates only if that beats an adequate solver.
- **Preference labels/human feedback:** collect a few reviewed match clips each campaign and use them to define failure cases, additional scenario generators, rankings, corrections, or a cautious preference/reward model. Keep human “sanity” judgments separate from win/loss data and guard against inconsistent subjective supervision.

### 7.3 Modular training procedure

1. Define a **contract and measurable goal** for one module.
2. Supply randomized goals from a scripted or synthetic parent, including deliberately difficult, conflicting and infeasible demands.
3. Supply reliable scripted/frozen children and reusable scenario initial states.
4. Train on progressively harder scenarios with domain randomization across maps, terrain, sides, compositions, resource states and enemy styles.
5. Test on held-out distributions, unseen maps and unit mixes. Ensure the policy learns the goal rather than exploiting a narrow reward proxy.
6. Add interfaces for meaningful feasibility reports, constraint/authority handling, and requests.
7. Integrate with one adjacent controller while weights are frozen; test that their distributions and semantics actually match.
8. Unfreeze selectively after the full system is stable; keep regression suites for every earlier capability.

### 7.4 Reward design rules

- Make **mission achievement and strategic value** primary. Metal efficiency is secondary and context-dependent.
- Separate task reward from hard constraints and from diagnostic metrics. For example, loss of protected units may be a constraint, not merely a small penalty.
- Do not reward pure spending or zero resource stall as absolute goals. A brief stall or intentional sacrifice might be optimal for an urgently needed defense.
- Avoid training a platoon to abandon every dangerous objective simply because survival reward dominates.
- When using potential-based shaping with discount `gamma`, the standard transition term is `gamma * Phi(s_next) - Phi(s_current)`; check how this generalizes to variable option duration and terminal states.
- Log each reward term and compare ablations. Beware reward hacking (kiting forever, scouting empty zones, endlessly producing units, capturing irrelevant regions, over-requesting reinforcements).
- Preserve sparse terminal objective/win signals where possible, then use carefully tested intermediate rewards to improve credit assignment.
- Design a way to attribute failures to a parent’s unrealistic plan versus a subordinate’s poor execution. Frozen competent children, diagnostic forecasts, and controlled counterfactual scenarios help.

---

## 8. Per-component training research specifications

These are **starting hypotheses for the next agent to investigate**. The exact observation tensors, network widths, optimizers, reward scales, and hyperparameters are open research tasks.

### 8.1 Unit execution / optional micro policy

**What it needs to learn (if anything):** avoid immediate avoidable damage, maintain safe firing geometry, select/execute targets, retreat within squad rules, use special abilities (including self-destruct) at tactically sensible times.

**Sandbox:** Small engine-accurate encounters: dodgeable projectiles, slow cannons, mixed units, elevation changes, target obstruction, repair/reclaim opportunity, self-destruct lead times, friendly-fire hazards.

**Inputs:** local legal observations, unit/weapon dynamics, squad instructions and risk tolerance, position-relative threats, projectile time-to-impact estimates.

**Actions:** small set of eventful corrections/target or ability decisions; fallback to engine movement/pathing controls.

**Training:** scripted reflex baseline; supervised target/dodge examples if available; then short-horizon RL or evolutionary search for the cases where learned control materially helps. Investigate batching and per-unit parameter sharing.

**Evaluation:** damage avoided at comparable damage output, survivability, successful ability timing, collateral damage, enemy wreck denial, orders issued per unit, CPU/frame load, pathing disruption, generalization to unfamiliar weapons.

**Gate:** Keep deterministic implementation unless learned micro produces repeatable gains **without** unacceptable inference or command cost.

### 8.2 Squad controller

**Goal:** fulfill one platoon-assigned subgoal by **selecting, retaining and switching among simple deterministic formations**. The first learned decision is `CHARGE`, `HOLD`, `ORBIT`, `ENCIRCLE` or `KITE`, not the individual unit motions that implement it. Preserve screening, support positioning, target priorities and mission authority through the assigned envelope and deterministic execution rules.

**Design basis:** The project owner's reported tests find these basic commands highly effective and difficult to outperform. Earlier discussion particularly favors encircle for tested brawlers/raiders, kiting with an exploitable friendly range advantage, and orbit as a fallback. These are provisional, matchup-dependent starting preferences. Benchmark data and simulator/engine agreement have not been independently checked for this revision; do not infer a universal ordering or eliminate charge/hold.

**Sandbox:** Brief reproducible engagements, initially using applicable single-type combat squads and fixed scripted parents/siblings. Vary effective range and mobility matchups, force ratios, starting geometry, terrain/obstructions, accessible arcs, retreat space, sibling support, mission constraints and enemy behavior. Include changing threats within an encounter so retaining versus switching modes can matter. Add high-ground cannon cases only with validated mechanics, then imperfect reconnaissance and more varied roles.

**Inputs:** subgoal and authority; assigned target/anchor/sector; roster/stat and health summaries; legal local contacts and uncertainty; effective weapon/range and mobility relationships; terrain, firing and retreat geometry; sibling support; current mode, elapsed time, actual progress and blockers. Derive mechanical quantities through shared services. Distinguish different threat groups rather than treating a mixed-range enemy force as one average opponent.

**Outputs:** initially a discrete mode choice or `CONTINUE`. Mode-specific distances, widths, spacing, directions, target execution and reflex thresholds come from fixed role/stat-conditioned presets and the parent assignment. Feedback reports the active mode, contribution, feasibility, constraints and blockers. Target/preset selection or a few bounded parameter adjustments may be added later as separate experiments; arbitrary per-unit waypoints and learned encirclement geometry are not initial outputs.

**Training:**

1. **Reproduce the execution baselines.** Implement or reuse all five deterministic modes; test state persistence, missing targets, blocked routes and mission-aware fallbacks. Freeze and version their formulas/presets and the reflex layer for selector comparisons.
2. **Establish selection baselines and headroom.** Evaluate each fixed mode where applicable and a strong mission-aware heuristic selector on matched encounters. Compare outcomes by matchup and objective. Repeated runs of alternative modes from matched initial states can indicate whether selection has value; any hindsight “best mode per encounter” score is an offline diagnostic, not a deployable policy or proof of an optimal switching sequence.
3. **Warm-start judgment, optionally.** Use trusted heuristic choices as supervised examples, or use finite state-dependent heuristic scores plus an MLP correction. Preserve room to try legal alternatives; a preference for surround is not a reward for drawing a circle. Reported tactical rankings guide initialization, not hard action bans.
4. **Train selection only.** Compare a small shared role/stat-conditioned MLP trained by goal-conditioned RL with the existing evolutionary approach where practical. Use the same observations, actions, priors, fixed executors, missions and evaluation budget. Start with fixed decision intervals and retained mode state; account explicitly for action duration before moving to event-driven learning.
5. **Validate before expanding.** Add varied missions, siblings, uncertainty and terrain while retaining earlier regression cases. Use clear human corrections to label mode choices; route execution bugs to the deterministic controller instead of teaching the selector that the entire mode is bad. Only after demonstrating a remaining limitation, test bounded parameter/preset choices or an additional composite mode. Learned per-unit execution remains a separate §8.1 experiment.

**Reward:** assigned subgoal achievement and timing first, with losses interpreted through parent authority and useful contribution to sibling squads. Judge screening by protection/delay, not only personal damage. Track firing efficiency, traffic obstruction, objective abandonment and unnecessary switching as appropriate. Do not reward a mode label, perfect circle, constant motion or survival regardless of mission.

**Evaluation:** best fixed applicable mode, heuristic switching, and learned selection must use the **same deterministic formation library and reflexes**. Evaluate held-out maps, matchups, objectives and opponents; test the same physical situation under different legitimate missions. Measure mission success, authorized costs, support coverage, firing participation, range maintenance, mode duration, switching and recovery behavior, orders issued, selector inference, executor geometry cost and pathfinding impact separately. Validate relevant tactic rankings in actual BAR as well as the custom simulator.

**Gate:** Promote a learned selector only for a repeatable, practically useful improvement over the strongest matched heuristic selector, without unacceptable mission regressions or runtime/command cost. It must respond to changed goals with the same weights. If selection adds little, retain the heuristic and proceed to Platoon/Operations learning; competent deterministic squads are sufficient children. Do not require learned formation geometry, parameter heads, or per-unit MLPs to advance the hierarchy.

### 8.3 Platoon controller

**Goal:** turn a mission contract into combined-arms execution while managing squads, terrain, engagement posture, risk, and upward requests.

**Sandbox:** 1–3 minute scenarios with premade combined-arms forces, multiple approach routes and terrain, meaningful objectives (capture/hold/delay/raid/escort/recon/withdraw), enemy reinforcements, and constraints on acceptable losses.

**Inputs:** full mission contract, roster/capabilities, predicted opponent threat, terrain candidates, local weapon geometry, nearby allies, availability of support and mission state.

**Outputs:** tactical options, routes/positions, per-squad roles and sectors, support relationships, coordinated commitment timing, target priority, staged withdrawal/reinforcement requests, progress/feasibility report. Delegate formation selection within explicit constraints; prescribe a mode only where the coordinated plan requires it.

**Training:** use competent frozen Squad controllers: deterministic formation executors with heuristic or previously validated learned selectors. Do not require learned squads before training the platoon, and do not train the platoon to calculate their per-unit geometry. Randomize **legitimate and impossible missions**; train not only execution but appropriate escalation. Use goal-conditioned RL; add mission examples and preferences from reviewed battles as supplementary supervision.

**Reward:** mission value and completion, timing/deadline, constraint adherence, controllable losses, support synergy, preservation of important assets when ordered. Penalize pointless replan/order churn. Do not let raw metal-trade efficiency override sacrificial `DELAY` missions.

**Evaluation:** success by mission type; effect of accepting/rejecting unrealistic orders; precision and calibration of feasibility reports; use of cover/high ground; coordination; retreat quality; robustness to new terrain and enemy counters.

**Gate:** Demonstrates at least two genuinely different strategies for the same map/force under different mission goals, plus a useful “cannot accomplish without X” report.

### 8.4 Operations Commander

**Goal:** decide **where the army should fight, which objectives are valuable, which force accomplishes each task, when to request capabilities, when to use reserves, and when to change plans**.

**Sandbox:** A multi-sector battle on reduced maps with existing platoons, changing intelligence, critical terrain, fronts that can fail, contested resources, and a scripted capability-delivery service. Introduce multiple simultaneous objectives before coupling live production.

**Inputs:** strategy intent/budgets, sector/candidate graph, threat and uncertainty maps, terrain opportunity estimates, platoon capability vectors, mission outcomes, reserve options, delivery ETA/cost offers, economic/tech context at an appropriate summary level.

**Outputs:** mission assignment and timing, objective ranking, coordinated plans, sector priorities, reserves, recon/spec-ops/reinforcement capability requests, mission cancellations/revisions.

**Training:** start with rule/heuristic sector allocation; score a bounded set of candidate operations/actions. Train using multi-objective scenarios and scripted/competent platoons; then varied opponent strategies and partial-observation self-play. Compare flat MLP to spatial or graph/entity encoder if variable front and force sizes justify it.

**Reward:** control of **valuable** objectives over time, strategic effects achieved, force preservation versus authorized cost, timeliness of needed capabilities, exploitation of opportunities, information value, reduced catastrophic unguarded leaks, and continuity of sensible plans. Penalize useless requests and constant reassignment.

**Evaluation:** sector win rates, response to new vulnerabilities, value of high-ground selection, selection of spec-ops versus conventional attacks, prediction of force insufficiency, requisition quality, multi-front performance, and quality of responses to subordinate objections.

**Gate:** In held-out cases, chooses critical terrain or capability requests that a flat “nearest enemy / biggest army” heuristic misses and productively adapts when a platoon reports infeasibility.

### 8.5 Quartermaster (coupled economy + factory production)

**Goal:** satisfy strategic spending objectives and operational capability requests while handling BAR's streaming economy, build power, technology prerequisites, construction travel, and tactical urgency.

**Sandbox:** Economic/production scenarios, initially without enemies but with diverse map economies, changing wind/resources, reclaim windfalls, constructor losses, factory damage, technology restrictions, deadlines and urgent force requests. Then introduce raiding/territory constraints and live logistics.

**Inputs:** metal/energy stocks and flows, current and forecast demand, build powers, factory/constructor locations and queues, resources and territory available, tech tree, authorized budgets, production/capability requests, delivery time/risk, defensive emergency status.

**Outputs:** feasible composition offers, production and construction task assignments, queue order, build power/priority allocations, economic investments, technology jobs, delivered-ready forces, revised ETAs, and blockers.

**Training:** implement a deterministic scheduling/forecast baseline first. Consider a learned policy for choosing marginal spending priorities, build power, and available candidate build/placement plans; supervised imitation from optimizer/teacher and RL in randomized economy sandboxes. Keep strict legality and budget ownership in the scheduler even if priorities are learned.

**Reward:** value of requested capabilities **delivered on time**, useful economic expansion, avoidance of waste and avoidable stalls, responsiveness to emergencies, and accurate feasibility estimates. Compute strategic value using goals/deadlines rather than equating metal spent with metal well spent.

**Evaluation:** delivery success/on-time rate, composition fitness for requested effect, metal/energy utilization, resource stall cost, builder/factory idle time, vulnerability of construction plans, ability to handle impossible requests, ETA calibration, robustness to raids and economic shocks.

**Gate:** Reliably delivers real capability requests, refuses or revises impossible commitments, and performs at least as well as a strong rule-based economy benchmark under diverse randomized demand.

### 8.6 Strategic Director

**Goal:** select broad objectives, long-term investments, risk postures, production budgets and commitments that maximize team victory probability over a full match.

**Sandbox:** Full games with frozen trained Operations/Quartermaster/Platoon/Squad policies or competent scripted substitutes. Start with reduced strategic choices and progressively harder economies, maps, opponents, tech transitions, hidden enemy strategies and long-game threats.

**Inputs:** broad economic and military forecasts, map/theater outcomes, inferred enemy strategies, tech and timing windows, capability inventory, projected production, uncertainty, major defensive deficits, reports from Operations and Quartermaster.

**Outputs:** priority goals and mission intent, resource envelopes and authorization, macro doctrine (eco/tech/army/defense emphasis), decisions to invest in new capabilities, expected time horizons and risk tolerance.

**Training:** script-first macro teacher; consider replay-informed imitation if macro intent can be reconstructed; full-game RL with checkpoints/opponent pools and diverse scenario resets. Train with lower policies frozen at first. Explicitly test rare late-game crises rather than expecting ordinary full games to produce enough examples.

**Reward:** primarily win/loss or long-horizon strategic outcome, with carefully validated shaping tied to genuine advantage and mission progress. Avoid rewarding raw army/economy value so strongly that the policy turtles indefinitely, refuses necessary attacks, or overbuilds exposed infrastructure.

**Evaluation:** win rate with confidence intervals against diversified opponents, adaptive tech/counter behavior, time to secure decisive map control, catastrophic risk handling, reaction to operational requests, long-match competence and cross-map/faction generalization.

**Gate:** Consistently improves full-game outcomes over a fixed strategic baseline with identical lower controllers and no extra observation privileges.

---

## 9. Recommended build and training sequence

The stages below are ordered to get an **observable end-to-end result quickly** while reducing the cost of debugging. Quartermaster work can proceed in parallel once the contracts are stable.

### M0 — Verify mechanics and make a small feasibility prototype

**Build / investigate:**

- Confirm BAR/Recoil headless or accelerated simulation and automation; determinism/replay fidelity, Lua/native integration constraints, data capture, and command/observation APIs.
- Benchmark simulation throughput and neural inference/command budgets rather than assuming arbitrary frame rates.
- Validate partial-observation access and all game mechanics needed by the first tactical tests.
- Inventory existing custom simulation/training code and compare its action semantics and physics against actual engine encounters. Reproduce the reported charge/hold/orbit/encircle/kite baselines and check relevant tactic rankings, not only aggregate casualties.
- Choose a small first game format (e.g., 1v1 tactical scenarios), with a documented extension path toward team games.

**Done when:** A reproducible scenario runner can set initial states, observe legally, issue commands, collect rewards/status, and replay the same encounter for debugging.

### M1 — Define contracts, world state and instrumentation

**Build:** Typed mission/order/report schemas; capability-request/offer schema; event log; mission lifecycle; unit ownership and resource ownership; observation feature registry; action legality masks; action scheduler with rate limits; metrics and trace viewer.

**Done when:** A wholly scripted hierarchy can issue a mission, receive an objection, change/cancel an order, and finish the scenario without conflicting controllers or illegal information.

### M2 — Create deterministic tactical and economic baselines

**Build:** Scripted unit reflexes; a versioned deterministic charge/hold/orbit/encircle/kite library with fixed role/stat-conditioned presets; a mission-aware heuristic formation selector; role-appropriate support scripts; platoon mission execution; basic theater/sector allocation; economic throughput scheduler and production planner; initial terrain, LOS and path candidate services.

**Done when:** The system handles a handful of battle objectives and can meet basic production requests **with zero learning**. Reproduce each applicable fixed-mode baseline and the heuristic selector. Record baseline outcomes, mode persistence/fallback behavior and separate selector, executor, CPU/pathfinding and command costs.

### M3 — Establish squad formation selection; train only where useful

**Build:** Tiny randomized combat sandboxes, paired mission goals, scripted parents/siblings, mode-selection traces, per-role metrics, and train/eval splits by map, weapon and composition. Freeze the formation library/reflexes. Compare the best fixed modes, heuristic selection and an optional small learned selector before adding parameter heads or learned execution. Use trusted teacher choices or human corrections for an optional warm-start.

**Done when:** Squads fulfill varied subgoals reliably using the fixed formation library. Promote learning only if selecting/retaining/switching modes improves held-out mission outcomes enough to justify its cost, without serious regressions. Otherwise keep the heuristic selector and move on. Matching a simpler controller is not, by itself, a reason to replace it. Optional parameter or micro experiments must demonstrate gains separately and are not prerequisites for M4.

### M4 — Train platoons on mission contracts

**Build:** Combined-arms tactical arenas with frozen competent formation-based squads; multiple feasible routes; objective-specific success/termination; infeasible missions, reinforcements and retreats; upward reports. Test complementary squad modes within a coordinated assignment, not independent mode choices that defeat screening or synchronization.

**Done when:** A platoon both completes realistic assigned missions and correctly escalates difficult/impossible missions; its behavior changes as the superior's risk and intent change.

### M5 — Train Operations using prebuilt forces

**Build:** Reduced multi-sector campaign scenarios; strategic objectives provided by a script; deterministic terrain-opportunity service; force/capability inventory; dummy Quartermaster that offers predefined forces with costs/ETAs.

**Done when:** Operations chooses meaningful objectives and force assignments, recognizes missing capability, proposes specialized missions (including recon/spec-ops), and revises assignments based on platoon reports.

**This is the preferred first full vertical slice:** Operations → Platoon → Squad, using supplied forces rather than a fully trained economy.

### M6 — Connect Quartermaster, then train production priorities

**Build:** Real capability catalog and force-assembly pipeline, budget arbitration, realistic production queue scheduler, economic shock and emergency scenarios, force delivery interface to Operations.

**Done when:** An operational need can travel all the way from an intelligence observation to a capability request, resource authorization, production, assembly, mission assignment, and report of outcome. ETA and feasibility errors are visible to all consumers.

### M7 — Train Strategic Director in full games

**Build:** Scripted global macro teacher; strategically meaningful option set; full-game self-play/opponent pool; global reward and reporting; varied map/economic starts; tech and late-game crisis scenarios.

**Done when:** The Strategic policy chooses and funds useful objectives, responds to real Operations/Quartermaster feedback, and beats controlled baselines across a meaningful evaluation set.

### M8 — Joint adaptation and scaling

**Build:** Integration regression suites; module freezing/unfreezing; self-play league using past policies and scripted/exploiter opponents; expanded team-game support; profiling and inference batching; rare-event scenario injection.

**Done when:** Co-adaptation improves full-game outcomes without losing previously verified squad/platoon/economy competence, command cost stays within engine budget, and results hold up against opponents and maps not used in training.

### Build priority if resources are tight

1. **Mission/report protocol and instrumented sandbox**
2. **Squad + platoon tactical competence**
3. **Operations and terrain/opportunity selection**
4. **End-to-end capability requests + Quartermaster**
5. **Strategic full-game training**
6. Advanced unit micro, complex joint RL, and late-game specialist refinements

The exact development order of M5/M6 can swap if the existing project already has a strong economic controller. Preserve the **interface dependency**, not a rigid calendar.

---

## 10. Required evaluation scenarios (regression suite)

Every proposed model should be run against consistent seeds, held-out map/force configurations, and deterministic baselines. The following scenarios must exist before declaring the system integrated.

| Scenario                                  | What it demonstrates                                                                                                               | Critical checks                                                                                                                        |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| **Formation-selection baseline**          | Fixed charge/hold/orbit/encircle/kite, heuristic selection and learned selection share identical executors                         | Does selection improve mission results rather than benefit from changed movement code? Are reported tactic rankings reproduced?        |
| **Mode persistence and recovery**         | Threats move, a route becomes blocked, or the target disappears during a maneuver                                                  | Does it retain useful state, switch for a reason, use a legal fallback and report blockers without order thrash?                       |
| **Mission versus movement mode**          | Identical forces receive hold, delay, destroy or withdraw assignments with different authority                                     | Does it choose modes for the goal, rather than equating a HOLD mission with standing still or kiting regardless of objective?          |
| **Complementary squad modes**             | One squad screens or pressures while another kites or encircles within the same platoon plan                                       | Are support, timing and sector constraints preserved? Can a local selector report infeasibility without silently changing the mission? |
| **High-ground opportunity**               | Operations identifies a valuable ridge and assigns an appropriate force; platoon/squads choose weapon-specific positions           | Does it understand range/LOS, path access and opportunity cost? Is high ground sometimes correctly ignored?                            |
| **Need spec-ops**                         | Enemy high-value installation is costly to assault frontally; Operations proposes infiltration/recon; Quartermaster offers options | Is the request effects-based, feasible, budgeted and deadline-aware? Does the AI ever choose a non-spec-ops solution?                  |
| **Suicidal hold / subordinate objection** | Platoon reports likely failure and offers withdrawal; Operations either revises or authorizes sacrifice                            | Is local efficiency subordinate to mission intent? Are overrides and deadlines obeyed?                                                 |
| **Missing counter capability**            | A force encounters an enemy it cannot counter, e.g., AA/anti-armor shortage                                                        | Does it avoid futile engagement and request the appropriate capability?                                                                |
| **Economy stress**                        | Sudden wind collapse, lost mexes, extra constructors, urgent defensive request                                                     | Does Quartermaster forecast and reprioritize? Are estimates honest?                                                                    |
| **Fog-of-war trap**                       | Enemy silently relocates or jams contacts                                                                                          | Are observations legal, uncertainty updated, and scouting used sensibly?                                                               |
| **Coordinated capture**                   | Two platoons attack and support a sector with a common deadline                                                                    | Does Operations sequence them and refrain from defeating coordination with reassignments?                                              |
| **Wreck-control decision**                | Assault might give opponent a highly profitable reclaim field; injured units may self-destruct                                     | Does the AI understand metal-denial trade-offs where mechanics permit?                                                                 |
| **Elevation-dependent micro**             | Kiting/line-of-fire changes with relative elevation and different weapons                                                          | Are decisions weapon-specific, not based on a universal elevation bonus?                                                               |
| **Late-game nuclear saturation**          | Interception capacity and reload timing matter beyond coverage circles                                                             | Does the support estimator use real mechanics and does Strategy respond to actual risk?                                                |
| **Rare-tech crisis**                      | Artillery, experimentals, shields or unusual enemy composition appears                                                             | Can Strategy/Operations adapt without having seen that exact situation frequently?                                                     |
| **Module replacement**                    | Swap learned squad for scripted squad or upgrade Quartermaster                                                                     | Do typed interfaces remain stable and does performance degrade gracefully?                                                             |

### 10.1 Metrics to track per episode

- **Primary:** actual match win/loss, explicit mission success/failure, objective value captured/denied and timing.
- **Tactical:** damage dealt and losses weighted by mission; active formation and duration; useful firing participation; range maintenance; valid mode changes and recovery; support coverage; positioning, kiting quality, valid retreat, friendly fire, ability effectiveness, enemy reclaim opportunities.
- **Operational:** force utilization and reserve readiness, response times, missed opportunities, avoidable sector losses, appropriately requested capabilities, mission churn and cross-platoon coordination.
- **Economic:** delivered capability value and timeliness, build-power utilization, resources stalled/wasted, builder travel and queuing delays, economic vulnerability and forecast accuracy.
- **Interfaces:** feasibility-report calibration, frequency of blocked/impossible orders, changes-of-mind, request acceptance, replan latency, failed acknowledgments, conflicting ownership.
- **Runtime:** simulation wall-clock throughput, policy training samples/sec, policy inference latency (including the squad selector), deterministic formation/geometry cost, peak memory, orders/sec, pathfinding work, simulation frame-time spikes.
- **Robustness:** held-out maps/factions/units, unseen enemy strategies, random seeds, observation corruption, long matches, patch compatibility.
- **Human review:** short ranked annotations of “efficient,” “sane,” “effective,” “reckless,” or “missed obvious opportunity,” with specific clips and explanations.

Evaluate modules against matched scripted baselines and run **ablations**: replace a learned formation selector with the heuristic while retaining the identical executors; remove upward feedback, remove high-ground candidates, remove uncertainty features, remove capability requests, disable learned micro, etc. These comparisons will show whether architectural complexity earns its cost.

---

## 11. Major risks and design decisions to investigate

### 11.1 Simulator mismatch

A lightweight custom battle simulator can make RL or evolutionary training cheap but may teach tactics that exploit inaccurate projectile physics, pathing, obstruction, targeting, cloaking, terrain, or economy mechanics. Maintain engine-in-the-loop validation and a library of small fidelity tests. Decide what can be safely approximated in each training stage.

### 11.2 Hierarchical distribution shift

A lower policy trained on random synthetic orders may encounter a narrow but adversarial distribution from a learned parent; a parent may learn to rely on children doing things they cannot do. Train around boundary cases and test entire command/report loops. Frozen-policy integration should precede fine-tuning.

### 11.3 Ambiguous ownership of units and resources

If Operations, Platoons and Squads all issue orders directly to the same units, behavior will thrash. If Operations and Quartermaster both independently allocate metal, capabilities cannot be delivered predictably. Establish ownership, priorities, command arbitration and handoffs as hard software constraints.

### 11.4 Reward mismatch between levels

A squad winning metal trades may cause a platoon to fail a hold; a platoon surviving may allow a front to collapse; a Quartermaster maximizing resource throughput may delay critical units; Strategy maximizing army value may neglect objectives. Use explicit mission-specific rewards plus measured global outcomes. Preserve the ability for parents to authorize local sacrifices.

### 11.5 Feasibility forecasting and uncertainty

Subordinates need to object intelligently, but forecasting success, enemy strength, and ETA is difficult. Start with heuristics and coarse classifications, then train/calibrate predictors separately if their decision value is proven. Reports should include confidence, evidence, and alternative proposals rather than deceptive precision.

### 11.6 Too many networks too soon

A role or time scale does not automatically justify a distinct model. Try shared trunks and policy conditioning; share low-level embeddings where helpful; keep geometric/scheduling services deterministic until clear evidence supports learned alternatives. Avoid premature choices such as fixed 64/128/256-float input budgets or mandatory 6–8 archetype MLPs.

### 11.7 Limited examples of rare situations

Natural matches may contain few nuke salvos, shield-breakthroughs, commander assassination attempts, late-tech transitions, or stealth infiltrations. Use snapshot resets, scenario injection, synthetic strategic goals and opponent specialists. Ensure special training does not cause catastrophic forgetting of common cases.

### 11.8 Training algorithm and compute cost

Estimate through experiments: game seconds/simulation second, trajectories to learn each mission type, samples per optimizer update, accelerator utilization, inference impact on simulation, and total engineer complexity. Compare PPO/actor-critic against existing evolutionary search or alternate methods under the **same simulator and evaluation budget**.

### 11.9 Patch and format stability

Game changes may alter unit mechanics, tech prerequisites, radar behavior, maps and action legality. Version all observation features, unit metadata, contracts, and policy checkpoints; use regression suites rather than assuming weight transplantation always works.

### 11.10 Team-game coordination

8v8 and larger games introduce allied humans or other bots, shared defenses, duplicate roles, ambiguous ownership, and cross-player coordination. Treat this as a planned expansion, not an assumption the 1v1 architecture automatically solves. Extend Operations' knowledge and delegation to ally coordination only after the core hierarchy works.

---

## 12. Questions the next agent must resolve

### A. Engine integration and observability

1. What BAR/Recoil APIs are actually accessible for collecting observations, replays, unit stats, terrain, LOS/radar, ability state, and issuing commands?
2. What headless and accelerated simulation configurations are reliable? What can be parallelized on the available CPU/GPU hardware?
3. Which data in replays are actual observations versus authoritative hidden state? How can training avoid information leakage?
4. How do native group orders, pathfinding caching, line-drag formations, and per-unit command rate affect CPU and behavior?
5. Which game mechanics need direct engine tests (elevation and ballistics, self-destruct/wrecks, shield sharing, anti-nuke reload, radar behavior)?

### B. Architecture/interface choices

1. What minimum mission vocabulary covers BAR gameplay without making the action space unmanageable?
2. Should Operations select from a generated candidate-operation list, a spatial map, a sector graph, or a mix?
3. How should **capability vectors** represent force effects while preserving composition-specific weaknesses and unit tech constraints?
4. What is the shortest useful upward status summary without hiding rare but critical reasons for failure?
5. How should emergency override, command ownership, persistence/hysteresis, mission expiry, and unit reassignment be implemented?
6. How much autonomy can each tier safely exercise, and when does it need higher-level approval?

### C. Training choices for each layer

1. What are the preferred observation/action heads, network families, reward decompositions, curricula, and algorithm choices for each controller?
2. Which components have sufficiently strong deterministic teachers for imitation learning? How are human replay commands mapped, if at all, to high-level intents?
3. Does learned squad formation selection improve on the strongest mission-aware heuristic using identical deterministic executors? For useful learned selectors and Platoon policies, should models share goal/role-conditioned weights, use specialist heads, or require multi-agent centralized critics?
4. Where do the five deterministic modes or their fixed presets demonstrably fall short? Should the next experiment add a bounded preset/parameter choice, a composite mode, or learned per-unit micro, and what are the actual inference/executor/command costs?
5. How can Operations be trained on spec-ops and terrain decisions before a live Quartermaster is available?
6. Is Quartermaster best modeled as a scheduler/optimizer with learned weights or as an end-to-end RL policy?
7. How do we handle rewards and credit assignment across controllers that decide on different time scales?
8. What role should existing evolutionary reinforcement learning have compared with gradient-based PPO, supervised imitation and planning?
9. How should limited manual match reviews contribute useful feedback without creating an incoherent or exploitable reward model?

### D. Benchmarking and rollout

1. Which baseline BAR AIs or scripted agents are appropriate and comparable in target game modes?
2. How many seeds, maps and opponent variants are needed for meaningful validation at each stage?
3. What early experiment demonstrates that **upward objections and capability requests** improve outcomes?
4. What is the minimum vertical slice deliverable in the shortest engineering time? What is the fallback if strategic RL proves too expensive?
5. How will policy releases be versioned, regression-tested, rolled back and compared with previous checkpoints?

---

## 13. Definition of success

The proposed system succeeds when it demonstrates the following causal chain **in an unfamiliar scenario**:

1. The shared intelligence/terrain services identify a *plausible* opportunity or threat without using hidden enemy knowledge.
2. **Operations** recognizes a strategically relevant objective, perhaps controlling high ground or neutralizing an important enemy installation.
3. Operations determines that its existing platoons lack some required capability and submits an **effects-based** request.
4. **Strategy** decides whether the broader situation justifies the investment and allocates authority/resources.
5. **Quartermaster** produces a feasible composition and delivery schedule or supplies a useful, honest alternative.
6. **Operations** forms/assigns the force at the correct time, coordinates it with other forces and reserves, and issues a mission contract.
7. **Platoon and Squad** controllers execute that mission through coordinated assignments and appropriate selection of deterministic formations, using terrain, weapon mechanics, screening, support and selective micro where justified.
8. If local facts invalidate the plan, the subordinate sends meaningful **feedback**, the parent revises or confirms the order, and the system avoids command thrash.
9. The mission outcome propagates to future operational/strategic decisions and training data.
10. The behavior is measurably better than an appropriate scripted baseline, scales to the engine budget, and generalizes outside its training examples.

**The end goal is not six clever isolated MLPs. It is an integrated command system that learns what to accomplish, recognizes when it lacks the means, obtains and employs the required means, and responds intelligently when reality disagrees with the plan.**

---

## 14. Source, provenance, and confidence

- **Source proposal:** `BAR MLP argon-output.pdf`, 28 pages. It originally proposed separate Strategy, Economy, Operations, Platoon and Unit Micro brains; later moved toward Strategy → Platoon → Squad with a parallel Factory+Eco controller and bidirectional feasibility/status reports. It also discusses specialized recon/spec-ops, terrain, static defense, nuclear saturation, and bottom-up training.
- **Formation-first revision (2026-10-19):** The project owner requested that Squad control start with simple deterministic charge/hold/orbit/encircle/kite formulas and prioritize choosing the right formation, based on reported strong results in existing tests. This revision narrows squad outputs and training accordingly across §§1–2, 4–6, 8–10 and the handoff questions. The reported performance is a project observation, not an independently inspected benchmark or proof across BAR modes/maps. Formation definitions, lifecycle rules and promotion gates here are proposed design refinements; exact formulas and engine semantics still require implementation review.
- **Retained from that proposal:** modular training, operational-scale awareness, a tightly coordinated economy/factory controller, platoon/squad hierarchy, goal-conditioned action, role-based squad policies, reuse of deterministic mechanics solvers, and two-way feedback.
- **Important revisions in this document:** restore **Operations** as a persistent planning tier; make orders and feedback explicit typed contracts; allow opportunities to propagate upward; separate *capability requirements* from specific unit recipes; distinguish responsibility tiers from learned models; treat per-unit micro as an empirical choice rather than universally wrong; avoid rigid network sizes/frequencies and oversimplified nuclear/terrain formulas; use mission-aware rewards rather than global metal efficiency.
- **Confidence level:** High in the conceptual separation of responsibilities and need for feedback/measurement; medium in preferred baseline learning approaches; **unverified** in BAR-specific engine details, exact model shapes, training speed, frame/update frequencies, and tactical/nuclear numerical mechanics. Those require direct investigation.
- **Research starting points (verify applicability rather than treating them as implementation recipes):** hierarchical/options reinforcement learning; PPO/GAE; multi-agent centralized-training/decentralized-execution; behavioral cloning and offline imitation; domain randomization and sim-to-engine transfer; policy checkpoint leagues/self-play; constrained resource scheduling and model-based planning.
