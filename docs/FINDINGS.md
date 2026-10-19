# Findings log

What each training experiment taught us. Newest first. Numbers were measured on
identical, fresh battles unless stated; the raw data lives in the run's own
directory (`history.jsonl`, `final/summary.json`, `final/report.md`).

## 2026-10-16: do units run away from combat? (`scripts/flee_report.py`)

A user report ("a number of combat units run away from combat, dooming their fellows") was
checked on replayed battles of the objective champion (12 matchups x `charge`/`kite` x both
sides, about 82,000 unit decisions).

- **40% of unit-time is spent moving away from the nearest enemy** (52% when the platoon
  is not clearly ahead, 8% when it is). 95% of that is the `spacing` module's negative
  `advance`, never `engagement` (which is constant "fight"), and 87% happens with the enemy
  already inside weapon range: it is range-keeping, not leaving. Ranged units do it most
  (Shellshockers 86%, Rockos 78%, Samsons 72%; Warriors 2%).
- Backing off costs little firepower (units fire in 1.7% of samples while backing off
  against 2.3% otherwise, per 10 Hz sample) and **forbidding it did not help**: allowing it
  only inside 0.6 of range cost 2 points (70% against 72%), inside 0.85 changed nothing.
- **What does leave fellows alone is absence:** 7.0% of unit-time was spent over 1.5 ranges
  from every enemy while a friend was in weapon range of one (21% for Hover Scouts, 18% for
  Garpikes, 8.4% at low commitment against 1.7% when ahead). Almost none of it is fleeing
  (3.9%): slow or distracted units simply stay out.
- Fix: the out-of-contact pull now also runs whenever a friend is already in range of the
  unit's target (`ENGAGED_ALLY_DRIVE`), cutting absence to 5.1% with the win rate 72% to 73%.
  Kiting by ranged units was left alone because the numbers do not show it hurting.

## 2026-10-16: attack and defend a target (`data/brains/obj/`)

**Setup.** `examples/train-t1-objectives.json`: the platoon mix plus 25% attack and 25% defend
fights with an immobile target (`armsolar` or `armlab`) behind its owner's platoons, and the
objective-aware opponents `strike` and `guard`. Measured on one fixed batch (90 fights x 4 seeds x
2 sides x 6 opponents), starting from the round-4 platoon champion.

| | plain | attack | defend |
| --- | --: | --: | --: |
| unchanged council (no objective rule) | 66% | 67% | 79% |
| + attacker rule (head for the target, shoot it in reach) | 66% | 73% | 79% |
| + trained on the mix for 4 rounds | 66% | 74% | 79-80% |

**What we learned**

1. The council ignores objectives, yet it already wins 67% of attacks (destroying a structure is easier
   than annihilation: weaker attackers win 51% against 35% in plain fights) and 79% of defences.
2. A fixed attacker rule helped: +6 points overall and 63% to 91% against `orbit`, at the price of more
   reckless dives (losses 19% to 24%).
3. **A defender leash backfired.** Pulling defenders back to within 2 weapon ranges of the target cost 6
   points overall and 32 points against `kite` (64% to 32%): the fight moves onto the target, where stray
   fire and splash hit it. A longer leash did nothing and was removed. Defenders must fight forward.
4. **Training added nothing** (4 rounds, both promotions within noise): without objective inputs the
   council has nothing new to learn. Remaining defence losses are matchups where the enemy outranges the
   target (a Wolverine at 710 range against swarms of Fleas).
5. Next, in order of expected value: learnable objective inputs (`tactical_v4`), speed and turn-rate inputs
   for chasing, a shared platoon assignment.

## 2026-10-16: generated T1 platoon matchups (`data/brains/t1/`)

**Setup.** Because the Pawn and Thug runs risked overtraining on a few fixed scenarios,
training moved to fresh random fights every generation: 19 then 26 T1 bots, vehicles
and hovercraft, platoons of one domain, half mixed and half pure, balanced across
underdog / even / overdog (see `docs/REFERENCE.md`). Four 2-round cycles of about
30 to 50 minutes each, started from the overnight Pawn champion. Every number below
is on one fixed batch of unseen matchups (60 fights x 4 seeds x 2 sides x 4 baselines).

