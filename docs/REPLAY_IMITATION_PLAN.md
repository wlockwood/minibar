# Replay imitation plan (started 2026-10-16)

Goal: train MiniBAR brains to approximate what human players did in BAR replays.
Status of each item is tracked here so work survives context compaction.
Resume by reading this file and the memory note `minibar-replay-imitation-data`.

## Data (already exists, in the sibling visigoth repo)

- 527 duels: `D:\code\BAR\visigoth\data\replays\*.sdfz` + `.json` sidecar.
- Pipeline guide: `D:\code\BAR\visigoth\docs\REPLAY_PIPELINE_GUIDE.md`.
- Trajectories: `visigoth\data\trajectories\per-replay\*.parquet` (one shard per replay) and the split
  dataset `visigoth\data\trajectories\datasets\trajectories-v2\{train,validation,test_ood}.parquet`.
  Row = player x 10 s sample; `sample_json` = `observation` (fog-limited own units, enemies, resources)
  + `commands` issued in the interval.
- Verified 2026-10-16: ALL 840,890 build commands in the 527 shards already carry `build_def`
  (an earlier note claiming "legacy shards without names" was wrong; no re-extraction needed).
  One record per issuing builder; a factory queue entry is a build command with `params == [0]`;
  placed structures have `[x, y, z, facing]`; `options.shift` = queued. A multi-unit order is
  replicated per unit, so dedupe on (frame, build_def, params).

## Work items (do in order; tick when done)

- [x] 1. ~~Re-extract all 527 replays~~ NOT NEEDED: shards already have `build_def` (verified).
      If re-extraction is ever needed, output to a NEW directory, never overwrite the shards.
- [x] 2. ~~Rebuild splits~~ NOT NEEDED: trajectories-v2 splits (train/validation/test_ood) are valid.
- [x] 3. **Build-order extraction** DONE 2026-10-16: `python scripts/replay_buildorders.py --out data/replay/buildorders-v1.jsonl`
      (41 s). 1,054 player-games (train 774 / validation 208 / test_ood 72), 472,744 build events after
      dedupe (840,890 raw). Per 10 s sample: eco (metal/energy level, storage, income, expense), unit counts
      (finished + in progress), and the deduped build decisions (place/factory, item, pos, facing, queued,
      builders). Openings look right (mex + wind, vehicle plant at ~42 s, then factory queue).
- [x] 4. **Catalog** DONE 2026-10-16: `python scripts/make_full_catalog.py` (34 s) writes to `data/replay/`:
      `catalog-full.json` (583 units: Armada 193, Cortex 188, Legion 202), `catalog-simulable.json`
      (354; drops flight/transport/resurrect/water-bound/gadget-weapon units the sim cannot do yet),
      `catalog-manifest.json` (usage counts, exclusions, unsupported reasons), `unit_usage.json` (cache).
      Universe = every unit seen in observations or build orders (492) + closure over build options (103)
      minus excluded (scavenger, raptor, hats/shoulder cosmetics, units/other joke+debug). 0 conversion
      errors; every build option resolves. Coverage of the 472,744 human build events: 100% full,
      95.4% simulable (Legion 78.1% simulable, few Legion games). Both load in the real simulator.
      Converter is `prepare_from_replays.unit_entry` (other worker's); do not duplicate it.
- [ ] 5. **Build-order imitation model**: small labelled perceptron(s) predicting the next build
      item from eco state / time / what exists. Held-out evaluation vs a most-common-action baseline.
- [ ] 6. **Converter for tactical windows** (20-60 s of one player's play -> MiniBAR scenario plus the
      human's move/fight/attack orders as a target trace). Skip windows with units outside the catalog.
- [ ] 7. **Score existing brains** (champion, template) against held-out windows before training.
- [ ] 8. **Train tactical brain** with the existing ES loop; fitness = similarity to the human trace.
      Then use as `--seed-brain` for the win-rate objective.
- [ ] 9. **Strategic wiring**: MiniBAR `Strategy` that issues build orders from item 5 so the economy
      produces units for item 8 to command.

## Why build orders first

Without a believable economy there are no units to command. Command mix in the corpus is ~70% Move,
10% Fight, 8% Repair, 10% build; build orders are the only part that needs this re-extraction.

## Notes / findings

- Alphabetical order of local `Beyond-All-Reason/units` names does NOT reproduce engine defIDs.
- Game build for the corpus: `Beyond All Reason test-31251-0ceadc7`, engine 2026.07.04.
- Catalog coverage (2026-10-16): only 27,434 of 472,744 human build events (5.8%) involve an item in
  `examples/t1-catalog-extended.json` (26 combat units, no economy). Top human items are armwin, armmex,
  corwin, cormex, armnanotc, cornanotc, armmoho, armllt, armsolar. Item 4/9 therefore need the catalog
  extended with economy structures, labs/vehicle plants, constructors and nanos. `scripts/make_catalog.py`
  only searches Bots/Vehicles/Hovercraft folders (UNIT_FOLDERS); extend it to buildings. Existing
  `examples/complete-opening.json` already uses armck/armlab/armmex/armsolar, so MiniBAR's economy
  layer exists; check how its catalog entries were made before duplicating.
- `data/replay/` (100 MB jsonl) is untracked and not git-ignored.
- Fixed `scripts/bar_lua.py`: it could not read `[[long strings]]` (yardmaps) or `1 / 70`, which broke
  legvp, legavp and legeconv (Legion labs). Regression test: `scripts/test_bar_lua.py`.
- Another worker is building `data/brains/from_replays` + `src/learn/strategic.rs` (plan:
  `docs/superpowers/plans/2026-10-16-from-replays.md`): authored (not imitation) strategic_v1 brain over a
  22-unit baseline catalog. Coordinate before touching `scripts/prepare_from_replays.py`, `src/learn/strategic.rs`.
  Item 5/9 should plug into that controller's economy rather than build a second one.
- Catalog support audit (2026-10-16, for Codex handoff): `python scripts/audit_catalog.py` ->
  `data/brains/from_replays/catalog_audit/{support_audit.json,SUPPORT_AUDIT.md,CURRICULUM_SENSOR_GAPS.md}`.
  Item 5/9 caveat: the converter drops fields the runtime consumes (commandfire, interceptor, shield, energyconv,
  jammer, wrecks), so metal makers produce nothing and commander D-Guns auto-fire until `make_catalog.py` /
  `prepare_from_replays.unit_entry` are fixed. Regenerate catalogs and re-audit after that.
