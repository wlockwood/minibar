# Next-curriculum sensor and decision gaps

Follow-up to `docs/MLP_SENSOR_RESEARCH.md`, read against `src/learn/tactical.rs`,
`src/learn/strategic.rs` and `src/engine.rs` as they stand today. Design note only; nothing was
implemented. Scope: Jaguar/Marauder/Banisher mixed roles, emergency AA response, protecting economy.
Facts below are from code or the catalog; **inferences are marked**.

## What exists today (so the gaps are specific)

- **tactical_v4** (`tactical.rs:285-320`): seven modules. `engagement` already takes 22 inputs of the 32 allowed;
  `target_selector` 10; `mission` 14. Damage balance is conditioned on the chosen target through
  `eligible_damage_rate`/`eligible_weapon` (`tactical.rs:371-418`), which honour `only_target_categories`.
  That is the right primitive and is why a Marauder is not charged AA DPS against a tank.
- **Candidates** are the 3 nearest enemies plus the nearest eligible key asset (`tactical.rs:62,1055-1080`).
- **Execution**: `update_weapons` (`engine.rs:1836`) fires only at `commands[index].target` and skips a unit that has
  none (`engine.rs:1858-1860`); the single target is then tried against every mount that can legally hit it
  (loop at `engine.rs:1871`). So there is no automatic target acquisition at all: every shot is policy-commanded,
  and there is no second target. Commands come from `set_command` (`engine.rs:1092-1111`).
- **strategic_v1** (`strategic.rs:24-71`): `investment` 20 inputs, `production` 12, `mission` 10. Resource
  pressure is present (`metal_unmet`, `energy_overflow`, ...). Threat appears only as the scalars `known_threat`
  and `observed_threat`.
- **Catalog facts** for the three target units: Marauder (`armmar`) = SURFACE gauss 350 range, reload 0.7 +
  VTOL missile 600 range, reload 2.1, AA damage stored in `damage_by_armour.vtol = 90` with default 0.
  Banisher (`corban`) = one mount, 1000 damage, reload 7.5, targets `NOTSUB`. Jaguar (`armlatnk`) is in
  `catalog-full.json` but **excluded from `catalog-simulable.json`** ("gadget weapon": lightning chaining) and
  exports its lightning as a plain 22-damage hitscan.

## Mixed roles: Jaguar / Marauder / Banisher

1. **Blocking, engine-side: no independent AA target.** Because one target feeds all mounts, a Marauder that is
   shooting a tank can never engage an aircraft with its missile at the same time, and an idle unit never fires
   at all. No sensor fixes this. Either add a second command slot (`aa_target`), or let non-primary mounts
   auto-acquire by category (which also makes AA defense work for units the policy is not steering). Without it,
   "hybrid" units are only evaluated for one domain at a time and the research's own validation case 1 cannot run.
2. **Per-mount readiness (C02).** `reload_remaining` comes from the single selected mount
   (`tactical.rs:1157`). Add `ground_mount_ready`, `air_mount_ready` (0..1 each) to `spacing`/`strafe`.
   Cheap: values already exist in `UnitState.weapons[i].reload_until`.
3. **Target priority (`badtargetcategory`) is not in the catalog.** Banisher's source discourages VTOL targets
   (`MLP_SENSOR_RESEARCH.md` Banisher row); the exporter drops that field (see `SUPPORT_AUDIT.md`,
   `bad_target_categories`), so with one 7.5 s, 1000-damage shot the sim cannot tell "may shoot aircraft" from
   "should prefer the tank". Export it and add `target_is_discouraged` to `target_selector`. *Inference:* this is
   the main reason Banisher salvo-wasting will look random to a learner.
4. **Shot saturation / overkill (C10).** A 1000-damage shot at a 300-HP target is waste, and nothing reserves
   damage across the platoon. Add `target_hp_over_my_shot` and `target_already_assigned_damage_fraction`
   (needs a platoon-level reservation table; pure Rust bookkeeping, no new learning).
5. **Domain-split candidate slots.** Replace "3 nearest" with: nearest air threat, nearest ground threat,
   nearest key asset, plus one nearest for the rest. This is the smallest change that lets AA ever be chosen
   (`TARGET_CANDIDATES`, `tactical.rs:62`).
6. **Jaguar specifically:** chaining lightning means damage depends on enemy density (research table).
   Add `enemy_cluster_density` (enemies within chain radius of the target) to `target_selector`, and decide
   whether the sim should carry a chain model or Jaguar stays out of curricula. Today it is silently a
   single-target 22-damage gun.
