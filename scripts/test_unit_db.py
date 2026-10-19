"""Tests for the BAR unit database (scripts/unit_db.py) and its MCP server (scripts/unit_db_mcp.py).

The drift tests read the BAR checkout at unit_db.BAR and skip when it is absent:
- the saved data/units/bar_units.json must equal a fresh build from the checkout, so a BAR update
  (or a hand edit of the JSON) fails here until `python scripts/unit_db.py build` is rerun;
- every unit's plain top-level fields are re-read from the raw .lua text with a regex, independently
  of the bar_lua parser that built the database, and compared with the saved values.
The golden values were checked by hand against the BAR source on 2026-10-19 (BAR 138018b6b2).

    python scripts/test_unit_db.py
"""
import io
import json
import math
import os
import re
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import unit_db  # noqa: E402
import unit_db_mcp  # noqa: E402

HAVE_BAR = os.path.isdir(os.path.join(unit_db.BAR, "units"))
DB = unit_db.load_db()
UNITS = DB["units"]


def weapon(code, key):
    return next(w for w in UNITS[code]["weapons"] if w["weapon_def"] == key)


class GoldenValuesTest(unittest.TestCase):
    """Hand-checked values, including the engine defaults the extractor has to apply."""

    def test_costs_sensors_and_static_units(self):
        u = UNITS["cordoom"]
        self.assertEqual((u["name"], u["faction"], u["tech_level"]), ("Bulwark", "Cortex", 2))
        self.assertEqual(u["cost"], {"metal": 3000, "energy": 37000, "buildtime": 55200})
        self.assertEqual((u["health"], u["sensors"]["los"], u["sensors"]["radar"]), (9400, 780, 1200))
        self.assertEqual(u["sensors"]["air_los"], 1170)  # engine default 1.5 x los
        self.assertIsNone(u["movement"])
        self.assertEqual(UNITS["armanni"]["cost"], {"metal": 3500, "energy": 74000, "buildtime": 52000})
        self.assertEqual(UNITS["corsjam"]["sensors"]["radar_jam"], 900)

    def test_movement(self):
        m = UNITS["legsrail"]["movement"]
        self.assertEqual((m["speed"], m["acceleration"], m["turn_rate"]), (42, 0.08, 800))
        self.assertAlmostEqual(m["acceleration_elmos_s2"], 0.08 * 900)
        self.assertAlmostEqual(m["turn_rate_deg_s"], 800 * 360 / 65536 * 30, places=2)

    def test_aircraft_turning_by_move_type(self):
        fighter = UNITS["armfig"]["movement"]  # strafe air: turnradius = 64, no turnrate in the def
        self.assertEqual((fighter["move_type"], fighter["turn_radius"], fighter["turn_rate"]), ("strafe_air", 64, None))
        gunship = UNITS["corape"]["movement"]  # hoverattack: steers by turnrate = 600
        self.assertEqual((gunship["move_type"], gunship["turn_rate"], gunship["turn_radius"]), ("hover_air", 600, None))
        self.assertEqual(UNITS["armca"]["movement"]["move_type"], "hover_air")  # air builders hover too
        self.assertEqual(UNITS["armpw"]["movement"]["move_type"], "ground")

    def test_wreck_values_after_bar_standardisation(self):
        pawn = UNITS["armpw"]["wreck"]  # file says metal 29 / damage 192; BAR uses 0.6 x 54 and unit health
        self.assertEqual((pawn["metal"], pawn["health"]), (32, 370))
        self.assertEqual([(f["feature"], f["metal"], f["resurrectable"]) for f in pawn["chain"]],
                         [("dead", 32, True), ("heap", 13, False)])
        self.assertEqual((UNITS["cordoom"]["wreck"]["metal"], UNITS["cordoom"]["wreck"]["health"]), (1800, 9400))
        com = UNITS["armcom"]["wreck"]  # commanders keep their file values
        self.assertEqual((com["metal"], com["health"], com["chain"][1]["health"]), (1250, 10000, 5000))
        self.assertIsNone(UNITS["armfig"]["wreck"])  # no corpse

    def test_damage_carries_range_per_target(self):
        doom = UNITS["cordoom"]
        self.assertEqual(weapon("cordoom", "atadr")["vs"]["land"]["range"], 950)
        self.assertIsNone(weapon("cordoom", "atadr")["vs"]["air"]["range"])
        self.assertEqual(doom["combat"]["max_range"], {"land": 950, "air": 650, "sub": None})
        land = doom["combat"]["dps_by_range"]["land"]
        self.assertEqual([(b["range"], b["mounts"]) for b in land], [(950, [1]), (650, [2]), (370, [3])])
        self.assertEqual(land[0]["cumulative_dps"], 750)
        self.assertAlmostEqual(land[-1]["cumulative_dps"], doom["combat"]["dps"]["land"], places=1)

    def test_builder_abilities_follow_engine_defaults(self):
        rez = UNITS["armrectr"]["builder"]  # builder, canassist=false, canresurrect=true
        self.assertEqual((rez["buildpower"], rez["can_assist"], rez["can_repair"], rez["can_reclaim"],
                          rez["can_resurrect"], rez["can_build"]), (200, False, True, True, True, False))
        nano = UNITS["armnanotc"]["builder"]  # builder with no build options
        self.assertEqual((nano["buildpower"], nano["build_distance"], nano["can_build"], nano["can_assist"]),
                         (200, 400, False, True))
        spy = UNITS["armspy"]["builder"]  # canrepair=false, canreclaim left to the builder default
        self.assertEqual((spy["can_repair"], spy["can_reclaim"], spy["can_resurrect"]), (False, True, False))
        self.assertTrue(UNITS["armcom"]["builder"]["can_build"])
        self.assertFalse(UNITS["cordoom"]["builder"]["can_assist"])

    def test_stealth_and_cloak(self):
        spy = UNITS["armspy"]
        self.assertTrue(spy["stealth"])
        self.assertEqual(spy["cloak"], {"can_cloak": True, "cost_static": 15, "cost_moving": 40,
                                        "decloak_range": 75, "decloak_spherical": True})
        self.assertFalse(UNITS["cordoom"]["cloak"]["can_cloak"])

    def test_weapon_damage_reload_and_targets(self):
        thud = weapon("corthud", UNITS["corthud"]["weapons"][0]["weapon_def"])
        self.assertEqual((thud["range"], thud["vs"]["land"]["damage"], thud["vs"]["air"]["damage"]), (380, 104, 21))
        self.assertAlmostEqual(thud["reload_s"], 52 / 30, places=4)  # 1.73333 s rounded to whole frames
        self.assertFalse(thud["vs"]["sub"]["can_target"])
        atadr = weapon("cordoom", "atadr")
        self.assertEqual((atadr["vs"]["land"]["dps"], atadr["vs"]["air"]["can_target"]), (750, False))
        self.assertEqual(atadr["damage_by_armor"], {"commanders": 999, "default": 4500})
        ata = weapon("armanni", "ata")
        self.assertEqual((ata["range"], ata["vs"]["land"]["damage"]), (1400, 10800))

    def test_aoe_and_impact_only_beam_rewrite(self):
        atadr = weapon("cordoom", "atadr")["aoe"]
        self.assertEqual((atadr["radius"], atadr["edge_effectiveness"]), (6, 0.15))
        self.assertAlmostEqual(atadr["half_damage_radius"], 6 / (2 - 0.15), places=2)
        green = weapon("cordoom", "doomsday_green_laser")["aoe"]  # impactonly BeamLaser -> aoe 11, edge 1
        self.assertEqual((green["diameter"], green["edge_effectiveness"], green["impact_only"]), (11, 1, False))
        self.assertEqual(green["effective_radius"], 5.5)
        rail = UNITS["legsrail"]["weapons"][0]["aoe"]  # impactonly LaserCannon keeps no splash
        self.assertTrue(rail["impact_only"])
        self.assertEqual(rail["effective_radius"], 0)

    def test_turret_speeds_from_scripts(self):
        self.assertEqual(UNITS["corthud"]["weapons"][0]["turret_turn"]["yaw_deg_s"], 140)  # turn ... speed <140>
        self.assertEqual(UNITS["corthud"]["weapons"][0]["turret_turn"]["pitch_deg_s"], 50)
        pawn = UNITS["armpw"]["weapons"][0]["turret_turn"]  # Weapon1TurretY from customparams.turretspeedy
        self.assertEqual((pawn["yaw_deg_s"], pawn["method"]), (300, "turn-speed"))
        hlt = UNITS["corhlt"]["weapons"][0]["turret_turn"]  # CATT1_MAX_VELOCITY <9.0> per frame
        self.assertEqual((hlt["yaw_deg_s"], hlt["method"]), (270, "accelerated"))
        doom = UNITS["cordoom"]["weapons"][0]["turret_turn"]  # MAX_AIMY1_VELOCITY <3.00> per frame
        self.assertEqual(doom["yaw_deg_s"], 90)

    def test_size_radius(self):
        size = UNITS["cordoom"]["size"]  # 48 x 48 box collision volume, 4 x 4 footprint
        self.assertAlmostEqual(size["collision_radius"], math.sqrt(48 * 48 / math.pi), places=2)
        self.assertAlmostEqual(size["footprint_radius"], math.sqrt(64 * 64 / math.pi), places=2)
        self.assertEqual(size["radius_source"], "collision volume")


