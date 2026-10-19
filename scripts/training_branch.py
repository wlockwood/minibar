"""Export independent training from validated champions; import and assess returns."""
from __future__ import annotations

import argparse
import copy
from contextlib import contextmanager
import json
import math
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import tempfile

import branch_selection as selection
import branch_transfer as transfer
import package_training as package


def source_run(run):
    run = Path(run).resolve()
    with transfer.exclusive_lock(run / ".campaign.lock"):
        payload = transfer.safe_file(run / "state.json").read_bytes()
        state = transfer.read_json(run / "state.json")
    if state.get("format") != "minibar-campaign-state" or state.get("version") != 1:
        raise ValueError("unsupported source checkpoint")
    if not state.get("tracks") or any(t.get("best_report") is None or t.get("initial_report") is None for t in state["tracks"]):
        raise ValueError("source run needs validated champions, not unfinished initialization")
    return {"kind": "campaign_run", "state_sha256": transfer.digest(payload), "backend_identity": state["backend_identity"],
            "completed_generations": state["completed_generations"], "tracks": state["tracks"]}


def native_probe(root, run):
    manifest = transfer.read_json(root / "manifest.json")
    state_path = run / "state.json"; before = state_path.read_bytes() if state_path.exists() else None
    generation = transfer.read_json(state_path)["completed_generations"] if before else 0
    cmd = [str(root / manifest["binary"]), "--train-campaign", str(root / manifest["config"]),
           "--train-out", str(run), "--threads", "1", "--generations", str(min(1_000_000, max(1, generation + 1))),
           "--max-seconds", "0.000000001"]
    if before: cmd.append("--resume")
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode: raise ValueError(f"native checkpoint validation failed: {result.stderr[-2500:]}")
    if before is not None and state_path.read_bytes() != before: raise ValueError("native probe unexpectedly advanced checkpoint")
    return transfer.read_json(state_path)


def validate_cached_work(root, scratch):
    """The native worker validates existing results without running simulations."""
    manifest = transfer.read_json(root / "manifest.json"); cfg = transfer.read_json(root / manifest["config"])
    tracks = {t["name"]: t for t in cfg["tracks"]}; lineage = transfer.read_json(root / "branch.json")
    jobs = {p.stem: p for p in (root / "run/work/jobs").glob("*.json")}
    for identity, path in sorted(jobs.items()):
        job = transfer.read_json(path)
        try:
            purpose = "validation" if job["plan"] == "promotion" else job["plan"]
            plan = tracks[job["track"]][purpose]
            if type(job["stage"]) is not int or job["stage"] < 0: raise ValueError("bad stage")
            stage = plan["stages"][job["stage"]]
        except (KeyError, IndexError, TypeError, ValueError) as error:
            raise ValueError("invalid cached work stage/track") from error
        cached = root / "run/work/results" / path.name
        if not cached.exists():
            # A synthetic EXISTING result forces the worker's validation/reuse
            # path, so checking an unfinished job cannot launch expensive work.
            cached = scratch / path.name
            transfer.write_json(cached, {"format": "minibar-evaluation-result", "version": 1, "job_id": identity,
                "input_identity": lineage["input_identity"], "backend_identity": lineage["backend_identity"],
                "report": {"name": stage["name"], "difficulty": stage["difficulty"], "weight": stage["weight"], "fitness": 0, "details": None}})
        cmd = [str(root / manifest["binary"]), "--campaign-worker", str(path), "--job-out", str(cached), "--campaign-config", str(root / manifest["config"])]
        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode: raise ValueError(f"invalid cached work: {result.stderr[-1500:]}")
    # A crash during post-commit cleanup can leave an orphan result. Preserve it,
    # checking basic metadata; the coordinator validates against a matching job
    # before ever using it. It cannot affect a different job identity.
    for path in (root / "run/work/results").glob("*.json"):
        if path.stem in jobs: continue
        result = transfer.read_json(path); score = result.get("report", {}).get("fitness")
        if result.get("format") != "minibar-evaluation-result" or result.get("version") != 1 or result.get("job_id") != path.stem or result.get("input_identity") != lineage["input_identity"] or result.get("backend_identity") != lineage["backend_identity"] or type(score) not in (int, float) or not math.isfinite(score):
            raise ValueError("invalid orphan cached work result")


def enable_structural_search(config, every):
    """Configure one structural explorer and retain an ordinary weight-training island."""
    if type(every) is not int or every <= 0:
        raise ValueError("structural cadence must be a positive integer")
    for track in config["tracks"]:
        islands = track["islands"]
        anchor = next((i for i in islands if not i.get("structural_search")), None)
        if anchor is None:
            raise ValueError("structural export needs an existing weight-only island")
        explorer = next((i for i in islands if i is not anchor), None)
        if explorer is None:
            explorer = copy.deepcopy(anchor)
            explorer["name"] = "structural" if anchor["name"] != "structural" else "structure"
            islands.append(explorer)
        explorer["restart"] = False
        explorer["follow_champion"] = False
        explorer["structural_search"] = {"every": every}


