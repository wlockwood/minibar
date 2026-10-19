import json
import os
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import campaign_dashboard as cd  # noqa: E402


class CheckpointSharingTest(unittest.TestCase):
    def test_jsonl_keeps_unicode_separators_inside_string_values(self):
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "history.jsonl")
            row = {"label": "first\u2028second\u0085third\u2029fourth"}
            with open(path, "w", encoding="utf-8") as f:
                f.write(json.dumps(row, ensure_ascii=False) + "\n")
            self.assertEqual(cd.read_jsonl(path), [row])

    def test_json_reader_releases_file_before_decoding(self):
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "champion-δ.json")
            replacement = os.path.join(directory, "next.json")
            with open(path, "w", encoding="utf-8") as f:
                json.dump({"generation": 94}, f)
            with open(replacement, "w", encoding="utf-8") as f:
                json.dump({"generation": 95}, f)
            decode = json.loads
            def replace_during_decode(payload, *args, **kwargs):
                os.replace(replacement, path)
                return decode(payload, *args, **kwargs)
            with patch.object(cd.json, "loads", side_effect=replace_during_decode):
                self.assertEqual(cd.read_json(path), {"generation": 94})
            self.assertEqual(cd.read_json(path), {"generation": 95})

    def test_jsonl_reader_decodes_a_closed_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "history.jsonl")
            replacement = os.path.join(directory, "next.jsonl")
            with open(path, "w", encoding="utf-8") as f:
                f.write('{"generation":94}\n{"generation":95}\n{"generation":')
            with open(replacement, "w", encoding="utf-8") as f:
                f.write('{"generation":96}\n')
            decode = json.loads
            def replace_during_decode(payload, *args, **kwargs):
                if os.path.exists(replacement):
                    os.replace(replacement, path)
                return decode(payload, *args, **kwargs)
            with patch.object(cd.json, "loads", side_effect=replace_during_decode):
                self.assertEqual(cd.read_jsonl(path), [{"generation":94}, {"generation":95}])
            self.assertEqual(cd.read_jsonl(path), [{"generation":96}])



def demo_model(generations=40):
    directory = tempfile.mkdtemp()
    cd.make_demo(directory, generations=generations)
    return directory, cd.load_model(directory)


class WilsonTest(unittest.TestCase):
    def test_promotion_decisions_explain_skipped_and_failed_tests(self):
        record = {"generation": 5, "islands": [{"name": "local", "decisions": [
            {"candidate": "center", "reason": "screen_gain_too_small", "screen_gain": 0.0001},
            {"candidate": "sample", "reason": "fresh_majority_failed", "screen_gain": 0.02}]}]}
        summary = cd.summarize_generation(record, False)
        self.assertEqual(summary["islands"][0]["decisions"], record["islands"][0]["decisions"])
        events = cd.build_events({"generations": [summary]}, {})
        self.assertTrue(any("gain" in e["text"] for e in events))
        self.assertTrue(any("majority" in e["text"] for e in events))

    def test_interval_brackets_the_rate_and_shrinks_with_data(self):
        lo, hi = cd.wilson(5, 10)
        self.assertLess(lo, 0.5)
        self.assertGreater(hi, 0.5)
        lo2, hi2 = cd.wilson(500, 1000)
        self.assertLess(hi2 - lo2, hi - lo)

    def test_edges_stay_inside_zero_to_one(self):
        self.assertEqual(cd.wilson(0, 0), (0.0, 1.0))
        lo, hi = cd.wilson(10, 10)
        self.assertGreater(lo, 0.5)
        self.assertLessEqual(hi, 1.0)
        self.assertGreaterEqual(cd.wilson(0, 10)[0], 0.0)


