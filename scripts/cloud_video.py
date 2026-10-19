#!/usr/bin/env python3
"""Render a training run as a "cloud" video: many battles of one checkpoint
played on top of each other, one scene per checkpoint.

Every scene replays the same scenario and opponent for N seeds with one brain.
Units are drawn as translucent blobs in a shared frame, so the middle of the
cloud is what the brain usually does and its width is how much that varies.
Scenes follow the training generations, so the trend shows up as the cloud
moving, tightening, or turning from red-heavy to blue-heavy.

Corner labels on every scene:
  upper left    the checkpoint ("Generation 5")
  lower left    fitness range (of the battles on screen; and, when the run's
                history.jsonl has it, of that generation's ES population)
  lower right   simulated seconds elapsed (this scene, and the whole video)
  upper right   outcome tally of the battles on screen
  left / right  the brain's units (blue) and the opponent's (red), one line per
                unit type ("40x Pawns"), most valuable line first (metal + energy / 60)

Two modes:
  scenes (default)   one scene per checkpoint; --samples battles of it share one clock.
  progressive        evolution as a stream: the top --top runs of every generation are queued
                     generation by generation, and every --interval seconds the next --per-second
                     runs leave the front of the queue and start playing, each on its own clock.
                     A finished run fades away, so the cloud always holds recent runs and slides
                     from early generations to late ones. Corner readouts then describe what is
                     on screen right now: the upper left names the newest generation started (and
                     the span still visible), the fitness range and outcomes cover the runs
                     playing, the clock counts simulated time since the video began, and the lower
                     right counts runs started. --select random shows typical runs, not the best.

Battles are played with the simulator's own `--watch-battle` mode (about 40 ms
each), cached on disk, so re-rendering with different looks costs no simulation.
Seeds default to small integers, far from the bands training and validation use,
so the cloud shows fresh battles and not the ones the optimizer fitted.

Campaign run (checkpoints are the candidates/<track>/generation-N-<island>-<kind>.json
files, plus the track's initial brain as generation 0):

  python scripts/cloud_video.py \\
      --run data/branches/NAME/runs/production --track ground --island local \\
      --catalog data/branches/NAME/catalog/catalog.json \\
      --train-config data/branches/NAME/stages/production/ground-heavy_swarm.json \\
      --out cloud.mp4

Any brains (evolve.py hall of fame, hand-picked files):

  python scripts/cloud_video.py --brains a.json b.json c.json --catalog C --train-config T --out cloud.mp4

`--list` prints the scenarios and opponents the config offers (pick one with
--scenario / --opponent). `--stills DIR` writes one PNG per scene to check the
look quickly; add `--stills-only` to skip encoding. Needs ffmpeg on PATH (or --ffmpeg).
"""

import argparse
import contextlib
import hashlib
import json
import math
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
import time
from bisect import bisect_left, bisect_right
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path
from types import SimpleNamespace

import numpy as np
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent.parent

LEARNER_RGB = (111, 177, 255)  # same colours as viewer/brain_watch.html
ENEMY_RGB = (255, 93, 93)
BACKGROUND_RGB = (15, 18, 23)
GRID_RGB = (23, 27, 34)
TEXT_RGB = (236, 240, 246)
DIM_TEXT_RGB = (150, 160, 175)
OUTCOMES = ("win", "timeout", "loss", "wipe-out")
CACHE_FORMAT = 2  # bump when what a cached battle holds changes (2: unit names)


# --------------------------------------------------------------------------------------
# Checkpoints
# --------------------------------------------------------------------------------------


@dataclass
class Checkpoint:
    label: str
    path: Path
    generation: int | None = None
    # (lowest, highest, mean, count) of the ES population that generation, if recorded.
    population: tuple | None = None

    def load_brain(self):
        """The brain object, whether the file is a bare brain or a campaign receipt."""
        data = json.loads(self.path.read_text(encoding="utf-8"))
        if isinstance(data, dict) and isinstance(data.get("brain"), dict) and "modules" not in data:
            data = data["brain"]
        if not isinstance(data, dict) or "modules" not in data:
            raise ValueError(f"{self.path}: not a brain file (no 'modules')")
        return data


def generation_label(generation):
    return "Generation 0 (start)" if generation == 0 else f"Generation {generation}"


def discover_run(run_dir, track, island=None, kind="center", include_initial=True):
    """Checkpoints of a campaign run directory, oldest first."""
    run_dir = Path(run_dir)
    folder = run_dir / "candidates" / track
    pattern = re.compile(r"^generation-(\d+)-(.+)-([A-Za-z_]+)\.json$")
    found = {}
    islands = set()
    for path in sorted(folder.glob("generation-*.json")) if folder.is_dir() else []:
        match = pattern.match(path.name)
        if not match:
            continue
        # The kind is the text after the last '-': center, sample or module_<name>.
        gen, name, tail = int(match.group(1)), match.group(2), match.group(3)
        islands.add(name)
        if (island is None or name == island) and tail == kind:
            found.setdefault(name, {})[gen] = path
    if island is None:
        if len(found) > 1:
            raise ValueError(f"several islands have '{kind}' checkpoints: {sorted(found)}; pass --island")
        island = next(iter(found), None)
    if island is None:
        raise ValueError(
            f"no 'generation-N-<island>-{kind}.json' files under {folder} "
            f"(islands seen: {sorted(islands) or 'none'})"
        )
    checkpoints = [
        Checkpoint(generation_label(gen), path, gen) for gen, path in sorted(found.get(island, {}).items())
    ]
    if not checkpoints:
        raise ValueError(
            f"island '{island}' has no '{kind}' checkpoints under {folder} (islands seen: {sorted(islands)})"
        )
    initial = run_dir / "tracks" / track / "brain.initial.json"
    if include_initial and initial.is_file():
        checkpoints.insert(0, Checkpoint(generation_label(0), initial, 0))
    return checkpoints, island


def read_history(path):
    """Rows of a campaign history.jsonl, skipping a half-written last line."""
    rows = []
    try:
        text = Path(path).read_text(encoding="utf-8")
    except FileNotFoundError:
        return rows
    for line in text.split("\n"):
        line = line.strip()
        if not line:
            continue
        try:
            rows.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return rows


def population_range(history, generation, track, island):
    """(lowest, highest, mean, count) of that generation's candidate fitness, or None."""
    for row in history:
        if row.get("generation") != generation:
            continue
        for t in row.get("tracks", []):
            if t.get("name") != track:
                continue
            for i in t.get("islands", []):
                values = [v for v in i.get("candidate_fitness", []) if isinstance(v, (int, float))]
                if i.get("name") == island and values:
                    return (min(values), max(values), sum(values) / len(values), len(values))
    return None


def checkpoints_from_files(paths, labels=None):
    checkpoints = []
    for index, path in enumerate(paths):
        path = Path(path)
        match = re.search(r"generation[-_ ]?(\d+)", path.stem, re.IGNORECASE)
        generation = int(match.group(1)) if match else None
        if labels and index < len(labels):
            label = labels[index]
        elif generation is not None:
            label = generation_label(generation)
        else:
            label = path.stem
        checkpoints.append(Checkpoint(label, path, generation))
    return checkpoints


def parse_generations(text):
    """'1,5,10-20' -> {1, 5, 10..20}."""
    wanted = set()
    for part in text.split(","):
        part = part.strip()
        if not part:
            continue
        low, _, high = part.partition("-")
        wanted.update(range(int(low), int(high or low) + 1))
    return wanted


