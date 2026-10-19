#!/usr/bin/env python3
"""BAR unit database: one JSON record per Armada, Cortex and Legion unit.

The records are read from a Beyond All Reason checkout (unit .lua files, unit
scripts and language/en/units.json) and are meant for agents to look up or
validate unit facts. scripts/unit_db_mcp.py serves them over MCP.

    python scripts/unit_db.py build            # (re)write data/units/bar_units.json
    python scripts/unit_db.py check            # exit 1 if the JSON differs from the checkout
    python scripts/unit_db.py show cordoom     # print one record

Values are the def-file values after the parts of BAR's default post-processing
that touch these fields (gamedata/alldefs_post.lua with no mod options) and the
engine's defaults (RecoilEngine UnitDef.cpp / WeaponDef.cpp). FIELD_NOTES below
says where each field comes from; it is copied into the JSON so readers of the
JSON alone can see it too.
"""
import argparse
import json
import math
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from bar_lua import parse_unit_file, parse_unit_text  # noqa: E402

BAR = r"D:\code\BAR\Beyond-All-Reason"
DB_PATH = os.path.join(os.path.dirname(HERE), "data", "units", "bar_units.json")
SCHEMA_VERSION = 2

GAME_SPEED = 30  # sim frames per second
CIRCLE_DIVS = 65536  # engine heading units per full turn
FOOTPRINT_ELMOS = 16  # one footprint square = 2 map squares of 8 elmos
FACTIONS = {"arm": "Armada", "cor": "Cortex", "leg": "Legion"}
SKIPPED_TOP_DIRS = ("scavengers", "other")  # scavenger copies, raptors, critters, debug/event units

# Which damage-table column each target type reads (gamedata/armordefs.lua).
# No faction weapon sets `standard`, so ground and surface units take `default`.
TARGET_ARMOR = {"land": "default", "air": "vtol", "sub": "subs"}

# Categories a typical target of each type carries after alldefs_post.lua adds
# them; a weapon can aim at the type when its onlytargetcategory names any one.
TARGET_CATEGORIES = {
    "land": {"ALL", "MOBILE", "NOTMOBILE", "WEAPON", "NOWEAPON", "NOTAIR", "HOVER", "NOTHOVER", "SHIP",
             "NOTSHIP", "NOTSUB", "CANBEUW", "SURFACE", "EMPABLE", "MINE", "COMMANDER", "GROUNDSCOUT"},
    "air": {"ALL", "MOBILE", "WEAPON", "NOWEAPON", "VTOL", "NOTHOVER", "NOTSHIP", "NOTSUB",
            "LIGHTAIRSCOUT", "T4AIR"},
    "sub": {"ALL", "MOBILE", "WEAPON", "NOWEAPON", "NOTAIR", "NOTHOVER", "NOTSHIP", "UNDERWATER", "CANBEUW"},
}

FIELD_NOTES = {
    "units": "Keyed by codename (the unit file name and UnitDefs key).",
    "name": "English display name from language/en/units.json (customparams.i18nfromunit is followed).",
    "tech_level": "customparams.techlevel; alldefs_post.lua sets 1 when absent. 1.5 marks T1.5 units.",
    "cost": "metal = metalcost, energy = energycost, buildtime = buildtime (the build power cost: "
            "seconds to build with 1 build power).",
    "builder.buildpower": "workertime: build power. 0 for non-builders.",
    "builder.can_build": "Engine builder flag (builder and workertime > 0) and at least one build option.",
    "builder.can_assist/can_repair/can_reclaim": "Engine defaults to the builder flag; the def can override. "
            "Repair and reclaim also need repairspeed/reclaimspeed > 0 (default workertime).",
    "builder.can_resurrect": "canresurrect defaults to false, and resurrectspeed (default workertime) must be > 0.",
    "stealth": "radar stealth (stealth); sonar_stealth is listed separately.",
    "cloak": "cost_static = cloakcost and cost_moving = cloakcostmoving (default cloakcost), both energy per "
             "second; decloak_range = mincloakdistance (elmos). can_cloak defaults to cloakcost != 0.",
    "sensors": "Radii in elmos. air_los defaults to 1.5 x los (engine).",
    "movement": "null for units that cannot move. move_type is ground, hover_air (aircraft that build, transport "
                "or have hoverattack, i.e. gunships: they steer by turn_rate at all times) or strafe_air (fighters, "
                "bombers and scouts: the engine ignores turnrate and steers them by turn_radius, in elmos; default "
                "500, doubled for bombers). speed is elmos/s. acceleration and brake are the def values in "
                "elmos/frame^2 (engine unit) with elmos/s^2 conversions; strafe_air units with a weapon default "
                "to 0.065. turn_rate is heading units (65536 per turn) per frame and turn_rate_deg_s converts it; "
                "both are null for strafe_air, and turn_radius is null otherwise.",
    "size": "radius is the circle with the same area as the unit's XZ outline (height ignored): from the "
            "collision volume when the def sets one, otherwise from the footprint (16 elmos per square).",
    "wreck": "null when the unit leaves no corpse. metal, energy and health are the corpse's (the wreck); chain "
             "lists the corpse then what it decays into (usually the heap) with each one's metal, energy, health, "
             "reclaimable and resurrectable. alldefs_post.lua sets non-commander dead/heap health to the unit's "
             "health and metal to floor(metalcost x 0.6) / floor(metalcost x 0.25); commanders keep their file "
             "values. resurrectable follows the engine: feature resurrectable -1 (default) means only the "
             "unit's own corpse.",
    "weapons": "One entry per weapons[] mount, in mount order. Death explosions and weapon defs that no "
               "mount uses are left out.",
    "weapons[].reload_s": "reloadtime rounded down to whole frames, as alldefs_post.lua does.",
    "weapons[].aoe": "radius = areaofeffect / 2 (the def value is a diameter). Engine falloff at distance d "
                     "from the target's surface: (R - d) / (R - d * edge). half_damage_radius is where that "
                     "reaches 0.5, R / (2 - edge). effective_radius is the radius of a full-damage disc that "
                     "deals the same total area damage. impact_only weapons have no splash. BeamLasers with "
                     "impactonly are rewritten by alldefs_post.lua to areaofeffect 11 and edge 1.",
    "weapons[].vs": "Per target type (land = ground and surface-water units, air = aircraft, sub = submarines). "
                    "damage is per projectile hit, read from damage.<armor> with damage.default as fallback "
                    "(land: default, air: vtol, sub: subs). salvo = damage x burst x projectiles. "
                    "dps = salvo / reload_s. range is the weapon range (null when it cannot target that type). "
                    "can_target uses onlytargetcategory, and sub also needs waterweapon "
                    "(engine: non-water weapons cannot hit underwater targets from above water).",
    "weapons[].arc": "Mount aim limits from weapons[].maindir and maxangledif (default 360 = no limit). "
                     "Angles in degrees relative to the unit's front, positive towards the unit's left (x+). "
                     "Scripts may limit aim further.",
    "weapons[].turret_turn": "Present when the weapon has turret = true. yaw_deg_s and pitch_deg_s come from "
                             "the unit script's AimWeapon function (COB turn speed <x> is degrees per second; "
                             "accelerated turrets use MAX_AIMY<n>_VELOCITY x 30). method 'unresolved' means "
                             "the script computes the speed in a way this reader does not follow.",
    "combat": "dps is the sum of weapons[].vs.<type>.dps over mounts that fire on their own (excludes "
              "manual-fire, stockpiled, interceptor, shield, bogus and zero-damage mounts). max_range is the "
              "longest of those mounts' ranges per type. dps_by_range lists one band per distinct range, longest "
              "first, with that band's dps, the mounts in it, and cumulative_dps: the DPS of every counted mount "
              "that reaches that distance.",
    "summary": "One-line plain-text description for search/RAG indexing.",
}


