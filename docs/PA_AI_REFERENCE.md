# Planetary Annihilation AI: what we verified, and how it shaped `tactical_v2`

Planetary Annihilation (PA) is the closest published analogue to BAR's AI problem,
so we studied how its AI uses neural networks. This note records **what was
checked, against what, and what is still unknown**, because several secondhand
summaries of PA's AI (including AI-generated ones) turned out to be wrong.

Confidence markers: **verified** = read directly from a primary artefact by us;
**documented** = stated by a source we trust but did not re-derive;
**inferred** = our reading, not confirmed; **unknown** = not publicly available.

## 1. The networks (verified)

Source: the three serialized network files in the *Vanilla AI Variations* mod
(`quildtide/vanilla-variations-ai`, `pa/ai_vaiv_x/x_aleph_x/neural_networks/`),
read for their metadata only. These are **modified** networks, so they verify the
interface and shape, not that every label matches the unmodified game.

| network | inputs | hidden | outputs | file size |
| --- | ---: | --- | ---: | ---: |
| `land_attack` | 77 | 2 layers of 385 | 46 | 7.4 MB |
| `bomber_attack` | 77 | 2 layers of 385 | 14 | 4.0 MB |
| `fighter_attack` | 77 | 2 layers of 385 | 9 | 4.0 MB |

- Weight layout: `inputWeights` (77 x 385 = 29,645), `inputBiasWeights` (385),
  `hiddenLayers[0]` (385 x 385 + 385 bias), `hiddenLayers[1]` (385 x N outputs +
  N bias; this entry is the output layer). About 196k parameters for `land_attack`.
- Hidden activation `Exponential`, output activation `Sigmoid`.
- Each output has a name (`outputNames`) and a unit-type filter (`outputToTypes`).
- **There are no input names anywhere in the files.** The meaning and order of the
  77 inputs is **unknown** publicly; recovering it needs the C++ that builds the
  input vector.

### Output vocabulary (verified)

Every output is a *targeting policy*, not a build or strategy decision.

- `land_attack`: 15 target policies x 3 modes (plain, `Range`, `Micro`) + `RunAway`
  = 46. Policies: Weakest, Highest DPS, Closest, Highest Value, Mobile Artillery,
  Fast Units, Mobile Units, Combat Fab, Fabber, Resource, Factory, Teleporter,
  Anti Surface Defense, Other Defense, Commander.
- `bomber_attack`: 13 policies (Weakest, Highest DPS, Closest, Highest Value,
  Mobile Anti Air, Fabber, Combat Fab, Resource, Factory, Teleporter, Anti Air
  Defense, Other Defense, Commander) + `RunAway` = 14.
- `fighter_attack`: 8 policies (Weakest, Highest DPS, Closest, Highest Value,
  Mobile Anti Air, Mobile Anti Surface, Mobile Missile Defense, Fabber) + `Run Away`
  = 9.
- Each output is paired with a unit-type expression such as
  `Structure & (MetalProduction | EnergyProduction)`; the action space is **defined
  in data**, which is why the outputs are readable.

## 2. How the networks connect to the rest of the AI (documented)

Sources: Queller-AI's `docs/ai-engine.md` and `docs/architecture.md`
(github.com/Quitch/Queller-AI, CC BY 4.0), cross-checked against the files above.

- Three networks, one per platoon domain; **no network decides builds, expansion,
  or where to attack**. Macro is a JSON rule matrix: `fabber_builds`,
  `factory_builds`, `platoon_builds`, each item having a `priority` and
  `build_conditions` (an OR of AND-groups).
