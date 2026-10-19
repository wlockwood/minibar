import json
from pathlib import Path
import tempfile
import unittest
import training_experiments as experiments

class ExperimentTests(unittest.TestCase):
    def test_snapshot_reads_one_payload_for_hash_and_weights(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);run=root/"run";run.mkdir()
            source={"format":"minibar-campaign-state","version":1,"backend_identity":"old","completed_generations":5,"tracks":[{"name":"x","base":{"w":0},"best":{"w":1},"best_report":{"fitness":1},"initial_report":{"fitness":0}}]}
            (run/"state.json").write_text(json.dumps(source))
            result=experiments.snapshot_source(run,root/"source")
            source["tracks"][0]["best"]={"w":2};(run/"state.json").write_text(json.dumps(source))
            self.assertEqual(result["tracks"][0]["best"],{"w":1})
            self.assertEqual(json.loads((root/"source/state.json").read_text())["tracks"][0]["base"],{"w":0})

    def test_arm_treatments_and_independent_seeds(self):
        manifest=experiments.default_manifest()
        self.assertEqual(len(manifest["arms"]),5)
        self.assertEqual(len(set(manifest["training_seeds"])),3)
        self.assertEqual(manifest["arms"][3]["sigma_scale"],0.5)
        self.assertEqual(manifest["arms"][4]["modules"]["strategic"],["production"])
        self.assertEqual(manifest["pilot_generations"],1)

    def test_training_run_variation_pairs_treatments_by_seed_and_retains_failures(self):
        runs=[{"arm":arm,"seed":seed,"status":"completed" if seed<3 or arm=="legacy-v1" else "failed"} for arm in ["legacy-v1","treatment"] for seed in [1,2,3]]
        brains=[{"label":r["arm"]+"/"+str(r["seed"]),"fitness":r["seed"]+(1 if r["arm"]=="treatment" else 0)} for r in runs if r["status"]=="completed"]
        summary=experiments.run_summary([{"track":"tactical","brains":brains}],runs)
        treatment=next(s for s in summary if s["arm"]=="treatment")
        self.assertEqual(treatment["missing_seeds"],[3]);self.assertEqual(treatment["versus_legacy"]["mean_gain"],1)
        self.assertEqual(treatment["completed_seeds"],2)

    def test_common_comparison_profile_is_part_of_input_seal(self):
        with tempfile.TemporaryDirectory() as tmp:
            out=Path(tmp).resolve();profile=out/"comparison-profile";profile.mkdir();experiments.write(profile/"campaign.json",{"w":1})
            (out/"source").mkdir();(out/"source/state.json").write_bytes(b"state");sim=out/"sim.exe";sim.write_bytes(b"sim")
            m={"simulator":str(sim),"simulator_sha256":experiments.comparison.digest(b"sim"),"source_snapshot":{"state_sha256":experiments.comparison.digest(b"state")}}
            experiments.write(out/"input-hashes.json",{str(profile):experiments.comparison.dependency_hashes(profile/"campaign.json")})
            experiments._check_inputs(out,m)
            experiments.write(profile/"campaign.json",{"w":2})
            with self.assertRaisesRegex(ValueError,"experiment inputs changed"):experiments._check_inputs(out,m)

if __name__=="__main__":unittest.main()
