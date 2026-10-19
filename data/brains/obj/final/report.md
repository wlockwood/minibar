# Council report: `tactical_v3`

6 modules, 420 parameters. Each module is a small perceptron with one job; the formula that combines them is fixed code (see `learn/tactical.rs`). Behaviour below is measured from 6000 sampled decisions.

## Results on validation battles

| scenario | opponent | W | timeout | wipe-out | L | win rate | 95% CI | mean exchange |
|---|---|---|---|---|---|---|---|---|
| m00 [under 1.32x p0.42] veh brawl-mix 2xcorgarp+1xarmanac+1xarmpincer+1xcorsnap vs veh line-pure 8xarmpincer | charge | 0 | 12 | 0 | 0 | 0% | 0–24% | +0.03 |
| m00 [under 1.32x p0.42] veh brawl-mix 2xcorgarp+1xarmanac+1xarmpincer+1xcorsnap vs veh line-pure 8xarmpincer | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.42 |
| m00 [under 1.32x p0.42] veh brawl-mix 2xcorgarp+1xarmanac+1xarmpincer+1xcorsnap vs veh line-pure 8xarmpincer | kite | 0 | 12 | 0 | 0 | 0% | 0–24% | +0.24 |
| m00 [under 1.32x p0.42] veh brawl-mix 2xcorgarp+1xarmanac+1xarmpincer+1xcorsnap vs veh line-pure 8xarmpincer | orbit | 0 | 12 | 0 | 0 | 0% | 0–24% | +0.05 |
| m00 [under 1.32x p0.42] veh brawl-mix 2xcorgarp+1xarmanac+1xarmpincer+1xcorsnap vs veh line-pure 8xarmpincer | strike | 11 | 1 | 0 | 0 | 92% | 65–99% | +0.42 |
| m00 [under 1.32x p0.42] veh brawl-mix 2xcorgarp+1xarmanac+1xarmpincer+1xcorsnap vs veh line-pure 8xarmpincer | guard | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.42 |
| m01 [even 0.90x p0.63] veh scout-pure 16xcorsh vs veh line-pure 9xcorgator | charge | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.26 |
| m01 [even 0.90x p0.63] veh scout-pure 16xcorsh vs veh line-pure 9xcorgator | hold | 0 | 12 | 0 | 0 | 0% | 0–24% | -0.29 |
| m01 [even 0.90x p0.63] veh scout-pure 16xcorsh vs veh line-pure 9xcorgator | kite | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.25 |
| m01 [even 0.90x p0.63] veh scout-pure 16xcorsh vs veh line-pure 9xcorgator | orbit | 0 | 6 | 0 | 6 | 0% | 0–24% | -0.28 |
| m01 [even 0.90x p0.63] veh scout-pure 16xcorsh vs veh line-pure 9xcorgator | strike | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.30 |
| m01 [even 0.90x p0.63] veh scout-pure 16xcorsh vs veh line-pure 9xcorgator | guard | 0 | 12 | 0 | 0 | 0% | 0–24% | -0.29 |
| m02 [over 0.80x p10.70] bot scout-pure 24xarmflea & bot line-pure 13xcorak vs veh artillery-mix 1xcormist+1xcorwolv+1xarmsam+1xarmjanus+1xarmart | charge | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.30 |
| m02 [over 0.80x p10.70] bot scout-pure 24xarmflea & bot line-pure 13xcorak vs veh artillery-mix 1xcormist+1xcorwolv+1xarmsam+1xarmjanus+1xarmart | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.31 |
| m02 [over 0.80x p10.70] bot scout-pure 24xarmflea & bot line-pure 13xcorak vs veh artillery-mix 1xcormist+1xcorwolv+1xarmsam+1xarmjanus+1xarmart | kite | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.35 |
| m02 [over 0.80x p10.70] bot scout-pure 24xarmflea & bot line-pure 13xcorak vs veh artillery-mix 1xcormist+1xcorwolv+1xarmsam+1xarmjanus+1xarmart | orbit | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.35 |
| m02 [over 0.80x p10.70] bot scout-pure 24xarmflea & bot line-pure 13xcorak vs veh artillery-mix 1xcormist+1xcorwolv+1xarmsam+1xarmjanus+1xarmart | strike | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.32 |
| m02 [over 0.80x p10.70] bot scout-pure 24xarmflea & bot line-pure 13xcorak vs veh artillery-mix 1xcormist+1xcorwolv+1xarmsam+1xarmjanus+1xarmart | guard | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.31 |
| m03 [under 2.02x p0.14 def>armsolar] veh line-mix 1xarmpincer+1xcorlevlr+3xarmsam vs veh line-pure 8xcorlevlr | charge | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.06 |
| m03 [under 2.02x p0.14 def>armsolar] veh line-mix 1xarmpincer+1xcorlevlr+3xarmsam vs veh line-pure 8xcorlevlr | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.47 |
| m03 [under 2.02x p0.14 def>armsolar] veh line-mix 1xarmpincer+1xcorlevlr+3xarmsam vs veh line-pure 8xcorlevlr | kite | 0 | 0 | 0 | 12 | 0% | 0–24% | +0.02 |
| m03 [under 2.02x p0.14 def>armsolar] veh line-mix 1xarmpincer+1xcorlevlr+3xarmsam vs veh line-pure 8xcorlevlr | orbit | 11 | 0 | 0 | 1 | 92% | 65–99% | +0.13 |
| m03 [under 2.02x p0.14 def>armsolar] veh line-mix 1xarmpincer+1xcorlevlr+3xarmsam vs veh line-pure 8xcorlevlr | strike | 0 | 0 | 0 | 12 | 0% | 0–24% | +0.36 |
| m03 [under 2.02x p0.14 def>armsolar] veh line-mix 1xarmpincer+1xcorlevlr+3xarmsam vs veh line-pure 8xcorlevlr | guard | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.47 |
| m04 [even 1.09x p1.88] bot line-mix 4xcorak+3xcorstorm & bot brawl-mix 1xcorthud+1xarmwar vs veh line-mix 1xarmpincer+3xarmsam+1xcorlevlr+1xcorgator | charge | 11 | 0 | 0 | 1 | 92% | 65–99% | +0.21 |
| m04 [even 1.09x p1.88] bot line-mix 4xcorak+3xcorstorm & bot brawl-mix 1xcorthud+1xarmwar vs veh line-mix 1xarmpincer+3xarmsam+1xcorlevlr+1xcorgator | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.34 |
| m04 [even 1.09x p1.88] bot line-mix 4xcorak+3xcorstorm & bot brawl-mix 1xcorthud+1xarmwar vs veh line-mix 1xarmpincer+3xarmsam+1xcorlevlr+1xcorgator | kite | 4 | 8 | 0 | 0 | 33% | 14–61% | +0.07 |
| m04 [even 1.09x p1.88] bot line-mix 4xcorak+3xcorstorm & bot brawl-mix 1xcorthud+1xarmwar vs veh line-mix 1xarmpincer+3xarmsam+1xcorlevlr+1xcorgator | orbit | 5 | 7 | 0 | 0 | 42% | 19–68% | +0.06 |
| m04 [even 1.09x p1.88] bot line-mix 4xcorak+3xcorstorm & bot brawl-mix 1xcorthud+1xarmwar vs veh line-mix 1xarmpincer+3xarmsam+1xcorlevlr+1xcorgator | strike | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.32 |
| m04 [even 1.09x p1.88] bot line-mix 4xcorak+3xcorstorm & bot brawl-mix 1xcorthud+1xarmwar vs veh line-mix 1xarmpincer+3xarmsam+1xcorlevlr+1xcorgator | guard | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.34 |
| m05 [over 0.57x p5.09] bot brawl-pure 3xarmwar & bot line-pure 3xarmham vs veh scout-pure 8xarmsh | charge | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.31 |
| m05 [over 0.57x p5.09] bot brawl-pure 3xarmwar & bot line-pure 3xarmham vs veh scout-pure 8xarmsh | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.37 |
| m05 [over 0.57x p5.09] bot brawl-pure 3xarmwar & bot line-pure 3xarmham vs veh scout-pure 8xarmsh | kite | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.31 |
| m05 [over 0.57x p5.09] bot brawl-pure 3xarmwar & bot line-pure 3xarmham vs veh scout-pure 8xarmsh | orbit | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.27 |
| m05 [over 0.57x p5.09] bot brawl-pure 3xarmwar & bot line-pure 3xarmham vs veh scout-pure 8xarmsh | strike | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.32 |
| m05 [over 0.57x p5.09] bot brawl-pure 3xarmwar & bot line-pure 3xarmham vs veh scout-pure 8xarmsh | guard | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.37 |
| m06 [under 1.83x p0.07 atk>armsolar] veh artillery-pure 4xarmsam vs bot brawl-mix 4xcorthud+2xarmwar | charge | 6 | 0 | 0 | 6 | 50% | 25–75% | -0.07 |
| m06 [under 1.83x p0.07 atk>armsolar] veh artillery-pure 4xarmsam vs bot brawl-mix 4xcorthud+2xarmwar | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.62 |
| m06 [under 1.83x p0.07 atk>armsolar] veh artillery-pure 4xarmsam vs bot brawl-mix 4xcorthud+2xarmwar | kite | 3 | 0 | 0 | 9 | 25% | 9–53% | -0.12 |
| m06 [under 1.83x p0.07 atk>armsolar] veh artillery-pure 4xarmsam vs bot brawl-mix 4xcorthud+2xarmwar | orbit | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.34 |
| m06 [under 1.83x p0.07 atk>armsolar] veh artillery-pure 4xarmsam vs bot brawl-mix 4xcorthud+2xarmwar | strike | 5 | 0 | 0 | 7 | 42% | 19–68% | -0.10 |
| m06 [under 1.83x p0.07 atk>armsolar] veh artillery-pure 4xarmsam vs bot brawl-mix 4xcorthud+2xarmwar | guard | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.61 |
| m07 [over 0.42x p2.47 def>armsolar] veh scout-pure 14xarmsh vs bot scout-pure 24xarmflea | charge | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.13 |
| m07 [over 0.42x p2.47 def>armsolar] veh scout-pure 14xarmsh vs bot scout-pure 24xarmflea | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.15 |
| m07 [over 0.42x p2.47 def>armsolar] veh scout-pure 14xarmsh vs bot scout-pure 24xarmflea | kite | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.15 |
| m07 [over 0.42x p2.47 def>armsolar] veh scout-pure 14xarmsh vs bot scout-pure 24xarmflea | orbit | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.04 |
| m07 [over 0.42x p2.47 def>armsolar] veh scout-pure 14xarmsh vs bot scout-pure 24xarmflea | strike | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.28 |
| m07 [over 0.42x p2.47 def>armsolar] veh scout-pure 14xarmsh vs bot scout-pure 24xarmflea | guard | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.15 |
| m08 [over 0.65x p1.19 def>armsolar] veh artillery-pure 7xarmart vs veh scout-pure 5xcorsh & veh scout-mix 1xarmsh+3xcorfav+1xcorsh | charge | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.62 |
| m08 [over 0.65x p1.19 def>armsolar] veh artillery-pure 7xarmart vs veh scout-pure 5xcorsh & veh scout-mix 1xarmsh+3xcorfav+1xcorsh | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.32 |
| m08 [over 0.65x p1.19 def>armsolar] veh artillery-pure 7xarmart vs veh scout-pure 5xcorsh & veh scout-mix 1xarmsh+3xcorfav+1xcorsh | kite | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.55 |
| m08 [over 0.65x p1.19 def>armsolar] veh artillery-pure 7xarmart vs veh scout-pure 5xcorsh & veh scout-mix 1xarmsh+3xcorfav+1xcorsh | orbit | 5 | 0 | 0 | 7 | 42% | 19–68% | -0.08 |
| m08 [over 0.65x p1.19 def>armsolar] veh artillery-pure 7xarmart vs veh scout-pure 5xcorsh & veh scout-mix 1xarmsh+3xcorfav+1xcorsh | strike | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.31 |
| m08 [over 0.65x p1.19 def>armsolar] veh artillery-pure 7xarmart vs veh scout-pure 5xcorsh & veh scout-mix 1xarmsh+3xcorfav+1xcorsh | guard | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.32 |
| m09 [under 1.84x p1.17 atk>armlab] bot raid-mix 5xarmflea+3xcorak+4xarmpw & bot raid-pure 10xcorak vs veh artillery-pure 6xarmsam & veh line-pure 6xcorgator | charge | 5 | 0 | 0 | 7 | 42% | 19–68% | +0.23 |
| m09 [under 1.84x p1.17 atk>armlab] bot raid-mix 5xarmflea+3xcorak+4xarmpw & bot raid-pure 10xcorak vs veh artillery-pure 6xarmsam & veh line-pure 6xcorgator | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.34 |
| m09 [under 1.84x p1.17 atk>armlab] bot raid-mix 5xarmflea+3xcorak+4xarmpw & bot raid-pure 10xcorak vs veh artillery-pure 6xarmsam & veh line-pure 6xcorgator | kite | 6 | 0 | 0 | 6 | 50% | 25–75% | +0.18 |
| m09 [under 1.84x p1.17 atk>armlab] bot raid-mix 5xarmflea+3xcorak+4xarmpw & bot raid-pure 10xcorak vs veh artillery-pure 6xarmsam & veh line-pure 6xcorgator | orbit | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.18 |
| m09 [under 1.84x p1.17 atk>armlab] bot raid-mix 5xarmflea+3xcorak+4xarmpw & bot raid-pure 10xcorak vs veh artillery-pure 6xarmsam & veh line-pure 6xcorgator | strike | 6 | 0 | 0 | 6 | 50% | 25–75% | +0.29 |
| m09 [under 1.84x p1.17 atk>armlab] bot raid-mix 5xarmflea+3xcorak+4xarmpw & bot raid-pure 10xcorak vs veh artillery-pure 6xarmsam & veh line-pure 6xcorgator | guard | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.51 |
| m10 [even 0.93x p0.64 atk>armlab] veh scout-pure 16xarmfav & bot artillery-mix 2xarmham+2xcorstorm+2xarmrock vs bot raid-mix 4xarmpw+8xcorak+6xarmflea & veh scout-pure 6xcorsh | charge | 1 | 0 | 0 | 11 | 8% | 1–35% | -0.09 |
| m10 [even 0.93x p0.64 atk>armlab] veh scout-pure 16xarmfav & bot artillery-mix 2xarmham+2xcorstorm+2xarmrock vs bot raid-mix 4xarmpw+8xcorak+6xarmflea & veh scout-pure 6xcorsh | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.38 |
| m10 [even 0.93x p0.64 atk>armlab] veh scout-pure 16xarmfav & bot artillery-mix 2xarmham+2xcorstorm+2xarmrock vs bot raid-mix 4xarmpw+8xcorak+6xarmflea & veh scout-pure 6xcorsh | kite | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.21 |
| m10 [even 0.93x p0.64 atk>armlab] veh scout-pure 16xarmfav & bot artillery-mix 2xarmham+2xcorstorm+2xarmrock vs bot raid-mix 4xarmpw+8xcorak+6xarmflea & veh scout-pure 6xcorsh | orbit | 7 | 5 | 0 | 0 | 58% | 32–81% | +0.03 |
| m10 [even 0.93x p0.64 atk>armlab] veh scout-pure 16xarmfav & bot artillery-mix 2xarmham+2xcorstorm+2xarmrock vs bot raid-mix 4xarmpw+8xcorak+6xarmflea & veh scout-pure 6xcorsh | strike | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.05 |
| m10 [even 0.93x p0.64 atk>armlab] veh scout-pure 16xarmfav & bot artillery-mix 2xarmham+2xcorstorm+2xarmrock vs bot raid-mix 4xarmpw+8xcorak+6xarmflea & veh scout-pure 6xcorsh | guard | 10 | 2 | 0 | 0 | 83% | 55–95% | +0.31 |
| m11 [over 0.45x p1.69 atk>armlab] veh brawl-pure 2xcorsnap vs veh raid-pure 2xcorsh & veh raid-pure 1xcorgator | charge | 1 | 0 | 0 | 11 | 8% | 1–35% | -0.13 |
| m11 [over 0.45x p1.69 atk>armlab] veh brawl-pure 2xcorsnap vs veh raid-pure 2xcorsh & veh raid-pure 1xcorgator | hold | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.36 |
| m11 [over 0.45x p1.69 atk>armlab] veh brawl-pure 2xcorsnap vs veh raid-pure 2xcorsh & veh raid-pure 1xcorgator | kite | 0 | 0 | 0 | 12 | 0% | 0–24% | -0.29 |
| m11 [over 0.45x p1.69 atk>armlab] veh brawl-pure 2xcorsnap vs veh raid-pure 2xcorsh & veh raid-pure 1xcorgator | orbit | 11 | 1 | 0 | 0 | 92% | 65–99% | +0.26 |
| m11 [over 0.45x p1.69 atk>armlab] veh brawl-pure 2xcorsnap vs veh raid-pure 2xcorsh & veh raid-pure 1xcorgator | strike | 7 | 0 | 0 | 5 | 58% | 32–81% | +0.15 |
| m11 [over 0.45x p1.69 atk>armlab] veh brawl-pure 2xcorsnap vs veh raid-pure 2xcorsh & veh raid-pure 1xcorgator | guard | 12 | 0 | 0 | 0 | 100% | 76–100% | +0.40 |

