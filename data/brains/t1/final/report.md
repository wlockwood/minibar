# Council report: `tactical_v3`

6 modules, 420 parameters. Each module is a small perceptron with one job; the formula that combines them is fixed code (see `learn/tactical.rs`). Behaviour below is measured from 6000 sampled decisions.

## Results on validation battles

| scenario | opponent | W | timeout | wipe-out | L | win rate | 95% CI | mean exchange |
|---|---|---|---|---|---|---|---|---|
| m00 [under 1.32x p0.42] veh brawl-mix 2xcorgarp+1xarmanac+1xarmpincer+1xcorsnap vs veh line-pure 8xarmpincer | charge | 0 | 8 | 0 | 4 | 0% | 0–24% | +0.03 |
| m00 [under 1.32x p0.42] veh brawl-mix 2xcorgarp+1xarmanac+1xarmpincer+1xcorsnap vs veh line-pure 8xarmpincer | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.46 |
| m00 [under 1.32x p0.42] veh brawl-mix 2xcorgarp+1xarmanac+1xarmpincer+1xcorsnap vs veh line-pure 8xarmpincer | kite | 0 | 12 | 0 | 0 | 0% | 0–24% | +0.21 |
| m00 [under 1.32x p0.42] veh brawl-mix 2xcorgarp+1xarmanac+1xarmpincer+1xcorsnap vs veh line-pure 8xarmpincer | orbit | 0 | 12 | 0 | 0 | 0% | 0–24% | -0.00 |
| m01 [even 0.90x p0.63] veh scout-pure 16xcorsh vs veh line-pure 9xcorgator | charge | 12 | 0 | 0 | 0 | 100% | 76–100% | -0.06 |
| m01 [even 0.90x p0.63] veh scout-pure 16xcorsh vs veh line-pure 9xcorgator | hold | 0 | 12 | 0 | 0 | 0% | 0–24% | -0.18 |
| m01 [even 0.90x p0.63] veh scout-pure 16xcorsh vs veh line-pure 9xcorgator | kite | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.27 |
| m01 [even 0.90x p0.63] veh scout-pure 16xcorsh vs veh line-pure 9xcorgator | orbit | 0 | 12 | 0 | 0 | 0% | 0–24% | -0.28 |
| m02 [over 0.80x p10.70] bot scout-pure 24xarmflea & bot line-pure 13xcorak vs veh artillery-mix 1xcormist+1xcorwolv+1xarmsam+1xarmjanus+1xarmart | charge | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.32 |
| m02 [over 0.80x p10.70] bot scout-pure 24xarmflea & bot line-pure 13xcorak vs veh artillery-mix 1xcormist+1xcorwolv+1xarmsam+1xarmjanus+1xarmart | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.25 |
| m02 [over 0.80x p10.70] bot scout-pure 24xarmflea & bot line-pure 13xcorak vs veh artillery-mix 1xcormist+1xcorwolv+1xarmsam+1xarmjanus+1xarmart | kite | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.30 |
| m02 [over 0.80x p10.70] bot scout-pure 24xarmflea & bot line-pure 13xcorak vs veh artillery-mix 1xcormist+1xcorwolv+1xarmsam+1xarmjanus+1xarmart | orbit | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.30 |
| m03 [under 2.02x p0.14] veh line-mix 1xarmpincer+1xcorlevlr+3xarmsam vs veh line-pure 8xcorlevlr | charge | 0 | 0 | 0 | 12 | 0% | 0–24% | +0.22 |
| m03 [under 2.02x p0.14] veh line-mix 1xarmpincer+1xcorlevlr+3xarmsam vs veh line-pure 8xcorlevlr | hold | 10 | 2 | 0 | 0 | 83% | 55–95% | +0.50 |
| m03 [under 2.02x p0.14] veh line-mix 1xarmpincer+1xcorlevlr+3xarmsam vs veh line-pure 8xcorlevlr | kite | 4 | 0 | 0 | 8 | 33% | 14–61% | +0.28 |
| m03 [under 2.02x p0.14] veh line-mix 1xarmpincer+1xcorlevlr+3xarmsam vs veh line-pure 8xcorlevlr | orbit | 0 | 12 | 0 | 0 | 0% | 0–24% | +0.18 |
| m04 [even 1.09x p1.88] bot line-mix 4xcorak+3xcorstorm & bot brawl-mix 1xcorthud+1xarmwar vs veh line-mix 1xarmpincer+3xarmsam+1xcorlevlr+1xcorgator | charge | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.28 |
| m04 [even 1.09x p1.88] bot line-mix 4xcorak+3xcorstorm & bot brawl-mix 1xcorthud+1xarmwar vs veh line-mix 1xarmpincer+3xarmsam+1xcorlevlr+1xcorgator | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.31 |
| m04 [even 1.09x p1.88] bot line-mix 4xcorak+3xcorstorm & bot brawl-mix 1xcorthud+1xarmwar vs veh line-mix 1xarmpincer+3xarmsam+1xcorlevlr+1xcorgator | kite | 8 | 4 | 0 | 0 | 67% | 39–86% | +0.14 |
| m04 [even 1.09x p1.88] bot line-mix 4xcorak+3xcorstorm & bot brawl-mix 1xcorthud+1xarmwar vs veh line-mix 1xarmpincer+3xarmsam+1xcorlevlr+1xcorgator | orbit | 2 | 10 | 0 | 0 | 17% | 5–45% | -0.03 |
| m05 [over 0.57x p5.09] bot brawl-pure 3xarmwar & bot line-pure 3xarmham vs veh scout-pure 8xarmsh | charge | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.31 |
| m05 [over 0.57x p5.09] bot brawl-pure 3xarmwar & bot line-pure 3xarmham vs veh scout-pure 8xarmsh | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.37 |
| m05 [over 0.57x p5.09] bot brawl-pure 3xarmwar & bot line-pure 3xarmham vs veh scout-pure 8xarmsh | kite | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.32 |
| m05 [over 0.57x p5.09] bot brawl-pure 3xarmwar & bot line-pure 3xarmham vs veh scout-pure 8xarmsh | orbit | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.33 |
| m06 [under 1.83x p0.07] veh artillery-pure 4xarmsam vs bot brawl-mix 4xcorthud+2xarmwar | charge | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.16 |
| m06 [under 1.83x p0.07] veh artillery-pure 4xarmsam vs bot brawl-mix 4xcorthud+2xarmwar | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.61 |
| m06 [under 1.83x p0.07] veh artillery-pure 4xarmsam vs bot brawl-mix 4xcorthud+2xarmwar | kite | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.12 |
| m06 [under 1.83x p0.07] veh artillery-pure 4xarmsam vs bot brawl-mix 4xcorthud+2xarmwar | orbit | 0 | 12 | 0 | 0 | 0% | 0–24% | +0.11 |
| m07 [over 0.42x p2.47] veh scout-pure 14xarmsh vs bot scout-pure 24xarmflea | charge | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.15 |
| m07 [over 0.42x p2.47] veh scout-pure 14xarmsh vs bot scout-pure 24xarmflea | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.18 |
| m07 [over 0.42x p2.47] veh scout-pure 14xarmsh vs bot scout-pure 24xarmflea | kite | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.14 |
| m07 [over 0.42x p2.47] veh scout-pure 14xarmsh vs bot scout-pure 24xarmflea | orbit | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.17 |
| m08 [over 0.65x p1.19] veh artillery-pure 7xarmart vs veh scout-pure 5xcorsh & veh scout-mix 1xarmsh+3xcorfav+1xcorsh | charge | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.59 |
| m08 [over 0.65x p1.19] veh artillery-pure 7xarmart vs veh scout-pure 5xcorsh & veh scout-mix 1xarmsh+3xcorfav+1xcorsh | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.36 |
| m08 [over 0.65x p1.19] veh artillery-pure 7xarmart vs veh scout-pure 5xcorsh & veh scout-mix 1xarmsh+3xcorfav+1xcorsh | kite | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.54 |
| m08 [over 0.65x p1.19] veh artillery-pure 7xarmart vs veh scout-pure 5xcorsh & veh scout-mix 1xarmsh+3xcorfav+1xcorsh | orbit | 0 | 12 | 0 | 0 | 0% | 0–24% | -0.07 |
| m09 [under 1.84x p1.17] bot raid-mix 5xarmflea+3xcorak+4xarmpw & bot raid-pure 10xcorak vs veh artillery-pure 6xarmsam & veh line-pure 6xcorgator | charge | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.32 |
| m09 [under 1.84x p1.17] bot raid-mix 5xarmflea+3xcorak+4xarmpw & bot raid-pure 10xcorak vs veh artillery-pure 6xarmsam & veh line-pure 6xcorgator | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.35 |
| m09 [under 1.84x p1.17] bot raid-mix 5xarmflea+3xcorak+4xarmpw & bot raid-pure 10xcorak vs veh artillery-pure 6xarmsam & veh line-pure 6xcorgator | kite | 0 | 1 | 0 | 11 | 0% | 0–24% | -0.02 |
| m09 [under 1.84x p1.17] bot raid-mix 5xarmflea+3xcorak+4xarmpw & bot raid-pure 10xcorak vs veh artillery-pure 6xarmsam & veh line-pure 6xcorgator | orbit | 5 | 7 | 0 | 0 | 42% | 19–68% | +0.18 |
| m10 [even 0.93x p0.64] veh scout-pure 16xarmfav & bot artillery-mix 2xarmham+2xcorstorm+2xarmrock vs bot raid-mix 4xarmpw+8xcorak+6xarmflea & veh scout-pure 6xcorsh | charge | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.11 |
| m10 [even 0.93x p0.64] veh scout-pure 16xarmfav & bot artillery-mix 2xarmham+2xcorstorm+2xarmrock vs bot raid-mix 4xarmpw+8xcorak+6xarmflea & veh scout-pure 6xcorsh | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.29 |
| m10 [even 0.93x p0.64] veh scout-pure 16xarmfav & bot artillery-mix 2xarmham+2xcorstorm+2xarmrock vs bot raid-mix 4xarmpw+8xcorak+6xarmflea & veh scout-pure 6xcorsh | kite | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.26 |
| m10 [even 0.93x p0.64] veh scout-pure 16xarmfav & bot artillery-mix 2xarmham+2xcorstorm+2xarmrock vs bot raid-mix 4xarmpw+8xcorak+6xarmflea & veh scout-pure 6xcorsh | orbit | 0 | 12 | 0 | 0 | 0% | 0–24% | -0.08 |
| m11 [over 0.45x p1.69] veh brawl-pure 2xcorsnap vs veh raid-pure 2xcorsh & veh raid-pure 1xcorgator | charge | 6 | 0 | 0 | 6 | 50% | 25–75% | -0.34 |
| m11 [over 0.45x p1.69] veh brawl-pure 2xcorsnap vs veh raid-pure 2xcorsh & veh raid-pure 1xcorgator | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.14 |
| m11 [over 0.45x p1.69] veh brawl-pure 2xcorsnap vs veh raid-pure 2xcorsh & veh raid-pure 1xcorgator | kite | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.46 |
| m11 [over 0.45x p1.69] veh brawl-pure 2xcorsnap vs veh raid-pure 2xcorsh & veh raid-pure 1xcorgator | orbit | 0 | 12 | 0 | 0 | 0% | 0–24% | -0.17 |

