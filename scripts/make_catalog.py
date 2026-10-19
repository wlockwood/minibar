"""Derive simulator catalog entries from Beyond All Reason unit definitions.

This applies the same conversions as visigoth/bridge/bar_ai_bridge.lua to the
data-only unit files, so training does not need a running game:

  acceleration = maxacc * 900        braking = maxdec * 900
  turn_rate    = turnrate * 0.16     radius  = max(collision scale x, z) / 2
  area_of_effect = areaofeffect / 2  (the def file gives a diameter)
  angles (sprayangle, accuracy, firetolerance) = value * 2*pi / 65536
  gravity = mygravity * 900 when set, else the map gravity for gravityaffected
  turret speeds: weapon customparams turretspeedy / turretspeedx or
  turretspeeds "y x", else the unit script's AimWeapon1 turns, else instant aim.

Assumptions that are NOT verified against a game export: map gravity 130, missile
speed is the time-averaged flight speed over the weapon's range, and unit
scripts that aim through a shared CATT include get an instant yaw.

    python scripts/make_catalog.py --out examples/t1-catalog.json armpw corthud ...
"""

import argparse
import glob
import json
import math
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bar_lua import parse_unit_file  # noqa: E402

BAR = os.environ.get("BAR_ROOT", r"D:\code\BAR\Beyond-All-Reason")
UNIT_FOLDERS = ("ArmBots", "CorBots", "ArmVehicles", "CorVehicles", "ArmHovercraft", "CorHovercraft")
MAP_GRAVITY = 130.0
ANGLE = 2.0 * math.pi / 65536.0
HITSCAN = {"BeamLaser", "LightningCannon", "Melee", "Rifle"}


def sensor_fields(ud):
    """Recoil UnitDef sensor defaults; useAirLos itself is runtime aircraft state.

    Keep optional radar emit height and moving cloak cost unset so the runtime
    can resolve their source-defined fallbacks. Ground-source exports derive
    leavesGhost from immobility plus a non-empty yardmap, not immobility alone.
    """
    sight = float(ud.get("sightdistance", 0))
    cloak_cost = float(ud.get("cloakcost", 0))
    return {
        "sight_distance": sight,
        "radar_radius": float(ud.get("radardistance", 0)),
        "sonar_radius": float(ud.get("sonardistance", 0)),
        "jammer_radius": float(ud.get("radardistancejam", 0)),
        "sonar_jammer_radius": float(ud.get("sonardistancejam", 0)),
        "stealth": bool(ud.get("stealth", False)),
        "sonar_stealth": bool(ud.get("sonarstealth", False)),
        "sight_emit_height": float(ud.get("sightemitheight", ud.get("losemitheight", 20))),
        "radar_emit_height": float(ud["radaremitheight"]) if "radaremitheight" in ud else None,
        "air_sight_distance": float(ud.get("airsightdistance", 1.5 * sight)),
        "use_air_los": bool(ud.get("useairlos", False)),
        "leaves_ghost": bool(ud.get("leavesghost", not ud.get("canfly", False)
                                     and float(ud.get("speed", 0)) <= 0 and bool(ud.get("yardmap")))),
        "can_cloak": bool(ud.get("cancloak", cloak_cost != 0)),
        "start_cloaked": bool(ud.get("initcloaked", False)),
        "cloak_cost": cloak_cost,
        "cloak_cost_moving": float(ud["cloakcostmoving"]) if "cloakcostmoving" in ud else None,
        "min_cloak_distance": float(ud.get("mincloakdistance", 0)),
        "decloak_spherical": bool(ud.get("decloakspherical", True)),
        "decloak_on_fire": bool(ud.get("decloakonfire", True)),
        "targeting_facility": bool(ud.get("istargetingupgrade", False)),
    }


def find_unit(name):
    for folder in UNIT_FOLDERS:
        path = os.path.join(BAR, "units", folder, name + ".lua")
        if os.path.exists(path):
            return parse_unit_file(path)[name]
    raise SystemExit("unit %r not found under %s" % (name, BAR))


def script_text(name):
    path = os.path.join(BAR, "scripts", "Units", name + ".bos")
    if not os.path.exists(path):
        return ""
    with open(path, encoding="utf-8", errors="replace") as handle:
        return handle.read()