Overall win rate 63%, mean fitness +0.72, **57% in battles where it was the underdog** (432 battles).

## `target_selector`

Scores one enemy at a time (same weights for every candidate); the highest-scoring of the three nearest enemies becomes the target.

Shape 6 → 6 → 1 · 49 parameters · 1484 samples

### output `score`

mean -0.54 · 10th pct -0.99 · median -0.90 · 90th pct +0.66 · strongly active (|x| > 0.5) 86% of the time

What it listens to, most influential first. *Swing* is the average change in `score` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `distance_over_range` | 0.10 – 1.91 | 1.46 | higher → lowers `score` | `████████████████████` |
| `enemy_hp_fraction` | 0.31 – 1.00 | 0.17 | higher → lowers `score` | `██··················` |
| `is_nearest` | 0.00 – 1.00 | 0.15 | higher → raises `score` | `██··················` |
| `enemy_value_over_mine` | 0.66 – 1.00 | 0.09 | higher → lowers `score` | `█···················` |
| `enemy_threat_over_mine` | 1.00 – 1.10 | 0.04 | higher → raises `score` | `█···················` |
| `enemy_range_over_mine` | 0.82 – 1.00 | 0.02 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `distance_over_range` | 0.05 → +0.54 | 0.79 → -0.88 | 1.53 → -0.97 | 2.26 → -0.99 | 3.00 → -0.99 |
| `enemy_hp_fraction` | 0.00 → -0.80 | 0.25 → -0.85 | 0.50 → -0.89 | 0.75 → -0.92 | 1.00 → -0.94 |
| `enemy_range_over_mine` | 0.82 → -0.91 | 0.87 → -0.92 | 0.91 → -0.92 | 0.96 → -0.92 | 1.00 → -0.93 |
| `enemy_threat_over_mine` | 1.00 → -0.93 | 1.03 → -0.92 | 1.05 → -0.92 | 1.08 → -0.92 | 1.10 → -0.92 |
| `enemy_value_over_mine` | 0.66 → -0.90 | 0.75 → -0.91 | 0.83 → -0.92 | 0.92 → -0.93 | 1.00 → -0.94 |
| `is_nearest` | 0.00 → -0.98 | 0.25 → -0.97 | 0.50 → -0.96 | 0.75 → -0.95 | 1.00 → -0.93 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias +0.03: reads -2.13`distance_over_range` +1.56`enemy_threat_over_mine` -0.53`enemy_hp_fraction`; pushes +1.27 → `score`
- **h1** bias +0.69: reads -0.89`enemy_value_over_mine` -0.43`distance_over_range` +0.28`is_nearest`; pushes +0.44 → `score`
- **h2** bias -0.06: reads -1.41`distance_over_range` -0.90`enemy_threat_over_mine` -0.20`enemy_hp_fraction`; pushes -0.10 → `score`
- **h3** bias -1.78: reads +0.88`enemy_hp_fraction` +0.81`enemy_range_over_mine` -0.77`is_nearest`; pushes -0.77 → `score`
- **h4** bias +0.12: reads -1.36`enemy_value_over_mine` -0.78`is_nearest` -0.57`enemy_range_over_mine`; pushes +0.07 → `score`
- **h5** bias -0.37: reads -1.07`distance_over_range` -1.02`is_nearest` -0.64`enemy_threat_over_mine`; pushes +0.84 → `score`

