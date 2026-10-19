"""Checked files, snapshots and archives for trusted portable training branches."""
from __future__ import annotations

from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import tempfile
import zipfile

from package_training import digest, encoded, refuse_reparse, safe_file


def parse_json(payload):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result: raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result
    def invalid(value): raise ValueError(f"nonfinite JSON value: {value}")
    return json.loads(payload.decode("utf-8-sig"), object_pairs_hook=pairs, parse_constant=invalid)


def read_json(path):
    return parse_json(Path(path).read_bytes())


def write_json(path, value):
    path = Path(path); path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".branch-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(encoded(value)); stream.flush(); os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary): os.unlink(temporary)


@contextmanager
def exclusive_lock(path):
    path = Path(path); refuse_reparse(path); path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a+b") as stream:
        try:
            if os.name == "nt":
                import msvcrt
                stream.seek(0); msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            raise ValueError(f"active trainer/import holds lock; stop it first: {path}") from error
        try: yield
        finally:
            if os.name == "nt":
                stream.seek(0); msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
            else: fcntl.flock(stream.fileno(), fcntl.LOCK_UN)


def relative_name(value):
    if not isinstance(value, str) or not value or any(c in value for c in "\\:\x00"):
        raise ValueError(f"unsafe archive path: {value!r}")
    parts = value.split("/")
    reserved = re.compile(r"^(con|prn|aux|nul|com[1-9]|lpt[1-9])$", re.I)
    if value.startswith("/") or any(p in ("", ".", "..") or p.endswith((".", " ")) or reserved.match(p.split(".")[0]) for p in parts):
        raise ValueError(f"unsafe archive path: {value!r}")
    return PurePosixPath(value)


def extract_checked(archive, out):
    out = Path(out)
    if out.exists(): raise ValueError(f"extraction output exists: {out}")
    with zipfile.ZipFile(archive) as zipped:
        files = []; seen = set(); size = 0
        for info in zipped.infolist():
            # ZipInfo normalizes Windows separators and truncates NUL names.
            # Validate the original header spelling before using that result.
            original = info.orig_filename.rstrip("/") if info.is_dir() else info.orig_filename
            relative_name(original)
            name = info.filename.rstrip("/") if info.is_dir() else info.filename
            relative_name(name)
            folded = name.casefold()
            if folded in seen: raise ValueError("duplicate/case-aliased ZIP paths")
            seen.add(folded)
            kind = stat.S_IFMT(info.external_attr >> 16)
            if kind not in (0, stat.S_IFREG, stat.S_IFDIR): raise ValueError("unsafe ZIP file type/symlink")
            size += info.file_size
            if size > 2 * 1024**3 or info.file_size > 1024**3 or len(seen) > 100_000:
                raise ValueError("training ZIP exceeds file/size limits")
            if not info.is_dir(): files.append(info)
        # Reject a file that is also another file's parent before making output.
        names = {i.filename.casefold() for i in files}
        for info in files:
            if any(str(p).casefold() in names for p in PurePosixPath(info.filename).parents if str(p) != "."):
                raise ValueError("ZIP file/directory aliases")
        out.mkdir(parents=True)
        for info in files:
            path = out / info.filename; path.parent.mkdir(parents=True, exist_ok=True)
            with zipped.open(info) as source, path.open("xb") as target:
                import shutil
                shutil.copyfileobj(source, target)
            if os.name != "nt": path.chmod((info.external_attr >> 16) & 0o777 or 0o644)


def record(name, payload):
    relative_name(name)
    return {"path": name, "sha256": digest(payload), "bytes": len(payload)}


def verify_records(root, records):
    seen = set()
    for item in records:
        name = str(relative_name(item["path"]))
        if name.casefold() in seen: raise ValueError("duplicate manifest paths")
        seen.add(name.casefold())
        path = safe_file(Path(root) / name, Path(root).resolve())
        payload = path.read_bytes()
        if len(payload) != item["bytes"] or digest(payload) != item["sha256"]:
            raise ValueError(f"hash/checksum mismatch: {name}")


def verify_bundle(root, expected=None):
    root = Path(root)
    manifest = read_json(root / "manifest.json")
    if manifest.get("format") != "minibar.training_package" or manifest.get("version") != 1:
        raise ValueError("unsupported package manifest")
    if expected is not None and (root / "manifest.json").read_bytes() != Path(expected).read_bytes():
        raise ValueError("returned manifest differs from original export")
    verify_records(root, manifest["files"])
    names = {item["path"] for item in manifest["files"]}
    if manifest["binary"] not in names or manifest["config"] not in names or "branch.json" not in names:
        raise ValueError("branch manifest lacks binary, config or lineage")
    for field in ("binary", "config"): relative_name(manifest[field])
    return manifest


def backend_identity(path):
    value = 0xcbf29ce484222325
    for byte in Path(path).read_bytes(): value = ((value ^ byte) * 0x100000001b3) & ((1 << 64) - 1)
    return f"fnv1a64-{value:016x}"


def snapshot_files(run):
    run = Path(run)
    files = {"run/state.json": safe_file(run / "state.json").read_bytes()}
    for folder in ("jobs", "results"):
        for path in sorted((run / "work" / folder).glob("*.json")):
            if not re.fullmatch(r"fnv1a64-[0-9a-f]{16}\.json", path.name):
                raise ValueError(f"unexpected work filename: {path.name}")
            files[f"run/work/{folder}/{path.name}"] = safe_file(path).read_bytes()
    return files


def zip_payloads(path, payloads, binary=None):
    path = Path(path); refuse_reparse(path); path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists(): raise ValueError(f"output exists: {path}")
    # Complete bytes are captured before opening output, including a locked state.
    with zipfile.ZipFile(path, "x", zipfile.ZIP_DEFLATED) as zipped:
        for name, payload in sorted(payloads.items()):
            relative_name(name)
            info = zipfile.ZipInfo(name); info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (0o100755 if name == binary else 0o100644) << 16
            zipped.writestr(info, payload)


def return_results(package, out):
    """Python equivalent of Return-Results.ps1, useful for automation and tests."""
    package = Path(package).resolve(); manifest = verify_bundle(package)
    lineage = read_json(package / "branch.json")
    with exclusive_lock(package / "run/.campaign.lock"):
        state_files = snapshot_files(package / "run")
        state = read_json(package / "run/state.json")
        if state["input_identity"] != lineage["input_identity"] or state["backend_identity"] != lineage["backend_identity"]:
            raise ValueError("checkpoint identity differs from exported branch")
        payloads = {r["path"]: (package / r["path"]).read_bytes() for r in manifest["files"]}
        payloads["manifest.json"] = (package / "manifest.json").read_bytes()
        payloads.update(state_files)
        result = {"format": "minibar.branch_return", "version": 1, "branch_id": lineage["branch_id"],
                  "state_sha256": digest(state_files["run/state.json"]), "completed_generations": state["completed_generations"],
                  "files": [record(n, p) for n, p in sorted(payloads.items())]}
        payloads["return.json"] = encoded(result)
        zip_payloads(out, payloads, manifest["binary"])
    return result