7. **Input budget.** `engagement` is at 22/32. Add domain splits by *replacing* the three range-band DPS pairs
   (`dps_short/mid/long`, 6 inputs) with `dps_vs_air`/`dps_vs_ground` pairs, rather than appending.

## Emergency AA response

1. **Prerequisite, runtime: there is no aircraft to respond to.** The simulable catalog drops all flying units
   (56 `flight` reasons in the manifest) and the research notes there is no altitude model. A learner cannot be
   trained on AA response until a minimal air attacker exists (e.g. a VTOL-category unit that flies a straight
   line over terrain at constant height and strafes or bombs). Codex's call; I would do this before any AA sensor work.
2. **Observation honesty.** Tactical policies read the raw `WorldView` (the research flags this): an AA module would
   see air units that, in the replays, are only radar blips until LOS. Before training the AA reaction, add the perspective
   adapter, or the policy will learn from information a human never had.
3. **Sensors** (all from the research registry; this is the minimal subset): `air_threat_present`,
   `air_threat_eta` (C05/C07), `own_ready_aa_dps_local` and `aa_cover_reachable_eta` (C12), `unanswered_by_air`
   (C07: incoming air damage vs ready AA). Scope them to the *platoon*, not the unit (T02).
4. **Strategic side.** `production` has `observed_threat` (one scalar) and `candidate_combat`, so it cannot learn
   "build AA when aircraft appear". Add `air_threat_observed`, `aa_gap` (S21/S15), `candidate_can_hit_air`
   (derive from the candidate's weapons) and `aa_delivery_eta`. `candidate_can_hit_air` is deterministic legality
   data, not something to learn.
5. **Timescale.** Research asks for fast hazard checks; replay sampling is 10 s, so response latency cannot be
   validated from the replays at all, only from simulated scenarios.

## Protecting economy

1. **Key asset ≠ economy.** `mission` protects one key unit (`own_key_*`). Mexes, solar farms and factories are not
   key assets, so nothing in tactics or strategy defends income. Add an asset-at-risk list (S11): per threatened
   economic asset, `income_share_at_risk`, `threat_arrival_eta` (S12) and `intercept_slack` (T03 = threat ETA minus
   my best responder ETA). These feed the existing `mission` module as a second asset class instead of a new brain.
2. **Do not train a short leash.** The research already records that leashing defenders to the asset pulled
   splash onto it and lost fights; use intercept slack, as above.
3. **Expansion security (S09)** is currently the single scalar `known_threat` inside `investment`. Needed: route
   exposure and builder ETA for the candidate site, so "expand into a raid" is learnable as bad.
4. **Missing strategic telemetry** (research S03/S04/S06/S08/S10): `depletion_horizon`, `stall_persistence`,
   `production_utilization` (idle vs starved vs blocked factories), `payback_horizon` for an expansion (yield and
   cost exist but are not combined), and `site_commitment` (committed builders and progress). These are cheap,
   deterministic, and three of them explain most of the replay economy mistakes.
5. **Economy realism caveats that bias the training signal** (details in `SUPPORT_AUDIT.md`): wind output is a global
   scalar, metal makers produce nothing, solar takes full damage (BAR's `damagemodifier` is 0.5), and wrecks leave no
   reclaim. A strategic brain trained on this will undervalue energy conversion and reclaim. Fix or disclose before
   comparing to human economies.

## Suggested order

1. Exporter fixes from the audit that change unit behavior: `commandfire`, `badtargetcategory`, slaved mounts
   (small converter edits; no runtime change).
2. Engine: independent AA target or secondary-mount auto-acquire; domain-split candidate slots.
3. Minimal air attacker; perspective adapter for tactical observation.
4. Sensors: per-mount readiness, shot saturation, air threat set, then the economy asset-at-risk set.
5. Strategic: AA production gap, expansion security, the cheap telemetry in item 4 above.
6. Validate in the research's cases (hybrid weapons, unanswered threats, asset cases) with sensor-family ablations.

## Not checked

- I did not run any simulation; engine behavior is read from code (the single-target firing rule is explicit in
  `update_weapons`, so I am confident of it, but a two-target test should still precede the engine change).
- Input counts are from the `*_INPUTS` constants; `tactical_v4` appends four, three and fourteen inputs to
  `target_selector`, `spacing` and `mission` (`tactical.rs:296-316`).
