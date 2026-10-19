# Council report: `tactical_v3`

6 modules, 420 parameters. Each module is a small perceptron with one job; the formula that combines them is fixed code (see `learn/tactical.rs`). Behaviour below is measured from 6000 sampled decisions.

## Results on validation battles

| scenario | opponent | W | timeout | wipe-out | L | win rate | 95% CI | mean exchange |
|---|---|---|---|---|---|---|---|---|
| 1v1 | charge | 42 | 0 | 2 | 4 | 88% | 75–94% | +0.09 |
| 1v1 | kite | 48 | 0 | 0 | 0 | 100% | 93–100% | +0.05 |
| 1v1 | orbit | 48 | 0 | 0 | 0 | 100% | 93–100% | +0.11 |
| 3v3 | charge | 46 | 0 | 0 | 2 | 96% | 86–99% | +0.10 |
| 3v3 | kite | 48 | 0 | 0 | 0 | 100% | 93–100% | +0.14 |
| 3v3 | orbit | 43 | 5 | 0 | 0 | 90% | 78–95% | +0.14 |
| 2v3 underdog | charge | 0 | 0 | 0 | 48 | 0% | 0–7% | -0.01 |
| 2v3 underdog | kite | 48 | 0 | 0 | 0 | 100% | 93–100% | +0.34 |
| 2v3 underdog | orbit | 27 | 21 | 0 | 0 | 56% | 42–69% | +0.27 |
| 3v5 underdog | charge | 0 | 0 | 0 | 48 | 0% | 0–7% | -0.01 |
| 3v5 underdog | kite | 48 | 0 | 0 | 0 | 100% | 93–100% | +0.35 |
| 3v5 underdog | orbit | 0 | 48 | 0 | 0 | 0% | 0–7% | +0.17 |
| 5v3 favourite | charge | 48 | 0 | 0 | 0 | 100% | 93–100% | +0.11 |
| 5v3 favourite | kite | 48 | 0 | 0 | 0 | 100% | 93–100% | +0.25 |
| 5v3 favourite | orbit | 48 | 0 | 0 | 0 | 100% | 93–100% | +0.20 |

Overall win rate 75%, mean fitness +0.88, **43% in battles where it was the underdog** (288 battles).

## `target_selector`

Scores one enemy at a time (same weights for every candidate); the highest-scoring of the three nearest enemies becomes the target.

Shape 6 → 6 → 1 · 49 parameters · 1382 samples

### output `score`

mean -0.93 · 10th pct -1.00 · median -0.98 · 90th pct -0.88 · strongly active (|x| > 0.5) 99% of the time

What it listens to, most influential first. *Swing* is the average change in `score` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `distance_over_range` | 0.95 – 3.00 | 0.05 | higher → lowers `score` | `████████████████████` |
| `enemy_hp_fraction` | 0.25 – 0.97 | 0.04 | higher → lowers `score` | `██████████████······` |
| `is_nearest` | 0.00 – 1.00 | 0.03 | higher → raises `score` | `█████████████·······` |
| `enemy_range_over_mine` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `enemy_threat_over_mine` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `enemy_value_over_mine` | 1.00 – 1.00 | 0.00 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `distance_over_range` | 0.11 → +0.14 | 0.83 → -0.95 | 1.55 → -0.99 | 2.28 → -0.99 | 3.00 → -1.00 |
| `enemy_hp_fraction` | 0.00 → -0.96 | 0.25 → -0.97 | 0.50 → -0.98 | 0.75 → -0.98 | 1.00 → -0.99 |
| `is_nearest` | 0.00 → -0.98 | 0.25 → -0.97 | 0.50 → -0.96 | 0.75 → -0.95 | 1.00 → -0.94 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias +0.24: reads -2.08`distance_over_range` +1.58`enemy_threat_over_mine` -0.86`enemy_hp_fraction`; pushes +1.33 → `score`
- **h1** bias +0.89: reads -0.99`enemy_value_over_mine` -0.35`enemy_threat_over_mine` +0.28`enemy_range_over_mine`; pushes +0.48 → `score`
- **h2** bias +0.00: reads -0.75`distance_over_range` -0.60`enemy_threat_over_mine` -0.30`enemy_range_over_mine`; pushes +0.09 → `score`
- **h3** bias -2.10: reads -0.86`is_nearest` +0.74`distance_over_range` +0.61`enemy_hp_fraction`; pushes -0.83 → `score`
- **h4** bias -0.04: reads -0.97`enemy_value_over_mine` -0.97`enemy_range_over_mine` -0.77`is_nearest`; pushes +0.25 → `score`
- **h5** bias -0.21: reads -1.11`distance_over_range` -1.03`is_nearest` -0.72`enemy_threat_over_mine`; pushes +0.78 → `score`