class FormulaTest(unittest.TestCase):
    def test_aoe_profile_limits(self):
        flat = unit_db.aoe_profile(100, 1.0, False)
        self.assertAlmostEqual(flat["effective_radius"], 50, places=1)
        linear = unit_db.aoe_profile(100, 0.0, False)  # f = 1 - d/R gives area fraction 1/3
        self.assertAlmostEqual(linear["effective_radius"], 50 / math.sqrt(3), delta=0.2)
        self.assertEqual(linear["half_damage_radius"], 25)
        self.assertEqual(unit_db.aoe_profile(100, 0.5, True)["effective_radius"], 0)

    def test_frames_rounding(self):
        self.assertAlmostEqual(unit_db.frames(0.53333), 16 / 30)
        self.assertAlmostEqual(unit_db.frames(0.001), 1 / 30)

    def test_compare_reports_changed_values(self):
        saved = {"cordoom": json.loads(json.dumps(UNITS["cordoom"]))}
        fresh = json.loads(json.dumps(saved))
        fresh["cordoom"]["cost"]["metal"] = 3100
        fresh["cordoom"]["weapons"][0]["range"] = 1000
        self.assertEqual(unit_db.compare_units(saved, fresh),
                         [("cordoom.cost.metal", 3000, 3100), ("cordoom.weapons[0].range", 950, 1000)])

    def test_target_categories(self):
        self.assertTrue(unit_db.can_target("VTOL", False, "air"))
        self.assertFalse(unit_db.can_target("VTOL", False, "land"))
        self.assertTrue(unit_db.can_target("NOTSUB", False, "air"))
        self.assertFalse(unit_db.can_target(None, False, "sub"))  # needs waterweapon
        self.assertTrue(unit_db.can_target(None, True, "sub"))


