# Campaign curriculum review

Read-only review of `docs/superpowers/plans/2026-10-17-training-campaign.md`, `examples/campaign/` (config,
README, all eight stage files, smoke files not examined), and the code they run (`train.rs`, `baselines.rs`,
`campaign.rs`, `campaign_eval.rs`, `scripts/package_training.py`). Written 2026-10-17 around 10:45 while Codex's
work was in progress, so file contents may have moved since. **[V]** = verified in files or by computation;
**[I]** = inference or not run. No simulation was run.

What is already sound (so it is not re-litigated): training seeds are fresh per generation and shared by all
candidates and islands (common random numbers; `campaign.rs:569-575`); validation and holdout use separate seed
bands; the learner is evaluated on both sides (`train.rs:616`, `strategic.rs:1006`); stage weights are
independent of battle counts (`campaign_eval.rs:130`); the packaging step records a backend identity per job.

## 1. Scenarios that cannot exercise their named decisions

Ordered by how much training signal is wasted.

1. **High [V]: `defend` scenarios against the `guard` opponent have no threat.** `Guard` defends *its own* key unit
   (`baselines.rs:68-71`). In a defend scenario the lab belongs to the learner, so the enemy has no `home`; it then stands
   still at throttle 0 and fires only at what wanders into range. The objectives stage uses opponents
   `strike, guard, kite` for all six scenarios, so one third of every defend battle is a free, passive win.
   `kite` is also not an asset threat. Only `strike` actually attacks the lab, and the holdout adds `charge`.
   Effect: the learner can ignore the asset and still win about two thirds of "defend", so the protect-asset decision
   is under-trained and the stage fitness over-states it. Fix: per-scenario opponent lists, with defend scenarios using
   only `strike`/`charge`, or add an attacker mode that targets the nearest enemy asset.
2. **High [V]: `mission` has no signal in two of three stages for either track.** Tactical `mission` (v4) and strategic
   `mission`/combat need a key asset or `combat: true`. `tactical-easy` and `tactical-mixed` have no objective, and
   `strategic-easy` and `strategic-mixed` set `combat: false` (60% of stage weight). So the mission modules train on
   the 0.40-weight objectives stage only. That is probably intended, but then the "easy stages remain in every
   generation" guarantee is mostly a *forgetting* guard for other modules, not extra training for `mission`.
3. **Medium [V]: 1v1 easy cases cannot exercise `target_selector`, `cohesion` or `encircle`.** There is one candidate,
   no allies, and one enemy (`TARGET_CANDIDATES` logic in `tactical.rs`). With `train_modules: []` every
   module is perturbed, so those modules' perturbations add pure noise to the ES gradient in two of three easy scenarios
   (armpw vs corak, leggob vs armpw). Not wrong, but weight these scenarios down or drop 1v1 once duels are won.
4. **Medium [V]: every scenario is count-balanced (1v1, 2v2, 5v5).** The `commit` doctrine (0 at 1.5x, 1 at 2.5x side power,
   `tactical.rs:79-80`) and retreat/underdog behavior never meet a clearly stronger or weaker force except by faction chance. Previous
   work showed those regimes carry most of the learned gain (underdog wins 0 to 250 of 576, 5v3 at 100%). Add 5v3 and 3v5
   variants, and keep favourite-win and underdog-win weighting in the objective explicit.
5. **Medium [I]: no stage uses the cover/terrain decisions.** `easy` and `mixed` are on flat synthetic maps; the
   formation always lines up on the x axis (`train.rs:510-560`). Only objectives and holdout use real maps, and those are
   32 by 32 coarse captures (256 by 128 elmo cells on Center Command), so terrain decisions are learned from very
   coarse passability.
6. **Low [V]: `layout_jitter: 8` elmo** against separations of 220 to 350 means geometry is nearly identical every
   generation; variation comes only from engine noise and opponent policy. See section 4 (overfitting).

## 2. Representation and placement traps