# ---------------------------------------------------------------- source files

def unit_sources(bar_root):
    """{codename: path} for every faction unit file outside the scavenger and other folders."""
    root = os.path.join(bar_root, "units")
    found = {}
    for dirpath, _, files in os.walk(root):
        rel = os.path.relpath(dirpath, root).replace("\\", "/")
        if rel.split("/")[0].lower() in SKIPPED_TOP_DIRS:
            continue
        for f in files:
            name = f[:-4].lower()
            if f.endswith(".lua") and name[:3] in FACTIONS and not name.endswith("_scav"):
                found[name] = os.path.join(dirpath, f)
    return dict(sorted(found.items()))


def load_language(bar_root):
    path = os.path.join(bar_root, "language", "en", "units.json")
    with open(path, encoding="utf-8") as handle:
        units = json.load(handle)["units"]
    return units.get("names", {}), units.get("descriptions", {})


def load_armor_classes(bar_root):
    """{codename: armor class} from gamedata/armordefs.lua's literal lists."""
    with open(os.path.join(bar_root, "gamedata", "armordefs.lua"), encoding="utf-8") as handle:
        text = handle.read()
    body = text.split("local scavArmorDefs")[0]
    classes = {}
    for m in re.finditer(r"^\t(\w+) = \{(.*?)^\t\}", body, re.S | re.M):
        for unit in re.findall(r'"([^"]+)"', m.group(2)):
            classes[unit.lower()] = m.group(1)
    return classes


_modoption_cache = {}


def modoption_defaults(bar_root):
    """{key: Lua literal of its def value} from modoptions.lua."""
    if bar_root not in _modoption_cache:
        with open(os.path.join(bar_root, "modoptions.lua"), encoding="utf-8") as handle:
            text = handle.read()
        defaults = {}
        for m in re.finditer(r'\bkey\s*=\s*"(\w+)"(.*?)(?=\bkey\s*=\s*"|\Z)', text, re.S):
            d = re.search(r"\bdef\s*=\s*([^,\n]+)", m.group(2))
            if d:
                defaults.setdefault(m.group(1).lower(), d.group(1).strip())
        _modoption_cache[bar_root] = defaults
    return _modoption_cache[bar_root]


def read_unit_def(path, bar_root):
    """Parse a unit file, replacing Spring.GetModOptions().<key> with the option's default value."""
    with open(path, encoding="utf-8") as handle:
        text = handle.read()
    defaults = modoption_defaults(bar_root)

    def sub(m):
        return defaults.get(m.group(1).lower(), "nil")
    return parse_unit_text(re.sub(r"Spring\.GetModOptions\(\)\.(\w+)", sub, text), path)


def bar_commit(bar_root):
    try:
        out = subprocess.run(["git", "-C", bar_root, "log", "-1", "--format=%H %cI"],
                             capture_output=True, text=True, check=True).stdout.split()
        return {"commit": out[0], "commit_date": out[1]}
    except (OSError, subprocess.CalledProcessError, IndexError):
        return {"commit": None, "commit_date": None}


_standalone_cache = {}


def standalone_weapon(bar_root, key):
    """A weapon def from BAR's weapons/ folder, for mounts whose def is not in the unit file."""
    if bar_root not in _standalone_cache:
        defs = {}
        for dirpath, _, files in os.walk(os.path.join(bar_root, "weapons")):
            for f in files:
                if f.endswith(".lua"):
                    try:
                        defs.update({k.lower(): v for k, v in parse_unit_file(os.path.join(dirpath, f)).items()})
                    except (ValueError, IndexError):
                        pass
        _standalone_cache[bar_root] = defs
    return _standalone_cache[bar_root].get(key)


# ---------------------------------------------------------------- unit scripts

_SCRIPT_INDEX = {}