## `spacing`

Owns range: close on the target (+1) or back away from it (-1) along the line between them.

Shape 11 → 6 → 1 · 79 parameters · 1500 samples

### output `advance`

mean +0.08 · 10th pct -1.00 · median +0.43 · 90th pct +1.00 · strongly active (|x| > 0.5) 90% of the time

What it listens to, most influential first. *Swing* is the average change in `advance` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `distance_over_range` | 0.10 – 1.87 | 1.83 | higher → raises `advance` | `████████████████████` |
| `target_approaching` | -0.54 – 1.00 | 0.74 | higher → lowers `advance` | `████████············` |
| `my_hp_fraction` | 0.47 – 1.00 | 0.60 | higher → raises `advance` | `███████·············` |
| `my_reload_remaining` | 0.00 – 0.83 | 0.60 | higher → lowers `advance` | `███████·············` |
| `target_hp_fraction` | 0.30 – 1.00 | 0.28 | higher → lowers `advance` | `███·················` |
| `platoon_ours_over_theirs_power` | 0.17 – 1.00 | 0.25 | higher → raises `advance` | `███·················` |
| `platoon_theirs_over_ours_power` | 0.87 – 1.00 | 0.08 | higher → lowers `advance` | `█···················` |
| `ours_over_theirs_dps` | 0.40 – 0.96 | 0.05 | higher → raises `advance` | `█···················` |
| `target_range_over_mine` | 0.82 – 1.00 | 0.04 | higher → raises `advance` | `····················` |
| `my_range_rank` | 0.82 – 1.00 | 0.04 | higher → lowers `advance` | `····················` |
| `theirs_over_ours_dps` | 1.00 – 1.00 | 0.00 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `distance_over_range` | 0.05 → -1.00 | 0.79 → -0.21 | 1.53 → +1.00 | 2.26 → +1.00 | 3.00 → +1.00 |
| `my_reload_remaining` | 0.00 → +0.99 | 0.24 → +0.98 | 0.49 → +0.11 | 0.73 → -0.63 | 0.98 → -0.92 |
| `my_hp_fraction` | 0.01 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -0.04 | 1.00 → +0.99 |
| `target_range_over_mine` | 0.82 → +0.96 | 0.87 → +0.97 | 0.91 → +0.97 | 0.96 → +0.98 | 1.00 → +0.98 |
| `target_hp_fraction` | 0.00 → +1.00 | 0.25 → +1.00 | 0.50 → +1.00 | 0.75 → +0.99 | 1.00 → +0.95 |
| `target_approaching` | -1.00 → +1.00 | -0.50 → +1.00 | 0.00 → +0.98 | 0.50 → +0.42 | 1.00 → -0.65 |
| `ours_over_theirs_dps` | 0.14 → +0.98 | 0.36 → +0.98 | 0.57 → +0.98 | 0.79 → +0.98 | 1.00 → +0.98 |
| `theirs_over_ours_dps` | 0.00 → +1.00 | 0.25 → +1.00 | 0.50 → +1.00 | 0.75 → +1.00 | 1.00 → +0.98 |
| `my_range_rank` | 0.82 → +0.98 | 0.87 → +0.98 | 0.91 → +0.98 | 0.96 → +0.97 | 1.00 → +0.97 |
| `platoon_ours_over_theirs_power` | 0.03 → +0.93 | 0.27 → +0.97 | 0.52 → +0.99 | 0.76 → +0.99 | 1.00 → +0.99 |
| `platoon_theirs_over_ours_power` | 0.00 → +1.00 | 0.25 → +1.00 | 0.50 → +1.00 | 0.75 → +1.00 | 1.00 → +0.98 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -1.74: reads +3.34`distance_over_range` +3.18`my_hp_fraction` +1.13`platoon_ours_over_theirs_power`; pushes +3.73 → `advance`
- **h1** bias +19.91: reads +40.39`my_reload_remaining` -38.38`distance_over_range` +1.36`target_hp_fraction`; pushes -0.71 → `advance`
- **h2** bias -0.03: reads -0.58`my_range_rank` -0.55`target_hp_fraction` +0.47`distance_over_range`; pushes +1.29 → `advance`
- **h3** bias -0.03: reads -2.28`my_hp_fraction` +1.80`theirs_over_ours_dps` -1.30`target_range_over_mine`; pushes -2.09 → `advance`
- **h4** bias +0.17: reads +1.44`distance_over_range` +1.08`ours_over_theirs_dps` -0.97`theirs_over_ours_dps`; pushes -0.05 → `advance`
- **h5** bias -0.06: reads -1.54`my_reload_remaining` -1.21`target_approaching` +1.05`distance_over_range`; pushes +0.85 → `advance`