class CoverageTest(unittest.TestCase):
    def test_all_three_factions_and_names(self):
        factions = {u["faction"] for u in UNITS.values()}
        self.assertEqual(factions, {"Armada", "Cortex", "Legion"})
        self.assertGreater(len(UNITS), 600)
        named = sum(1 for u in UNITS.values() if u["name"])
        self.assertGreater(named / len(UNITS), 0.99)

    def test_records_are_complete(self):
        keys = {"codename", "name", "tech_level", "cost", "builder", "stealth", "cloak", "sensors", "movement",
                "size", "weapons", "combat", "summary"}
        for code, u in UNITS.items():
            self.assertLessEqual(keys, set(u), code)
            for w in u["weapons"]:
                self.assertEqual(set(w["vs"]), {"land", "air", "sub"}, code)
                self.assertEqual("turret_turn" in w, w["turret"], code)

    def test_turret_speeds_mostly_resolved(self):
        turrets = [w["turret_turn"] for u in UNITS.values() for w in u["weapons"] if w["turret"]]
        unresolved = [t for t in turrets if t["method"] == "unresolved"]
        self.assertLess(len(unresolved) / len(turrets), 0.08)


TOP_LEVEL = re.compile(r"^\t\t(\w+) = (-?[\d.]+|true|false),?\s*$", re.M)
DIRECT_FIELDS = {  # raw unit field -> path in the record
    "metalcost": "cost.metal", "energycost": "cost.energy", "buildtime": "cost.buildtime", "health": "health",
    "workertime": "builder.buildpower", "sightdistance": "sensors.los", "radardistance": "sensors.radar",
    "sonardistance": "sensors.sonar", "radardistancejam": "sensors.radar_jam",
    "sonardistancejam": "sensors.sonar_jam", "stealth": "stealth", "cloakcost": "cloak.cost_static",
    "mincloakdistance": "cloak.decloak_range",
}
MOBILE_FIELDS = {"speed": "movement.speed", "maxacc": "movement.acceleration", "turnrate": "movement.turn_rate"}


