"""Prepare a bounded three-faction baseline from real replay maps and BAR source.

Replay terrain is a 32x32 engine capture, NOT a native-resolution acquisition.
Historical role metadata and current-source simulation approximations are kept
separate in provenance. This script never creates brain or training config files.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import subprocess
from pathlib import Path

from bar_lua import _Parser, _tokens, parse_unit_file
import make_catalog

FACTIONS = [
    {"name": "armada", "builder": "armck", "mex": "armmex", "power": "armsolar",
     "factory": "armlab", "combat": ["armpw", "armrock"], "additional_combat": ["armham"]},
    {"name": "cortex", "builder": "corck", "mex": "cormex", "power": "corsolar",
     "factory": "corlab", "combat": ["corak", "corstorm"], "additional_combat": ["corthud"]},
    {"name": "legion", "builder": "legck", "mex": "legmex", "power": "legsolar",
     "factory": "leglab", "combat": ["leggob", "legcen", "leglob", "legbal"]},
]
REPLAYS = [
    ("02b3a46ade11f0faca553ab8c65f4f50", "center-command"),
    ("accaa36a1e08853d863db036c9e7aa7b", "zed-remake"),
]
WARNINGS = [
    "Current BAR source definitions enrich historical replay metadata; this is not a runtime catalog export.",
    "32x32 point-sampled terrain/movement/metal data can miss narrow terrain and resource spots; 64-elmo simulation raster does not add captured detail.",
    "Metal density samples are area-weighted by coarse cell area / 16x16 engine metal-square area; resulting income is an approximation, not calibrated BAR income.",
    "Map wind, tidal, geothermal vents, static features and full mod options were not recorded and are unavailable; only solar is used in the baseline.",
    "Weapon approximation: gravity 130, missile velocity averaged over range, CATT/shared-script aiming may be instantaneous; no game-runtime calibration claimed.",
    "BAR postprocessing, rocket overrange/descent, death animation and model-piece origins are not replicated; source costs/build times and regular direct weapons are retained.",
    "Terrain leveling is not simulated: exported level_ground is false because BAR levelground requests leveling, while MiniBAR interprets true as an exactly-flat placement constraint. Source max_slope and water constraints remain active; raw source levelground is retained per unit in manifest provenance.",
]


def write_json(path: Path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n", encoding="utf-8")


def sha256(path: Path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def content_hash(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def flatten_xmajor(columns):
    if not columns or not columns[0] or any(len(c) != len(columns[0]) for c in columns):
        raise ValueError("expected a non-empty rectangular X-major grid")
    cols, rows = len(columns), len(columns[0])
    return cols, rows, [columns[x][z] for z in range(rows) for x in range(cols)]


def scalar_grid(columns, width, depth):
    cols, rows, values = flatten_xmajor(columns)
    dx, dz = width / cols, depth / rows
    return {"cols": cols, "rows": rows, "origin_x": dx / 2, "origin_z": dz / 2,
            "step_x": dx, "step_z": dz, "values": values}


def replay_map_source(header, surface, replay_id):
    width, depth = float(header["map_size_x"]), float(header["map_size_z"])
    terrain = surface["terrain"]
    masks = []
    for name, columns in sorted(terrain["movement_classes"].items()):
        cols, rows, values = flatten_xmajor(columns)
        masks.append({"movement_class": name.lower(), "cols": cols, "rows": rows,
                      "cell_size_x": width / cols, "cell_size_z": depth / rows,
                      "passable": [bool(x) for x in values]})
    if not any(m["movement_class"] == "bot2" for m in masks):
        raise ValueError("replay lacks the required engine-tested bot2 mask")
    metal = None
    nodes = []
    if header.get("metal_grid"):
        density = scalar_grid(header["metal_grid"], width, depth)
        scale = density["step_x"] * density["step_z"] / (16 * 16)
        metal = {"density": density, "extractor_radius": 500.0, "extraction_scale": scale}
        # Candidate sites are the SAME sparse metal samples, not additional deposits.
        # MiniBAR takes max(field, node), so they do not double-count the resource.
        for z in range(density["rows"]):
            for x in range(density["cols"]):
                value = density["values"][z * density["cols"] + x]
                if value > 0:
                    nodes.append({"x": density["origin_x"] + x * density["step_x"],
                                  "y": 0.0,
                                  "z": density["origin_z"] + z * density["step_z"],
                                  "yield_value": value * scale})
    result = {"source_version": 1, "map_name": header["map"], "width": width, "depth": depth,
              "water_level": float(surface.get("water_level", 0)),
              "heights": scalar_grid(terrain["heights"], width, depth), "passability": masks,
              "metal_nodes": nodes, "continuous_metal": metal, "geothermal_nodes": [],
              "static_blockers": [], "wind_min": None, "wind_max": None, "tidal": None}
    result["content_hash"] = "replay-coarse:" + replay_id + ":" + content_hash(result)
    return result


def flat_map_source(size):
    count = size // 64
    nodes = [{"x": x, "y": 10.0, "z": z, "yield_value": 2000.0}
             for x, z in ((size * .25, size * .25), (size * .75, size * .75),
                          (size * .25, size * .75), (size * .75, size * .25))]
    result = {"source_version": 1, "map_name": f"Synthetic flat {size}", "width": float(size),
              "depth": float(size), "water_level": 0.0,
              "heights": {"cols": count + 1, "rows": count + 1, "origin_x": 0.0, "origin_z": 0.0,
                          "step_x": 64.0, "step_z": 64.0, "values": [10.0] * ((count + 1) ** 2)},
              "passability": [{"movement_class": "bot2", "cols": count, "rows": count,
                               "cell_size_x": 64.0, "cell_size_z": 64.0, "passable": [True] * count ** 2}],
              "metal_nodes": nodes, "continuous_metal": None, "geothermal_nodes": [],
              "static_blockers": [], "wind_min": 0.0, "wind_max": 0.0, "tidal": 0.0}
    result["content_hash"] = "synthetic:" + content_hash(result)
    return result


def choose_positions(source):
    """Use fully open 3x3 captured-cell neighbourhoods; select largest open square for fights."""
    mask = next(m for m in source["passability"] if m["movement_class"] == "bot2")
    cols, rows, values = mask["cols"], mask["rows"], mask["passable"]
    dx, dz = mask["cell_size_x"], mask["cell_size_z"]
    candidates = [(x, z) for z in range(1, rows - 1) for x in range(1, cols - 1)
                  if all(values[zz * cols + xx] for zz in range(z-1, z+2) for xx in range(x-1, x+2))]
    if len(candidates) < 2:
        raise ValueError("no two legal 3x3 bot2 start neighbourhoods")
    first = min(candidates, key=lambda p: (p[0] + p[1], p))
    second = max(candidates, key=lambda p: ((p[0]-first[0])**2 * dx**2 + (p[1]-first[1])**2 * dz**2, p))
    starts = [{"x": (x + .5) * dx, "z": (z + .5) * dz} for x, z in (first, second)]
    dp = [[0] * (cols + 1) for _ in range(rows + 1)]
    best = (0, 0, 0)
    for z in range(rows):
        for x in range(cols):
            if values[z * cols + x]:
                side = dp[z+1][x+1] = 1 + min(dp[z][x+1], dp[z+1][x], dp[z][x])
                if side > best[0]:
                    best = (side, x+1, z+1)
    side, right, bottom = best
    center = [(right - side/2) * dx, (bottom - side/2) * dz]
    return starts, center


def clear_radius(source, center):
    mask = next(m for m in source["passability"] if m["movement_class"] == "bot2")
    dx, dz = mask["cell_size_x"], mask["cell_size_z"]
    x, z = center
    radius = min(x, z, source["width"] - x, source["depth"] - z)
    for index, passable in enumerate(mask["passable"]):
        if not passable:
            col, row = index % mask["cols"], index // mask["cols"]
            gap_x = max(col * dx - x, 0, x - (col + 1) * dx)
            gap_z = max(row * dz - z, 0, z - (row + 1) * dz)
            radius = min(radius, math.hypot(gap_x, gap_z))
    return max(0.0, radius - 32.0)


def categories_for(raw):
    """alldefs_post.lua categories specialized to the supported non-amphibious land subset."""
    mobile = float(raw.get("speed", 0)) > 0
    categories = set(str(raw.get("category", "")).upper().split())
    categories.update(("ALL", "NOTAIR", "NOTHOVER", "NOTSHIP", "NOTSUB", "SURFACE"))
    categories.add("MOBILE" if mobile else "NOTMOBILE")
    categories.add("WEAPON" if raw.get("weapondefs") else "NOWEAPON")
    if raw.get("customparams", {}).get("paralyzemultiplier", 1) != 0:
        categories.add("EMPABLE")
    categories.difference_update(str(raw.get("exemptcategory", "")).upper().split())
    return sorted(categories)


def unit_entry(name, raw, historical, supported):
    for key in ("health", "metalcost", "energycost", "buildtime"):
        if key not in raw:
            # A real zero energy cost (solar) is fine; missing data is not.
            raise ValueError(f"{name}: missing source {key}")
        if not math.isfinite(float(raw[key])) or (key != "energycost" and raw[key] <= 0):
            raise ValueError(f"{name}: invalid source {key}")
    speed = float(raw.get("speed", 0))
    builder = bool(raw.get("builder", False))
    factory = builder and speed == 0 and bool(raw.get("buildoptions"))
    options = [v for _, v in sorted(raw.get("buildoptions", {}).items(), key=lambda kv: float(kv[0])) if v in supported]
    scales = [float(x) for x in str(raw.get("collisionvolumescales", "")).split()]
    radius = max(scales[0], scales[2]) / 2 if len(scales) >= 3 else float(raw.get("footprintx", 2)) * 8
    height = scales[1] if len(scales) >= 3 else radius * 2
    weapons = []
    definitions = raw.get("weapondefs", {})
    for mount_index, mount in sorted(raw.get("weapons", {}).items(), key=lambda kv: float(kv[0])):
        wd = definitions[str(mount["def"]).lower()]
        weapon = make_catalog.weapon_entry(name, raw, int(mount_index), mount, wd)
        weapon.update({"damage_by_armour": {k: float(v) for k, v in wd.get("damage", {}).items() if k != "default"},
                       "only_target_categories": str(mount.get("onlytargetcategory", "")).upper().split(),
                       "energy_cost": float(wd.get("energypershot", 0)), "metal_cost": float(wd.get("metalpershot", 0)),
                       "avoid_friendly": bool(wd.get("avoidfriendly", True)),
                       "avoid_ground": bool(wd.get("avoidground", True)),
                       "collide_enemy": bool(wd.get("collideenemy", True)),
                       "collide_non_target": bool(wd.get("collidenontarget", True)),
                       "stockpile": bool(wd.get("stockpile", False)), "stockpile_time": float(wd.get("stockpiletime", 0)),
                       "paralyze_time": float(wd.get("paralyzetime", 10)),
                       "intercepted_by_shield_type": int(wd.get("interceptedbyshieldtype", 0))})
        weapons.append(weapon)
    entry = {"human_name": historical.get("human_name", raw.get("name", name)),
             "health": float(raw["health"]), "metal_cost": float(raw["metalcost"]),
             "energy_cost": float(raw["energycost"]), "build_time": float(raw["buildtime"]),
             "build_speed": float(raw.get("workertime", 0)), "build_distance": float(raw.get("builddistance", 128 if builder else 0)),
             "is_builder": builder, "is_factory": factory, "is_building": speed == 0,
             "is_mobile": speed > 0, "build_options": options,
             "footprint": [int(raw.get("footprintx", 1)), int(raw.get("footprintz", 1))],
             "movement_class": str(raw.get("movementclass", "")).lower() or None,
             "target_categories": categories_for(raw), "armour_class": raw.get("customparams", {}).get("armordef", "default"),
             "sight_distance": float(raw.get("sightdistance", 0)), "radar_radius": float(raw.get("radardistance", 0)),
             "can_assist": bool(raw.get("canassist", builder)), "can_be_assisted": bool(raw.get("canbeassisted", True)),
             "can_repeat": bool(raw.get("canrepeat", factory)), "activate_when_built": bool(raw.get("activatewhenbuilt", True)),
             "onoffable": bool(raw.get("onoffable", False)), "need_geo": bool(raw.get("needgeo", False)),
             "max_slope": raw.get("maxslope"), "min_water_depth": raw.get("minwaterdepth"),
             # BAR's flag requests terrain leveling. MiniBAR currently treats it
             # as zero allowed height difference, which rejects coarse sloped
             # samples. Keep slope/depth rules without inventing flattening.
             "max_water_depth": raw.get("maxwaterdepth"), "level_ground": False,
             "floater": bool(raw.get("floater", False)), "yardmap": raw.get("yardmap"),
             "paralyze_multiplier": float(raw.get("customparams", {}).get("paralyzemultiplier", 1)),
             "simulation": {"max_speed": speed, "reverse_speed": float(raw.get("rspeed", 0)),
                            "acceleration": float(raw.get("maxacc", 0)) * 900,
                            "braking": float(raw.get("maxdec", 0)) * 900,
                            "turn_rate": float(raw.get("turnrate", 0)) * .16,
                            "turn_in_place": bool(raw.get("turninplace", True)),
                            "turn_in_place_angle_limit": float(raw.get("turninplaceanglelimit", 90)),
                            "radius": radius, "height": height,
                            "blocks_movement": bool(raw.get("blocking", True)),
                            "blocks_fire": bool(raw.get("blocking", True)), "weapons": weapons}}
    for output, source in (("metal_storage", "metalstorage"), ("energy_storage", "energystorage"),
                           ("extracts_metal", "extractsmetal"), ("metal_make", "metalmake"),
                           ("energy_make", "energymake"), ("metal_upkeep", "metalupkeep"),
                           ("energy_upkeep", "energyupkeep"), ("wind_generator", "windgenerator"),
                           ("tidal_generator", "tidalgenerator")):
        entry[output] = float(raw.get(source, 0))
    entry["extract_range"] = float(raw.get("extractrange", 500 if entry["extracts_metal"] > 0 else 0))
    entry.update(make_catalog.sensor_fields(raw))
    entry.update(make_catalog.corpse_fields(name, raw))
    return make_catalog.apply_canonical_fields(name, raw, entry)


def read_replay(path):
    with path.open(encoding="utf-8-sig") as handle:
        header = json.loads(handle.readline())
        for line_number, line in enumerate(handle, start=2):
            if '"map_surface"' in line:
                surface = json.loads(line)
                if surface.get("kind") == "map_surface" and surface.get("terrain"):
                    return header, surface, line_number
    raise ValueError(f"no engine map_surface record in {path}")


def armor_classes(bar_root):
    path = bar_root / "gamedata" / "armordefs.lua"
    match = re.search(r"local armorDefs\s*=\s*(\{.*?\n\})", path.read_text(encoding="utf-8"), re.S)
    if not match:
        raise ValueError("cannot locate BAR's static armor definitions")
    tokens = _tokens(match.group(1)); parser = _Parser(tokens); parser.take()
    return {name: cls for cls, names in parser.table().items() for name in names.values()}


def prepare(out, bar_root, replay_root, sim=None):
    out, bar_root, replay_root = Path(out).resolve(), Path(bar_root).resolve(), Path(replay_root).resolve()
    make_catalog.BAR = str(bar_root)
    out.mkdir(parents=True, exist_ok=True)
    captures = []
    for replay_id, slug in REPLAYS:
        path = replay_root / replay_id / "replay-extract" / f"{replay_id}.jsonl"
        header, surface, line = read_replay(path)
        captures.append((replay_id, slug, path, header, surface, line))
    historical = captures[0][3]["unit_catalog"]
    wanted = {v for f in FACTIONS for k, v in f.items() if k in ("builder", "mex", "power", "factory")}
    wanted.update(n for f in FACTIONS for n in f["combat"])
    wanted.update(n for f in FACTIONS for n in f.get("additional_combat", []))
    unit_paths = {p.stem.lower(): p for p in (bar_root / "units").rglob("*.lua")
                  if not any(part in ("Scavengers", "other") for part in p.relative_to(bar_root / "units").parts)}
    armor = armor_classes(bar_root)
    units, evidence, excluded_options = {}, {}, {}
    for name in sorted(wanted):
        path = unit_paths[name]
        raw = parse_unit_file(path)[name]
        if name not in historical:
            raise ValueError(f"{name} absent from selected all-faction replay catalog")
        if raw.get("canfly") or raw.get("transportcapacity") or raw.get("canresurrect"):
            raise ValueError(f"unsupported mechanic selected for {name}")
        for wd in raw.get("weapondefs", {}).values():
            cp = wd.get("customparams", {})
            if any(k in cp for k in ("carried_unit", "cluster_def", "area_onhit_damage", "spark_forkdamage")):
                raise ValueError(f"unsupported gadget weapon selected for {name}")
        units[name] = unit_entry(name, raw, historical[name], wanted)
        units[name]["armour_class"] = raw.get("customparams", {}).get("armordef", armor.get(name, "default"))
        evidence[name] = {"source_path": str(path), "sha256": sha256(path),
                          "historical_role_metadata": historical[name],
                          "raw_source_levelground": raw.get("levelground"),
                          "exported_level_ground": False,
                          "raw_weapon_mounts": raw.get("weapons", {})}
        excluded_options[name] = [n for n in raw.get("buildoptions", {}).values() if n not in wanted]
    version = subprocess.run(["git", "-c", f"safe.directory={bar_root.as_posix()}", "-C", str(bar_root),
                              "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    catalog = {"type": "hello", "protocol": 1, "catalog_version": 2,
               "game_version": f"source-derived approximations BAR {version}; historical roles test-31251-0ceadc7",
               "starting_resources": {"metal": 1000, "energy": 1000}, "unit_catalog": units}
    write_json(out / "catalog.json", catalog)
    maps, sources = [], []
    for replay_id, slug, path, header, surface, line in captures:
        source = replay_map_source(header, surface, replay_id)
        starts, center = choose_positions(source)
        source_path = out / "maps" / f"{slug}.map_source.json"
        write_json(source_path, source)
        evidence_record = {"format": "minibar.replay_coarse_provenance", "version": 1,
                           "replay_id": replay_id, "source_path": str(path), "sha256": sha256(path),
                           "header_line": 1, "map_surface_line": line, "game_version": header.get("game_version"),
                           "capture_cols": source["heights"]["cols"], "capture_rows": source["heights"]["rows"],
                           "sample_spacing_x": source["heights"]["step_x"], "sample_spacing_z": source["heights"]["step_z"],
                           "warnings": WARNINGS[1:4]}
        write_json(out / "maps" / f"{slug}.replay_provenance.json", evidence_record)
        sources.append(evidence_record)
        maps.append({"name": header["map"], "source": str(source_path.relative_to(out)).replace("\\", "/"),
                     "cache": None, "movement_class": "bot2", "cell_size": 64,
                     "starts": starts, "center": center, "synthetic": False,
                     "tactical_clear_radius": clear_radius(source, center),
                     "capture_resolution": [source["heights"]["cols"], source["heights"]["rows"]],
                     "capture_sample_spacing": [source["heights"]["step_x"], source["heights"]["step_z"]]})
    for size in (512, 1024, 2048):
        source = flat_map_source(size); starts, center = choose_positions(source)
        relative = f"maps/synthetic-{size}.map_source.json"
        write_json(out / relative, source)
        maps.append({"name": source["map_name"], "source": relative, "cache": None,
                     "movement_class": "bot2", "cell_size": 64, "starts": starts, "center": center,
                     "tactical_clear_radius": clear_radius(source, center),
                     "synthetic": True, "capture_resolution": None})
    if sim:
        for record in maps:
            cache = record["source"].replace(".map_source.json", ".vsgmap")
            subprocess.run([str(Path(sim).resolve()), "--compile-map", str(out / record["source"]),
                            "--map-out", str(out / cache)], check=True)
            record["cache"] = cache
    manifest = {"format": "minibar.from_replays_assets", "version": 1, "catalog": "catalog.json",
                "source_version": version, "replay_game_version": captures[0][3].get("game_version"),
                "factions": FACTIONS, "maps": maps, "warnings": WARNINGS,
                "unit_count": len(units), "unit_sources": evidence, "replay_sources": sources,
                "preparation_inputs": [{"path": str(p), "sha256": sha256(p)} for p in
                                       (Path(__file__), Path(make_catalog.__file__),
                                        Path(__file__).with_name("bar_lua.py"),
                                        bar_root / "gamedata" / "armordefs.lua",
                                        bar_root / "gamedata" / "alldefs_post.lua")],
                "excluded_build_options": excluded_options,
                "exclusions": {"legkark": "reactive armor gadget is unsupported",
                               "air_naval_amphibious": "no flight/submerged movement or sensing fidelity",
                               "carriers_cluster_persistent_chain": "gadget damage/spawn behavior unsupported",
                               "other_tiers_and_support": "bounded T1 ground/economy baseline; filtered build options explicit"}}
    write_json(out / "manifest.json", manifest)
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("data/brains/from_replays"))
    parser.add_argument("--bar-root", type=Path, default=Path(r"D:\code\BAR\Beyond-All-Reason"))
    parser.add_argument("--replay-root", type=Path, default=Path(r"D:\code\BAR\visigoth\data\replay-runs"))
    parser.add_argument("--sim", type=Path, help="optional MiniBAR executable to compile map caches")
    args = parser.parse_args()
    manifest = prepare(args.out, args.bar_root, args.replay_root, args.sim)
    print(f"Prepared {manifest['unit_count']} units and {len(manifest['maps'])} maps in {args.out}")


if __name__ == "__main__":
    main()