class ExtractionTest(unittest.TestCase):
    def test_elite_pool_metadata_and_retest_cadence_are_exposed_without_weights(self):
        directory,_=demo_model(generations=2)
        state_path=os.path.join(directory,'run','state.json')
        with open(state_path,encoding='utf-8') as handle: saved=json.load(handle)
        saved['tracks'][0]['elite_pool']=[{'brain_sha256':'abc','brain':{'secret_weights':[1,2]},
            'origin':{'generation':1,'island':'local','candidate':'sample'},'observations':0,
            'mean_gain':0.0,'last_retested':None,'last_reason':'awaiting_fresh_evaluation'}]
        saved['tracks'][0]['last_elite_retest']={'generation':2,'brain_sha256':'abc','reason':'screen_below_champion'}
        with open(state_path,'w',encoding='utf-8') as handle:json.dump(saved,handle)
        config_path=os.path.join(directory,'campaign.json')
        with open(config_path,encoding='utf-8') as handle:config=json.load(handle)
        config['elite_retest_every']=5;config['elite_size']=8
        with open(config_path,'w',encoding='utf-8') as handle:json.dump(config,handle)
        model=cd.load_model(directory)
        self.assertEqual(model['meta']['elite_retest_every'],5)
        self.assertEqual(len(model['tracks'][0]['elite_pool']),1)
        self.assertNotIn('brain',model['tracks'][0]['elite_pool'][0])
        self.assertEqual(model['tracks'][0]['last_elite_retest']['generation'],2)

    def test_wreck_metal_means_and_legacy_unavailable(self):
        def stage(sums=None):
            tally = dict(battles=2)
            if sums is not None:
                tally['wreck_metal_left_sum'] = sums
            return dict(name='wrecks', details=dict(total=tally))
        report = cd.summarize_report({'stages': [stage([12, 36])]})
        self.assertEqual(report['stages'][0]['wreck_metal_left_mean'], [6, 18])
        old = cd.summarize_report({'stages': [stage()]})
        self.assertIsNone(old['stages'][0]['wreck_metal_left_mean'])

    def test_firing_rates_use_pooled_counts_and_legacy_data_is_unavailable(self):
        def stage(name, fired, armed, blocked, ready):
            stats = dict(units_fired=fired, armed_units=armed, blocked_friendly=blocked,
                         blocked_terrain=0, blocked_resources=0, ready_checks=ready, shots_fired=ready-blocked)
            return dict(name=name, details=dict(total=dict(battles=1, firing=[stats,stats])))
        report = cd.summarize_report({'stages':[stage('a',1,2,1,2),stage('b',9,10,0,8)]}, with_rows=True)
        self.assertEqual(report['firing'][0]['units_fired'],10)
        self.assertAlmostEqual(report['firing'][0]['participation_rate'],10/12)
        self.assertAlmostEqual(report['firing'][0]['friendly_blocked_rate'],0.1)
        self.assertEqual(report['stages'][0]['firing'][0]['friendly_blocked_rate'],0.5)
        old = cd.summarize_report({'stages':[{'details':{'total':{'battles':1}}}]})
        self.assertIsNone(old['firing'])

    @classmethod
    def setUpClass(cls):
        cls.directory, cls.model = demo_model()

    def test_both_tracks_and_every_generation_are_read(self):
        names = [t["name"] for t in self.model["tracks"]]
        self.assertEqual(names, ["tactical", "strategic"])
        self.assertEqual([len(t["generations"]) for t in self.model["tracks"]], [40, 40])
        self.assertEqual(self.model["tracks"][0]["kind"], "tactical")
        self.assertEqual(self.model["tracks"][1]["kind"], "strategic")

    def test_tactical_outcomes_add_up_and_have_rows_only_where_needed(self):
        tactical = self.model["tracks"][0]
        outcome = tactical["best"]["outcome"]
        self.assertEqual(outcome["wins"] + outcome["draws"] + outcome["timeouts"] + outcome["losses"], outcome["battles"])
        self.assertTrue(tactical["best"]["stages"][0]["rows"])
        older = tactical["generations"][0]["islands"][0]["center"]
        self.assertTrue(older is None or "rows" not in older["stages"][0])     # rows are kept only for the latest generation

    def test_strategic_trials_are_summarised_to_means(self):
        strategic = self.model["tracks"][1]
        means = strategic["best"]["trial_means"]
        self.assertIn("projects_completed", means)
        self.assertEqual(len(strategic["best"]["trials"]), 18)

    def test_initial_holdout_files_with_a_different_shape_are_understood(self):
        tactical = self.model["tracks"][0]
        self.assertIsNotNone(tactical["holdout_initial"])
        self.assertIsNotNone(tactical["holdout_initial"]["fitness"])

    def test_wrong_directory_is_a_clear_error(self):
        with self.assertRaises(SystemExit):
            cd.locate(tempfile.mkdtemp())

    def test_sensor_support_counts_assigned_sides_without_inventing_old_coverage(self):
        cases=[]
        for a,b in [(None,None),('radar',None),('radar','scout')]:
            cases.append({'family':'duel','sensor_support':[
                {'radar':a,'air_scout':None,'jammer':None},
                {'radar':None,'air_scout':b,'jammer':'jammer'}]})
        result=cd.resource_coverage({'matchup_records':cases},True)
        self.assertEqual(result['sensor_support_coverage'],{'none':1,'one':1,'both':1,'recorded':3})
        self.assertIsNone(cd.resource_coverage({'matchup_records':[{'family':'old'}]},True).get('sensor_support_coverage'))

    def test_resource_coverage_reads_layouts_and_preserves_opponent_identity(self):
        record = {"scenario": {"name": "heavy-swarm", "learner": [{"unit": "corsumo", "count": 1}],
                               "enemy": [{"unit": "armpw", "count": 40}]},
                  "family": "heavy_swarm", "seed": 7, "index": 0, "combat_value": [2783.3, 2760],
                  "starting_value": [2783.3, 2760], "absolute_gap": 0.0084,
                  "opponent_identity": "scripted", "learner_side": 0, "support_status": "supported"}
        historical = dict(record, opponent_identity="champion:abc")
        evaluation = {"matchup_records": [record], "families": {"heavy_swarm": 0.4}}
        details = {"layouts": [{"report": {"baseline": evaluation, "historical": {
            "matchup_records": [historical], "families": {"heavy_swarm": 0.2}}}}]}
        report = {"stages": [{"name": "mixed", "details": details}]}
        entry = cd.summarize_report(report, with_rows=True)["stages"][0]
        self.assertEqual(entry["coverage"], {"heavy_swarm": 2})
        self.assertAlmostEqual(entry["family_fitness"]["heavy_swarm"], 0.3)
        self.assertEqual(entry["matchup_records"][1]["opponent_identity"], "champion:abc")
        self.assertEqual(entry["matchup_records"][0]["scenario"]["enemy"][0]["count"], 40)
        self.assertEqual(entry["matchup_records"][0]["absolute_gap"], 0.0084)
        compact = cd.summarize_report(report)["stages"][0]
        self.assertEqual(compact["coverage"], entry["coverage"])
        self.assertNotIn("matchup_records", compact)

    def test_old_reports_have_explicitly_unavailable_resource_coverage(self):
        entry = cd.summarize_report({"stages": [{"details": {}}]}, True)["stages"][0]
        self.assertEqual(entry["coverage"], {})
        self.assertEqual(entry["matchup_records"], [])

    def test_direct_and_wrapped_copy_of_a_record_is_counted_once(self):
        record = {"scenario": {"name": "air duel"}, "family": "air_air", "seed": 19,
                  "opponent_identity": "scripted", "combat_value": [100, 100]}
        details = {"matchup_records": [record], "families": {"air_air": 0.5},
                   "layouts": [{"report": {"matchup_records": [record]}}]}
        result = cd.resource_coverage(details, True)
        self.assertEqual(result["coverage"], {"air_air": 1})
        self.assertEqual(result["matchup_records"], [record])
        self.assertEqual(result["family_fitness"], {"air_air": 0.5})

    def test_pooled_native_family_score_is_authoritative_over_layout_scores(self):
        details = {"families": {"heavy_swarm": 0.2}, "layouts": [
            {"report": {"families": {"heavy_swarm": 0.9}}},
            {"report": {"families": {"heavy_swarm": 0.9}}}]}
        self.assertEqual(cd.resource_coverage(details, False)["family_fitness"], {"heavy_swarm": 0.2})

    def test_opponent_wrapper_weights_each_roster_equally_despite_layout_count(self):
        details = {"baseline": {"layouts": [
            {"report": {"families": {"heavy_swarm": 0.8}}},
            {"report": {"families": {"heavy_swarm": 0.8}}}]},
            "historical": {"families": {"heavy_swarm": 0.2}}}
        self.assertEqual(cd.resource_coverage(details, False)["family_fitness"], {"heavy_swarm": 0.5})

    def test_compacted_history_uses_full_coverage_counts_instead_of_saved_examples(self):
        details = {"coverage_counts": {"heavy_swarm": 80, "artillery": 20},
                   "matchup_records_sampled": True,
                   "matchup_records": [{"family": "heavy_swarm", "seed": 1, "index": 0}]}
        result = cd.resource_coverage(details, True)
        self.assertEqual(result["coverage"], {"heavy_swarm": 80, "artillery": 20})
        self.assertTrue(result["matchup_records_sampled"])
        self.assertEqual(len(result["matchup_records"]), 1)

    def test_air_brain_identity_is_independent_of_track_label(self):
        directory, _ = demo_model(generations=1)
        path = os.path.join(directory, "run", "state.json")
        state = cd.read_json(path)
        state["tracks"][0]["best"]["kind"] = "tactical-air-v1"
        with open(path, "w", encoding="utf-8") as handle:
            json.dump(state, handle)
        self.assertEqual(cd.load_model(directory)["tracks"][0]["brain_domain"], "air")

    def test_versioned_ground_brain_kind_is_recognized(self):
        self.assertEqual(cd.brain_domain({"best": {"kind": "tactical_v4"}}), "ground")

    def test_successor_package_run_uses_its_profile_configuration(self):
        with tempfile.TemporaryDirectory() as root:
            with open(os.path.join(root, "receipt.json"), "w", encoding="utf-8") as handle:
                json.dump({"format": "minibar.diverse_successor", "version": 1}, handle)
            profiles = {"pilot": ("pilot.json", 0.001, 1),
                        "cohesion": ("pilot-cohesion.json", 0.002, 2),
                        "production": ("campaign.json", 0.005, 5)}
            for profile, (filename, gain, cadence) in profiles.items():
                with self.subTest(profile=profile):
                    run = os.path.join(root, "runs", profile)
                    os.makedirs(run)
                    with open(os.path.join(run, "state.json"), "w", encoding="utf-8") as handle:
                        json.dump({"tracks": [], "history": []}, handle)
                    with open(os.path.join(root, filename), "w", encoding="utf-8") as handle:
                        json.dump({"name": profile, "min_gain": gain, "validation_every": cadence}, handle)
                    self.assertEqual(cd.locate(run), (root, run))
                    model = cd.load_model(run)
                    self.assertEqual(model["meta"]["min_gain"], gain)
                    self.assertEqual(model["meta"]["validation_every"], cadence)
                    self.assertEqual(model["meta"]["directory"], root)


