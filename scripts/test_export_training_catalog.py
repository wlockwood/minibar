"""Source-backed expanded curriculum catalog contracts."""
import unittest
import export_training_catalog as exporter


def raw(**fields):
    result = {"health": 100, "metalcost": 48, "energycost": 1260, "buildtime": 100,
              "speed": 60, "turnradius":64, "movementclass": "BOT2", "weapondefs": {}, "weapons": {}}
    result.update(fields)
    return result


class ExportTests(unittest.TestCase):
    def test_approach_presets_cover_both_sides_and_turn_directions(self):
        approaches = exporter.curriculum({})["approaches"]
        angles = {(a["learner_heading_offset_deg"], a["enemy_heading_offset_deg"]) for a in approaches}
        self.assertTrue({(0, 90), (0, -90), (90, 0), (-90, 0), (0, 180), (180, 0)} <= angles)
        self.assertTrue(any(a["learner_speed_fraction"] == 0 for a in approaches))
        self.assertTrue(any(a["learner_speed_fraction"] == 1 for a in approaches))
    def test_combat_curriculum_starts_within_ground_contact_range(self):
        cfg = exporter.curriculum({name:{} for name in exporter.DEFAULT_UNITS})
        self.assertLessEqual(cfg["separation"][1] + 2 * 30, 400)

    def test_aircraft_without_ground_turnrate_can_turn_using_declared_radius(self):
        import math
        entry = exporter.convert_unit("fighter", raw(canfly=True, speed=128, turnradius=64), set())
        self.assertAlmostEqual(entry["simulation"]["turn_rate"], math.degrees(2.0))
        self.assertEqual(entry["flight"]["turn_radius"], 64)
        with self.assertRaisesRegex(ValueError, "turnradius"):
            exporter.convert_unit("fighter", raw(canfly=True, turnradius=0), set())

    def test_aircraft_effective_defaults_do_not_export_zero_acceleration(self):
        source = raw(canfly=True, weapondefs={"gun": {"weapontype": "Cannon"}},
                     weapons={1: {"def": "gun"}})
        source.pop("turnradius")
        entry = exporter.convert_unit("fighter", source, set())
        self.assertEqual(entry["simulation"]["acceleration"], 58.5)
        self.assertEqual(entry["flight"]["turn_radius"], 500)

    def test_hover_aircraft_can_stop_and_uses_heading_turn_rate(self):
        entry = exporter.convert_unit("gunship", raw(canfly=True, hoverattack=True, turnrate=800), set())
        self.assertTrue(entry["flight"]["hover"])
        self.assertEqual(entry["flight"]["minimum_speed"], 0)
        self.assertIsNone(entry["flight"]["turn_radius"])
        self.assertAlmostEqual(entry["simulation"]["turn_rate"], 131.84)

    def test_flying_builders_use_hover_model_without_hoverattack(self):
        entry = exporter.convert_unit("airbuilder", raw(canfly=True, builder=True, turnrate=550), set())
        self.assertTrue(entry["flight"]["hover"])
        self.assertFalse(entry["flight"]["attack_run"])
        self.assertAlmostEqual(entry["simulation"]["turn_rate"], 90.64)

    def test_resource_value_and_air_categories(self):
        entry = exporter.convert_unit("fighter", raw(canfly=True, cruisealtitude=140, airstrafe=True), set())
        self.assertEqual(entry["metal_cost"] + entry["energy_cost"] / 60, 69)
        self.assertEqual(entry["flight"]["cruise_altitude"], 140)
        self.assertIn("VTOL", entry["target_categories"])
        self.assertNotIn("NOTAIR", entry["target_categories"])
        self.assertNotIn("SURFACE", entry["target_categories"])
        self.assertTrue(entry["use_air_los"])

    def test_unsupported_gadgets_fail_explicitly(self):
        with self.assertRaisesRegex(ValueError, "transport"):
            exporter.convert_unit("carrier", raw(transportcapacity=4), set())
        with self.assertRaisesRegex(ValueError, "cluster"):
            exporter.convert_unit("cluster", raw(weapondefs={"gun": {"customparams": {"cluster_def": "x"}}}), set())

    def test_flight_export_has_no_ground_occupancy(self):
        entry = exporter.convert_unit("bomber", raw(canfly=True, hoverattack=False), set())
        self.assertFalse(entry["simulation"]["blocks_movement"])
        self.assertIsNone(entry["movement_class"])
        self.assertTrue(entry["flight"]["attack_run"])


if __name__ == "__main__":
    unittest.main()
