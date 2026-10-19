"""Portable campaign dependency and relocation contracts."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
import zipfile

import package_training as package


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value), encoding="utf-8")


class PackageTests(unittest.TestCase):
    def test_companion_brain_is_a_portable_dependency(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "source"; root.mkdir()
            config, sim = self.fixture(root)
            original = root / "data/air.json"
            write(original, {"kind":"tactical-air-v1", "modules":[]})
            stage = root / "easy/stage.json"
            document = json.loads(stage.read_text())
            document["companion_brain"] = "../data/air.json"
            write(stage, document)
            out = Path(temp) / "bundle"
            manifest = package.build_package(config, sim, out)
            dependency = next(d for d in manifest["dependencies"] if d["field"] == "companion_brain")
            self.assertEqual(json.loads((out / dependency["target"]).read_text()), json.loads(original.read_text()))

    def fixture(self, root):
        (root / ".git").mkdir()
        sim = root / "target" / "sim.exe"
        sim.parent.mkdir(); sim.write_bytes(b"portable-native-fixture")
        write(root / "data" / "catalog.json", {"unit_catalog": {}})
        write(root / "data" / "baseline.json", {"kind": "tactical_v4", "modules": []})
        (root / "data" / "map.vsgmap").write_bytes(b"VSGMAP-fixture")
        for folder in ("easy", "hard"):
            write(root / folder / "stage.json", {
                "init_from": "../data/baseline.json",
                "opponents": ["charge", "brain:../data/baseline.json"],
                "arena": {"map": "../data/map.vsgmap"}})
        config = root / "campaign.json"
        write(config, {"tracks": [{"brain": "data/baseline.json", "training": {
            "catalog": "data/catalog.json", "seeds": 1,
            "stages": [{"name": name, "difficulty": i, "weight": 1,
                        "task": {"kind": "tactical", "config": f"{name}/stage.json"}}
                       for i, name in enumerate(("easy", "hard"))]}}]})
        return config, sim

    def test_relocation_preserves_recursive_dependencies_and_same_basenames(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "source"; root.mkdir()
            config, sim = self.fixture(root)
            out = Path(temp) / "bundle"
            manifest = package.build_package(config, sim, out)
            moved = Path(temp) / "relocated"; shutil.move(str(out), moved)
            shutil.rmtree(root)
            top = json.loads((moved / manifest["config"]).read_text())
            track = top["tracks"][0]
            self.assertTrue((moved / track["brain"]).is_file())
            stages = track["training"]["stages"]
            self.assertNotEqual(stages[0]["task"]["config"], stages[1]["task"]["config"])
            for stage in stages:
                stage_path = moved / stage["task"]["config"]
                document = json.loads(stage_path.read_text())
                for value in (document["init_from"], document["opponents"][1][6:], document["arena"]["map"]):
                    self.assertFalse(Path(value).is_absolute())
                    self.assertTrue((stage_path.parent / value).is_file())
            for record in manifest["files"]:
                path = moved / record["path"]
                self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), record["sha256"])
            self.assertTrue((moved / manifest["binary"]).is_file())

    def test_refuses_existing_output_and_preserves_it(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "source"; root.mkdir(); config, sim = self.fixture(root)
            out = Path(temp) / "out"; out.mkdir(); (out / "keep").write_text("mine")
            with self.assertRaisesRegex(ValueError, "exists"):
                package.build_package(config, sim, out)
            self.assertEqual((out / "keep").read_text(), "mine")

    def test_missing_dependency_fails_before_output_creation(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "source"; root.mkdir(); config, sim = self.fixture(root)
            (root / "data" / "map.vsgmap").unlink()
            out = Path(temp) / "out"
            with self.assertRaisesRegex(ValueError, "missing"):
                package.build_package(config, sim, out)
            self.assertFalse(out.exists())

    def test_path_traversal_outside_source_root_is_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "source"; root.mkdir(); config, sim = self.fixture(root)
            write(Path(temp) / "outside.json", {})
            write(config, {"brain": "../outside.json"})
            with self.assertRaisesRegex(ValueError, "outside|escape"):
                package.build_package(config, sim, Path(temp) / "out")

    def test_absolute_dependency_outside_source_root_is_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "source"; root.mkdir(); config, sim = self.fixture(root)
            outside = Path(temp) / "outside.json"; write(outside, {})
            write(config, {"brain": str(outside)})
            with self.assertRaisesRegex(ValueError, "outside|escape"):
                package.build_package(config, sim, Path(temp) / "out")

    def test_source_inputs_remain_unchanged(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "source"; root.mkdir(); config, sim = self.fixture(root)
            before = {p: p.read_bytes() for p in root.rglob("*") if p.is_file()}
            package.build_package(config, sim, Path(temp) / "out")
            self.assertEqual(before, {p: p.read_bytes() for p in before})

    def test_dependency_manifest_keeps_original_hash_and_zip_is_complete(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "source"; root.mkdir(); config, sim = self.fixture(root)
            original_hash = hashlib.sha256(config.read_bytes()).hexdigest()
            archive = Path(temp) / "portable.zip"
            out = Path(temp) / "out"
            manifest = package.build_package(config, sim, out, zip_output=archive)
            record = next(r for r in manifest["files"] if r["path"] == "campaign.json")
            self.assertEqual(record["source_sha256"], original_hash)
            self.assertNotEqual(record["sha256"], original_hash)
            with zipfile.ZipFile(archive) as zipped:
                self.assertEqual(set(zipped.namelist()), {r["path"] for r in manifest["files"]} | {"manifest.json"})
                self.assertEqual(zipped.read("campaign.json"), (out / "campaign.json").read_bytes())

    def test_symlink_dependency_is_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "source"; root.mkdir(); config, sim = self.fixture(root)
            link = root / "linked.json"
            try:
                os.symlink(root / "data" / "baseline.json", link)
            except OSError:
                self.skipTest("creating symlinks is unavailable on this Windows account")
            write(config, {"brain": "linked.json"})
            with self.assertRaisesRegex(ValueError, "symlink|reparse"):
                package.build_package(config, sim, Path(temp) / "out")


if __name__ == "__main__":
    unittest.main()