class DriftTest(unittest.TestCase):
    def brain(self, w):
        return {"modules": [{"name": "spacing", "inputs": ["a", "b"], "role": "r",
                             "layers": [{"weights": [w], "biases": [0.0]}]}]}

    def test_untouched_module_has_zero_drift(self):
        drift = cd.brain_drift(self.brain([1.0, 2.0]), self.brain([1.0, 2.0]))
        self.assertEqual(drift[0]["change"], 0.0)
        self.assertEqual(drift[0]["top_inputs"], [])

    def test_the_moved_input_is_named_first(self):
        drift = cd.brain_drift(self.brain([1.0, 2.0]), self.brain([1.0, 2.5]))
        self.assertEqual(drift[0]["top_inputs"][0]["input"], "b")
        self.assertAlmostEqual(drift[0]["change"], 0.5)
        self.assertGreater(drift[0]["relative"], 0.0)

    def test_missing_or_mismatched_brains_give_no_drift(self):
        self.assertEqual(cd.brain_drift(None, self.brain([1.0, 2.0])), [])
        other = {"modules": [{"name": "spacing", "inputs": ["a", "b"], "layers": []}]}
        self.assertEqual(cd.brain_drift(self.brain([1.0, 2.0]), other), [])


class EventsAndVerdictTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory, cls.model = demo_model()
        cls.tactical, cls.strategic = cls.model["tracks"]

    def kinds(self, track):
        return {e["kind"] for e in track["events"]}

    def test_champion_changes_and_restarts_are_reported(self):
        self.assertIn("champion", self.kinds(self.tactical))
        self.assertIn("restart", self.kinds(self.tactical))

    def test_a_stall_is_reported_once_ten_generations_pass_without_a_new_champion(self):
        self.assertEqual(self.tactical["best_generation"], 30)
        self.assertIn("stall", self.kinds(self.tactical))
        self.assertEqual(self.tactical["verdict"]["level"], "stalled")

    def test_flat_stage_spread_is_flagged_as_no_learning_signal(self):
        self.assertIn("signal", self.kinds(self.strategic))

    def test_rejected_build_orders_are_flagged(self):
        self.assertIn("rejections", self.kinds(self.strategic))

    def test_events_are_newest_first(self):
        generations = [e["generation"] for e in self.tactical["events"]]
        self.assertEqual(generations, sorted(generations, reverse=True))

    def test_early_runs_never_claim_progress(self):
        _, short = demo_model(generations=2)
        for track in short["tracks"]:
            self.assertIn(track["verdict"]["level"], ("early", "regress", "flat", "waiting"))
            self.assertIn("too early", track["verdict"]["text"].lower())

    def test_overlapping_win_rate_intervals_are_called_noise(self):
        track = {"initial": {"fitness": 0.0, "outcome": {"wins": 5, "battles": 10}, "stages": []},
                 "best": {"fitness": 0.1, "outcome": {"wins": 6, "battles": 10}, "stages": []},
                 "generations": [{"generation": g} for g in range(1, 8)], "best_generation": 7}
        verdict = cd.build_verdict(track, {})
        self.assertIn("within noise", verdict["text"])
        self.assertEqual(verdict["level"], "early")

    def test_separated_intervals_are_not_called_noise(self):
        track = {"initial": {"fitness": 0.0, "outcome": {"wins": 20, "battles": 200}, "stages": []},
                 "best": {"fitness": 0.5, "outcome": {"wins": 150, "battles": 200}, "stages": []},
                 "generations": [{"generation": g} for g in range(1, 12)], "best_generation": 11}
        verdict = cd.build_verdict(track, {})
        self.assertNotIn("within noise", verdict["text"])
        self.assertEqual(verdict["level"], "progress")

    def test_forgetting_and_overfitting_warnings(self):
        track = {"initial": {"fitness": 0.0, "stages": [{"name": "easy", "fitness": 0.5}]},
                 "best": {"fitness": 0.2, "stages": [{"name": "easy", "fitness": 0.2}]},
                 "holdout": {"fitness": -0.5}, "holdout_initial": {"fitness": -0.1},
                 "generations": [{"generation": 3, "best_generation": 3, "islands": [], "champion_fitness": 0.2}],
                 "best_generation": 3}
        kinds = {e["kind"] for e in cd.build_events(track, {"max_stage_regression": 0.15})}
        self.assertIn("forgetting", kinds)
        self.assertIn("overfit", kinds)


