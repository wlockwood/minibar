"""Bundle a campaign, native simulator, and all runtime file dependencies.

No BAR checkout is needed after relocation. The native executable requires the
same operating system and architecture as the machine where it was built.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import shutil
import stat
import sys
import tempfile
import zipfile

PATH_KEYS = frozenset({"catalog", "config", "brain", "reference_brain", "init_from", "companion_brain", "map"})


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(document):
    return (json.dumps(document, indent=2, sort_keys=True, allow_nan=False) + "\n").encode("utf-8")


def refuse_reparse(path):
    """Check lexical ancestors before resolve() can hide a symlink/junction."""
    path = Path(path).absolute()
    for part in (path, *path.parents):
        try:
            info = part.lstat()
        except FileNotFoundError:
            continue
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 1024):
            raise ValueError(f"symlink/reparse path is unsafe to package: {part}")


def safe_file(path, root=None):
    path = Path(path).absolute()
    refuse_reparse(path)
    resolved = path.resolve()
    if root is not None:
        try:
            relative = resolved.relative_to(root)
        except ValueError as error:
            raise ValueError(f"dependency escapes outside source root: {path}") from error
        if any(p in (".git", ".aws", ".codex", ".agents") for p in relative.parts):
            raise ValueError(f"private workspace metadata is not a runtime dependency: {path}")
    if not resolved.is_file():
        raise ValueError(f"missing input file: {path}")
    return resolved


def find_root(config):
    for parent in (config.parent, *config.parent.parents):
        if (parent / ".git").exists():
            return parent
    return config.parent


def build_package(config, sim, out, *, source_root=None, zip_output=None):
    config = safe_file(config)
    root = Path(source_root).resolve() if source_root is not None else find_root(config)
    config = safe_file(config, root)
    simulator = safe_file(sim)
    out = Path(out).absolute()
    refuse_reparse(out)
    if out.exists():
        raise ValueError(f"output already exists: {out}")
    zip_path = Path(zip_output).absolute() if zip_output is not None else None
    if zip_path is not None:
        refuse_reparse(zip_path)
        if zip_path.exists():
            raise ValueError(f"zip output already exists: {zip_path}")
        if zip_path == out or out in zip_path.parents:
            raise ValueError("zip output must be outside the package directory")

    destinations = {config: Path("campaign.json")}
    original, rendered, active = {}, {}, set()
    dependencies = []

    def visit(path):
        path = safe_file(path, root)
        if path in active:
            raise ValueError(f"cyclic configuration dependency: {path.relative_to(root)}")
        if path in rendered:
            return destinations[path]
        destination = destinations.setdefault(path, Path("assets") / path.relative_to(root))
        payload = original[path] = path.read_bytes()
        active.add(path)

        def dependency(value, field):
            candidate = Path(value)
            source = safe_file(candidate if candidate.is_absolute() else path.parent / candidate, root)
            target = visit(source)
            rewritten = Path(os.path.relpath(target, destination.parent)).as_posix()
            dependencies.append({"owner": destination.as_posix(), "field": field,
                                 "target": target.as_posix()})
            return rewritten

        def rewrite(value, field=""):
            if isinstance(value, dict):
                result = {}
                for key, item in value.items():
                    label = f"{field}.{key}" if field else key
                    if key in PATH_KEYS and isinstance(item, str):
                        if not item.strip():
                            raise ValueError(f"empty dependency path: {path}:{label}")
                        result[key] = dependency(item, label)
                    elif key == "opponents" and isinstance(item, list):
                        result[key] = ["brain:" + dependency(opponent[6:], f"{label}[{index}]")
                                       if isinstance(opponent, str) and opponent.startswith("brain:") else rewrite(opponent, label)
                                       for index, opponent in enumerate(item)]
                    elif key == "elite_brains" and isinstance(item, list):
                        if any(not isinstance(brain, str) or not brain.strip() for brain in item):
                            raise ValueError(f"invalid elite brain path: {path}:{label}")
                        result[key] = [dependency(brain, f"{label}[{index}]") for index, brain in enumerate(item)]
                    else:
                        result[key] = rewrite(item, label)
                return result
            if isinstance(value, list):
                return [rewrite(item, f"{field}[{index}]") for index, item in enumerate(value)]
            return value

        if path.suffix.lower() == ".json":
            try:
                document = json.loads(payload.decode("utf-8-sig"))
            except (ValueError, UnicodeError) as error:
                raise ValueError(f"invalid JSON dependency {path}: {error}") from error
            rendered[path] = encoded(rewrite(document))
            if path.name.endswith(".map_source.json"):
                sidecar = path.with_name(path.name[:-len(".map_source.json")] + ".acquisition.json")
                if sidecar.exists():
                    # CompiledMap discovers this exact sibling name automatically.
                    visit(sidecar)
        else:
            rendered[path] = payload
        active.remove(path)
        return destination

    visit(config)  # Complete all dependency/security checks before writing output.
    binary_name = simulator.name
    if any(c in binary_name for c in "'\"`$"):
        raise ValueError("unsafe simulator filename")
    binary = Path("bin") / binary_name
    files = {destinations[source]: payload for source, payload in rendered.items()}
    files[binary] = simulator.read_bytes()
    readme = f"""# Portable MiniBAR campaign

