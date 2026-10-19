"""Preparation contracts: preserve acquisition axes and real build economics."""
import json
import tempfile
import unittest
from pathlib import Path

import prepare_from_replays as prep


class PreparationTests(unittest.TestCase):
    def test_x_major_samples_become_row_major_without_rotation(self):
        self.assertEqual(prep.flatten_xmajor([[1, 2, 3], [4, 5, 6]]),
                         (2, 3, [1, 4, 2, 5, 3, 6]))

    def test_economics_are_source_derived_and_negative_upkeep_survives(self):
        raw = {"health": 100, "metalcost": 155, "energycost": 0,
               "buildtime": 2600, "energyupkeep": -20,
               "footprintx": 4, "footprintz": 4, "yardmap": "o" * 16,
               "buildoptions": {1: "armmex", 2: "unsupported_air"}}
        entry = prep.unit_entry("armsolar", raw, {"metal_cost": 1}, {"armmex"})
        self.assertEqual(entry["metal_cost"], 155)
        self.assertEqual(entry["build_time"], 2600)
        self.assertEqual(entry["energy_upkeep"], -20)
        self.assertEqual(entry["footprint"], [4, 4])
        self.assertEqual(entry["build_options"], ["armmex"])
        self.assertTrue(entry["is_building"])
        self.assertEqual(entry["simulation"]["weapons"], [])

    def test_missing_real_build_cost_or_time_is_an_error(self):
        with self.assertRaisesRegex(ValueError, "buildtime"):
            prep.unit_entry("broken", {"health": 100, "metalcost": 10, "energycost": 20}, {}, set())

    def test_coarse_terrain_does_not_turn_source_leveling_into_flat_only_placement(self):
        raw = {"health": 100, "metalcost": 150, "energycost": 0, "buildtime": 2800,
               "levelground": True, "maxslope": 15, "maxwaterdepth": 0}
        entry = prep.unit_entry("corsolar", raw, {}, set())
        self.assertFalse(entry["level_ground"])
        self.assertEqual(entry["max_slope"], 15)
        self.assertEqual(entry["max_water_depth"], 0)

    def test_mixed_weapons_keep_aa_mount_damage_categories_and_shot_cost(self):
        raw = {"health": 100, "metalcost": 10, "energycost": 200, "buildtime": 100,
               "weapondefs": {
                   "ground": {"damage": {"default": 30, "commanders": 5}, "energypershot": 7,
                              "weapontype": "Cannon", "reloadtime": 2, "range": 300},
                   "aa": {"damage": {"vtol": 90}, "weapontype": "MissileLauncher",
                          "reloadtime": 3, "range": 600, "metalpershot": 2}},
               "weapons": {1: {"def": "GROUND", "onlytargetcategory": "SURFACE"},
                           2: {"def": "AA", "onlytargetcategory": "VTOL"}}}
        weapons = prep.unit_entry("hybrid", raw, {}, set())["simulation"]["weapons"]
        self.assertEqual(len(weapons), 2)
        self.assertEqual(weapons[0]["energy_cost"], 7)
        self.assertEqual(weapons[0]["damage_by_armour"]["commanders"], 5)
        self.assertEqual(weapons[1]["only_target_categories"], ["VTOL"])
        self.assertEqual(weapons[1]["damage_by_armour"]["vtol"], 90)
        self.assertEqual(weapons[1]["metal_cost"], 2)

    def test_all_three_factions_have_required_roles_and_combat(self):
        self.assertEqual({x["name"] for x in prep.FACTIONS}, {"armada", "cortex", "legion"})
        for faction in prep.FACTIONS:
            self.assertTrue(all(faction[key] for key in ("builder", "mex", "power", "factory")))
            self.assertGreaterEqual(len(faction["combat"]), 2)

    def test_map_shape_keeps_center_sample_origins_and_masks(self):
        header = {"map": "Fixture", "map_size_x": 128, "map_size_z": 192,
                  "metal_grid": [[0, 4, 0], [2, 0, 0]]}
        surface = {"water_level": 0, "terrain": {
            "heights": [[10, 11, 12], [20, 21, 22]],
            "movement_classes": {"BOT2": [[1, 0, 1], [1, 1, 1]]}}}
        source = prep.replay_map_source(header, surface, "fixture")
        self.assertEqual(source["source_version"], 1)
        self.assertEqual(source["heights"]["origin_x"], 32)
        self.assertEqual(source["heights"]["origin_z"], 32)
        self.assertEqual(source["heights"]["values"], [10, 20, 11, 21, 12, 22])
        self.assertEqual(source["passability"][0]["movement_class"], "bot2")
        self.assertEqual(source["passability"][0]["passable"], [True, True, False, True, True, True])
        self.assertEqual(source["continuous_metal"]["density"]["values"], [0, 2, 4, 0, 0, 0])
        self.assertNotEqual(source.get("provenance", {}).get("format"), "visigoth.native_map")

    def test_flat_fixture_has_legal_starts_and_tactical_space(self):
        for size in (512, 1024, 2048):
            source = prep.flat_map_source(size)
            starts, center = prep.choose_positions(source)
            self.assertEqual(len(starts), 2)
            self.assertNotEqual(starts[0], starts[1])
            self.assertTrue(source["metal_nodes"])
            self.assertTrue(0 < center[0] < size and 0 < center[1] < size)
            self.assertGreater(prep.clear_radius(source, center), 150)
            with tempfile.TemporaryDirectory() as temp:
                target = Path(temp) / "fixture.json"
                prep.write_json(target, source)
                loaded = json.loads(target.read_text())
                self.assertEqual(len(loaded["heights"]["values"]),
                                 loaded["heights"]["cols"] * loaded["heights"]["rows"])

    @unittest.skipUnless(Path(r"D:\code\BAR\Beyond-All-Reason\units").exists(), "BAR source unavailable")
    def test_real_assets_have_complete_economy_and_filtered_legal_build_graph(self):
        with tempfile.TemporaryDirectory() as temp:
            manifest = prep.prepare(Path(temp), Path(r"D:\code\BAR\Beyond-All-Reason"),
                                    Path(r"D:\code\BAR\visigoth\data\replay-runs"))
            catalog = json.loads((Path(temp) / manifest["catalog"]).read_text())["unit_catalog"]
            for faction in manifest["factions"]:
                builder, factory = catalog[faction["builder"]], catalog[faction["factory"]]
                self.assertTrue({faction[k] for k in ("mex", "power", "factory")} <= set(builder["build_options"]))
                self.assertTrue(set(faction["combat"] + [faction["builder"]]) <= set(factory["build_options"]))
                self.assertEqual(catalog[faction["power"]]["energy_upkeep"], -20)
                for name in faction["combat"]:
                    self.assertEqual(catalog[name]["movement_class"], "bot2")
            for unit in catalog.values():
                self.assertGreater(unit["build_time"], 0)
                self.assertGreater(unit["health"], 0)
                self.assertGreater(unit["metal_cost"], 0)
                self.assertIn("simulation", unit)
                self.assertFalse(unit["level_ground"])
            self.assertTrue(any("leveling" in note.lower() for note in manifest["warnings"]))
            self.assertIn("raw_source_levelground", manifest["unit_sources"]["armsolar"])
            self.assertEqual(len([m for m in manifest["maps"] if not m["synthetic"]]), 2)
            self.assertTrue(all(not Path(m["source"]).is_absolute() for m in manifest["maps"]))


if __name__ == "__main__":
    unittest.main()