## `strafe`

Owns sideways motion: circle the target counter-clockwise (+1) or clockwise (-1), e.g. to dodge a projectile weapon.

Shape 8 → 6 → 1 · 61 parameters · 1500 samples

### output `orbit`

mean -0.39 · 10th pct -0.52 · median -0.40 · 90th pct -0.24 · strongly active (|x| > 0.5) 13% of the time

What it listens to, most influential first. *Swing* is the average change in `orbit` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `distance_over_range` | 0.10 – 1.87 | 0.28 | higher → lowers `orbit` | `████████████████████` |
| `my_hp_fraction` | 0.47 – 1.00 | 0.16 | higher → raises `orbit` | `████████████········` |
| `ours_over_theirs_dps` | 0.40 – 0.96 | 0.08 | higher → lowers `orbit` | `██████··············` |
| `my_reload_remaining` | 0.00 – 0.83 | 0.08 | higher → raises `orbit` | `██████··············` |
| `target_approaching` | -0.54 – 1.00 | 0.04 | mixed (depends on other inputs) | `███·················` |
| `my_speed_rank` | 0.84 – 1.00 | 0.03 | higher → raises `orbit` | `██··················` |
| `target_range_over_mine` | 0.82 – 1.00 | 0.01 | ignored | `█···················` |
| `theirs_over_ours_dps` | 1.00 – 1.00 | 0.00 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `distance_over_range` | 0.05 → -0.24 | 0.79 → -0.40 | 1.53 → -0.48 | 2.26 → -0.52 | 3.00 → -0.55 |
| `my_reload_remaining` | 0.00 → -0.41 | 0.24 → -0.41 | 0.49 → -0.40 | 0.73 → -0.37 | 0.98 → -0.31 |
| `target_approaching` | -1.00 → -0.39 | -0.50 → -0.40 | 0.00 → -0.41 | 0.50 → -0.43 | 1.00 → -0.43 |
| `target_range_over_mine` | 0.82 → -0.40 | 0.87 → -0.41 | 0.91 → -0.41 | 0.96 → -0.41 | 1.00 → -0.41 |
| `my_hp_fraction` | 0.01 → -0.60 | 0.25 → -0.58 | 0.50 → -0.54 | 0.75 → -0.48 | 1.00 → -0.41 |
| `ours_over_theirs_dps` | 0.14 → -0.34 | 0.36 → -0.38 | 0.57 → -0.41 | 0.79 → -0.44 | 1.00 → -0.47 |
| `theirs_over_ours_dps` | 0.00 → -0.17 | 0.25 → -0.21 | 0.50 → -0.27 | 0.75 → -0.34 | 1.00 → -0.41 |
| `my_speed_rank` | 0.84 → -0.43 | 0.88 → -0.42 | 0.92 → -0.41 | 0.96 → -0.40 | 1.00 → -0.39 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -0.82: reads +1.12`my_reload_remaining` -0.83`distance_over_range` +0.48`my_hp_fraction`; pushes +0.38 → `orbit`
- **h1** bias -0.07: reads -1.34`my_hp_fraction` +0.97`theirs_over_ours_dps` +0.89`ours_over_theirs_dps`; pushes -0.25 → `orbit`
- **h2** bias +0.86: reads +0.96`ours_over_theirs_dps` -0.80`my_reload_remaining` -0.67`theirs_over_ours_dps`; pushes +0.04 → `orbit`
- **h3** bias -0.75: reads -0.90`my_reload_remaining` +0.87`my_speed_rank` -0.45`my_hp_fraction`; pushes +0.15 → `orbit`
- **h4** bias -0.08: reads +0.75`theirs_over_ours_dps` -0.75`my_hp_fraction` -0.31`my_speed_rank`; pushes -0.10 → `orbit`
- **h5** bias -0.47: reads -0.66`distance_over_range` -0.56`my_speed_rank` +0.37`target_approaching`; pushes +0.06 → `orbit`

