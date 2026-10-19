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
    def test_sensor_support_is_enabled_without_altering_combat_anchors(self):
        docs=self.docs()
        for path,document in docs.items():
            if path.startswith('stages/'):
                support=document['matchups']['sensor_support']
                self.assertEqual(support['radars'],['armrad','corrad'])
                self.assertEqual(support['air_scouts'],['armpeep','corfink'])
                self.assertEqual(support['jammers'],['armjamt','corjamt'])
                self.assertAlmostEqual(support['jammer_share'],1/3)

    def test_elite_retests_and_unit_derived_ranges_are_enabled(self):
        docs = self.docs()
        self.assertEqual(docs['campaign.json']['elite_retest_every'],5)
        self.assertEqual(docs['pilot.json']['elite_retest_every'],1)
        self.assertEqual(docs['campaign.json']['elite_size'],8)
        for path,document in docs.items():
            if path.startswith('stages/'):
                self.assertEqual(document['matchups']['starting_distance_mode'],'unit_ranges')
                self.assertEqual(document['max_time'],30 if '/pilot/' in path else 120)

    def test_reciprocal_swarm_and_frozen_opponents(self):
        docs = self.docs()
        for profile in ('production', 'holdout', 'pilot'):
            reverse = docs[f'stages/{profile}/ground-swarm_heavy.json']
            anchor = reverse['matchups']['anchors'][0]
            self.assertEqual(anchor['learner'], [{'unit': 'armpw', 'count': 40}])
            self.assertEqual(anchor['enemy'], [{'unit': 'corsumo', 'count': 1}])
            for domain, family in [('ground', 'heavy_swarm'), ('air', 'air_air')]:
                cfg = docs[f'stages/{profile}/{domain}-{family}.json']
                self.assertIn('surround', cfg['opponents'])
                self.assertIn(f'brain:../../brains/{domain}.frozen.json', cfg['opponents'])
                self.assertEqual(cfg['max_time'], 30 if profile == 'pilot' else 120)

    def test_equal_investment_is_primary_and_unequal_controls_are_explicit(self):
        docs = self.docs()
        for track in docs["campaign.json"]["tracks"]:
            for stage in track["training"]["stages"]:
                cfg = docs[stage["task"]["config"]]
                if stage["name"].startswith("resource_"):
                    self.assertLess(stage["weight"], 1.0)
                else:
                    for key in ("underdog_ratio", "even_ratio", "overdog_ratio"):
                        self.assertEqual(cfg["matchups"][key], [0.98,1.02])
            self.assertEqual(sum(s["name"].startswith("resource_") for s in track["training"]["stages"]),2)

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
                self.assertEqual(cfg["opponents"][:4], ["charge", "kite", "hold", "surround"])
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
                self.assertEqual(docs[stage["task"]["config"]]["max_time"], 30)
        module = docs["pilot-cohesion.json"]
        self.assertEqual(len(module["tracks"]), 1)
        self.assertEqual(module["tracks"][0]["train_modules"], ["cohesion"])
        self.assertFalse(any(i.get("structural_search") for i in module["tracks"][0]["islands"]))

    def test_build_is_self_contained_and_never_launches_training(self):
        with tempfile.TemporaryDirectory() as td:
            root=Path(td); candidate=root/"candidate.json"; reference=root/"reference.json"; binary=root/"native.exe"
            elite=root/'elite.json';elite.write_text(json.dumps({'kind':'tactical-air-v1','saved':'breakthrough'}))
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
                receipt=builder.prepare(out,binary,root,candidate=candidate,reference=reference,checkpoint=None,elite_brains=[elite,elite])
                self.assertEqual(run.call_count,1)
            self.assertEqual(before,{p:builder.digest(p) for p in before})
            self.assertEqual(json.loads((out/"brains/ground.initial.json").read_text())["anchor"],"candidate")
            self.assertEqual((out/'brains/ground.frozen.json').read_bytes(), (out/'brains/ground.initial.json').read_bytes())
            self.assertEqual((out/'brains/air.frozen.json').read_bytes(), (out/'brains/air.initial.json').read_bytes())
            campaign=json.loads((out/'campaign.json').read_text())
            self.assertEqual(campaign['tracks'][0]['elite_brains'],[])
            self.assertEqual(campaign['tracks'][1]['elite_brains'],['brains/elites/air-1.json'])
            self.assertEqual((out/'brains/elites/air-1.json').read_bytes(),elite.read_bytes())
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
