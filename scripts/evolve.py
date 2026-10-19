#!/usr/bin/env python3
"""Open-ended evolution of a tactical council: train, tweak, select, repeat.

One *round* does this:

  1. Start `--children` training runs from the current champion. Child 0 keeps
     the incumbent hyperparameters as a control. The next `--focus-children`
     train ONLY against the champion's weakest baseline (found from the last
     benchmark), so the loop keeps working on whatever it is currently worst
     at; selection on the full benchmark stops that from forgetting the rest.
     The remaining children get small random tweaks to sigma, learning rate,
     L1, population, seeds per battle, layout jitter, and occasionally train
     only one or two modules.
  2. Score the champion and every child on the SAME fresh seeds and layouts
     (baseline opponents plus the most recent champions).
  3. Promote the best child only if it beats the champion by `--min-gain`.
     The promoted child's hyperparameters become the new incumbent, and the
     champion joins the hall of fame, which later children train against.

The objective, scenarios and baseline opponents come from the base config and
never change, so scores stay comparable across the whole run. Only optimiser
settings are tweaked.

Stop with Ctrl+C (the current round is discarded; the champion is kept) or by
creating a file named STOP in the output directory (stops after the round).
Re-running with the same --out resumes from state.json.
"""

import argparse
import copy
import json
import math
import os
import random
import shutil
import subprocess
import sys
import time
from pathlib import Path

MODULES = ["target_selector", "spacing", "strafe", "cohesion"]
L1_CHOICES = [0.0, 0.05, 0.1, 0.2]
BOUNDS = {
    "sigma": (0.02, 0.4),
    "learning_rate": (0.005, 0.1),
    # Cost knobs are capped: an overnight run adopted population 108, 4 seeds and
    # jitter 70 because selection only sees score, and rounds grew from 5 to 22
    # minutes while quality had plateaued.
    "population": (32, 56),
    "seeds_per_battle": (2, 2),
    "layout_jitter": (0.0, 50.0),
}
KNOBS = ["sigma", "learning_rate", "l1", "population", "seeds_per_battle", "layout_jitter", "focus"]


# --- Pure helpers (unit-tested) ------------------------------------------------


def clamp(value, key):
    low, high = BOUNDS[key]
    return max(low, min(high, value))


def default_hp(base):
    """Incumbent hyperparameters, taken from the base config."""
    es = base.get("es", {})
    return {
        "es": {
            "population": es.get("population", 64),
            "sigma": es.get("sigma", 0.1),
            "learning_rate": es.get("learning_rate", 0.03),
            "l1": es.get("l1", 0.0),
            "weight_decay": es.get("weight_decay", 0.0),
        },
        "seeds_per_battle": base.get("seeds_per_battle", 4),
        "layout_jitter": base.get("layout_jitter", 30.0),
    }


def tweak(hp, rng, scale=0.2, count=2):
    """Small random change to `count` knobs. Returns (new_hp, focus_modules, notes).

    `focus_modules` is a child-only restriction (train just those modules); it
    is never adopted into the incumbent.
    """
    new = copy.deepcopy(hp)
    focus = []
    notes = []
    for knob in rng.sample(KNOBS, count):
        if knob in ("sigma", "learning_rate"):
            before = new["es"][knob]
            new["es"][knob] = round(clamp(before * math.exp(rng.gauss(0, scale)), knob), 5)
            notes.append(f"{knob} {before:g}->{new['es'][knob]:g}")
        elif knob == "l1":
            before = new["es"]["l1"]
            at = min(range(len(L1_CHOICES)), key=lambda i: abs(L1_CHOICES[i] - before))
            to = max(0, min(len(L1_CHOICES) - 1, at + rng.choice([-1, 1])))
            new["es"]["l1"] = L1_CHOICES[to]
            notes.append(f"l1 {before:g}->{new['es']['l1']:g}")
        elif knob == "population":
            before = new["es"]["population"]
            value = int(clamp(before * rng.choice([0.75, 1.5]), knob))
            new["es"]["population"] = value - value % 2
            notes.append(f"population {before}->{new['es']['population']}")
        elif knob == "seeds_per_battle":
            before = new["seeds_per_battle"]
            new["seeds_per_battle"] = int(clamp(round(before * rng.choice([0.5, 2])), knob))
            notes.append(f"seeds_per_battle {before}->{new['seeds_per_battle']}")
        elif knob == "layout_jitter":
            before = new["layout_jitter"]
            new["layout_jitter"] = clamp(before + rng.choice([-10.0, 10.0]), knob)
            notes.append(f"layout_jitter {before:g}->{new['layout_jitter']:g}")
        else:
            focus = rng.sample(MODULES, rng.choice([1, 2]))
            notes.append("train only " + "+".join(focus))
    return new, focus, notes


