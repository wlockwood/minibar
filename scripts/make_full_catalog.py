#!/usr/bin/env python3
"""Build a MiniBAR catalog of every unit human players used in the replay corpus.

Universe : every unit def that appears in a replay observation (owned units) or as a
           `build_def` of a build command, across the 527 visigoth replays.
Excluded : scavenger, raptor, critter, lootbox, hat/shoulder cosmetic and other debug or
           joke units (anything under BAR units/Scavengers or units/other, `*_scav` names,
           `raptor*`, `cor_hat_*`, `*_leftshoulder_*`), plus defs with no source file.
Detail   : per-unit fields come from BAR unit source through prepare_from_replays.unit_entry
           (same converter as data/brains/from_replays); human names/roles from the replay
           header's runtime catalog. Not a game-runtime export; see the catalog game_version.

Outputs (default dir data/replay/):
  catalog-full.json       every convertible unit (includes air/naval the sim cannot yet fly/float)
  catalog-simulable.json  subset without unsupported mechanics (flight, transport, resurrect,
                          gadget weapons, water-bound movement)
  catalog-manifest.json   per-unit usage counts, support flags/reasons, exclusions, build options
                          dropped because their target is not in the catalog

    python scripts/make_full_catalog.py                # uses data/replay/unit_usage.json if present
    python scripts/make_full_catalog.py --refresh-usage
"""
import argparse
import collections
import glob
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import make_catalog  # noqa: E402
import prepare_from_replays as prep  # noqa: E402
from bar_lua import parse_unit_file  # noqa: E402

VISIGOTH = r"D:\code\BAR\visigoth"
BAR = r"D:\code\BAR\Beyond-All-Reason"
EXCLUDED_TOP = ("scavengers", "other")
COSMETIC_MARKERS = ("_hat_", "_leftshoulder_", "_rightshoulder_")
GADGET_WEAPON_KEYS = ("carried_unit", "cluster_def", "area_onhit_damage", "spark_forkdamage")
WATER_CLASSES = ("boat", "ship", "sub", "uw", "hover")  # substring match on movementclass; hover handled below
NAVAL_MARKERS = ("boat", "ship", "sub", "uw")


def usage(visigoth, cache, refresh):
    if os.path.exists(cache) and not refresh:
        return json.load(open(cache, encoding="utf-8"))
    import pyarrow.parquet as pq

    games = collections.defaultdict(set)
    built = collections.Counter()
    for shard in sorted(glob.glob(os.path.join(visigoth, "data", "trajectories", "per-replay", "*.parquet"))):
        t = pq.read_table(shard, columns=["replay_id", "player", "sample_json"])
        for rid, pl, v in zip(*(t.column(c).to_pylist() for c in ("replay_id", "player", "sample_json"))):
            s = json.loads(v)
            for d in {u["def"] for u in s["observation"].get("units", [])}:
                games[d].add(f"{rid}:{pl}")
            for c in s["commands"]:
                if c.get("build_def"):
                    built[c["build_def"]] += 1
    result = {"player_games_owning": {k: len(v) for k, v in games.items()}, "build_orders": dict(built)}
    os.makedirs(os.path.dirname(cache), exist_ok=True)
    json.dump(result, open(cache, "w", encoding="utf-8"))
    return result


def source_index(bar):
    idx = {}
    root = os.path.join(bar, "units")
    for r, _, fs in os.walk(root):
        for f in fs:
            if f.endswith(".lua"):
                idx.setdefault(f[:-4].lower(), os.path.join(r, f))
    return idx


def exclusion(name, path, bar):
    if any(m in name for m in COSMETIC_MARKERS):
        return "cosmetic hat/shoulder piece"
    if name.endswith("_scav") or name.startswith("raptor") or "scav" in name:
        return "scavenger/raptor"
    if path is None:
        return "no source file in checkout"
    rel = os.path.relpath(path, os.path.join(bar, "units")).replace("\\", "/").lower()
    top = rel.split("/")[0]
    if top == "scavengers":
        return "scavenger"
    if top == "other":
        sub = rel.split("/")[1] if rel.count("/") else ""
        return f"other/{sub}" if sub else "other (debug/joke/event)"
    return None


