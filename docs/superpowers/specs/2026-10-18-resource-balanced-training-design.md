# Diverse, resource-balanced tactical training

Status: implemented and verified. See [the runnable workflow](../../DIVERSE_TRAINING.md).
The investigation below preceded implementation. The completed campaign, its
champions and optimizer state remain intact. The successor has separate inputs
and three-generation ground/air and cohesion-only validation pilots.

## Intent

Continue improving the learned champions while making the fights substantially
more varied. Equal investment should produce very different armies, rather than
the same five units on each side with different starting positions. The requested
investment measure is **metal + energy / 60**.

Coverage must include heavy units against swarms; Starlight versus Banisher;
mixed ground compositions; fighters against fighters; bombers against AA; and
LLTs, MLTs, HLTs, MRPCs and other combat structures against mobile forces.
Integer unit counts make exact equality uncommon. Actual resource differences
must be retained and included in scoring.

## Evidence from the completed run

The tactical recovery campaign completed 200 generations. Its only promotion
was a structural/weight candidate at generation 70, with one spacing input
disabled. No later candidate became champion.

The optimizer was still receiving a signal: candidate scores varied, and the
local branch's mean training objective-stage score rose from 0.378 over
generations 1–70 to 0.593 over generations 131–200. However, all 28 local
validation candidates in the final interval failed stage protection. Twenty-seven
failed the fixed initial objective floor; 23 also regressed on the fresh
objective screen. Only two had positive aggregate fresh screening gains.
These overlapping counts do not describe 50 separate rejected candidates.

All six training attack/defend templates are 5-versus-5. Validation includes
4-versus-6 and other uneven armies. Training also mixes scripted and historical
learned opponents equally, while promotion evaluates scripted opponents only.
That mismatch exists from initialization: the archive starts with the initial
champion, not at generation 70.

A 32-layout discovery study compared 12 models across scripted and learned
opponents, including seven individual module replacements. An independent
confirmation used 96 new paired batches, two fresh layouts per batch and both
opponent rosters. Replacing only the champion's cohesion module with the final
local branch's cohesion module improved fitness:

| Opponents | Mean gain over gen-70 champion | Batches won | Paired bootstrap 95% range |
| --- | ---: | ---: | ---: |
| Scripted | +0.04315 | 73/96 | +0.02757 to +0.05872 |
| Historical learned brain | +0.03411 | 53/96 | +0.00400 to +0.06343 |
| Equal mixture | +0.03863 | 63/96 | +0.02279 to +0.05435 |

On scripted opponents, all three stage means improved. The learned-opponent
mixed stage regressed by 0.04619, within the existing 0.15 protection allowance.
These intervals measure layout variation for the current models and scenario
family; they do not establish improvement on new unit types or in native BAR.
The holdout was not used in this investigation.

The candidate is preserved at
`data/experiments/tactical-plateau-20261018/cohesion-candidate.json`, with an
evaluation receipt. It has not been written into the completed run as a
fabricated historical promotion. Its fixed audit is 0.23256 versus the retained
champion's 0.18212, and it passes the existing fixed stage floors.

The generation-190 structural centre's earlier promising result did not
reproduce: confirmation mean gain was -0.01337 against scripted opponents and
-0.06134 against the historical opponent. Lowering the evidence threshold to
accept that candidate would have been a mistake.

## Resource values and composition generation

Use `UnitDef::resource_value()` throughout packing, realized-value reporting,
imbalance classification and scoring. It already implements the requested
formula. The current generator instead uses `metal_cost` when packing and
labelling armies; that inconsistency must be corrected.

Generate fights in two complementary ways:

1. **Anchored matchups:** choose a recognizable unit or composition and pack a
   deliberately different opposing composition near its value. This guarantees
   coverage such as one heavy unit against a swarm and one artillery type
   against another.
2. **Budgeted compositions:** choose a resource band and composition family,
   then independently pack both armies near that budget. Include homogeneous
   armies, mixed roles, combined arms and multiple separated platoons.

