import json
from pathlib import Path
import tempfile
import unittest

import refresh_sensor_catalog as refresh


class RefreshSensorCatalogTests(unittest.TestCase):
    def fixture(self, root):
        bar = root / "BAR"
        (bar / "units").mkdir(parents=True)
        (bar / "units/test.lua").write_text(
            'return { test = { stealth=true, sightdistance=450, cloakcost=20, yardmap="o" } }',
            encoding="utf-8")
        source = root / "original.json"
        source.write_text(json.dumps({"unit_catalog": {"test": {
            "health": 100, "simulation": {"radius": 9}, "stealth": False,
            "unrelated": "preserve"}}, "game_version": "test"}), encoding="utf-8")
        return bar, source

    def test_refresh_preserves_other_fields_and_original_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bar, source = self.fixture(root)
            original = source.read_bytes()
            destination = root / "fresh.json"
            report = refresh.refresh(source, destination, bar)
            result = json.loads(destination.read_text(encoding="utf-8"))
            unit = result["unit_catalog"]["test"]
            self.assertTrue(unit["stealth"] and unit["can_cloak"] and unit["leaves_ghost"])
            self.assertEqual(unit["sight_distance"], 450)
            self.assertEqual(unit["simulation"], {"radius": 9})
            self.assertEqual(unit["unrelated"], "preserve")
            self.assertEqual(source.read_bytes(), original)
            self.assertEqual(report["units"], 1)
            self.assertEqual(len(result["sensor_provenance"]["source_catalog_sha256"]), 64)

    def test_missing_source_fails_without_publishing_partial_catalog(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bar, source = self.fixture(root)
            source.write_text('{"unit_catalog":{"missing":{}}}', encoding="utf-8")
            destination = root / "fresh.json"
            with self.assertRaisesRegex(ValueError, "missing"):
                refresh.refresh(source, destination, bar)
            self.assertFalse(destination.exists())

    def test_cannot_overwrite_source_or_existing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bar, source = self.fixture(root)
            original = source.read_bytes()
            with self.assertRaises((ValueError, FileExistsError)):
                refresh.refresh(source, source, bar)
            self.assertEqual(source.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