def script_index(bar_root):
    if bar_root not in _SCRIPT_INDEX:
        idx = {}
        for dirpath, _, files in os.walk(os.path.join(bar_root, "scripts")):
            for f in files:
                idx.setdefault(f.lower(), os.path.join(dirpath, f))
        _SCRIPT_INDEX[bar_root] = idx
    return _SCRIPT_INDEX[bar_root]


def find_script(bar_root, script_field, codename):
    """Path of the readable source for a unit script (.bos for .cob, or the .lua file)."""
    idx = script_index(bar_root)
    base = os.path.basename(str(script_field or "")).lower()
    if base.endswith(".cob"):
        base = base[:-4] + ".bos"
    if not base:
        base = codename + ".bos"
    return idx.get(base) or idx.get(codename + ".bos") or idx.get(codename + ".lua")


class _Script:
    """Function bodies and #defines of a BOS or Lua unit script, enough to read aim turn speeds."""

    def __init__(self, path, bar_root, variables=None):
        self.path = path
        self.lua = path.lower().endswith(".lua")
        with open(path, encoding="utf-8", errors="replace") as handle:
            text = handle.read()
        self.variables = variables or {}  # script variables set from def values, in degrees per second
        self.defines = {}
        self.functions = {}
        if self.lua:
            text = re.sub(r"--\[\[.*?\]\]|--[^\n]*", "", text, flags=re.S)
            self.functions = self._lua_functions(text)
        else:
            self._read_bos(text, bar_root, depth=0)

    def _read_bos(self, text, bar_root, depth):
        """Collect #defines and functions; the including file's definitions win, like #ifndef guards."""
        text = re.sub(r"/\*.*?\*/|//[^\n]*", "", text, flags=re.S)
        for m in re.finditer(r"^\s*#define\s+(\w+)(?:[ \t]+([^\n]*))?$", text, re.M):
            self.defines.setdefault(m.group(1), (m.group(2) or "").strip())
        for name, body in self._bos_functions(text).items():
            self.functions.setdefault(name, body)
        if depth > 3:
            return
        for inc in re.findall(r'^\s*#include\s+"([^"]+)"', text, re.M):
            path = script_index(bar_root).get(os.path.basename(inc).lower())
            if path:
                with open(path, encoding="utf-8", errors="replace") as handle:
                    self._read_bos(handle.read(), bar_root, depth + 1)

    @staticmethod
    def _bos_functions(text):
        out = {}
        for m in re.finditer(r"^[ \t]*(\w+)\s*\(([^)]*)\)\s*\{", text, re.M):
            depth, i = 1, m.end()
            while i < len(text) and depth:
                depth += {"{": 1, "}": -1}.get(text[i], 0)
                i += 1
            out.setdefault(m.group(1).lower(), text[m.end():i - 1])
        return out

    @staticmethod
    def _lua_functions(text):
        out = {}
        starts = list(re.finditer(r"^function\s+([\w.:]+)\s*\(", text, re.M))
        for i, m in enumerate(starts):
            end = starts[i + 1].start() if i + 1 < len(starts) else len(text)
            name = m.group(1).split(".")[-1].split(":")[-1].lower()
            out.setdefault(name, text[m.end():end])
        return out

    def body(self, name):
        return self.functions.get(name.lower())

    def expand(self, expr, depth=0):
        def sub(m):
            word = m.group(0)
            if word in self.defines and depth < 8:
                return "(" + self.expand(self.defines[word], depth + 1) + ")"
            if self.variables.get(word) is not None:
                return "<%r>" % self.variables[word]
            return word
        return re.sub(r"[A-Za-z_]\w*", sub, expr)

    def degrees(self, expr):
        """Evaluate a turn-speed expression to degrees per second; None if it uses runtime state."""
        expr = expr.strip()
        if self.lua:
            if self.variables.get(expr) is not None:
                return float(self.variables[expr])
            m = re.fullmatch(r"(?:math\.)?rad\(\s*([-\d.eE+]+)\s*\)", expr)
            if m:
                return float(m.group(1))
            if re.fullmatch(r"[-\d.eE+]+", expr):
                return math.degrees(float(expr))
            return None
        expr = self.expand(expr)
        expr = re.sub(r"<\s*([-\d.]+)\s*>", r"(\1)", expr)
        if not re.fullmatch(r"[\d.\s()+\-*/]+", expr):
            return None
        try:
            return float(eval(expr, {"__builtins__": {}}, {}))  # noqa: S307 - digits and operators only
        except (SyntaxError, ZeroDivisionError, TypeError):
            return None


_BOS_TURN = re.compile(r"turn\s+(\w+)\s+to\s+([xyz])-axis\s+([^;]*?)\s+(?:speed\s+([^;]+?)|(now))\s*;", re.I)
_LUA_TURN = re.compile(r"Turn\(\s*(\w+)\s*,\s*(x_axis|y_axis|z_axis|[123])\s*,\s*(.*?)\s*,\s*"
                       r"((?:math\.)?rad\([^)]*\)|[\w.]+)\s*\)")
_LUA_AXIS = {"x_axis": "x", "1": "x", "y_axis": "y", "2": "y", "z_axis": "z", "3": "z"}
_CALLS = re.compile(r"(?:call-script|start-script)\s+(\w+)\s*\(", re.I)
_MACRO_CALL = re.compile(r"\b[A-Z][A-Z0-9_]+\s*\(")
# Accelerated turret helpers in scripts/*.h: (marker in the aim code, max-velocity macro, pitch-speed macro).
# Their velocity macros are degrees per frame; pitch speeds are ordinary turn speeds (degrees per second).
_ACCELERATED = (
    (re.compile(r"aimy(\d+)target\s*=\s*heading", re.I), "MAX_AIMY%s_VELOCITY", "AIMY%s_PITCH_SPEED"),
    (re.compile(r"\bCATT(\d+)_Aim\s*\("), "CATT%s_MAX_VELOCITY", "CATT%s_PITCH_SPEED"),
    (re.compile(r"\bKT(\d+)_SET_TARGET\s*\("), "KT%s_MAX_VELOCITY", "KT%s_PITCH_SPEED"),
)