def aim_speeds(name, weapon_cp, mount_index):
    """(yaw, pitch) degrees per second, None for instant."""
    if "turretspeedy" in weapon_cp or "turretspeedx" in weapon_cp:
        return weapon_cp.get("turretspeedy"), weapon_cp.get("turretspeedx")
    if "turretspeeds" in weapon_cp:
        parts = str(weapon_cp["turretspeeds"]).split()
        return float(parts[0]), float(parts[1]) if len(parts) > 1 else float(parts[0])
    text = script_text(name)
    match = re.search(r"AimWeapon%d\s*\([^)]*\)(.*?)\n\}" % mount_index, text, re.S)
    body = match.group(1) if match else ""
    yaw = re.search(r"y-axis\s+heading\s+speed\s+<([\d.]+)>", body)
    pitch = re.search(r"x-axis\s+[^;]*?pitch\s+speed\s+<([\d.]+)>", body)
    pitch_def = re.search(r"#define\s+CATT%d_PITCH_SPEED\s+<([\d.]+)>" % mount_index, text)
    yaw_value = float(yaw.group(1)) if yaw else None
    pitch_value = float(pitch.group(1)) if pitch else (float(pitch_def.group(1)) if pitch_def else None)
    return yaw_value, pitch_value


def missile_speed(wd, weapon_range):
    top = float(wd.get("weaponvelocity", 0.0))
    start = float(wd.get("startvelocity", top))
    accel = float(wd.get("weaponacceleration", 0.0))
    if top <= 0 or accel <= 0 or start >= top:
        return top or start
    t_acc = (top - start) / accel
    d_acc = (start + top) / 2.0 * t_acc
    if weapon_range <= d_acc:
        # v(t) = start + accel t; distance = start t + accel t^2 / 2
        t = (-start + math.sqrt(start * start + 2.0 * accel * weapon_range)) / accel
        return weapon_range / t
    return weapon_range / (t_acc + (weapon_range - d_acc) / top)


def weapon_entry(name, ud, mount_index, mount, wd):
    cp = wd.get("customparams", {}) or {}
    weapon_type = wd.get("weapontype", "Cannon")
    hitscan = weapon_type in HITSCAN
    my_gravity = abs(float(wd.get("mygravity", 0.0)))
    if my_gravity:
        gravity = my_gravity * 900.0
    elif str(wd.get("gravityaffected", "")).lower() == "true":
        gravity = MAP_GRAVITY
    else:
        gravity = 0.0
    weapon_range = float(wd.get("range", 0.0))
    if weapon_type == "MissileLauncher":
        speed = missile_speed(wd, weapon_range)
    else:
        speed = float(wd.get("weaponvelocity", 0.0))
    yaw, pitch = aim_speeds(name, cp, mount_index)
    accuracy = float(wd.get("accuracy", 0.0)) * ANGLE
    damage = float((wd.get("damage") or {}).get("default", 0.0))
    entry = {
        "name": wd.get("name", name),
        "damage": damage,
        "range": weapon_range,
        "reload": max(0.001, float(wd.get("reloadtime", 1.0))),
        "burst": int(wd.get("salvosize", wd.get("burst", 1))),
        "burst_rate": float(wd.get("salvodelay", wd.get("burstrate", 0.0))),
        "projectiles": int(wd.get("projectiles", 1)),
        "projectile_speed": round(speed, 3),
        "projectile_gravity": gravity,
        "high_trajectory": float(wd.get("hightrajectory", 0)) == 1.0,
        "weapon_type": weapon_type,
        "hitscan": hitscan,
        "turret": bool(wd.get("turret", False)),
        "accuracy": round(accuracy, 5),
        "moving_accuracy": round(float(wd.get("movingaccuracy", wd.get("accuracy", 0.0))) * ANGLE, 5),
        "spray_angle": round(float(wd.get("sprayangle", 0.0)) * ANGLE, 5),
        "target_move_error": float(wd.get("targetmoveerror", 0.0)),
        "lead_limit": float(wd.get("leadlimit", -1.0)),
        "predict_boost": float(wd.get("predictboost", 0.0)),
        "area_of_effect": float(wd.get("areaofeffect", 0.0)) / 2.0,
        "edge_effectiveness": float(wd.get("edgeeffectiveness", 0.0)),
        "fire_tolerance": round(float(wd.get("firetolerance", 3640.0)) * ANGLE, 5),
        "damage_duration": float(wd.get("beamtime", 1.0)) if weapon_type == "BeamLaser" else 0.0,
        "manual_fire": bool(wd.get("manualfire", False)),
        "paralyzer": bool(wd.get("paralyzer", False)),
    }
    if yaw is not None:
        entry["turret_yaw_speed"] = yaw
    if pitch is not None:
        entry["turret_pitch_speed"] = pitch
    if wd.get("tracks") and not hitscan:
        entry["tracks"] = True
        entry["projectile_turn_rate"] = float(wd.get("turnrate", 0.0)) * 0.16
        entry["projectile_lifetime"] = float(wd.get("flighttime", 0.0))
        entry["required_capabilities"] = ["homing_projectiles"]
    return entry


