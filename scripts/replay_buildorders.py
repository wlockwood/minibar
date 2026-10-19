#!/usr/bin/env python3
"""Extract human build orders from the visigoth replay trajectories.

Input : visigoth per-replay Parquet shards (one row = one player, one 10 s sample;
        see docs/REPLAY_IMITATION_PLAN.md) + the replay .json sidecars + the dataset
        split files (train / validation / test_ood).
Output: one JSON line per player-game with, for each 10 s sample, the player's economy
        and unit counts at the sample and the build decisions issued in the interval.

    python scripts/replay_buildorders.py --out data/replay/buildorders-v1.jsonl
    python scripts/replay_buildorders.py --limit 5            # smoke test, prints summary

A build command is `cmd_id < 0`. Placed structures carry [x, y, z, facing]; a factory
queue entry carries [0]. One multi-unit order is replicated per selected unit by the
extractor, so identical (frame, item, params) records are collapsed (count kept in `n`).
"""
import argparse
import collections
import glob
import json
import os
import sys

import pyarrow.parquet as pq

VISIGOTH = r"D:\code\BAR\visigoth"
SPLITS = ("train", "validation", "test_ood")


def load_splits(dataset):
    out = {}
    for name in SPLITS:
        path = os.path.join(dataset, name + ".parquet")
        if os.path.exists(path):
            for rid in set(pq.read_table(path, columns=["replay_id"]).column("replay_id").to_pylist()):
                out[rid] = name
    return out


def sidecar(replays_dir, shard):
    stem = os.path.splitext(os.path.basename(shard))[0]
    path = os.path.join(replays_dir, stem + ".json")
    return json.load(open(path, encoding="utf-8")) if os.path.exists(path) else {}


def state(obs):
    res = obs.get("resources", {})

    def r(k):
        v = res.get(k, {})
        return {f: round(v.get(f, 0.0), 1) for f in ("level", "storage", "income", "expense")}

    counts = collections.Counter(u["def"] for u in obs.get("units", []) if u.get("build_progress", 1) >= 1)
    building = collections.Counter(u["def"] for u in obs.get("units", []) if u.get("build_progress", 1) < 1)
    return {"metal": r("metal"), "energy": r("energy"), "units": dict(counts), "in_progress": dict(building)}


def collapse(commands):
    """Build commands in the interval, deduped, in frame order."""
    seen = collections.OrderedDict()
    for c in sorted((c for c in commands if c["cmd_id"] < 0), key=lambda c: c["frame"]):
        params = [round(p, 1) for p in c.get("params", [])]
        key = (c["frame"], c["build_def"], tuple(params))
        if key in seen:
            seen[key]["n"] += 1
            seen[key]["builders"].add(c["unit_def"])
            continue
        placed = len(params) >= 3
        seen[key] = {
            "frame": c["frame"],
            "t": round(c["time"], 2),
            "item": c["build_def"],
            "kind": "place" if placed else "factory",
            "pos": [params[0], params[2]] if placed else None,
            "facing": int(params[3]) if len(params) > 3 else None,
            "queued": bool(c["options"]["shift"]),
            "n": 1,
            "builders": {c["unit_def"]},
        }
    out = []
    for e in seen.values():
        e["builders"] = sorted(e["builders"])
        out.append(e)
    return out


def player_games(shard, split_of, meta):
    rows = pq.read_table(shard, columns=["replay_id", "player", "frame", "time", "sample_json"]).to_pylist()
    by_player = collections.defaultdict(list)
    for r in rows:
        by_player[r["player"]].append(r)
    info = {p["name"]: p for p in meta.get("players", [])}
    for player, prows in by_player.items():
        prows.sort(key=lambda r: r["frame"])
        samples = []
        for r in prows:
            s = json.loads(r["sample_json"])
            samples.append({
                "frame": r["frame"],
                "t": round(r["time"], 1),
                "state": state(s["observation"]),
                "builds": collapse(s["commands"]),
            })
        p = info.get(player, {})
        yield {
            "replay_id": prows[0]["replay_id"],
            "file": os.path.basename(shard),
            "split": split_of.get(prows[0]["replay_id"], "unsplit"),
            "player": player,
            "faction": p.get("faction"),
            "skill": p.get("skill"),
            "won": p.get("won"),
            "map": meta.get("map", {}).get("scriptName"),
            "duration": meta.get("duration_seconds"),
            "samples": samples,
        }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--visigoth", default=VISIGOTH)
    ap.add_argument("--dataset", default=None, help="split dataset dir (default trajectories-v2)")
    ap.add_argument("--out", default=None)
    ap.add_argument("--limit", type=int, default=0, help="only the first N shards")
    a = ap.parse_args()
    shards = sorted(glob.glob(os.path.join(a.visigoth, "data", "trajectories", "per-replay", "*.parquet")))
    if a.limit:
        shards = shards[: a.limit]
    dataset = a.dataset or os.path.join(a.visigoth, "data", "trajectories", "datasets", "trajectories-v2")
    split_of = load_splits(dataset)
    replays_dir = os.path.join(a.visigoth, "data", "replays")
    out = None
    if a.out:
        os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
        out = open(a.out, "w", encoding="utf-8")
    games = events = 0
    items = collections.Counter()
    splits = collections.Counter()
    for i, shard in enumerate(shards):
        meta = sidecar(replays_dir, shard)
        for g in player_games(shard, split_of, meta):
            games += 1
            splits[g["split"]] += 1
            for s in g["samples"]:
                for e in s["builds"]:
                    events += 1
                    items[e["item"]] += 1
            if out:
                out.write(json.dumps(g, separators=(",", ":")) + "\n")
        if (i + 1) % 50 == 0:
            print(f"  {i + 1}/{len(shards)} shards", file=sys.stderr)
    if out:
        out.close()
    print(f"{games} player-games from {len(shards)} shards; {events} build events after dedupe; splits {dict(splits)}")
    print("top items:", items.most_common(15))


if __name__ == "__main__":
    main()