def script_turret_variables(mounts):
    """Weapon<n>TurretX/Y as common/unit_script_attributes.lua sets them from weapon customparams.

    turretspeedx + turretspeedy (both present) take priority over turretspeed; values are degrees/s.
    Lua unit scripts receive the same numbers in radians, which turret_turn reports back in degrees.
    """
    out = {}
    for n, _, _, wd in mounts:
        cp = (wd or {}).get("customparams") or {}
        if cp.get("turretspeedx") is not None and cp.get("turretspeedy") is not None:
            x, y = _num(cp, "turretspeedx", None), _num(cp, "turretspeedy", None)
        elif cp.get("turretspeed") is not None:
            x = y = _num(cp, "turretspeed", None)
        else:
            continue
        out["Weapon%dTurretX" % n], out["Weapon%dTurretY" % n] = x, y
    return out


def _aim_names(mount):
    names = ["AimWeapon%d" % mount]
    if mount <= 3:
        names.append(("AimPrimary", "AimSecondary", "AimTertiary")[mount - 1])
    return names


def _turns(script, text):
    """(axis, target expression, speed in degrees/s; inf for `now`; None if not a constant) per turn."""
    if script.lua:
        for m in _LUA_TURN.finditer(text):
            yield _LUA_AXIS[m.group(2)], m.group(3), script.degrees(m.group(4))
    else:
        for m in _BOS_TURN.finditer(text):
            yield m.group(2).lower(), m.group(3), (math.inf if m.group(5) else script.degrees(m.group(4)))


def _turn_result(yaw, pitch, method, source):
    def clean(v):
        return None if v is None or math.isinf(v) else _r(v)
    return {"yaw_deg_s": clean(yaw), "pitch_deg_s": clean(pitch), "method": method, "source": source}


def turret_turn(script, mount, script_rel):
    """Yaw/pitch turn speeds of a turret mount, read from the unit script's aim function.

    method: turn-speed (constant `turn ... speed`), accelerated (per-frame velocity helpers),
    instant (`turn ... now`, or an aim function that turns nothing), no-aim-function (the engine
    aims without the script), unresolved (speed computed at runtime in a way not followed here).
    """
    if script is None:
        return _turn_result(None, None, "unresolved", None)
    fname = body = None
    names = _aim_names(mount) + (["AimWeapon"] if script.lua else [])  # Lua scripts may share one function
    for candidate in names:
        if script.body(candidate) is not None:
            fname, body = candidate, script.body(candidate)
            break
    if body is None:
        return _turn_result(None, None, "no-aim-function", script_rel)
    queue, seen = [(fname, body)], set()
    while queue:
        name, text = queue.pop(0)
        seen.add(name.lower())
        source = "%s:%s" % (script_rel, name)
        yaw = pitch = None
        yaw_found = False
        for axis, target, speed in _turns(script, text):
            if axis == "y" and not yaw_found and "head" in target.lower():  # heading, or a stepped nextHead
                yaw, yaw_found = speed, True
            elif axis == "x" and pitch is None and "pitch" in target.lower():
                pitch = speed
        if yaw_found:
            if yaw is None:
                return _turn_result(None, pitch, "unresolved", source)
            result = _turn_result(yaw, pitch, "instant" if math.isinf(yaw) else "turn-speed", source)
            if fname == "AimWeapon":
                result["note"] = "shared Lua AimWeapon(weapon, ...): first heading turn, may differ per weapon"
            return result
        for marker, velocity, pitch_speed in _ACCELERATED:
            m = marker.search(text)
            if m:
                v = script.degrees(velocity % m.group(1))
                if v is None:
                    return _turn_result(None, None, "unresolved", source)
                if pitch is None:
                    pitch = script.degrees(pitch_speed % m.group(1))
                return _turn_result(v * GAME_SPEED, pitch, "accelerated", source)
        if not script.lua:
            for callee in _CALLS.findall(text):
                callee_body = script.body(callee)
                if callee_body is not None and callee.lower() not in seen and len(seen) < 6:
                    queue.append((callee, callee_body))
    if not re.search(r"\bturn\b|Turn\(", body) and not _CALLS.search(body) and not _MACRO_CALL.search(body):
        return _turn_result(math.inf, math.inf, "instant", "%s:%s" % (script_rel, fname))
    return _turn_result(None, None, "unresolved", "%s:%s" % (script_rel, fname))


# ---------------------------------------------------------------- helpers

def _r(value, places=4):
    if value is None or isinstance(value, bool):
        return value
    if isinstance(value, float) and value.is_integer():
        return int(value)
    if isinstance(value, float):
        return round(value, places)
    return value


def _num(raw, key, default=0.0):
    value = raw.get(key)
    if value is None or isinstance(value, bool):
        return default
    try:
        return float(value)
    except (TypeError, ValueError):
        return default


def _bool(raw, key, default=False):
    value = raw.get(key)
    if value is None:
        return default
    if isinstance(value, bool):
        return value
    if isinstance(value, float):
        return value != 0
    return str(value).strip().lower() in ("true", "1")


def _vec(text, default):
    try:
        parts = [float(p) for p in str(text).split()]
        return parts if len(parts) == 3 else default
    except (TypeError, ValueError):
        return default


def frames(seconds):
    """alldefs_post.lua round_to_frames: whole frames, at least one."""
    if seconds is None:
        return None
    return max(1, math.floor((seconds + 1e-3) * GAME_SPEED)) / GAME_SPEED