def thin_log_spaced(items, count):
    """At most `count` items, always the first and last, spaced evenly on a log scale of position.

    Early training changes fast and late training barely moves, so scenes bunch up early.
    """
    n = len(items)
    if count is None or count >= n:
        return list(items)
    if count <= 0:
        return []
    if count == 1:
        return [items[-1]]
    picks = {int(round(n ** (k / (count - 1)) - 1)) for k in range(count)}
    while len(picks) < count:  # rounding merged some picks: split the widest gap
        ordered = sorted(picks)
        width, start = max((b - a, a) for a, b in zip(ordered, ordered[1:]))
        picks.add(start + width // 2)
    return [items[i] for i in sorted(picks)]


# --------------------------------------------------------------------------------------
# Battles
# --------------------------------------------------------------------------------------


@dataclass
class Battle:
    """One played battle, reduced to what the cloud needs."""

    seed: int
    outcome: str
    fitness: float
    dt: float  # seconds between stored frames
    pos: np.ndarray  # (T, U, 2) float32, x and z
    alive: np.ndarray  # (T, U) bool
    learner: np.ndarray  # (U,) bool, unit belongs to the brain being shown
    radius: np.ndarray  # (U,) float32
    names: tuple = ()  # (U,) unit definition names, e.g. "armpw"

    @property
    def duration(self):
        return (self.pos.shape[0] - 1) * self.dt


def parse_battle(data):
    """A `--watch-battle` JSON document -> Battle."""
    units = data["units"]
    index = {unit["id"]: i for i, unit in enumerate(units)}
    frames = data["frames"]
    if not frames:
        raise ValueError("battle has no frames")
    t, u = len(frames), len(units)
    pos = np.zeros((t, u, 2), np.float32)
    alive = np.zeros((t, u), bool)
    for ti, frame in enumerate(frames):
        for uid, x, z, _heading, hp in frame["u"]:
            ui = index[int(uid)]
            pos[ti, ui] = (x, z)
            alive[ti, ui] = hp > 0
    times = np.array([f["time"] for f in frames], np.float64)
    dt = float(np.median(np.diff(times))) if t > 1 else 0.1
    if not dt > 0:
        raise ValueError("battle frames are not in time order")
    side = data["learner_side"]
    return Battle(
        seed=int(data["seed"]),
        outcome=str(data["outcome"]),
        fitness=float(data["fitness"]),
        dt=dt,
        pos=pos,
        alive=alive,
        learner=np.array([unit["side"] == side for unit in units], bool),
        radius=np.array([unit["radius"] for unit in units], np.float32),
        names=tuple(str(unit["name"]) for unit in units),
    )


def save_battle(path, battle):
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + f".{os.getpid()}.tmp.npz")
    np.savez_compressed(
        tmp,
        meta=np.array(json.dumps({"seed": battle.seed, "outcome": battle.outcome,
                                  "fitness": battle.fitness, "dt": battle.dt,
                                  "names": list(battle.names)})),
        pos=battle.pos, alive=battle.alive, learner=battle.learner, radius=battle.radius,
    )
    os.replace(tmp, path)


def load_battle(path):
    with np.load(path, allow_pickle=False) as z:
        meta = json.loads(str(z["meta"]))
        return Battle(seed=meta["seed"], outcome=meta["outcome"], fitness=meta["fitness"], dt=meta["dt"],
                      pos=z["pos"], alive=z["alive"], learner=z["learner"], radius=z["radius"],
                      names=tuple(meta["names"]))  # KeyError on an older entry: the caller plays it again


def file_digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


class Simulator:
    """Plays battles through the simulator binary and caches them by content."""

    def __init__(self, exe, catalog, train_config, cache_dir, unit=None, scenario=0, opponent=0,
                 side=0, use_cache=True, timeout=300):
        self.exe, self.catalog, self.config = Path(exe).resolve(), Path(catalog).resolve(), Path(train_config).resolve()
        self.unit, self.scenario, self.opponent, self.side = unit, scenario, opponent, side
        self.cache_dir, self.use_cache, self.timeout = Path(cache_dir), use_cache, timeout
        if not self.exe.is_file():
            raise FileNotFoundError(f"simulator binary not found: {self.exe} (cargo build --release, or pass --exe)")
        # Everything that can change a battle besides the brain and the seed. The
        # config may point at a companion brain, whose contents matter too.
        parts = [CACHE_FORMAT, self.exe.stat().st_size, int(self.exe.stat().st_mtime), file_digest(self.catalog),
                 file_digest(self.config), self.unit or ""]
        try:
            companion = json.loads(self.config.read_text(encoding="utf-8")).get("companion_brain")
        except (OSError, ValueError):
            companion = None
        if companion and (self.config.parent / companion).is_file():
            parts.append(file_digest(self.config.parent / companion))
        self.environment = hashlib.sha256(json.dumps(parts).encode()).hexdigest()[:12]

    def _command(self, subcommand, brain_path):
        cmd = [str(self.exe), "--catalog", str(self.catalog)]
        if self.unit:
            cmd += ["--unit", self.unit]
        return cmd + [subcommand, str(brain_path), "--train-config", str(self.config)]

    def _run(self, cmd):
        env = dict(os.environ, RAYON_NUM_THREADS="1")
        done = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", timeout=self.timeout, env=env)
        if done.returncode != 0:
            raise RuntimeError(f"simulator failed ({done.returncode}): {done.stderr.strip()[-600:]}")
        return done.stdout

    def stage_brain(self, brain):
        """Writes the brain under a content hash and returns (digest, path)."""
        text = json.dumps(brain, sort_keys=True, separators=(",", ":"))
        digest = hashlib.sha256(text.encode()).hexdigest()[:16]
        path = self.cache_dir / "brains" / f"{digest}.json"
        if not path.is_file():
            path.parent.mkdir(parents=True, exist_ok=True)
            tmp = path.with_name(path.name + f".{os.getpid()}.tmp")
            tmp.write_text(text, encoding="utf-8")
            os.replace(tmp, path)
        return digest, path

    def battle(self, digest, brain_path, seed):
        name = f"s{self.scenario}-o{self.opponent}-l{self.side}-{seed}.npz"
        cached = self.cache_dir / "battles" / self.environment / digest / name
        if self.use_cache and cached.is_file():
            try:
                return load_battle(cached)
            except (OSError, ValueError, KeyError):
                pass  # unreadable cache entry: play it again
        cmd = self._command("--watch-battle", brain_path) + [
            "--watch-scenario", str(self.scenario), "--watch-opponent", str(self.opponent),
            "--watch-seed", str(seed), "--watch-side", str(self.side)]
        battle = parse_battle(json.loads(self._run(cmd)))
        save_battle(cached, battle)  # even with use_cache off: progressive mode reloads runs from here
        return battle

    def describe(self, brain_path):
        """Scenario and opponent names the config offers."""
        meta = json.loads(self._run(self._command("--watch-meta", brain_path)))
        return meta.get("scenarios", []), meta.get("opponents", [])


def stage_brains(simulator, checkpoints):
    """[(digest, path)] of each checkpoint's brain written where the simulator can read it."""
    return [simulator.stage_brain(c.load_brain()) for c in checkpoints]


@dataclass
class RunSummary:
    """What ranking and scheduling need to know about a battle, without its frames."""

    seed: int
    fitness: float
    outcome: str
    duration: float  # simulated seconds


def summarise(battle):
    return RunSummary(battle.seed, battle.fitness, battle.outcome, battle.duration)