def mean_fitness(evaluation):
    """The score candidates are selected on: the robust fitness (mean blended with
    the worst quarter of scenario/opponent rows) when the simulator reports it,
    else the plain mean over all battles."""
    if "robust_fitness" in evaluation:
        return evaluation["robust_fitness"]
    total = evaluation["total"]
    return total["fitness_sum"] / max(1, total["battles"])


def baseline_summary(evaluation, baselines):
    """Win/timeout/loss against the fixed baselines only, plus underdog win rate."""
    wins = timeouts = losses = battles = 0
    for row in evaluation["rows"]:
        if row["opponent"] in baselines:
            t = row["tally"]
            wins += t["wins"]
            timeouts += t.get("timeouts", 0)
            losses += t["losses"]
            battles += t["battles"]
    total = evaluation["total"]
    underdog = total["underdog_wins"] / total["underdog_battles"] if total["underdog_battles"] else None
    return {
        "baseline_win": wins / battles if battles else 0.0,
        "baseline_timeout": timeouts / battles if battles else 0.0,
        "baseline_loss": losses / battles if battles else 0.0,
        "underdog_win": underdog,
    }


def by_opponent(evaluation, baselines):
    """Win/timeout/loss rates and mean fitness against each fixed baseline."""
    totals = {}
    for row in evaluation["rows"]:
        label = row["opponent"]
        if label not in baselines:
            continue
        t = row["tally"]
        a = totals.setdefault(label, {"battles": 0, "wins": 0, "timeouts": 0, "losses": 0, "fitness": 0.0})
        a["battles"] += t["battles"]
        a["wins"] += t["wins"]
        a["timeouts"] += t.get("timeouts", 0)
        a["losses"] += t["losses"] + t.get("draws", 0)
        a["fitness"] += t["fitness_sum"]
    return {
        label: {
            "win": a["wins"] / a["battles"],
            "timeout": a["timeouts"] / a["battles"],
            "loss": a["losses"] / a["battles"],
            "fitness": a["fitness"] / a["battles"],
        }
        for label, a in totals.items()
        if a["battles"]
    }


def weakest_baseline(evaluation, baselines):
    """The baseline this candidate scores worst against, or None if there is
    nothing to prioritise (fewer than two baselines)."""
    per = by_opponent(evaluation, baselines)
    if len(per) < 2:
        return None
    ranked = sorted(per, key=lambda k: (per[k]["fitness"], k))
    label = ranked[0]
    return {"opponent": label, "fitness": per[label]["fitness"], "ranked": ranked}


def training_work(log_paths):
    """Battles simulated and battle time covered, summed over children's logs."""
    battles, sim_seconds = 0, 0.0
    for path in log_paths:
        try:
            lines = Path(path).read_text(encoding="utf-8").splitlines()
        except FileNotFoundError:
            continue
        for line in lines:
            try:
                record = json.loads(line)
            except ValueError:
                continue
            battles += record.get("battles", 0)
            sim_seconds += record.get("sim_seconds", 0.0)
    return {"battles": battles, "sim_seconds": sim_seconds}


def choose_winner(scores, champion_name, min_gain):
    """`scores` maps candidate name -> benchmark fitness. Returns (winner, promoted).

    A child replaces the champion only by beating it by `min_gain`. With no
    champion yet, the best child wins outright.
    """
    if champion_name is None:
        return max(scores, key=scores.get), True
    best_child = max((n for n in scores if n != champion_name), key=scores.get, default=None)
    if best_child is not None and scores[best_child] > scores[champion_name] + min_gain:
        return best_child, True
    return champion_name, False