def support_reasons(raw):
    reasons = []
    if raw.get("canfly"):
        reasons.append("flight")
    if raw.get("transportcapacity"):
        reasons.append("transport")
    if raw.get("canresurrect"):
        reasons.append("resurrect")
    mc = str(raw.get("movementclass", "")).lower()
    if any(m in mc for m in NAVAL_MARKERS) or float(raw.get("minwaterdepth", 0) or 0) > 0:
        reasons.append("water-bound")
    for wd in (raw.get("weapondefs") or {}).values():
        cp = wd.get("customparams", {}) or {}
        if any(k in cp for k in GADGET_WEAPON_KEYS):
            reasons.append("gadget weapon")
            break
    return reasons


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--visigoth", default=VISIGOTH)
    ap.add_argument("--bar-root", default=BAR)
    ap.add_argument("--out-dir", default=os.path.join("data", "replay"))
    ap.add_argument("--refresh-usage", action="store_true")
    ap.add_argument("--header-replay", default="02b3a46ade11f0faca553ab8c65f4f50",
                    help="replay-run whose header unit_catalog supplies human names/roles")
    a = ap.parse_args()

    use = usage(a.visigoth, os.path.join(a.out_dir, "unit_usage.json"), a.refresh_usage)
    universe = sorted(set(use["player_games_owning"]) | set(use["build_orders"]))
    header_path = os.path.join(a.visigoth, "data", "replay-runs", a.header_replay, "replay-extract", a.header_replay + ".jsonl")
    historical = json.loads(open(header_path, encoding="utf-8").readline())["unit_catalog"]
    make_catalog.BAR = a.bar_root
    index = source_index(a.bar_root)
    armor = prep.armor_classes(__import__("pathlib").Path(a.bar_root))

    excluded, errors, raws, flags = {}, {}, {}, {}
    for name in universe:
        why = exclusion(name, index.get(name), a.bar_root)
        if why:
            excluded[name] = why
            continue
        try:
            raws[name] = parse_unit_file(index[name])[name]
        except Exception as exc:  # noqa: BLE001 - recorded, not hidden
            errors[name] = f"parse: {exc}"
    # Close over build options so every constructor/factory keeps its full build tree, not only
    # the items humans happened to build. Added units are marked in the manifest (usage 0).
    closure = set()
    frontier = list(raws)
    while frontier:
        for opt in raws[frontier.pop()].get("buildoptions", {}).values():
            opt = str(opt).lower()
            if opt in raws or opt in excluded or opt in errors or opt not in index:
                continue
            if exclusion(opt, index[opt], a.bar_root):
                excluded[opt] = exclusion(opt, index[opt], a.bar_root)
                continue
            try:
                raws[opt] = parse_unit_file(index[opt])[opt]
            except Exception as exc:  # noqa: BLE001
                errors[opt] = f"parse: {exc}"
                continue
            closure.add(opt)
            frontier.append(opt)
    kept = set(raws)
    entries, dropped_options = {}, {}
    for name, raw in raws.items():
        try:
            entry = prep.unit_entry(name, raw, historical.get(name, {}), kept)
        except Exception as exc:  # noqa: BLE001
            errors[name] = f"convert: {exc!r}"
            continue
        entry["armour_class"] = raw.get("customparams", {}).get("armordef", armor.get(name, "default"))
        entries[name] = entry
        flags[name] = support_reasons(raw)
        lost = [v for v in raw.get("buildoptions", {}).values() if v not in kept]
        if lost:
            dropped_options[name] = lost
    # build options may only name units that survived conversion
    for name, entry in entries.items():
        entry["build_options"] = [o for o in entry["build_options"] if o in entries]

    version = subprocess.run(["git", "-c", f"safe.directory={a.bar_root.replace(os.sep, '/')}", "-C", a.bar_root,
                              "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()

    def doc(units, label):
        return {"type": "hello", "protocol": 1, "catalog_version": 2,
                "game_version": f"source-derived approximations BAR {version}; replay corpus test-31251-0ceadc7; {label}",
                "starting_resources": {"metal": 1000, "energy": 1000}, "unit_catalog": units}

    simulable = {n: e for n, e in entries.items() if not flags[n]}
    os.makedirs(a.out_dir, exist_ok=True)
    for fname, units, label in (("catalog-full.json", entries, "every convertible replay unit"),
                                ("catalog-simulable.json", simulable, "no flight/transport/resurrect/water/gadget units")):
        with open(os.path.join(a.out_dir, fname), "w", encoding="utf-8") as fh:
            json.dump(doc(units, label), fh, indent=1)
    simulable_options_lost = {n: [o for o in e["build_options"] if o not in simulable] for n, e in simulable.items()}
    manifest = {
        "bar_commit": version, "universe": len(universe), "added_by_build_option_closure": sorted(closure), "full": len(entries), "simulable": len(simulable),
        "excluded": excluded, "errors": errors,
        "unsupported_reasons": {n: r for n, r in flags.items() if r},
        "build_options_not_in_catalog": dropped_options,
        "build_options_not_in_simulable": {n: o for n, o in simulable_options_lost.items() if o},
        "usage": {n: {"player_games": use["player_games_owning"].get(n, 0), "build_orders": use["build_orders"].get(n, 0)}
                  for n in entries},
    }
    json.dump(manifest, open(os.path.join(a.out_dir, "catalog-manifest.json"), "w", encoding="utf-8"), indent=1)
    by_reason = collections.Counter(excluded.values())
    print(f"closure added {len(closure)} | universe {len(universe)} | full {len(entries)} | simulable {len(simulable)} | "
          f"excluded {len(excluded)} {dict(by_reason)} | errors {len(errors)}")
    for n, e in list(errors.items())[:15]:
        print("  error", n, e)


if __name__ == "__main__":
    main()
