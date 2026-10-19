import unittest
from pathlib import Path
import tempfile
from unittest.mock import patch
import json
import training_compare as compare

class CompareTests(unittest.TestCase):
    def test_constant_paired_gain_and_ties(self):
        result=compare.summarize_pairs([2.0]*4,[1.0]*4,17)
        self.assertEqual(result["interval_95"],[1.0,1.0])
        self.assertEqual(result["wins"],4)
        result=compare.summarize_pairs([1.0]*4,[1.0]*4,17)
        self.assertEqual(result["ties"],4)
        self.assertEqual(result["mean_gain"],0.0)

    def test_rescoring_requires_production_value_and_preserves_other_terms(self):
        trial={"fitness":3.0,"reward_components":{"production":1.0,"total":3.0},"reward_stats":{"units_produced":10}}
        self.assertEqual(compare.rescore(trial,{"production_mode":"none"})["fitness"],2.0)
        with self.assertRaises(ValueError):compare.rescore(trial,{"production_mode":"value"})
        trial["reward_stats"]["production_value"]=300.0
        self.assertEqual(compare.rescore(trial,{"production_mode":"value"})["fitness"],2.06)

    def test_pairs_reject_nonfinite_and_mismatched_samples(self):
        with self.assertRaises(ValueError):compare.summarize_pairs([1.0],[],7)
        with self.assertRaises(ValueError):compare.summarize_pairs([float("nan")],[1.0],7)

    def test_pooled_robust_score_is_distinct_from_mean_layout_score(self):
        def report(a,b):
            return {"stages":[{"name":"easy","weight":1,"fitness":-0.3,"details":{"rows":[{"scenario":"a","opponent":"hold","tally":{"fitness_sum":a,"battles":1}},{"scenario":"b","opponent":"hold","tally":{"fitness_sum":b,"battles":1}}]}}]}
        # Each layout has mean 0 and worst row -1: robust score -0.3.
        # Pooling the two layouts produces both row means 0: pooled score 0.
        self.assertAlmostEqual(compare.pooled_fitness([report(-1,1),report(1,-1)],{"easy":0.3}),0.0)

    def test_frozen_models_are_independent_of_live_champion_changes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);source=root/"live.json";source.write_text('{"kind":"x","w":1}')
            frozen=compare.freeze_file(source,root/"assessment")
            source.write_text('{"kind":"x","w":2}')
            self.assertEqual(compare.read(frozen)["w"],1)
            frozen.write_text("changed")
            source.write_text('{"kind":"x","w":1}')
            with self.assertRaises(ValueError):compare.freeze_file(source,root/"assessment")

    def test_branch_faction_guard_rejects_partial_metadata_and_regression(self):
        import branch_selection
        def report(a,b):return {"stages":[{"name":"easy","details":{"facets_complete":True,"facets":{"armada":a,"cortex":b}}}]}
        self.assertFalse(branch_selection.faction_protected([report(-.1,1)],[report(0,0)],.05))
        self.assertTrue(branch_selection.faction_protected([report(0,1)],[report(0,0)],.05))
        partial=report(0,1);partial["stages"][0]["details"]["facets_complete"]=False
        with self.assertRaises(ValueError):branch_selection.faction_protected([partial],[report(0,0)],.05)

    def test_changed_frozen_executable_cannot_leave_a_reusable_cache(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);brain=root/"brain.json";brain.write_text('{"kind":"test"}')
            stage=root/"stage.json";stage.write_text('{}')
            campaign=root/"campaign.json"
            compare.write(campaign,{"tracks":[{"name":"t","brain":"brain.json","validation":{"stages":[{"name":"easy","task":{"config":"stage.json"}}]}}]})
            sim=root/"sim.exe";sim.write_bytes(b"original")
            manifest={"simulator":str(sim),"campaign":str(campaign),"track":"t","reference":"a","brains":[{"label":"a","path":str(brain)}],"layouts":1}
            def corrupt(frozen,args,threads):
                Path(frozen).write_bytes(b"changed");return {"fitness":1}
            with patch.object(compare,"native",side_effect=corrupt):
                with self.assertRaisesRegex(ValueError,"frozen executable changed"):compare.compare(manifest,root/"result")
            self.assertEqual(list((root/"result/cache").glob("*.json")),[])
            self.assertEqual(compare.read(root/"result/progress.json")["status"],"failed")

    def test_native_float_identity_reuses_equivalent_json_weight_spellings(self):
        with tempfile.TemporaryDirectory() as tmp:
            a=Path(tmp)/"a.json";b=Path(tmp)/"b.json"
            def brain(value):return {"kind":"test","modules":[{"layers":[{"weights":[[value]],"biases":[0]}]}]}
            compare.write(a,brain(.1));compare.write(b,brain(.10000000149011612))
            self.assertEqual(compare.model_identity(a),compare.model_identity(b))
            compare.write(b,brain(.2));self.assertNotEqual(compare.model_identity(a),compare.model_identity(b))

    def test_atomic_write_retries_a_transient_windows_reader_lock(self):
        import os
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/"progress.json";compare.write(path,{"status":"old"})
            original=os.replace;attempts=[]
            def reader_lock(source,target):
                attempts.append(1)
                if len(attempts)<3:
                    self.assertEqual(compare.read(path),{"status":"old"})
                    raise PermissionError("reader temporarily denies file replacement")
                return original(source,target)
            with patch.object(compare.os,"replace",side_effect=reader_lock):compare.write(path,{"status":"new"})
            self.assertEqual(compare.read(path),{"status":"new"});self.assertEqual(len(attempts),3)
            self.assertFalse(path.with_name(path.name+".tmp").exists())

    def test_atomic_write_preserves_old_document_on_persistent_permission_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/"progress.json";compare.write(path,{"status":"old"})
            with patch.object(compare.os,"replace",side_effect=PermissionError("locked")) as replace,patch.object(compare.time,"sleep"):
                with self.assertRaises(PermissionError):compare.write(path,{"status":"new"})
            self.assertEqual(replace.call_count,51);self.assertEqual(compare.read(path),{"status":"old"})

if __name__=="__main__":unittest.main()
