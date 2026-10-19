# Catalog support audit: catalog-simulable.json

Audited 354 units from `data/replay/catalog-simulable.json` against the MiniBAR runtime (BAR source `905b691212`). Machine-readable detail: `support_audit.json` (per-feature unit lists, evidence, per-unit flags). Regenerate with `python scripts/audit_catalog.py`.

Statuses are kept separate:

- **direct**: exported by the converter and consumed by the runtime.
- **approximate**: exported and consumed, but simplified.
- **unsupported**: no runtime consumer exists (declared-but-inert counts here).
- **export_gap**: the runtime implements it, but the converter never exports it, so this catalog loses it.
  Rows marked **WRONG** make the unit do something actively incorrect in the sim, not merely lack a mechanic.

The key finding is the fourth class. Most of the high-impact problems are not missing simulator features;
the converter (`scripts/make_catalog.py` and `prepare_from_replays.unit_entry`) drops fields the runtime already consumes.
Because the catalog also exports no `required_capabilities` for those mechanics, the runtime's capability gate
(`src/engine.rs:753-785`) cannot refuse them, so they degrade silently.

## Highest-impact items

Weight = replay player-games that owned an affected unit (a unit counted once per player-game).

| Rank | Mechanic | Status | Wrong? | Sev | Units | Player-games | Examples |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | Command-fire (manual) weapons: D-Gun, nukes, special launchers (`manual_fire_weapons`) | export_gap | **WRONG** | high | 14 | 1112 | armcom, corcom, armsilo, legcom, corsilo |
| 2 | Anti-nuke interceptors (coverage, intercept classes) (`interceptor_weapons`) | export_gap | **WRONG** | high | 6 | 89 | armamd, corfmd, armscab, legabm, cormabm |
| 3 | `bogus` helper weapons with damage (`bogus_weapons`) | export_gap | **WRONG** | medium | 2 | 13 | armvader, corkorg |
| 4 | Energy-to-metal converters (metal makers) (`energy_converters`) | export_gap |  | high | 6 | 384 | armmakr, cormakr, armmmkr, cormmkr, legeconv |
| 5 | Shield generators (repulsor) (`shield_weapons`) | export_gap |  | high | 4 | 20 | armgate, corgate, legdeflector, leggatet3 |
| 6 | Death explosions and wrecks/corpses (`death_explosions_wrecks`) | export_gap |  | medium | 353 | 19416 | armmex, armcom, armllt, armrad, armnanotc |
| 7 | Cloaking (cancloak, initcloaked, cloakcost) (`cloak`) | unsupported |  | medium | 31 | 1300 | armcom, corcom, armpb, armamex, armferret |
| 8 | damagemodifier (flat damage scaling) (`damage_modifier`) | unsupported |  | medium | 23 | 1188 | armsolar, corsolar, armadvsol, coradvsol, armpb |
| 9 | Wind generators (`wind_output`) | approximate |  | medium | 3 | 872 | armwin, corwin, legwin |
| 10 | Radar jammer radius (`radar_jammers`) | export_gap |  | medium | 18 | 206 | armaser, armjamt, armjam, armmlv, cormaw |
| 11 | Reactive armor (damage-triggered restore) (`reactive_armor`) | unsupported |  | medium | 3 | 0 | legamph, legkark, legshot |
| 12 | badtargetcategory (target de-prioritisation) (`bad_target_categories`) | approximate |  | low | 168 | 7996 | armcom, armllt, armflash, armpw, corcom |

## Detail by status

### export_gap (8)

**Command-fire (manual) weapons: D-Gun, nukes, special launchers** (`manual_fire_weapons`, high, 14 units, WRONG)

- Runtime: src/engine.rs:1139,1872 and src/world.rs:1368,3302,3649 exclude weapon.manual_fire from automatic targeting.
- Exporter/source: scripts/make_catalog.py:131 reads `manualfire`; BAR sources use `commandfire` (e.g. armcom DISINTEGRATOR), so manual_fire is always false.
- Effect now: Commanders auto-fire the 99999-damage D-Gun (range 250) at any target; nuke/special launchers auto-fire too.
- Suggested fix: Treat `commandfire` as manual_fire in make_catalog.weapon_entry.

**Anti-nuke interceptors (coverage, intercept classes)** (`interceptor_weapons`, high, 6 units, WRONG)

- Runtime: src/engine.rs:781,1140,1874 consume weapon.interceptor / coverage_range / targetable; CAPABILITY_INTERCEPTORS implemented.
- Exporter/source: make_catalog.weapon_entry/prepare_from_replays.unit_entry never write interceptor, coverage_range or targetable.
- Effect now: armamd/corfmd-style launchers export as ordinary stockpiled StarburstLaunchers (damage 1500, range 72000 elmos) with no interceptor fields; by the engine.rs gates they are ordinary ground-targeting weapons, i.e. probable map-wide artillery (not observed in a run).
- Suggested fix: Export interceptor bitmask + coverage_range and targetable on nuclear weapons.