# --- Process plumbing -----------------------------------------------------------


class Runner:
    def __init__(self, exe, catalog, unit, stop_path=None, abort_path=None):
        self.exe, self.catalog, self.unit = exe, catalog, unit
        self.stop_path, self.abort_path = stop_path, abort_path
        self.announced = False
        self.running = []

    def spawn(self, args, stdout_path, stderr_path, threads):
        env = dict(os.environ, RAYON_NUM_THREADS=str(max(1, threads)))
        cmd = [self.exe, "--catalog", self.catalog, "--unit", self.unit, *map(str, args)]
        process = subprocess.Popen(
            cmd, stdout=open(stdout_path, "w"), stderr=open(stderr_path, "w"), env=env
        )
        self.running.append(process)
        return process

    def wait(self, processes):
        try:
            while any(p.poll() is None for p in processes):
                if self.abort_path and Path(self.abort_path).exists():
                    print("STOP_NOW found: abandoning the round in progress.")
                    raise KeyboardInterrupt
                if self.stop_path and not self.announced and Path(self.stop_path).exists():
                    self.announced = True
                    print(f"STOP found: finishing this round first (create {Path(self.abort_path).name} "
                          "to abandon it now).")
                    sys.stdout.flush()
                time.sleep(1.0)
        except BaseException:
            self.kill()
            raise
        finally:
            self.running = [p for p in self.running if p.poll() is None]

    def kill(self):
        for process in self.running:
            if process.poll() is None:
                process.terminate()
        for process in self.running:
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
        self.running = []


def write_json(path, value):
    tmp = Path(str(path) + ".tmp")
    tmp.write_text(json.dumps(value, indent=2), encoding="utf-8")
    tmp.replace(path)


def atomic_copy(source, destination):
    """Copy so that a reader (the watch server) never sees a half-written file."""
    tmp = Path(str(destination) + ".tmp")
    shutil.copyfile(source, tmp)
    tmp.replace(destination)


def read_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


# --- The loop -------------------------------------------------------------------


def stable_seed(*parts):
    return random.Random("-".join(map(str, parts))).randrange(1, 2**31)


