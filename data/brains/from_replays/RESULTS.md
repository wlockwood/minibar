# Baseline training results

Measured on 2026-10-16 in this checkout. The retained run is `baseline-v1`.
Starting brains are authored from the research; these are evolution-strategy
experiments in MiniBAR, not replay imitation or measured BAR win rates.

## Three generations

| Brain | Parameters | Initial validation fitness | Retained best | Latest | Best generation | Training wall time |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| tactical_v4 | 743 | -0.046388 | 0.012027 | -0.057301 | 1 | 6.16 s |
| strategic_v1 | 387 | 0.422335 | 0.422335 | 0.422335 | 0 (initial) | 84.01 s |

The two fitness functions have different definitions; their numbers are not
comparable across brain kinds. Wall times are observed command times using four
Rayon threads, not a controlled hardware benchmark.

Tactical validation comprised 144 battles: three factions, skirmish/attack/defend
objectives, four fixed opponents, both starting sides and two validation seeds.
Training used one different seed per battle, population 8, sigma 0.08 and learning
rate 0.02. Wins were **80/144 initially, 82/144 at the retained best, and 79/144
at generation 3**. The retained checkpoint improved its configured reward; the
latest checkpoint regressed. This small result is not statistically established
strength improvement.

Strategic validation comprised 18 rollouts: Center Command, Zed Remake and
synthetic 2048; all three factions; both learner sides. Each rollout lasted 180
simulated seconds against the authored strategic incumbent, with seeded start
jitter and filtered enemy observations. Population was 6, sigma 0.08, learning
rate 0.02 and one training/validation seed each, from separate seed ranges.
Training mutated weights but did not improve validation behavior or fitness in
three generations, so `brain.best.json` correctly retains the initial brain.

Across the best checkpoint's 18 validation rollouts, it completed **38
construction projects and produced 79 units**. Every rollout completed at least
one project and produced at least two units. The remaining placement limitation
is visible: one Legion/Zed starting-side case issued 22 rejected placements;
the other 17 cases had zero rejections. These were rejected by the authoritative
placement checks. Site retry/occupancy handling needs further refinement before
long unattended economic training. The saved summaries include rejection reasons.

## Additional tactical evaluations

All use evaluation seed base 2,000,000,000, separate from training and validation.
The scale cases have a 15-second simulated limit and three faction pairings,
mirrored across starting sides. The Zed arena was held out of tactical training.

| Evaluation | Battles | Initial fitness | Best fitness | Initial wins | Best wins |
| --- | ---: | ---: | ---: | ---: | ---: |
| 64 versus 64, Center Command | 6 | -0.035508 | -0.027385 | 2 | 2 |
| 256 versus 256, synthetic 2048 | 6 | -0.021146 | -0.017830 | 2 | 2 |
| Zed Remake objectives | 72 | -0.074023 | -0.052036 | 40 | 39 |

Each scale checkpoint had four timeouts out of six battles. The best checkpoint
improved the reward slightly without increasing scale-test wins. On Zed its
reward improved while wins fell by one. These results show why reward, wins and
timeouts must be reported separately. The six 512-unit battles took about 7.86 s
for the baseline and 8.45 s for the retained checkpoint.

## Reproducibility and limits

- [README.md](README.md) contains commands to repeat or extend training.
- [Run receipts](runs/baseline-v1/run_receipts.json) record commands, wall times,
  exit codes, stdout and stderr paths.
- [Input provenance](runs/baseline-v1/input_provenance.json) hashes the executable,
  scripts, catalog, curricula, baseline weights and map caches. The run directory
  retains copies of its catalog, asset manifest and catalog adjustment provenance.
- [Checkpoint hashes](runs/baseline-v1/checkpoint_hashes.json) identify initial,
  best and latest weights. The tactical and strategic training directories retain
  resolved configs, generation histories and full before/after summaries.
- A reloaded strategic best checkpoint is evaluated again in
  `runs/baseline-v1/strategy.reloaded.evaluation.json`; its complete validation
  report, including state hashes, must exactly match the saved best report.
  `strategy.heldout.evaluation.json` uses two additional seeds at 3,000,000,000:
  36 rollouts, fitness 0.417788.

Claude's 354-unit prepared catalog was used, but the curriculum selects only the
bounded T1 roles listed in the manifest. The [support audit](catalog_audit/SUPPORT_AUDIT.md)
identifies catalog entries that need converter/runtime work before training.
The [sensor gap review](catalog_audit/CURRICULUM_SENSOR_GAPS.md) records the next
steps for mixed weapons, emergency AA and economy defense.

Tactical combat still has full battlefield state and one commanded target per
chassis. Strategic rollouts use the economic World and its existing combat
executor; the two learned levels are not yet composed into one controller. The
short strategic curriculum mainly exercises early economic growth, not long
games or all mission sensors. The maps are coarse replay captures; flight,
underwater behavior, gadgets, native-runtime balance and human-match strength
are outside what these trials establish.

Earlier `trial`, `validated` and `final` directories are retained as development
diagnostics. They used earlier placement handling; use `baseline-v1` for the
current result. The original strict-flat catalog/manifest and receipts are also
preserved under `diagnostics/initial-flat-placement`.

Verification: `cargo test --release --quiet` passed 324 library tests and nine
other tests; the preparation/runner and existing evolution Python suites passed
24 tests. Release build, checkpoint reload comparison and whitespace checks
passed. The final code received a read-only review; native BAR integration was
not tested.