**`bogus` helper weapons with damage** (`bogus_weapons`, medium, 2 units, WRONG)

- Runtime: all exported weapons are treated as real.
- Exporter/source: customparams.bogus not filtered.
- Effect now: Display/targeting helper weapons would deal real damage.
- Suggested fix: Drop weapons with customparams.bogus.

**Energy-to-metal converters (metal makers)** (`energy_converters`, high, 6 units)

- Runtime: src/world.rs:3100-3102 only pays metal_make/energy_make and (negative) upkeep; there is no conversion gadget model.
- Exporter/source: customparams.energyconv_capacity/efficiency (applied by BAR luarules gadgets) are ignored; exported metal_make, energy_upkeep and metal_upkeep are all 0.
- Effect now: armmmkr (24,848 human build orders) and other makers cost resources but produce no metal and use no energy.
- Suggested fix: Approximate as metal_make = capacity*efficiency with energy_upkeep = capacity when energy is available, or add a converter rule in world economy.

**Shield generators (repulsor)** (`shield_weapons`, high, 4 units)

- Runtime: src/engine.rs:778,1138,1147 implement shield charge/regen/absorb; src/model.rs:471-489 define is_shield, shield_radius, shield_power, regen, delay, exterior, smart.
- Exporter/source: the BAR `shield = {power, radius, powerregen, ...}` table is never read; the weapon exports as damage 0 range 0 with no is_shield.
- Effect now: armgate/corgate/leg equivalents exist but protect nothing (inert 0-range weapon).
- Suggested fix: Map weapondef.shield.* to is_shield/shield_* fields and set intercepted_by_shield_type.

**Death explosions and wrecks/corpses** (`death_explosions_wrecks`, medium, 353 units)

