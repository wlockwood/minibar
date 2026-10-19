#!/usr/bin/env python3
"""How often do council units run away from a fight, and which ones?

Replays battles with `--watch-battle` and, at every recorded decision of a
learner unit, asks whether it is moving AWAY from its nearest enemy at real
speed while that enemy is close. Reports the share of unit-time spent fleeing,
by platoon commitment, by unit type, and for the most affected battles.

    python scripts/flee_report.py --exe <sim> --catalog examples/t1-catalog.json \
        --brain data/brains/obj/champion.json --config examples/train-t1-objectives.json
"""

import argparse
import json
import math
import os
import subprocess
import sys
from collections import defaultdict


def run(exe, catalog, brain, config, scenario, opponent, seed, side):
    out = subprocess.run(
        [os.path.abspath(exe), "--catalog", catalog, "--watch-battle", brain, "--train-config", config,
         "--watch-scenario", str(scenario), "--watch-opponent", str(opponent),
         "--watch-seed", str(seed), "--watch-side", str(side)],
        capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        sys.exit(out.stderr[-1500:])
    return json.loads(out.stdout)


def meta(exe, catalog, brain, config):
    out = subprocess.run([os.path.abspath(exe), "--catalog", catalog, "--watch-meta", brain, "--train-config", config],
                         capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        sys.exit(out.stderr[-1500:])
    return json.loads(out.stdout)


RANGES = {}


def weapon_range(name):
    return RANGES.get(name) or 200.0


def analyse(battle):
    units = {u["id"]: u for u in battle["units"]}
    side = battle["learner_side"]
    stats = {"samples": 0, "flee": 0, "cause": defaultdict(int), "gap": defaultdict(int), "fire": {True: [0, 0], False: [0, 0]}, "absent": [0, 0, 0], "absent_type": defaultdict(lambda: [0, 0]), "absent_commit": defaultdict(lambda: [0, 0]), "by_type": defaultdict(lambda: [0, 0]), "by_commit": defaultdict(lambda: [0, 0])}
    for frame in battle["frames"]:
        pos = {a[0]: (a[1], a[2], a[4]) for a in frame["u"] if a[4] > 0}
        shooters = {int(x[0]) for x in frame.get("f", [])}
        enemies = [i for i in pos if units[i]["side"] != side]
        if not enemies:
            continue
        for d in frame["d"]:
            me = pos.get(d["unit"])
            if me is None or units[d["unit"]]["side"] != side:
                continue
            nearest = min(enemies, key=lambda i: (pos[i][0] - me[0]) ** 2 + (pos[i][1] - me[1]) ** 2)
            dx, dz = pos[nearest][0] - me[0], pos[nearest][1] - me[1]
            dist = math.hypot(dx, dz)
            mx, mz = d["movement"]
            speed = math.hypot(mx, mz)
            fleeing = False
            if dist > 1e-6 and speed > 1e-6 and d["throttle"] > 0.3 and dist < weapon_range(units[d["unit"]]["name"]) * 3.0:
                fleeing = (mx * dx + mz * dz) / (speed * dist) < -0.3
            name = units[d["unit"]]["name"]
            # "Absent": more than 1.5 ranges from every enemy while a friend is in range of one.
            my_range = weapon_range(name)
            engaged_friend = any(
                units[j]["side"] == side and j != d["unit"] and
                min(math.hypot(pos[j][0] - pos[e][0], pos[j][1] - pos[e][1]) for e in enemies) <= weapon_range(units[j]["name"])
                for j in pos)
            if engaged_friend:
                stats["absent"][0] += 1
                far = dist > my_range * 1.5
                cb = "commit<0.2" if d["commit"] < 0.2 else "commit<0.7" if d["commit"] < 0.7 else "commit>=0.7"
                stats["absent_type"][name][0] += 1
                stats["absent_type"][name][1] += far
                stats["absent_commit"][cb][0] += 1
                stats["absent_commit"][cb][1] += far
                if far:
                    stats["absent"][1] += 1
                    stats["absent"][2] += fleeing
            bucket = "commit<0.2" if d["commit"] < 0.2 else "commit<0.7" if d["commit"] < 0.7 else "commit>=0.7"
            stats["samples"] += 1
            stats["flee"] += fleeing
            stats["fire"][fleeing][0] += 1
            stats["fire"][fleeing][1] += d["unit"] in shooters
            if fleeing:
                ratio = dist / weapon_range(units[d["unit"]]["name"])
                stats["gap"]["enemy within 0.6 of range" if ratio < 0.6 else "0.6-1.0 of range (kiting)" if ratio < 1.0 else "1.0-1.5 of range (out of range)" if ratio < 1.5 else "beyond 1.5 of range"] += 1
                advance, orbit, regroup, retreat, bloom = d["out"]
                cause = "advance<-0.3 (spacing backs off)" if advance < -0.3 else                     "retreat>0.3 (engagement)" if retreat > 0.3 else                     "other terms outweigh a forward advance"
                stats["cause"][cause] += 1
            stats["by_type"][name][0] += 1
            stats["by_type"][name][1] += fleeing
            stats["by_commit"][bucket][0] += 1
            stats["by_commit"][bucket][1] += fleeing
    return stats


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--exe", required=True)
    parser.add_argument("--catalog", required=True)
    parser.add_argument("--brain", required=True)
    parser.add_argument("--config", required=True)
    parser.add_argument("--opponents", default="charge,kite", help="comma list of names from the config")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--max-scenarios", type=int, default=12)
    args = parser.parse_args()
    m = meta(args.exe, args.catalog, args.brain, args.config)
    wanted = [m["opponents"].index(o) for o in args.opponents.split(",")]
    total = {"samples": 0, "flee": 0}
    causes = defaultdict(int)
    gaps = defaultdict(int)
    fire = {True: [0, 0], False: [0, 0]}
    absent = [0, 0, 0]
    absent_type, absent_commit = defaultdict(lambda: [0, 0]), defaultdict(lambda: [0, 0])
    for name, entry in json.load(open(args.catalog, encoding="utf-8"))["unit_catalog"].items():
        weapons = entry.get("simulation", {}).get("weapons", [])
        if weapons:
            RANGES[name] = weapons[0]["range"]
    by_type, by_commit, worst = defaultdict(lambda: [0, 0]), defaultdict(lambda: [0, 0]), []
    for si, scenario in enumerate(m["scenarios"][: args.max_scenarios]):
        for oi in wanted:
            for side in (0, 1):
                b = run(args.exe, args.catalog, args.brain, args.config, si, oi, args.seed, side)
                s = analyse(b)
                total["samples"] += s["samples"]
                total["flee"] += s["flee"]
                for k, n in s["cause"].items():
                    causes[k] += n
                for k, n in s["gap"].items():
                    gaps[k] += n
                for i in range(3):
                    absent[i] += s["absent"][i]
                for src, dst in ((s["absent_type"], absent_type), (s["absent_commit"], absent_commit)):
                    for k, (n, f) in src.items():
                        dst[k][0] += n
                        dst[k][1] += f
                for k in (True, False):
                    fire[k][0] += s["fire"][k][0]
                    fire[k][1] += s["fire"][k][1]
                for k, (n, f) in s["by_type"].items():
                    by_type[k][0] += n
                    by_type[k][1] += f
                for k, (n, f) in s["by_commit"].items():
                    by_commit[k][0] += n
                    by_commit[k][1] += f
                share = s["flee"] / max(1, s["samples"])
                worst.append((share, scenario["name"][:110], m["opponents"][oi], side, b["outcome"]))
    pct = lambda f, n: 100.0 * f / max(1, n)
    print("fleeing unit-time: %.1f%% of %d decisions" % (pct(total["flee"], total["samples"]), total["samples"]))
    print("\nwhy (share of fleeing decisions):")
    for k, n in sorted(causes.items(), key=lambda kv: -kv[1]):
        print("  %5.1f%%  %s" % (pct(n, total["flee"]), k))
    print("\nwhere the nearest enemy is when a unit backs off:")
    for k, n in sorted(gaps.items(), key=lambda kv: -kv[1]):
        print("  %5.1f%%  %s" % (pct(n, total["flee"]), k))
    print("\nshare of decision samples in which the unit fired:")
    print("  while backing off   %5.1f%%  (n=%d)" % (pct(fire[True][1], fire[True][0]), fire[True][0]))
    print("  otherwise           %5.1f%%  (n=%d)" % (pct(fire[False][1], fire[False][0]), fire[False][0]))
    print("\nabsent from the fight (over 1.5 ranges from every enemy while a friend is in range of one):")
    print("  %5.1f%% of unit-time while a friend is engaged (n=%d); %5.1f%% of those moving away" % (
        pct(absent[1], absent[0]), absent[0], pct(absent[2], absent[1])))
    for k in ("commit<0.2", "commit<0.7", "commit>=0.7"):
        n, f = absent_commit[k]
        print("    absent at %-12s %5.1f%%  (n=%d)" % (k, pct(f, n), n))
    for k, (n, f) in sorted(absent_type.items(), key=lambda kv: -pct(kv[1][1], kv[1][0]))[:6]:
        print("    absent %-10s %5.1f%%  (n=%d)" % (k, pct(f, n), n))
    print("\nby platoon commitment:")
    for k in ("commit<0.2", "commit<0.7", "commit>=0.7"):
        n, f = by_commit[k]
        print("  %-12s %5.1f%%  (n=%d)" % (k, pct(f, n), n))
    print("\nby unit type:")
    for k, (n, f) in sorted(by_type.items(), key=lambda kv: -pct(kv[1][1], kv[1][0])):
        print("  %-10s %5.1f%%  (n=%d)" % (k, pct(f, n), n))
    print("\nmost affected battles:")
    for share, name, opp, side, outcome in sorted(worst, reverse=True)[:8]:
        print("  %4.1f%%  vs %-6s side %d %-8s %s" % (100 * share, opp, side, outcome, name))


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
