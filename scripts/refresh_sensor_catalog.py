"""Copy a flat catalog with current BAR sensor properties, preserving its other data.

Always writes a NEW file: active campaigns and immutable packages remain intact.
"""
import argparse
import hashlib
import json
from pathlib import Path

from bar_lua import parse_unit_file
import make_catalog


def refresh(source: Path, destination: Path, bar_root: Path):
    source, destination, bar_root = Path(source), Path(destination), Path(bar_root)
    if destination.exists() or source.resolve() == destination.resolve():
        raise FileExistsError(f"choose a new output catalog: {destination}")
    payload = source.read_bytes()
    document = json.loads(payload)
    units = document.get("unit_catalog")
    if not isinstance(units, dict) or not units:
        raise ValueError("expected a nonempty flat unit_catalog")
    index = {}
    for path in sorted((bar_root / "units").rglob("*.lua")):
        if any(part.lower() in {"scavengers", "other"} for part in path.relative_to(bar_root / "units").parts[:-1]):
            continue
        index.setdefault(path.stem.lower(), []).append(path)
    source_hash = hashlib.sha256()
    for name, unit in sorted(units.items()):
        candidates = index.get(name.lower(), [])
        if len(candidates) != 1:
            raise ValueError(f"{name}: expected one BAR source file; found {len(candidates)}")
        path = candidates[0]
        raw = parse_unit_file(str(path)).get(name.lower())
        if not isinstance(raw, dict):
            raise ValueError(f"{name}: source definition missing")
        unit.update(make_catalog.sensor_fields(raw))
        source_hash.update(name.encode("utf-8") + b"\0" + path.read_bytes())
    provenance = {
        "source_catalog_sha256": hashlib.sha256(payload).hexdigest(),
        "unit_sources_sha256": source_hash.hexdigest(),
        "units": len(units),
        "scope": "Sensor fields only; existing movement, weapons, costs and brain files preserved.",
    }
    document["sensor_provenance"] = provenance
    encoded = (json.dumps(document, indent=2, sort_keys=True, allow_nan=False) + "\n").encode("utf-8")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("xb") as output:
        output.write(encoded)
    return {"path": str(destination), **provenance}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--bar-root", type=Path, default=Path(make_catalog.BAR))
    args = parser.parse_args()
    try:
        result = refresh(args.source, args.out, args.bar_root)
    except (ValueError, OSError) as error:
        parser.exit(1, f"sensor catalog error: {error}\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