## `spacing`

Owns range: close on the target (+1) or back away from it (-1) along the line between them.

Shape 11 → 6 → 1 · 79 parameters · 1500 samples

### output `advance`

mean -0.22 · 10th pct -1.00 · median -0.74 · 90th pct +0.99 · strongly active (|x| > 0.5) 85% of the time

What it listens to, most influential first. *Swing* is the average change in `advance` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `my_hp_fraction` | 0.40 – 1.00 | 1.14 | higher → raises `advance` | `████████████████████` |
| `target_approaching` | -0.96 – 1.00 | 1.06 | higher → lowers `advance` | `███████████████████·` |
| `distance_over_range` | 0.83 – 1.15 | 0.98 | higher → raises `advance` | `█████████████████···` |
| `platoon_theirs_over_ours_power` | 0.21 – 1.00 | 0.74 | higher → lowers `advance` | `█████████████·······` |
| `theirs_over_ours_dps` | 0.50 – 1.00 | 0.64 | higher → lowers `advance` | `███████████·········` |
| `my_reload_remaining` | 0.00 – 0.78 | 0.55 | higher → lowers `advance` | `██████████··········` |
| `target_hp_fraction` | 0.11 – 0.96 | 0.17 | higher → lowers `advance` | `███·················` |
| `platoon_ours_over_theirs_power` | 0.99 – 1.00 | 0.01 | ignored | `····················` |
| `target_range_over_mine` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_dps` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `my_range_rank` | 1.00 – 1.00 | 0.00 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `distance_over_range` | 0.11 → -1.00 | 0.83 → -0.99 | 1.55 → +1.00 | 2.28 → +1.00 | 3.00 → +1.00 |
| `my_reload_remaining` | 0.00 → +0.02 | 0.22 → -0.56 | 0.44 → -0.88 | 0.67 → -0.97 | 0.89 → -0.99 |
| `my_hp_fraction` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → +0.07 | 1.00 → +0.99 |
| `target_hp_fraction` | 0.00 → +0.19 | 0.25 → +0.01 | 0.50 → -0.21 | 0.75 → -0.43 | 1.00 → -0.63 |
| `target_approaching` | -1.00 → +0.66 | -0.50 → +0.64 | 0.00 → +0.18 | 0.50 → -0.91 | 1.00 → -1.00 |
| `ours_over_theirs_dps` | 0.33 → +0.38 | 0.50 → +0.27 | 0.67 → +0.11 | 0.83 → -0.08 | 1.00 → -0.29 |
| `theirs_over_ours_dps` | 0.00 → +1.00 | 0.25 → +1.00 | 0.50 → +0.98 | 0.75 → +0.82 | 1.00 → -0.29 |
| `platoon_ours_over_theirs_power` | 0.77 → -0.80 | 0.83 → -0.72 | 0.89 → -0.61 | 0.94 → -0.46 | 1.00 → -0.29 |
| `platoon_theirs_over_ours_power` | 0.00 → +0.99 | 0.25 → +0.98 | 0.50 → +0.85 | 0.75 → +0.04 | 1.00 → -0.87 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -1.91: reads +3.38`distance_over_range` +2.74`my_hp_fraction` -1.38`target_range_over_mine`; pushes +3.58 → `advance`
- **h1** bias +19.88: reads +40.69`my_reload_remaining` -38.11`distance_over_range` +1.07`theirs_over_ours_dps`; pushes -0.02 → `advance`
- **h2** bias +0.05: reads -0.77`target_hp_fraction` -0.59`my_range_rank` -0.22`theirs_over_ours_dps`; pushes +1.06 → `advance`
- **h3** bias +0.13: reads -2.69`my_hp_fraction` +1.77`target_approaching` -1.16`distance_over_range`; pushes -2.13 → `advance`
- **h4** bias +0.19: reads +1.55`distance_over_range` -1.26`target_range_over_mine` -1.15`theirs_over_ours_dps`; pushes +0.13 → `advance`
- **h5** bias -0.16: reads -1.14`target_approaching` -1.10`my_reload_remaining` +0.86`distance_over_range`; pushes +0.60 → `advance`

## `strafe`

Owns sideways motion: circle the target counter-clockwise (+1) or clockwise (-1), e.g. to dodge a projectile weapon.

Shape 8 → 6 → 1 · 61 parameters · 1500 samples

### output `orbit`

mean +0.09 · 10th pct -0.01 · median +0.09 · 90th pct +0.17 · strongly active (|x| > 0.5) 0% of the time

What it listens to, most influential first. *Swing* is the average change in `orbit` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `my_hp_fraction` | 0.40 – 1.00 | 0.15 | higher → lowers `orbit` | `████████████████████` |
| `target_approaching` | -0.96 – 1.00 | 0.12 | higher → raises `orbit` | `████████████████····` |
| `theirs_over_ours_dps` | 0.50 – 1.00 | 0.11 | higher → raises `orbit` | `███████████████·····` |
| `my_reload_remaining` | 0.00 – 0.78 | 0.02 | ignored | `███·················` |
| `distance_over_range` | 0.83 – 1.15 | 0.01 | ignored | `█···················` |
| `target_range_over_mine` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_dps` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `my_speed_rank` | 1.00 – 1.00 | 0.00 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `distance_over_range` | 0.11 → +0.09 | 0.83 → +0.10 | 1.55 → +0.12 | 2.28 → +0.16 | 3.00 → +0.19 |
| `my_reload_remaining` | 0.00 → +0.11 | 0.22 → +0.10 | 0.44 → +0.09 | 0.67 → +0.09 | 0.89 → +0.09 |
| `target_approaching` | -1.00 → +0.02 | -0.50 → +0.06 | 0.00 → +0.09 | 0.50 → +0.12 | 1.00 → +0.15 |
| `my_hp_fraction` | 0.00 → +0.23 | 0.25 → +0.20 | 0.50 → +0.16 | 0.75 → +0.10 | 1.00 → +0.02 |
| `ours_over_theirs_dps` | 0.33 → +0.08 | 0.50 → +0.08 | 0.67 → +0.09 | 0.83 → +0.10 | 1.00 → +0.10 |
| `theirs_over_ours_dps` | 0.00 → -0.11 | 0.25 → -0.07 | 0.50 → -0.01 | 0.75 → +0.04 | 1.00 → +0.10 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -0.31: reads +0.77`my_reload_remaining` -0.69`target_range_over_mine` -0.35`distance_over_range`; pushes +0.08 → `orbit`
- **h1** bias -0.03: reads -1.33`my_hp_fraction` -0.80`my_reload_remaining` +0.76`theirs_over_ours_dps`; pushes +0.21 → `orbit`
- **h2** bias +1.12: reads -1.18`my_reload_remaining` -0.94`theirs_over_ours_dps` +0.93`ours_over_theirs_dps`; pushes -0.08 → `orbit`
- **h3** bias -0.53: reads -0.66`my_reload_remaining` +0.50`ours_over_theirs_dps` +0.45`theirs_over_ours_dps`; pushes +0.02 → `orbit`
- **h4** bias +0.19: reads +0.90`theirs_over_ours_dps` -0.89`my_hp_fraction` -0.38`my_speed_rank`; pushes +0.03 → `orbit`
- **h5** bias -0.14: reads -0.53`my_reload_remaining` +0.52`my_hp_fraction` -0.45`distance_over_range`; pushes -0.02 → `orbit`