Use coverage quotas for fight families so random sampling cannot drown out
turrets, rare expensive units, aircraft or specialist counters. Sample small,
medium and larger fights. Retain balanced, advantage and disadvantage bands;
balanced investment is the main composition-comparison band.

For integer packing, examine nearby floor/ceiling counts and affordable mixed
substitutions. A configurable symmetric gap,
`abs(value_a-value_b) / ((value_a+value_b)/2)`, labels the realized imbalance.
A reasonable initial design target is at most 5%, with explicit curated pairs
allowed up to 10%. These are proposed tolerances to validate, not measured
optimal settings.

Examples from the checked-out BAR definitions:

| Army | Resource value |
| --- | ---: |
| 1 Mammoth (`corsumo`) | 2,783.33 |
| 40 Pawns (`armpw`) | 2,760.00 |
| 1 Starlight (`armmanni`) | 1,508.33 |
| 1 Banisher (`corban`) | 1,383.33 |

Mammoth versus 40 Pawns has about a 0.8% gap. Starlight versus Banisher one on
one has about an 8.6% symmetric gap and remains a useful explicitly labelled
pair. These figures come from local source definitions, not a claim about all
BAR mod options or a separately updated live game build.

Unit caps must never silently invalidate parity. The current default of 24
units cannot represent the Mammoth/Pawn example. Use suitable bounded swarm
capacity and report cap failures. If a requested value match cannot fit the
performance limit, resample or record it as an explicitly imbalanced case.
Missing, unsupported, non-finite or non-positive-cost units are rejected with
an actionable explanation.

## Scoring and evaluation

Equal resource value does not imply equal combat power. Keep asymmetric
matchups and hard counters; do not filter them into identical-looking armies
using a health-times-DPS balance requirement.

Use actual starting investment for outcome stakes and normalized damage/value
exchange, with explicit bounded ratio rules. Review the current victory stake
floor: it currently awards favourites a full win even when their investment is
higher. The chosen new scoring contract must consistently disclose how both
wins and losses account for the remaining imbalance.

Record for every fight: both compositions and counts, both realized values,
requested budget, signed ratio, absolute gap, fight family, objective, seed,
opponent identity, side, unit cap and resource-supply assumptions. Distinguish
the value of non-combat objective assets from combat investment. Platoon packing
balances combat investment; full starting-team investment, including objective
assets, remains explicit and is used by the outcome scorer. Combat turrets count
as fighting assets in the packing budget.

Use the same opponent-roster policy for training, screening and promotion.
Freeze evaluation opponents within a comparison round. Candidate and incumbent
face identical fights and mirrored sides. Training and validation share the
intended distribution, while using separate generated rosters and seeds.
Keep an untouched holdout.

Show family-level results for heavy/swarms, mixed ground, mobile/turret,
air/air and bomber/AA, plus faction and attack/defend breakdowns. Retain stage
and family protection. A single easy family must not compensate for destroying
an important skill elsewhere. Use more fresh confirmation batches for small
gains instead of lowering the bar to manufacture champion promotions.

Provide a curriculum preview using the same seeded matchup records as native
evaluation: readable unit names and counts, both investment totals, residual
imbalance, family and support status. Make sampled fights watchable with the
champion and candidate, including side swaps. The dashboard should show family
coverage and performance, so broad variation is visible instead of hidden in
configuration files.

## Separate ground and air brains

Air has its own brain because aircraft bypass ground terrain/pathing constraints
and have different movement and attack behavior. Preserve that separation in
the expanded curriculum. Aircraft must not be controlled by the ground brain.
Each domain has its own input contract, weights, evolving activation functions
and connections, optimizer state, champion, and promotion results.

Share resource accounting, matchup records and simulation infrastructure across
domains. In bomber-versus-AA fights, the air brain controls the aircraft and the
ground brain controls mobile AA; static AA uses its supported turret behavior.
When evaluating a candidate in either domain, freeze the other domain's champion
so simultaneous changes cannot obscure which candidate improved the result.
Report coverage and performance separately for each brain, including its role
in mixed air/ground fights.

Air movement bypasses ground passability and ground pathfinding. Model its own
altitude, turning and attack-run constraints; retain terrain interactions where
they affect altitude, visibility or projectile collision. Separate air policy
architecture does not by itself establish that those flight mechanics are
already supported by the current simulator.