def catalog_entry(name):
    ud = find_unit(name)
    weapondefs = {key.lower(): value for key, value in (ud.get("weapondefs") or {}).items()}
    weapons = []
    mounts = ud.get("weapons") or {}
    master_entry = {}
    for index, key in enumerate(sorted(mounts, key=lambda k: float(k)), start=1):
        mount = mounts[key]
        wd = weapondefs.get(str(mount.get("def", "")).lower())
        if wd is None:
            continue
        # Anti-air mounts cannot shoot at the ground, so a ground arena drops them.
        if str(mount.get("onlytargetcategory", "")).strip().upper() == "VTOL":
            continue
        entry = weapon_entry(name, ud, index, mount, wd)
        slave = mount.get("slaveto")
        if slave is not None and float(slave) in master_entry:
            # A slaved barrel fires with its master: one more projectile per shot.
            master_entry[float(slave)]["projectiles"] += 1
            continue
        master_entry[float(key)] = entry
        weapons.append(entry)
    scales = str(ud.get("collisionvolumescales", "")).split()
    if len(scales) >= 3:
        radius = max(float(scales[0]), float(scales[2])) / 2.0
    else:
        radius = float(ud.get("footprintx", 2)) * 4.0
    return {
        "health": float(ud["health"]),
        **sensor_fields(ud),
        "metal_cost": float(ud["metalcost"]),
        "energy_cost": float(ud.get("energycost", 0.0)),
        "is_mobile": True,
        "movement_class": ud.get("movementclass", "BOT2"),
        "sight_distance": float(ud.get("sightdistance", 0.0)),
        "radar_radius": float(ud.get("radardistance", 0.0)),
        "simulation": {
            "max_speed": float(ud["speed"]),
            "reverse_speed": float(ud.get("rspeed", 0.0)),
            "acceleration": round(float(ud["maxacc"]) * 900.0, 3),
            "braking": round(float(ud["maxdec"]) * 900.0, 3),
            "turn_rate": round(float(ud["turnrate"]) * 0.16, 3),
            "turn_in_place": bool(ud.get("turninplace", True)),
            "turn_in_place_angle_limit": float(ud.get("turninplaceanglelimit", 90.0)),
            "radius": radius,
            "weapons": weapons,
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("units", nargs="+")
    parser.add_argument("--out", required=True)
    parser.add_argument("--base", help="catalog whose entries are kept verbatim when a unit is already in it")
    args = parser.parse_args()
    catalog = {}
    keep = {}
    if args.base:
        keep = json.load(open(args.base, encoding="utf-8"))["unit_catalog"]
    for name in args.units:
        catalog[name] = keep[name] if name in keep else catalog_entry(name)
    document = {
        "type": "hello",
        "protocol": 1,
        "game_version": "derived from Beyond-All-Reason unit definitions by scripts/make_catalog.py; "
        "NOT a bridge export (map gravity assumed %g, missile speed time-averaged, CATT yaw instant)" % MAP_GRAVITY,
        "unit_catalog": catalog,
    }
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(document, handle, indent=1)
    print("wrote %d units to %s" % (len(catalog), args.out))


if __name__ == "__main__":
    main()