## `cohesion`

Owns group shape: drift toward the centre of friendly units (+1) or spread away from it (-1). Ignored when the unit has no allies.

Shape 5 → 6 → 1 · 43 parameters · 1286 samples

### output `regroup`

mean -0.22 · 10th pct -0.38 · median -0.22 · 90th pct -0.11 · strongly active (|x| > 0.5) 0% of the time

What it listens to, most influential first. *Swing* is the average change in `regroup` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `ally_centroid_distance_over_range` | 1.38 – 3.00 | 0.30 | higher → raises `regroup` | `████████████████████` |
| `distance_over_range` | 0.81 – 1.81 | 0.19 | higher → raises `regroup` | `█████████████·······` |
| `my_hp_fraction` | 0.40 – 1.00 | 0.09 | higher → lowers `regroup` | `██████··············` |
| `nearest_ally_distance_over_range` | 1.35 – 3.00 | 0.09 | higher → lowers `regroup` | `██████··············` |
| `ours_over_theirs_count` | 0.67 – 1.00 | 0.01 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `ally_centroid_distance_over_range` | 0.25 → -0.58 | 0.94 → -0.53 | 1.62 → -0.42 | 2.31 → -0.27 | 3.00 → -0.16 |
| `nearest_ally_distance_over_range` | 0.17 → -0.07 | 0.87 → -0.12 | 1.58 → -0.16 | 2.29 → -0.19 | 3.00 → -0.24 |
| `my_hp_fraction` | 0.00 → -0.11 | 0.25 → -0.15 | 0.50 → -0.19 | 0.75 → -0.23 | 1.00 → -0.26 |
| `ours_over_theirs_count` | 0.33 → -0.20 | 0.50 → -0.20 | 0.67 → -0.20 | 0.83 → -0.21 | 1.00 → -0.21 |
| `distance_over_range` | 0.11 → -0.33 | 0.83 → -0.24 | 1.55 → -0.10 | 2.28 → +0.04 | 3.00 → +0.12 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -1.24: reads +0.90`ally_centroid_distance_over_range` -0.30`nearest_ally_distance_over_range` -0.20`my_hp_fraction`; pushes +0.25 → `regroup`
- **h1** bias +0.26: reads +0.82`ally_centroid_distance_over_range` -0.54`my_hp_fraction` -0.51`nearest_ally_distance_over_range`; pushes +0.01 → `regroup`
- **h2** bias -0.61: reads +0.85`nearest_ally_distance_over_range` -0.75`my_hp_fraction` -0.40`ally_centroid_distance_over_range`; pushes +0.08 → `regroup`
- **h3** bias +0.70: reads +0.87`distance_over_range` -0.55`my_hp_fraction` -0.47`ours_over_theirs_count`; pushes +0.20 → `regroup`
- **h4** bias -0.38: reads -0.64`ally_centroid_distance_over_range` +0.49`ours_over_theirs_count` -0.27`my_hp_fraction`; pushes -0.10 → `regroup`
- **h5** bias -0.31: reads +0.44`ours_over_theirs_count` +0.22`ally_centroid_distance_over_range` -0.20`my_hp_fraction`; pushes +0.16 → `regroup`