Overall win rate 52%, mean fitness +0.35, **35% in battles where it was the underdog** (192 battles).

## `target_selector`

Scores one enemy at a time (same weights for every candidate); the highest-scoring of the three nearest enemies becomes the target.

Shape 6 → 6 → 1 · 49 parameters · 1478 samples

### output `score`

mean -0.30 · 10th pct -0.87 · median -0.59 · 90th pct +0.81 · strongly active (|x| > 0.5) 79% of the time

What it listens to, most influential first. *Swing* is the average change in `score` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `distance_over_range` | 0.09 – 1.55 | 1.40 | higher → lowers `score` | `████████████████████` |
| `is_nearest` | 0.00 – 1.00 | 0.31 | higher → raises `score` | `████················` |
| `enemy_hp_fraction` | 0.34 – 1.00 | 0.15 | higher → lowers `score` | `██··················` |
| `enemy_value_over_mine` | 0.66 – 1.00 | 0.06 | higher → lowers `score` | `█···················` |
| `enemy_threat_over_mine` | 1.00 – 1.10 | 0.04 | higher → raises `score` | `█···················` |
| `enemy_range_over_mine` | 0.82 – 1.00 | 0.03 | higher → lowers `score` | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `distance_over_range` | 0.00 → +0.81 | 0.65 → -0.27 | 1.30 → -0.65 | 1.95 → -0.71 | 2.60 → -0.75 |
| `enemy_hp_fraction` | 0.00 → -0.33 | 0.25 → -0.40 | 0.50 → -0.46 | 0.75 → -0.52 | 1.00 → -0.57 |
| `enemy_range_over_mine` | 0.82 → -0.51 | 0.87 → -0.52 | 0.91 → -0.53 | 0.96 → -0.53 | 1.00 → -0.54 |
| `enemy_threat_over_mine` | 1.00 → -0.54 | 1.03 → -0.53 | 1.05 → -0.52 | 1.08 → -0.51 | 1.10 → -0.50 |
| `enemy_value_over_mine` | 0.66 → -0.49 | 0.75 → -0.51 | 0.83 → -0.53 | 0.92 → -0.55 | 1.00 → -0.56 |
| `is_nearest` | 0.00 → -0.83 | 0.25 → -0.77 | 0.50 → -0.69 | 0.75 → -0.61 | 1.00 → -0.54 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -0.02: reads -2.12`distance_over_range` +1.62`enemy_threat_over_mine` -0.61`enemy_value_over_mine`; pushes +1.06 → `score`
- **h1** bias +0.71: reads -1.15`enemy_value_over_mine` -0.24`enemy_threat_over_mine` +0.24`is_nearest`; pushes +0.12 → `score`
- **h2** bias +0.05: reads -1.23`distance_over_range` -0.72`enemy_threat_over_mine` +0.26`enemy_value_over_mine`; pushes +0.07 → `score`
- **h3** bias -1.52: reads -0.93`is_nearest` +0.65`enemy_range_over_mine` +0.47`enemy_hp_fraction`; pushes -0.85 → `score`
- **h4** bias +0.00: reads -0.95`enemy_value_over_mine` -0.80`enemy_range_over_mine` -0.68`is_nearest`; pushes -0.19 → `score`
- **h5** bias -0.36: reads -0.97`distance_over_range` -0.71`is_nearest` +0.32`enemy_range_over_mine`; pushes +0.43 → `score`

