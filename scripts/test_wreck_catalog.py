"""Source feature export contracts; synthetic fixtures avoid balance assumptions."""
import unittest
import struct
import tempfile
from pathlib import Path
from unittest.mock import patch

import make_catalog
import prepare_from_replays


class WreckCatalogTests(unittest.TestCase):
    def source(self):
        return {"corpse": "DEAD", "featuredefs": {
            "dead": {"damage": 90, "collisionvolumescales": "20 6 12",
                     "blocking": True, "metal": 25, "featuredead": "HEAP"},
            "heap": {"damage": 30, "collisionvolumescales": "12 2 10",
                     "blocking": False, "metal": 8}}}

    def test_exports_named_chain_real_health_geometry_and_reclaim(self):
        fields = make_catalog.corpse_fields("fixture", self.source())
        self.assertEqual(fields["corpse"], "fixture_dead")
        self.assertEqual(fields["corpse_health"], 90)
        self.assertEqual(fields["corpse_radius"], 10)
        self.assertEqual(fields["corpse_height"], 6)
        self.assertEqual(fields["corpse_reclaim_time"], 150)
        heap = fields["corpse_heap_definition"]
        self.assertEqual(heap["name"], "fixture_heap")
        self.assertEqual(heap["reclaim"]["metal"], 8)
        self.assertFalse(heap["blocks_movement"])
        self.assertFalse(heap["blocks_fire"])

    def test_missing_or_cyclic_links_fail(self):
        for link in ("missing", "DEAD"):
            raw = self.source()
            raw["featuredefs"]["dead"]["featuredead"] = link
            with self.assertRaises(ValueError):
                make_catalog.corpse_fields("fixture", raw)

    def test_collision_midpoint_uses_s3o_model_and_feature_offset(self):
        raw = self.source()
        raw["featuredefs"]["dead"].update(object="fixture.s3o", collisionvolumeoffsets="0 -2 0")
        header = bytearray(52)
        header[:12] = b"Spring unit\0"
        struct.pack_into("<f", header, 28, 11.0)
        with tempfile.TemporaryDirectory() as directory:
            models = Path(directory) / "objects3d"
            models.mkdir()
            (models / "fixture.s3o").write_bytes(header)
            with patch.object(make_catalog, "BAR", directory):
                fields = make_catalog.corpse_fields("fixture", raw)
        self.assertEqual(fields["corpse_collision_mid_y"], 9.0)

    def test_invalid_geometry_and_health_fail(self):
        for key, value in (("damage", float("nan")), ("collisionvolumescales", "20 -1 12")):
            raw = self.source()
            raw["featuredefs"]["dead"][key] = value
            with self.assertRaises(ValueError):
                make_catalog.corpse_fields("fixture", raw)

    def test_both_catalog_paths_export_features(self):
        raw = self.source()
        raw.update(health=100, metalcost=30, energycost=0, buildtime=100,
                   speed=10, maxacc=1, maxdec=1, turnrate=10)
        with patch.object(make_catalog, "find_unit", return_value=raw):
            entry = make_catalog.catalog_entry("fixture")
        prepared = prepare_from_replays.unit_entry("fixture", raw, {}, set())
        self.assertEqual(entry["corpse_health"], prepared["corpse_health"])
        self.assertEqual(entry["corpse_heap_definition"], prepared["corpse_heap_definition"])


if __name__ == "__main__":
    unittest.main()
