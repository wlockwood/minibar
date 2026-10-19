#!/usr/bin/env python3
"""Support audit of data/replay/catalog-simulable.json against the MiniBAR runtime.

Every unit in the catalog is re-read from BAR source and tested against a registry of
mechanics. Each mechanic is classified, kept separate:

  direct       exported by the catalog converter AND consumed by the runtime
  approximate  exported and consumed, but simplified (declared or observed)
  unsupported  no runtime consumer exists (declared-but-inert counts here)
  export_gap   the runtime implements it but the converter does not export it, so the
               current catalog loses it. `wrong_behavior` marks gaps where the unit does
               something ACTIVELY WRONG in the sim (not just missing).

Impact = sum over affected units of replay player-games (units players actually owned)
and of human build orders, from catalog-manifest.json usage.

    python scripts/audit_catalog.py          # writes data/brains/from_replays/catalog_audit/
Read-only on catalogs and runtime; writes only the audit outputs.
"""
import argparse
import collections
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from bar_lua import parse_unit_file  # noqa: E402

BAR = r"D:\code\BAR\Beyond-All-Reason"
ROOT = os.path.dirname(HERE)


def weapons_of(raw):
    """Yield (mount_index, mount, weapondef) for every mounted weapon."""
    defs = {k.lower(): v for k, v in (raw.get("weapondefs") or {}).items()}
    mounts = raw.get("weapons") or {}
    for key in sorted(mounts, key=lambda k: float(k)):
        m = mounts[key]
        wd = defs.get(str(m.get("def", "")).lower())
        if wd is not None:
            yield key, m, wd


def cp(d):
    return {k.lower(): v for k, v in (d.get("customparams") or {}).items()}


def truthy(v):
    return v is True or (isinstance(v, (int, float)) and v == 1) or str(v).lower() in ("1", "true")


def wfind(pred):
    def detect(name, raw):
        hits = [wd.get("name", name) for _, m, wd in weapons_of(raw) if pred(m, wd)]
        return hits
    return detect


def ufind(pred):
    def detect(name, raw):
        return ["unit"] if pred(raw) else []
    return detect


def slaved(name, raw):
    return [str(m.get("def")) for _, m, wd in weapons_of(raw) if m.get("slaveto") is not None]


def bogus_damage(name, raw):
    return [wd.get("name", name) for _, m, wd in weapons_of(raw)
            if truthy(cp(wd).get("bogus")) and float((wd.get("damage") or {}).get("default", 0)) > 0]


