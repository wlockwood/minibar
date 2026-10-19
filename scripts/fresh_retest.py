"""Paired fresh-seed retest of a tactical campaign's start, champion and island brains.

The campaign's fixed validation set is reused every time champions are compared, so its
scores carry selection luck. This tool re-scores the starting brain, the champion and each
island's latest brain on many *new* seed bases (paired: every brain sees the same bases) and
on three scenario families:

  training    the scenarios the optimizer fits (fresh seeds per generation, same scenario files)
  validation  the scenarios champions are compared on
  holdout     separate scenarios that never select anything

It writes fresh-retest.<track>.json beside campaign.json (a new file; run/ is not touched), which
scripts/campaign_dashboard.py picks up automatically.

    python scripts/fresh_retest.py data/branches/<branch> --seeds 150
"""
import argparse
import concurrent.futures
import json
import math
import os
import statistics
import subprocess
import sys

FAMILIES = (
    ("training", "training scenarios"),
    ("validation", "validation scenarios"),
    ("holdout", "unselected holdout"),
)
DEFAULT_SEED_BASE = 7_000_000_000_000
BASE_STEP = 10_000_000          # between paired seed bases; far above the layouts*seeds a plan consumes
FAMILY_STRIDE = 1_000_000_000_000
T_CAP = 999.0                   # identical differences give zero spread; report a capped t instead of infinity


def paired_stats(baseline, candidate):
    """Mean difference candidate - baseline over paired samples, with standard error and t."""
    if len(baseline) != len(candidate) or len(baseline) < 2:
        raise ValueError("paired samples must have the same length of at least 2")
    diffs = [c - b for b, c in zip(baseline, candidate)]
    mean = statistics.fmean(diffs)
    se = statistics.stdev(diffs) / math.sqrt(len(diffs))
    t = mean / se if se > 0 else (0.0 if mean == 0 else math.copysign(T_CAP, mean))
    return {"mean": statistics.fmean(candidate), "diff": mean, "se": se,
            "t": max(-T_CAP, min(T_CAP, t)), "n": len(diffs)}


def brain_paths(root, track):
    base = os.path.join(root, "run", "tracks", track)
    found = {"start": os.path.join(base, "brain.initial.json"), "champion": os.path.join(base, "brain.best.json")}
    for name in sorted(os.listdir(base)) if os.path.isdir(base) else []:
        latest = os.path.join(base, name, "brain.latest.json")
        if os.path.isfile(latest):
            found[name] = latest
    for name in ("start", "champion"):
        if not os.path.isfile(found[name]):
            raise SystemExit(f"missing {found[name]}")
    return found


def evaluate(exe, brain, config, track, plan, base, battles_per_layout=None):
    command = [exe, "--evaluate-campaign", brain, "--campaign-config", config, "--campaign-track", track,
               "--campaign-plan", plan, "--eval-seed-base", str(base)]
    if battles_per_layout:
        command += ["--eval-seeds", str(battles_per_layout)]
    done = subprocess.run(command, capture_output=True, text=True)
    if done.returncode != 0:
        raise RuntimeError(f"evaluation failed ({done.returncode}): {done.stderr.strip()[-400:]}")
    return json.loads(done.stdout)["fitness"]