## `engagement`

Owns fight or flight: reads the friend/enemy force balance and, above zero, pulls the unit out of the fight by that fraction, running away from the enemy force. At or below zero it fights. Never retreats while the platoon is clearly ahead (the doctrine).

Shape 22 → 6 → 1 · 145 parameters · 1500 samples

### output `retreat`

mean -1.00 · 10th pct -1.00 · median -1.00 · 90th pct -1.00 · strongly active (|x| > 0.5) 100% of the time

What it listens to, most influential first. *Swing* is the average change in `retreat` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `my_hp_fraction` | 0.40 – 1.00 | 0.00 | ignored | `████████████████████` |
| `theirs_over_ours_hp` | 0.23 – 1.00 | 0.00 | ignored | `█████████████·······` |
| `theirs_over_ours_count` | 0.50 – 1.00 | 0.00 | ignored | `█████████···········` |
| `ours_over_theirs_hp` | 0.86 – 1.00 | 0.00 | ignored | `██████··············` |
| `theirs_over_ours_dps_mid` | 0.50 – 1.00 | 0.00 | ignored | `████················` |
| `platoon_theirs_over_ours_power` | 0.21 – 1.00 | 0.00 | ignored | `████················` |
| `theirs_over_ours_value` | 0.50 – 1.00 | 0.00 | ignored | `████················` |
| `theirs_over_ours_dps` | 0.50 – 1.00 | 0.00 | ignored | `███·················` |
| `platoon_ours_over_theirs_power` | 0.99 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_count` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_dps` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_value` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_speed` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_speed` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_dps_short` | 0.00 – 0.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_dps_short` | 0.00 – 0.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_dps_mid` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_dps_long` | 0.00 – 0.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_dps_long` | 0.00 – 0.00 | 0.00 | ignored | `····················` |
| `my_speed_rank` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `my_range_rank` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `my_hp_rank` | 1.00 – 1.00 | 0.00 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `ours_over_theirs_count` | 0.33 → -1.00 | 0.50 → -1.00 | 0.67 → -1.00 | 0.83 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_count` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_hp` | 0.34 → -1.00 | 0.50 → -1.00 | 0.67 → -1.00 | 0.83 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_hp` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_dps` | 0.33 → -1.00 | 0.50 → -1.00 | 0.67 → -1.00 | 0.83 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_dps` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_value` | 0.33 → -1.00 | 0.50 → -1.00 | 0.67 → -1.00 | 0.83 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_value` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_speed` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_dps_mid` | 0.33 → -1.00 | 0.50 → -1.00 | 0.67 → -1.00 | 0.83 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_dps_mid` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `my_hp_fraction` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `platoon_ours_over_theirs_power` | 0.77 → -1.00 | 0.83 → -1.00 | 0.89 → -1.00 | 0.94 → -1.00 | 1.00 → -1.00 |
| `platoon_theirs_over_ours_power` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -2.93: reads -4.78`platoon_ours_over_theirs_power` +3.08`platoon_theirs_over_ours_power` -1.11`theirs_over_ours_hp`; pushes +1.62 → `retreat`
- **h1** bias +0.09: reads -1.70`my_hp_rank` +1.43`ours_over_theirs_hp` +1.37`theirs_over_ours_hp`; pushes -0.16 → `retreat`
- **h2** bias -1.07: reads -1.71`theirs_over_ours_dps_long` +1.37`my_hp_rank` -1.07`my_hp_fraction`; pushes +0.16 → `retreat`
- **h3** bias -0.55: reads +0.95`ours_over_theirs_dps` -0.93`theirs_over_ours_dps_short` -0.71`my_hp_fraction`; pushes +0.11 → `retreat`
- **h4** bias -1.20: reads +1.26`ours_over_theirs_hp` +1.14`my_hp_fraction` +1.08`theirs_over_ours_value`; pushes -1.03 → `retreat`
- **h5** bias -0.62: reads +1.33`ours_over_theirs_dps_short` -0.92`theirs_over_ours_dps_mid` -0.78`theirs_over_ours_dps`; pushes +0.05 → `retreat`