def aoe_profile(diameter, edge, impact_only):
    radius = diameter / 2.0
    if impact_only or radius <= 0:
        return {"diameter": _r(diameter), "radius": _r(radius), "edge_effectiveness": _r(edge),
                "impact_only": bool(impact_only), "half_damage_radius": 0, "effective_radius": 0}
    edge = min(edge, 1.0)
    half = radius / (2.0 - edge)
    # Area-weighted damage fraction, integrated with Simpson's rule (falloff is smooth on [0, R]).
    n = 400
    h = radius / n
    total = 0.0
    for i in range(n + 1):
        d = i * h
        f = 1.0 if (edge >= 1.0 or d < 1.0) else (radius + 0.001 - d) / (radius + 0.001 - d * edge)
        w = 1 if i in (0, n) else (4 if i % 2 else 2)
        total += w * f * 2 * math.pi * d
    area = total * h / 3
    return {"diameter": _r(diameter), "radius": _r(radius), "edge_effectiveness": _r(edge),
            "impact_only": False, "half_damage_radius": _r(half, 2),
            "effective_radius": _r(math.sqrt(area / math.pi), 2)}


def can_target(only_category, water_weapon, target):
    if target == "sub" and not water_weapon:
        return False
    if not only_category:
        return True
    return bool(set(str(only_category).upper().split()) & TARGET_CATEGORIES[target])


def outline_radius(raw):
    fx, fz = _num(raw, "footprintx", 1), _num(raw, "footprintz", 1)
    footprint_radius = math.sqrt(fx * fz * FOOTPRINT_ELMOS ** 2 / math.pi)
    scales = _vec(raw.get("collisionvolumescales"), None)
    vtype = str(raw.get("collisionvolumetype") or "").lower()
    out = {"footprint_x": _r(fx), "footprint_z": _r(fz), "footprint_radius": _r(footprint_radius, 2),
           "collision_type": vtype or None, "collision_x": None, "collision_z": None, "collision_radius": None}
    if scales:
        x, z = scales[0], scales[2]
        if vtype in ("sphere", "ellipsoid", "cyly", "cylinder"):
            r = math.sqrt(x * z) / 2  # ellipse with diameters x and z
        else:  # box, cylX, cylZ: rectangular XZ outline
            r = math.sqrt(x * z / math.pi)
        out.update(collision_x=_r(x), collision_z=_r(z), collision_radius=_r(r, 2))
    out["radius"] = out["collision_radius"] if out["collision_radius"] is not None else out["footprint_radius"]
    out["radius_source"] = "collision volume" if out["collision_radius"] is not None else "footprint"
    return out


def arc(mount):
    main = _vec(mount.get("maindir"), [0.0, 0.0, 1.0])
    width = min(_num(mount, "maxangledif", 360.0), 360.0)
    heading = math.degrees(math.atan2(main[0], main[2]))
    horiz = math.hypot(main[0], main[2])
    pitch = math.degrees(math.atan2(main[1], horiz))
    unrestricted = width >= 360.0
    return {"unrestricted": unrestricted, "heading_deg": _r(heading, 2), "pitch_deg": _r(pitch, 2),
            "width_deg": _r(width), "min_deg": -180 if unrestricted else _r(heading - width / 2, 2),
            "max_deg": 180 if unrestricted else _r(heading + width / 2, 2)}


# ---------------------------------------------------------------- records

def weapon_record(index, mount, wd, def_key, script, script_rel):
    wtype = str(wd.get("weapontype") or "Cannon")
    damage = {str(k).lower(): _num(wd.get("damage") or {}, k) for k in (wd.get("damage") or {})}
    default_damage = damage.get("default", 0.0)
    impact_only = _bool(wd, "impactonly")
    diameter = _num(wd, "areaofeffect", 8.0)
    edge = _num(wd, "edgeeffectiveness", 0.0)
    if wtype == "BeamLaser" and impact_only:  # alldefs_post.lua weaponDef_Post
        impact_only, diameter, edge = False, 11.0, 1.0
    reload_s = frames(_num(wd, "reloadtime", 1.0))
    burst = int(_num(wd, "burst", 1))
    projectiles = int(_num(wd, "projectiles", 1))
    burst_rate = frames(_num(wd, "burstrate", 0.1))
    only = mount.get("onlytargetcategory")
    water = _bool(wd, "waterweapon")
    cp = wd.get("customparams") or {}
    shield = wtype == "Shield"
    flags = {
        "manual_fire": _bool(wd, "commandfire"),
        "stockpile": _bool(wd, "stockpile"),
        "interceptor": _num(wd, "interceptor", 0) != 0,
        "paralyzer": _bool(wd, "paralyzer"),
        "shield": shield,
        "bogus": _bool(cp, "bogus"),
        "water_weapon": water,
    }
    vs = {}
    for target, armor in TARGET_ARMOR.items():
        dmg = damage.get(armor, default_damage)
        ok = (not shield) and can_target(only, water, target)
        salvo = dmg * burst * projectiles
        vs[target] = {"can_target": ok, "range": _r(_num(wd, "range", 10.0)) if ok else None,
                      "damage": _r(dmg if ok else 0.0, 2),
                      "salvo": _r(salvo if ok else 0.0, 2),
                      "dps": _r(salvo / reload_s if ok and reload_s else 0.0, 2)}
    rec = {
        "mount": index,
        "weapon_def": def_key,
        "name": wd.get("name"),
        "type": wtype,
        "range": _r(_num(wd, "range", 10.0)),
        "reload_s": _r(reload_s),
        "burst": burst,
        "burst_rate_s": _r(burst_rate) if burst > 1 else None,
        "projectiles": projectiles,
        "only_target_category": only,
        "bad_target_category": mount.get("badtargetcategory"),
        **flags,
        "energy_per_shot": _r(_num(wd, "energypershot", 0)),
        "metal_per_shot": _r(_num(wd, "metalpershot", 0)),
        "stockpile_time_s": _r(_num(wd, "stockpiletime", 0)) if flags["stockpile"] else None,
        "aoe": aoe_profile(diameter, edge, impact_only),
        "damage_by_armor": {k: _r(v, 2) for k, v in sorted(damage.items())},
        "vs": vs,
        "turret": _bool(wd, "turret"),
        "arc": arc(mount),
    }
    if rec["turret"]:
        rec["turret_turn"] = turret_turn(script, index, script_rel)
    return rec