## `cohesion`

Owns group shape: drift toward the centre of friendly units (+1) or spread away from it (-1). Ignored when the unit has no allies.

Shape 5 → 6 → 1 · 43 parameters · 1471 samples

### output `regroup`

mean -0.46 · 10th pct -0.58 · median -0.49 · 90th pct -0.31 · strongly active (|x| > 0.5) 46% of the time

What it listens to, most influential first. *Swing* is the average change in `regroup` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `nearest_ally_distance_over_range` | 0.15 – 2.33 | 0.19 | higher → raises `regroup` | `████████████████████` |
| `ally_centroid_distance_over_range` | 0.47 – 2.48 | 0.12 | higher → raises `regroup` | `████████████········` |
| `my_hp_fraction` | 0.47 – 1.00 | 0.11 | higher → lowers `regroup` | `████████████········` |
| `distance_over_range` | 0.09 – 1.79 | 0.09 | higher → raises `regroup` | `█████████···········` |
| `ours_over_theirs_count` | 0.38 – 1.00 | 0.06 | higher → raises `regroup` | `██████··············` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `ally_centroid_distance_over_range` | 0.00 → -0.56 | 0.75 → -0.57 | 1.50 → -0.49 | 2.25 → -0.43 | 3.00 → -0.41 |
| `nearest_ally_distance_over_range` | 0.01 → -0.55 | 0.76 → -0.51 | 1.50 → -0.45 | 2.25 → -0.37 | 3.00 → -0.26 |
| `my_hp_fraction` | 0.00 → -0.28 | 0.25 → -0.36 | 0.50 → -0.42 | 0.75 → -0.48 | 1.00 → -0.53 |
| `ours_over_theirs_count` | 0.14 → -0.57 | 0.36 → -0.55 | 0.57 → -0.53 | 0.79 → -0.51 | 1.00 → -0.49 |
| `distance_over_range` | 0.01 → -0.55 | 0.65 → -0.53 | 1.29 → -0.52 | 1.94 → -0.51 | 2.58 → -0.48 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -1.51: reads +0.55`ally_centroid_distance_over_range` +0.34`distance_over_range` +0.33`nearest_ally_distance_over_range`; pushes +0.05 → `regroup`
- **h1** bias -0.07: reads +1.21`ally_centroid_distance_over_range` -0.68`distance_over_range` -0.54`my_hp_fraction`; pushes +0.18 → `regroup`
- **h2** bias -0.09: reads -0.88`my_hp_fraction` +0.53`nearest_ally_distance_over_range` -0.48`ally_centroid_distance_over_range`; pushes +0.25 → `regroup`
- **h3** bias +0.49: reads +0.45`distance_over_range` -0.33`my_hp_fraction` -0.28`ours_over_theirs_count`; pushes +0.09 → `regroup`
- **h4** bias -0.83: reads -0.90`ally_centroid_distance_over_range` +0.69`ours_over_theirs_count` +0.38`distance_over_range`; pushes +0.17 → `regroup`
- **h5** bias -0.56: reads +0.88`ours_over_theirs_count` +0.24`distance_over_range` +0.22`nearest_ally_distance_over_range`; pushes +0.19 → `regroup`