@unittest.skipUnless(HAVE_BAR, "BAR checkout unavailable")
class DriftTest(unittest.TestCase):
    def test_saved_db_matches_fresh_build_from_checkout(self):
        fresh = unit_db.build_db(unit_db.BAR)
        changes = unit_db.compare_units(UNITS, fresh["units"])
        detail = "\n".join("  %s: saved %r, checkout %r" % c for c in changes[:25])
        self.assertEqual(changes, [], "data/units/bar_units.json is stale (saved BAR %s, checkout %s); rerun "
                                      "`python scripts/unit_db.py build` and review:\n%s" % (
                             DB["source"]["commit"], fresh["source"]["commit"], detail))
        self.assertEqual(set(fresh["errors"]), set(DB["errors"]))

    def test_direct_fields_match_raw_lua_text(self):
        sources = unit_db.unit_sources(unit_db.BAR)
        mismatches = []
        for code, u in UNITS.items():
            with open(sources[code], encoding="utf-8") as handle:
                text = handle.read()
            text = text.split("\t\tcustomparams = {")[0]  # top-level unit fields come before nested tables
            # Last assignment wins, as in Lua 5.1 table constructors (legionnaire and legphoenix set maxacc twice).
            raw = {k.lower(): v for k, v in TOP_LEVEL.findall(text)}
            fields = dict(DIRECT_FIELDS, **(MOBILE_FIELDS if u["movement"] else {}))
            if u["movement"] and u["movement"]["move_type"] == "strafe_air":
                del fields["turnrate"]  # the engine ignores turnrate for fixed-wing aircraft
                fields["turnradius"] = "movement.turn_radius"
            for key, path in fields.items():
                if key not in raw:
                    continue
                expected = raw[key] == "true" if raw[key] in ("true", "false") else float(raw[key])
                actual = unit_db_mcp.get_path(u, path)
                ok = actual == expected if isinstance(expected, bool) else abs(float(actual) - expected) < 1e-3
                if not ok:
                    mismatches.append("%s.%s: lua %r, db %r" % (code, key, raw[key], actual))
        self.assertEqual(mismatches, [])

    def test_names_match_bar_language_file(self):
        names, _ = unit_db.load_language(unit_db.BAR)
        for code in ("cordoom", "armanni", "legsrail", "corthud", "armpw"):
            self.assertEqual(UNITS[code]["name"], names[code])

    def test_weapon_ranges_match_unit_files(self):
        sources = unit_db.unit_sources(unit_db.BAR)
        for code in ("cordoom", "armanni", "legsrail", "corthud", "armpw", "armamb", "legbart"):
            if code not in UNITS:
                continue
            raw = unit_db.read_unit_def(sources[code], unit_db.BAR)[code]
            defs = {k.lower(): v for k, v in raw.get("weapondefs", {}).items()}
            for w in UNITS[code]["weapons"]:
                wd = defs.get(w["weapon_def"])
                if wd is not None:
                    self.assertEqual(w["range"], wd.get("range", 10), "%s %s" % (code, w["weapon_def"]))


def call(store, name, arguments):
    reply = unit_db_mcp.handle(store, {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                                       "params": {"name": name, "arguments": arguments}})
    return reply["result"]