def export_branch(config, simulator, out, *, from_run=None, best_dir=None, seed=None, source_root=None,
                  structural_every=None, revalidate_backend=False):
    config = transfer.safe_file(config); simulator = transfer.safe_file(simulator)
    out = Path(out).absolute(); archive = out.with_name(out.name + ".zip")
    transfer.refuse_reparse(out); transfer.refuse_reparse(archive)
    if out.exists() or archive.exists(): raise ValueError("export output or ZIP already exists")
    if (from_run is None) == (best_dir is None): raise ValueError("choose exactly one of --from-run or --best-dir")
    source = source_run(from_run) if from_run is not None else selection.load_registry(best_dir)
    changed_backend = source["backend_identity"] != transfer.backend_identity(simulator)
    if changed_backend and not revalidate_backend:
        raise ValueError("source champions use a different executable; use --revalidate-backend to evaluate their weights with this binary in a new branch")
    seed = secrets.randbits(63) if seed is None else seed
    if not isinstance(seed, int) or not 0 <= seed < 1 << 64: raise ValueError("branch seed must fit u64")
    identity = [source["state_sha256"], transfer.digest(config.read_bytes()), seed]
    if structural_every is not None or changed_backend:
        identity.extend([structural_every, transfer.backend_identity(simulator)])
    branch_id = transfer.digest(transfer.encoded(identity))[:24]
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".training-branch-", dir=out.parent) as temporary:
        staging = Path(temporary); built = staging / "package"
        manifest = package.build_package(config, simulator, built, source_root=source_root)
        cfg = transfer.read_json(built / manifest["config"]); origins = {t["name"]: t for t in source["tracks"]}
        if structural_every is not None:
            enable_structural_search(cfg, structural_every)
        active_names = {t["name"] for t in cfg["tracks"]}
        if not active_names or not active_names <= set(origins):
            raise ValueError("target tracks must be a nonempty subset of source tracks")
        cfg["seed"] = seed; cfg["name"] = f"{cfg['name']} / branch-{branch_id}"
        extra = []; selected = []
        parked = sorted(set(origins) - active_names)
        for name in parked:
            if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", name): raise ValueError("unsafe parked track name")
            relative = f"parked/{name}.json"
            transfer.write_json(built / relative, origins[name]["best"])
            extra.append(relative)
        old_seeds = {i["es"]["config"]["seed"] for t in source["tracks"] for i in t.get("islands", [])}
        used = set(old_seeds)
        for index, track in enumerate(cfg["tracks"]):
            name = track["name"]
            if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", name): raise ValueError("unsafe track name")
            best = origins[name]["best"]; relative = f"best/{name}.json"
            transfer.write_json(built / relative, best); extra.append(relative); track["brain"] = relative
            reference = origins[name].get("reference") or origins[name]["base"]
            reference_relative = f"best/{name}.reference.json"
            transfer.write_json(built / reference_relative, reference)
            extra.append(reference_relative)
            track["reference_brain"] = reference_relative
            selected.append({"name": name, "brain_sha256": transfer.digest(transfer.encoded(best)), "kind": best["kind"]})
            for island in track["islands"]:
                derived = int(transfer.digest(transfer.encoded([branch_id, name, island["name"]]))[:16], 16)
                while derived in used: derived = (derived + 1) % (1 << 64)
                used.add(derived); island["es"]["seed"] = derived
            for dependency in manifest["dependencies"]:
                if dependency["owner"] == "campaign.json" and dependency["field"] == f"tracks[{index}].brain": dependency["target"] = relative
                if dependency["owner"] == "campaign.json" and dependency["field"] == f"tracks[{index}].reference_brain": dependency["target"] = reference_relative
            if not any(d["owner"] == "campaign.json" and d["field"] == f"tracks[{index}].reference_brain" for d in manifest["dependencies"]):
                manifest["dependencies"].append({"owner": "campaign.json", "field": f"tracks[{index}].reference_brain", "target": reference_relative})
        transfer.write_json(built / "campaign.json", cfg)
        # The unchanged native binary validates wiring, all inputs and its own ID.
        probe = native_probe(built, staging / "probe")
        if changed_backend:
            # Requalify champion weights; old scores and optimizer state are never reused.
            qualification = {"source_backend_identity": source["backend_identity"],
                             "backend_identity": probe["backend_identity"], "tracks": []}
            for track in cfg["tracks"]:
                report = selection.evaluate(built, {**track, "_native_evaluation": True}, origins[track["name"]]["best"],
                                            track["validation"].get("seeds", 1), 1_000_000_000,
                                            staging / "qualification", track["name"])
                qualification["tracks"].append({"name": track["name"], "report": report})
            transfer.write_json(built / "qualification.json", qualification)
            extra.append("qualification.json")
        lineage = {"format": "minibar.training_branch", "version": 1, "branch_id": branch_id, "seed": seed,
                   "input_identity": probe["input_identity"], "backend_identity": probe["backend_identity"],
                   "source": {k: source[k] for k in ("kind", "state_sha256", "backend_identity")}, "champions": selected,
                   "parked_tracks": parked,
                   "note": "Independent branch: champion weights warm-start fresh optimizers and random streams. Parked champions are preserved, not trained."}
        transfer.write_json(built / "branch.json", lineage); extra.append("branch.json")
        for name in ["Train.ps1", "Return-Results.ps1", "Branch-Common.ps1"]:
            shutil.copyfile(Path(__file__).parent / "training_branch" / name, built / name); extra.append(name)
        (built / "README.md").write_text("""# Independent MiniBAR training branch

Unzip the complete folder on another Windows computer with the same architecture
as the shipped executable. Rust, Python and a BAR checkout are not needed there.

From this folder, in PowerShell:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\Train.ps1 -MaxSeconds 28800

Repeat the command to resume. -Generations N sets a total target (default: the
campaign target). The time budget is cooperative: active jobs finish/cache.
The default uses all available logical processors; use -Threads N to limit it.
Create run/STOP to stop before subsequent jobs; remove it before resuming.
Read run/progress.json for current stage/jobs and run/summary.json for champions.

After the trainer exits:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\Return-Results.ps1

Send the resulting sibling *-results.zip back to the original computer. There:

    python scripts/training_branch.py import RETURN.zip --package ORIGINAL_EXPORT --out data/brains/branches --against-run LOCAL_RUN --best-dir data/brains/current-best

The original export folder or ZIP is required to verify immutable inputs.
Import preserves the complete checkpoint and partial work in a separate branch.
It never combines optimizer states. Champion integration reevaluates both sides
locally, including fresh paired seeds; holdout does not select. Duplicate returns
do not overwrite local progress. Future exports can use --best-dir.

This branch starts from validated champion weights, with new optimizer/random
streams, so it explores different work. A repeated explicit seed intentionally
recreates the same branch. Keep all shipped input files unchanged. Manifest hashes
detect accidental edits for trusted computers; this is not remote authentication.
Numeric equivalence across physical computers has not yet been demonstrated.
""", encoding="utf-8")
        old = {r["path"]: r for r in manifest["files"]}
        manifest["files"] = [{**old.get(name, {}), **transfer.record(name, (built / name).read_bytes())}
                             for name in sorted(set(old) | set(extra))]
        manifest["branch_id"] = branch_id
        transfer.write_json(built / "manifest.json", manifest); transfer.verify_bundle(built)
        built.rename(out)
        transfer.zip_payloads(archive, {**{r["path"]: (out / r["path"]).read_bytes() for r in manifest["files"]},
                                      "manifest.json": (out / "manifest.json").read_bytes()}, manifest["binary"])
    return {"status": "exported", "path": str(out), "zip": str(archive), "branch_id": branch_id, "seed": seed}


