# Should there be a platoon council as well as a unit council?

Short answer: yes, but as a thin layer that owns only the decisions that are
collective by nature, and only if a cheap experiment shows it pays. This note
records the reasoning and the experiment so the decision can be made on evidence.

## What the current design already tells us

Two failures in this project were platoon-level problems that the per-unit council
could not solve, and both were fixed by adding platoon-level rules to fixed code:

- **Five units fleeing three.** Every unit judged the fight alone.
- **Two units parked out of the fight while a third fought alone.** The local force
  ratios only looked three weapon ranges around each unit.

The fix for both was the same: a platoon-wide power ratio (`DPS x HP`) and a
`commit` value that suppresses retreat and pulls stragglers back. In other words
we have already built a primitive platoon layer, in code, because the unit council
could not express it. The first overnight run supports this: the `engagement` and
`target_selector` modules, the two that look most like platoon decisions, collapsed
to constants, while the doctrine did that work.

Planetary Annihilation does the same split: its networks run per *platoon* (target
policy for the platoon, with squad roles), and unit-level execution is code.

## What a platoon layer would buy

| Decision | Today | With a platoon layer |
| --- | --- | --- |
| Which enemy to shoot | each unit picks among its 3 nearest | one shared focus target (focus fire) |
| Fight or withdraw | each unit, plus the fixed doctrine | one posture for everyone, no half-fleeing |
| Ring formation | each unit infers its gaps from neighbours | slots assigned once, evenly spaced |
| Compute | every unit rescans the world 3 times | one scan per tick, shared |

Focus fire is the likeliest real gain. Against 1100-HP Thugs, spreading shots
across several of them is much worse than killing one at a time, and nothing in the
unit council can coordinate that.

## What it would cost

- **Who is in a platoon.** Today a side is one platoon. Real BAR forces split and
  merge, so this needs grouping (for example clustering by proximity) before it
  generalises.
- **A second thing to inspect.** The viewer and report would show two councils.
- **Two time scales.** Platoon decisions are naturally slower (a few per second)
  than unit micro (every tick); the cadence mechanism exists.
- **No training change.** All units already share one set of weights and the score
  is already platoon-level, so evolution strategies train both layers unchanged.

## Recommended path, cheapest and most informative first

1. **Shared platoon state.** Compute the side-wide quantities once per tick
   (centroids, power ratio, commit, enemy front) and hand them to units. This is a
   pure refactor with identical behaviour and a speedup. Gate: all existing tests
   pass and battle results are bit-identical.
2. **Focus-fire experiment.** Add one shared focus target (for example the enemy
   the platoon's centroid is nearest to, or the weakest in reach) and A/B it against
   per-unit choice, with everything else held fixed. Run it on the Thug scenarios,
   where it should matter most, and on the Pawn mirror, where it should not.
   Decision rule: keep it only if it wins clearly more battles or finishes them
   faster on the Thug scenarios without losing anything in the mirror.
3. **Move posture up.** Make fight/withdraw a platoon output and delete the
   per-unit `engagement` module if it stays a constant.
4. **Assigned ring slots.** Replace the gap heuristic in `encircle` with slots from
   the platoon layer if the ring stays uneven.

Stop after any step that does not pay. Steps 1 and 2 are small; 3 and 4 only make
sense if 2 shows that collective decisions help.

## What not to do

Do not build a second full neural council with its own inputs and outputs before
step 2 shows there is something for it to decide. The cheap version is a few shared
variables, and a learned platoon module can come later if those prove valuable.