class McpToolTest(unittest.TestCase):
    store = unit_db_mcp.UnitStore()

    def test_initialize_and_list(self):
        init = unit_db_mcp.handle(self.store, {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                                               "params": {"protocolVersion": "2025-03-26"}})["result"]
        self.assertEqual(init["protocolVersion"], "2025-03-26")
        self.assertIn("tools", init["capabilities"])
        tools = unit_db_mcp.handle(self.store, {"jsonrpc": "2.0", "id": 2, "method": "tools/list"})["result"]
        self.assertEqual({t["name"] for t in tools["tools"]},
                         {"get_units", "search_units", "resolve_unit_name", "validate_unit_data",
                          "check_db_freshness", "describe_fields"})
        self.assertIsNone(unit_db_mcp.handle(self.store, {"jsonrpc": "2.0", "method": "notifications/initialized"}))

    def test_get_units_by_name_and_codename_with_projection(self):
        data = call(self.store, "get_units", {"names": ["Bulwark", "ARMANNI", "Commander", "zzzz"],
                                              "fields": ["cost.metal", "weapons[0].range", "weapons.type"]})
        data = data["structuredContent"]
        self.assertEqual(data["units"]["cordoom"]["cost.metal"], 3000)
        self.assertEqual(data["units"]["armanni"]["weapons[0].range"], 1400)
        self.assertEqual(data["units"]["cordoom"]["weapons.type"], ["BeamLaser"] * 3)
        self.assertEqual(data["unresolved"]["Commander"]["error"], "ambiguous name")
        self.assertEqual(data["unresolved"]["zzzz"]["error"], "unknown unit")

    def test_validate_claims(self):
        data = call(self.store, "validate_unit_data", {"claims": [
            {"unit": "Pulsar", "field": "weapons[ata].range", "value": 1400},
            {"unit": "cordoom", "field": "cost.energy", "value": 30000},
            {"unit": "cordoom", "field": "cost.energy", "value": 36000, "rel_tolerance": 0.05},
            {"unit": "armspy", "field": "stealth", "value": True},
            {"unit": "cordoom", "field": "cost.gold", "value": 1},
            {"unit": "Commander", "field": "health", "value": 1},
        ]})["structuredContent"]
        self.assertEqual([r["status"] for r in data["results"]],
                         ["match", "mismatch", "match", "match", "unknown_field", "ambiguous_unit"])
        self.assertEqual(data["results"][1]["actual"], 37000)

    def test_search(self):
        data = call(self.store, "search_units", {
            "faction": "Armada", "filters": [{"field": "builder.can_resurrect", "value": True}],
            "fields": ["codename"], "sort_by": "cost.metal"})["structuredContent"]
        self.assertIn("armrectr", [r["codename"] for r in data["units"]])
        jam = call(self.store, "search_units", {"filters": [{"field": "sensors.radar_jam", "op": ">=", "value": 900}],
                                                "fields": ["codename"]})["structuredContent"]
        self.assertIn("corsjam", [r["codename"] for r in jam["units"]])

    def test_errors_are_reported_not_raised(self):
        result = call(self.store, "search_units", {"filters": [{"field": "health", "op": "~", "value": 1}]})
        self.assertTrue(result["isError"])
        reply = unit_db_mcp.handle(self.store, {"jsonrpc": "2.0", "id": 9, "method": "nope"})
        self.assertEqual(reply["error"]["code"], -32601)

    @unittest.skipUnless(HAVE_BAR, "BAR checkout unavailable")
    def test_freshness_for_listed_units(self):
        data = call(self.store, "check_db_freshness", {"units": ["Bulwark", "legsrail"]})["structuredContent"]
        self.assertTrue(data["up_to_date"], data["differences"])
        self.assertEqual(data["units_checked"], 2)

    def test_stdio_round_trip(self):
        requests = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
             "params": {"name": "resolve_unit_name", "arguments": {"name": "Thug"}}},
        ]
        proc = subprocess.run([sys.executable, os.path.join(HERE, "unit_db_mcp.py")],
                              input="".join(json.dumps(r) + "\n" for r in requests).encode("utf-8"),
                              capture_output=True, timeout=60, cwd=os.path.dirname(HERE))
        replies = [json.loads(line) for line in io.BytesIO(proc.stdout).read().decode("utf-8").splitlines()]
        self.assertEqual([r["id"] for r in replies], [1, 2])
        self.assertEqual(replies[1]["result"]["structuredContent"]["match"]["codename"], "corthud")


if __name__ == "__main__":
    unittest.main()