BOMBER_WEAPON_TYPES = ("AircraftBomb", "TorpedoLauncher")  # engine WeaponDef::IsAircraftWeapon, simplified


def movement_record(raw, builder, first_weapon):
    """Speed, acceleration and turning, with the engine's choice of aircraft move type.

    Aircraft that build, transport or have hoverattack use HoverAirMoveType and steer with turnrate
    like ground units. Other aircraft (fighters, bombers, scouts) use StrafeAirMoveType, which ignores
    turnrate and steers by turnradius (RecoilEngine UnitDef.cpp, UnitDef.h IsHoveringAirUnit).
    """
    speed = _num(raw, "speed", abs(_num(raw, "maxvelocity", 0)) * GAME_SPEED)
    can_fly = _bool(raw, "canfly")
    if not ((_bool(raw, "canmove") and speed > 0) or can_fly):
        return None
    transport = _num(raw, "transportcapacity", 0) > 0 and _num(raw, "transportmass", 100000) > 0
    hover_attack = _bool(raw, "hoverattack")
    if not can_fly:
        move_type = "ground"
    elif builder or transport or hover_attack:
        move_type = "hover_air"
    else:
        move_type = "strafe_air"
    armed = first_weapon is not None
    bomber = armed and str(first_weapon.get("weapontype") or "") in BOMBER_WEAPON_TYPES
    default_acc = 0.065 if move_type == "strafe_air" and armed else 0.5  # fighter/bomber "engine power"
    acc = _num(raw, "maxacc", default_acc if move_type == "strafe_air" else _num(raw, "acceleration", 0.5))
    dec = _num(raw, "maxdec", _num(raw, "brakerate", acc))
    out = {
        "move_type": move_type,
        "speed": _r(speed),
        "acceleration": _r(acc, 6), "acceleration_elmos_s2": _r(acc * GAME_SPEED ** 2, 3),
        "brake": _r(dec, 6), "brake_elmos_s2": _r(dec * GAME_SPEED ** 2, 3),
        "turn_rate": None, "turn_rate_deg_s": None,
        "turn_radius": None, "turn_radius_source": None,
        "can_fly": can_fly,
        "hover_attack": hover_attack,
        "movement_class": raw.get("movementclass"),
    }
    if move_type == "strafe_air":
        if raw.get("turnradius") is not None:
            out["turn_radius"], out["turn_radius_source"] = _r(_num(raw, "turnradius", 500)), "def"
        elif bomber:
            out["turn_radius"], out["turn_radius_source"] = 1000, "engine default (500, doubled for bombers)"
        else:
            out["turn_radius"], out["turn_radius_source"] = 500, "engine default"
    else:
        turn = _num(raw, "turnrate", 0)
        out["turn_rate"] = _r(turn)
        out["turn_rate_deg_s"] = _r(turn * 360.0 / CIRCLE_DIVS * GAME_SPEED, 2)
    return out


def wreck_record(raw, cp):
    """The corpse feature chain (wreck, then heap) after alldefs_post.lua's wreck standardisation.

    For non-commanders with featuredefs, BAR sets dead/heap health to the unit's health and their
    metal to floor(metalcost x 0.6) and floor(metalcost x 0.25) (wreck_metal_ratio / heap_metal_ratio
    mod options at their defaults) when the unit has both metal and energy costs.
    """
    features = {str(k).lower(): v for k, v in (raw.get("featuredefs") or {}).items()}
    corpse = str(raw.get("corpse") or "").lower()
    if not corpse or corpse not in features:
        return None
    standardize = not _bool(cp, "iscommander") and not _bool(cp, "iseffigy") and raw.get("health") is not None
    has_costs = raw.get("metalcost") is not None and raw.get("energycost") is not None
    ratios = {"dead": 0.6, "heap": 0.25}
    chain, key, level = [], corpse, 0
    while key in features and level < 4:
        fd = features[key]
        health = _num(fd, "health", _num(fd, "damage", 0))
        metal = _num(fd, "metal", 0)
        if standardize and key in ratios:
            health = _num(raw, "health", 100)
            if has_costs:
                metal = math.floor(_num(raw, "metalcost", 0) * ratios[key])
        res = int(_num(fd, "resurrectable", -1))
        chain.append({
            "feature": key,
            "metal": _r(float(metal)),
            "energy": _r(_num(fd, "energy", 0)),
            "health": _r(float(health)),
            "reclaimable": _bool(fd, "reclaimable", not _bool(fd, "indestructible")),
            # engine FeatureHandler: -1 = only the unit's own corpse can be resurrected
            "resurrectable": res > 0 or (res < 0 and level == 0),
            "blocking": _bool(fd, "blocking", True),
        })
        key, level = str(fd.get("featuredead") or "").lower(), level + 1
    return {"metal": chain[0]["metal"], "energy": chain[0]["energy"], "health": chain[0]["health"],
            "chain": chain}


def combat_record(weapons):
    """Total DPS per target type, plus DPS by range so mixed-range loadouts stay visible."""
    out = {"dps": {}, "max_range": {}, "dps_by_range": {}}
    for target in TARGET_ARMOR:
        counted = [w for w in weapons
                   if not (w["manual_fire"] or w["stockpile"] or w["interceptor"] or w["shield"] or w["bogus"])
                   and w["vs"][target]["can_target"] and w["vs"][target]["dps"]]
        by_range = {}
        for w in counted:
            band = by_range.setdefault(w["range"], {"range": w["range"], "dps": 0.0, "mounts": []})
            band["dps"] += w["vs"][target]["dps"]
            band["mounts"].append(w["mount"])
        bands, total = [], 0.0
        for rng in sorted(by_range, reverse=True):  # longest first; cumulative = DPS at that distance
            total += by_range[rng]["dps"]
            bands.append({"range": rng, "dps": _r(by_range[rng]["dps"], 2), "cumulative_dps": _r(total, 2),
                          "mounts": by_range[rng]["mounts"]})
        out["dps"][target] = _r(total, 2)
        out["max_range"][target] = bands[0]["range"] if bands else None
        out["dps_by_range"][target] = bands
    return out


