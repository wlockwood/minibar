"""Effective BAR database facts must survive the source-to-simulator boundary."""
import unittest
from pathlib import Path
from unittest.mock import patch

import make_catalog
import export_training_catalog
import unit_db


class MechanicsCatalogTests(unittest.TestCase):
    def test_fixed_forward_tolerance_uses_source_tolerance_not_reaim_threshold(self):
        import math
        weapon = make_catalog.weapon_entry("fixture", {}, 1, {},
            {"tolerance": 4096, "firetolerance": 8192})
        self.assertAlmostEqual(weapon["fire_tolerance"], math.pi / 8, places=5)
        turret = make_catalog.weapon_entry("fixture", {}, 1, {},
            {"turret": True, "tolerance": 4096, "firetolerance": 8192})
        self.assertAlmostEqual(turret["fire_tolerance"], math.pi / 4, places=5)

    def test_mount_cone_export_preserves_local_direction_and_full_angle(self):
        entry = make_catalog.weapon_entry("fixture", {}, 1,
            {"maindir": "1 0 0", "maxangledif": 60}, {})
        self.assertEqual(entry["main_direction"], [1, 0, 0])
        self.assertEqual(entry["max_angle_difference"], 60)
        unrestricted = make_catalog.weapon_entry("fixture", {}, 1, {}, {})
        self.assertNotIn("max_angle_difference", unrestricted)

    def test_exact_heading_conversion_and_engine_acceleration_defaults(self):
        movement = make_catalog.movement_fields({"canmove": True, "speed": 100, "turnrate": 1024})
        self.assertAlmostEqual(movement["turn_rate_deg_s"], 168.75)
        self.assertEqual(movement["acceleration_elmos_s2"], 450)

    def test_manual_launch_and_interceptor_masks_survive_export(self):
        raw = {"health": 100, "metalcost": 100, "energycost": 0, "buildtime": 100,
               "weapondefs": {"missile": {"commandfire": True, "stockpile": True,
                 "stockpiletime": 10, "targetable": 4, "interceptor": 2,
                 "coverage": 500, "customparams": {"stockpilelimit": 7}}},
               "weapons": {1: {"def": "missile"}}}
        weapon = export_training_catalog.convert_unit("fixture", raw, set())["simulation"]["weapons"][0]
        for field, expected in (("manual_fire", True), ("stockpile", True),
                                ("stockpile_time", 10), ("stockpile_limit", 7),
                                ("targetable", 4), ("interceptor", 2), ("coverage_range", 500)):
            self.assertEqual(weapon[field], expected, field)

    def test_standardized_wreck_values_use_shared_database_rules(self):
        raw = {"health": 100, "metalcost": 55, "energycost": 0,
               "corpse": "dead", "featuredefs": {
                   "dead": {"damage": 3, "metal": 2, "featuredead": "heap"},
                   "heap": {"damage": 1, "metal": 1}}}
        fields = make_catalog.corpse_fields("fixture", raw)
        self.assertEqual(fields["corpse_health"], 100)
        self.assertEqual(fields["corpse_metal"], 33)
        self.assertEqual(fields["corpse_heap_definition"]["reclaim"]["metal"], 13)
        raw["customparams"] = {"iscommander": True}
        self.assertEqual(make_catalog.corpse_fields("fixture", raw)["corpse_metal"], 2)

    def test_source_units_match_database_movement_and_weapon_facts(self):
        if not Path(unit_db.DB_PATH).is_file() or not Path(make_catalog.BAR).is_dir():
            self.skipTest("requires the local BAR unit database and source checkout")
        database = unit_db.load_db()["units"]
        for name in ("armpw", "armfig", "armbrawl", "armpeep", "armsilo", "armamd", "armspy", "armjamt", "armguard", "legfort"):
            with self.subTest(unit=name):
                record = database[name]
                raw = unit_db.read_unit_def(Path(make_catalog.BAR) / record["source_file"], make_catalog.BAR)[name]
                entry = export_training_catalog.convert_unit(name, raw, set())
                movement = record["movement"]
                if movement:
                    for field, source in (("max_speed", "speed"), ("acceleration", "acceleration_elmos_s2"),
                                          ("braking", "brake_elmos_s2")):
                        self.assertEqual(entry["simulation"][field], movement[source])
                    if movement["move_type"] == "strafe_air":
                        self.assertEqual(entry["flight"]["turn_radius"], movement["turn_radius"])
                    else:
                        self.assertEqual(entry["simulation"]["turn_rate"], movement["turn_rate_deg_s"])
                for field, source in (("sight_distance", "los"), ("air_sight_distance", "air_los"),
                                      ("radar_radius", "radar"), ("jammer_radius", "radar_jam")):
                    self.assertEqual(entry[field], record["sensors"][source])
                self.assertEqual(entry["stealth"], record["stealth"])
                self.assertEqual(entry["can_cloak"], record["cloak"]["can_cloak"])
                for weapon, canonical in zip(entry["simulation"]["weapons"], record["weapons"]):
                    self.assertEqual(weapon["reload"], canonical["reload_s"])
                    self.assertEqual(weapon["manual_fire"], canonical["manual_fire"])
                    self.assertEqual(weapon["stockpile"], canonical["stockpile"])
                    self.assertEqual(weapon["area_of_effect"], canonical["aoe"]["radius"])
                    for field, source in (("turret_yaw_speed", "yaw_deg_s"),
                                          ("turret_pitch_speed", "pitch_deg_s")):
                        expected = (canonical.get("turret_turn") or {}).get(source)
                        if expected is not None:
                            self.assertEqual(weapon[field], expected)
                        else:
                            self.assertNotIn(field, weapon)
                    self.assertEqual(weapon["source_mount"], canonical["mount"])
                # A changed effective record cannot silently contaminate a fresh export.
                if name == "armpw":
                    import copy
                    stale = copy.deepcopy(database[name])
                    stale["movement"]["speed"] += 1
                    with patch.object(unit_db, "load_db", return_value={"units": {name: stale}}):
                        with self.assertRaisesRegex(ValueError, "stale unit database"):
                            make_catalog.canonical_record(name, raw)


if __name__ == "__main__":
    unittest.main()