## `engagement`

Owns fight or flight: reads the friend/enemy force balance and, above zero, pulls the unit out of the fight by that fraction, running away from the enemy force. At or below zero it fights. Never retreats while the platoon is clearly ahead (the doctrine).

Shape 22 → 6 → 1 · 145 parameters · 1500 samples

### output `retreat`

mean -0.98 · 10th pct -1.00 · median -1.00 · 90th pct -1.00 · strongly active (|x| > 0.5) 100% of the time

What it listens to, most influential first. *Swing* is the average change in `retreat` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `platoon_ours_over_theirs_power` | 0.17 – 1.00 | 0.02 | ignored | `████████████████████` |
| `ours_over_theirs_hp` | 0.44 – 1.00 | 0.00 | ignored | `████················` |
| `ours_over_theirs_value` | 0.57 – 1.00 | 0.00 | ignored | `███·················` |
| `ours_over_theirs_dps` | 0.40 – 0.96 | 0.00 | ignored | `██··················` |
| `ours_over_theirs_count` | 0.43 – 1.00 | 0.00 | ignored | `██··················` |
| `theirs_over_ours_hp` | 0.77 – 1.00 | 0.00 | ignored | `█···················` |
| `my_hp_fraction` | 0.47 – 1.00 | 0.00 | ignored | `█···················` |
| `my_speed_rank` | 0.84 – 1.00 | 0.00 | ignored | `█···················` |
| `ours_over_theirs_dps_mid` | 0.40 – 0.96 | 0.00 | ignored | `█···················` |
| `platoon_theirs_over_ours_power` | 0.87 – 1.00 | 0.00 | ignored | `····················` |
| `my_hp_rank` | 0.78 – 1.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_value` | 0.82 – 1.00 | 0.00 | ignored | `····················` |
| `my_range_rank` | 0.82 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_speed` | 0.99 – 1.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_speed` | 0.99 – 1.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_count` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_dps` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_dps_short` | 0.00 – 0.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_dps_short` | 0.00 – 0.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_dps_mid` | 1.00 – 1.00 | 0.00 | ignored | `····················` |
| `ours_over_theirs_dps_long` | 0.00 – 0.00 | 0.00 | ignored | `····················` |
| `theirs_over_ours_dps_long` | 0.00 – 0.00 | 0.00 | ignored | `····················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `ours_over_theirs_count` | 0.14 → -1.00 | 0.36 → -1.00 | 0.57 → -1.00 | 0.79 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_count` | 0.00 → -0.99 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_hp` | 0.01 → -0.99 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_hp` | 0.00 → -0.99 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_dps` | 0.14 → -1.00 | 0.36 → -1.00 | 0.57 → -1.00 | 0.79 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_dps` | 0.00 → -0.99 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_value` | 0.16 → -1.00 | 0.37 → -1.00 | 0.58 → -1.00 | 0.79 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_value` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_speed` | 0.93 → -1.00 | 0.95 → -1.00 | 0.96 → -1.00 | 0.98 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_speed` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `ours_over_theirs_dps_mid` | 0.14 → -1.00 | 0.36 → -1.00 | 0.57 → -1.00 | 0.79 → -1.00 | 1.00 → -1.00 |
| `theirs_over_ours_dps_mid` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `my_hp_fraction` | 0.01 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |
| `my_speed_rank` | 0.84 → -1.00 | 0.88 → -1.00 | 0.92 → -1.00 | 0.96 → -1.00 | 1.00 → -1.00 |
| `my_range_rank` | 0.82 → -1.00 | 0.87 → -1.00 | 0.91 → -1.00 | 0.96 → -1.00 | 1.00 → -1.00 |
| `my_hp_rank` | 0.78 → -1.00 | 0.83 → -1.00 | 0.89 → -1.00 | 0.94 → -1.00 | 1.00 → -1.00 |
| `platoon_ours_over_theirs_power` | 0.03 → -0.95 | 0.27 → -1.00 | 0.52 → -1.00 | 0.76 → -1.00 | 1.00 → -1.00 |
| `platoon_theirs_over_ours_power` | 0.00 → -1.00 | 0.25 → -1.00 | 0.50 → -1.00 | 0.75 → -1.00 | 1.00 → -1.00 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -3.06: reads -4.64`platoon_ours_over_theirs_power` +2.71`platoon_theirs_over_ours_power` +1.22`ours_over_theirs_dps`; pushes +1.94 → `retreat`
- **h1** bias +0.12: reads -1.79`my_hp_rank` -1.23`ours_over_theirs_dps_short` -1.14`my_hp_fraction`; pushes -0.03 → `retreat`
- **h2** bias -1.53: reads +1.66`my_hp_rank` -1.47`my_hp_fraction` -1.36`theirs_over_ours_dps_long`; pushes -0.20 → `retreat`
- **h3** bias -0.25: reads -0.69`theirs_over_ours_value` +0.58`ours_over_theirs_speed` +0.51`ours_over_theirs_dps`; pushes +0.37 → `retreat`
- **h4** bias -1.31: reads +1.54`ours_over_theirs_hp` +1.15`theirs_over_ours_value` -0.93`ours_over_theirs_dps_short`; pushes -0.96 → `retreat`
- **h5** bias -0.16: reads +1.69`ours_over_theirs_dps_short` -1.20`theirs_over_ours_dps_mid` +0.69`ours_over_theirs_speed`; pushes -0.18 → `retreat`

