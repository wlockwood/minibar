"""Portable successor preparation must preserve inputs and policy domains."""
import tempfile
import unittest
import json
import os
import shutil
import subprocess
import hashlib
from unittest.mock import patch
from pathlib import Path
import prepare_diverse_campaign as builder
import export_training_catalog as exporter


class DiverseCampaignTests(unittest.TestCase):
    @unittest.skipUnless(os.name == "nt" and shutil.which("powershell"), "Windows PowerShell compatibility")
    def test_launcher_verifies_hashes_in_windows_powershell(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td)
            script=root/"Train.ps1"; script.write_text(builder.TRAIN_PS1, encoding="utf-8")
            native=root/"native.cmd"; native.write_bytes(b"@exit /b 0\r\n")
            payload=root/"input.txt"; payload.write_bytes(b"portable hash contract")
            builder.write(root/"receipt.json", {"binary":"native.cmd", "files":[{
                "path":"input.txt", "sha256":hashlib.sha256(payload.read_bytes()).hexdigest()}]})
            result=subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script)], capture_output=True,text=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_seed_reserves_all_pilot_optimizer_stream_offsets(self):
        with self.assertRaisesRegex(ValueError, "seed"):
            builder.campaign_documents(exporter.curriculum({name:{} for name in exporter.DEFAULT_UNITS}), (1 << 64) - 1050)

    def docs(self):
        units = {name: {} for name in exporter.DEFAULT_UNITS}
        matchups = exporter.curriculum(units)
        return builder.campaign_documents(matchups, 2026101829)

    def test_domain_specific_families_and_frozen_companions(self):
        docs = self.docs()
        campaign = docs["campaign.json"]
        self.assertEqual([t["name"] for t in campaign["tracks"]], ["ground", "air"])
        for track in campaign["tracks"]:
            self.assertEqual(track["training"]["stages"], track["validation"]["stages"])
            self.assertEqual(track["validation"]["layouts"], 2)
            for stage in track["training"]["stages"]:
                cfg = docs[stage["task"]["config"]]
                self.assertEqual(cfg["opponents"], ["charge", "kite", "hold"])
                self.assertEqual(cfg["objective"]["win_stake_floor"], 0.0)
                self.assertEqual(cfg["companion_brain"], "../../brains/air.initial.json" if track["name"] == "ground" else "../../brains/ground.initial.json")
                for family in cfg["matchups"]["families"]:
                    self.assertEqual(family["learner_type"] in ("fighters", "bombers"), track["name"] == "air")
        self.assertEqual(campaign["archive_size"], 0)
        self.assertEqual(campaign["promotion_min_t"], 2)
        self.assertEqual(campaign["module_recombination_every"], 5)

    def test_pilot_is_small_and_never_contains_holdout(self):
        docs = self.docs()
        pilot = docs["pilot.json"]
        self.assertEqual(pilot["generations"], 3)
        self.assertEqual(pilot["validation_every"], 1)
        for track in pilot["tracks"]:
            self.assertNotIn("holdout", track)
            self.assertEqual(track["islands"][0]["es"]["population"], 4)
            for stage in track["training"]["stages"]:
                self.assertEqual(docs[stage["task"]["config"]]["max_time"], 6)
        module = docs["pilot-cohesion.json"]
        self.assertEqual(len(module["tracks"]), 1)
        self.assertEqual(module["tracks"][0]["train_modules"], ["cohesion"])
        self.assertFalse(any(i.get("structural_search") for i in module["tracks"][0]["islands"]))

    def test_build_is_self_contained_and_never_launches_training(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td); candidate=root/"candidate.json"; reference=root/"reference.json"; binary=root/"native.exe"
            candidate.write_text(json.dumps({"kind":"tactical_v4","anchor":"candidate"}))
            reference.write_text(json.dumps({"kind":"tactical_v4","anchor":"reference"}))
            binary.write_bytes(b"bounded test executable")
            before={p:builder.digest(p) for p in (candidate,reference,binary)}
            def export(out, bar, selected):
                out.mkdir(parents=True)
                units={name:{} for name in selected}
                builder.write(out/"catalog.json",{"unit_catalog":units})
                builder.write(out/"matchups.json",exporter.curriculum(units))
                builder.write(out/"manifest.json",{"rejected":{}})
                return {"rejected":{}}
            def native(command, **kwargs):
                self.assertIn("--seed-brain",command)
                self.assertNotIn("--train-campaign",command)
                builder.write(Path(command[command.index("--brain-out")+1]),{"kind":"tactical-air-v1"})
            out=root/"successor"
            with patch.object(builder.exporter,"export",side_effect=export), patch.object(builder.subprocess,"run",side_effect=native) as run:
                receipt=builder.prepare(out,binary,root,candidate=candidate,reference=reference,checkpoint=None)
                self.assertEqual(run.call_count,1)
            self.assertEqual(before,{p:builder.digest(p) for p in before})
            self.assertEqual(json.loads((out/"brains/ground.initial.json").read_text())["anchor"],"candidate")
            self.assertIn("not evaluated",receipt["holdout_status"])
            self.assertFalse((out/"runs").exists())
            for item in receipt["files"]:
                self.assertEqual(builder.digest(out/item["path"]),item["sha256"])

    def test_existing_output_is_refused_before_work(self):
        with tempfile.TemporaryDirectory() as td:
            marker = Path(td) / "keep.txt"
            marker.write_text("existing")
            with self.assertRaisesRegex(ValueError, "exists"):
                builder.prepare(Path(td), Path("missing.exe"), Path("missing-source"))
            self.assertEqual(marker.read_text(), "existing")


if __name__ == "__main__":
    unittest.main()