# id, title, scope, status, severity, wrong_behavior, detector, runtime, exporter, behavior, fix
REGISTRY = [
    dict(id="manual_fire_weapons", title="Command-fire (manual) weapons: D-Gun, nukes, special launchers",
         status="export_gap", severity="high", wrong=True,
         detect=wfind(lambda m, wd: truthy(wd.get("commandfire")) or truthy(wd.get("manualfire"))),
         runtime="src/engine.rs:1139,1872 and src/world.rs:1368,3302,3649 exclude weapon.manual_fire from automatic targeting",
         exporter="scripts/make_catalog.py:131 reads `manualfire`; BAR sources use `commandfire` (e.g. armcom DISINTEGRATOR), so manual_fire is always false",
         behavior="Commanders auto-fire the 99999-damage D-Gun (range 250) at any target; nuke/special launchers auto-fire too",
         fix="Treat `commandfire` as manual_fire in make_catalog.weapon_entry"),
    dict(id="interceptor_weapons", title="Anti-nuke interceptors (coverage, intercept classes)",
         status="export_gap", severity="high", wrong=True,
         detect=wfind(lambda m, wd: wd.get("interceptor") is not None),
         runtime="src/engine.rs:781,1140,1874 consume weapon.interceptor / coverage_range / targetable; CAPABILITY_INTERCEPTORS implemented",
         exporter="make_catalog.weapon_entry/prepare_from_replays.unit_entry never write interceptor, coverage_range or targetable",
         behavior="armamd/corfmd-style launchers export as ordinary stockpiled StarburstLaunchers (damage 1500, range 72000 elmos) with no interceptor fields; by the engine.rs gates they are ordinary ground-targeting weapons, i.e. probable map-wide artillery (not observed in a run)",
         fix="Export interceptor bitmask + coverage_range and targetable on nuclear weapons"),
    dict(id="shield_weapons", title="Shield generators (repulsor)",
         status="export_gap", severity="high", wrong=False,
         detect=wfind(lambda m, wd: str(wd.get("weapontype", "")).lower() == "shield"),
         runtime="src/engine.rs:778,1138,1147 implement shield charge/regen/absorb; src/model.rs:471-489 define is_shield, shield_radius, shield_power, regen, delay, exterior, smart",
         exporter="the BAR `shield = {power, radius, powerregen, ...}` table is never read; the weapon exports as damage 0 range 0 with no is_shield",
         behavior="armgate/corgate/leg equivalents exist but protect nothing (inert 0-range weapon)",
         fix="Map weapondef.shield.* to is_shield/shield_* fields and set intercepted_by_shield_type"),
    dict(id="energy_converters", title="Energy-to-metal converters (metal makers)",
         status="export_gap", severity="high", wrong=False,
         detect=ufind(lambda r: "energyconv_capacity" in cp(r)),
         runtime="src/world.rs:3100-3102 only pays metal_make/energy_make and (negative) upkeep; there is no conversion gadget model",
         exporter="customparams.energyconv_capacity/efficiency (applied by BAR luarules gadgets) are ignored; exported metal_make, energy_upkeep and metal_upkeep are all 0",
         behavior="armmmkr (24,848 human build orders) and other makers cost resources but produce no metal and use no energy",
         fix="Approximate as metal_make = capacity*efficiency with energy_upkeep = capacity when energy is available, or add a converter rule in world economy"),
    dict(id="cloak", title="Cloaking (cancloak, initcloaked, cloakcost)",
         status="unsupported", severity="medium", wrong=False,
         detect=ufind(lambda r: truthy(r.get("cancloak")) or truthy(r.get("initcloaked")) or r.get("cloakcost") is not None),
         runtime="can_cloak exists only in src/catalog.rs/src/model.rs; no consumer in engine.rs/world.rs even though CAPABILITY_CLOAK is listed in IMPLEMENTED_CAPABILITIES",
         exporter="can_cloak never exported",
         behavior="Cloakable units are permanently visible; the declared capability claims support that does not exist",
         fix="Either implement cloak visibility masks or remove CAPABILITY_CLOAK from IMPLEMENTED_CAPABILITIES and export required_capabilities so scenarios fail loudly"),
    dict(id="radar_stealth", title="Radar/sonar stealth flags",
         status="unsupported", severity="low", wrong=False,
         detect=ufind(lambda r: truthy(r.get("stealth")) or truthy(r.get("sonarstealth"))),
         runtime="no `stealth` consumer in src/", exporter="not exported",
         behavior="Stealth units show on radar", fix="Add a stealth flag to observation masks"),
    dict(id="radar_jammers", title="Radar jammer radius",
         status="export_gap", severity="medium", wrong=False,
         detect=ufind(lambda r: float(r.get("radardistancejam", 0) or 0) > 0),
         runtime="src/world.rs:1577-1606 jam radar contacts using strategic.jammer_radius",
         exporter="radardistancejam is never written to jammer_radius",
         behavior="Jammer units do not jam", fix="Export jammer_radius = radardistancejam"),
    dict(id="sonar", title="Sonar radius",
         status="unsupported", severity="low", wrong=False,
         detect=ufind(lambda r: float(r.get("sonardistance", 0) or 0) > 0),
         runtime="sonar_radius is stored (src/model.rs) but no world/engine consumer exists though CAPABILITY_SONAR is declared",
         exporter="not exported", behavior="Moot on land maps (mostly water-bound units)",
         fix="Low priority"),
    dict(id="death_explosions_wrecks", title="Death explosions and wrecks/corpses",
         status="export_gap", severity="medium", wrong=False,
         detect=ufind(lambda r: bool(r.get("explodeas") or r.get("corpse"))),
         runtime="src/catalog.rs:788-815 and src/world.rs support death_explosion_*, corpse, reclaimable, corpse_metal",
         exporter="none of death_explosion_*, corpse*, reclaim_* are exported (explodeas names a weapon in gamedata; corpse metal is in the unit's featuredefs)",
         behavior="Units die silently and leave no reclaimable metal; chain explosions (e.g. fusion, nukes) and wreck economy are absent",
         fix="Resolve explodeas against gamedata and featuredefs.metal for corpse_metal"),
    dict(id="slaved_weapons", title="Slaved weapon mounts (shared fire)",
         status="export_gap", severity="low", wrong=False,
         detect=slaved,
         runtime="WeaponDef has no slave link; each mount fires independently",
         exporter="scripts/make_catalog.py:161 merges slaved mounts into projectiles, but prepare_from_replays.unit_entry (used for these catalogs) loops all mounts and does not",
         behavior="Slaved barrels (e.g. armjanus: two JANUS_ROCKET mounts) export as two independent weapons: total salvo damage is similar, but they aim and retarget separately and per-shot energy/metal/stockpile costs are doubled",
         fix="Reuse make_catalog's slaveto merge in unit_entry"),
    dict(id="bogus_weapons", title="`bogus` helper weapons with damage",
         status="export_gap", severity="medium", wrong=True,
         detect=bogus_damage,
         runtime="all exported weapons are treated as real", exporter="customparams.bogus not filtered",
         behavior="Display/targeting helper weapons would deal real damage", fix="Drop weapons with customparams.bogus"),
    dict(id="stockpile", title="Stockpiled weapons (stockpilelimit)",
         status="approximate", severity="low", wrong=False,
         detect=wfind(lambda m, wd: truthy(wd.get("stockpile"))),
         runtime="src/engine.rs:1446,1487; src/model.rs WeaponState keeps ONE stockpiled round queued",
         exporter="stockpile + stockpile_time exported; customparams.stockpilelimit ignored",
         behavior="Magazine depth is 1 regardless of stockpilelimit", fix="Add a stockpile_limit field"),
    dict(id="missile_flight", title="Missile acceleration, starburst climb, overrange/descent",
         status="approximate", severity="low", wrong=False,
         detect=wfind(lambda m, wd: str(wd.get("weapontype", "")) in ("MissileLauncher", "StarburstLauncher")
                      or "overrange_distance" in cp(wd) or "projectile_destruction_method" in cp(wd)),
         runtime="src/score.rs:495 declares 'weapon acceleration, bounce... omitted'; guided rounds use tracks/projectile_turn_rate (src/engine.rs:775,2085)",
         exporter="time-averaged missile speed (make_catalog.missile_speed); overrange_distance and descent ignored",
         behavior="Missile timing and long-range falloff are approximate", fix="Accept as declared approximation"),
    dict(id="bounce_projectiles", title="Bouncing projectiles",
         status="unsupported", severity="low", wrong=False,
         detect=wfind(lambda m, wd: truthy(wd.get("groundbounce")) or wd.get("numbounce") is not None),
         runtime="src/score.rs:495 states bounce is omitted", exporter="not exported",
         behavior="Bounce cannon rounds detonate on first impact", fix="Accept"),
    dict(id="water_weapons", title="Water/torpedo weapons on land-capable units",
         status="unsupported", severity="low", wrong=False,
         detect=wfind(lambda m, wd: str(wd.get("weapontype", "")) == "TorpedoLauncher" or truthy(wd.get("waterweapon"))),
         runtime="no waterweapon/firesubmersed logic in src/engine.rs", exporter="not exported",
         behavior="Torpedo weapons are fired like cannons or never; irrelevant on dry maps", fix="Gate by water depth"),
    dict(id="bad_target_categories", title="badtargetcategory (target de-prioritisation)",
         status="approximate", severity="low", wrong=False,
         detect=wfind(lambda m, wd: m.get("badtargetcategory") is not None),
         runtime="only only_target_categories is consumed (src/engine.rs:2794); no target-priority penalty",
         exporter="badtargetcategory is not exported; Recoil only de-prioritises such targets (RecoilEngine rts/Sim/Weapons/Weapon.cpp:709,745), it does not forbid them",
         behavior="Weapons pick e.g. VTOL or ALL-category targets with normal priority instead of last",
         fix="Optional: add a bad-target priority penalty in target scoring"),
    dict(id="reactive_armor", title="Reactive armor (damage-triggered restore)",
         status="unsupported", severity="medium", wrong=False,
         detect=ufind(lambda r: "reactive_armor_health" in cp(r)),
         runtime="no gadget model", exporter="gadget behavior in luarules/gadgets/unit_reactive_armor.lua is not exported",
         behavior="Units are plainly squishier than in BAR", fix="Model as extra damage buffer or exclude from curricula"),
    dict(id="damage_modifier", title="damagemodifier (flat damage scaling)",
         status="unsupported", severity="medium", wrong=False,
         detect=ufind(lambda r: r.get("damagemodifier") is not None),
         runtime="no consumer", exporter="not exported", behavior="Damage taken is unscaled (armsolar/corsolar take 100% instead of damagemodifier 0.5)", fix="Export a damage_taken multiplier"),
    dict(id="auto_heal", title="Autoheal / idle autoheal",
         status="unsupported", severity="low", wrong=False,
         detect=ufind(lambda r: r.get("autoheal") is not None or r.get("idleautoheal") is not None),
         runtime="no consumer", exporter="not exported", behavior="No passive healing", fix="Add regen fields"),
    dict(id="self_destruct_mines", title="Instant self-destruct and mines",
         status="unsupported", severity="low", wrong=False,
         detect=ufind(lambda r: truthy(cp(r).get("instantselfd")) or truthy(cp(r).get("mine"))),
         runtime="no self-destruct or proximity-detonation model; gadgets: unit_instant_self_destruct.lua, unit_minesweeper_detection.lua",
         exporter="not exported", behavior="Crawling bombs and mines do not detonate", fix="Add self-destruct order and proximity trigger"),
    dict(id="decoys", title="Decoy units (decoyfor)",
         status="unsupported", severity="low", wrong=False,
         detect=ufind(lambda r: "decoyfor" in cp(r) or r.get("decoyfor") is not None),
         runtime="no decoy model", exporter="not exported", behavior="Decoy commanders have distinct stats but no decoy semantic", fix="Accept"),
    dict(id="respawn_and_carriers", title="Commander respawn pad (spawnpad_unit) and weapon-spawned drones",
         status="unsupported", severity="low", wrong=False,
         detect=ufind(lambda r: "spawnpad_unit" in cp(r) or any("carried_unit" in cp(wd) for _, _, wd in weapons_of(r))),
         runtime="no spawn model", exporter="not exported (gadget: luarules/gadgets/game_commander_builder.lua)",
         behavior="Commanders have no respawn pad; no carried drones", fix="Accept; commander respawn is a game option"),
    dict(id="capture_terraform", title="Capture, restore and terraform orders",
         status="unsupported", severity="low", wrong=False,
         detect=ufind(lambda r: truthy(r.get("cancapture")) or truthy(r.get("canrestore"))),
         runtime="no capture/restore orders in src/strategy.rs StrategyAction", exporter="not exported",
         behavior="Capture/restore commands cannot be issued", fix="Low priority"),
    dict(id="crush_interaction", title="Crushing and crush resistance",
         status="unsupported", severity="low", wrong=False,
         detect=ufind(lambda r: r.get("crushresistance") is not None or truthy(cp(r).get("stompable"))),
         runtime="only a crushable:false constant (src/world.rs:4426,4457)", exporter="not exported",
         behavior="Tracked units do not run over small units", fix="Low priority"),
    dict(id="juno_special", title="Juno / script-only special weapons",
         status="unsupported", severity="low", wrong=False,
         detect=ufind(lambda r: "juno_kill" in cp(r) or "juno_deny" in cp(r)),
         runtime="no model", exporter="not exported", behavior="Juno radar/jammer pulse missing", fix="Accept"),
    dict(id="sight_air_sensors", title="3D sensor details (air LOS, emit heights)",
         status="approximate", severity="low", wrong=False,
         detect=ufind(lambda r: r.get("airsightdistance") is not None or r.get("radaremitheight") is not None),
         runtime="src/score.rs:499 declares sensors are 2D horizontal masks", exporter="sight_distance and radar_radius exported",
         behavior="Emit height and air-specific LOS ignored", fix="Accept"),
    dict(id="turret_reaim", title="Weapon re-aim delays and aim scripts",
         status="approximate", severity="low", wrong=False,
         detect=ufind(lambda r: "reaimtime" in cp(r)),
         runtime="turret yaw/pitch speeds supported (src/engine.rs); no re-aim latency",
         exporter="turret speeds from customparams or unit script; reaimtime ignored; scripts with shared CATT include get instant yaw (make_catalog docstring)",
         behavior="Aim response is slightly optimistic", fix="Accept"),
    dict(id="wind_output", title="Wind generators",
         status="approximate", severity="medium", wrong=False,
         detect=ufind(lambda r: float(r.get("windgenerator", 0) or 0) > 0),
         runtime="src/world.rs:3104 wind_generator * config.wind_output (a global scalar)",
         exporter="wind_generator exported; BAR's windgenerator is rescaled from map wind in luarules, map wind is not captured (prepare_from_replays WARNINGS)",
         behavior="Wind income is a scenario constant, not the map's wind; armwin is the single most-built structure (148k orders)", fix="Capture map wind min/max in the map source"),
]
DIRECT = [
    ("armour_damage", "Per-armour-class damage tables (damage_by_armour)", "src/engine.rs:3203,3325"),
    ("hitscan_beams", "BeamLaser/LaserCannon/Melee hitscan with damage_duration", "src/engine.rs:2558"),
    ("aoe_edge", "Area of effect and edge effectiveness", "src/engine.rs"),
    ("paralysis", "EMP/paralyzer damage with stun time and paralyze_multiplier", "src/engine.rs:28,1079"),
    ("economy", "Metal/energy income, storage, solar (negative upkeep), geothermal, tidal, mex", "src/world.rs:3090-3110"),
    ("construction", "Builders, factories, build options, assist, repair, reclaim, yardmaps, footprints, slope/water limits", "src/construction.rs, src/placement.rs"),
    ("sensors_basic", "Line of sight and radar radius with 2D masks", "src/world.rs:1568-1631"),
    ("activation", "onoffable/activate_when_built", "src/world.rs:4053"),
]