def run(root, track, seeds, exe, workers, seed_base, families, holdout_seeds=None):
    config_path = os.path.join(root, "campaign.json")
    with open(config_path, encoding="utf-8") as handle:
        config = json.load(handle)
    track_config = next((t for t in config.get("tracks", []) if t.get("name") == track), None)
    if track_config is None:
        raise SystemExit(f"track {track!r} is not in {config_path}")
    brains = brain_paths(root, track)
    temporary = os.path.join(root, f".retest-training.{os.getpid()}.json")
    result = {"format": "minibar.fresh_retest", "version": 1, "track": track, "seeds": seeds,
              "seed_base": seed_base, "baseline": "start", "families": []}
    try:
        for index, (family, label) in enumerate(FAMILIES):
            if family not in families:
                continue
            if family == "training":
                # --evaluate-campaign only accepts validation or holdout plans, so evaluate the
                # training plan as a copy of the config whose validation plan is the training plan.
                swapped = json.loads(json.dumps(config))
                for t in swapped["tracks"]:
                    if t.get("name") == track:
                        t["validation"] = t["training"]
                with open(temporary, "w", encoding="utf-8") as handle:
                    json.dump(swapped, handle)
                cfg, plan = temporary, "validation"
            elif family == "holdout":
                if not track_config.get("holdout"):
                    continue
                cfg, plan = config_path, "holdout"
                # the holdout plan is small (few battles per evaluation), so it is the noisiest family
            else:
                cfg, plan = config_path, "validation"
            jobs = [(name, k) for k in range(seeds) for name in brains]
            scores = {}
            with concurrent.futures.ThreadPoolExecutor(workers) as pool:
                futures = {pool.submit(evaluate, exe, brains[name], cfg, track, plan,
                                       seed_base + index * FAMILY_STRIDE + k * BASE_STEP,
                                       holdout_seeds if family == "holdout" else None): (name, k) for name, k in jobs}
                for future in concurrent.futures.as_completed(futures):
                    scores[futures[future]] = future.result()
            baseline = [scores[("start", k)] for k in range(seeds)]
            rows = []
            for name in brains:
                if name == "start":
                    continue
                rows.append({"name": name, **paired_stats(baseline, [scores[(name, k)] for k in range(seeds)])})
            result["families"].append({"name": family, "label": label, "n": seeds,
                                       "baseline": statistics.fmean(baseline), "brains": rows})
            print(f"{family}: start {statistics.fmean(baseline):.3f}; " + "; ".join(
                f"{r['name']} {r['diff']:+.3f} (t {r['t']:+.1f})" for r in rows), flush=True)
    finally:
        if os.path.exists(temporary):
            os.remove(temporary)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("campaign", help="campaign root (the folder holding campaign.json and run/)")
    parser.add_argument("--track", default="tactical")
    parser.add_argument("--seeds", type=int, default=150, help="paired fresh seed bases per scenario family")
    parser.add_argument("--exe", help="simulator that produced the run (default: <campaign>/bin/visigoth-simulator.exe)")
    parser.add_argument("--threads", type=int, default=os.cpu_count() or 4)
    parser.add_argument("--seed-base", type=int, default=DEFAULT_SEED_BASE,
                        help="first seed base; keep it outside the campaign's own seed bands")
    parser.add_argument("--holdout-seeds", type=int, default=6,
                        help="battles per layout in each holdout evaluation (the plan's own value is smaller and noisier)")
    parser.add_argument("--families", default=",".join(f for f, _ in FAMILIES))
    parser.add_argument("--out", help="default: <campaign>/fresh-retest.<track>.json")
    args = parser.parse_args()
    root = os.path.abspath(args.campaign)
    exe = os.path.abspath(args.exe or os.path.join(root, "bin", "visigoth-simulator.exe"))
    if not os.path.isfile(exe):
        raise SystemExit(f"simulator not found: {exe} (pass --exe with the build that produced the run)")
    if args.seeds < 2:
        raise SystemExit("--seeds must be at least 2")
    families = {f.strip() for f in args.families.split(",") if f.strip()}
    unknown = families - {f for f, _ in FAMILIES}
    if unknown:
        raise SystemExit(f"unknown families: {sorted(unknown)}")
    result = run(root, args.track, args.seeds, exe, max(1, args.threads), args.seed_base, families, args.holdout_seeds)
    out = args.out or os.path.join(root, f"fresh-retest.{args.track}.json")
    temporary = out + ".tmp"
    with open(temporary, "w", encoding="utf-8") as handle:
        json.dump(result, handle, indent=2)
    os.replace(temporary, out)
    print(f"wrote {out}")


if __name__ == "__main__":
    sys.exit(main())