def unit_record(codename, path, bar_root, names, descriptions, armor_classes):
    raw = read_unit_def(path, bar_root)[codename]
    cp = raw.get("customparams") or {}
    rel = os.path.relpath(path, bar_root).replace("\\", "/")
    name_key = str(cp.get("i18nfromunit") or codename).lower()

    workertime = _num(raw, "workertime", 0)
    build_distance = max(38.0, _num(raw, "builddistance", 128.0))
    builder = _bool(raw, "builder") and workertime > 0 and build_distance > 0
    options = [str(o).lower() for _, o in sorted((k, v) for k, v in (raw.get("buildoptions") or {}).items()
                                                    if isinstance(k, float))]
    repair_speed = _num(raw, "repairspeed", workertime)
    reclaim_speed = _num(raw, "reclaimspeed", workertime)
    resurrect_speed = _num(raw, "resurrectspeed", workertime)

    los = _num(raw, "sightdistance", 0)
    cloak_cost = _num(raw, "cloakcost", 0)
    script_path = find_script(bar_root, raw.get("script"), codename)
    script_rel = os.path.relpath(script_path, bar_root).replace("\\", "/") if script_path else None
    script = None
    weapons = []
    weapondefs = {str(k).lower(): v for k, v in (raw.get("weapondefs") or {}).items()}
    mounts = [v for k, v in sorted((k, v) for k, v in (raw.get("weapons") or {}).items() if isinstance(k, float))]
    resolved = []
    for i, mount in enumerate(mounts, start=1):
        key = str(mount.get("def") or "").lower()
        resolved.append((i, mount, key, weapondefs.get(key) or standalone_weapon(bar_root, key)))
    for i, mount, key, wd in resolved:
        if wd is None:
            continue
        if script is None and script_path and _bool(wd, "turret"):
            script = _Script(script_path, bar_root, script_turret_variables(resolved))
        weapons.append(weapon_record(i, mount, wd, key, script, script_rel))

    first_weapon = resolved[0][3] if resolved and resolved[0][3] is not None else None
    movement = movement_record(raw, builder, first_weapon)
    tech = cp.get("techlevel", 1)
    rec = {
        "codename": codename,
        "name": names.get(name_key),
        "description": descriptions.get(name_key),
        "faction": FACTIONS[codename[:3]],
        "tech_level": _r(float(tech)) if isinstance(tech, (int, float)) else tech,
        "unit_group": cp.get("unitgroup"),
        "is_commander": _bool(cp, "iscommander"),
        "source_file": rel,
        "script_file": script_rel,
        "armor_class": armor_classes.get(codename, "default"),
        "health": _r(_num(raw, "health", _num(raw, "maxdamage", 100))),
        "mobile": movement is not None,
        "cost": {"metal": _r(_num(raw, "metalcost", _num(raw, "buildcostmetal", 0))),
                 "energy": _r(_num(raw, "energycost", _num(raw, "buildcostenergy", 0))),
                 "buildtime": _r(_num(raw, "buildtime", 100))},
        "builder": {
            "buildpower": _r(workertime),
            "build_distance": _r(build_distance) if builder else None,
            "can_build": builder and bool(options),
            "can_assist": _bool(raw, "canassist", builder),
            "can_repair": _bool(raw, "canrepair", builder) and repair_speed > 0,
            "can_reclaim": _bool(raw, "canreclaim", builder) and reclaim_speed > 0,
            "can_resurrect": _bool(raw, "canresurrect", False) and resurrect_speed > 0,
            "build_options": options,
        },
        "stealth": _bool(raw, "stealth"),
        "sonar_stealth": _bool(raw, "sonarstealth"),
        "cloak": {
            "can_cloak": _bool(raw, "cancloak", cloak_cost != 0),
            "cost_static": _r(cloak_cost),
            "cost_moving": _r(_num(raw, "cloakcostmoving", cloak_cost)),
            "decloak_range": _r(_num(raw, "mincloakdistance", 0)),
            "decloak_spherical": _bool(raw, "decloakspherical", True),
        },
        "sensors": {
            "los": _r(los),
            "air_los": _r(_num(raw, "airsightdistance", 1.5 * los)),
            "radar": _r(_num(raw, "radardistance", 0)),
            "sonar": _r(_num(raw, "sonardistance", 0)),
            "radar_jam": _r(_num(raw, "radardistancejam", 0)),
            "sonar_jam": _r(_num(raw, "sonardistancejam", 0)),
            "seismic": _r(_num(raw, "seismicdistance", 0)),
        },
        "movement": movement,
        "size": outline_radius(raw),
        "wreck": wreck_record(raw, cp),
        "weapons": weapons,
        "combat": combat_record(weapons),
    }
    rec["summary"] = summarize(rec)
    return rec