## `spacing`

Owns range: close on the target (+1) or back away from it (-1) along the line between them.

Shape 11 → 6 → 1 · 79 parameters · 1500 samples

### output `advance`

mean -0.05 · 10th pct -1.00 · median -0.13 · 90th pct +1.00 · strongly active (|x| > 0.5) 81% of the time

What it listens to, most influential first. *Swing* is the average change in `advance` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `distance_over_range` | 0.09 – 1.53 | 1.48 | higher → raises `advance` | `████████████████████` |
| `target_approaching` | -0.85 – 1.00 | 1.14 | higher → lowers `advance` | `███████████████·····` |
| `my_reload_remaining` | 0.00 – 0.85 | 0.74 | higher → lowers `advance` | `██████████··········` |
| `my_hp_fraction` | 0.54 – 1.00 | 0.58 | higher → raises `advance` | `████████············` |
| `platoon_ours_over_theirs_power` | 0.21 – 1.00 | 0.30 | higher → raises `advance` | `████················` |
| `target_hp_fraction` | 0.25 – 1.00 | 0.26 | higher → lowers `advance` | `███·················` |
| `ours_over_theirs_dps` | 0.36 – 0.80 | 0.06 | higher → lowers `advance` | `█···················` |
| `platoon_theirs_over_ours_power` | 0.93 – 1.00 | 0.04 | higher → lowers `advance` | `█···················` |
| `target_range_over_mine` | 0.82 – 1.00 | 0.04 | mixed (depends on other inputs) | `█···················` |
| `my_range_rank` | 0.82 – 1.00 | 0.04 | higher → lowers `advance` | `█···················` |
| `theirs_over_ours_dps` | 1.00 – 1.00 | 0.00 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `distance_over_range` | 0.00 → -1.00 | 0.65 → -0.95 | 1.30 → +0.99 | 1.95 → +1.00 | 2.59 → +1.00 |
| `my_reload_remaining` | 0.00 → +0.90 | 0.24 → +0.68 | 0.49 → -0.48 | 0.73 → -0.79 | 0.98 → -0.93 |
| `my_hp_fraction` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -0.93 | 1.00 → +0.10 |
| `target_range_over_mine` | 0.82 → -0.28 | 0.87 → -0.24 | 0.91 → -0.20 | 0.96 → -0.16 | 1.00 → -0.12 |
| `target_hp_fraction` | 0.00 → +0.88 | 0.25 → +0.70 | 0.50 → +0.37 | 0.75 → -0.05 | 1.00 → -0.41 |
| `target_approaching` | -1.00 → +1.00 | -0.50 → +0.97 | 0.00 → -0.12 | 0.50 → -0.92 | 1.00 → -0.98 |
| `ours_over_theirs_dps` | 0.12 → +0.31 | 0.34 → +0.10 | 0.56 → -0.10 | 0.78 → -0.24 | 1.00 → -0.34 |
| `theirs_over_ours_dps` | 0.21 → +0.99 | 0.41 → +0.98 | 0.60 → +0.91 | 0.80 → +0.61 | 1.00 → -0.12 |
| `my_range_rank` | 0.82 → -0.12 | 0.87 → -0.16 | 0.91 → -0.19 | 0.96 → -0.22 | 1.00 → -0.25 |
| `platoon_ours_over_theirs_power` | 0.00 → -0.70 | 0.25 → -0.34 | 0.50 → -0.03 | 0.75 → +0.16 | 1.00 → +0.28 |
| `platoon_theirs_over_ours_power` | 0.00 → +0.98 | 0.25 → +0.95 | 0.50 → +0.85 | 0.75 → +0.51 | 1.00 → -0.12 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -1.55: reads +3.27`my_hp_fraction` +3.23`distance_over_range` -1.40`theirs_over_ours_dps`; pushes +3.50 → `advance`
- **h1** bias +19.81: reads +40.53`my_reload_remaining` -38.17`distance_over_range` +1.28`theirs_over_ours_dps`; pushes -0.51 → `advance`
- **h2** bias +0.01: reads -0.45`my_range_rank` -0.40`target_hp_fraction` -0.30`theirs_over_ours_dps`; pushes +0.90 → `advance`
- **h3** bias +0.35: reads -2.07`my_hp_fraction` +1.63`target_approaching` +1.23`theirs_over_ours_dps`; pushes -2.15 → `advance`
- **h4** bias +0.00: reads -1.35`theirs_over_ours_dps` +1.22`ours_over_theirs_dps` +1.15`distance_over_range`; pushes +0.30 → `advance`
- **h5** bias -0.00: reads -1.17`target_approaching` -1.13`my_reload_remaining` +0.73`distance_over_range`; pushes +0.92 → `advance`