@contextmanager
def original_package(path):
    path = Path(path).resolve()
    if path.is_dir():
        transfer.verify_bundle(path); yield path
    else:
        with tempfile.TemporaryDirectory(prefix="minibar-original-") as temporary:
            root = Path(temporary) / "package"; transfer.extract_checked(path, root); transfer.verify_bundle(root); yield root


def import_branch(archive, original, out, *, against_run=None, best_dir=None):
    out = Path(out).absolute(); transfer.refuse_reparse(out)
    source = source_run(against_run) if against_run is not None and (best_dir is None or not (Path(best_dir) / "current.json").exists()) else None
    if best_dir is not None and source is None and not (Path(best_dir) / "current.json").exists():
        raise ValueError("first best-registry integration requires --against-run")
    with original_package(original) as expected, tempfile.TemporaryDirectory(prefix="minibar-return-") as temporary:
        staged = Path(temporary) / "package"; transfer.extract_checked(archive, staged)
        result = transfer.read_json(staged / "return.json")
        manifest = transfer.verify_bundle(staged, expected / "manifest.json")
        lineage = transfer.read_json(expected / "branch.json")
        if result.get("format") != "minibar.branch_return" or result.get("version") != 1 or result.get("branch_id") != lineage["branch_id"]:
            raise ValueError("returned branch identity differs from original export")
        transfer.verify_records(staged, result["files"])
        actual = {p.relative_to(staged).as_posix() for p in staged.rglob("*") if p.is_file()} - {"return.json"}
        if actual != {r["path"] for r in result["files"]}: raise ValueError("return manifest has extra/missing files")
        immutable = {r["path"] for r in manifest["files"]} | {"manifest.json"}
        for name in actual - immutable:
            if name != "run/state.json" and not re.fullmatch(r"run/work/(jobs|results)/fnv1a64-[0-9a-f]{16}\.json", name):
                raise ValueError(f"unexpected return file: {name}")
        payload = (staged / "run/state.json").read_bytes(); identity = transfer.digest(payload)
        if identity != result["state_sha256"]: raise ValueError("state checksum mismatch")
        state = native_probe(staged, staged / "run")
        validate_cached_work(staged, Path(temporary) / "cache-probe")
        if state["completed_generations"] != result["completed_generations"] or state["input_identity"] != lineage["input_identity"] or state["backend_identity"] != lineage["backend_identity"]:
            raise ValueError("returned checkpoint counters/identity differ")
        if not re.fullmatch(r"[0-9a-f]{24}", lineage["branch_id"]): raise ValueError("invalid branch ID")
        # A stopped generation may have the same committed state and hours of
        # additional finished jobs. Those are distinct, resumable snapshots.
        runtime = sorted((r for r in result["files"] if r["path"].startswith("run/")), key=lambda r: r["path"])
        snapshot = transfer.digest(transfer.encoded(runtime))
        destination = out / lineage["branch_id"] / snapshot
        with transfer.exclusive_lock(out / ".import.lock"):
            if destination.exists():
                transfer.verify_bundle(destination, expected / "manifest.json")
                receipt = transfer.read_json(destination / "import.json")
                if receipt["state_sha256"] != identity or receipt["snapshot_sha256"] != snapshot: raise ValueError("existing import receipt differs")
                status = "duplicate"
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                # Copy to an adjacent private folder, then rename on this volume.
                with tempfile.TemporaryDirectory(prefix=".import-", dir=destination.parent) as adjacent:
                    prepared = Path(adjacent) / "package"; shutil.copytree(staged, prepared)
                    receipt = {"branch_id": lineage["branch_id"], "state_sha256": identity, "snapshot_sha256": snapshot,
                               "completed_generations": state["completed_generations"], "selection": None}
                    transfer.write_json(prepared / "import.json", receipt); prepared.rename(destination)
                status = "imported"
            if best_dir is not None and receipt.get("selection") is None:
                incoming = {**state, "state_sha256": identity}
                receipt["selection"] = selection.integrate(destination, incoming, source, best_dir, destination / "assessment")
                transfer.write_json(destination / "import.json", receipt)
        return {"status": status, "path": str(destination), "branch_id": lineage["branch_id"],
                "completed_generations": receipt["completed_generations"], "selection": receipt.get("selection")}