def summarize(u):
    tech = {1: "T1", 1.5: "T1.5", 2: "T2", 3: "T3", 4: "T4"}.get(u["tech_level"], "T%s" % u["tech_level"])
    kind = "mobile" if u["mobile"] else "static"
    parts = ["%s (%s): %s %s %s unit" % (u["name"] or u["codename"], u["codename"], u["faction"], tech, kind)]
    if u["description"]:
        parts[0] += ", \"%s\"" % u["description"]
    c = u["cost"]
    parts.append("costs %s metal, %s energy, %s build time; %s health" % (c["metal"], c["energy"], c["buildtime"],
                                                                         u["health"]))
    b = u["builder"]
    abilities = [k[4:] for k in ("can_build", "can_assist", "can_repair", "can_reclaim", "can_resurrect") if b[k]]
    if b["buildpower"]:
        parts.append("%s build power (%s)" % (b["buildpower"], ", ".join(abilities) or "no build abilities"))
    if u["movement"]:
        m = u["movement"]
        turning = ("turn radius %s" % m["turn_radius"] if m["move_type"] == "strafe_air"
                   else "turn %s deg/s" % m["turn_rate_deg_s"])
        verb = {"ground": "moves", "hover_air": "flies (hovers)", "strafe_air": "flies (fixed-wing)"}
        parts.append("%s, speed %s, %s" % (verb[m["move_type"]], m["speed"], turning))
    s = u["sensors"]
    parts.append("LOS %s" % s["los"] + "".join(", %s %s" % (k.replace("_", " "), s[k])
                                                  for k in ("radar", "sonar", "radar_jam", "sonar_jam") if s[k]))
    if u["stealth"]:
        parts.append("radar stealth")
    if u["cloak"]["can_cloak"]:
        cl = u["cloak"]
        parts.append("cloaks for %s E/s still, %s E/s moving, decloak range %s" % (
            cl["cost_static"], cl["cost_moving"], cl["decloak_range"]))
    combat = u["combat"]
    for target in ("land", "air", "sub"):
        bands = combat["dps_by_range"][target]
        if bands:
            parts.append("%s DPS %s up to range %s" % (target, combat["dps"][target], bands[0]["range"]) + (
                " (%s)" % ", ".join("%s within %s" % (b["cumulative_dps"], b["range"]) for b in bands[1:])
                if len(bands) > 1 else ""))
    if u["wreck"]:
        parts.append("wreck %s metal, %s health" % (u["wreck"]["metal"], u["wreck"]["health"]))
    if u["weapons"]:
        parts.append("weapons: " + "; ".join(
            "%s (%s, range %s)" % (w["name"] or w["weapon_def"], w["type"], w["range"]) for w in u["weapons"]))
    parts.append("radius %s" % u["size"]["radius"])
    return ". ".join(parts) + "."


def build_db(bar_root=BAR, only=None):
    names, descriptions = load_language(bar_root)
    armor = load_armor_classes(bar_root)
    sources = unit_sources(bar_root)
    units, errors = {}, {}
    for codename, path in sources.items():
        if only is not None and codename not in only:
            continue
        try:
            units[codename] = unit_record(codename, path, bar_root, names, descriptions, armor)
        except Exception as exc:  # noqa: BLE001 - recorded in the output, not hidden
            errors[codename] = "%s: %s" % (type(exc).__name__, exc)
    return {
        "schema_version": SCHEMA_VERSION,
        "generated_by": "scripts/unit_db.py build",
        "source": dict(bar_commit(bar_root), bar_root=bar_root),
        "unit_count": len(units),
        "field_notes": FIELD_NOTES,
        "errors": errors,
        "units": units,
    }


def diff(old, new, path=""):
    """Leaf-level differences between two JSON values, as (path, old, new)."""
    if isinstance(old, dict) and isinstance(new, dict):
        out = []
        for k in sorted(set(old) | set(new), key=str):
            out += diff(old.get(k, "<missing>"), new.get(k, "<missing>"), "%s.%s" % (path, k) if path else str(k))
        return out
    if isinstance(old, list) and isinstance(new, list) and len(old) == len(new):
        out = []
        for i, (a, b) in enumerate(zip(old, new)):
            out += diff(a, b, "%s[%d]" % (path, i))
        return out
    return [] if old == new else [(path, old, new)]


def compare_units(saved, fresh):
    """Differences between saved and freshly built unit tables, ignoring provenance fields."""
    return diff(saved, fresh)


def write_db(db, path=DB_PATH):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        json.dump(db, handle, indent=1, ensure_ascii=False, sort_keys=False)
        handle.write("\n")


def load_db(path=DB_PATH):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=("build", "check", "show"))
    ap.add_argument("units", nargs="*")
    ap.add_argument("--bar-root", default=BAR)
    ap.add_argument("--out", default=DB_PATH)
    a = ap.parse_args(argv)
    if sys.stdout.encoding and sys.stdout.encoding.lower() != "utf-8":
        sys.stdout.reconfigure(encoding="utf-8")

    if a.command == "show":
        db = load_db(a.out)
        for u in a.units:
            print(json.dumps(db["units"].get(u.lower()), indent=1, ensure_ascii=False))
        return 0
    fresh = build_db(a.bar_root, set(u.lower() for u in a.units) or None)
    if a.command == "build":
        write_db(fresh, a.out)
        unresolved = sum(1 for u in fresh["units"].values() for w in u["weapons"]
                         if w.get("turret_turn", {}).get("method") == "unresolved")
        turrets = sum(1 for u in fresh["units"].values() for w in u["weapons"] if w["turret"])
        print("wrote %d units to %s (BAR %s); %d errors; turret speeds read for %d of %d turrets" % (
            fresh["unit_count"], a.out, (fresh["source"]["commit"] or "?")[:10], len(fresh["errors"]),
            turrets - unresolved, turrets))
        for k, v in fresh["errors"].items():
            print("  error", k, v)
        return 0
    saved = load_db(a.out)
    saved_units = saved["units"] if not a.units else {k: saved["units"].get(k) for k in fresh["units"]}
    changes = compare_units(saved_units, fresh["units"])
    print("saved DB from BAR %s; checkout is BAR %s" % ((saved["source"].get("commit") or "?")[:10],
                                                       (fresh["source"]["commit"] or "?")[:10]))
    for p, old, new in changes[:200]:
        print("  %s: %r -> %r" % (p, old, new))
    print("%d differences" % len(changes))
    return 1 if changes else 0


if __name__ == "__main__":
    sys.exit(main())