def retest(champion, extra=()):
    """A fresh_retest.py document; champion is {family: (diff, t)}, extra is [(brain, {family: (diff, t)})]."""
    names = list(champion)
    brains = [("champion", champion)] + list(extra)
    return {"format": "minibar.fresh_retest", "version": 1, "track": "tactical", "seeds": 100, "baseline": "start",
            "families": [{"name": f, "label": f, "n": 100, "baseline": 0.2,
                          "brains": [{"name": n, "mean": 0.2 + c[f][0], "diff": c[f][0], "se": 0.01, "t": c[f][1], "n": 100}
                                     for n, c in brains if f in c]} for f in names]}


class FreshRetestTest(unittest.TestCase):
    def test_gain_on_validation_but_loss_on_holdout_is_mixed_evidence_and_an_overfit_event(self):
        directory, model = demo_model(generations=3)
        track = model["tracks"][0]
        track["retest"] = retest({"validation": (0.05, 12.0), "holdout": (-0.03, -2.5), "training": (0.02, 3.0)})
        verdict = cd.build_verdict(track, model["meta"])
        self.assertEqual(verdict["level"], "mixed")
        self.assertIn("Fresh-seed retest", verdict["text"])
        self.assertTrue(verdict["text"].startswith("Fresh-seed retest"))
        self.assertTrue(any(e["kind"] == "overfit" and "retest" in e["text"] for e in cd.build_events(track, model["meta"])))

    def test_gains_everywhere_are_progress(self):
        directory, model = demo_model(generations=3)
        track = model["tracks"][0]
        track["retest"] = retest({"validation": (0.05, 5.0), "holdout": (0.04, 3.0)})
        self.assertEqual(cd.build_verdict(track, model["meta"])["level"], "progress")

    def test_noise_everywhere_is_flat_and_small_t_values_are_not_gains(self):
        directory, model = demo_model(generations=3)
        track = model["tracks"][0]
        track["retest"] = retest({"validation": (0.05, 1.9), "holdout": (-0.04, -1.9)})
        verdict = cd.build_verdict(track, model["meta"])
        self.assertEqual(verdict["level"], "flat")
        self.assertIn("No family shows a clear difference", verdict["text"])

    def test_validation_gain_without_a_holdout_gain_is_not_full_progress(self):
        directory, model = demo_model(generations=3)
        track = model["tracks"][0]
        track["retest"] = retest({"validation": (0.05, 8.0), "holdout": (0.0, 0.1)})
        self.assertEqual(cd.build_verdict(track, model["meta"])["level"], "early")

    def test_an_island_brain_ahead_on_the_holdout_is_named(self):
        directory, model = demo_model(generations=3)
        track = model["tracks"][0]
        track["retest"] = retest({"validation": (0.05, 8.0), "holdout": (-0.03, -2.5)},
                                 extra=[("local", {"validation": (0.0, 0.5), "holdout": (0.05, 4.6)})])
        self.assertIn("Not champion, but ahead on the holdout: local", cd.build_verdict(track, model["meta"])["text"])

    def test_missing_or_foreign_retest_files_are_ignored(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertIsNone(cd.load_retest(directory, "tactical"))
            with open(os.path.join(directory, "fresh-retest.tactical.json"), "w", encoding="utf-8") as handle:
                json.dump({"format": "something else"}, handle)
            self.assertIsNone(cd.load_retest(directory, "tactical"))

    def test_a_retest_beside_the_campaign_is_loaded_into_the_model_and_rendered(self):
        directory, _ = demo_model(generations=3)
        with open(os.path.join(directory, "fresh-retest.tactical.json"), "w", encoding="utf-8") as handle:
            json.dump(retest({"validation": (0.05, 12.0)}), handle)
        model = cd.load_model(directory)
        tactical = next(t for t in model["tracks"] if t["name"] == "tactical")
        self.assertEqual(tactical["retest"]["seeds"], 100)
        self.assertIn("chartRetest", cd.render(model))

    def test_without_a_retest_a_fresh_screened_tactical_run_points_to_the_tool(self):
        directory, model = demo_model(generations=3)
        track = model["tracks"][0]
        track["retest"] = None
        meta = dict(model["meta"], fresh_screening=True)
        track["kind"] = "tactical"
        self.assertIn("scripts/fresh_retest.py", cd.build_verdict(track, meta)["text"])


class TemplateTest(unittest.TestCase):
    def test_resize_redraw_ignores_height_changes_from_expanding_tables(self):
        import shutil
        import subprocess
        node = shutil.which('node')
        if not node:
            self.skipTest('Node is needed for the resize regression test')
        with open(cd.TEMPLATE, encoding='utf-8') as handle:
            template = handle.read()
        function = 'function observeDashboardWidth' + template.split('function observeDashboardWidth', 1)[1].split('\nfunction boot()', 1)[0]
        source = r'''
const assert = require('node:assert/strict');
let observer, redraws=0, width=1000;
const main={getBoundingClientRect:()=>({width})};
class ResizeObserver {constructor(cb){observer=cb;}observe(node){assert.equal(node,main);}}
const setTimeout=fn=>{fn();return 1;},clearTimeout=()=>{};
''' + function + r'''
observeDashboardWidth(main,()=>redraws++);
observer([{contentRect:{width:1000,height:500}}]);
observer([{contentRect:{width:1000,height:1500}}]);
assert.equal(redraws,0,'Opening a table must not rebuild it');
observer([{contentRect:{width:800,height:1500}}]);
assert.equal(redraws,1,'A width change must still resize charts');
observer([{contentRect:{width:800,height:500}}]);
assert.equal(redraws,1);
'''
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, 'resize-test.js')
            with open(path, 'w', encoding='utf-8') as handle:
                handle.write(source)
            result = subprocess.run([node, path], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_coverage_explains_validation_and_unlimited_supply_in_readable_cards(self):
        import shutil
        import subprocess
        node = shutil.which("node")
        if not node:
            self.skipTest("node is not installed")
        with open(cd.TEMPLATE, encoding="utf-8") as handle:
            template = handle.read()
        function = template.split("function chartCoverage(t) {", 1)[1].split("function chartRetest", 1)[0]
        source = r'''
const assert = require('node:assert/strict');
const h = (tag, attrs, ...kids) => ({tag, attrs:attrs||{}, kids:kids.flat(Infinity),
  append(...more){this.kids.push(...more.flat(Infinity));}, replaceChildren(...more){this.kids=more.flat(Infinity);}, classList:{add(){}}});
const card=(title,note)=>h('section',{},title,note);
const tableView=(headers,rows)=>h('table',{},headers,rows);
const f3=x=>x==null?'–':x.toFixed(3), num=x=>String(x);
const state={island:null,kind:'center'};
const islandAt=()=>null;
const text=n=>typeof n==='object'&&n!==null?[n.attrs?.text||'',...(n.kids||[]).map(text)].join(' '):String(n);
''' + "function chartCoverage(t) {" + function + r'''
const record={scenario:{name:'Mammoth versus Pawns',learner:[{unit:'corsumo',count:1}],enemy:[{unit:'armpw',count:40}]},
  family:'heavy_swarm',combat_value:[2783.3,2760],objective_value:[0,0],starting_value:[2783.3,2760],resource_supply:null};
const view=chartCoverage({brain_domain:'ground',generations:[],best:{stages:[{name:'mixed',coverage:{heavy_swarm:80},family_fitness:{heavy_swarm:0.4},matchup_records:[record],matchup_records_sampled:true}]}});
view.draw();
const output=text(view.el);
for(const label of ['Validation fight coverage','Only this coverage panel changes','matchup category','1 × corsumo','40 × armpw',
  'Requested budget','Starting investment','Unlimited firing resources','Modeled mechanics support','saved examples']) assert.ok(output.includes(label),label);
assert.ok(!output.includes('[object Object]'));
'''
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "coverage-test.js")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(source)
            result = subprocess.run([node, path], capture_output=True, text=True, encoding="utf-8")
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_every_script_block_parses(self):
        import re
        import shutil
        import subprocess
        node = shutil.which("node")
        if not node:
            self.skipTest("node is not installed")
        with open(cd.TEMPLATE, encoding="utf-8") as handle:
            blocks = re.findall(r"<script>([\s\S]*?)</script>", handle.read())
        self.assertGreaterEqual(len(blocks), 2, "the error handler must be its own block")
        for number, source in enumerate(blocks):
            path = os.path.join(tempfile.mkdtemp(), f"block{number}.js")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(source)
            result = subprocess.run([node, "--check", path], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)


