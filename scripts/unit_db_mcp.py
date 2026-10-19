#!/usr/bin/env python3
"""MCP server (stdio) for the BAR unit database in data/units/bar_units.json.

No dependencies beyond the Python standard library. Claude Code starts it from .mcp.json and
Codex from .codex/config.toml; both run `python scripts/unit_db_mcp.py` in the repo root.
See docs/UNIT_DB.md for the tools and the record format.
"""
import difflib
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import unit_db  # noqa: E402

SERVER_NAME = "bar-units"
SERVER_VERSION = "1.0.0"
PROTOCOL_VERSIONS = ("2025-06-18", "2025-03-26", "2024-11-05")
DEFAULT_SEARCH_FIELDS = ["codename", "name", "faction", "tech_level", "cost.metal", "cost.energy",
                         "cost.buildtime"]

INSTRUCTIONS = (
    "Beyond All Reason unit facts for Armada, Cortex and Legion units, extracted from the BAR checkout "
    "(units/*.lua, unit scripts, language/en/units.json). Use get_units for one or many units (codenames "
    "like 'cordoom' or English names like 'Bulwark'), search_units to filter, resolve_unit_name when a "
    "name is uncertain, validate_unit_data to check claimed values, and check_db_freshness to confirm "
    "the database still matches the BAR checkout. describe_fields explains units and conventions."
)


# ---------------------------------------------------------------- data access

class UnitStore:
    def __init__(self, path=unit_db.DB_PATH):
        self.path = path
        self._mtime = None
        self.db = None
        self.by_name = {}

    def load(self):
        mtime = os.path.getmtime(self.path)
        if mtime != self._mtime:
            self.db = unit_db.load_db(self.path)
            self._mtime = mtime
            self.by_name = {}
            for code, rec in self.db["units"].items():
                if rec.get("name"):
                    self.by_name.setdefault(rec["name"].lower(), []).append(code)
        return self.db

    @property
    def units(self):
        return self.load()["units"]

    def resolve(self, query):
        """(codename, None) or (None, {"error": ..., "candidates": [...]})."""
        units = self.units
        q = str(query).strip()
        key = q.lower()
        if key in units:
            return key, None
        exact = self.by_name.get(key, [])
        if len(exact) == 1:
            return exact[0], None
        if len(exact) > 1:
            return None, {"error": "ambiguous name", "candidates": [self.brief(c) for c in exact]}
        return None, {"error": "unknown unit", "candidates": [self.brief(c) for c in self.similar(q)]}

    def similar(self, query, limit=8):
        key = query.lower()
        names = list(self.by_name) + list(self.units)
        hits = [n for n in names if key and key in n]
        hits += difflib.get_close_matches(key, names, n=limit, cutoff=0.6)
        out = []
        for n in hits:
            for code in self.by_name.get(n, [n] if n in self.units else []):
                if code not in out:
                    out.append(code)
        return out[:limit]

    def brief(self, code):
        u = self.units[code]
        return {"codename": code, "name": u.get("name"), "faction": u.get("faction"),
                "tech_level": u.get("tech_level"), "description": u.get("description")}


_PART = re.compile(r"([^.\[\]]+)|\[([^\]]*)\]")


def _parts(path):
    out = []
    for m in _PART.finditer(path):
        out.append(m.group(1) if m.group(1) is not None else ("[", m.group(2)))
    return out


_MISSING = object()


def get_path(value, path):
    """Read a dotted path. Lists take [index] (0-based) or [weapon_def]; a bare key maps over a list."""
    for part in _parts(path):
        if isinstance(part, tuple):
            sel = part[1].strip()
            if not isinstance(value, list):
                return _MISSING
            if re.fullmatch(r"-?\d+", sel):
                i = int(sel)
                if not -len(value) <= i < len(value):
                    return _MISSING
                value = value[i]
            else:
                hits = [v for v in value if isinstance(v, dict) and str(v.get("weapon_def", "")).lower() == sel.lower()]
                if not hits:
                    return _MISSING
                value = hits[0]
        elif isinstance(value, dict):
            if part not in value:
                return _MISSING
            value = value[part]
        elif isinstance(value, list):
            if re.fullmatch(r"\d+", part):
                i = int(part)
                if i >= len(value):
                    return _MISSING
                value = value[i]
            else:
                mapped = [get_path(v, part) for v in value]
                if mapped and all(m is _MISSING for m in mapped):
                    return _MISSING
                value = [None if m is _MISSING else m for m in mapped]
        else:
            return _MISSING
    return value


def project(rec, fields):
    if not fields:
        return rec
    out = {}
    for f in fields:
        v = get_path(rec, f)
        out[f] = None if v is _MISSING else v
    return out