## `encircle`

Owns the formation around the enemy force: slides the unit around it (+1 counter-clockwise) toward the larger gap between itself and its neighbours, so friends spread into an even ring, and away from the enemy's line of advance, so they flank instead of bunching in front. Scaled up as the platoon gets further ahead.

Shape 5 → 6 → 1 · 43 parameters · 1484 samples

### output `bloom`

mean -0.18 · 10th pct -0.96 · median -0.48 · 90th pct +0.96 · strongly active (|x| > 0.5) 80% of the time

What it listens to, most influential first. *Swing* is the average change in `bloom` when the input goes from its typical low to its typical high.

| input | typical range (10–90%) | swing | effect | |
|---|---|---|---|---|
| `gap_ccw` | 0.01 – 0.72 | 1.56 | higher → raises `bloom` | `████████████████████` |
| `gap_cw` | 0.02 – 0.70 | 1.32 | higher → lowers `bloom` | `█████████████████···` |
| `enemy_side` | -0.87 – 0.97 | 0.68 | higher → lowers `bloom` | `█████████···········` |
| `enemy_front_alignment` | -0.72 – 1.00 | 0.13 | higher → raises `bloom` | `██··················` |
| `ring_distance_over_range` | 0.52 – 1.95 | 0.07 | higher → raises `bloom` | `█···················` |

Sweeps (one input varied across what it really takes; others at their median):

| input | point 1 | point 2 | point 3 | point 4 | point 5 |
|---|---|---|---|---|---|
| `enemy_front_alignment` | -1.00 → -0.84 | -0.50 → -0.80 | 0.00 → -0.73 | 0.50 → -0.64 | 1.00 → -0.59 |
| `enemy_side` | -1.00 → +0.67 | -0.50 → +0.18 | 0.00 → -0.46 | 0.50 → -0.79 | 1.00 → -0.88 |
| `gap_ccw` | 0.00 → -0.93 | 0.25 → +0.31 | 0.50 → +0.96 | 0.75 → +0.96 | 1.00 → +0.96 |
| `gap_cw` | 0.00 → +0.92 | 0.25 → -0.90 | 0.50 → -0.95 | 0.75 → -0.95 | 1.00 → -0.95 |
| `ring_distance_over_range` | 0.06 → -0.69 | 0.70 → -0.64 | 1.35 → -0.60 | 1.99 → -0.55 | 2.63 → -0.49 |

Hidden units (each reads the inputs, then pushes the outputs):

- **h0** bias -0.34: reads -8.51`gap_cw` +7.28`gap_ccw` -0.99`enemy_side`; pushes +1.96 → `bloom`
- **h1** bias +0.46: reads +2.73`enemy_side` +0.80`enemy_front_alignment` -0.28`ring_distance_over_range`; pushes +0.12 → `bloom`
- **h2** bias +0.43: reads +1.15`enemy_front_alignment` +0.84`enemy_side` -0.59`ring_distance_over_range`; pushes +0.17 → `bloom`
- **h3** bias +1.48: reads -1.94`enemy_front_alignment` -0.95`gap_ccw` +0.87`enemy_side`; pushes +0.08 → `bloom`
- **h4** bias +0.45: reads -0.86`enemy_front_alignment` +0.77`enemy_side` +0.51`gap_ccw`; pushes +0.03 → `bloom`
- **h5** bias +0.34: reads -1.02`gap_ccw` -0.63`ring_distance_over_range` +0.51`enemy_side`; pushes +0.08 → `bloom`

## Worked decisions