## `strafe`

Owns sideways motion: circle the target counter-clockwise (+1) or clockwise (-1), e.g. to dodge a projectile weapon.

Shape 8 → 6 → 1 · 61 parameters · 1500 samples

### output `orbit`

mean -0.27 · 10th pct -0.38 · median -0.28 · 90th pct -0.16 · strongly active (|x| > 0.5) 0% of the time

What it listens to, most influential first. *Swing* is the average change in `orbit` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `my_reload_remaining` | 0.00 – 0.85 | 0.15 | higher → raises `orbit` | `████████████████████` |
| `distance_over_range` | 0.09 – 1.53 | 0.12 | higher → lowers `orbit` | `████████████████····` |
| `target_approaching` | -0.85 – 1.00 | 0.05 | higher → raises `orbit` | `███████·············` |
| `target_range_over_mine` | 0.82 – 1.00 | 0.01 | ignored | `██··················` |
| `my_hp_fraction` | 0.54 – 1.00 | 0.01 | ignored | `██··················` |
| `ours_over_theirs_dps` | 0.36 – 0.80 | 0.01 | ignored | `█···················` |
| `my_speed_rank` | 0.84 – 1.00 | 0.00 | ignored | `█···················` |
| `theirs_over_ours_dps` | 1.00 – 1.00 | 0.00 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `distance_over_range` | 0.00 → -0.23 | 0.65 → -0.29 | 1.30 → -0.33 | 1.95 → -0.37 | 2.59 → -0.38 |
| `my_reload_remaining` | 0.00 → -0.35 | 0.24 → -0.31 | 0.49 → -0.27 | 0.73 → -0.22 | 0.98 → -0.17 |
| `target_approaching` | -1.00 → -0.33 | -0.50 → -0.32 | 0.00 → -0.30 | 0.50 → -0.29 | 1.00 → -0.27 |
| `target_range_over_mine` | 0.82 → -0.29 | 0.87 → -0.29 | 0.91 → -0.30 | 0.96 → -0.30 | 1.00 → -0.30 |
| `my_hp_fraction` | 0.00 → -0.27 | 0.25 → -0.28 | 0.50 → -0.29 | 0.75 → -0.30 | 1.00 → -0.30 |
| `ours_over_theirs_dps` | 0.12 → -0.29 | 0.34 → -0.30 | 0.56 → -0.30 | 0.78 → -0.31 | 1.00 → -0.31 |
| `theirs_over_ours_dps` | 0.21 → -0.42 | 0.41 → -0.40 | 0.60 → -0.37 | 0.80 → -0.34 | 1.00 → -0.30 |
| `my_speed_rank` | 0.84 → -0.30 | 0.88 → -0.30 | 0.92 → -0.30 | 0.96 → -0.30 | 1.00 → -0.31 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -0.50: reads +0.74`my_reload_remaining` -0.64`target_range_over_mine` -0.37`distance_over_range`; pushes +0.17 → `orbit`
- **h1** bias +0.02: reads -1.60`my_hp_fraction` +0.83`theirs_over_ours_dps` +0.57`target_approaching`; pushes -0.03 → `orbit`
- **h2** bias +1.23: reads -1.03`my_reload_remaining` -0.91`theirs_over_ours_dps` +0.63`ours_over_theirs_dps`; pushes -0.19 → `orbit`
- **h3** bias -0.78: reads -0.93`my_reload_remaining` +0.57`my_speed_rank` -0.50`my_hp_fraction`; pushes +0.06 → `orbit`
- **h4** bias +0.02: reads -0.90`my_hp_fraction` +0.80`theirs_over_ours_dps` -0.42`target_range_over_mine`; pushes +0.16 → `orbit`
- **h5** bias -0.15: reads -0.47`distance_over_range` +0.28`target_approaching` +0.27`ours_over_theirs_dps`; pushes +0.18 → `orbit`