This bundle contains its campaign, stage configs, catalogs, brains, maps and native
simulator. BAR source and the original checkout are not needed. Use the same OS
and architecture as this executable: {sys.platform} / {platform.machine()}.

From this directory in PowerShell:

```powershell
& './{binary.as_posix()}' --train-campaign './campaign.json' --train-out './runs/default' --threads 4
& './{binary.as_posix()}' --train-campaign './campaign.json' --train-out './runs/default' --resume
& './{binary.as_posix()}' --campaign-worker './jobs/job.json' --job-out './results/result.json' --campaign-config './campaign.json'
```

The worker consumes a coordinator-produced job. Its referenced files must also be
transferred; this package provides runtime assets, not a network scheduler. Native
campaign stages require compiled `.vsgmap` maps, avoiding implicit acquisition
sidecars. To use manual workers, start the coordinator with `--external`; it
returns `waiting_for_workers` and writes jobs under `runs/default/work/jobs`.
Write each result to `runs/default/work/results/JOB_ID.json`, transfer it back,
then rerun the coordinator with `--resume --external`. Repeat for later stages.
Use `--campaign-config` to address the local campaign after relocating a job.

Resume requires the same executable and dependency contents. Training seed ranges
change per generation; validation and holdout use separate fixed seed ranges.
The normal profile also requires fresh paired promotion checks before replacing
its champion. These checks use a separate generation-specific seed range.
Changing a seed/config/input requires a new experiment; the total generation
target may increase. Very short smoke stages can stop with partial construction.

Read RUN_DIRECTORY/progress.json for current stage/job counts and elapsed time;
summary.json reports committed champions. To stop cooperatively, create a STOP
file in the run directory or add --max-seconds N. Checks occur before stages and
job launches, including initial validation and holdout. Active jobs finish/cache,
so the time budget can overrun. Partial generations keep their completed work;
remove STOP and resume with the same inputs to reuse it exactly.

The SHA256 manifest covers every shipped file except the manifest itself. Source and
packaged hashes differ when JSON paths are rewritten. Generated results belong
under `runs/`; package input files must stay unchanged when resuming.
"""
    files[Path("README.md")] = readme.encode()
    manifest_files = []
    reverse = {destination: source for source, destination in destinations.items()}
    for destination, payload in sorted(files.items(), key=lambda item: item[0].as_posix()):
        record = {"path": destination.as_posix(), "sha256": digest(payload), "bytes": len(payload)}
        if destination in reverse:
            source = reverse[destination]
            record.update({"source": source.relative_to(root).as_posix(), "source_sha256": digest(original[source])})
        elif destination == binary:
            record["source_sha256"] = digest(payload)
        manifest_files.append(record)
    manifest = {"format": "minibar.training_package", "version": 1, "config": "campaign.json",
                "binary": binary.as_posix(), "platform": sys.platform, "architecture": platform.machine(),
                "files": manifest_files, "dependencies": dependencies}
    files[Path("manifest.json")] = encoded(manifest)
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=out.parent, prefix=f".{out.name}.staging-") as temporary:
        staged = Path(temporary) / "package"
        staged.mkdir()
        for relative, payload in files.items():
            target = staged / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(payload)
        (staged / binary).chmod(simulator.stat().st_mode & 0o777)
        # rename fails if another writer has claimed the output in the meantime.
        staged.rename(out)
    if zip_path is not None:
        zip_path.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(zip_path, "x", zipfile.ZIP_DEFLATED) as archive:
            for relative in sorted(files, key=lambda path: path.as_posix()):
                archive.write(out / relative, relative.as_posix())
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=Path("examples/campaign/replay-informed.json"))
    parser.add_argument("--sim", type=Path, default=Path("target/release/visigoth-simulator.exe"))
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--source-root", type=Path, help="dependency boundary; defaults to config's enclosing Git root")
    parser.add_argument("--zip", action="store_true", help="also write OUT.zip next to the package")
    args = parser.parse_args()
    try:
        manifest = build_package(args.config, args.sim, args.out, source_root=args.source_root,
                                 zip_output=args.out.with_name(args.out.name + ".zip") if args.zip else None)
    except (OSError, ValueError) as error:
        parser.exit(1, f"package error: {error}\n")
    print(f"Packaged {len(manifest['files'])} files in {args.out}")


if __name__ == "__main__":
    main()
