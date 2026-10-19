# Tiered Brains

A design for splitting a Beyond All Reason (BAR) AI into several small neural
"brains" that each work at their own level, from overall strategy down to
groups of one unit type. This is a cleaned-up version of a Gemini conversation
(<https://share.gemini.google/m92iDXxFPGHx>), captured 2026-10-19. The
conversation built the design over seven turns, and this document gives the
design as it stood at the end, not a transcript. Where the user changed
Gemini's first proposal, the user's version is the one kept here.

> **How far to trust this.** The architecture and training ideas are the
> useful part. The BAR-specific facts Gemini gave are much less reliable:
> about half of its unit names were wrong, out of date or the wrong unit for
> the role (see [Corrections to Gemini's BAR facts](#corrections-to-geminis-bar-facts)).
> This document uses the names from `asym/units.json`, plus the codename where
> it helps. Treat every other number (ranges, tick rates, layer widths, reward
> weights, timings) as a starting guess to tune, not as a measured value.
> Gemini cited a few sources (an HRL survey, Reddit threads about BAR's AI,
> the BAR unit pages), but most of its claims had no citation.

## Contents

1. [The hierarchy at a glance](#1-the-hierarchy-at-a-glance)
2. [Why not one brain per unit](#2-why-not-one-brain-per-unit)
3. [Platoon and squad brains](#3-platoon-and-squad-brains)
4. [Strategic brain and Factory + Eco brain](#4-strategic-brain-and-factory--eco-brain)
5. [Support systems: what needs a brain and what doesn't](#5-support-systems-what-needs-a-brain-and-what-doesnt)
6. [Saturation defence: nukes, anti-nukes and shields](#6-saturation-defence-nukes-anti-nukes-and-shields)
7. [Sensors](#7-sensors)
8. [Network shapes](#8-network-shapes)
9. [Training pipeline](#9-training-pipeline)
10. [Open questions](#10-open-questions)
11. [Corrections to Gemini's BAR facts](#corrections-to-geminis-bar-facts)
12. [How the conversation went](#how-the-conversation-went)

---

## 1. The hierarchy at a glance

Gemini first argued that two tiers (strategic + platoon) are not enough for
BAR. Its reasons were BAR's streaming economy, build power that sits in one
place, real projectile ballistics and very large unit counts. One brain asked
to handle both tech transitions and second-by-second energy-to-metal
conversion, or both flanking and dodging individual shells, gets too large an
action space and too noisy a reward to learn well. The fix is the standard one
from hierarchical reinforcement learning (HRL) and the "options" framework:
several brains, each with its own tick rate and time horizon.

The user then reshaped the lower tiers into **Strategic → Platoon → Squad** and
asked for a separate **Factory + Eco** brain under Strategic. Gemini agreed
with both changes. The resulting design:

```
Strategic brain (~0.1 Hz)
 ├─► Factory + Eco brain, "Quartermaster" (~1 Hz)
 │     └─► constructor tasks, factory queues, build priorities
 │           └─► deterministic placement solvers (radar, anti-nuke, shields)
 ├─► [Operational / theater brain (~0.5–1 Hz), optional; see §10]
 └─► Platoon brain, one per platoon (~2–5 Hz)
       └─► Squad brains, one per unit-type group (~3–5 Hz)
             └─► one group move/fight order per squad
                   └─► deterministic per-unit reflexes (retreat, rejoin)
```

| Tier | Tick rate | Horizon | Decides | Outputs |
| :--- | :--- | :--- | :--- | :--- |
| **Strategic** | 0.1 Hz (every 10 s) | 3–20 min | Tech timing (T1 → T2 → Gantry), factory mix (bots, vehicles, air), overall posture, nuke doctrine | Budget split **w** = (eco, tech, army, defence); platoon orders ("raider platoon + AA screen, ~3,500 metal, take the north mex cluster") |
| **Factory + Eco** | 1 Hz (every 30 frames) | 10–60 s | Metal/energy balance, wind vs. solar, converters, construction turret use, reclaim, factory queues | Constructor tasks, unit mix for platoon orders, high/normal/low build priority |
| **Operational** *(optional)* | 0.5–1 Hz | 15–90 s | Which front each platoon covers, holding the front vs. stopping raids, radar/jammer coverage | Sector assignments, reinforcement streams |
| **Platoon** | 2–5 Hz | 2–15 s | How to carry out the platoon's task: where to anchor, which way the threat is, when to commit or retreat | Platoon anchor (X, Y), threat-axis angle, posture (advance, hold choke, kite back, retreat), priority target sector |
| **Squad** | 3–5 Hz | 0.1–2 s | How one unit type is placed and fights inside the platoon | Formation offset relative to the anchor, line width and angle, stance, target filter, retreat threshold |

Deterministic code, not neural nets, does the jobs that are pure geometry or
fixed rules. That covers building placement, nuke salvo timing and pulling one
damaged unit out of a squad. Sections 3.4, 5 and 6 cover each case.

---

## 2. Why not one brain per unit

The user asked whether each unit inside a platoon needs its own network.
Gemini said no, for three reasons.

1. **Pathing and collisions.** The Recoil pathfinder is built around group
   orders and line-drag formations. Thirty Rocketeers given one line-drag
   order share a path and spread out cleanly. Thirty separate networks each
   nudging their own unit every few frames keep invalidating each other's
   paths, bump into each other, and block each other's shots (BAR simulates
   real projectile blocking).
2. **Wasted inference.** Twenty identical units side by side have the same
   speed, turn rate, range and ballistics. Twenty forward passes that all
   decide "step back 80 elmos" spend ~95% of the CPU budget on nothing.
3. **Credit assignment.** When a 60-unit platoon wins a fight, splitting the
   reward across 60 unit brains gives very noisy gradients. Splitting it
   across 4–6 squads is clean and easy to read.

This matches feudal reinforcement learning (Vezhnevets et al., 2017) and the
shared-parameter approach of multi-agent methods like MAPPO and QMIX.

---

## 3. Platoon and squad brains

A **platoon** is mixed (combined arms). A **squad** is a single unit type, or
units that move and shoot alike. Strategic forms a platoon with a task in
mind. The platoon brain decides how to carry out that task. Each squad brain
decides how its unit type should be used to support that plan.

### 3.1 Worked example: one platoon, five squads

Strategic orders a T2 Cortex push on a canyon mex cluster. The platoon brain
picks the anchor point and threat axis, then five squad brains place their
units relative to that anchor:

| Squad | Units | Role | What the squad brain does |
| :--- | :--- | :--- | :--- |
| A, front line | Sumo / Mammoth (`corcan`, `corsumo`) | High HP, short–medium range, slow | Tight, slightly curved line ~150 elmos ahead of the anchor on the threat axis. Soaks fire and physically blocks enemies from the back line. |
| B, stand-off | Long-range skirmishers *(see note)* | Long range, fragile, big volleys | Wide concave line ~200 elmos behind A, keeping ~90% of max range to the nearest threat with A between them and the enemy. |
| C, flank screen | Fast anti-swarm units, e.g. Fiend (`corpyro`) or Grunt (`corak`) | Fast, strong at close range | Sits on the flanks (±75° off the threat axis) and waits for raiders (Ticks, Pawns, Rovers) that try to slip past A to reach B. |
| D, cover | Mobile jammer Deceiver (`corspec`) + mobile flak | No ground weapons | Follows B's centre, slightly behind it, so the jammer hides B without stepping into enemy sight. |
| E, logistics | Graverobber (`cornecro`) rez/repair bots | Repair and reclaim | Stays behind A and repairs damaged front units. Moves forward to reclaim wrecks only when enemy DPS in the forward zone is near zero. |

*Note:* Gemini put 16 "Sheldons" in squad B as 700-range T2 skirmishers. In
BAR, Sheldon (`cormort`) is a T1 mortar bot, so the example needs a real
Cortex T2 long-range unit there.

### 3.2 Use a platoon-relative frame for squad inputs and outputs

Never give a squad brain world coordinates. Put everything in a frame centred
on the platoon:

- **Origin:** the platoon anchor set by the platoon brain.
- **0°:** the threat axis set by the platoon brain.
- **Squad output:** polar formation parameters relative to the anchor:

  $$a_{\text{squad}} = (\Delta r,\ \Delta\theta,\ \text{line width},\ \text{line angle},\ \text{stance})$$

A thin bridge converts this back to world coordinates and issues one line-drag
move or fight order for the whole squad. Because the frame does not depend on
map position or compass direction, every fight in every replay trains the
same weights.

### 3.3 Archetype brains plus unit stats, not one brain per unit type

BAR has 400+ units across three factions. A separate squad brain for every
unit wastes samples and breaks whenever a balance patch changes a range by
20 elmos. Instead, train **6–8 archetype squad brains** whose weights are
shared across factions, and feed each one a small vector of unit stats:

$$\mathbf{u}_{\text{stats}} = [\text{speed},\ \text{turn rate},\ \text{max range},\ \text{min range},\ \text{projectile speed},\ \text{AoE},\ \text{HP},\ \text{DPS},\ \text{is ballistic},\ \text{can cloak},\ \text{can reclaim},\ \text{is amphibious}]$$

Because range and speed are inputs, one skirmisher brain naturally kites at
different distances for different units.

| # | Archetype | Example units (corrected names) |
| :--- | :--- | :--- |
| 1 | Bruiser / assault | Stout (`armstump`), Sumo, Tzar (`corgol`), Bull, Tiger (`correap`), Banisher |
| 2 | Stand-off skirmisher / artillery | Rocketeer (`armrock`), Aggravator (`corstorm`), Janus, Sheldon (`cormort`), Wolverine |
| 3 | Fast raider / swarm | Tick, Pawn, Grunt, Rover, Incisor, Fiend (`corpyro`) |
| 4 | Support: mobile AA, shield, jammer | Gremlin, Deceiver (`corspec`), Smuggler (`armaser`), Phalanx, mobile flak |
| 5 | Combat engineer / reclaim | Lazarus (`armrectr`), Graverobber (`cornecro`), Consul, Twitcher (`corfast`) |
| 6 | Air wing | Fighters, bombers, gunships |
| 7 | Recon and special ops *(added in §5)* | Blink (`armpeep`), Finch (`corfink`), Oracle (`armawac`), Condor (`corawac`), Ghost (`armspy`), Spectre (`corspy`) |

### 3.4 Individual units without per-unit brains

Only two cases need per-unit control, and neither needs a per-unit network:

1. **Squads of one.** A commander or an experimental (Juggernaut, Ragnarok,
   Thor) is just a squad with N = 1. The squad brain's output then drives that
   one unit directly, including abilities like the commander's D-Gun.
2. **Simple per-unit reflexes.** If one Sumo out of eight drops to 15% HP, you
   don't want the whole squad to retreat. Give the squad brain continuous
   outputs such as `retreat_health_threshold` ∈ [0, 0.5] and
   `scatter_spacing` ∈ [0, 1]. A ~10-line loop checks each unit about once a
   second. A unit below the threshold leaves the line and walks to the
   platoon's engineer squad, then rejoins once it is back above ~70% HP.

---

## 4. Strategic brain and Factory + Eco brain

### 4.1 Factory and economy belong in one brain, separate from Strategic

The user proposed a Factory + Eco brain under Strategic, with Strategic
deciding the split between economy, tech and army. Gemini agreed, and added
that factories and economy must not be split from **each other**.

In BAR, every active factory, construction turret and constructor drains
metal and energy every frame in proportion to its build power. If a separate
factory brain queues 15 units with 6 turrets assisting at the same moment a
separate eco brain starts an Advanced Fusion Reactor, energy hits zero. Mexes
stop, energy weapons stop firing, and build power collapses for both
projects. A single brain that owns all production and the economy avoids
this. The split is then:

- **Strategic** decides *what the side needs*.
- **Factory + Eco** decides *how to balance the resource streams to deliver it*.

| Brain | Tick | Outputs |
| :--- | :--- | :--- |
| Strategic | 0.1 Hz | 1. Budget split **w** = (w_eco, w_tech, w_army, w_def), summing to 1.<br>2. Platoon orders (archetype mix, budget, task). |
| Factory + Eco | 1 Hz | 1. Constructor tasks: wind, solar, converters, mexes, turret placement.<br>2. Factory queues to fill the platoon orders, plus high/normal/low build priority. |

### 4.2 Example handshake

At 8:30 a scout spots the enemy building a Cortex Experimental Gantry.

1. **Strategic** expects a T3 push that will crush light units. It shifts the
   budget from a balanced (0.35, 0.15, 0.40, 0.10) to (0.45, 0.25, 0.20, 0.10)
   and changes its platoon order to stand-off heavy skirmishers with EMP
   support.
2. **Factory + Eco** takes **w** and the new platoon order as its goal input.
   Over the next minute it:
   - sees wind fall from ~14 to ~4, so before starting the T2 lab it puts
     constructors on advanced solar at high priority to avoid an energy stall;
   - drops the T1 lab to low priority, so it only uses spare resources, and
     sends most turret build power to the Fusion Reactor;
   - once the fusion is up, queues the requested platoon mix, gathers it at
     the rally point and hands it to a platoon brain.

### 4.3 Upward feedback: the feasibility summary

Lower brains should report back, not just obey. On every Strategic tick,
Factory + Eco passes up a ~6-float summary that becomes part of Strategic's
input:

| Signal | Range / scale |
| :--- | :--- |
| `current_stall_severity` | [0, 1] |
| `build_power_saturation_ratio` | [0, 2] (idle vs. bottlenecked constructors) |
| `estimated_seconds_to_complete_current_platoon` | normalised |
| `reclaim_buffer_available` | log-scaled metal in reach |
| `energy_volatility_risk` | wind energy / total energy |
| `actual_vs_target_budget_error` | L1 distance between requested and achieved **w** |

This lets Strategic see when the economy is strained, so it learns not to
demand T2 during an energy brownout.

---

## 5. Support systems: what needs a brain and what doesn't

The user asked whether intel, radar or anti-nuke need their own brains.
Gemini's answer was a **hybrid split**:

- **When and how many** (a judgement about the game state) goes to the
  existing Strategic and Factory + Eco brains.
- **Exactly where** (a geometry problem) goes to fast deterministic solvers.
- **Active scouting and infiltration** gets one new squad archetype
  (#7 above).

| System | When / how many (neural) | Where / how (execution) | Why no separate brain |
| :--- | :--- | :--- | :--- |
| Radar, sonar, cameras (Beholder) | Factory + Eco builds when front-line coverage drops below target | Viewshed solver: local high ground for radar, cliff edges for cloaked cameras | A network that outputs (x, y) struggles to learn terrain line-of-sight across hundreds of maps |
| Targeting facility, Pinpointer (`armtarg`/`cortarg`/`legtarg`) | Factory + Eco queues 1–3 once long-range firepower passes a metal threshold | Placed safely inside the rear AA/anti-nuke cover | Radar wobble reduction follows a fixed curve *(Gemini said 50% → 75% → 87.5%; not verified)* |
| Anti-nuke: Citadel (`armamd`), Prevailer (`corfmd`), Aegis (`legabm`) | Strategic scales priority from scouted silos and game time | Coverage solver; see §6 for layering | Interception is radius and capacity maths |
| Juno (`armjuno`/`corjuno`/`legjuno`) | Strategic/Operational approves it when enemy jammers or mines stall a front | Fire at the densest cluster of jammers, mines or scouts | Fixed area and duration, simple targeting rule |
| Active recon and spy bots | Operational/Strategic picks high-uncertainty sectors | **Recon & special-ops squad brain** | **Does need a network:** moving around AA and timing EMP strikes is reactive control |

Sensors this adds:

- **Factory + Eco:** `frontline_radar_coverage_ratio` ∈ [0, 1] and a
  `radar_wobble_multiplier` based on how many targeting facilities are up.
- **Recon squad:** a coarse 16×16 "uncertainty map" where each cell is
  `time_since_last_los × suspected_enemy_presence`. The brain flies scouts and
  radar planes on paths through the most uncertain enemy cells while avoiding
  known AA. Spy bots (Ghost, Spectre) path around dense enemy screens, since
  bumping into units uncloaks them, and trigger their EMP on fusions,
  experimentals or anti-nukes just before a friendly nuke lands.

---

## 6. Saturation defence: nukes, anti-nukes and shields

This section comes from a correction by the user. Gemini first treated
anti-nuke as a matter of coverage plus stockpile. The user pointed out:

- an anti-nuke can fire at most **about seven interceptors** in the time one
  nuke takes to climb, travel and come down, however many it has stored;
- so anti-nukes can be **overwhelmed**, and failing to realise this is one of
  the current AI's critical mistakes;
- per metal, the **8th nuke launcher is worth vastly more than the 7th**;
- the same idea, with different maths, applies to **shields against massed
  LRPCs**;
- layered anti-nuke or shields over the eco base or key defences can be
  worth it in some cases, and anyone relying on a single anti-nuke layer at
  45 minutes will eat 10 nukes to their economy and lose.

Scripted BAR AIs treat anti-nukes and shields as yes/no coverage: once a
sector is inside a circle, it is marked safe. In fact both are
**throughput-limited buffers**.

### 6.1 Burst capacity and the 8th-nuke cliff

For a point (x, y) covered by the set of anti-nukes U(x, y), with s_i
interceptors stored in launcher i and B_max ≈ 7:

$$S_{\text{total}}(x,y) = \sum_{i \in U(x,y)} s_i \qquad\qquad C_{\text{burst}}(x,y) = \sum_{i \in U(x,y)} \min(s_i,\ B_{\max})$$

S_total matters for slow trickle exchanges. C_burst is the largest salvo that
point survives in one flight window. Against L fully loaded layers
(C_burst = 7L), the damage from N nukes is:

$$D(N, L) = \max(0,\ N - 7L) \times V_{\text{chain}}(x,y)$$

V_chain is the value lost to the blast plus chain explosions, which on a
late-game fusion and converter field can be tens of thousands of metal.
Against one layer, nukes 1–7 do no structural damage and only cost the
defender interceptors, so they have negative return. Nuke 8 gets through and
can end the game.

**Why plain RL misses this:** building 1, 3 or 6 silos and firing each as it
loads always gives a bad result. There is no gradient leading towards
"build 8 and hold fire until all 8 are loaded", and random exploration almost
never finds it.

### 6.2 Shields against LRPCs and rapid-fire artillery

Shields (Keeper `armgate`, Overseer `corgate`, Soteria `legdeflector`) facing
LRPCs (Basilica `armbrtha`, Basilisk `corint`, Olympus `leglrpc`) or T3
rapid-fire artillery (Ragnarok `armvulc`, Calamity `corbuzz`, Starfall
`legstarfall`) fail in two ways:

- **Sustained fire.** Shield charge Q_j regenerates at r_j while energy lasts.
  Once incoming plasma DPS Φ exceeds total regeneration, the array breaks in
  a set time:

  $$T_{\text{collapse}} = \frac{\sum_j Q_j}{\Phi - \sum_j r_j} \quad\text{when}\quad \Phi > \sum_j r_j \cdot \mathbb{1}(E > 0)$$

- **Energy brownout.** Shield regeneration and deflection cost a lot of
  energy. If a raid or a nuke clips one fusion, or factories spike energy use
  mid-barrage, energy hits zero, regeneration stops on every shield at once,
  and the whole array goes down within seconds.

**Stagger shields, don't stack them.** Shells hit the outer edge of the dome
nearest the attacker. Four shields in a tight square means the front one
drains while the rear ones sit full and regenerate nothing. Staggered in
depth along the threat direction, the second dome catches the next volleys
while the first one recharges.

### 6.3 What changes in the brains

No new top-level brain is needed, but three parts change:

1. **Replace coverage inputs with safety-margin inputs** for each high-value
   cluster (eco core, main defensive line):

   $$\Delta_{\text{nuke}}(x,y) = C_{\text{burst}}(x,y) - \max(\hat N_{\text{enemy silos}},\ N_{\text{time prior}}(t))$$

   $$\Delta_{\text{shield}}(x,y) = \frac{\sum_j r_j}{\max(1,\ \Phi_{\text{incoming}}(x,y))}$$

   N_time_prior(t) is a game-clock estimate of how many silos the enemy
   *could* have hidden (Gemini suggested 8–12 by 45 minutes in a long game),
   so a lack of scouting doesn't read as safety. Weight each cluster by its
   chain-explosion value. When Δ_shield falls below ~1.3, Factory + Eco should
   add staggered shields and spare energy before the first dome falls.

2. **Give Strategic a strike doctrine instead of letting it build one silo at
   a time.** It gets a discrete choice:
   - **0. None.** No silos; spend on army and economy.
   - **1. One silo plus spy EMP.** Punish an uncovered expansion, or pair it
     with a spy bot that stuns the enemy anti-nuke.
   - **2. Saturation salvo.** Pick a target cluster with estimated burst
     capacity Ĉ = 7L and have Factory + Eco build 7L + 2 silos (9 against one
     anti-nuke, 16 against two). A small deterministic **salvo gate** stops
     silos firing as they load. It holds them until ready missiles ≥ Ĉ + 1,
     then fires all of them on the same frame, with targets spread a few
     hundred elmos across the reactor field.

3. **Placement solver works from a required layer count per building.**
   Instead of covering uncovered buildings, give each building a target
   number of layers:

   $$L_{\text{req}}(b) = \left\lceil \frac{\text{cluster chain value}(b)}{25{,}000\ \text{metal}} \right\rceil \times \text{late-game threat multiplier}(t)$$

   Gemini's illustration: ~12k metal at 18 min → 1 layer; ~50k at 32 min →
   2 layers (burst 14); ~120k at 45 min → 4 layers (burst 28) plus a 4-deep
   staggered shield wall facing the enemy. The solver picks the tile that
   covers the most value among buildings below their target.

### 6.4 Training for late-game saturation

Most normal matches end before 30 minutes, so the brains rarely see a
10-nuke salvo or an artillery duel. In the Factory + Eco and Strategic
sandboxes (§9.2):

1. Start some episodes from ~1,000 snapshots taken at 30–40 minutes in strong
   players' replays.
2. In ~30% of episodes, spawn an enemy 8-silo battery, or an LRPC or Ragnarok
   battery, firing on a timer.
3. After Strategic commits to doctrine 2, add a shaping reward of
   σ(N_ready − C_burst(target)). This turns the 7-silo cliff into a slope it
   can climb, so it learns "never stop at 6 silos against one anti-nuke: go
   to 8 or don't build them at all".

---

## 7. Sensors

### 7.1 What a sensor must do

Raw lists of thousands of units are too large for a small network, and their
order would change the output (swapping two identical Grunts in the list
shouldn't change anything). Each brain needs a fixed-width input. Before
adding a sensor, check that it:

1. **Includes rates of change.** A feed-forward network can't infer momentum,
   so pass net flow (income + reclaim − spend) and a ~5 s moving average, not
   just stored metal.
2. **Doesn't depend on unit count or order.** Group variable-length unit
   lists into fixed bins or rings around the platoon centre, so the input is
   the same size for 4 units or 40.
3. **Doesn't cheat.** Respect BAR's visibility levels:
   - *Line of sight:* exact unit, HP, reload state, velocity.
   - *Radar blip:* position with radar wobble, rough speed, air or ground,
     but no unit type unless it was scouted earlier.
   - *Jammed or stealthed:* nothing. Use a decaying memory of the last known
     threat instead, e.g. weight e^(−λΔt).

### 7.2 Normalisation

- **Signed, unbounded values** (net energy, local metal difference):
  x' = sign(x) · ln(1 + |x|). This keeps small early-game differences visible
  while compressing late-game scale.
- **Capacities and health:** ratios in [0, 1] (storage, HP sum / max HP sum,
  EMP level, build progress).
- **Angles** (heading, wind direction): (sin θ, cos θ), never raw degrees,
  which jump from 360 back to 0.

### 7.3 Example inputs per brain

- **Factory + Eco (~64–96 floats):** metal and energy storage ratios, net
  income and drain, reclaimable metal near the base, factory build-power
  saturation, min/max/current wind, tidal strength, mex spots vs. claimed T1
  and T2 mexes, tech flags (T1 lab, T2 lab, T2 constructor, fusion count,
  advanced converter count), radar coverage, saturation margins (§6.3).
- **Platoon (~128–256 floats):** centre (x, y), velocity, weapon-range
  quantiles, HP ratio, total DPS, slowest unit's speed, share of anti-swarm,
  anti-armour and AA weapons. Plus **threat rings**: 3 distances (0–350,
  350–800, 800–1600 elmos) × 8 directions = 24 cells, each holding friendly
  metal, visible enemy metal, enemy static-defence DPS, terrain slope and
  wreck density.
- **Squad:** everything in the platoon-relative frame (§3.2), plus the unit
  stats vector (§3.3).

### 7.4 Pruning sensors

Start from a large candidate set pulled from replays, then cut it down:

1. Drop one of any pair of features with Spearman correlation above ~0.9 over
   ~10k replay frames, or use the variance inflation factor.
2. Train a supervised model to predict strong players' actions, then shuffle
   one input column at a time on the validation set. If loss rises by less
   than ~0.5%, drop that input.

---

## 8. Network shapes

Inference shares each 30 Hz frame with pathfinding, collision and Lua
widgets, so the networks must be small. For vector inputs, **shallow,
moderately wide networks (2–4 hidden layers)** beat deep, narrow ones.

| Brain | Hidden layers | Notes |
| :--- | :--- | :--- |
| Strategic, Factory + Eco | 256 → 256 → 128 | Runs at most once a second, so it can afford more weights |
| Platoon | 256 → 128 → 64 | LayerNorm, residual connections |
| Squad / per-unit | 64 → 64 | Many instances, so keep it small enough for CPU cache; int8 or fp16 quantised |

**Variable numbers of entities.** When a brain needs to look at individual
enemies (focus fire, D-Gun target), add a small Deep Sets encoder in front of
the main network. Pass each entity through a shared small network φ, then
concatenate the mean and the max over all N entities into a fixed-size
vector:

$$z = \left[\ \tfrac{1}{N}\textstyle\sum_i \phi(e_i)\ \Big\|\ \max_i \phi(e_i)\ \right]$$

| Part | Choice | Why |
| :--- | :--- | :--- |
| Hidden layers | SiLU or LeakyReLU (α = 0.01) | Avoids dead units when sparse inputs (nuke warning, EMP) are zero most of the game |
| Normalisation | LayerNorm, before activation | Works at batch size 1 during live play, unlike BatchNorm |
| Discrete choices | Masked softmax | Invalid actions get −∞ before the softmax |
| Continuous positions | tanh on [−1, 1], or a Beta distribution | Bounded offsets, no runaway coordinates |
| Picking a target | Dot-product attention scores (q·k / √d) | Scores each candidate enemy against the brain's query |

**Always mask invalid actions.** Don't make the network learn rules that are
known in advance through penalties. With no T2 lab, mask out T2-only builds.
This removes wasted exploration and speeds up learning a lot.

$$P(a_i \mid s) = \frac{\exp(l_i + \ln m_i)}{\sum_j \exp(l_j + \ln m_j)}, \qquad m_i \in \{0, 1\}$$

---

## 9. Training pipeline

The user expected that most brains could be trained separately at first,
greatly cutting complexity. Gemini agreed. Trained end to end from the start,
the joint action space is the product of every brain's choices, and the
credit horizon is ~30,000 frames. A good Strategic decision gets punished
because an untrained squad walks its units into a lake. Training each brain
alone, with made-up orders from above and fixed (mock or frozen) brains
below, changes the cost from **multiplicative to additive**.

### 9.1 Stage 1: imitation from replays

BAR replays (`.sdfz`) store every command, so they can be replayed exactly.

1. Filter for strong players (Gemini suggested OpenSkill > 30) in 1v1 and
   team front/eco roles.
2. Step through each replay in a headless engine as fast as possible,
   sampling (state, action) pairs at each brain's tick rate.
3. Pre-train Strategic, Factory + Eco and Platoon with cross-entropy on
   discrete choices plus MSE on continuous outputs:
   L = −Σ y log p̂ + λ‖u − û‖².

### 9.2 Stage 2: separate sandboxes, bottom up

| Module | Sandbox | Made-up input from above | Reward | Speed-up (Gemini's estimate) |
| :--- | :--- | :--- | :--- | :--- |
| **A. Squad** (each archetype) | 30 s combat arenas on a small map patch, no base or economy | Random anchor and threat axis | Metal dealt / metal lost, plus a formation cohesion bonus | ~50× |
| **B. Factory + Eco** | 15 min economy run, "SimCity" style, no enemy | Random budget splits **w** ~ Dirichlet(α) every 30–60 s, plus fake events (lose 2 mexes, spawn 1,500 metal of wrecks) | Throughput minus stall and overflow penalties, plus how closely spending matches **w** (below) | ~100× |
| **C. Platoon** | 3 min sector fights (e.g. 40v40 mixed arms on a canyon) | Fixed sector objective; **frozen** squad brains carry out its orders | Time holding the sector, survival, enemy value destroyed | ~15× |
| **D. Strategic** | Full matches or a fast macro simulator | Real game state; drives **frozen** B and C | Win/loss ±1, plus potential-based shaping on total value | ~10× fewer episodes |

Factory + Eco reward:

$$R_{\text{eco}} = \ln\!\left(1 + M_{\text{spent}} + \tfrac{E_{\text{spent}}}{70}\right) - \lambda_{\text{align}}\, D_{\text{KL}}(\mathbf{w}_{\text{target}} \,\|\, \mathbf{w}_{\text{actual}}) - \lambda_{\text{stall}}\, \mathbb{1}(E_{\text{storage}} < 0.03)$$

(70 energy per metal matches the T1 Energy Converter, `armmakr`:
`energyconv_capacity = 70`, `energyconv_efficiency = 0.01429`.)

Module B is especially strong: the economy follows fixed rules plus random
wind, so it can learn excellent economic scaling without simulating a single
battle. Vary maps (high wind, tidal, low metal).

Platoon/squad reward in full-game sandboxes (Gemini's original version):

$$R_{\text{tactical}} = \frac{\Delta M_{\text{enemy destroyed}} - 1.2\,\Delta M_{\text{friendly lost}}}{1000} + \beta\,\mathbb{1}(\text{sector captured}) - c_{\text{idle}}$$

The 1.2 weight on own losses discourages leaving wrecks for the enemy to
reclaim. Gemini proposed PPO with GAE for these stages.

### 9.3 Stage 3: putting the brains together without breaking them

Brains trained alone can misbehave when connected: Strategic asks for things
the economy can't deliver, and Platoon orders a damaged squad to hold a choke
out of reach of repair. Three rules:

1. **Train lower brains on the full range of inputs.** Sample
   **w** ~ Dirichlet(0.8) so Factory + Eco sees extreme orders (90% army
   panic, 80% eco boom). Nothing Strategic later asks for is then new to it.
2. **Send status upward** (§4.3) so the upper brain can see what is feasible.
3. **Freeze, then thaw.** First freeze Squad, Platoon and Factory + Eco
   (learning rate 0) and train only Strategic (Gemini suggested ~5,000
   matches) until its value estimate settles. Then unfreeze the lower brains
   at a 10× lower learning rate (e.g. 3e-5 vs. 3e-4) so they adapt to
   Strategic's style without forgetting their skills.

### 9.4 Stage 4: league self-play

To avoid rock-paper-scissors cycling (e.g. learning only Tick rushes against
an opponent that forgot one LLT), keep a pool of opponents:

- ~50% the current policy;
- ~30% past checkpoints and scripted AIs (e.g. BARbarian at different
  handicaps), so it never forgets how to hold scripted all-ins;
- ~20% "exploiter" policies trained only to find weak spots (amphibious
  switches, EMP bomber commander snipes).

When a balance patch changes units, or new inputs are added, use
OpenAI Five-style **network surgery**: add new input weights initialised to
zero and keep the existing hidden layers, instead of retraining from scratch.

---

## 10. Open questions

- **Is the operational / theater brain needed?** Gemini's first answer
  included it, for wide team-game fronts. The user's
  Strategic → Platoon → Squad design has Strategic assign each platoon its
  task directly, and Gemini's later answers worked without it. It probably
  matters only for large team games and could start as part of Strategic.
- **Squad tick rate.** Gemini gave the bottom tier 10–15 Hz when it was
  per-unit micro, but 3–5 Hz (every 6–10 frames) once it became squad
  formations. The lower rate fits group orders better.
- **Where it runs.** The conversation never settled the runtime or its CPU
  budget.
- **Unverified numbers.** The ~7 interceptor burst limit comes from the user.
  Most other figures (radar wobble curve, ranges, reward weights, layer
  widths, speed-ups, the 25,000-metal layer rule) are Gemini's and still need
  checking against the unit defs or by experiment.

---

## Corrections to Gemini's BAR facts

Checked against `asym/units.json` (`units.names`) on 2026-10-19. "Not a BAR
name" means no current unit has that display name. Many look like older
Spring/TA-era names.

| Gemini said | Actually |
| :--- | :--- |
| Anti-nuke "Fortitude / Foil" | Citadel (`armamd`), Prevailer (`corfmd`), Aegis (`legabm`) |
| Mobile anti-nuke "Scarab / Mab" | Umbrella (`armscab`), Saviour (`cormabm`) |
| "Keeper / Saviour" as shields | Keeper (`armgate`) is a shield; Saviour is the Cortex **mobile anti-nuke**. Cortex shield is Overseer (`corgate`); Legion is Soteria (`legdeflector`). |
| LRPC "Intimidator" | Not a BAR name. LRPCs: Basilica (`armbrtha`), Basilisk (`corint`), Olympus (`leglrpc`) |
| T3 artillery "Vulcan / Buzzsaw" | Old names. Ragnarok (`armvulc`), Calamity (`corbuzz`), Starfall (`legstarfall`) |
| Targeting facility "Ichnaea" | Pinpointer (`armtarg`/`cortarg`/`legtarg`). Ichnaea (`legsd`) is Legion's intrusion detector, like Tracer (`armsd`) and Nemesis (`corsd`). |
| "Castro" as a mobile jammer escorting a squad | Castro (`corjamt`) is a static jammer tower. Mobile jammers: Deceiver (`corspec`), Smuggler (`armaser`). |
| "Sheldon" as a 700-range T2 skirmisher | Sheldon (`cormort`) is a T1 mortar bot. |
| "Rocko", "Storm" | Rocketeer (`armrock`), Aggravator (`corstorm`) |
| "Necro" | Graverobber (`cornecro`) |
| "Pyro" | Fiend (`corpyro`). "Pyro" only matches a raptor unit. |
| "Stumpy", "Goliath", "Reaper" | Stout (`armstump`), Tzar (`corgol`), Tiger (`correap`) |
| "Morty", "Luger", "Freaker", "Copperhead", "Eraser" | Not BAR names |
| Scouts "Fink / Peeper", radar planes "Eagle / Vulture" | Finch (`corfink`), Blink (`armpeep`); radar planes Oracle (`armawac`), Condor (`corawac`) |
| Spy bots "Spy / Parasite" | Ghost (`armspy`), Spectre (`corspy`) |
| T1 converter, 70 E → 1 M | Correct: `armmakr` has `energyconv_capacity = 70`, `energyconv_efficiency = 0.01429` |

Correct as used: Sumo (`corcan`), Mammoth (`corsumo`), Pawn, Grunt, Tick,
Rover, Lazarus, Bull, Banisher, Janus, Wolverine, Incisor, Gremlin, Phalanx,
Consul, Juggernaut, Ragnarok, Thor, Basilisk, Calamity, Beholder, Juno, Nyx,
Marauder, Pulsar, Sharpshooter.

---

## How the conversation went

1. **Prompt:** an in-depth, cited guide to training MLPs for an RTS. How to
   choose sensors and perceptrons, how they should vary, and how many layers
   or brain types BAR needs ("is it as simple as strategic + platoon?").
   **Answer:** a 5-tier hierarchy (strategic, economy, operational, platoon,
   unit micro), sensor rules, network sizes, and a 4-stage pipeline
   (imitation → bottom-up RL → full game → league).
2. **Prompt:** put it in a Markdown file. *(Gemini's reply to this turn was
   empty in the share.)*
3. **User's idea:** Strategic → Platoon → Squad instead of per-unit brains.
   **Answer:** agreed. Added the platoon-relative frame, archetype brains with
   unit stats, squads of one, and deterministic per-unit reflexes.
4. **User's idea:** a Factory + Eco brain separate from Strategic, with brains
   trained separately at first. **Answer:** agreed, but factory and economy
   must stay together. Added the budget split, the upward status summary and
   the sandbox modules A–D.
5. **Prompt:** are support brains needed for intel, radar or anti-nuke?
   **Answer:** hybrid split. Timing goes in existing brains, placement in
   deterministic solvers, and one new recon/special-ops squad archetype.
6. **User's correction:** anti-nukes and shields can be overwhelmed. Burst of
   ~7 interceptors, the 8th nuke matters most, layering is needed late.
   **Answer:** burst-capacity and flux maths, safety-margin sensors, strike
   doctrines with a salvo gate, layer-count placement, late-game crisis
   training.
7. **Prompt:** generate the full guide as one Markdown file. **Answer:**
   ignored. Gemini had run out of credits and dropped from Argon 4 to
   Flash-Lite 3.5, and the reply went back to the 5-tier layout and left out
   most of the later design. This document replaces it.