## `cohesion`

Owns group shape: drift toward the centre of friendly units (+1) or spread away from it (-1). Ignored when the unit has no allies.

Shape 5 → 6 → 1 · 43 parameters · 1466 samples

### output `regroup`

mean -0.18 · 10th pct -0.33 · median -0.19 · 90th pct +0.01 · strongly active (|x| > 0.5) 0% of the time

What it listens to, most influential first. *Swing* is the average change in `regroup` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `distance_over_range` | 0.10 – 1.54 | 0.29 | higher → raises `regroup` | `████████████████████` |
| `ally_centroid_distance_over_range` | 0.15 – 1.55 | 0.29 | higher → lowers `regroup` | `████████████████████` |
| `nearest_ally_distance_over_range` | 0.12 – 1.09 | 0.10 | higher → raises `regroup` | `███████·············` |
| `my_hp_fraction` | 0.57 – 1.00 | 0.09 | higher → lowers `regroup` | `███████·············` |
| `ours_over_theirs_count` | 0.40 – 0.83 | 0.00 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `ally_centroid_distance_over_range` | 0.00 → -0.07 | 0.75 → -0.20 | 1.50 → -0.36 | 2.25 → -0.48 | 3.00 → -0.54 |
| `nearest_ally_distance_over_range` | 0.03 → -0.22 | 0.77 → -0.14 | 1.51 → -0.05 | 2.26 → +0.05 | 3.00 → +0.13 |
| `my_hp_fraction` | 0.01 → +0.03 | 0.25 → -0.02 | 0.50 → -0.08 | 0.75 → -0.14 | 1.00 → -0.19 |
| `ours_over_theirs_count` | 0.12 → -0.18 | 0.34 → -0.19 | 0.56 → -0.19 | 0.78 → -0.19 | 1.00 → -0.19 |
| `distance_over_range` | 0.04 → -0.35 | 0.68 → -0.23 | 1.32 → -0.10 | 1.96 → +0.03 | 2.59 → +0.14 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -1.11: reads +0.70`ally_centroid_distance_over_range` +0.16`distance_over_range` -0.05`ours_over_theirs_count`; pushes -0.07 → `regroup`
- **h1** bias -0.05: reads +0.96`ally_centroid_distance_over_range` -0.84`my_hp_fraction` -0.27`distance_over_range`; pushes -0.12 → `regroup`
- **h2** bias -0.15: reads -0.78`my_hp_fraction` +0.48`nearest_ally_distance_over_range` +0.40`distance_over_range`; pushes +0.30 → `regroup`
- **h3** bias +0.61: reads -0.79`my_hp_fraction` +0.66`distance_over_range` -0.20`ours_over_theirs_count`; pushes +0.18 → `regroup`
- **h4** bias -0.36: reads -0.70`ally_centroid_distance_over_range` +0.39`distance_over_range` +0.32`ours_over_theirs_count`; pushes +0.00 → `regroup`
- **h5** bias -0.47: reads +0.66`ours_over_theirs_count` +0.32`ally_centroid_distance_over_range` +0.26`distance_over_range`; pushes +0.02 → `regroup`

## `engagement`

Owns fight or flight: reads the friend/enemy force balance and, above zero, pulls the unit out of the fight by that fraction, running away from the enemy force. At or below zero it fights. Never retreats while the platoon is clearly ahead (the doctrine).

Shape 22 → 6 → 1 · 145 parameters · 1500 samples

### output `retreat`

mean -1.00 · 10th pct -1.00 · median -1.00 · 90th pct -1.00 · strongly active (|x| > 0.5) 100% of the time

