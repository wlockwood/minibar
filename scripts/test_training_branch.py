"""Portable branch round trips use the real native executable when available."""
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import training_branch as branch
import branch_transfer as transfer
import branch_selection as selection

REPO = Path(__file__).resolve().parents[1]
SIM = Path(os.environ.get("MINIBAR_TEST_SIM", REPO / "target/release/visigoth-simulator.exe"))


class SelectionTests(unittest.TestCase):
    def test_subset_integration_keeps_parked_registry_champion(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); pkg = root / 'package'
            transfer.write_json(pkg / 'campaign.json', {'tracks': [{'name': 'tactical'}]})
            transfer.write_json(pkg / 'branch.json', {'backend_identity': 'native', 'branch_id': 'subset'})
            active = {'name': 'tactical', 'best': {'a': 1}, 'base': {'a': 0}}
            parked = {'name': 'strategic', 'best': {'s': 2}, 'base': {'s': 0}}
            source = {'backend_identity': 'native', 'tracks': [active, parked]}
            incoming = {'state_sha256': 'return', 'tracks': [active]}
            result = selection.integrate(pkg, incoming, source, root / 'best', root / 'evidence')
            registry = selection.load_registry(root / 'best')
            self.assertEqual({t['name']: t for t in registry['tracks']}['strategic'], parked)
            self.assertEqual(result['parked_tracks'], ['strategic'])

    def test_structural_export_settings_keep_a_weight_only_search(self):
        config = {"tracks": [{"islands": [{"name": "local", "es": {"seed": 1}, "follow_champion": True}]}]}
        branch.enable_structural_search(config, 4)
        islands = config["tracks"][0]["islands"]
        self.assertNotIn("structural_search", islands[0])
        self.assertEqual(islands[1]["structural_search"]["every"], 4)
        self.assertFalse(islands[1]["restart"])
        self.assertFalse(islands[1].get("follow_champion", False))
        with self.assertRaises(ValueError):
            branch.enable_structural_search(config, 0)

    def report(self, easy, hard):
        return selection.aggregate([
            {"name": "easy", "difficulty": 1, "weight": 1, "fitness": easy},
            {"name": "hard", "difficulty": 2, "weight": 1, "fitness": hard}])

    def test_import_fresh_mode_bypasses_fixed_aggregate_but_keeps_final_guards(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); pkg = root / 'pkg'
            transfer.write_json(pkg / 'campaign.json', {'fresh_screening': True, 'promotion_seeds': 4,
                'promotion_min_t': 2.0, 'min_gain': .001, 'max_stage_regression': .15,
                'tracks': [{'name': 'tactical', 'validation': {'seeds': 1, 'layouts': 2}}]})
            transfer.write_json(pkg / 'branch.json', {'backend_identity': 'native', 'branch_id': 'fresh'})
            source = {'backend_identity': 'native', 'tracks': [{'name': 'tactical', 'best': {'w': 1}, 'base': {'w': 0}}]}
            incoming = {'state_sha256': 'return', 'tracks': [{'name': 'tactical', 'best': {'w': 2}, 'best_report': {}}]}
            def measured(package, track, brain, seeds, seed_base, directory, label):
                if label.endswith('fixed'):
                    return self.report(.95, .95) if label.startswith('candidate') else self.report(1, 1)
                return self.report(1.02, 1.02) if label.startswith('candidate') else self.report(1, 1)
            with patch.object(selection, 'evaluate', side_effect=measured):
                result = selection.integrate(pkg, incoming, source, root / 'best', root / 'evidence')
            self.assertTrue(result['tracks'][0]['accepted'])
            self.assertGreaterEqual(result['tracks'][0]['paired']['t_statistic'], 2.0)
            # Positive mean and a majority of wins still fail if gains are
            # inconsistent under the explicitly requested evidence threshold.
            def noisy(package, track, brain, seeds, seed_base, directory, label):
                if label.endswith('fixed'): return measured(package, track, brain, seeds, seed_base, directory, label)
                gain = .2 if label.endswith('-3') else .001
                return self.report(1+gain, 1+gain) if label.startswith('candidate') else self.report(1, 1)
            with patch.object(selection, 'evaluate', side_effect=noisy):
                rejected = selection.integrate(pkg, incoming, source, root / 'other-best', root / 'other-evidence')
            self.assertEqual(rejected['tracks'][0]['paired']['wins'], 4)
            self.assertFalse(rejected['tracks'][0]['accepted'])


    def test_fresh_majority_mean_gain_and_stage_protection(self):
        base = self.report(2, 2)
        self.assertTrue(selection.assess([self.report(3, 3)] * 4, [base] * 4, .01, .1)["accepted"])
        # Three small wins cannot offset a catastrophic fourth loss.
        report = selection.assess([self.report(2.1, 2.1)] * 3 + [self.report(0, 0)], [base] * 4, .01, .1)
        self.assertEqual(report["wins"], 3)
        self.assertFalse(report["accepted"])
        self.assertFalse(selection.assess([self.report(2.1, 2.1)] * 2 + [base] * 2, [base] * 4, .01, .1)["accepted"])
        self.assertFalse(selection.assess([self.report(1.8, 4)] * 4, [base] * 4, .01, .1)["accepted"])

    def test_comparison_scores_must_be_finite_and_metadata_must_match(self):
        with self.assertRaises(ValueError):
            selection.aggregate([{"name": "bad", "difficulty": 1, "weight": 1, "fitness": float("nan")}])
        bad = self.report(2, 2); bad["stages"][0]["name"] = "other"
        with self.assertRaises(ValueError):
            selection.assess([bad], [self.report(2, 2)], 0, .1)

    def test_successful_local_comparison_publishes_candidate_and_keeps_initial_reference(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); pkg = root / "package"; best = root / "best"
            incumbent = transfer.read_json(REPO / "data/brains/from_replays/tactical/baseline.json")
            candidate = copy.deepcopy(incumbent); candidate["modules"][0]["layers"][0]["weights"][0][0] += .25
            transfer.write_json(pkg / "campaign.json", {"min_gain": .01, "max_stage_regression": .1, "promotion_seeds": 4,
                "tracks": [{"name": "tactical", "validation": {"seeds": 1}}]})
            transfer.write_json(pkg / "branch.json", {"backend_identity": "same-backend", "branch_id": "branch"})
            incoming = {"state_sha256": "checkpoint", "tracks": [{"name": "tactical", "best": candidate, "best_report": {}}]}
            source = {"backend_identity": "same-backend", "tracks": [{"name": "tactical", "best": incumbent, "base": incumbent}]}
            def measured(package, track, brain, seeds, seed_base, directory, label):
                return self.report(3, 3) if label.startswith("candidate") else self.report(2, 2)
            with patch.object(selection, "evaluate", side_effect=measured) as native_evaluations:
                result = selection.integrate(pkg, incoming, source, best, root / "evidence")
                self.assertTrue(result["tracks"][0]["accepted"])
                registry = selection.load_registry(best)
                self.assertEqual(registry["tracks"][0]["best"], candidate)
                self.assertEqual(registry["tracks"][0]["base"], incumbent)
                self.assertEqual(native_evaluations.call_count, 11)
                seeds = {call.args[6]: call.args[4] for call in native_evaluations.call_args_list}
                for index in range(4): self.assertEqual(seeds[f"candidate-fresh-{index}"], seeds[f"incumbent-fresh-{index}"])
                repeated = selection.integrate(pkg, incoming, source, best, root / "second-evidence")
                self.assertEqual(repeated["tracks"][0]["reason"], "identical_brain")
                self.assertEqual(native_evaluations.call_count, 11, "already selected brains must not reroll evidence")

    def test_explicit_reference_without_layouts_uses_native_default_one(self):
        with tempfile.TemporaryDirectory() as tmp:
            package=Path(tmp)/"package";transfer.write_json(package/"manifest.json",{"binary":"sim.exe"})
            track={"name":"t","reference_brain":"reference.json","validation":{"seeds":1}}
            with patch.object(selection.subprocess,"run",return_value=subprocess.CompletedProcess([],0,'{"fitness":0,"stages":[]}',"")) as run:
                selection.evaluate(package,track,{"w":1},1,100,Path(tmp)/"evidence","candidate")
                args=run.call_args.args[0];self.assertEqual(args[args.index("--eval-layouts")+1],"1")

    def test_explicit_reference_cannot_replace_the_warm_start_stage_guard(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);pkg=root/"package"
            transfer.write_json(pkg/"campaign.json",{"min_gain":.01,"max_stage_regression":.1,"max_faction_regression":.05,"tracks":[{"name":"t","reference_brain":"reference.json","validation":{"seeds":1}}]})
            transfer.write_json(pkg/"branch.json",{"backend_identity":"same","branch_id":"branch"})
            transfer.write_json(pkg/"reference.json",{"role":"reference"})
            source={"backend_identity":"same","tracks":[{"name":"t","best":{"role":"incumbent"},"base":{"role":"base"}}]}
            incoming={"state_sha256":"snapshot","tracks":[{"name":"t","best":{"role":"candidate"},"best_report":{}}]}
            def measured(package,track,brain,*args):
                # Candidate beats incumbent and reference but regresses one initial stage.
                report=self.report(*{"incumbent":(0,0),"candidate":(1,1),"reference":(0,0),"base":(3,0)}[brain["role"]])
                for stage in report["stages"]:stage["details"]={"facets_complete":True,"facets":{"armada":stage["fitness"]}}
                return report
            with patch.object(selection,"evaluate",side_effect=measured):
                result=selection.integrate(pkg,incoming,source,root/"best",root/"evidence")
            self.assertFalse(result["tracks"][0]["accepted"])


