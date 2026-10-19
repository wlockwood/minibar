"""Export a source-backed diverse tactical catalog and matching curriculum pools.

No completed campaign is modified. Flight export requires the native flight
capability and bounded fixtures before the generated catalog can train champions.
"""
from __future__ import annotations
import argparse
import json
import math
import subprocess
from pathlib import Path

import make_catalog
from bar_lua import parse_unit_file
from prepare_from_replays import unit_entry, armor_classes, sha256, write_json

DEFAULT_UNITS = "armpw armrock armham corak corstorm corthud corsumo armmanni corban armstump corraid armjeth corcrash armllt armbeamer armhlt armguard armbrtha armvulc armrl armflak armfig corveng armpnix corhurc armrad corrad armpeep corfink armjamt corjamt".split()
UNSUPPORTED_GADGETS = ("carried_unit", "cluster_def", "area_onhit_damage", "spark_forkdamage")


def convert_unit(name, raw, supported):
    for key in ("transportcapacity", "canresurrect"):
        if raw.get(key):
            raise ValueError(f"{name}: unsupported {key} mechanic")
    if float(raw.get("minwaterdepth", 0)) > 0 or raw.get("floater"):
        raise ValueError(f"{name}: naval movement is outside the tactical curriculum")
    for wd in raw.get("weapondefs", {}).values():
        for key in UNSUPPORTED_GADGETS:
            if key in wd.get("customparams", {}):
                raise ValueError(f"{name}: unsupported gadget {key}")
    entry = unit_entry(name, raw, {}, supported)
    value = entry["metal_cost"] + entry["energy_cost"] / 60
    if not math.isfinite(value) or value <= 0:
        raise ValueError(f"{name}: resource value must be finite and positive")
    if raw.get("canfly"):
        if raw.get("hoverattack"):
            raise ValueError(f"{name}: hovering gunships need a separate validated flight model")
        speed = float(raw.get("speed", 0))
        radius = float(raw.get("turnradius", 0))
        if not math.isfinite(radius) or radius <= 0 or not math.isfinite(speed) or speed <= 0:
            raise ValueError(f"{name}: fixed-wing export needs positive speed and turnradius")
        # Bounded kinematic approximation: angular speed = forward speed / radius.
        # Aircraft source definitions do not use the ground unit turnrate field.
        entry["simulation"]["turn_rate"] = math.degrees(speed / radius)
        entry["flight"] = {"cruise_altitude": float(raw.get("cruisealtitude", 100)),
                           "minimum_speed": speed * 0.55,
                           "attack_run": not bool(raw.get("airstrafe", False))}
        categories = set(entry["target_categories"])
        categories.difference_update(("NOTAIR", "SURFACE"))
        categories.update(("AIR", "VTOL", "NOTSURFACE"))
        entry["target_categories"] = sorted(categories)
        entry["movement_class"] = None
        entry["use_air_los"] = True
        entry["simulation"]["blocks_movement"] = False
        entry["simulation"]["blocks_fire"] = False
        entry["required_capabilities"] = ["flight"]
    return entry


def curriculum(units):
    def pool(names):
        return [{"unit": n, "weight": 1.0} for n in names if n in units]
    types = [
        {"name": "ground", "bots": pool("armpw armrock armham corak corstorm corthud corsumo".split()), "vehicles": pool("armstump corraid armmanni corban".split())},
        {"name": "turrets", "static_units": pool("armllt armbeamer armhlt armguard armbrtha armvulc".split())},
        {"name": "fighters", "aircraft": pool(["armfig", "corveng"])},
        {"name": "bombers", "aircraft": pool(["armpnix", "corhurc"])},
        {"name": "aa", "bots": pool(["armjeth", "corcrash"]), "static_units": pool(["armrl", "armflak"])},
    ]
    types = [t for t in types if any(t.get(k) for k in ("bots", "vehicles", "static_units", "aircraft"))]
    names = {t["name"] for t in types}
    families = [{"family": family, "learner_type": a, "enemy_type": b} for family,a,b in
                [("mixed_ground", "ground", "ground"), ("mobile_turret", "ground", "turrets"),
                 ("mobile_turret", "turrets", "ground"), ("air_air", "fighters", "fighters"),
                 ("bomber_aa", "bombers", "aa"), ("bomber_aa", "aa", "bombers")]
                if a in names and b in names]
    def army(unit, count):
        return [{"unit": unit, "count": count}]
    anchors = []
    for name,family,a,na,b,nb,gap in [("Mammoth vs 40 Pawns","heavy_swarm","corsumo",1,"armpw",40,0.05),
                                     ("Starlight vs Banisher","artillery","armmanni",1,"corban",1,0.1),
                                     ("Ragnarok vs 30 Mammoths","mobile_turret","armvulc",1,"corsumo",30,0.05)]:
        if a in units and b in units:
            anchors.append({"name":name,"family":family,"learner":army(a,na),"enemy":army(b,nb),"max_gap":gap})
    # These are combat-control exercises. Starts outside both armies' sight
    # produce stationary policies with no contacts rather than useful fighting.
    return {"count": 24, "metal": [500, 6000], "max_units": 96, "max_balanced_gap": 0.05,
            "separation": [200, 320],
            "even_ratio": [0.98, 1.02], "types":types,"families":families,"anchors":anchors}