| brain | all | stronger (>=1.5x power) | close | weaker (<=0.67x) | vs orbit | vs kite |
| --- | --: | --: | --: | --: | --: | --: |
| overnight Pawn champion | 60% | 84% | 64% | 27% | 44% | 51% |
| after round 3 | 62% | 87% | 68% | 28% | 46% | 53% |
| after round 4 | 64% | 88% | 70% | 31% | 52% | 54% |
| after round 6 (cycle 4) | 63% | 87% | 68% | 31% | 49% | 51% |

**What we learned**

1. Cycle 1 gave the large gain (54% to 62% on its own batch, underdog fights 32% to 45%);
   the next three cycles added +0, +2 and -1 points. The win rate is asymptotic for this
   council, objective and roster.
2. **Metal says little about strength.** 24 Fleas or 4 Rockos have metal but little power
   against the opposing mix. Judging by power ratio (health x DPS) shows the real picture:
   87% when stronger, 68% when close, 28% when weaker. A weaker side losing to `charge`
   (22% wins) or `kite` (12%) is mostly physics, not a council failure.
3. **The remaining headroom is chasing evasive units**: stronger sides still lose 16% to
   `kite` and time out 15% against `orbit`. The council cannot see speed advantage or turn
   rate, so adding those inputs is the main untested hypothesis.
4. **Round scores below about 0.05 are noise.** Even with a 48-matchup benchmark, two of
   four promotions were indistinguishable from the champion on the fixed batch. Judge
   promotions on `scripts/analyze_matchups.py`, not on the round score.
5. **Cost drifts upward whenever a knob allows it** (population 72, 4 seeds per battle:
   rounds of 18 to 30 minutes against a 15-minute target). Cost knobs are capped now.

## 2026-10-16: first overnight run (Pawn mirror, `tactical_v3`)

**Setup.** `examples/train-overnight.json`, started from the authored template
council, 22 rounds over about 5 hours (`data/brains/overnight/`, champion from
round 17, 16 promotions, 709,247 simulated hours of fighting). Opponents: `charge`,
`kite`, `orbit` plus recent champions.

**Result, on the same 576 battles per brain (48 seeds, 5 scenarios x 2 sides x 3
baselines):**

| brain | vs charge | vs kite | vs orbit | underdog wins | 1v1 | 3v3 | 2v3 | 3v5 | 5v3 |
| --- | --: | --: | --: | --: | --: | --: | --: | --: | --: |
| authored template, untrained | 43% | 20% | 25% | 0 / 576 | 26% | 21% | 0% | 0% | 99% |
| round 0 (5 minutes) | 49% | 83% | 60% | 148 | 78% | 94% | 29% | 23% | 95% |
| round 7 (about 1 hour) | 54% | 98% | 58% | 183 | 93% | 92% | 33% | 31% | 100% |
| round 17 (champion) | 56% | 99% | 71% | 250 | 95% | 93% | 55% | 32% | 100% |

**What we learned**

1. **Most of the gain came in the first minutes.** Five minutes took the council
   from losing even fights to winning them. The next four hours added about 5
   points against `orbit` and doubled the 2v3 underdog result. Hours 2 to 5 were a
   slow climb, not a steady one.
2. **The complaint is fixed.** 5v3 is 100% (it was fleeing from three enemies).
   The platoon-commitment doctrine and the authored starting council did that, not
   the training: the untrained template already won 5v3 99% of the time.
3. **Only two modules do real work.** From the measured report:
   - `spacing` carries the micro: advance depends on own HP (hurt units back
     off), whether the target is approaching, distance, and the platoon power ratio.
   - `encircle` is heavily used and amplified the authored rule: the gap to the
     neighbour on each side dominates it.
   - `engagement` collapsed to a constant "always fight" (output -1.00, every input
     ignored). The fixed doctrine already handles retreat, so there was nothing for it
     to add against Pawn mirrors.
   - `target_selector` saturated and is nearly input-independent: with identical
     enemies there is nothing to choose between.
   - `strafe` and `cohesion` stayed close to silent (|output| < 0.25).