class OutputTest(unittest.TestCase):
    def test_page_embeds_data_and_survives_hostile_text(self):
        directory, model = demo_model(generations=3)
        model["tracks"][0]["events"].append({"generation": 1, "kind": "x", "level": "info", "text": "</script><script>alert(1)</script>"})
        html = cd.render(model)
        self.assertNotIn("</script><script>alert(1)", html)
        self.assertNotIn("/*__DATA__*/null", html)

    def test_live_mode_sets_the_refresh_interval(self):
        directory, model = demo_model(generations=3)
        self.assertIn('"live_seconds":15', cd.render(model, live_seconds=15))

    def test_write_is_atomic_and_leaves_no_temp_file(self):
        directory, _ = demo_model(generations=3)
        out = os.path.join(directory, "page.html")
        cd.write_dashboard(directory, out)
        self.assertTrue(os.path.getsize(out) > 1000)
        self.assertFalse(os.path.exists(out + ".tmp"))

    def test_a_half_written_history_line_is_ignored(self):
        directory, _ = demo_model(generations=3)
        run = os.path.join(directory, "run")
        os.remove(os.path.join(run, "state.json"))
        with open(os.path.join(run, "history.jsonl"), "w", encoding="utf-8") as handle:
            handle.write(json.dumps({"generation": 1, "tracks": []}) + "\n" + '{"generation": 2, "tr')
        model = cd.load_model(directory)
        self.assertEqual(model["meta"]["completed_generations"], 1)

    def test_nothing_in_the_campaign_directory_is_modified(self):
        directory, _ = demo_model(generations=3)
        before = {}
        for root, _, files in os.walk(directory):
            for name in files:
                path = os.path.join(root, name)
                before[path] = (os.path.getmtime(path), os.path.getsize(path))
        cd.load_model(directory)
        after = {}
        for root, _, files in os.walk(directory):
            for name in files:
                path = os.path.join(root, name)
                after[path] = (os.path.getmtime(path), os.path.getsize(path))
        self.assertEqual(before, after)


if __name__ == "__main__":
    unittest.main()