STATUS_ORDER = ("export_gap", "unsupported", "approximate")


def render_note(result):
    """Readable companion to support_audit.json; every number comes from the JSON."""
    f = result["features"]
    lines = [
        "# Catalog support audit: catalog-simulable.json",
        "",
        f"Audited {result['inputs']['units_audited']} units from `data/replay/catalog-simulable.json` against the MiniBAR runtime "
        f"(BAR source `{(result['inputs']['bar_commit'] or '')[:10]}`). Machine-readable detail: `support_audit.json` "
        "(per-feature unit lists, evidence, per-unit flags). Regenerate with `python scripts/audit_catalog.py`.",
        "",
        "Statuses are kept separate:",
        "",
        "- **direct**: exported by the converter and consumed by the runtime.",
        "- **approximate**: exported and consumed, but simplified.",
        "- **unsupported**: no runtime consumer exists (declared-but-inert counts here).",
        "- **export_gap**: the runtime implements it, but the converter never exports it, so this catalog loses it.",
        "  Rows marked **WRONG** make the unit do something actively incorrect in the sim, not merely lack a mechanic.",
        "",
        "The key finding is the fourth class. Most of the high-impact problems are not missing simulator features;",
        "the converter (`scripts/make_catalog.py` and `prepare_from_replays.unit_entry`) drops fields the runtime already consumes.",
        "Because the catalog also exports no `required_capabilities` for those mechanics, the runtime's capability gate",
        "(`src/engine.rs:753-785`) cannot refuse them, so they degrade silently.",
        "",
        "## Highest-impact items",
        "",
        "Weight = replay player-games that owned an affected unit (a unit counted once per player-game).",
        "",
        "| Rank | Mechanic | Status | Wrong? | Sev | Units | Player-games | Examples |",
        "| --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for i, x in enumerate(f[:12], 1):
        lines.append(f"| {i} | {x['title']} (`{x['id']}`) | {x['status']} | {'**WRONG**' if x['wrong_behavior'] else ''} | "
                     f"{x['severity']} | {x['units_affected']} | {x['player_games_weight']} | {', '.join(x['example_units'][:5])} |")
    lines += ["", "## Detail by status", ""]
    for status in STATUS_ORDER:
        group = [x for x in f if x["status"] == status]
        lines += [f"### {status} ({len(group)})", ""]
        for x in group:
            lines += [f"**{x['title']}** (`{x['id']}`, {x['severity']}, {x['units_affected']} units"
                      f"{', WRONG' if x['wrong_behavior'] else ''})", "",
                      f"- Runtime: {x['evidence_runtime']}.",
                      f"- Exporter/source: {x['evidence_exporter']}.",
                      f"- Effect now: {x['current_behavior']}.",
                      f"- Suggested fix: {x['suggested_fix']}.", ""]
    lines += ["### Directly represented", ""]
    lines += [f"- {d['title']} ({d['runtime']})" for d in result["directly_represented"]]
    lines += ["", "## Impact on the current from_replays training roster", ""]
    for fac, hits in result["roster_impact"].items():
        lines.append(f"- **{fac}**: " + "; ".join(f"{u}: {', '.join(v)}" for u, v in hits.items()))
    lines += [
        "",
        "The bounded T1 roster is untouched by every WRONG item above: no commander, interceptor, shield, converter or slaved-weapon unit is in it.",
        "It is touched by low-severity approximations (wrecks/explosions, aim latency, missile flight, `badtargetcategory`) and by",
        "`damage_modifier` on solar panels (BAR halves damage to them; the sim does not). Treat those as known approximations.",
        "Any curriculum that adds commanders, anti-nukes, shields, metal makers, or the Jaguar/Marauder/Banisher set needs the fixes first.",
        "",
        "## What was verified, and what was not",
        "",
        "- Verified by reading code: every runtime line anchor above, the converter lines, and that `armcom` exports its D-Gun with",
        "  `manual_fire: false` (source key is `commandfire`), `armamd` as a 72,000-range stockpiled launcher with no interceptor fields,",
        "  `armgate` as a damage-0 range-0 weapon, and `armmmkr` with metal_make 0 and no upkeep.",
        "- Not verified by simulation: that these units then misbehave in a running duel. The behavior column follows from the gates in",
        "  `src/engine.rs` (manual_fire / interceptor / is_shield exclusions), not from an observed run.",
        "- Jaguar (`armlatnk`) is excluded from the simulable catalog (`gadget weapon`: lightning chaining) but present in `catalog-full.json`.",
        "- Per-unit counts use pattern detectors over BAR source; a mechanic expressed only in a unit script (`.bos`) is not seen.",
        "",
    ]
    return "\n".join(lines)


def build_roster(manifest_path):
    if not os.path.exists(manifest_path):
        return {}
    m = json.load(open(manifest_path, encoding="utf-8"))
    out = {}
    for f in m.get("factions", []):
        out[f["name"]] = sorted({v for k, v in f.items() if k in ("builder", "mex", "power", "factory") }
                                | set(f.get("combat", [])) | set(f.get("additional_combat", [])))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--catalog", default=os.path.join(ROOT, "data", "replay", "catalog-simulable.json"))
    ap.add_argument("--manifest", default=os.path.join(ROOT, "data", "replay", "catalog-manifest.json"))
    ap.add_argument("--roster-manifest", default=os.path.join(ROOT, "data", "brains", "from_replays", "manifest.json"))
    ap.add_argument("--out-dir", default=os.path.join(ROOT, "data", "brains", "from_replays", "catalog_audit"))
    a = ap.parse_args()

    catalog = json.load(open(a.catalog, encoding="utf-8"))["unit_catalog"]
    manifest = json.load(open(a.manifest, encoding="utf-8"))
    usage = manifest["usage"]
    index = {}
    for r, _, fs in os.walk(os.path.join(BAR, "units")):
        for f in fs:
            if f.endswith(".lua"):
                index.setdefault(f[:-4].lower(), os.path.join(r, f))
    raws = {n: parse_unit_file(index[n])[n] for n in catalog}

    features, per_unit = [], collections.defaultdict(list)
    for spec in REGISTRY:
        hits = {}
        for n, raw in raws.items():
            ev = spec["detect"](n, raw)
            if ev:
                hits[n] = ev
        pg = sum(usage.get(n, {}).get("player_games", 0) for n in hits)
        bo = sum(usage.get(n, {}).get("build_orders", 0) for n in hits)
        for n in hits:
            per_unit[n].append(spec["id"])
        top = sorted(hits, key=lambda n: -usage.get(n, {}).get("player_games", 0))
        features.append({
            "id": spec["id"], "title": spec["title"], "status": spec["status"], "severity": spec["severity"],
            "wrong_behavior": spec["wrong"], "units_affected": len(hits),
            "player_games_weight": pg, "build_order_weight": bo,
            "example_units": top[:8], "units": {n: hits[n][:4] for n in sorted(hits)},
            "evidence_runtime": spec["runtime"], "evidence_exporter": spec["exporter"],
            "current_behavior": spec["behavior"], "suggested_fix": spec["fix"],
        })
    sev = {"high": 3, "medium": 2, "low": 1}
    features.sort(key=lambda f: (-(f["wrong_behavior"]), -sev[f["severity"]], -f["player_games_weight"]))
    roster = build_roster(a.roster_manifest)
    roster_hits = {}
    for faction, names in roster.items():
        roster_hits[faction] = {n: per_unit.get(n, []) for n in names if per_unit.get(n)}
    result = {
        "inputs": {"catalog": a.catalog, "manifest": a.manifest, "bar_commit": manifest.get("bar_commit"),
                   "units_audited": len(catalog)},
        "legend": {"direct": "exported and consumed", "approximate": "exported and consumed but simplified",
                   "unsupported": "no runtime consumer", "export_gap": "runtime implements it; converter does not export it",
                   "wrong_behavior": "unit actively misbehaves in the sim rather than merely lacking the mechanic"},
        "directly_represented": [{"id": i, "title": t, "runtime": r} for i, t, r in DIRECT],
        "features": features,
        "status_counts": dict(collections.Counter(f["status"] for f in features)),
        "units_with_any_gap": len(per_unit),
        "units": {n: {"features": per_unit[n], "usage": usage.get(n, {})} for n in sorted(per_unit)},
        "roster_impact": roster_hits,
    }
    os.makedirs(a.out_dir, exist_ok=True)
    json.dump(result, open(os.path.join(a.out_dir, "support_audit.json"), "w", encoding="utf-8"), indent=1)
    with open(os.path.join(a.out_dir, "SUPPORT_AUDIT.md"), "w", encoding="utf-8") as fh:
        fh.write(render_note(result))
    print(f"{len(catalog)} units; {len(per_unit)} touched by at least one gap; {dict(result['status_counts'])}")
    for f in features[:14]:
        print(f"  {f['status']:<11} {f['severity']:<6} wrong={f['wrong_behavior']!s:<5} units={f['units_affected']:<4} "
              f"games={f['player_games_weight']:<6} {f['id']}  e.g. {','.join(f['example_units'][:4])}")
    for fac, h in roster_hits.items():
        print("roster", fac, h)


if __name__ == "__main__":
    main()