- Networks appear in two places: a **gate** inside a rule
  (`CanAttackWithPoolUnitsLand` / `...Bomber` / `...Fighter`: "can this pool
  meaningfully attack those targets across those world layers?") and **runtime
  target selection** for a platoon whose task type is `LandAttack`,
  `BomberAttack` or `FighterAttack` (the task-to-network mapping is **inferred**
  from naming).
- Platoon templates list unit-type slots with `min_count` / `max_count` /
  `percent` and a **squad role** (`Fast`, `General`, `Close`, `Artillery`,
  `Defense`, `Escort`, `Suicide`); roles drive micro, and the network runs per
  platoon.
- Personalities carry `neural_data_mod`: `1.0` neutral, above it a handicap. How it
  works is **unknown** (see section 4).

## 3. The threat vocabulary (documented)

Source: planetaryannihilation.com/ai/ (the official page, saved locally by the
user because it blocks automated requests). It is an outline with several TODO
sections and **says nothing about the neural networks**. What it does give:

- About 40 **influence types**, mostly DPS summed over weapons that can hit a layer
  (`AntiSurface`, `AntiAir`, `AntiSub`, `AntiOrbital`, `AirToAir`, `SurfaceAntiAir`,
  `Artillery` (structures only), `MobileArtillery`, `Bomber`, `Tactical`, ...), rate-of-fire
  variants, and economy values: `Economy = build_metal_cost + max_health +
  storage.metal`, `EconomyProduction = production.energy + production.metal`,
  split by domain.
- Ratio conditions are "enemy `string0` / AI `string1`" and can compare different
  influence types. Conditions run at base, planet or system scope; non-real-time
  ones are checked every 2 seconds.

## 4. The Supreme Commander 2 chapter (verified, but a different game)

Source: M. Robbins, "Using Neural Networks to Control Agent Threat Response",
*Game AI Pro* ch. 30. It is about **Supreme Commander 2** and never mentions
Planetary Annihilation. The same designer built both, so the ideas are a good
guide, but **its numbers are not PA's**.

- Inputs: 17 friend/enemy **ratios** over a radius around the platoon (unit count,
  health, DPS, speed, resource value, shield health, short/medium/long-range DPS,
  repair rate), each clamped to 0..1 with its reciprocal included = **34 inputs**.
- One MLP per platoon type (land, naval, bomber, fighter), 98 hidden nodes, 15
  outputs. Each output is the **expected utility** of an action; the highest wins.
  Retreat = no output above 0.5.
- Training: self-play with **random** actions; the chosen output's target is
  `output * (1 + (friendRatio - enemyRatio))`, where each ratio is the average of
  new/old statistics over the engagement; other outputs keep their current value.
  Learning rate 0.8 down to 0.2, momentum 0.9 down to 0. About an hour per network.
- The **commander bug**: a purely tactical fitness taught the AI to flee the enemy
  commander, whose death explosion killed its attackers. Fix: a large utility for
  killing it.
- A personality "aggression" value scaled the input ratios so units looked
  stronger. `neural_data_mod` plausibly works the same way; this is **inferred**.

## 5. Claims we rejected

A Gemini summary was checked and found unreliable. Among its errors: it attributed
the SC2 chapter to PA; invented naval, orbital and threat-response networks (PA has
three); gave outputs "Engage / Standoff / Retreat / Augment" (the real outputs are
target policies x mode); presented a named list of PA's 77 inputs (not public);
quoted a fitness function not in the chapter; and named the wrong net-backed
condition. A ChatGPT summary matched the files on every point we could check
(three networks, sizes, outputs, no public input names, the 34-input list being SC2's)
and flagged its one unverified claim (planet-wide context added in a PA patch).
Prefer the artefacts above to any summary.

## 6. What this changed in MiniBAR

`tactical_v2` (`src/learn/tactical.rs`):

- Inputs follow the SC2/PA approach: friend/enemy **ratio pairs** (each statistic as
  `ours/theirs` and `theirs/ours`, clamped to 0..1) over a local radius of three
  weapon ranges, for count, HP, DPS, value, speed and DPS by range band (short, mid,
  long relative to the unit's own range). This replaced a plain unit-count ratio.
- Squad-role information enters as three **relative ranks** (speed, range, max HP
  versus the best friendly combatant).
- A fifth module, `engagement`, outputs `retreat`: the explicit RunAway PA's networks
  have. Positive values blend movement toward running from the enemy force.
- The objective gained a **key-unit term** (`key_units`, `key_unit_bonus`,
  `key_unit_penalty`), the commander-bug fix: killing an enemy key unit is rewarded
  and losing ours is charged, regardless of whether the battle is otherwise won.

Not done yet, and the obvious next steps: a data-labelled **target-class policy**
(PA's `outputToTypes` idea), network-backed **gate conditions** for the strategic
layer, **squad templates** for rosters, a utility-style (SC2) trainer as an
alternative to evolution strategies, and ending a battle when a key unit dies.

## Sources

- planetaryannihilation.com/ai/ (official, user-supplied copy)
- github.com/Quitch/Queller-AI, `docs/ai-engine.md`, `docs/architecture.md` (CC BY 4.0)
- github.com/quildtide/vanilla-variations-ai (network files, metadata only)
- M. Robbins, *Game AI Pro* ch. 30 (gameaipro.com, 2015)