**t = 0.0s, unit #1** at (606, 404) chose enemy #6
- `target_selector` [enemy #6 (rank 0)]: distance_over_range=2.58, enemy_hp_fraction=1.00, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=1.00 → **score=-0.99**
- `target_selector` [enemy #7 (rank 1)]: distance_over_range=2.59, enemy_hp_fraction=1.00, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=0.00 → **score=-1.00**
- `target_selector` [enemy #8 (rank 2)]: distance_over_range=2.60, enemy_hp_fraction=1.00, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=0.00 → **score=-1.00**
- `spacing`: distance_over_range=2.58, my_reload_remaining=0.00, my_hp_fraction=1.00, target_range_over_mine=1.00, target_hp_fraction=1.00, target_approaching=0.00, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, my_range_rank=0.82, platoon_ours_over_theirs_power=0.42, platoon_theirs_over_ours_power=1.00 → **advance=+1.00**
- `strafe`: distance_over_range=2.58, my_reload_remaining=0.00, target_approaching=0.00, target_range_over_mine=1.00, my_hp_fraction=1.00, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, my_speed_rank=0.84 → **orbit=-0.53**
- `cohesion`: ally_centroid_distance_over_range=0.39, nearest_ally_distance_over_range=0.16, my_hp_fraction=1.00, ours_over_theirs_count=0.62, distance_over_range=2.58 → **regroup=-0.43**
- `engagement`: ours_over_theirs_count=0.62, theirs_over_ours_count=1.00, ours_over_theirs_hp=0.69, theirs_over_ours_hp=1.00, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, ours_over_theirs_value=0.76, theirs_over_ours_value=1.00, ours_over_theirs_speed=1.00, theirs_over_ours_speed=0.99, ours_over_theirs_dps_short=0.00, theirs_over_ours_dps_short=0.00, ours_over_theirs_dps_mid=0.60, theirs_over_ours_dps_mid=1.00, ours_over_theirs_dps_long=0.00, theirs_over_ours_dps_long=0.00, my_hp_fraction=1.00, my_speed_rank=0.84, my_range_rank=0.82, my_hp_rank=0.83, platoon_ours_over_theirs_power=0.42, platoon_theirs_over_ours_power=1.00 → **retreat=-1.00**
- `encircle`: enemy_front_alignment=0.99, enemy_side=0.12, gap_ccw=0.96, gap_cw=0.01, ring_distance_over_range=2.63 → **bloom=+0.95**
- combined: advance +1.00 · orbit -0.53 · regroup -0.43 · retreat -1.00 · bloom +0.95 (platoon commit 0.00) → movement (+1.02, -1.16), throttle 1.00

**t = 18.0s, unit #1** at (888, 560) chose enemy #13
- `target_selector` [enemy #13 (rank 0)]: distance_over_range=1.60, enemy_hp_fraction=0.96, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=1.00 → **score=-0.98**
- `target_selector` [enemy #12 (rank 1)]: distance_over_range=1.66, enemy_hp_fraction=1.00, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=0.00 → **score=-0.99**
- `target_selector` [enemy #11 (rank 2)]: distance_over_range=1.69, enemy_hp_fraction=0.95, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=0.89, is_nearest=0.00 → **score=-0.99**
- `spacing`: distance_over_range=1.60, my_reload_remaining=0.00, my_hp_fraction=1.00, target_range_over_mine=1.00, target_hp_fraction=0.96, target_approaching=-0.91, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, my_range_rank=0.82, platoon_ours_over_theirs_power=0.37, platoon_theirs_over_ours_power=1.00 → **advance=+1.00**
- `strafe`: distance_over_range=1.60, my_reload_remaining=0.00, target_approaching=-0.91, target_range_over_mine=1.00, my_hp_fraction=1.00, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, my_speed_rank=0.84 → **orbit=-0.44**
- `cohesion`: ally_centroid_distance_over_range=1.24, nearest_ally_distance_over_range=0.63, my_hp_fraction=1.00, ours_over_theirs_count=0.62, distance_over_range=1.60 → **regroup=-0.52**
- `engagement`: ours_over_theirs_count=0.62, theirs_over_ours_count=1.00, ours_over_theirs_hp=0.61, theirs_over_ours_hp=1.00, ours_over_theirs_dps=0.60, theirs_over_ours_dps=1.00, ours_over_theirs_value=0.76, theirs_over_ours_value=1.00, ours_over_theirs_speed=1.00, theirs_over_ours_speed=0.99, ours_over_theirs_dps_short=0.00, theirs_over_ours_dps_short=0.00, ours_over_theirs_dps_mid=0.60, theirs_over_ours_dps_mid=1.00, ours_over_theirs_dps_long=0.00, theirs_over_ours_dps_long=0.00, my_hp_fraction=1.00, my_speed_rank=0.84, my_range_rank=0.82, my_hp_rank=0.83, platoon_ours_over_theirs_power=0.37, platoon_theirs_over_ours_power=1.00 → **retreat=-1.00**
- `encircle`: enemy_front_alignment=-0.90, enemy_side=0.44, gap_ccw=0.22, gap_cw=0.06, ring_distance_over_range=1.76 → **bloom=+0.66**
- combined: advance +1.00 · orbit -0.44 · regroup -0.52 · retreat -1.00 · bloom +0.66 (platoon commit 0.00) → movement (-0.70, +0.38), throttle 0.79

**t = 39.6s, unit #4** at (1300, 289) chose enemy #8
- `target_selector` [enemy #8 (rank 0)]: distance_over_range=0.70, enemy_hp_fraction=0.10, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=1.00, is_nearest=1.00 → **score=-0.59**
- `target_selector` [enemy #9 (rank 1)]: distance_over_range=0.81, enemy_hp_fraction=0.74, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=1.00, is_nearest=0.00 → **score=-0.97**
- `target_selector` [enemy #13 (rank 2)]: distance_over_range=0.87, enemy_hp_fraction=0.17, enemy_range_over_mine=1.00, enemy_threat_over_mine=1.00, enemy_value_over_mine=1.00, is_nearest=0.00 → **score=-0.94**
- `spacing`: distance_over_range=0.70, my_reload_remaining=0.09, my_hp_fraction=0.47, target_range_over_mine=1.00, target_hp_fraction=0.10, target_approaching=0.00, ours_over_theirs_dps=1.00, theirs_over_ours_dps=0.83, my_range_rank=0.82, platoon_ours_over_theirs_power=1.00, platoon_theirs_over_ours_power=0.24 → **advance=+1.00**
- `strafe`: distance_over_range=0.70, my_reload_remaining=0.09, target_approaching=0.00, target_range_over_mine=1.00, my_hp_fraction=0.47, ours_over_theirs_dps=1.00, theirs_over_ours_dps=0.83, my_speed_rank=0.90 → **orbit=-0.52**
- `cohesion`: ally_centroid_distance_over_range=0.98, nearest_ally_distance_over_range=0.83, my_hp_fraction=0.47, ours_over_theirs_count=1.00, distance_over_range=0.70 → **regroup=-0.38**
- `engagement`: ours_over_theirs_count=1.00, theirs_over_ours_count=0.80, ours_over_theirs_hp=1.00, theirs_over_ours_hp=0.28, ours_over_theirs_dps=1.00, theirs_over_ours_dps=0.83, ours_over_theirs_value=1.00, theirs_over_ours_value=0.66, ours_over_theirs_speed=1.00, theirs_over_ours_speed=0.99, ours_over_theirs_dps_short=0.00, theirs_over_ours_dps_short=0.00, ours_over_theirs_dps_mid=1.00, theirs_over_ours_dps_mid=0.83, ours_over_theirs_dps_long=0.00, theirs_over_ours_dps_long=0.00, my_hp_fraction=0.47, my_speed_rank=0.90, my_range_rank=0.82, my_hp_rank=0.78, platoon_ours_over_theirs_power=1.00, platoon_theirs_over_ours_power=0.24 → **retreat=-1.00**
- `encircle`: enemy_front_alignment=0.32, enemy_side=-0.95, gap_ccw=0.18, gap_cw=0.31, ring_distance_over_range=0.83 → **bloom=-0.89**
- combined: advance +1.00 · orbit -0.52 · regroup -0.38 · retreat -1.00 · bloom -0.89 (platoon commit 1.00) → movement (-0.14, +0.73), throttle 0.75