# ---------------------------------------------------------------- tools

def _compare(actual, expected, tolerance, rel_tolerance):
    if isinstance(actual, bool) or isinstance(expected, bool):
        return bool(actual) == bool(expected) if isinstance(expected, bool) and isinstance(actual, bool) else (
            str(actual).lower() == str(expected).lower())
    if isinstance(actual, (int, float)) and not isinstance(expected, (list, dict)):
        try:
            exp = float(expected)
        except (TypeError, ValueError):
            return False
        allowed = max(float(tolerance or 0), abs(exp) * float(rel_tolerance or 0), 1e-9)
        return abs(float(actual) - exp) <= allowed
    if isinstance(actual, str) and isinstance(expected, str):
        return actual.strip().lower() == expected.strip().lower()
    return actual == expected


def tool_get_units(store, args):
    names = args.get("names") or ([args["name"]] if args.get("name") else [])
    if not names:
        raise ValueError("pass names (a list) or name")
    found, problems = {}, {}
    for n in names:
        code, err = store.resolve(n)
        if code:
            found[code] = project(store.units[code], args.get("fields"))
        else:
            problems[str(n)] = err
    return {"units": found, "unresolved": problems}


_OPS = {
    "==": lambda a, b: _compare(a, b, 0, 0),
    "!=": lambda a, b: not _compare(a, b, 0, 0),
    "<": lambda a, b: a is not None and float(a) < float(b),
    "<=": lambda a, b: a is not None and float(a) <= float(b),
    ">": lambda a, b: a is not None and float(a) > float(b),
    ">=": lambda a, b: a is not None and float(a) >= float(b),
    "contains": lambda a, b: str(b).lower() in json.dumps(a, ensure_ascii=False).lower(),
    "in": lambda a, b: any(_compare(a, x, 0, 0) for x in (b or [])),
}


def _matches(rec, flt):
    value = get_path(rec, flt["field"])
    if value is _MISSING:
        return False
    op = _OPS[flt.get("op", "==")]
    if isinstance(value, list) and flt.get("op", "==") != "contains":
        return any(_safe(op, v, flt.get("value")) for v in value)
    return _safe(op, value, flt.get("value"))


def _safe(op, a, b):
    try:
        return op(a, b)
    except (TypeError, ValueError):
        return False


def tool_search_units(store, args):
    filters = list(args.get("filters") or [])
    for f in filters:
        if f.get("op", "==") not in _OPS:
            raise ValueError("unknown op %r; use one of %s" % (f.get("op"), ", ".join(_OPS)))
    if args.get("faction"):
        filters.append({"field": "faction", "op": "==", "value": args["faction"]})
    if args.get("tech_level") is not None:
        filters.append({"field": "tech_level", "op": "==", "value": args["tech_level"]})
    if args.get("mobile") is not None:
        filters.append({"field": "mobile", "op": "==", "value": bool(args["mobile"])})
    text = str(args.get("text") or "").lower()
    rows = []
    for code, rec in store.units.items():
        if text and text not in " ".join(str(rec.get(k) or "") for k in ("codename", "name", "description",
                                                                          "summary")).lower():
            continue
        if all(_matches(rec, f) for f in filters):
            rows.append(rec)
    sort_by = args.get("sort_by")
    if sort_by:
        present = [r for r in rows if get_path(r, sort_by) not in (None, _MISSING)]
        missing = [r for r in rows if get_path(r, sort_by) in (None, _MISSING)]
        present.sort(key=lambda r: (lambda v: (0, v) if isinstance(v, (int, float)) else (1, str(v)))(
            get_path(r, sort_by)), reverse=bool(args.get("descending")))
        rows = present + missing  # units without the field always go last
    limit = int(args.get("limit") or 50)
    fields = args.get("fields") or DEFAULT_SEARCH_FIELDS
    return {"total_matches": len(rows), "returned": min(limit, len(rows)),
            "units": [project(r, fields) for r in rows[:limit]]}


def tool_resolve_unit_name(store, args):
    query = args.get("name") or args.get("query")
    if not query:
        raise ValueError("pass name")
    code, err = store.resolve(query)
    if code:
        return {"match": store.brief(code)}
    return dict(err, match=None)


