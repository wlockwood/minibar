"""BAR sensor-definition export contracts shared by both catalog generators."""
import unittest
from pathlib import Path
from unittest.mock import patch

from bar_lua import parse_unit_file
import make_catalog
import prepare_from_replays


class SensorExportTests(unittest.TestCase):
    def test_source_defaults_match_recoil_and_cloak_cost_enables_cloak(self):
        fields = make_catalog.sensor_fields({"sightdistance": 300, "cloakcost": 15})
        self.assertTrue(fields["can_cloak"])
        self.assertEqual(fields["cloak_cost"], 15)
        self.assertIsNone(fields["cloak_cost_moving"])
        self.assertEqual(fields["sight_emit_height"], 20)
        self.assertIsNone(fields["radar_emit_height"])
        self.assertEqual(fields["air_sight_distance"], 450)
        self.assertTrue(fields["decloak_spherical"])
        self.assertTrue(fields["decloak_on_fire"])
        self.assertFalse(fields["leaves_ghost"])
        self.assertFalse(fields["use_air_los"])

    def test_source_ghost_default_requires_yardmap_and_immobility(self):
        self.assertTrue(make_catalog.sensor_fields({"yardmap": "oooo"})["leaves_ghost"])
        self.assertFalse(make_catalog.sensor_fields({"yardmap": "oooo", "speed": 30})["leaves_ghost"])
        self.assertFalse(make_catalog.sensor_fields({"yardmap": "oooo", "canfly": True})["leaves_ghost"])

    def test_source_overrides_and_stealth_modalities_are_separate(self):
        fields = make_catalog.sensor_fields({
            "sightdistance": 200, "airsightdistance": 700,
            "losemitheight": 12, "sightemitheight": 30, "radaremitheight": 80,
            "stealth": True, "sonarstealth": False, "sonardistance": 900,
            "radardistancejam": 450, "sonardistancejam": 275,
            "cancloak": False, "cloakcost": 20, "cloakcostmoving": 40,
            "initcloaked": True, "mincloakdistance": 75, "decloakspherical": False,
            "decloakonfire": False, "leavesghost": False, "istargetingupgrade": True})
        self.assertFalse(fields["can_cloak"])
        self.assertTrue(fields["start_cloaked"])
        self.assertTrue(fields["stealth"])
        self.assertFalse(fields["sonar_stealth"])
        self.assertEqual(fields["sonar_radius"], 900)
        self.assertEqual(fields["sonar_jammer_radius"], 275)
        self.assertEqual(fields["jammer_radius"], 450)
        self.assertEqual(fields["sight_emit_height"], 30)
        self.assertEqual(fields["radar_emit_height"], 80)
        self.assertEqual(fields["air_sight_distance"], 700)
        self.assertEqual(fields["cloak_cost_moving"], 40)
        self.assertEqual(fields["min_cloak_distance"], 75)
        self.assertFalse(fields["decloak_spherical"])
        self.assertFalse(fields["decloak_on_fire"])
        self.assertFalse(fields["leaves_ghost"])
        self.assertTrue(fields["targeting_facility"])

    def test_both_generators_retain_the_sensor_contract(self):
        raw = {"health": 100, "metalcost": 10, "energycost": 100, "buildtime": 200,
               "speed": 30, "maxacc": .2, "maxdec": .3, "turnrate": 100,
               "sightdistance": 300, "stealth": True, "cloakcost": 15,
               "sonardistancejam": 200}
        with patch.object(make_catalog, "find_unit", return_value=raw):
            ground = make_catalog.catalog_entry("test")
        complete = prepare_from_replays.unit_entry("test", raw, {}, set())
        for key, value in make_catalog.sensor_fields(raw).items():
            self.assertEqual(ground[key], value, key)
            self.assertEqual(complete[key], value, key)

    @unittest.skipUnless(Path(r"D:\code\BAR\Beyond-All-Reason\units").exists(), "BAR reference unavailable")
    def test_real_spy_and_pinpointer_source_mechanics_are_not_dropped(self):
        root = Path(r"D:\code\BAR\Beyond-All-Reason\units")
        spy = make_catalog.sensor_fields(parse_unit_file(root / "ArmBots/T2/armspy.lua")["armspy"])
        self.assertTrue(spy["can_cloak"] and spy["stealth"] and spy["sonar_stealth"])
        self.assertEqual((spy["cloak_cost"], spy["cloak_cost_moving"], spy["min_cloak_distance"]), (15, 40, 75))
        pinpointer = make_catalog.sensor_fields(parse_unit_file(root / "ArmBuildings/LandUtil/armtarg.lua")["armtarg"])
        self.assertTrue(pinpointer["targeting_facility"] and pinpointer["leaves_ghost"])


if __name__ == "__main__":
    unittest.main()