4. **Round scores are not comparable across rounds.** Each round's benchmark adds
   the latest champions as opponents, so the headline score fell from +0.36 to
   +0.15 while the win rates against the fixed baselines rose. Use the per-baseline
   win rates for trend, and the identical-battle table above for comparisons.
5. **The loop drifted toward cost.** Selection sees only score, so it adopted
   population 64 -> 108, seeds 2 -> 4 and layout jitter 30 -> 70. Rounds grew from
   5 to 22 minutes while quality plateaued around round 7. Those knobs are now
   capped.
6. **Late promotions are probably noise.** 16 of 22 rounds promoted a champion with
   a 0.005 minimum gain, while the baseline win rates barely moved after round 7.
   A larger minimum gain or more benchmark seeds would stop the ratchet.
7. **The focus child never won.** It trained only against the weakest baseline,
   which was almost always `charge`, an even matchup with a natural ceiling near
   55% in fair fights. Earlier, a hand-run 60-generation focus on `orbit` took
   `orbit` from 0% to 78%, so the idea works when there is headroom. The loop now
   alternates between the two weakest baselines.
8. **Orbit is slippery, not strong.** See `docs/REFERENCE.md`. A council trained from
   scratch against it alone learns to stay out of range (zero damage either way)
   and never wins; one that already knows how to fight beats it quickly.
9. **A stop needs two speeds.** `STOP` only takes effect between rounds, which were
   up to 22 minutes. `STOP_NOW` now abandons the round immediately.

**Caveats.** All of this is Pawn against Pawn, so the target-selection and
role-rank inputs had nothing to learn from. The benchmark noise is not measured, so
differences of a few points between rounds should not be read as real.

## Baseline for the Thug experiment (before any Thug training)

The overnight champion, trained only against Pawns, fighting an equal *metal* worth
of Thugs (`examples/train-thugs.json`, 24 seeds x 2 sides = 48 battles per cell,
separation 600). Wins / timeouts / losses:

| Pawns vs Thugs | Thugs charge | Thugs hold | Thugs kite | Thugs orbit |
| --- | --: | --: | --: | --: |
| 3 vs 1 | 44 / 1 / 3 | 38 / 5 / 5 | 36 / 7 / 5 | 47 / 0 / 1 |
| 5 vs 2 | 39 / 1 / 8 | 2 / 43 / 3 | 1 / 47 / 0 | 26 / 21 / 1 |
| 8 vs 3 | 46 / 0 / 2 | 21 / 27 / 0 | 3 / 45 / 0 | 45 / 3 / 0 |
| 13 vs 5 | 48 / 0 / 0 | 7 / 41 / 0 | 0 / 48 / 0 | 8 / 40 / 0 |

Overall 54% wins, fitness +0.18. The pattern is clear: a Pawn council that fights
Pawns well handles Thugs that charge, but against Thugs that **hold or kite** it
mostly cannot close and finish (40 or more timeouts of 48, exchange -0.2 to -0.46).
The Thug's cannon outranges the Pawn's gun (380 against 180) and the council still
keeps its distance as if the enemy were a Pawn. The metal-equal fights also come out
slightly in the Pawns' favour on the objective's value measure (metal plus energy
over 60): enemy-to-own value ratios of 0.77 to 0.92.

Provenance caveat: the Thug entry is hand-derived from BAR's unit definition and
script with the exporter's conversions, not exported from a running game.

## Open questions

- **Next stage, after the win rate flattens on the T1 platoon mix:** objectives, where a
  side attacks a target or defends one, instead of only fighting to the death. Requested
  by the user; not started. It needs a target entity in the arena, an objective term in
  `objective.rs`, and probably the platoon layer (`docs/PLATOON_LAYER_PLAN.md`).

- Does a shared platoon decision (focus fire, one posture, assigned ring slots) beat
  per-unit decisions? See `docs/PLATOON_LAYER_PLAN.md`.
- Does a Pawn-trained council learn to close on long-range Thugs that hold or kite
  (the starting point is 40+ timeouts out of 48)? See the Thug experiment in
  `docs/REFERENCE.md`.