What it listens to, most influential first. *Swing* is the average change in `retreat` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `platoon_ours_over_theirs_power` | 0.21 – 1.00 | 0.01 | ignored | `████████████████████` |
| `ours_over_theirs_value` | 0.45 – 1.00 | 0.00 | ignored | `████████████········` |
| `ours_over_theirs_dps` | 0.36 – 0.80 | 0.00 | ignored | `████████············` |
| `ours_over_theirs_count` | 0.38 – 0.83 | 0.00 | ignored | `███·················` |
| `ours_over_theirs_hp` | 0.43 – 1.00 | 0.00 | ignored | `██··················` |
| `theirs_over_ours_hp` | 0.75 – 1.00 | 0.00 | ignored | `██··················` |
| `my_hp_fraction` | 0.54 – 1.00 | 0.00 | ignored | `█···················` |
| `platoon_theirs_over_ours_power` | 0.93 – 1.00 | 0.00 | ignored | `█···················` |
| `ours_over_theirs_dps_mid` | 0.36 – 0.80 | 0.00 | ignored | `█···················` |
| `my_speed_rank` | 0.84 – 1.00 | 0.00 | ignored | `█···················` |
| `my_hp_rank` | 0.78 – 1.00 | 0.00 | ignored | `█···················` |
| `my_range_rank` | 0.82 – 1.00 | 0.00 | ignored | `█···················` |
| `theirs_over_ours_speed` | 0.97 – 0.99 | 0.00 | ignored | `····················` |
| `theirs_over_ours_value` | 0.98 – 1.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_count` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_dps` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_speed` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_dps_short` | 0.00 – 0.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_dps_short` | 0.00 – 0.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_dps_mid` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_dps_long` | 0.00 – 0.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_dps_long` | 0.00 – 0.00 | 0.00 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `ours_over_theirs_count` | 0.12 → -1.00 | 0.34 → -1.00 | 0.56 → -1.00 | 0.78 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_count` | 0.20 → -1.00 | 0.40 → -1.00 | 0.60 → -1.00 | 0.80 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_hp` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_hp` | 0.01 → -1.00 | 0.26 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_dps` | 0.12 → -1.00 | 0.34 → -1.00 | 0.56 → -1.00 | 0.78 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_dps` | 0.21 → -0.99 | 0.41 → -1.00 | 0.60 → -1.00 | 0.80 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_value` | 0.14 → -1.00 | 0.36 → -1.00 | 0.57 → -1.00 | 0.79 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_value` | 0.16 → -1.00 | 0.37 → -1.00 | 0.58 → -1.00 | 0.79 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_speed` | 0.93 → -1.00 | 0.95 → -1.00 | 0.96 → -1.00 | 0.98 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_speed` | 0.94 → -1.00 | 0.96 → -1.00 | 0.97 → -1.00 | 0.99 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_dps_mid` | 0.12 → -1.00 | 0.34 → -1.00 | 0.56 → -1.00 | 0.78 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_dps_mid` | 0.21 → -1.00 | 0.41 → -1.00 | 0.60 → -1.00 | 0.80 → -1.00 | 1.00 → -1.00 |
| `my_hp_fraction` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `my_speed_rank` | 0.84 → -1.00 | 0.88 → -1.00 | 0.92 → -1.00 | 0.96 → -1.00 | 1.00 → -1.00 |
| `my_range_rank` | 0.82 → -1.00 | 0.87 → -1.00 | 0.91 → -1.00 | 0.96 → -1.00 | 1.00 → -1.00 |
| `my_hp_rank` | 0.78 → -1.00 | 0.83 → -1.00 | 0.89 → -1.00 | 0.94 → -1.00 | 1.00 → -1.00 |
| `platoon_ours_over_theirs_power` | 0.00 → -0.81 | 0.25 → -0.99 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `platoon_theirs_over_ours_power` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -2.98: reads -4.78`platoon_ours_over_theirs_power` +2.72`platoon_theirs_over_ours_power` +1.11`ours_over_theirs_value`; pushes +1.80 → `retreat`
- **h1** bias +0.22: reads -2.00`my_hp_rank` +1.58`ours_over_theirs_hp` -1.08`theirs_over_ours_dps_long`; pushes -0.01 → `retreat`
- **h2** bias -1.12: reads -1.88`theirs_over_ours_dps_long` +1.46`my_hp_rank` -1.46`my_hp_fraction`; pushes +0.08 → `retreat`
- **h3** bias -0.29: reads +0.83`ours_over_theirs_dps` -0.71`theirs_over_ours_dps_short` +0.61`platoon_theirs_over_ours_power`; pushes +0.31 → `retreat`
- **h4** bias -1.03: reads +1.21`ours_over_theirs_hp` +0.92`my_hp_fraction` +0.89`ours_over_theirs_dps`; pushes -1.01 → `retreat`
- **h5** bias -0.56: reads +1.44`ours_over_theirs_dps_short` -0.93`theirs_over_ours_dps_mid` +0.69`ours_over_theirs_speed`; pushes -0.00 → `retreat`

## `encircle`

Owns the formation around the enemy force: slides the unit around it (+1 counter-clockwise) toward the larger gap between itself and its neighbours, so friends spread into an even ring, and away from the enemy's line of advance, so they flank instead of bunching in front. Scaled up as the platoon gets further ahead.

Shape 5 → 6 → 1 · 43 parameters · 1500 samples

### output `bloom`

mean +0.00 · 10th pct -0.97 · median +0.05 · 90th pct +0.99 · strongly active (|x| > 0.5) 74% of the time

What it listens to, most influential first. *Swing* is the average change in `bloom` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `gap_cw` | 0.01 – 0.70 | 1.35 | higher → lowers `bloom` | `████████████████████` |
| `gap_ccw` | 0.01 – 0.90 | 1.24 | higher → raises `bloom` | `██████████████████··` |
| `enemy_front_alignment` | -0.81 – 1.00 | 0.29 | higher → raises `bloom` | `████················` |
| `enemy_side` | -0.93 – 0.87 | 0.26 | mixed (depends on other inputs) | `████················` |
| `ring_distance_over_range` | 0.28 – 1.89 | 0.04 | higher → lowers `bloom` | `█···················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `enemy_front_alignment` | -1.00 → -0.51 | -0.50 → -0.29 | 0.00 → -0.00 | 0.50 → +0.30 | 1.00 → +0.51 |
| `enemy_side` | -1.00 → +0.36 | -0.50 → +0.34 | 0.00 → +0.45 | 0.50 → +0.23 | 1.00 → -0.12 |
| `gap_ccw` | 0.00 → -0.67 | 0.25 → +0.94 | 0.50 → +0.97 | 0.75 → +0.98 | 1.00 → +0.98 |
| `gap_cw` | 0.00 → +0.93 | 0.25 → -0.79 | 0.50 → -0.89 | 0.75 → -0.89 | 1.00 → -0.88 |
| `ring_distance_over_range` | 0.05 → +0.48 | 0.70 → +0.47 | 1.35 → +0.42 | 2.00 → +0.32 | 2.64 → +0.20 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias +0.04: reads -8.41`gap_cw` +7.13`gap_ccw` -0.78`enemy_side`; pushes +1.78 → `bloom`
- **h1** bias +0.54: reads +2.53`enemy_side` +0.44`enemy_front_alignment` -0.33`gap_ccw`; pushes +0.39 → `bloom`
- **h2** bias +0.27: reads +1.22`enemy_front_alignment` +0.89`enemy_side` -0.73`ring_distance_over_range`; pushes +0.28 → `bloom`
- **h3** bias +1.45: reads -1.45`enemy_front_alignment` -0.96`gap_ccw` +0.65`enemy_side`; pushes +0.00 → `bloom`
- **h4** bias +0.29: reads +0.76`enemy_side` -0.67`enemy_front_alignment` +0.38`gap_ccw`; pushes +0.55 → `bloom`
- **h5** bias +0.29: reads -0.56`gap_ccw` -0.51`ring_distance_over_range` -0.48`gap_cw`; pushes -0.16 → `bloom`