## `encircle`

Owns the formation around the enemy force: slides the unit around it (+1 counter-clockwise) toward the larger gap between itself and its neighbours, so friends spread into an even ring, and away from the enemy's line of advance, so they flank instead of bunching in front. Scaled up as the platoon gets further ahead.

Shape 5 → 6 → 1 · 43 parameters · 1453 samples

### output `bloom`

mean -0.27 · 10th pct -0.87 · median -0.45 · 90th pct +0.97 · strongly active (|x| > 0.5) 58% of the time

What it listens to, most influential first. *Swing* is the average change in `bloom` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `gap_cw` | 0.19 – 1.00 | 1.44 | higher → lowers `bloom` | `████████████████████` |
| `gap_ccw` | 0.17 – 1.00 | 0.86 | higher → raises `bloom` | `████████████········` |
| `enemy_side` | -0.98 – 0.66 | 0.55 | higher → lowers `bloom` | `████████············` |
| `enemy_front_alignment` | -0.99 – 1.00 | 0.34 | higher → raises `bloom` | `█████···············` |
| `ring_distance_over_range` | 0.84 – 1.57 | 0.12 | higher → raises `bloom` | `██··················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `enemy_front_alignment` | -1.00 → -0.85 | -0.50 → -0.82 | 0.00 → -0.74 | 0.50 → -0.59 | 1.00 → -0.43 |
| `enemy_side` | -1.00 → +0.02 | -0.50 → -0.56 | -0.00 → -0.69 | 0.50 → -0.75 | 1.00 → -0.80 |
| `gap_ccw` | 0.00 → -0.91 | 0.25 → -0.90 | 0.50 → -0.89 | 0.75 → -0.87 | 1.00 → -0.69 |
| `gap_cw` | 0.02 → +0.98 | 0.26 → +0.98 | 0.51 → +0.98 | 0.75 → +0.96 | 1.00 → -0.69 |
| `ring_distance_over_range` | 0.11 → -0.73 | 0.80 → -0.70 | 1.49 → -0.68 | 2.18 → -0.65 | 2.87 → -0.61 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias +0.05: reads -8.32`gap_cw` +7.01`gap_ccw` -0.83`enemy_side`; pushes +1.87 → `bloom`
- **h1** bias +0.76: reads +2.75`enemy_side` -0.65`gap_ccw` +0.36`gap_cw`; pushes +0.26 → `bloom`
- **h2** bias +0.17: reads +1.21`enemy_front_alignment` -0.97`ring_distance_over_range` +0.65`enemy_side`; pushes +0.08 → `bloom`
- **h3** bias +1.42: reads -1.39`enemy_front_alignment` -1.09`gap_ccw` +0.82`enemy_side`; pushes -0.39 → `bloom`
- **h4** bias +0.23: reads -0.93`enemy_front_alignment` +0.61`enemy_side` +0.56`ring_distance_over_range`; pushes +0.37 → `bloom`
- **h5** bias +0.19: reads -0.98`gap_ccw` -0.63`ring_distance_over_range` +0.62`enemy_front_alignment`; pushes -0.31 → `bloom`

## Worked decisions

**t = 0.0s, unit #1** at (850, 500) chose enemy #2
- `target_selector` [enemy #2 (rank 0)]: distance_over_range=1.67, enemy_hp_fraction=1.00, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=1.00, is_nearest=1.00 → **score=-0.96**
- `spacing`: distance_over_range=1.67, my_reload_remaining=0.00, my_hp_fraction=1.00, target_range_over_mine=1.00, target_hp_fraction=1.00, target_approaching=0.00, ours_over_theirs_dps=1.00, theirs_over_ours_dps=1.00, my_range_rank=1.00, platoon_ours_over_theirs_power=1.00, platoon_theirs_over_ours_power=1.00 → **advance=+1.00**
- `strafe`: distance_over_range=1.67, my_reload_remaining=0.00, target_approaching=0.00, target_range_over_mine=1.00, my_hp_fraction=1.00, ours_over_theirs_dps=1.00, theirs_over_ours_dps=1.00, my_speed_rank=1.00 → **orbit=+0.04**
- `engagement`: ours_over_theirs_count=1.00, theirs_over_ours_count=1.00, ours_over_theirs_hp=1.00, theirs_over_ours_hp=1.00, ours_over_theirs_dps=1.00, theirs_over_ours_dps=1.00, ours_over_theirs_value=1.00, theirs_over_ours_value=1.00, ours_over_theirs_speed=1.00, theirs_over_ours_speed=1.00, ours_over_theirs_dps_short=0.00, theirs_over_ours_dps_short=0.00, ours_over_theirs_dps_mid=1.00, theirs_over_ours_dps_mid=1.00, ours_over_theirs_dps_long=0.00, theirs_over_ours_dps_long=0.00, my_hp_fraction=1.00, my_speed_rank=1.00, my_range_rank=1.00, my_hp_rank=1.00, platoon_ours_over_theirs_power=1.00, platoon_theirs_over_ours_power=1.00 → **retreat=-1.00**
- `encircle`: enemy_front_alignment=1.00, enemy_side=0.00, gap_ccw=1.00, gap_cw=1.00, ring_distance_over_range=1.67 → **bloom=-0.30**
- combined: advance +1.00 · orbit +0.04 · regroup +0.00 · retreat -1.00 · bloom -0.30 (platoon commit 0.00) → movement (+1.00, +0.10), throttle 1.00

**t = 24.0s, unit #2** at (1009, 505) chose enemy #1
- `target_selector` [enemy #1 (rank 0)]: distance_over_range=0.99, enemy_hp_fraction=0.23, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=1.00, is_nearest=1.00 → **score=-0.87**
- `spacing`: distance_over_range=0.99, my_reload_remaining=0.00, my_hp_fraction=0.53, target_range_over_mine=1.00, target_hp_fraction=0.23, target_approaching=0.18, ours_over_theirs_dps=1.00, theirs_over_ours_dps=1.00, my_range_rank=1.00, platoon_ours_over_theirs_power=1.00, platoon_theirs_over_ours_power=0.43 → **advance=-0.35**
- `strafe`: distance_over_range=0.99, my_reload_remaining=0.00, target_approaching=0.18, target_range_over_mine=1.00, my_hp_fraction=0.53, ours_over_theirs_dps=1.00, theirs_over_ours_dps=1.00, my_speed_rank=1.00 → **orbit=+0.15**
- `engagement`: ours_over_theirs_count=1.00, theirs_over_ours_count=1.00, ours_over_theirs_hp=1.00, theirs_over_ours_hp=0.43, ours_over_theirs_dps=1.00, theirs_over_ours_dps=1.00, ours_over_theirs_value=1.00, theirs_over_ours_value=1.00, ours_over_theirs_speed=1.00, theirs_over_ours_speed=1.00, ours_over_theirs_dps_short=0.00, theirs_over_ours_dps_short=0.00, ours_over_theirs_dps_mid=1.00, theirs_over_ours_dps_mid=1.00, ours_over_theirs_dps_long=0.00, theirs_over_ours_dps_long=0.00, my_hp_fraction=0.53, my_speed_rank=1.00, my_range_rank=1.00, my_hp_rank=1.00, platoon_ours_over_theirs_power=1.00, platoon_theirs_over_ours_power=0.43 → **retreat=-1.00**
- `encircle`: enemy_front_alignment=0.18, enemy_side=-0.98, gap_ccw=1.00, gap_cw=1.00, ring_distance_over_range=0.99 → **bloom=-0.02**
- combined: advance -0.35 · orbit +0.15 · regroup +0.00 · retreat -1.00 · bloom -0.02 (platoon commit 0.80) → movement (-0.10, -0.17), throttle 0.19

**t = 19.6s, unit #2** at (1240, 777) chose enemy #4
- `target_selector` [enemy #4 (rank 0)]: distance_over_range=1.05, enemy_hp_fraction=0.64, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=1.00, is_nearest=1.00 → **score=-0.91**
- `target_selector` [enemy #5 (rank 1)]: distance_over_range=1.36, enemy_hp_fraction=0.54, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=1.00, is_nearest=0.00 → **score=-0.98**
- `target_selector` [enemy #6 (rank 2)]: distance_over_range=1.55, enemy_hp_fraction=0.77, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=1.00, is_nearest=0.00 → **score=-0.99**
- `spacing`: distance_over_range=1.05, my_reload_remaining=0.00, my_hp_fraction=0.78, target_range_over_mine=1.00, target_hp_fraction=0.64, target_approaching=-0.69, ours_over_theirs_dps=1.00, theirs_over_ours_dps=1.00, my_range_rank=1.00, platoon_ours_over_theirs_power=1.00, platoon_theirs_over_ours_power=0.82 → **advance=+0.97**
- `strafe`: distance_over_range=1.05, my_reload_remaining=0.00, target_approaching=-0.69, target_range_over_mine=1.00, my_hp_fraction=0.78, ours_over_theirs_dps=1.00, theirs_over_ours_dps=1.00, my_speed_rank=1.00 → **orbit=+0.04**
- `cohesion`: ally_centroid_distance_over_range=1.30, nearest_ally_distance_over_range=0.61, my_hp_fraction=0.78, ours_over_theirs_count=1.00, distance_over_range=1.05 → **regroup=-0.35**
- `engagement`: ours_over_theirs_count=1.00, theirs_over_ours_count=1.00, ours_over_theirs_hp=1.00, theirs_over_ours_hp=0.82, ours_over_theirs_dps=1.00, theirs_over_ours_dps=1.00, ours_over_theirs_value=1.00, theirs_over_ours_value=1.00, ours_over_theirs_speed=1.00, theirs_over_ours_speed=1.00, ours_over_theirs_dps_short=0.00, theirs_over_ours_dps_short=0.00, ours_over_theirs_dps_mid=1.00, theirs_over_ours_dps_mid=1.00, ours_over_theirs_dps_long=0.00, theirs_over_ours_dps_long=0.00, my_hp_fraction=0.78, my_speed_rank=1.00, my_range_rank=1.00, my_hp_rank=1.00, platoon_ours_over_theirs_power=1.00, platoon_theirs_over_ours_power=0.82 → **retreat=-1.00**
- `encircle`: enemy_front_alignment=-1.00, enemy_side=-0.00, gap_ccw=0.07, gap_cw=0.47, ring_distance_over_range=1.31 → **bloom=-0.90**
- combined: advance +0.97 · orbit +0.04 · regroup -0.35 · retreat -1.00 · bloom -0.90 (platoon commit 0.00) → movement (+0.46, +0.48), throttle 0.66