## Catalog and simulator support

The current replay-derived training catalog has only 22 ground/economy units.
The bounded exporter explicitly excludes flight and advanced mechanics. A
larger source-backed catalog is required for the requested ground and turret
families, including actual costs, armor, category restrictions, ranges,
projectiles, splash, turret traverse, mobility and combat-relevant capabilities.
Source provenance and unsupported-mechanic reporting remain mandatory.

The existing generator supports bot and vehicle pools. Add explicit static
combat-structure and aircraft families rather than forcing them through an
incorrect movement category.

Aircraft work is part of the requested scope and requires simulator support:

- Airborne altitude and air movement separated from ground occupancy/pathing.
- Appropriate speed, turning and attack-run behavior for fighters and bombers.
- Correct weapon targeting categories, altitude-aware range/aim/visibility and AA.
- Bomb release, travel and splash behavior suitable for bomber-versus-AA tests.
- Required policy observations through the air brain's explicit versioned input
  contract, independently of the ground brain's inputs and parameters.

Validate these mechanics with bounded combat fixtures before aircraft results
can influence champion selection. Ground and turret expansion can be verified
independently while flight support is developed. Do not label an unsupported
aircraft matchup as a successful simulator training case merely because a
catalog file loads.

## Learning stability and preserving useful progress

Fix the optimizer reset bookkeeping before the next long campaign. `recentred`
zeros Adam moments while preserving `EsState.generation`; `tell_masked` uses
that global generation for bias correction. After the generation-71 adoption,
the coded normalization makes some steps about 1.8 times the properly reset
normalization. Keep noise-stream generation separate from updates-since-moment-
reset, and test exact continuation and recentering behavior.

Use the preserved cohesion candidate and the old champion as explicit ground
pilot anchors. Preserve the air brain's own champion and lineage if an existing
checkpoint is available; otherwise identify its initialization explicitly rather
than treating ground weights as an inherited air champion.
The successful module replacement shows useful progress can be hidden
inside a worse whole-brain update. Compare targeted module training and periodic
module-combination candidates with whole-brain training; validate the assembled
brain, not isolated module scores.

The structural branch currently uses population 8 and noise 0.16, versus 32
and 0.04 in the local branch. It changes the wiring while perturbing every
active weight much more strongly. Test structural refinement with gentle
weight noise and an adequate population, alongside a weight-only control.

Save evaluated candidate weights and lineage, including promising rejected
candidates. Add bounded stagnation recovery that reanchors a branch at a
preserved champion with fresh moment state after a configured unsuccessful
window. Do not confuse that optimizer reset with discarding learned weights.

## Validation and rollout

1. Verify resource arithmetic, nearest-count packing, realized imbalance,
   cap handling and deterministic paired generation, including the Mammoth/Pawn
   and Starlight/Banisher examples.
2. Verify coverage across all requested families and source-backed catalog
   capability checks. Exercise distinct compositions and unequal counts.
3. Verify turret-versus-mobile combat, air movement/targeting and bomber/AA
   behavior before admitting those families to training. Verify domain-specific
   brain routing, independent checkpoints and promotion, and frozen opposing
   domain policies during mixed air/ground comparisons.
4. Verify reset clocks, dormant parameters, exact resume, challenger retention,
   immutable lineage and dashboard explanations.
5. Warm-start separate short pilots from preserved weights with new seeds;
   compare learning per unit of compute and family-level retention. Keep at
   least one unchanged reference. Confirm promising candidates independently
   before a long campaign.
6. Package any successor with a validated executable/config identity and clear
   provenance. Do not modify the completed campaign's frozen inputs to resume it
   under changed semantics. Run visible training with 32 logical processors on
   this machine, with cooperative stop/resume and informative progress.

Evidence and reproducible probes are under `.quality-work/plateau-200/`, including
the original-checkpoint digest, gate audit, exact optimizer replay, discovery,
independent confirmation and fixed-audit results. No holdout results were used to
select the diagnostic candidate.