1. **Metal is wildly asymmetric and differently scaled per map [V].** Center Command has 2 metal nodes: (7040, 3136)
   yield 640, and (384, 3776) yield 2432. Start 0 (384, 192) is 3,584 elmo from its nearest node; start 1 (7808, 3904) is
   about 1,086 from its nearest. Zed has 3 nodes of yield 27,907 (about 11 to 44 times Center Command's per node);
   start 0 (1440, 1056) is about 1,822 from its nearest node, start 1 (4704, 5088) about 607. Both sides are evaluated
   (the learner swaps sides), which cancels the *bias*, but raw economy fitness is not comparable across maps, and the
   far-node starts may make "expand" almost unaffordable on one side. Normalize economy terms per map, or choose starts
   with comparable node distances. `continuous_metal` exists in both sources and may offset this; I did not check
   how construction uses it.
2. **Zed starts sit beside impassable cells [V].** Both Zed starts have only 32 of 45 sampled points passable within
   400 elmo (centre cell passable), and the whole map is 54% passable for `bot2` (87% on Center Command). Placement
   rejections are already recorded for one Legion/Zed case (Codex's earlier note); expect more once the
   construction stage places many structures here. Holdout-only, so it affects reporting rather than selection.
3. **Arena centres are fine [V].** Both tactical centres (Center Command (6400, 2048), Zed (1728, 4032)) match
   the manifest and have 45 of 45 sampled points passable within 400 elmo. Note the Center Command centre is only 1,792 elmo
   from the east edge, so it is not the map's centre; harmless but misleading in docs.
4. **Cell size vs unit size [I].** Every config uses `cell_size: 64` while captured cells are 192 to 256 elmo wide.
   The 64-elmo raster adds no detail (README says so), but it does make 10 to 30 elmo units path through resampled
   cells; keep the same cell size in training and in any later evaluation.
5. **`bot2` mask for every unit [V in docs, I in effect].** The README states all mobile combat types use `bot2`; leggob,
   legcen, legbal and the others have different real movement classes. Hover-like or taller-slope units are
   mis-routed in a way that is the same for both sides here, but it is a fidelity loss to disclose in results.
6. **Faction cycle is fixed [V].** Armada faces Cortex, Cortex faces Legion, Legion faces Armada in every stage.
   The learner never plays the same faction twice against the same enemy faction, so a win rate per matchup has one
   data point per stage; do not read per-faction conclusions from it.

## 3. Replay leakage

1. **High for later work [V]: both source replays are in the imitation `train` split.** Center Command
   (`02b3a46a...`) and Zed (`accaa36a...`) are in `trajectories-v2/train` and in neither validation nor
   `test_ood`. Today's baselines are authored, not imitation-trained, so there is no leak *now*. The moment a
   brain is initialised from replay imitation (items 5 to 8 in `docs/REPLAY_IMITATION_PLAN.md`), the "unselected Zed holdout" is
   contaminated for that brain. Exclude the Zed replay (and any replay on Zed) from imitation training, or move the holdout
   to a map in `test_ood` (Comet Catcher Remake, Eight Horses; neither has a captured map yet).
2. **Medium [V]: the catalog's historical roles come from one replay header (`02b3...`, Center Command)** and both
   playable maps come from the two replays. The training and "holdout" maps share a data origin and capture
   method. Fine for a map-transfer test; do not call it independent evidence of generalization to human play.
3. **Medium [V]: "validation" is the training set with more seeds.** `validation` references the same eight stage
   files as `training`, with 4 seeds instead of 2 (`replay-informed.json`), and jitter is 8 elmo. It cannot detect
   overfitting to those scenarios or those scripted opponents, only to seeds. Promotion on it will prefer the best
   fit to the training scenarios. Fix: add at least one validation-only scenario set (new rosters/separations/maps
   from the same families) and keep the Zed holdout for reporting.
4. **Medium [V]: the holdout is nearly the objectives stage.** `tactical-holdout.json` has the same six scenarios,
   rosters, separations and objectives as `tactical-objectives.json`; only the map and one opponent (`charge` instead of
   `kite`) differ. It measures map transfer for these rosters, nothing about new unit mixes. Say so in reports.
5. **Low [I]: the opponent set is scripted and fixed** (`charge, hold, kite, orbit, strike, guard`) plus the archive of past
   champions; the learner can specialise to exact script timing. The archive helps; check it is actually sampled
   in the objectives stage and not only in `easy`.

## 4. Unfair stage weighting

1. **Weights do not equalize stage influence on ES [I, high value to check].** `aggregate` weights raw stage fitness
   (`campaign_eval.rs:131-155`). ES follows score *differences* across perturbations, so the stage with the largest
   fitness variance dominates the update regardless of its 0.25/0.35/0.40 weight. Objectives and strategic combat stages
   include key-unit and exchange terms with a different spread than the exchange-only easy stage. Log per-stage
   across-population standard deviation for a few generations, then normalize (z-score or rank per stage per generation)
   before applying weights.
2. **Unequal effective sample sizes [V].** Per candidate and seed: easy has 3 scenarios by 2 opponents by 2 sides = 12 battles,
   mixed 3 by 3 by 2 = 18, objectives 6 by 3 by 2 = 36, holdout 6 by 3 by 2 = 36. The 0.40 weight on 36 battles has lower
   variance than 0.25 on 12, which is reasonable, but the cheapest stage is also the noisiest. Consider giving easy more seeds
   per candidate, since its cost is low.
3. **Island split is 80/20 by candidates only [V].** Local pop 32 (sigma 0.04) and wide pop 8 (sigma 0.16) do not
   share an ES estimate (good), and only the wide island restarts. Check that restart counters and the 30-generation
   patience are measured in *validated* gain, not in training fitness, or the wide island restarts on noise.
4. **`validation_every: 5`, `min_gain: 0.005`, `max_stage_regression: 0.15` [I].** With validation fitness on the same
   scenarios, a 0.005 gain is likely within seed noise at 4 seeds; check the standard error before trusting promotion.
5. **Strategic stage scales differ by map [V]:** see 2.1. The objectives stage mixes synthetic-2048 and Center Command, whose
   metal incomes differ by an order of magnitude, so Center Command contributes disproportionately (or not at all) to a
   raw economy score.

## 5. Deployment dependencies

1. **Platform lock [V].** `package_training.py` records `sys.platform` and `platform.machine()`, and the generated
   instructions use PowerShell syntax (`& './bin/...'`), so a Linux or macOS worker would not have runnable commands even
   if a compatible binary existed. Generate shell-specific instructions or a launcher script.
2. **Cross-machine determinism is not guaranteed [I].** The build has no `target-cpu=native` (good), but MLP forward
   passes use `tanh` and other transcendental calls whose last-bit results can differ between libm implementations
   (Windows vs Linux). Work units already carry `backend_identity`; make the coordinator *refuse to mix* workers whose
   executable hash or platform differs for one campaign, and add a smoke job that must reproduce the same fitness
   bit-for-bit on every worker before it is admitted.
3. **Map cache versioning [I].** The package ships `.vsgmap` files compiled by a given executable (`--compile-map`) and the
   executable it was passed with `--sim`. If these differ, the loader should reject, not silently reparse. Record the
   compile-time executable hash in the manifest and verify it at start.
4. **Unbundleable inputs [V in design].** The plan promises failure when an input cannot be bundled; verify with a negative
   test that deletes a stage's map and a baseline brain, since several referenced paths go through `../../../data/...`
   and the rewriting depends on resolving them from `examples/campaign/stages/`.
5. **Large raw data is not in the package, correctly, but the audit gaps are.** The catalog is the 22-unit bounded
   baseline. `data/brains/from_replays/catalog_audit/SUPPORT_AUDIT.md` shows converter export gaps (manual-fire, shield, interceptor,
   energy-converter); none touch these rosters, but a package built from a *different* catalog (for example
   `data/replay/catalog-simulable.json`) would silently inherit them. Make packaging refuse a catalog without a recorded
   audit hash for the units in use, or at minimum record the catalog hash in results.
6. **Resume across versions [I].** Resume rejects changed inputs by fingerprint, which is right. Also fingerprint the
   *executable* (not only configs), otherwise a rebuilt simulator with changed combat code resumes optimizer state against a
   different fitness function.

## Suggested changes, smallest first

1. Per-scenario opponent lists; remove `guard`/`kite` from defend scenarios (1.1).
2. Log per-stage fitness standard deviation, then z-score or rank-normalize stages before weighting (4.1).
3. Add a validation-only scenario set; keep Zed for reporting only (3.3, 3.4).
4. Add 5v3 and 3v5 rows to the tactical mixed stage (1.4).
5. Normalize strategic economy per map, or pick starts with comparable metal distance (2.1, 4.5).
6. Exclude the Zed replay from any imitation training before using it as a holdout (3.1).
7. Platform-specific launch instructions and executable hashes in work-unit identity and resume fingerprint (5.1 to 5.3, 5.6).

## Not checked

Smoke stage files, the exact per-stage fitness spread, `continuous_metal` use in construction, whether the
archive is sampled in every stage, and any behavior of the worker CLI beyond its documented command line.