class ArchiveTests(unittest.TestCase):
    def test_archive_paths_and_case_aliases_are_rejected_before_extraction(self):
        for names in [["../outside"], ["C:/outside"], ["a\\outside"], ["CON"], ["file."] , ["A.json", "a.json"]]:
            with self.subTest(names=names), tempfile.TemporaryDirectory() as tmp:
                archive = Path(tmp) / "bad.zip"
                with zipfile.ZipFile(archive, "w") as zipped:
                    for name in names:
                        info = zipfile.ZipInfo("placeholder"); info.filename = name; info.orig_filename = name
                        zipped.writestr(info, b"bad")
                with self.assertRaises(ValueError): transfer.extract_checked(archive, Path(tmp) / "out")
                self.assertFalse((Path(tmp) / "out").exists())

    def test_zip_symlink_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            archive = Path(tmp) / "bad.zip"
            with zipfile.ZipFile(archive, "w") as zipped:
                link = zipfile.ZipInfo("linked"); link.create_system = 3; link.external_attr = 0o120777 << 16
                zipped.writestr(link, "../../outside")
            with self.assertRaises(ValueError): transfer.extract_checked(archive, Path(tmp) / "out")


@unittest.skipUnless(SIM.exists(), "build the release simulator for native branch round-trip tests")
class NativeBranchTests(unittest.TestCase):
    def test_subset_campaign_parks_other_champions_without_changing_source(self):
        state = transfer.read_json(self.origin / "state.json")
        parked = copy.deepcopy(state["tracks"][0]); parked["name"] = "paused"
        state["tracks"].append(parked)
        transfer.write_json(self.origin / "state.json", state)
        before = (self.origin / "state.json").read_bytes()
        out = self.root / "subset"
        branch.export_branch(self.config, SIM, out, from_run=self.origin, seed=881, source_root=self.source)
        self.assertEqual(transfer.read_json(out / "parked/paused.json"), parked["best"])
        self.assertEqual(transfer.read_json(out / "branch.json")["parked_tracks"], ["paused"])
        self.assertEqual((self.origin / "state.json").read_bytes(), before)
        transfer.verify_bundle(out)

    def test_structural_round_trip_preserves_history_and_source_champion(self):
        before = (self.origin / "state.json").read_bytes()
        out = self.root / "structural"
        branch.export_branch(self.config, SIM, out, from_run=self.origin, seed=891, source_root=self.source, structural_every=1)
        self.native(out / "campaign.json", out / "run", 2)
        state = transfer.read_json(out / "run/state.json")
        self.assertEqual(len(state["tracks"][0]["structural_history"]), 2)
        returned = self.root / "structural-return.zip"
        transfer.return_results(out, returned)
        imported = branch.import_branch(returned, out, self.root / "structural-import")
        self.assertEqual(transfer.read_json(Path(imported["path"]) / "run/state.json"), state)
        self.assertEqual((self.origin / "state.json").read_bytes(), before)

    def test_backend_change_requires_explicit_requalification(self):
        state = transfer.read_json(self.origin / "state.json")
        state["backend_identity"] = "previous-executable"
        transfer.write_json(self.origin / "state.json", state)
        out = self.root / "requalified"
        with self.assertRaisesRegex(ValueError, "different executable"):
            branch.export_branch(self.config, SIM, out, from_run=self.origin, source_root=self.source)
        branch.export_branch(self.config, SIM, out, from_run=self.origin, source_root=self.source, revalidate_backend=True)
        qualification = transfer.read_json(out / "qualification.json")
        self.assertEqual(qualification["source_backend_identity"], "previous-executable")
        self.assertEqual(len(qualification["tracks"]), 1)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="minibar branch test ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"; self.source.mkdir()
        shutil.copyfile(REPO / "data/brains/from_replays/catalog.json", self.source / "catalog.json")
        shutil.copyfile(REPO / "data/brains/from_replays/tactical/baseline.json", self.source / "brain.json")
        stages = []
        for i, name in enumerate(["easy", "mixed", "objectives"]):
            transfer.write_json(self.source / f"{name}.json", {"name": name, "brain_kind": "tactical_v4", "hidden": 8,
                "max_time": 1, "layout_jitter": 1, "opponents": ["charge"], "seeds_per_battle": 1, "validation_seeds": 1,
                "scenarios": [{"name": name, "learner": [{"unit": "armpw", "count": 1}], "enemy": [{"unit": "armpw", "count": 1}], "separation": 200}]})
            stages.append({"name": name, "difficulty": i + 1, "weight": 1,
                           "task": {"kind": "tactical", "config": f"{name}.json"}})
        plan = {"catalog": "catalog.json", "seeds": 1, "stages": stages}
        self.config = self.source / "campaign.json"
        transfer.write_json(self.config, {"format": "minibar-campaign", "version": 1, "name": "branch test", "generations": 2,
            "seed": 17, "promotion_seeds": 0, "min_gain": 100, "tracks": [{"name": "tactical", "brain": "brain.json",
            "train_modules": ["mission"], "training": plan, "validation": plan,
            "islands": [{"name": "local", "es": {"population": 2, "sigma": .04, "learning_rate": .01, "seed": 41}}]}]})
        self.origin = self.source / "run"
        self.native(self.config, self.origin, 1)

    def native(self, config, run, generations, resume=False):
        cmd = [str(SIM), "--train-campaign", str(config), "--train-out", str(run), "--generations", str(generations), "--threads", "1"]
        if resume: cmd.append("--resume")
        result = subprocess.run(cmd, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def export(self, name="bundle", seed=891):
        out = self.root / name
        branch.export_branch(self.config, SIM, out, from_run=self.origin, seed=seed, source_root=self.source)
        return out

    def test_warm_start_uses_champions_distinct_streams_and_preserves_source(self):
        before = (self.origin / "state.json").read_bytes()
        out = self.export()
        state = json.loads(before); cfg = transfer.read_json(out / "campaign.json")
        best = transfer.read_json(out / cfg["tracks"][0]["brain"])
        self.assertEqual(best, state["tracks"][0]["best"])
        reference_path = cfg["tracks"][0].get("reference_brain")
        self.assertIsNotNone(reference_path, "warm starts must retain the original regression reference")
        self.assertEqual(transfer.read_json(out / reference_path), state["tracks"][0]["base"])
        self.assertNotEqual(cfg["seed"], 17)
        self.assertNotEqual(cfg["tracks"][0]["islands"][0]["es"]["seed"], 41)
        self.assertEqual((self.origin / "state.json").read_bytes(), before)
        self.assertTrue((out / "Train.ps1").is_file())
        self.assertTrue((out / "Return-Results.ps1").is_file())
        self.assertTrue(out.with_suffix(".zip").is_file())
        other = self.export("other", 892)
        self.assertNotEqual(transfer.read_json(out / "branch.json")["branch_id"], transfer.read_json(other / "branch.json")["branch_id"])
        repeat = self.export("repeat", 891)
        self.assertEqual(out.with_suffix(".zip").read_bytes(), repeat.with_suffix(".zip").read_bytes(), "an explicit seed and same source must reproduce the export")

    def test_actual_return_import_duplicate_and_native_resume_equivalence(self):
        out = self.export(); moved = self.root / "remote machine"; shutil.copytree(out, moved)
        self.native(moved / "campaign.json", moved / "run", 1)
        returned = self.root / "returned.zip"; transfer.return_results(moved, returned)
        collection = self.root / "imported"
        before = (self.origin / "state.json").read_bytes()
        result = branch.import_branch(returned, out, collection)
        self.assertEqual(result["status"], "imported")
        local = Path(result["path"])
        self.assertEqual((local / "run/state.json").read_bytes(), (moved / "run/state.json").read_bytes())
        self.native(local / "campaign.json", local / "run", 2, True)
        self.native(moved / "campaign.json", moved / "run", 2, True)
        self.assertEqual(transfer.read_json(local / "run/state.json"), transfer.read_json(moved / "run/state.json"))
        advanced = (local / "run/state.json").read_bytes()
        self.assertEqual(branch.import_branch(returned, out, collection)["status"], "duplicate")
        self.assertEqual((local / "run/state.json").read_bytes(), advanced)
        self.assertEqual((self.origin / "state.json").read_bytes(), before)

    def test_corrupt_return_or_wrong_original_export_does_not_create_import(self):
        out = self.export(); self.native(out / "campaign.json", out / "run", 1)
        returned = self.root / "returned.zip"; transfer.return_results(out, returned)
        bad = self.root / "corrupt.zip"
        with zipfile.ZipFile(returned) as source, zipfile.ZipFile(bad, "w") as target:
            for item in source.infolist():
                target.writestr(item, b"{}" if item.filename == "run/state.json" else source.read(item.filename))
        with self.assertRaisesRegex(ValueError, "hash|checksum"):
            branch.import_branch(bad, out, self.root / "bad-import")
        self.assertFalse((self.root / "bad-import").exists())
        other = self.export("other", 892)
        with self.assertRaisesRegex(ValueError, "manifest|export|branch"):
            branch.import_branch(returned, other, self.root / "wrong-import")
        self.assertFalse((self.root / "wrong-import").exists())

    def test_unchanged_state_with_more_completed_work_is_a_new_snapshot(self):
        out = self.export()
        cmd = [str(SIM), "--train-campaign", str(out / "campaign.json"), "--train-out", str(out / "run"), "--external", "--threads", "1"]
        result = subprocess.run(cmd, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        before = (out / "run/state.json").read_bytes()
        first = self.root / "first.zip"; transfer.return_results(out, first)
        jobs = list((out / "run/work/jobs").glob("*.json")); self.assertEqual(len(jobs), 1)
        destination = out / "run/work/results" / jobs[0].name
        result = subprocess.run([str(SIM), "--campaign-worker", str(jobs[0]), "--job-out", str(destination)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((out / "run/state.json").read_bytes(), before)
        second = self.root / "second.zip"; transfer.return_results(out, second)
        a = branch.import_branch(first, out, self.root / "imports")
        b = branch.import_branch(second, out, self.root / "imports")
        self.assertEqual(b["status"], "imported", "new completed work must not be discarded as duplicate state")
        self.assertNotEqual(a["path"], b["path"])
        self.assertFalse((Path(a["path"]) / "run/work/results" / jobs[0].name).exists())
        self.assertEqual((Path(b["path"]) / "run/work/results" / jobs[0].name).read_bytes(), destination.read_bytes())
        self.assertEqual(branch.import_branch(second, out, self.root / "imports")["status"], "duplicate")
        remote = self.root / "moved after return"; out.rename(remote)
        self.native(Path(b["path"]) / "campaign.json", Path(b["path"]) / "run", 1, True)
        self.native(remote / "campaign.json", remote / "run", 1, True)
        self.assertEqual(transfer.read_json(Path(b["path"]) / "run/state.json"), transfer.read_json(remote / "run/state.json"))

    def test_native_and_python_locks_interoperate(self):
        with transfer.exclusive_lock(self.origin / ".campaign.lock"):
            result = subprocess.run([str(SIM), "--train-campaign", str(self.config), "--train-out", str(self.origin), "--resume", "--max-seconds", "0.000000001"], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("already in use", result.stderr)
            with self.assertRaisesRegex(ValueError, "active|lock|use"):
                branch.export_branch(self.config, SIM, self.root / "blocked", from_run=self.origin, source_root=self.source)

    def test_registry_snapshot_hash_matches_the_single_version_that_was_read(self):
        best = self.root / "registry"
        brain = transfer.read_json(self.origin / "state.json")["tracks"][0]["best"]
        transfer.write_json(best / "objects/brain.json", brain)
        sha = transfer.digest(transfer.encoded(brain))
        doc = {"format": "minibar.best_brains", "version": 1, "backend_identity": "backend", "tracks": [
            {"name": "tactical", "brain": "objects/brain.json", "initial_brain": "objects/brain.json", "brain_sha256": sha, "initial_brain_sha256": sha}]}
        current = best / "current.json"; transfer.write_json(current, doc)
        original = current.read_bytes(); changed = {**doc, "note": "a concurrent publication"}
        read_bytes = Path.read_bytes; read_text = Path.read_text; calls = []
        def capture(path, text=False, **kwargs):
            data = read_text(path, **kwargs) if text else read_bytes(path)
            if path == current:
                calls.append(path)
                if len(calls) == 1: transfer.write_json(current, changed)
            return data
        with patch.object(Path, "read_bytes", lambda p: capture(p)), patch.object(Path, "read_text", lambda p, **kw: capture(p, True, **kw)):
            source = selection.load_registry(best)
        self.assertEqual(source["state_sha256"], transfer.digest(original), "provenance must hash the same bytes that supplied the champions")
        self.assertEqual(len(calls), 1)

    def test_import_can_register_current_best_and_export_from_that_registry(self):
        out = self.export(); self.native(out / "campaign.json", out / "run", 1)
        returned = self.root / "returned.zip"; transfer.return_results(out, returned)
        best = self.root / "best"
        result = branch.import_branch(returned, out, self.root / "imported", against_run=self.origin, best_dir=best)
        self.assertEqual(result["selection"]["tracks"][0]["reason"], "identical_brain")
        registry = transfer.read_json(best / "current.json")
        self.assertEqual(registry["tracks"][0]["brain_sha256"], transfer.digest(transfer.encoded(transfer.read_json(self.origin / "state.json")["tracks"][0]["best"])))
        next_out = self.root / "next"
        branch.export_branch(self.config, SIM, next_out, best_dir=best, seed=985, source_root=self.source)
        self.assertEqual(transfer.read_json(next_out / "branch.json")["source"]["kind"], "best_registry")

    def test_import_does_not_trust_remote_fitness_when_selecting_champions(self):
        out = self.export(); self.native(out / "campaign.json", out / "run", 1)
        state = transfer.read_json(out / "run/state.json")
        state["tracks"][0]["best"]["modules"][0]["layers"][0]["weights"][0][0] += .25
        state["tracks"][0]["best_report"]["fitness"] = 1_000_000
        for stage in state["tracks"][0]["best_report"]["stages"]: stage["fitness"] = 1_000_000
        transfer.write_json(out / "run/state.json", state)
        returned = self.root / "winner.zip"; transfer.return_results(out, returned)
        # Forged champion claims now fail the checkpoint lineage check before
        # reaching the existing independent local fitness reassessment.
        with self.assertRaisesRegex(ValueError, "checkpoint.*promotion lineage"):
            branch.import_branch(returned, out, self.root / "imported", against_run=self.origin, best_dir=self.root / "best")
        self.assertFalse((self.root / "best/current.json").exists())

    def test_recomputed_hashes_do_not_bypass_native_checkpoint_validation(self):
        out = self.export(); self.native(out / "campaign.json", out / "run", 1)
        state = transfer.read_json(out / "run/state.json")
        state["tracks"][0]["islands"][0]["es"]["config"]["sigma"] = .08
        transfer.write_json(out / "run/state.json", state)
        returned = self.root / "invalid-state.zip"; transfer.return_results(out, returned)
        with self.assertRaisesRegex(ValueError, "native checkpoint validation"):
            branch.import_branch(returned, out, self.root / "bad-native-import")
        self.assertFalse((self.root / "bad-native-import").exists())

    def test_returned_cache_is_native_validated_before_import_commit(self):
        out = self.export()
        result = subprocess.run([str(SIM), "--train-campaign", str(out / "campaign.json"), "--train-out", str(out / "run"), "--external"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        job = next((out / "run/work/jobs").glob("*.json")); cached = out / "run/work/results" / job.name
        result = subprocess.run([str(SIM), "--campaign-worker", str(job), "--job-out", str(cached)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        document = transfer.read_json(cached); document["report"]["name"] = "wrong-stage"
        transfer.write_json(cached, document)
        returned = self.root / "invalid-cache.zip"; transfer.return_results(out, returned)
        with self.assertRaisesRegex(ValueError, "cached work"):
            branch.import_branch(returned, out, self.root / "bad-cache-import")
        self.assertFalse((self.root / "bad-cache-import").exists())

    @unittest.skipUnless(shutil.which("powershell"), "Windows PowerShell is needed for launchers")
    def test_windows_powershell_launch_and_return_need_no_python_on_remote(self):
        out = self.export()
        for script, args in [("Train.ps1", ["-Generations", "1", "-Threads", "1"]),
                             ("Return-Results.ps1", ["-Out", str(self.root / "powershell-return.zip")])]:
            result = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(out / script)] + args, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        result = branch.import_branch(self.root / "powershell-return.zip", out, self.root / "ps-import")
        self.assertEqual(result["status"], "imported")
        transfer.return_results(out, self.root / "python-return.zip")
        self.assertEqual(branch.import_branch(self.root / "python-return.zip", out, self.root / "ps-import")["status"], "duplicate")
        with transfer.exclusive_lock(out / "run/.campaign.lock"):
            result = subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(out / "Return-Results.ps1"), "-Out", str(self.root / "busy.zip")], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((self.root / "busy.zip").exists())


if __name__ == "__main__":
    unittest.main()