def tool_validate_unit_data(store, args):
    claims = args.get("claims") or []
    if not claims:
        raise ValueError("pass claims: [{unit, field, value}]")
    results, counts = [], {}
    for c in claims:
        unit, field = c.get("unit"), c.get("field")
        res = {"unit": unit, "field": field, "expected": c.get("value")}
        code, err = store.resolve(unit) if unit else (None, {"error": "missing unit"})
        if not code:
            res.update(status="ambiguous_unit" if err.get("error") == "ambiguous name" else "unknown_unit",
                       candidates=err.get("candidates"))
        elif not field:
            res.update(codename=code, status="unknown_field", hint="pass field, e.g. cost.metal")
        else:
            rec = store.units[code]
            actual = get_path(rec, field)
            res.update(codename=code, source_file=rec.get("source_file"))
            if actual is _MISSING:
                res.update(status="unknown_field",
                           hint="see describe_fields; list items use [index] or [weapon_def]")
            else:
                ok = _compare(actual, c.get("value"), c.get("tolerance"), c.get("rel_tolerance"))
                res.update(status="match" if ok else "mismatch", actual=actual)
        counts[res["status"]] = counts.get(res["status"], 0) + 1
        results.append(res)
    out = {"counts": counts, "results": results, "db_source": store.load().get("source")}
    if args.get("check_source"):
        codes = sorted({r["codename"] for r in results if r.get("codename")})
        out["freshness"] = tool_check_db_freshness(store, {"units": codes})
    return out


def tool_check_db_freshness(store, args):
    db = store.load()
    bar_root = args.get("bar_root") or db.get("source", {}).get("bar_root") or unit_db.BAR
    only = None
    if args.get("units"):
        only = set()
        for n in args["units"]:
            code, _ = store.resolve(n)
            only.add(code or str(n).lower())
    fresh = unit_db.build_db(bar_root, only)
    saved = db["units"] if only is None else {k: db["units"].get(k) for k in only if k in db["units"]}
    fresh_units = fresh["units"] if only is None else {k: v for k, v in fresh["units"].items() if k in saved}
    changes = unit_db.compare_units(saved, fresh_units)
    limit = int(args.get("limit") or 100)
    return {
        "up_to_date": not changes,
        "saved_bar_commit": db.get("source", {}).get("commit"),
        "checkout_bar_commit": fresh["source"].get("commit"),
        "units_checked": len(fresh_units) if only is not None else len(fresh["units"]),
        "difference_count": len(changes),
        "differences": [{"path": p, "saved": a, "checkout": b} for p, a, b in changes[:limit]],
        "rebuild_command": "python scripts/unit_db.py build" if changes else None,
    }


def tool_describe_fields(store, args):
    db = store.load()
    sample = next(iter(db["units"].values()))
    return {"field_notes": db.get("field_notes"), "source": db.get("source"), "unit_count": db.get("unit_count"),
            "unparsed_units": db.get("errors"), "record_keys": list(sample),
            "path_syntax": "dotted paths, e.g. cost.metal, sensors.radar_jam, weapons[0].range, "
                           "weapons[atadr].vs.air.dps; a bare key maps over lists (weapons.range)"}


_FIELDS_SCHEMA = {"type": "array", "items": {"type": "string"},
                  "description": "Optional dotted paths to return instead of whole records, e.g. "
                                 "[\"cost.metal\", \"sensors.los\", \"weapons.vs.air.dps\"]."}