def run_round(args, runner, out, state, base):
    r = state["round"]
    started = time.time()
    round_dir = out / "rounds" / f"r{r:04d}"
    round_dir.mkdir(parents=True, exist_ok=True)
    rng = random.Random(stable_seed(args.seed, "tweak", r))
    baselines = [o for o in base["opponents"] if not o.startswith("brain:")]
    hof = state["hall_of_fame"]
    champion = state["champion"]

    start_brain = None
    if champion:
        start_brain = round_dir / "start.json"
        shutil.copyfile(champion["path"], start_brain)

    # 1. Children ---------------------------------------------------------------
    children = []
    weakest = state.get("weakest")
    if weakest and weakest["opponent"] not in baselines:
        weakest = None  # the base config changed since it was recorded
    # Alternate between the two weakest baselines: the single weakest is often an
    # even matchup with a natural ceiling (a 1v1 against `charge` is ~50%), and a
    # night of focusing on it alone never produced a promotion.
    ranked = [label for label in (weakest or {}).get("ranked", []) if label in baselines]
    focus_label = ranked[r % min(2, len(ranked))] if ranked else (weakest or {}).get("opponent")
    for i in range(args.children):
        only = None  # restrict the baselines this child trains against
        if i == 0:
            hp, focus, notes = copy.deepcopy(state["hp"]), [], ["control (incumbent settings)"]
        elif weakest and i <= args.focus_children:
            hp, focus = copy.deepcopy(state["hp"]), []
            only = [focus_label]
            notes = [f"focus: train only vs {focus_label} (alternating between the two weakest baselines)"]
        else:
            hp, focus, notes = tweak(state["hp"], rng, args.tweak_scale)
        cfg = copy.deepcopy(base)
        cfg["name"] = f"{base.get('name', 'tactical')}-r{r}-c{i}"
        cfg["generations"] = args.generations_per_round
        cfg["es"] = {**base.get("es", {}), **hp["es"], "seed": stable_seed(args.seed, r, i)}
        cfg["seeds_per_battle"] = hp["seeds_per_battle"]
        cfg["layout_jitter"] = hp["layout_jitter"]
        cfg["train_modules"] = focus
        cfg["validation_every"] = max(5, args.generations_per_round // 4)
        cfg["init_from"] = str(start_brain.resolve()) if start_brain else None
        opponents = list(only or baselines)
        pool = hof[-args.hof_pool:]
        for path in rng.sample(pool, min(args.hof_opponents, len(pool))):
            opponents.append(f"brain:{Path(path).resolve()}")
        cfg["opponents"] = opponents
        child_dir = round_dir / f"c{i}"
        child_dir.mkdir(exist_ok=True)
        write_json(child_dir / "config.json", cfg)
        children.append({"name": f"c{i}", "dir": child_dir, "hp": hp, "notes": notes, "cfg": cfg})

    threads = max(1, (os.cpu_count() or 4) // args.children)
    procs = []
    for child in children:
        procs.append(
            runner.spawn(
                ["--train", child["dir"] / "config.json", "--train-out", child["dir"] / "out"],
                child["dir"] / "summary.stdout.json",
                child["dir"] / "stderr.log",
                threads,
            )
        )
    runner.wait(procs)
    alive = []
    for child, process in zip(children, procs):
        best = child["dir"] / "out" / "brain.best.json"
        if process.returncode == 0 and best.exists():
            child["brain"] = best
            alive.append(child)
        else:
            print(f"  ! {child['name']} failed (see {child['dir'] / 'stderr.log'})")
    if not alive:
        raise RuntimeError("every child failed; see the stderr logs under " + str(round_dir))

    # 2. Benchmark on shared fresh seeds -----------------------------------------
    bench = copy.deepcopy(base)
    bench["opponents"] = baselines + [f"brain:{Path(p).resolve()}" for p in hof[-args.hof_benchmark:]]
    bench["layout_jitter"] = base.get("layout_jitter", 30.0)
    if args.eval_matchups and bench.get("matchups"):
        # Generated fights vary far more than shot noise: a bigger batch of
        # different matchups steadies the promotion decision more than seeds do.
        bench["matchups"]["count"] = args.eval_matchups
    bench_path = round_dir / "benchmark_config.json"
    write_json(bench_path, bench)
    seed_base = 7_000_000 + r * 1000

    candidates = [(c["name"], c["brain"]) for c in alive]
    if champion:
        candidates.insert(0, ("champion", Path(champion["path"])))
    eval_threads = max(1, (os.cpu_count() or 4) // len(candidates))
    eprocs = []
    for name, brain in candidates:
        eprocs.append(
            runner.spawn(
                ["--evaluate", Path(brain).resolve(), "--train-config", bench_path,
                 "--eval-seeds", args.eval_seeds, "--eval-seed-base", seed_base],
                round_dir / f"eval-{name}.json",
                round_dir / f"eval-{name}.stderr.log",
                eval_threads,
            )
        )
    runner.wait(eprocs)
    evals = {}
    for (name, _), process in zip(candidates, eprocs):
        if process.returncode == 0:
            evals[name] = read_json(round_dir / f"eval-{name}.json")
    scores = {name: mean_fitness(ev) for name, ev in evals.items()}
    if not scores:
        raise RuntimeError("benchmark evaluation failed for every candidate")

    # 3. Select ---------------------------------------------------------------------
    champion_name = "champion" if champion and "champion" in scores else None
    winner, promoted = choose_winner(scores, champion_name, args.min_gain)
    champion_before = scores.get("champion")
    if promoted:
        child = next(c for c in alive if c["name"] == winner)
        champion_dir = out / "hall_of_fame"
        champion_dir.mkdir(exist_ok=True)
        saved = champion_dir / f"champion-r{r:04d}.json"
        atomic_copy(child["brain"], saved)
        atomic_copy(child["brain"], out / "champion.json")
        state["champion"] = {"path": str(saved), "round": r, "score": scores[winner]}
        state["hall_of_fame"].append(str(saved))
        state["hp"] = copy.deepcopy(child["hp"])
    elif champion:
        state["champion"]["score"] = champion_before

    # Where is the best candidate weakest? The next round's focus child works on it.
    state["weakest"] = weakest_baseline(evals[winner], baselines)

    # 4. Record -----------------------------------------------------------------------
    elapsed = time.time() - started
    summary_for = lambda n: baseline_summary(evals[n], baselines)
    record = {
        "round": r,
        "seconds": round(elapsed, 1),
        "winner": winner,
        "promoted": promoted,
        "champion_score_before": champion_before,
        "candidates": {
            n: {
                "score": scores[n],
                **summary_for(n),
                "notes": next((c["notes"] for c in alive if c["name"] == n), []),
                "hp": next((c["hp"] for c in alive if c["name"] == n), None),
            }
            for n in scores
        },
        "incumbent_hp": state["hp"],
        "by_opponent": by_opponent(evals[winner], baselines),
        "weakest": state["weakest"],
        "work": {
            **training_work([c["dir"] / "out" / "training_log.jsonl" for c in children]),
            "wall_seconds": elapsed,
        },
    }
    with open(out / "history.jsonl", "a") as log:
        log.write(json.dumps(record) + "\n")
    state["round"] = r + 1
    write_json(out / "state.json", state)
    print_round(record)


def print_round(record):
    r, w = record["round"], record["winner"]
    head = f"round {r:>4} ({record['seconds'] / 60:.1f} min)"
    verdict = f"PROMOTED {w}" if record["promoted"] else "champion kept"
    print(f"{head}: {verdict}")
    for name, c in record["candidates"].items():
        under = "  n/a" if c["underdog_win"] is None else f"{c['underdog_win'] * 100:>4.0f}%"
        note = "; ".join(c["notes"])
        mark = " <-" if name == w else ""
        print(
            f"    {name:<9} score {c['score']:+.3f}  win {c['baseline_win'] * 100:>3.0f}% "
            f"timeout {c['baseline_timeout'] * 100:>3.0f}% loss {c['baseline_loss'] * 100:>3.0f}% "
            f"underdog {under}  {note}{mark}"
        )
    work = record.get("work")
    if work and work["battles"]:
        hours = work["sim_seconds"] / 3600
        multiple = work["sim_seconds"] / max(work["wall_seconds"], 1e-9)
        print(
            f"    simulated {work['battles'] / 1e6:.2f}M training battles = {hours:,.0f} hours of fighting "
            f"({hours / 8760:.1f} years) in {work['wall_seconds'] / 60:.1f} min = {multiple:,.0f}x realtime"
        )
    per = record.get("by_opponent") or {}
    if per:
        line = "  ".join(f"{label} {v['win'] * 100:.0f}%" for label, v in per.items())
        weak = record.get("weakest")
        print(f"    best vs (win rate): {line}" + (f"   -> next focus: {weak['opponent']}" if weak else ""))
    sys.stdout.flush()


def digest(out):
    """A plain summary of a run from its history file, for the final snapshot."""
    path = Path(out) / "history.jsonl"
    records = []
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            try:
                records.append(json.loads(line))
            except ValueError:
                continue
    promotions = [
        {"round": r["round"], "winner": r["winner"], "score": r["candidates"][r["winner"]]["score"]}
        for r in records
        if r.get("promoted")
    ]
    work = {"battles": 0, "sim_seconds": 0.0, "wall_seconds": 0.0}
    for r in records:
        for key in work:
            work[key] += (r.get("work") or {}).get(key, 0)
    last = records[-1] if records else {}
    return {
        "rounds": len(records),
        "promotions": promotions,
        "last_by_opponent": last.get("by_opponent"),
        "last_weakest": last.get("weakest"),
        "work": work,
    }


def finalize(runner, out, state):
    """Snapshot what was learned into `<out>/final/`: the champion, a readable
    explanation of it, a standalone viewer file and a summary. Safe to run after
    any stop, and again later with --finalize-only."""
    champion = state.get("champion")
    if not champion or not Path(champion["path"]).exists():
        print("no champion was promoted yet, so there is nothing to save.")
        return
    final = Path(out) / "final"
    final.mkdir(exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    atomic_copy(champion["path"], final / f"champion-{stamp}.json")
    atomic_copy(champion["path"], final / "champion.json")
    summary = {"saved": stamp, "champion": champion, **digest(out)}
    write_json(final / "summary.json", summary)
    config = Path(out) / "base_config.json"
    brain = (final / "champion.json").resolve()
    jobs = [
        runner.spawn(["--explain", brain, "--train-config", config, "--explain-out", final / "report.md"],
                     final / "explain.stdout.log", final / "explain.stderr.log", 4),
        runner.spawn(["--watch", brain, "--train-config", config, "--watch-out", final / "watch.html"],
                     final / "watch.stdout.log", final / "watch.stderr.log", 4),
    ]
    runner.wait(jobs)
    work = summary["work"]
    print(f"saved to {final}:")
    print(f"  champion.json   the best council found (round {champion['round']}, score {(champion.get('score') or 0):+.3f})")
    print(f"  report.md       what each module does, measured    ({'ok' if jobs[0].returncode == 0 else 'FAILED'})")
    print(f"  watch.html      open in a browser to replay it      ({'ok' if jobs[1].returncode == 0 else 'FAILED'})")
    print(f"  summary.json    {summary['rounds']} rounds, {len(summary['promotions'])} promotions, "
          f"{work['sim_seconds'] / 3600:,.0f} simulated hours of fighting")


class Tee:
    """Mirror everything printed into a log file, so a night's progress can be read later."""

    def __init__(self, stream, path):
        self.stream = stream
        self.file = open(path, "a", buffering=1, encoding="utf-8")

    def write(self, text):
        self.stream.write(text)
        self.file.write(text)

    def flush(self):
        self.stream.flush()
        self.file.flush()


def keep_awake(on):
    """Ask Windows not to sleep while training (the display may still turn off)."""
    if os.name != "nt":
        return
    import ctypes

    continuous, system_required = 0x80000000, 0x00000001
    ctypes.windll.kernel32.SetThreadExecutionState(continuous | (system_required if on else 0))


def load_state(out, args, base):
    path = out / "state.json"
    if path.exists():
        print(f"resuming from {path}")
        return read_json(path)
    return {"round": 0, "hp": default_hp(base), "champion": None, "hall_of_fame": []}


def main():
    here = Path(__file__).resolve().parent.parent
    exe_default = here / "target" / "release" / ("visigoth-simulator.exe" if os.name == "nt" else "visigoth-simulator")
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--catalog", required=True, help="unit catalog JSON (e.g. examples/pawn-hello.json)")
    parser.add_argument("--unit", default="armpw")
    parser.add_argument("--config", default=str(here / "examples" / "train-tactical.json"), help="base training config")
    parser.add_argument("--out", help="output directory (default data/brains/evolve-<name>)")
    parser.add_argument("--exe", default=str(exe_default))
    parser.add_argument("--children", type=int, default=4, help="parallel training runs per round")
    parser.add_argument("--focus-children", type=int, default=1,
                        help="children (after the control) that train only against the weakest baseline")
    parser.add_argument("--seed-brain",
                        help="start from this brain: it becomes the first champion, so children must beat it")
    parser.add_argument("--finalize-only", action="store_true",
                        help="just write <out>/final/ from the saved state (use after a hard kill) and exit")
    parser.add_argument("--max-failures", type=int, default=5,
                        help="give up after this many rounds in a row fail outright")
    parser.add_argument("--generations-per-round", type=int, default=40)
    parser.add_argument("--eval-seeds", type=int, default=48, help="benchmark seeds per battle")
    parser.add_argument("--eval-matchups", type=int, default=0,
                        help="with generated matchups: fights per benchmark batch (default: the config's count)")
    parser.add_argument("--min-gain", type=float, default=0.005, help="score a child must beat the champion by")
    parser.add_argument("--hof-opponents", type=int, default=2, help="past champions each child trains against")
    parser.add_argument("--hof-pool", type=int, default=8, help="how many recent champions are sampled from")
    parser.add_argument("--hof-benchmark", type=int, default=2, help="recent champions included in the benchmark")
    parser.add_argument("--tweak-scale", type=float, default=0.2)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--rounds", type=int, help="stop after this many rounds in total")
    parser.add_argument("--max-hours", type=float, help="stop starting new rounds after this long")
    parser.add_argument("--watch-port", type=int, help="also serve the council viewer on this port (follows the champion)")
    parser.add_argument("--no-browser", action="store_true", help="with --watch-port: do not open a browser tab")
    args = parser.parse_args()

    if not Path(args.exe).exists():
        sys.exit(f"simulator binary not found at {args.exe}; run `cargo build --release` or pass --exe")
    base = read_json(args.config)
    out = Path(args.out or (here / "data" / "brains" / f"evolve-{base.get('name', 'tactical')}"))
    out.mkdir(parents=True, exist_ok=True)
    if args.focus_children >= args.children:
        sys.exit("--focus-children must be smaller than --children (child 0 is the control)")
    sys.stdout = Tee(sys.stdout, out / "evolve.log")
    if not (out / "base_config.json").exists():
        write_json(out / "base_config.json", base)
    base = read_json(out / "base_config.json")  # a resumed run keeps its original config
    state = load_state(out, args, base)
    runner = Runner(args.exe, args.catalog, args.unit, out / "STOP", out / "STOP_NOW")
    if (out / "STOP_NOW").exists():
        sys.exit(f"{out / 'STOP_NOW'} exists from an earlier stop; delete it to run.")
    if args.seed_brain and not state["champion"] and not args.finalize_only:
        seed = Path(args.seed_brain)
        if not seed.exists():
            sys.exit(f"--seed-brain {seed} not found")
        (out / "hall_of_fame").mkdir(exist_ok=True)
        saved = out / "hall_of_fame" / "champion-seed.json"
        atomic_copy(seed, saved)
        atomic_copy(seed, out / "champion.json")
        state["champion"] = {"path": str(saved), "round": -1, "score": None}
        state["hall_of_fame"].append(str(saved))
        write_json(out / "state.json", state)
        print(f"seeded from {seed}: it is the first champion and also a hall-of-fame opponent")
    if args.finalize_only:
        finalize(runner, out, state)
        return
    deadline = time.time() + args.max_hours * 3600 if args.max_hours else None

    print(f"evolving into {out}; Ctrl+C or create {out / 'STOP'} to stop; log: {out / 'evolve.log'}")
    keep_awake(True)
    viewer = None
    if args.watch_port:
        viewer = subprocess.Popen([sys.executable, str(Path(__file__).with_name("watch.py")), "--catalog", args.catalog,
                                   "--unit", args.unit, "--exe", args.exe, "--dir", str(out), "--port", str(args.watch_port),
                                   *(["--no-browser"] if args.no_browser else [])])
    else:
        print(f"watch it:  python scripts/watch.py --catalog {args.catalog} --dir {out}")
    failures = 0
    try:
        while True:
            if (out / "STOP").exists():
                print("STOP file found; finishing.")
                break
            if args.rounds is not None and state["round"] >= args.rounds:
                print(f"reached {args.rounds} rounds; finishing.")
                break
            if deadline and time.time() > deadline:
                print("time limit reached; finishing.")
                break
            try:
                run_round(args, runner, out, state, base)
                failures = 0
            except RuntimeError as error:
                failures += 1
                print(f"round failed ({failures}/{args.max_failures}): {error}")
                if failures >= args.max_failures:
                    print("too many failed rounds in a row; stopping.")
                    break
                time.sleep(30)
    except KeyboardInterrupt:
        runner.kill()
        print("\ninterrupted; the unfinished round was discarded.")
    finally:
        # The viewer is a companion of this run: it exits with it. To keep
        # watching the final champion, start scripts/watch.py on its own.
        keep_awake(False)
        if viewer:
            viewer.terminate()
        try:
            finalize(runner, out, state)
        except BaseException as error:  # never lose the exit path to a snapshot problem
            print(f"could not write the final snapshot ({error}); rerun with --finalize-only")
    if state["champion"]:
        print(f"champion: {out / 'champion.json'} (round {state['champion']['round']})")
        print(f"read it:  --explain {out / 'champion.json'} --train-config {out / 'base_config.json'}")


if __name__ == "__main__":
    main()