## Worked decisions

**t = 0.0s, unit #1** at (604, 404) chose enemy #6
- `target_selector` [enemy #6 (rank 0)]: distance_over_range=2.59, enemy_hp_fraction=1.00, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=1.00 → **score=-0.76**
- `target_selector` [enemy #7 (rank 1)]: distance_over_range=2.60, enemy_hp_fraction=1.00, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=0.00 → **score=-0.94**
- `target_selector` [enemy #8 (rank 2)]: distance_over_range=2.61, enemy_hp_fraction=1.00, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=0.00 → **score=-0.94**
- `spacing`: distance_over_range=2.59, my_reload_remaining=0.00, my_hp_fraction=1.00, target_range_over_mine=1.00, target_hp_fraction=1.00, target_approaching=0.00, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, my_range_rank=0.82, platoon_ours_over_theirs_power=0.42, platoon_theirs_over_ours_power=1.00 → **advance=+1.00**
- `strafe`: distance_over_range=2.59, my_reload_remaining=0.00, target_approaching=0.00, target_range_over_mine=1.00, my_hp_fraction=1.00, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, my_speed_rank=0.84 → **orbit=-0.43**
- `cohesion`: ally_centroid_distance_over_range=0.39, nearest_ally_distance_over_range=0.16, my_hp_fraction=1.00, ours_over_theirs_count=0.62, distance_over_range=2.59 → **regroup=+0.15**
- `engagement`: ours_over_theirs_count=0.62, theirs_over_ours_count=1.00, ours_over_theirs_hp=0.69, theirs_over_ours_hp=1.00, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, ours_over_theirs_value=0.76, theirs_over_ours_value=1.00, ours_over_theirs_speed=1.00, theirs_over_ours_speed=0.99, ours_over_theirs_dps_short=0.00, theirs_over_ours_dps_short=0.00, ours_over_theirs_dps_mid=0.60, theirs_over_ours_dps_mid=1.00, ours_over_theirs_dps_long=0.00, theirs_over_ours_dps_long=0.00, my_hp_fraction=1.00, my_speed_rank=0.84, my_range_rank=0.82, my_hp_rank=0.83, platoon_ours_over_theirs_power=0.42, platoon_theirs_over_ours_power=1.00 → **retreat=-1.00**
- `encircle`: enemy_front_alignment=0.99, enemy_side=0.12, gap_ccw=0.96, gap_cw=0.01, ring_distance_over_range=2.64 → **bloom=+0.98**
- combined: advance +1.00 · orbit -0.43 · regroup +0.15 · retreat -1.00 · bloom +0.98 (platoon commit 0.00) → movement (+1.02, -0.47), throttle 1.00

**t = 29.0s, unit #2** at (684, 19) chose enemy #12
- `target_selector` [enemy #12 (rank 0)]: distance_over_range=0.10, enemy_hp_fraction=0.81, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=1.00 → **score=+0.75**
- `target_selector` [enemy #11 (rank 1)]: distance_over_range=0.11, enemy_hp_fraction=0.91, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=0.00 → **score=+0.34**
- `target_selector` [enemy #7 (rank 2)]: distance_over_range=0.11, enemy_hp_fraction=0.98, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=0.00 → **score=+0.27**
- `spacing`: distance_over_range=0.10, my_reload_remaining=0.44, my_hp_fraction=0.12, target_range_over_mine=1.00, target_hp_fraction=0.81, target_approaching=1.00, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, my_range_rank=0.82, platoon_ours_over_theirs_power=0.38, platoon_theirs_over_ours_power=1.00 → **advance=-1.00**
- `strafe`: distance_over_range=0.10, my_reload_remaining=0.44, target_approaching=1.00, target_range_over_mine=1.00, my_hp_fraction=0.12, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, my_speed_rank=0.84 → **orbit=-0.15**
- `cohesion`: ally_centroid_distance_over_range=1.10, nearest_ally_distance_over_range=0.10, my_hp_fraction=0.12, ours_over_theirs_count=0.62, distance_over_range=0.10 → **regroup=-0.29**
- `engagement`: ours_over_theirs_count=0.62, theirs_over_ours_count=1.00, ours_over_theirs_hp=0.63, theirs_over_ours_hp=1.00, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, ours_over_theirs_value=0.76, theirs_over_ours_value=1.00, ours_over_theirs_speed=1.00, theirs_over_ours_speed=0.99, ours_over_theirs_dps_short=0.00, theirs_over_ours_dps_short=0.00, ours_over_theirs_dps_mid=0.60, theirs_over_ours_dps_mid=1.00, ours_over_theirs_dps_long=0.00, theirs_over_ours_dps_long=0.00, my_hp_fraction=0.12, my_speed_rank=0.84, my_range_rank=0.82, my_hp_rank=0.83, platoon_ours_over_theirs_power=0.38, platoon_theirs_over_ours_power=1.00 → **retreat=-1.00**
- `encircle`: enemy_front_alignment=0.42, enemy_side=0.91, gap_ccw=0.01, gap_cw=0.45, ring_distance_over_range=0.28 → **bloom=-0.72**
- combined: advance -1.00 · orbit -0.15 · regroup -0.29 · retreat -1.00 · bloom -0.72 (platoon commit 0.00) → movement (+1.00, -0.86), throttle 1.00

**t = 19.2s, unit #3** at (1528, 187) chose enemy #12
- `target_selector` [enemy #12 (rank 0)]: distance_over_range=0.83, enemy_hp_fraction=1.00, enemy_range_over_mine=0.82, enemy_threat_over_mine=1.10, enemy_value_over_mine=0.75, is_nearest=1.00 → **score=-0.37**
- `target_selector` [enemy #8 (rank 1)]: distance_over_range=0.87, enemy_hp_fraction=1.00, enemy_range_over_mine=0.82, enemy_threat_over_mine=1.10, enemy_value_over_mine=0.75, is_nearest=0.00 → **score=-0.79**
- `target_selector` [enemy #13 (rank 2)]: distance_over_range=0.93, enemy_hp_fraction=1.00, enemy_range_over_mine=0.82, enemy_threat_over_mine=1.10, enemy_value_over_mine=0.75, is_nearest=0.00 → **score=-0.81**
- `spacing`: distance_over_range=0.83, my_reload_remaining=0.60, my_hp_fraction=1.00, target_range_over_mine=0.82, target_hp_fraction=1.00, target_approaching=0.00, ours_over_theirs_dps=0.80, theirs_over_ours_dps=1.00, my_range_rank=1.00, platoon_ours_over_theirs_power=0.77, platoon_theirs_over_ours_power=1.00 → **advance=-0.69**
- `strafe`: distance_over_range=0.83, my_reload_remaining=0.60, target_approaching=0.00, target_range_over_mine=0.82, my_hp_fraction=1.00, ours_over_theirs_dps=0.80, theirs_over_ours_dps=1.00, my_speed_rank=1.00 → **orbit=-0.24**
- `cohesion`: ally_centroid_distance_over_range=0.88, nearest_ally_distance_over_range=0.26, my_hp_fraction=1.00, ours_over_theirs_count=0.83, distance_over_range=0.83 → **regroup=-0.25**
- `engagement`: ours_over_theirs_count=0.83, theirs_over_ours_count=1.00, ours_over_theirs_hp=0.96, theirs_over_ours_hp=1.00, ours_over_theirs_dps=0.80, theirs_over_ours_dps=1.00, ours_over_theirs_value=1.00, theirs_over_ours_value=0.98, ours_over_theirs_speed=1.00, theirs_over_ours_speed=0.99, ours_over_theirs_dps_short=0.00, theirs_over_ours_dps_short=0.00, ours_over_theirs_dps_mid=0.80, theirs_over_ours_dps_mid=1.00, ours_over_theirs_dps_long=0.00, theirs_over_ours_dps_long=0.00, my_hp_fraction=1.00, my_speed_rank=1.00, my_range_rank=1.00, my_hp_rank=0.89, platoon_ours_over_theirs_power=0.77, platoon_theirs_over_ours_power=1.00 → **retreat=-1.00**
- `encircle`: enemy_front_alignment=0.69, enemy_side=0.72, gap_ccw=0.61, gap_cw=0.04, ring_distance_over_range=0.97 → **bloom=+0.99**
- combined: advance -0.69 · orbit -0.24 · regroup -0.25 · retreat -1.00 · bloom +0.99 (platoon commit 0.00) → movement (+0.84, -0.65), throttle 1.00