def export(out, bar_root, selected=DEFAULT_UNITS):
    out, bar_root = Path(out).resolve(), Path(bar_root).resolve()
    make_catalog.BAR = str(bar_root)
    paths = {p.stem.lower(): p for p in (bar_root / "units").rglob("*.lua")
             if not any(part in ("Scavengers", "other") for part in p.relative_to(bar_root / "units").parts)}
    armor = armor_classes(bar_root)
    language_path = bar_root / "language" / "en" / "units.json"
    human_names = json.loads(language_path.read_text(encoding="utf-8"))["units"]["names"]
    units, evidence, rejected = {}, {}, {}
    for name in sorted(set(selected)):
        try:
            if name not in paths:
                raise ValueError(f"{name}: missing source unit")
            path = paths[name]
            raw = parse_unit_file(path)[name]
            units[name] = convert_unit(name, raw, set(selected))
            units[name]["human_name"] = human_names.get(name, name)
            units[name]["armour_class"] = raw.get("customparams", {}).get("armordef", armor.get(name, "default"))
            evidence[name] = {"source_path": str(path), "sha256": sha256(path),
                              "raw_weapon_mounts": raw.get("weapons", {}),
                              "raw_levelground": raw.get("levelground"),
                              "resource_value": units[name]["metal_cost"] + units[name]["energy_cost"] / 60}
        except (ValueError, KeyError) as error:
            rejected[name] = str(error)
    revision = subprocess.run(["git", "-c", f"safe.directory={bar_root.as_posix()}", "-C", str(bar_root),
                               "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
    out.mkdir(parents=True, exist_ok=True)
    write_json(out / "catalog.json", {"type":"hello","protocol":1,"catalog_version":2,
               "game_version":f"BAR source {revision}; bounded tactical approximations",
               "starting_resources":{"metal":1000000,"energy":1000000},"unit_catalog":units})
    matchups = curriculum(units)
    matchups["unit_names"] = {name: entry["human_name"] for name, entry in units.items()}
    write_json(out / "matchups.json", matchups)
    manifest = {"format":"minibar.tactical_source_catalog","version":1,"source_revision":revision,
                "units":evidence,"rejected":rejected,
                "unit_name_source":{"path":str(language_path),"sha256":sha256(language_path)},
                "resource_formula":"metal + energy / 60",
                "supply_assumption":"Finite 1,000,000 metal and energy per side; no resupply.",
                "support_status":"Requires native capability validation and combat fixtures; not native BAR equivalence.",
                "approximations":["Source unit/script exporter conversions, not a running engine export.",
                                  "Flat terrain uses map gravity 130; missile speed averaged over range.",
                                  "Unresolved script turret speeds use instantaneous aim.",
                                  "Flight is bounded cruise/turn/attack-run simulation; takeoff/landing is absent.",
                                  "Aircraft angular speed is speed / source turnradius (radians/second), a kinematic approximation rather than BAR aerodynamic steering."]}
    write_json(out / "manifest.json", manifest)
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bar-root", type=Path, default=Path(make_catalog.BAR))
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("units", nargs="*")
    args = parser.parse_args()
    manifest = export(args.out, args.bar_root, args.units or DEFAULT_UNITS)
    print(json.dumps({"exported":len(manifest["units"]),"rejected":manifest["rejected"],"out":str(args.out)}))
    return 1 if manifest["rejected"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