def main():
    parser = argparse.ArgumentParser(description=__doc__); commands = parser.add_subparsers(dest="command", required=True)
    export = commands.add_parser("export", help="create independent training from validated current best")
    export.add_argument("--config", type=Path, default=Path("examples/campaign/replay-informed.json"))
    export.add_argument("--sim", type=Path, default=Path("target/release/visigoth-simulator.exe"))
    origin = export.add_mutually_exclusive_group(required=True); origin.add_argument("--from-run", type=Path); origin.add_argument("--best-dir", type=Path)
    export.add_argument("--out", type=Path, required=True); export.add_argument("--seed", type=int); export.add_argument("--source-root", type=Path)
    export.add_argument("--structural-every", type=int, metavar="GENERATIONS", help="explore one inherited structural edit per interval")
    export.add_argument("--revalidate-backend", action="store_true", help="reevaluate old champion weights with this executable in a fresh branch")
    inbound = commands.add_parser("import", help="verify a return and optionally integrate its champions")
    inbound.add_argument("archive", type=Path); inbound.add_argument("--package", type=Path, required=True)
    inbound.add_argument("--out", type=Path, default=Path("data/brains/branches")); inbound.add_argument("--against-run", type=Path); inbound.add_argument("--best-dir", type=Path)
    args = parser.parse_args()
    try:
        result = export_branch(args.config, args.sim, args.out, from_run=args.from_run, best_dir=args.best_dir, seed=args.seed, source_root=args.source_root, structural_every=args.structural_every, revalidate_backend=args.revalidate_backend) if args.command == "export" else import_branch(args.archive, args.package, args.out, against_run=args.against_run, best_dir=args.best_dir)
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error: parser.exit(1, f"training branch error: {error}\n")
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == "__main__": main()