- Runtime: src/catalog.rs:788-815 and src/world.rs support death_explosion_*, corpse, reclaimable, corpse_metal.
- Exporter/source: none of death_explosion_*, corpse*, reclaim_* are exported (explodeas names a weapon in gamedata; corpse metal is in the unit's featuredefs).
- Effect now: Units die silently and leave no reclaimable metal; chain explosions (e.g. fusion, nukes) and wreck economy are absent.
- Suggested fix: Resolve explodeas against gamedata and featuredefs.metal for corpse_metal.

**Radar jammer radius** (`radar_jammers`, medium, 18 units)

- Runtime: src/world.rs:1577-1606 jam radar contacts using strategic.jammer_radius.
- Exporter/source: radardistancejam is never written to jammer_radius.
- Effect now: Jammer units do not jam.
- Suggested fix: Export jammer_radius = radardistancejam.

**Slaved weapon mounts (shared fire)** (`slaved_weapons`, low, 7 units)

- Runtime: WeaponDef has no slave link; each mount fires independently.
- Exporter/source: scripts/make_catalog.py:161 merges slaved mounts into projectiles, but prepare_from_replays.unit_entry (used for these catalogs) loops all mounts and does not.
- Effect now: Slaved barrels (e.g. armjanus: two JANUS_ROCKET mounts) export as two independent weapons: total salvo damage is similar, but they aim and retarget separately and per-shot energy/metal/stockpile costs are doubled.
- Suggested fix: Reuse make_catalog's slaveto merge in unit_entry.

### unsupported (14)

**Cloaking (cancloak, initcloaked, cloakcost)** (`cloak`, medium, 31 units)

- Runtime: can_cloak exists only in src/catalog.rs/src/model.rs; no consumer in engine.rs/world.rs even though CAPABILITY_CLOAK is listed in IMPLEMENTED_CAPABILITIES.
- Exporter/source: can_cloak never exported.
- Effect now: Cloakable units are permanently visible; the declared capability claims support that does not exist.
- Suggested fix: Either implement cloak visibility masks or remove CAPABILITY_CLOAK from IMPLEMENTED_CAPABILITIES and export required_capabilities so scenarios fail loudly.

**damagemodifier (flat damage scaling)** (`damage_modifier`, medium, 23 units)

- Runtime: no consumer.
- Exporter/source: not exported.
- Effect now: Damage taken is unscaled (armsolar/corsolar take 100% instead of damagemodifier 0.5).
- Suggested fix: Export a damage_taken multiplier.

**Reactive armor (damage-triggered restore)** (`reactive_armor`, medium, 3 units)

- Runtime: no gadget model.
- Exporter/source: gadget behavior in luarules/gadgets/unit_reactive_armor.lua is not exported.
- Effect now: Units are plainly squishier than in BAR.
- Suggested fix: Model as extra damage buffer or exclude from curricula.

**Juno / script-only special weapons** (`juno_special`, low, 30 units)

- Runtime: no model.
- Exporter/source: not exported.
- Effect now: Juno radar/jammer pulse missing.
- Suggested fix: Accept.

**Crushing and crush resistance** (`crush_interaction`, low, 12 units)

- Runtime: only a crushable:false constant (src/world.rs:4426,4457).
- Exporter/source: not exported.
- Effect now: Tracked units do not run over small units.
- Suggested fix: Low priority.

**Sonar radius** (`sonar`, low, 23 units)

- Runtime: sonar_radius is stored (src/model.rs) but no world/engine consumer exists though CAPABILITY_SONAR is declared.
- Exporter/source: not exported.
- Effect now: Moot on land maps (mostly water-bound units).
- Suggested fix: Low priority.

**Water/torpedo weapons on land-capable units** (`water_weapons`, low, 21 units)

- Runtime: no waterweapon/firesubmersed logic in src/engine.rs.
- Exporter/source: not exported.
- Effect now: Torpedo weapons are fired like cannons or never; irrelevant on dry maps.
- Suggested fix: Gate by water depth.

**Autoheal / idle autoheal** (`auto_heal`, low, 10 units)

- Runtime: no consumer.
- Exporter/source: not exported.
- Effect now: No passive healing.
- Suggested fix: Add regen fields.

**Capture, restore and terraform orders** (`capture_terraform`, low, 6 units)

- Runtime: no capture/restore orders in src/strategy.rs StrategyAction.
- Exporter/source: not exported.
- Effect now: Capture/restore commands cannot be issued.
- Suggested fix: Low priority.

**Commander respawn pad (spawnpad_unit) and weapon-spawned drones** (`respawn_and_carriers`, low, 3 units)

- Runtime: no spawn model.
- Exporter/source: not exported (gadget: luarules/gadgets/game_commander_builder.lua).
- Effect now: Commanders have no respawn pad; no carried drones.
- Suggested fix: Accept; commander respawn is a game option.

**Radar/sonar stealth flags** (`radar_stealth`, low, 33 units)

- Runtime: no `stealth` consumer in src/.
- Exporter/source: not exported.
- Effect now: Stealth units show on radar.
- Suggested fix: Add a stealth flag to observation masks.

**Instant self-destruct and mines** (`self_destruct_mines`, low, 15 units)

- Runtime: no self-destruct or proximity-detonation model; gadgets: unit_instant_self_destruct.lua, unit_minesweeper_detection.lua.
- Exporter/source: not exported.
- Effect now: Crawling bombs and mines do not detonate.
- Suggested fix: Add self-destruct order and proximity trigger.

**Bouncing projectiles** (`bounce_projectiles`, low, 4 units)

- Runtime: src/score.rs:495 states bounce is omitted.
- Exporter/source: not exported.
- Effect now: Bounce cannon rounds detonate on first impact.
- Suggested fix: Accept.

**Decoy units (decoyfor)** (`decoys`, low, 7 units)

- Runtime: no decoy model.
- Exporter/source: not exported.
- Effect now: Decoy commanders have distinct stats but no decoy semantic.
- Suggested fix: Accept.

### approximate (6)

**Wind generators** (`wind_output`, medium, 3 units)

- Runtime: src/world.rs:3104 wind_generator * config.wind_output (a global scalar).
- Exporter/source: wind_generator exported; BAR's windgenerator is rescaled from map wind in luarules, map wind is not captured (prepare_from_replays WARNINGS).
- Effect now: Wind income is a scenario constant, not the map's wind; armwin is the single most-built structure (148k orders).
- Suggested fix: Capture map wind min/max in the map source.

**badtargetcategory (target de-prioritisation)** (`bad_target_categories`, low, 168 units)

- Runtime: only only_target_categories is consumed (src/engine.rs:2794); no target-priority penalty.
- Exporter/source: badtargetcategory is not exported; Recoil only de-prioritises such targets (RecoilEngine rts/Sim/Weapons/Weapon.cpp:709,745), it does not forbid them.
- Effect now: Weapons pick e.g. VTOL or ALL-category targets with normal priority instead of last.
- Suggested fix: Optional: add a bad-target priority penalty in target scoring.

**Weapon re-aim delays and aim scripts** (`turret_reaim`, low, 79 units)

- Runtime: turret yaw/pitch speeds supported (src/engine.rs); no re-aim latency.
- Exporter/source: turret speeds from customparams or unit script; reaimtime ignored; scripts with shared CATT include get instant yaw (make_catalog docstring).
- Effect now: Aim response is slightly optimistic.
- Suggested fix: Accept.

**3D sensor details (air LOS, emit heights)** (`sight_air_sensors`, low, 54 units)

- Runtime: src/score.rs:499 declares sensors are 2D horizontal masks.
- Exporter/source: sight_distance and radar_radius exported.
- Effect now: Emit height and air-specific LOS ignored.
- Suggested fix: Accept.

**Missile acceleration, starburst climb, overrange/descent** (`missile_flight`, low, 63 units)

- Runtime: src/score.rs:495 declares 'weapon acceleration, bounce... omitted'; guided rounds use tracks/projectile_turn_rate (src/engine.rs:775,2085).
- Exporter/source: time-averaged missile speed (make_catalog.missile_speed); overrange_distance and descent ignored.
- Effect now: Missile timing and long-range falloff are approximate.
- Suggested fix: Accept as declared approximation.

**Stockpiled weapons (stockpilelimit)** (`stockpile`, low, 16 units)

- Runtime: src/engine.rs:1446,1487; src/model.rs WeaponState keeps ONE stockpiled round queued.
- Exporter/source: stockpile + stockpile_time exported; customparams.stockpilelimit ignored.
- Effect now: Magazine depth is 1 regardless of stockpilelimit.
- Suggested fix: Add a stockpile_limit field.

### Directly represented

- Per-armour-class damage tables (damage_by_armour) (src/engine.rs:3203,3325)
- BeamLaser/LaserCannon/Melee hitscan with damage_duration (src/engine.rs:2558)
- Area of effect and edge effectiveness (src/engine.rs)
- EMP/paralyzer damage with stun time and paralyze_multiplier (src/engine.rs:28,1079)
- Metal/energy income, storage, solar (negative upkeep), geothermal, tidal, mex (src/world.rs:3090-3110)
- Builders, factories, build options, assist, repair, reclaim, yardmaps, footprints, slope/water limits (src/construction.rs, src/placement.rs)
- Line of sight and radar radius with 2D masks (src/world.rs:1568-1631)
- onoffable/activate_when_built (src/world.rs:4053)

## Impact on the current from_replays training roster

- **armada**: armck: death_explosions_wrecks; armham: death_explosions_wrecks, bad_target_categories, turret_reaim; armlab: death_explosions_wrecks; armmex: death_explosions_wrecks; armpw: death_explosions_wrecks, bad_target_categories, crush_interaction, turret_reaim; armrock: death_explosions_wrecks, missile_flight, bad_target_categories, turret_reaim; armsolar: death_explosions_wrecks, damage_modifier
- **cortex**: corak: death_explosions_wrecks, bad_target_categories, crush_interaction, turret_reaim; corck: death_explosions_wrecks; corlab: death_explosions_wrecks; cormex: death_explosions_wrecks; corsolar: death_explosions_wrecks, damage_modifier; corstorm: death_explosions_wrecks, missile_flight, bad_target_categories, turret_reaim; corthud: death_explosions_wrecks, bad_target_categories, turret_reaim
- **legion**: legbal: death_explosions_wrecks, missile_flight, bad_target_categories, turret_reaim; legcen: death_explosions_wrecks, bad_target_categories, turret_reaim; legck: death_explosions_wrecks; leggob: death_explosions_wrecks, bad_target_categories, crush_interaction, turret_reaim; leglab: death_explosions_wrecks; leglob: death_explosions_wrecks, bad_target_categories, turret_reaim; legmex: death_explosions_wrecks; legsolar: death_explosions_wrecks, damage_modifier

The bounded T1 roster is untouched by every WRONG item above: no commander, interceptor, shield, converter or slaved-weapon unit is in it.
It is touched by low-severity approximations (wrecks/explosions, aim latency, missile flight, `badtargetcategory`) and by
`damage_modifier` on solar panels (BAR halves damage to them; the sim does not). Treat those as known approximations.
Any curriculum that adds commanders, anti-nukes, shields, metal makers, or the Jaguar/Marauder/Banisher set needs the fixes first.

## What was verified, and what was not

- Verified by reading code: every runtime line anchor above, the converter lines, and that `armcom` exports its D-Gun with
  `manual_fire: false` (source key is `commandfire`), `armamd` as a 72,000-range stockpiled launcher with no interceptor fields,
  `armgate` as a damage-0 range-0 weapon, and `armmmkr` with metal_make 0 and no upkeep.
- Not verified by simulation: that these units then misbehave in a running duel. The behavior column follows from the gates in
  `src/engine.rs` (manual_fire / interceptor / is_shield exclusions), not from an observed run.
- Jaguar (`armlatnk`) is excluded from the simulable catalog (`gadget weapon`: lightning chaining) but present in `catalog-full.json`.
- Per-unit counts use pattern detectors over BAR source; a mechanic expressed only in a unit script (`.bos`) is not seen.