def play_all(simulator, checkpoints, seeds, jobs, log=print, summaries=False, staged=None):
    """Battles for every checkpoint: {checkpoint index: [Battle sorted by seed]}.

    With `summaries` the lists hold RunSummary instead, so thousands of battles need no memory
    (they stay in the simulator's disk cache). `staged` reuses already staged brains."""
    if staged is None:
        staged = stage_brains(simulator, checkpoints)
    tasks = [(ci, seed) for ci in range(len(checkpoints)) for seed in seeds]
    results = {ci: [] for ci in range(len(checkpoints))}
    done, started = 0, time.time()

    def work(task):
        ci, seed = task
        digest, path = staged[ci]
        battle = simulator.battle(digest, path, seed)
        return ci, summarise(battle) if summaries else battle

    with ThreadPoolExecutor(max_workers=max(1, jobs)) as pool:
        for ci, battle in pool.map(work, tasks):
            results[ci].append(battle)
            done += 1
            if done % max(1, len(tasks) // 10) == 0 or done == len(tasks):
                log(f"  played {done}/{len(tasks)} battles ({time.time() - started:.0f} s)")
    for battles in results.values():
        battles.sort(key=lambda b: b.seed)
    return results


# --------------------------------------------------------------------------------------
# Unit rosters
# --------------------------------------------------------------------------------------


@dataclass
class UnitInfo:
    name: str  # display name, e.g. "Pawn"
    value: float  # resource value of one unit: metal + energy / 60 (the repo's formula)


@dataclass
class Roster:
    """One side's starting units, one line per unit type, most valuable line first."""

    lines: list = field(default_factory=list)  # "40x Pawns"
    values: list = field(default_factory=list)  # resource value of each line
    total: float = 0.0


def load_unit_info(catalog_path):
    """{definition name: UnitInfo} from a unit catalog; empty if the file has no unit table."""
    try:
        data = json.loads(Path(catalog_path).read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    units = data.get("unit_catalog") if isinstance(data, dict) else None
    if not isinstance(units, dict):
        return {}
    info = {}
    for code, unit in units.items():
        if not isinstance(unit, dict):
            continue
        value = float(unit.get("metal_cost") or 0.0) + float(unit.get("energy_cost") or 0.0) / 60.0
        info[code] = UnitInfo(str(unit.get("human_name") or code), value)
    return info


def pluralise(name):
    if name.endswith(("s", "x", "z", "ch", "sh")):
        return name + "es"
    if name.endswith("y") and name[-2:-1] not in ("a", "e", "i", "o", "u", ""):
        return name[:-1] + "ies"
    return name + "s"


def build_roster(names, mask, info):
    """Starting units flagged by `mask`, grouped by type and sorted by the line's total value."""
    counts = {}
    for name, flagged in zip(names, mask):
        if flagged:
            counts[name] = counts.get(name, 0) + 1
    rows = []
    for code, count in counts.items():
        unit = info.get(code) or UnitInfo(code, 0.0)
        rows.append((-count * unit.value, -count, unit.name, count, count * unit.value))
    rows.sort()
    roster = Roster()
    for _, _, name, count, value in rows:
        roster.lines.append(f"{count}x {name if count == 1 else pluralise(name)}")
        roster.values.append(value)
        roster.total += value
    return roster


# --------------------------------------------------------------------------------------
# Scenes and camera
# --------------------------------------------------------------------------------------


@dataclass
class Scene:
    checkpoint: Checkpoint
    pos: np.ndarray  # (B, T, U, 2)
    alive: np.ndarray  # (B, T, U)
    learner: np.ndarray  # (B, U)
    radius: np.ndarray  # (B, U)
    dt: float
    outcomes: list
    fitness: np.ndarray  # (B,)
    sim_seconds: float = 0.0  # simulated time the scene plays
    video_seconds: float = 0.0
    sim_before: float = 0.0  # simulated seconds in earlier scenes
    rosters: tuple = (Roster(), Roster())  # (the brain's side, the opponent's side)

    @property
    def battles(self):
        return self.pos.shape[0]


def build_scene(checkpoint, battles, max_sim_seconds=None, unit_info=None):
    """Stack battles (padding shorter ones with their last frame and missing units as dead)."""
    if not battles:
        raise ValueError(f"{checkpoint.label}: no battles")
    dt = battles[0].dt
    if any(abs(b.dt - dt) > 1e-6 for b in battles):
        raise ValueError(f"{checkpoint.label}: battles disagree on frame interval")
    frames = max(b.pos.shape[0] for b in battles)
    units = max(b.pos.shape[1] for b in battles)
    n = len(battles)
    pos = np.zeros((n, frames, units, 2), np.float32)
    alive = np.zeros((n, frames, units), bool)
    learner = np.zeros((n, units), bool)
    radius = np.zeros((n, units), np.float32)
    for i, b in enumerate(battles):
        t, u = b.pos.shape[:2]
        pos[i, :t, :u] = b.pos
        alive[i, :t, :u] = b.alive
        pos[i, t:, :u] = b.pos[-1]
        alive[i, t:, :u] = b.alive[-1]
        learner[i, :u] = b.learner
        radius[i, :u] = b.radius
    sim = max(b.duration for b in battles)
    if max_sim_seconds:
        sim = min(sim, max_sim_seconds)
    # Every battle of a scene starts from the same roster; the first one stands for all.
    return Scene(checkpoint, pos, alive, learner, radius, dt, [b.outcome for b in battles],
                 np.array([b.fitness for b in battles], np.float64), sim_seconds=sim,
                 rosters=rosters_of(battles[0], unit_info))


def rosters_of(battle, unit_info=None):
    """(the brain's roster, the opponent's roster) at the start of a battle."""
    if not battle.names:
        return (Roster(), Roster())
    start, info = battle.alive[0], unit_info or {}
    return (build_roster(battle.names, battle.learner & start, info),
            build_roster(battle.names, ~battle.learner & start, info))


@dataclass
class View:
    cx: float
    cz: float
    scale: float  # pixels per world unit
    ox: float | None = None  # pixel column of (cx, cz); the canvas centre when unset
    oy: float | None = None

    def to_pixels(self, xz, width, height):
        ox = width / 2 if self.ox is None else self.ox
        oy = height / 2 if self.oy is None else self.oy
        return (ox + (xz[..., 0] - self.cx) * self.scale,
                oy + (xz[..., 1] - self.cz) * self.scale)


def _margins(margin):
    """An int, or (left, top, right, bottom), as four pixel margins."""
    if isinstance(margin, (tuple, list)):
        left, top, right, bottom = margin
        return left, top, right, bottom
    return margin, margin, margin, margin


def fit_view(scenes, width, height, margin_px, pad=0.06, clip=0.4):
    """One camera for the whole video, so scenes are comparable: the area holding
    all but the outermost `clip` percent of live unit positions, plus padding.
    `margin_px` is an int or (left, top, right, bottom): space kept free for labels."""
    points = []
    for scene in scenes:
        step = max(1, scene.pos.shape[1] // 40)
        points.append(scene.pos[:, ::step][scene.alive[:, ::step]])
    return view_from_points(points, width, height, margin_px, pad, clip)


def battle_points(battle, samples=40):
    """Live unit positions (M, 2) at about `samples` moments of one battle."""
    step = max(1, battle.pos.shape[0] // samples)
    return battle.pos[::step][battle.alive[::step]]


def view_from_points(points, width, height, margin_px, pad=0.06, clip=0.4):
    """Camera over a list of (M, 2) position arrays, ignoring the outermost `clip` percent."""
    points = [p for p in points if p.size]
    if not points:
        raise ValueError("no live units to frame")
    p = np.concatenate(points)
    x0, x1 = np.percentile(p[:, 0], [clip, 100 - clip])
    z0, z1 = np.percentile(p[:, 1], [clip, 100 - clip])
    return view_from_box(x0, z0, x1, z1, width, height, margin_px, pad)


def view_from_box(x0, z0, x1, z1, width, height, margin_px, pad=0.0):
    left, top, right, bottom = _margins(margin_px)
    w, h = max(x1 - x0, 1.0), max(z1 - z0, 1.0)
    w, h = w * (1 + 2 * pad), h * (1 + 2 * pad)
    free_w, free_h = width - left - right, height - top - bottom
    scale = min(free_w / w, free_h / h)
    return View((x0 + x1) / 2, (z0 + z1) / 2, scale, left + free_w / 2, top + free_h / 2)


# --------------------------------------------------------------------------------------
# Rendering
# --------------------------------------------------------------------------------------


def blend_frames(pos, alive, dt, sim_t, time_axis):
    """Positions linearly interpolated to a simulated time, and who is alive at the earlier frame.
    Past the last frame the final state holds."""
    frames = pos.shape[time_axis]
    f = min(max(sim_t / dt, 0.0), frames - 1)
    i0 = int(f)
    i1 = min(i0 + 1, frames - 1)
    a = np.float32(f - i0)
    first, second = np.take(pos, i0, axis=time_axis), np.take(pos, i1, axis=time_axis)
    return first * (1 - a) + second * a, np.take(alive, i0, axis=time_axis)


def _stamp(radius_px):
    """Offsets and weights of a soft disc, peak weight 1."""
    r = max(1, int(math.ceil(radius_px)))
    dy, dx = np.mgrid[-r:r + 1, -r:r + 1]
    d = np.sqrt(dx * dx + dy * dy) / max(radius_px, 0.5)
    w = np.clip((1.0 - d) * 2.0, 0.0, 1.0)  # flat core, soft rim
    w = w * w * (3 - 2 * w)
    keep = w > 0.02
    return dx[keep].astype(np.int32), dy[keep].astype(np.int32), w[keep].astype(np.float32)


class Renderer:
    def __init__(self, width, height, view, fps, speed, glow=1.0, dot_scale=0.4, trail_seconds=0.5,
                 min_dot_px=1.6, grid_step=100.0):
        self.width, self.height, self.view = width, height, view
        self.fps, self.speed = fps, speed
        self.glow, self.dot_scale, self.min_dot_px = glow, dot_scale, min_dot_px
        # Trail memory in video frames: halves every `trail_seconds` of video time.
        self.decay = 0.5 ** (1.0 / max(1.0, trail_seconds * fps)) if trail_seconds > 0 else 0.0
        self.background = self._background(grid_step)
        self._stamps = {}
        self.reset()

    def reset(self):
        self.trail = [np.zeros((self.height, self.width), np.float32) for _ in range(2)]

    def _background(self, step):
        img = np.empty((self.height, self.width, 3), np.float32)
        img[:] = BACKGROUND_RGB
        v = self.view
        ox = self.width / 2 if v.ox is None else v.ox
        oy = self.height / 2 if v.oy is None else v.oy
        x_lo, x_hi = v.cx - ox / v.scale, v.cx + (self.width - ox) / v.scale
        z_lo, z_hi = v.cz - oy / v.scale, v.cz + (self.height - oy) / v.scale
        for x in np.arange(math.ceil(x_lo / step), math.floor(x_hi / step) + 1) * step:
            px = int(round(ox + (x - v.cx) * v.scale))
            if 0 <= px < self.width:
                img[:, px] = GRID_RGB
        for z in np.arange(math.ceil(z_lo / step), math.floor(z_hi / step) + 1) * step:
            py = int(round(oy + (z - v.cz) * v.scale))
            if 0 <= py < self.height:
                img[py, :] = GRID_RGB
        return img

    def stamp(self, radius_px):
        key = int(round(radius_px * 2))
        if key not in self._stamps:
            self._stamps[key] = _stamp(key / 2)
        return self._stamps[key]

    def _density(self, pos, alive, mine, radius, weight, group):
        """Summed soft discs of every live unit of one side (0 = the brain's, 1 = the opponent's).
        Arguments are flat per-unit arrays: positions (K, 2), liveness, side flag, radius, opacity."""
        h, w = self.height, self.width
        mask = alive & (mine == (group == 0))
        if not mask.any():
            return np.zeros((h, w), np.float32)
        px, py = self.view.to_pixels(pos, w, h)
        radii = np.maximum(radius * self.view.scale * self.dot_scale, self.min_dot_px)
        classes = np.round(radii[mask] * 2).astype(np.int32)
        x = np.rint(px[mask]).astype(np.int32)
        y = np.rint(py[mask]).astype(np.int32)
        opacity = weight[mask]
        flat, weights = [], []
        for cls in np.unique(classes):
            sel = classes == cls
            dx, dy, wt = self.stamp(cls / 2)
            xx = x[sel][:, None] + dx[None, :]
            yy = y[sel][:, None] + dy[None, :]
            ok = (xx >= 0) & (xx < w) & (yy >= 0) & (yy < h)
            flat.append((yy * w + xx)[ok])
            weights.append((wt[None, :] * opacity[sel][:, None])[ok])
        idx, wt = np.concatenate(flat), np.concatenate(weights)
        return np.bincount(idx, weights=wt, minlength=h * w).astype(np.float32).reshape(h, w)

    @staticmethod
    def balance_for(mine_units, their_units):
        """Per-side brightness factors so a side with few units (one Mammoth against
        forty Pawns) is as easy to follow as the crowd: sqrt of the unit-count ratio, up to 4x."""
        mine, theirs = max(int(mine_units), 1), max(int(their_units), 1)
        top = max(mine, theirs)
        return (min(4.0, math.sqrt(top / mine)), min(4.0, math.sqrt(top / theirs)))

    @classmethod
    def _balance(cls, scene):
        start = scene.alive[:, 0]
        return cls.balance_for((start & scene.learner).sum(), (start & ~scene.learner).sum())

    def positions_at(self, scene, sim_t):
        """Interpolated positions (B, U, 2) and liveness (B, U) at a simulated time."""
        return blend_frames(scene.pos, scene.alive, scene.dt, sim_t, time_axis=1)

    def _compose(self, pos, alive, mine, radius, weight, runs_on_screen, balance):
        """Draws flat per-unit arrays as translucent clouds over the background."""
        # About 1/sqrt(N) opacity per ghost, so a lone ghost is visible and a crowd saturates.
        gain = self.glow * 4.0 / math.sqrt(max(runs_on_screen, 1))
        out = self.background.copy()
        for group, rgb in enumerate((LEARNER_RGB, ENEMY_RGB)):
            density = self._density(pos, alive, mine, radius, weight, group)
            self.trail[group] = self.trail[group] * self.decay + density * (1.0 - self.decay)
            strength = 1.0 - np.exp(-gain * balance[group] * (density + 0.8 * self.trail[group]))
            for c in range(3):
                out[..., c] = out[..., c] * (1.0 - strength) + rgb[c] * strength
        return out

    def draw(self, scene, sim_t):
        """One frame of a scene (all its battles on one clock) as float RGB (H, W, 3), not yet clipped."""
        positions, alive = self.positions_at(scene, sim_t)
        flat = positions.reshape(-1, 2)
        return self._compose(flat, alive.reshape(-1), scene.learner.reshape(-1), scene.radius.reshape(-1),
                             np.ones(len(flat), np.float32), scene.battles, self._balance(scene))

    def draw_runs(self, runs, nominal_runs, balance):
        """One frame of battles that each have their own clock and opacity:
        `runs` is a list of (Battle, simulated seconds into it, opacity 0..1)."""
        if not runs:
            return self._compose(np.zeros((0, 2), np.float32), np.zeros(0, bool), np.zeros(0, bool),
                                 np.zeros(0, np.float32), np.zeros(0, np.float32), nominal_runs, balance)
        parts = []
        for battle, sim_t, opacity in runs:
            pos, alive = blend_frames(battle.pos, battle.alive, battle.dt, sim_t, time_axis=0)
            parts.append((pos, alive, battle.learner, battle.radius, np.full(len(alive), opacity, np.float32)))
        pos, alive, mine, radius, weight = (np.concatenate(column) for column in zip(*parts))
        return self._compose(pos, alive, mine, radius, weight, nominal_runs, balance)


class Labels:
    """Corner text and the two unit lists. Built from the scenes so the lists share one width."""

    def __init__(self, width, height, scenes=(), rosters=True, bottom_lines=3):
        """`scenes` is anything with a `.rosters` pair; `bottom_lines` is how many lines the
        lower-left text can take, which sets how much room the map leaves at the bottom."""
        self.width, self.height = width, height
        k = self.k = height / 720.0
        self.margin = int(round(26 * k))
        self.big = self._font(34 * k, bold=True)
        self.mid = self._font(22 * k, bold=True)
        self.small = self._font(17 * k, bold=False)
        self.head = self._font(18 * k, bold=True)
        self.item = self._font(21 * k, bold=False)
        self.line_height = int(round(30 * k))
        self.stroke = max(1, int(round(2 * k)))
        self.bottom_reserve = int(round((90 + 20 * max(0, bottom_lines - 3)) * k))  # room the lower text takes
        self.panel_top = self.margin + int(round(58 * k))  # below the checkpoint title
        self.panel_width = self._measure_panels(scenes) if rosters else 0

    def _measure_panels(self, scenes):
        """One width for both lists in every scene, so nothing shifts between scenes."""
        widest = 0.0
        for scene in scenes:
            for panel in roster_panels(scene):
                texts = [(panel["title"], self.head), (panel["footer"], self.small)]
                texts += [(line, self.item) for line in panel["lines"]]
                for text, font in texts:
                    widest = max(widest, font.getlength(text))
        if not widest:
            return 0
        return int(min(widest + 18 * self.k, 0.26 * self.width))

    def reserved_margins(self):
        """(left, top, right, bottom) pixels the map must leave free for the labels and lists."""
        side = self.margin + (self.panel_width + int(round(22 * self.k)) if self.panel_width else 0)
        return side, self.margin + int(round(40 * self.k)), side, self.margin + self.bottom_reserve

    @staticmethod
    def _font(size, bold):
        size = max(8, int(round(size)))
        names = (["segoeuib.ttf", "arialbd.ttf", "DejaVuSans-Bold.ttf"] if bold
                 else ["segoeui.ttf", "arial.ttf", "DejaVuSans.ttf"])
        for name in names:
            try:
                return ImageFont.truetype(name, size)
            except OSError:
                continue
        return ImageFont.load_default(size=size)

    def _text(self, draw, xy, text, font, anchor, fill=TEXT_RGB):
        draw.text(xy, text, font=font, fill=fill, anchor=anchor, stroke_width=self.stroke, stroke_fill=(8, 10, 14))

    def _panel(self, draw, panel, x):
        """One unit list: coloured title, one line per unit type, total value."""
        k, y = self.k, self.panel_top
        rgb = panel["rgb"]
        bar = max(2, int(round(3 * k)))
        text_x = x + bar + int(round(10 * k))
        lines, hidden = list(panel["lines"]), 0
        room = (self.height - self.margin - self.bottom_reserve - y - self.line_height * 2) // self.line_height
        if len(lines) > max(room, 1):
            keep = max(room - 1, 1)
            hidden = len(lines) - keep
            lines = lines[:keep]
        footer_y = y + self.line_height * (1 + len(lines) + (1 if hidden else 0))
        draw.rectangle([x, y, x + bar - 1, footer_y + self.line_height // 2], fill=rgb)
        self._text(draw, (text_x, y), panel["title"].upper(), self.head, "la", rgb)
        y += self.line_height
        for text in lines:
            self._text(draw, (text_x, y), text, self.item, "la")
            y += self.line_height
        if hidden:
            self._text(draw, (text_x, y), f"+ {hidden} more types", self.small, "la", DIM_TEXT_RGB)
            y += self.line_height
        self._text(draw, (text_x, y), panel["footer"], self.small, "la", DIM_TEXT_RGB)

    def draw(self, image, upper_left, lower_left, lower_right, upper_right, panels=None):
        """Each corner argument is a list of (text, 'big'|'mid'|'small') lines, top to bottom.
        `panels` is (left, right) from roster_panels()."""
        draw = ImageDraw.Draw(image)
        if panels and self.panel_width:
            self._panel(draw, panels[0], self.margin)
            self._panel(draw, panels[1], self.width - self.margin - self.panel_width)
        m, w, h = self.margin, self.width, self.height
        fonts = {"big": self.big, "mid": self.mid, "small": self.small}

        def stack(lines, x, y, anchor_x, from_bottom):
            ordered = list(reversed(lines)) if from_bottom else lines
            for text, size in ordered:
                font = fonts[size]
                fill = DIM_TEXT_RGB if size == "small" else TEXT_RGB
                anchor = ("l" if anchor_x == "l" else "r") + ("d" if from_bottom else "a")
                self._text(draw, (x, y), text, font, anchor, fill)
                height = font.getbbox("Ag")[3] - font.getbbox("Ag")[1]
                y += -(height + 6) if from_bottom else (height + 6)

        stack(upper_left, m, m, "l", False)
        stack(upper_right, w - m, m, "r", False)
        stack(lower_left, m, h - m, "l", True)
        stack(lower_right, w - m, h - m, "r", True)
        return image


def format_clock(seconds):
    seconds = max(0.0, seconds)
    return f"{int(seconds // 60)}:{int(seconds % 60):02d}"


def population_text(pop):
    """The ES population's fitness (all training stages together, so not comparable with one scenario's battles)."""
    return f"ES population, all stages ({pop[3]}):  {pop[0]:+.3f} to {pop[1]:+.3f},  mean {pop[2]:+.3f}"


def fitness_text(values):
    low, high = float(np.min(values)), float(np.max(values))
    return f"{low:+.2f}  to  {high:+.2f}"


def corner_text(scene, sim_t, show_outcomes=True):
    """Corner label lines for one frame of a scene."""
    upper_left = [(scene.checkpoint.label, "big")]
    lower_left = [("Fitness range  " + fitness_text(scene.fitness), "mid")]
    pop = scene.checkpoint.population
    if pop:
        lower_left.append((population_text(pop), "small"))
    lower_left.append((f"{scene.battles} battles, same scenario, different seeds", "small"))
    lower_right = [(f"Sim time  {sim_t:5.1f} s", "mid"),
                   (f"video total  {format_clock(scene.sim_before + sim_t)}", "small")]
    upper_right = []
    if show_outcomes:
        n = scene.battles
        parts = []
        for name in OUTCOMES:
            count = sum(1 for o in scene.outcomes if o == name)
            if count:
                parts.append(f"{name} {round(100 * count / n)}%")
        upper_right = [("   ".join(parts), "mid")]
    return upper_left, lower_left, lower_right, upper_right


def roster_panels(scene):
    """(left, right) unit-list contents: the brain's units in blue, the opponent's in red."""
    panels = []
    for title, rgb, roster in (("Council units", LEARNER_RGB, scene.rosters[0]),
                               ("Red units", ENEMY_RGB, scene.rosters[1])):
        panels.append({"title": title, "rgb": rgb, "lines": roster.lines,
                       "footer": f"total value {roster.total:,.0f}" if roster.lines else ""})
    return tuple(panels)


def render_frame(renderer, labels, scene, sim_t, show_outcomes=True):
    rgb = renderer.draw(scene, sim_t)
    image = Image.fromarray(np.clip(rgb, 0, 255).astype(np.uint8), "RGB")
    labels.draw(image, *corner_text(scene, sim_t, show_outcomes), panels=roster_panels(scene))
    return image


# --------------------------------------------------------------------------------------
# Encoding
# --------------------------------------------------------------------------------------


class Encoder:
    """Feeds RGB frames to ffmpeg and writes an H.264 MP4."""

    def __init__(self, ffmpeg, out, width, height, fps, crf=18, preset="medium"):
        self.out = Path(out)
        self.out.parent.mkdir(parents=True, exist_ok=True)
        self.errors = tempfile.TemporaryFile()
        cmd = [ffmpeg, "-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24",
               "-s", f"{width}x{height}", "-r", str(fps), "-i", "-", "-an",
               "-c:v", "libx264", "-preset", preset, "-crf", str(crf),
               "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(self.out)]
        self.process = subprocess.Popen(cmd, stdin=subprocess.PIPE, stderr=self.errors)

    def write(self, image):
        try:
            self.process.stdin.write(image.tobytes())
        except (BrokenPipeError, OSError):
            raise RuntimeError("ffmpeg stopped early: " + self._errors())

    def _errors(self):
        self.errors.seek(0)
        return self.errors.read().decode("utf-8", "replace").strip()[-800:]

    def close(self):
        try:
            self.process.stdin.close()
        except OSError:
            pass
        code = self.process.wait()
        if code != 0:
            message = f"ffmpeg exited with {code}: {self._errors()}"
            self._discard()
            raise RuntimeError(message)
        self.errors.close()

    def abort(self):
        """Stops ffmpeg and removes the half-written video, so a failed run leaves no broken file."""
        self.process.kill()
        try:
            self.process.stdin.close()
        except OSError:
            pass
        self.process.wait()
        self._discard()

    def _discard(self):
        self.errors.close()
        try:
            self.out.unlink()
        except FileNotFoundError:
            pass


def plan_scenes(scenes, speed, hold_seconds, fps):
    """Fill in each scene's video length and the simulated time before it."""
    before = 0.0
    for scene in scenes:
        scene.sim_before = before
        scene.video_seconds = scene.sim_seconds / speed + hold_seconds
        before += scene.sim_seconds
    return sum(s.video_seconds for s in scenes)


def render_video(scenes, renderer, labels, out, ffmpeg, fps, speed, hold_seconds, show_outcomes=True,
                 crf=18, preset="medium", log=print):
    total = plan_scenes(scenes, speed, hold_seconds, fps)
    encoder = Encoder(ffmpeg, out, renderer.width, renderer.height, fps, crf, preset)
    started, written = time.time(), 0
    try:
        for scene in scenes:
            renderer.reset()
            frames = max(1, int(round(scene.video_seconds * fps)))
            for k in range(frames):
                sim_t = min(k / fps * speed, scene.sim_seconds)
                encoder.write(render_frame(renderer, labels, scene, sim_t, show_outcomes))
                written += 1
            log(f"  {scene.checkpoint.label}: {frames} frames  ({written}/{int(round(total * fps))}, "
                f"{time.time() - started:.0f} s)")
    except BaseException:
        encoder.abort()
        raise
    encoder.close()
    return written


def write_stills(scenes, renderer, labels, folder, fps, speed, show_outcomes=True):
    """One PNG per scene, a bit past the middle so trails and the clock are visible."""
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    paths = []
    plan_scenes(scenes, speed, 0.0, fps)
    for i, scene in enumerate(scenes):
        renderer.reset()
        target = scene.sim_seconds * 0.6
        steps = max(1, int(target / speed * fps))
        for k in range(steps + 1):  # play up to the target so trails exist
            sim_t = min(k / fps * speed, target)
            image = render_frame(renderer, labels, scene, sim_t, show_outcomes)
        path = folder / f"scene-{i:03d}.png"
        image.save(path)
        paths.append(path)
    return paths


# --------------------------------------------------------------------------------------
# Progressive mode: every generation's top runs go in a queue, a few start each second
# --------------------------------------------------------------------------------------


def select_runs(summaries, top, how="best", salt=0):
    """The `top` runs of one generation, in seed order (so a generation is not played best-first).

    `best` keeps the highest-fitness runs, which shows each generation at its best.
    `random` keeps an unbiased sample, which shows what a generation typically does."""
    summaries = list(summaries)
    if how not in ("best", "random"):
        raise ValueError(f"unknown selection '{how}' (use best or random)")
    if top >= len(summaries):
        chosen = summaries
    elif how == "best":
        chosen = sorted(summaries, key=lambda s: (-s.fitness, s.seed))[:top]
    else:
        chosen = random.Random(salt).sample(summaries, top)
    return sorted(chosen, key=lambda s: s.seed)


@dataclass
class Run:
    start: float  # video second the run begins
    end: float  # video second its battle ends
    checkpoint: int  # index into the checkpoint list
    summary: RunSummary


class Timeline:
    """The queue and its schedule. Runs are queued generation by generation; every `interval`
    video seconds the next `per_second` runs leave the front of the queue and start playing,
    each on its own clock. A run that has ended fades out instead of freezing, so the cloud
    always holds recent runs and its makeup slides from early generations to late ones."""

    def __init__(self, selected, per_second, interval, speed, max_sim=None, fade_out=0.6, fade_in=0.25,
                 spread=True):
        """`spread` staggers the launches evenly (one every interval / per_second) so runs arrive and
        leave one at a time, a stream. Without it a whole batch starts on one frame, which pulses."""
        if per_second < 1 or interval <= 0 or speed <= 0:
            raise ValueError("per-second must be at least 1, and interval and speed above 0")
        self.per_second, self.interval, self.speed = per_second, interval, speed
        self.fade_out, self.fade_in, self.spread = fade_out, fade_in, spread
        self.runs = []
        for checkpoint, summaries in enumerate(selected):
            for summary in summaries:
                index = len(self.runs)
                start = index * interval / per_second if spread else (index // per_second) * interval
                simulated = min(summary.duration, max_sim) if max_sim else summary.duration
                self.runs.append(Run(start, start + simulated / speed, checkpoint, summary))
        self.starts = [r.start for r in self.runs]
        self.span = max((r.end - r.start for r in self.runs), default=0.0) + fade_out
        self.total_seconds = max((r.end for r in self.runs), default=0.0) + fade_out

    def frame_count(self, fps):
        return max(1, int(math.ceil(self.total_seconds * fps)))

    def launched(self, t):
        """How many runs have left the queue by video time t."""
        return bisect_right(self.starts, t)

    def active(self, t):
        """Indices of runs visible at video time t, playing or fading out."""
        lo, hi = bisect_left(self.starts, t - self.span), bisect_right(self.starts, t)
        return [i for i in range(lo, hi) if self.runs[i].end + self.fade_out > t]

    def playing(self, indices, t):
        return [i for i in indices if self.runs[i].end > t]

    def opacity(self, run, t):
        rising = 1.0 if self.fade_in <= 0 else (t - run.start) / self.fade_in
        falling = 1.0 if t <= run.end else (0.0 if self.fade_out <= 0 else 1.0 - (t - run.end) / self.fade_out)
        return max(0.0, min(1.0, rising, falling))

    def sim_time(self, run, t):
        """Simulated seconds into the run's battle at video time t."""
        return min(max(t - run.start, 0.0) * self.speed, run.summary.duration)

    def typical_concurrency(self):
        """About how many runs are on screen at once once the queue is flowing."""
        if not self.runs:
            return 1.0
        mean = sum(r.end - r.start for r in self.runs) / len(self.runs)
        return max(1.0, self.per_second * mean / self.interval)


def span_text(checkpoints, low, high):
    """'showing generations 5 to 7', or None when only one checkpoint is on screen."""
    if low == high:
        return None
    a, b = checkpoints[low], checkpoints[high]
    if a.generation is not None and b.generation is not None:
        return f"showing generations {a.generation} to {b.generation}"
    return f"showing {a.label} to {b.label}"


def progressive_text(timeline, checkpoints, t, selection_note, show_outcomes=True):
    """Corner text for one moment of a progressive video.

    Nothing is static any more, so each readout describes what is on screen right now:
    the title names the newest generation to have left the queue (and the span still visible),
    the fitness range and outcome tally cover the runs playing, the clock counts simulated
    time since the video began, and the run counter shows how far through the queue it is."""
    launched = timeline.launched(t)
    visible = timeline.active(t)
    shown = timeline.playing(visible, t) or visible
    newest = timeline.runs[max(launched, 1) - 1].checkpoint
    checkpoint = checkpoints[newest]
    generations = [timeline.runs[i].checkpoint for i in shown] or [newest]
    upper_left = [(checkpoint.label, "big")]
    span = span_text(checkpoints, min(generations), max(generations))
    if span:
        upper_left.append((span, "small"))
    fitness = [timeline.runs[i].summary.fitness for i in shown]
    if fitness:
        lower_left = [("Fitness range  " + fitness_text(fitness), "mid"),
                      (f"mean {sum(fitness) / len(fitness):+.2f}   {len(fitness)} runs on screen", "small")]
    else:
        lower_left = [("Fitness range  n/a", "mid")]
    pop = checkpoint.population
    if pop:
        lower_left.append((population_text(pop), "small"))
    lower_left.append((selection_note, "small"))
    lower_right = [(f"Sim time  {format_clock(t * timeline.speed)}", "mid"),
                   (f"run {min(launched, len(timeline.runs))} of {len(timeline.runs)} started", "small")]
    upper_right = []
    if show_outcomes and shown:
        outcomes = [timeline.runs[i].summary.outcome for i in shown]
        parts = [f"{name} {round(100 * outcomes.count(name) / len(outcomes))}%"
                 for name in OUTCOMES if name in outcomes]
        upper_right = [("   ".join(parts), "mid")]
    return upper_left, lower_left, lower_right, upper_right


def progressive_frames(timeline, checkpoints, load, renderer, labels, rosters, balance, selection_note,
                       fps, first, last, show_outcomes=True):
    """Yields (frame number, PIL image) for frames first..last. `load(run)` returns the run's Battle.
    Trails start empty at `first`, so begin a second early when a still should show them."""
    renderer.reset()
    nominal = timeline.typical_concurrency()
    panels = roster_panels(SimpleNamespace(rosters=rosters))
    loaded = {}
    for k in range(first, last + 1):
        t = k / fps
        active = timeline.active(t)
        keep = set(active)
        for gone in [i for i in loaded if i not in keep]:
            del loaded[gone]
        items = []
        for i in active:
            if i not in loaded:
                loaded[i] = load(timeline.runs[i])
            run = timeline.runs[i]
            items.append((loaded[i], timeline.sim_time(run, t), timeline.opacity(run, t)))
        rgb = renderer.draw_runs(items, nominal, balance)
        image = Image.fromarray(np.clip(rgb, 0, 255).astype(np.uint8), "RGB")
        labels.draw(image, *progressive_text(timeline, checkpoints, t, selection_note, show_outcomes),
                    panels=panels)
        yield k, image


def render_progressive_video(timeline, checkpoints, load, renderer, labels, rosters, balance, selection_note,
                             out, ffmpeg, fps, show_outcomes=True, crf=18, preset="medium", log=print):
    frames = timeline.frame_count(fps)
    encoder = Encoder(ffmpeg, out, renderer.width, renderer.height, fps, crf, preset)
    started = time.time()
    try:
        for k, image in progressive_frames(timeline, checkpoints, load, renderer, labels, rosters, balance,
                                           selection_note, fps, 0, frames - 1, show_outcomes):
            encoder.write(image)
            if (k + 1) % max(1, frames // 10) == 0 or k + 1 == frames:
                log(f"  {k + 1}/{frames} frames ({time.time() - started:.0f} s)")
    except BaseException:
        encoder.abort()
        raise
    encoder.close()
    return frames


def write_progressive_stills(timeline, checkpoints, load, renderer, labels, rosters, balance, selection_note,
                             folder, fps, count=6, show_outcomes=True):
    """`count` PNGs spread over the video, each rendered after a second of warm-up so trails show."""
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    frames = timeline.frame_count(fps)
    paths = []
    for n in range(count):
        target = min(frames - 1, int(frames * (n + 0.5) / count))
        image = None
        for _, image in progressive_frames(timeline, checkpoints, load, renderer, labels, rosters, balance,
                                           selection_note, fps, max(0, target - fps), target, show_outcomes):
            pass
        path = folder / f"still-{n:03d}.png"
        image.save(path)
        paths.append(path)
    return paths


def selection_text(top, pool, how):
    if how == "random":
        return f"{top} random runs per generation (of {pool})"
    return f"top {top} of {pool} runs per generation" if pool > top else f"{top} runs per generation"


def run_progressive(args, checkpoints, simulator, unit_info, width, height):
    pool = args.pool or 2 * args.top
    staged = stage_brains(simulator, checkpoints)
    seeds = range(args.seed_base, args.seed_base + pool)
    print(f"playing {len(checkpoints) * pool} battles ({pool} per generation, scenario {args.scenario}, "
          f"opponent {args.opponent}) ...")
    summaries = play_all(simulator, checkpoints, seeds, args.jobs, summaries=True, staged=staged)
    selected = [select_runs(summaries[i], args.top, args.select, salt=i) for i in range(len(checkpoints))]
    timeline = Timeline(selected, args.per_second, args.interval, args.speed, args.max_sim_seconds, args.fade,
                        min(0.3, args.fade / 2), spread=args.launch == "stream")

    def load(run):
        digest, path = staged[run.checkpoint]
        return simulator.battle(digest, path, run.summary.seed)

    queue = len(timeline.runs)
    print(f"{queue} runs queued, {args.per_second} start every {args.interval:g} s: "
          f"{timeline.total_seconds:.0f} s of video, about {timeline.typical_concurrency():.0f} runs on screen at once")
    if timeline.total_seconds > 600:
        print("note: that is a long video; lower --top, raise --per-second, or thin the generations with --max-scenes")
    reference = load(timeline.runs[0])
    rosters = rosters_of(reference, unit_info)
    start = reference.alive[0]
    balance = Renderer.balance_for((start & reference.learner).sum(), (start & ~reference.learner).sum())
    labels = Labels(width, height, [SimpleNamespace(rosters=rosters)], rosters=not args.no_rosters, bottom_lines=4)
    margin = labels.reserved_margins()
    if args.view:
        view = view_from_view_arg(args.view, width, height, margin)
    else:
        picks = sorted({int(i * (queue - 1) / max(1, min(queue, 150) - 1)) for i in range(min(queue, 150))})
        view = view_from_points([battle_points(load(timeline.runs[i])) for i in picks], width, height, margin)
    renderer = Renderer(width, height, view, args.fps, args.speed, args.glow, args.dot_scale, args.trail)
    note = selection_text(args.top, pool, args.select)
    show_outcomes = not args.no_outcomes
    if args.stills:
        paths = write_progressive_stills(timeline, checkpoints, load, renderer, labels, rosters, balance, note,
                                         args.stills, args.fps, show_outcomes=show_outcomes)
        print(f"wrote {len(paths)} stills to {args.stills}")
    if args.stills_only:
        return 0
    if not shutil.which(args.ffmpeg) and not Path(args.ffmpeg).is_file():
        sys.exit(f"ffmpeg not found ({args.ffmpeg}); install it or pass --ffmpeg")
    out = Path(args.out)
    print(f"rendering {out} ({width}x{height} @ {args.fps} fps, {args.speed:g}x) ...")
    frames = render_progressive_video(timeline, checkpoints, load, renderer, labels, rosters, balance, note,
                                      out, args.ffmpeg, args.fps, show_outcomes, args.crf, args.preset)
    print(f"wrote {out}: {frames} frames, {frames / args.fps:.1f} s, {out.stat().st_size / 1e6:.1f} MB")
    return 0


def view_from_view_arg(text, width, height, margin):
    try:
        x0, z0, x1, z1 = (float(v) for v in text.split(","))
    except ValueError:
        sys.exit("--view must look like x0,z0,x1,z1")
    return view_from_box(x0, z0, x1, z1, width, height, margin)


# --------------------------------------------------------------------------------------
# Command line
# --------------------------------------------------------------------------------------


def build_parser():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    src = p.add_argument_group("checkpoints (give --run or --brains)")
    src.add_argument("--run", help="campaign run directory (holds candidates/, tracks/, history.jsonl)")
    src.add_argument("--track", default="ground", help="campaign track (default ground)")
    src.add_argument("--island", help="island name (default: the only one with checkpoints)")
    src.add_argument("--kind", default="center", help="checkpoint kind: center, sample, ... (default center)")
    src.add_argument("--no-initial", action="store_true", help="leave out the track's initial brain (generation 0)")
    src.add_argument("--brains", nargs="+", help="brain files, in the order to show them")
    src.add_argument("--labels", nargs="+", help="upper-left text for each of --brains")
    src.add_argument("--history", help="history.jsonl for the population fitness line (default: <run>/history.jsonl)")
    src.add_argument("--generations", help="only these generations, e.g. 0,5,10-20")
    src.add_argument("--max-scenes", type=int, help="keep at most N scenes, log-spaced over generation")
    mode = p.add_argument_group(
        "mode",
        "scenes (default): one scene per checkpoint, --samples battles on one clock. "
        "progressive: the top runs of every generation are queued and a few start each second.")
    mode.add_argument("--mode", choices=("scenes", "progressive"), default="scenes")
    mode.add_argument("--top", type=int, default=50, help="progressive: runs kept per generation (default 50)")
    mode.add_argument("--per-second", type=int, default=5,
                      help="progressive: runs taken from the front of the queue each interval (default 5)")
    mode.add_argument("--interval", type=float, default=1.0,
                      help="progressive: video seconds between launches (default 1)")
    mode.add_argument("--pool", type=int,
                      help="progressive: battles played per generation to choose the top from (default 2 x --top)")
    mode.add_argument("--select", choices=("best", "random"), default="best",
                      help="progressive: keep each generation's highest-fitness runs, or an unbiased random sample")
    mode.add_argument("--launch", choices=("stream", "burst"), default="stream",
                      help="progressive: stream starts runs evenly (one every interval / per-second); "
                           "burst starts each batch together once per interval (default stream)")
    mode.add_argument("--fade", type=float, default=0.6, help="progressive: seconds a finished run takes to fade (default 0.6)")
    sim = p.add_argument_group("battles")
    sim.add_argument("--catalog", help="unit catalog JSON")
    sim.add_argument("--train-config", help="tactical train config / stage file that names the scenarios")
    sim.add_argument("--unit", help="forwarded as the simulator's --unit")
    sim.add_argument("--exe", default=str(HERE / "target" / "release" /
                                          ("visigoth-simulator.exe" if os.name == "nt" else "visigoth-simulator")))
    sim.add_argument("--scenario", type=int, default=0, help="scenario index in the config (see --list)")
    sim.add_argument("--opponent", type=int, default=0, help="opponent index in the config (see --list)")
    sim.add_argument("--side", type=int, default=0, choices=(0, 1), help="side the brain plays")
    sim.add_argument("--samples", type=int, default=48, help="battles per scene, one seed each (default 48)")
    sim.add_argument("--seed-base", type=int, default=1,
                     help="first seed (default 1; training and validation use huge seeds, so these are fresh)")
    sim.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 4) - 2), help="parallel simulator processes")
    sim.add_argument("--cache", help="keep played battles in this folder so later renders skip the simulation "
                                     "(default: a temporary folder that is removed when the run ends)")
    sim.add_argument("--fresh", action="store_true", help="ignore cached battles and play them again")
    sim.add_argument("--list", action="store_true", help="print the config's scenarios and opponents, then exit")
    out = p.add_argument_group("video")
    out.add_argument("--out", default="cloud.mp4")
    out.add_argument("--size", default="1280x720", help="WIDTHxHEIGHT, both even (default 1280x720)")
    out.add_argument("--fps", type=int, default=30)
    out.add_argument("--speed", type=float, default=4.0, help="simulated seconds per video second (default 4)")
    out.add_argument("--hold", type=float, default=0.8, help="seconds to hold each scene's last frame (default 0.8)")
    out.add_argument("--max-sim-seconds", type=float, help="cut every scene off after this much simulated time")
    out.add_argument("--glow", type=float, default=1.0, help="overall brightness of the cloud (default 1)")
    out.add_argument("--dot-scale", type=float, default=0.4, help="unit blob size relative to its real radius (default 0.4)")
    out.add_argument("--trail", type=float, default=0.5, help="trail half-life in video seconds, 0 for none (default 0.5)")
    out.add_argument("--view", help="fixed camera x0,z0,x1,z1 in world units (default: fitted to all scenes)")
    out.add_argument("--no-outcomes", action="store_true", help="leave the outcome tally out of the upper right")
    out.add_argument("--no-rosters", action="store_true", help="leave out the unit lists on the left and right")
    out.add_argument("--crf", type=int, default=18, help="x264 quality, lower is better (default 18)")
    out.add_argument("--preset", default="medium", help="x264 speed/size preset (default medium)")
    out.add_argument("--ffmpeg", default=shutil.which("ffmpeg") or "ffmpeg")
    out.add_argument("--stills", help="also write one PNG per scene into this folder")
    out.add_argument("--stills-only", action="store_true", help="write the stills and stop before encoding")
    return p


def gather_checkpoints(args):
    if bool(args.run) == bool(args.brains):
        raise ValueError("give exactly one of --run or --brains")
    if args.brains:
        return checkpoints_from_files(args.brains, args.labels), None
    checkpoints, island = discover_run(args.run, args.track, args.island, args.kind, not args.no_initial)
    history = read_history(args.history or Path(args.run) / "history.jsonl")
    for c in checkpoints:
        if c.generation:
            c.population = population_range(history, c.generation, args.track, island)
    return checkpoints, island


@contextlib.contextmanager
def cache_scope(explicit=None):
    """The folder battles are cached in. Without --cache it is a fresh folder in the system temp
    directory, removed when the run ends (however it ends), so nothing is left behind and nothing
    lands in a repository. A folder named with --cache is kept: that is how re-renders skip the
    simulation."""
    if explicit:
        yield Path(explicit)
        return
    folder = Path(tempfile.mkdtemp(prefix="cloud_video_"))
    try:
        yield folder
    finally:
        shutil.rmtree(folder, ignore_errors=True)
        if folder.exists():
            print(f"warning: could not remove the temporary cache {folder}", file=sys.stderr)


def main(argv=None):
    args = build_parser().parse_args(argv)
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    try:
        width, height = (int(v) for v in args.size.lower().split("x"))
    except ValueError:
        sys.exit("--size must look like 1280x720")
    if width % 2 or height % 2:
        sys.exit("--size must have even width and height (H.264 4:2:0)")
    if not args.catalog or not args.train_config:
        sys.exit("--catalog and --train-config are required (they name the units and scenarios)")
    if args.mode == "progressive":
        if args.top < 1 or args.per_second < 1 or args.interval <= 0 or args.fade < 0:
            sys.exit("--top and --per-second must be at least 1, --interval above 0, --fade not negative")
        if args.pool is not None and args.pool < args.top:
            sys.exit(f"--pool ({args.pool}) must be at least --top ({args.top}): the top runs are chosen from the pool")
    try:
        checkpoints, island = gather_checkpoints(args)
    except ValueError as error:
        sys.exit(str(error))
    if args.generations:
        wanted = parse_generations(args.generations)
        checkpoints = [c for c in checkpoints if c.generation in wanted]
    if args.max_scenes:
        checkpoints = thin_log_spaced(checkpoints, args.max_scenes)
    if not checkpoints:
        sys.exit("no checkpoints left after filtering")

    with cache_scope(args.cache) as cache:
        return render(args, checkpoints, cache, width, height)


def render(args, checkpoints, cache, width, height):
    out = Path(args.out)
    try:
        simulator = Simulator(args.exe, args.catalog, args.train_config, cache, args.unit, args.scenario,
                              args.opponent, args.side, use_cache=not args.fresh)
    except FileNotFoundError as error:
        sys.exit(str(error))

    if args.list:
        _, path = simulator.stage_brain(checkpoints[0].load_brain())
        scenarios, opponents = simulator.describe(path)
        print("scenarios:")
        for i, s in enumerate(scenarios):
            print(f"  {i}: {s['name']}  [{s['learner']} vs {s['enemy']}]")
        print("opponents:")
        for i, name in enumerate(opponents):
            print(f"  {i}: {name}")
        return 0

    unit_info = load_unit_info(args.catalog)
    if not unit_info:
        print("note: the catalog has no unit table, so unit lists show definition names and sort by count")
    if args.mode == "progressive":
        print(f"{len(checkpoints)} generations: " + ", ".join(c.label for c in checkpoints))
        return run_progressive(args, checkpoints, simulator, unit_info, width, height)

    print(f"{len(checkpoints)} scenes: " + ", ".join(c.label for c in checkpoints))
    seeds = range(args.seed_base, args.seed_base + args.samples)
    print(f"playing {len(checkpoints) * len(seeds)} battles (scenario {args.scenario}, opponent {args.opponent}) ...")
    played = play_all(simulator, checkpoints, seeds, args.jobs)
    scenes = [build_scene(c, played[i], args.max_sim_seconds, unit_info) for i, c in enumerate(checkpoints)]

    labels = Labels(width, height, scenes, rosters=not args.no_rosters)
    margin = labels.reserved_margins()
    if args.view:
        view = view_from_view_arg(args.view, width, height, margin)
    else:
        view = fit_view(scenes, width, height, margin)
    renderer = Renderer(width, height, view, args.fps, args.speed, args.glow, args.dot_scale, args.trail)
    show_outcomes = not args.no_outcomes

    if args.stills:
        paths = write_stills(scenes, renderer, labels, args.stills, args.fps, args.speed, show_outcomes)
        print(f"wrote {len(paths)} stills to {args.stills}")
    if args.stills_only:
        return 0
    if not shutil.which(args.ffmpeg) and not Path(args.ffmpeg).is_file():
        sys.exit(f"ffmpeg not found ({args.ffmpeg}); install it or pass --ffmpeg")
    print(f"rendering {out} ({width}x{height} @ {args.fps} fps, {args.speed:g}x) ...")
    frames = render_video(scenes, renderer, labels, out, args.ffmpeg, args.fps, args.speed, args.hold,
                          show_outcomes, args.crf, args.preset)
    print(f"wrote {out}: {frames} frames, {frames / args.fps:.1f} s, {out.stat().st_size / 1e6:.1f} MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
