# Objectives: attacking and defending a target

**Status (2026-10-16):** steps 1 and 2 are done and a fixed attacker rule is in `tactical.rs`
(results in `docs/FINDINGS.md`). The defender leash was tried and removed. Step 3 (`tactical_v4` inputs) is the
open decision. The outcome rules live in `objective.rs`, the target spawn in `train.rs`, the matchup shares in
`matchups.rs`, and the baselines `strike` and `guard` in `baselines.rs`.

Requested by the user as the stage after the win rate on generated T1 platoon
matchups becomes asymptotic. This note records what exists, what was missing, and
the order to build it so each step can be measured.

## What exists

- **Key units.** `StrategicDef.key_unit`, `SideScore.key_units_started/surviving` and the
  objective term `key_unit_bonus * enemy_fraction_lost - key_unit_penalty * own_fraction_lost`
  already flow from the training config (`key_units` names) through the engine into fitness.
- **Battles end by elimination or time.** `src/engine.rs` runs until one side has no
  units alive or `max_time`; `BattleResult.winner` is derived from alive counts.
- **Immobile units exist.** The catalog has structures (for example `armdrag`, `armmex`),
  and the engine already treats static units as obstacles that can be shot.

## What is missing

1. **A battle that ends on the objective.** An attacker wins when the target dies;
   a defender wins when it is still alive at `max_time`. Add an optional scenario
   field (`objective: { kind: attack | defend, target_unit }`) and end the loop when the
   key unit dies, recording which side lost it.
2. **A target in the arena.** One immobile key unit placed on the defender's side,
   behind its platoon, with the attacker starting farther away.
3. **Scoring.** Win/loss and stake come from the objective, not from annihilation:
   attacker win = target destroyed, defender win = target survives. Keep the existing
   exchange and key-unit terms. A defender who holds until time is a full win (unlike an
   even-match timeout, which is a partial loss).
4. **Inputs the council does not have.** Today a unit knows the nearest enemies and
   its own side. For objectives it needs: distance and bearing to the target,
   whether the target is ours or theirs, and the target's remaining health. This changes the
   brain's input specs, so it is a new brain kind (`tactical_v4`) and old brains cannot be
   loaded unchanged; the authored template needs matching wiring (defenders stay within a
   leash of the target and intercept; attackers prefer the target and push through).
5. **Generated objective fights.** Extend `src/learn/matchups.rs` so a share of matchups
   (for example 50% annihilation, 25% attack, 25% defend) carry an objective, with the
   same underdog / even / overdog balance judged by power.

## Order of work, each step measured before the next

1. Engine: objective ending plus scoring, with unit tests (target dies, target survives,
   ends early, both sides' sums consistent). No council change.
2. Baseline: run the current champion on attack/defend matchups unchanged. It will go for
   the nearest enemy, so the number says how much the objective actually changes the
   problem. Record it in `docs/FINDINGS.md`.
3. Inputs and template (`tactical_v4`): add the objective inputs, wire the template, check
   by test that the template alone beats the unchanged council on attack/defend fights.
4. Train on the mixed objective set in the same 2-round cycles and judge on a fixed batch
   split by objective kind, as `scripts/analyze_matchups.py` already does for class and power.
5. Only then ask whether a platoon layer (`docs/PLATOON_LAYER_PLAN.md`) helps: a shared
   assignment such as "these units guard, these attack" is naturally collective.

## Risks

- Attackers with no incentive to avoid losses will suicide into the defence; keep the
  exchange term so trading cheaply still matters.
- A defender that clumps on the target is easy to wall off; a leash radius larger than
  the weapon range keeps it fighting instead of idling.
- Old brains stop loading with the new input specs; the hall of fame for a v4 run starts
  empty, seeded from a converted v3 champion if the conversion is straightforward.
