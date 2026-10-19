#!/usr/bin/env python3
"""Compare brains on the same large batch of freshly generated platoon matchups.

Breaks results down by matchup class (underdog / even / overdog by metal), by
what the learner fields (bots, vehicles, both; mixed or pure) and by baseline
opponent, so a change that helps one kind of fight but hurts another shows up.

    python scripts/analyze_matchups.py --config data/brains/t1/base_config.json \
        --catalog examples/t1-catalog.json --exe data/brains/t1/bin/visigoth-simulator.exe \
        --brain old=data/brains/overnight/final/champion.json --brain new=data/brains/t1/champion.json
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from collections import defaultdict
from pathlib import Path


def evaluate(exe, catalog, config_path, brain, seeds, seed_base):
    result = subprocess.run(
        [os.path.abspath(exe), "--catalog", catalog, "--evaluate", brain, "--train-config", config_path,
         "--eval-seeds", str(seeds), "--eval-seed-base", str(seed_base)],
        capture_output=True, text=True, encoding="utf-8")
    if result.returncode != 0:
        sys.exit(result.stderr[-2000:])
    return json.loads(result.stdout)


POWER_CLASS = {}
OBJECTIVE = {}


def classify(scenario):
    """(class, learner domains, learner mixed?, learner platoon types)"""
    match = re.match(r"m\d+ \[(\w+) ([\d.]+)x(?: p([\d.]+))?(?: (atk|def)>(\w+))?\] (.*)", scenario)
    if not match:
        return "fixed", "-", "-", "-"
    cls, _, power, objective, _target, rest = match.groups()
    OBJECTIVE[scenario] = objective or "none"
    if power is not None:
        # Strength by the doctrine's own measure (health x damage per second):
        # a better yardstick than metal when a swarm meets artillery.
        POWER_CLASS[scenario] = "weaker" if float(power) <= 0.67 else "stronger" if float(power) >= 1.5 else "close"
    learner = rest.split(" vs ")[0]
    parts = [p.strip() for p in learner.split(" & ")]
    domains = sorted({p.split()[0] for p in parts})
    mixed = any("-mix" in p for p in parts)
    types = sorted({p.split()[1].split("-")[0] for p in parts})
    return cls, "+".join(domains), "mixed" if mixed else "pure", "+".join(types)


def add(table, key, tally):
    row = table[key]
    for field in ("battles", "wins", "timeouts", "losses", "fitness_sum", "exchange_sum"):
        row[field] += tally.get(field, 0)


def cells(row):
    n = max(1, row["battles"])
    return "%5.0f%% win %4.0f%% timeout %4.0f%% loss  fit %+.2f  exch %+.2f  (n=%d)" % (
        100 * row["wins"] / n, 100 * row["timeouts"] / n, 100 * row["losses"] / n,
        row["fitness_sum"] / n, row["exchange_sum"] / n, row["battles"])


def breakdown(evaluation):
    tables = {name: defaultdict(lambda: defaultdict(float))
              for name in ("class", "domain", "mix", "type", "opponent", "all", "class_opponent", "class_type", "class_domain", "power", "power_opponent", "objective", "objective_power", "objective_opponent")}
    for row in evaluation["rows"]:
        cls, domain, mix, types = classify(row["scenario"])
        t = row["tally"]
        add(tables["class"], cls, t)
        add(tables["domain"], domain, t)
        add(tables["mix"], mix, t)
        for single in types.split("+"):
            add(tables["type"], single, t)
        add(tables["opponent"], row["opponent"], t)
        add(tables["objective"], OBJECTIVE.get(row["scenario"], "none"), t)
        add(tables["objective_opponent"], OBJECTIVE.get(row["scenario"], "none") + "/" + row["opponent"], t)
        if row["scenario"] in POWER_CLASS:
            add(tables["objective_power"], OBJECTIVE.get(row["scenario"], "none") + "/" + POWER_CLASS[row["scenario"]], t)
        pclass = POWER_CLASS.get(row["scenario"])
        if pclass:
            add(tables["power"], pclass, t)
            add(tables["power_opponent"], pclass + "/" + row["opponent"], t)
        add(tables["class_opponent"], cls + "/" + row["opponent"], t)
        add(tables["class_domain"], cls + "/" + domain, t)
        for single in types.split("+"):
            add(tables["class_type"], cls + "/" + single, t)
        add(tables["all"], "all", t)
    return tables


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--config", required=True)
    parser.add_argument("--catalog", required=True)
    parser.add_argument("--exe", required=True)
    parser.add_argument("--brain", action="append", required=True, help="label=path (repeatable)")
    parser.add_argument("--count", type=int, default=60, help="matchups per batch")
    parser.add_argument("--eval-seeds", type=int, default=4)
    parser.add_argument("--seed-base", type=int, default=424242)
    args = parser.parse_args()

    config = json.loads(Path(args.config).read_text(encoding="utf-8"))
    if config.get("matchups"):
        config["matchups"]["count"] = args.count
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "analysis_config.json"
        path.write_text(json.dumps(config), encoding="utf-8")
        results = {}
        for spec in args.brain:
            label, brain = spec.split("=", 1)
            results[label] = breakdown(evaluate(args.exe, args.catalog, str(path), brain, args.eval_seeds, args.seed_base))
    for name in ("all", "class", "domain", "mix", "type", "opponent", "class_opponent", "class_domain", "class_type", "power", "power_opponent", "objective", "objective_power", "objective_opponent"):
        print("\n== by %s" % name)
        keys = sorted({k for r in results.values() for k in r[name]})
        for key in keys:
            for label, tables in results.items():
                if key in tables[name]:
                    print("  %-18s %-8s %s" % (key, label, cells(tables[name][key])))


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