TOOLS = [
    {
        "name": "get_units",
        "description": "Full or projected records for one or more BAR units. Accepts codenames (cordoom) "
                       "or English names (Bulwark). Unknown or ambiguous names come back under 'unresolved' "
                       "with candidates instead of a guess.",
        "inputSchema": {"type": "object", "properties": {
            "names": {"type": "array", "items": {"type": "string"}, "description": "Codenames or names."},
            "name": {"type": "string", "description": "A single codename or name (alternative to names)."},
            "fields": _FIELDS_SCHEMA}},
        "handler": tool_get_units,
    },
    {
        "name": "search_units",
        "description": "Filter units and return compact rows. Filters are {field, op, value} with op one of "
                       "==, !=, <, <=, >, >=, contains, in; a filter on a list field (weapons.range) matches "
                       "when any item matches. Example: faction=Cortex, filters=[{field: 'builder.can_resurrect', "
                       "value: true}].",
        "inputSchema": {"type": "object", "properties": {
            "faction": {"type": "string", "enum": ["Armada", "Cortex", "Legion"]},
            "tech_level": {"type": "number", "description": "1, 1.5, 2 or 3"},
            "mobile": {"type": "boolean"},
            "text": {"type": "string", "description": "Substring of codename, name, description or summary."},
            "filters": {"type": "array", "items": {"type": "object", "properties": {
                "field": {"type": "string"}, "op": {"type": "string"}, "value": {}}, "required": ["field"]}},
            "fields": _FIELDS_SCHEMA,
            "sort_by": {"type": "string"},
            "descending": {"type": "boolean"},
            "limit": {"type": "integer", "description": "Default 50."}}},
        "handler": tool_search_units,
    },
    {
        "name": "resolve_unit_name",
        "description": "Map an English unit name or codename to its codename, or list close candidates.",
        "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
        "handler": tool_resolve_unit_name,
    },
    {
        "name": "validate_unit_data",
        "description": "Check claimed unit values against the database. Each claim is {unit, field, value} "
                       "with optional tolerance (absolute) or rel_tolerance (fraction). Returns match, "
                       "mismatch (with the actual value), unknown_unit, ambiguous_unit or unknown_field per "
                       "claim. Set check_source=true to also re-read those units from the BAR checkout and "
                       "report any drift between the database and the source.",
        "inputSchema": {"type": "object", "properties": {
            "claims": {"type": "array", "items": {"type": "object", "properties": {
                "unit": {"type": "string"}, "field": {"type": "string"}, "value": {},
                "tolerance": {"type": "number"}, "rel_tolerance": {"type": "number"}},
                "required": ["unit", "field", "value"]}},
            "check_source": {"type": "boolean"}}, "required": ["claims"]},
        "handler": tool_validate_unit_data,
    },
    {
        "name": "check_db_freshness",
        "description": "Rebuild records from the BAR checkout and diff them against the saved database "
                       "(all units, or the listed ones). Use before relying on the data after BAR updates.",
        "inputSchema": {"type": "object", "properties": {
            "units": {"type": "array", "items": {"type": "string"}},
            "limit": {"type": "integer", "description": "Max differences to list (default 100)."}}},
        "handler": tool_check_db_freshness,
    },
    {
        "name": "describe_fields",
        "description": "Field meanings, units and sources for unit records, plus the BAR commit the data "
                       "came from and the path syntax used by the other tools.",
        "inputSchema": {"type": "object", "properties": {}},
        "handler": tool_describe_fields,
    },
]
_HANDLERS = {t["name"]: t["handler"] for t in TOOLS}


# ---------------------------------------------------------------- JSON-RPC over stdio

def handle(store, msg):
    """Response dict for one request, or None for notifications."""
    method, mid = msg.get("method"), msg.get("id")
    if mid is None:
        return None
    try:
        if method == "initialize":
            requested = (msg.get("params") or {}).get("protocolVersion")
            version = requested if requested in PROTOCOL_VERSIONS else PROTOCOL_VERSIONS[0]
            result = {"protocolVersion": version, "capabilities": {"tools": {"listChanged": False}},
                      "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
                      "instructions": INSTRUCTIONS}
        elif method == "ping":
            result = {}
        elif method == "tools/list":
            result = {"tools": [{k: v for k, v in t.items() if k != "handler"}
                                | {"annotations": {"readOnlyHint": True, "openWorldHint": False}}
                                for t in TOOLS]}
        elif method == "tools/call":
            params = msg.get("params") or {}
            name = params.get("name")
            if name not in _HANDLERS:
                return {"jsonrpc": "2.0", "id": mid, "error": {"code": -32602, "message": "unknown tool %s" % name}}
            try:
                data = _HANDLERS[name](store, params.get("arguments") or {})
                result = {"content": [{"type": "text", "text": json.dumps(data, ensure_ascii=False, indent=1)}],
                          "structuredContent": data, "isError": False}
            except (ValueError, KeyError, TypeError, OSError) as exc:
                result = {"content": [{"type": "text", "text": "%s: %s" % (type(exc).__name__, exc)}],
                          "isError": True}
        elif method in ("resources/list", "prompts/list"):
            result = {method.split("/")[0]: []}
        else:
            return {"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": "method not found: %s" % method}}
    except Exception as exc:  # noqa: BLE001 - report instead of killing the server
        return {"jsonrpc": "2.0", "id": mid, "error": {"code": -32603, "message": "%s: %s" % (type(exc).__name__, exc)}}
    return {"jsonrpc": "2.0", "id": mid, "result": result}


def serve(stdin=None, stdout=None, store=None):
    stdin = stdin or sys.stdin.buffer
    stdout = stdout or sys.stdout.buffer
    store = store or UnitStore()
    for line in stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except ValueError:
            reply = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "parse error"}}
        else:
            batch = msg if isinstance(msg, list) else [msg]
            replies = [r for r in (handle(store, m) for m in batch if isinstance(m, dict)) if r is not None]
            if not replies:
                continue
            reply = replies if isinstance(msg, list) else replies[0]
        stdout.write((json.dumps(reply, ensure_ascii=False) + "\n").encode("utf-8"))
        stdout.flush()


if __name__ == "__main__":
    serve()
