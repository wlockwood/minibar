"""Structural history must describe executable graphs without inventing progress."""
import copy
import json
from pathlib import Path
import tempfile
import shutil
import subprocess
import unittest

import campaign_dashboard as dashboard


def brain():
    return {"format": "minibar-brain", "version": 2, "kind": "tactical_v4", "modules": [{
        "name": "spacing", "role": "Approach or back away", "inputs": ["health", "distance"],
        "hidden": [2], "outputs": ["advance"], "layers": [
            {"weights": [[1., 2.], [3., 4.]], "biases": [0., 0.]},
            {"weights": [[1., 1.]], "biases": [0.]}],
        "structure": {"inputs": [True, True], "connections": [[[True, True], [True, True]], [[True, True]]],
                      "hidden": [["tanh", "tanh"]]}}]}


class StructureTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("node"), "Node is needed to verify standalone viewer calculations")
    def test_both_brain_watch_pages_execute_masks_and_activations(self):
        root = Path(__file__).resolve().parents[1]
        for name in ("viewer/brain_watch.html",):
            source = (root / name).read_text(encoding="utf-8")
            functions = source[source.index("function edgeEnabled("):source.index("const macs")]
            mod = {"layers": [{"weights": [[1, 100]], "biases": [0]},
                              {"weights": [[1]], "biases": [0]}],
                   "structure": {"inputs": [True, False], "connections": [[[True, True]], [[True]]], "hidden": [["softsign"]]}}
            script = functions + "\nconst mod=" + json.dumps(mod) + ";\n" + """
                const actual=forward(mod,[3,10]).acts.at(-1)[0];
                if(Math.abs(actual-Math.tanh(.75))>1e-12) throw Error('disabled input or Softsign differs');
                mod.structure.inputs[1]=true;mod.structure.connections[0][0][1]=false;
                if(forward(mod,[3,10]).acts.at(-1)[0]!==actual) throw Error('disabled edge differs');
                mod.structure.hidden[0][0]='leaky_relu';
                if(Math.abs(forward(mod,[-5,10]).acts.at(-1)[0]-Math.tanh(-.05))>1e-12) throw Error('Leaky ReLU differs');
                delete mod.structure;
                if(forward(mod,[0,0]).acts.at(-1)[0]!==0) throw Error('legacy defaults differ');
            """
            result = subprocess.run([shutil.which("node")], input=script, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, f"{name}: {result.stderr}")

    def test_counts_distinguish_enabled_inputs_and_connected_paths(self):
        value = brain()
        value["modules"][0]["structure"]["connections"][0] = [[False, True], [False, True]]
        module = dashboard.summarize_structure(value)["modules"][0]
        self.assertEqual(module["enabled_inputs"], 2)
        self.assertEqual(module["connected_inputs"], 1)
        self.assertEqual(module["enabled_connections"], 4)

    def test_all_three_changes_are_named_and_dormant_weights_do_not_drift(self):
        parent = brain()
        child = copy.deepcopy(parent)
        structure = child["modules"][0]["structure"]
        structure["inputs"][0] = False
        structure["hidden"][0][1] = "softsign"
        structure["connections"][1][0][1] = False
        child["modules"][0]["layers"][0]["weights"][0][0] += 500
        changes = dashboard.structure_diff(parent, child)
        self.assertEqual({c["kind"] for c in changes}, {"input", "activation", "connection"})
        self.assertEqual(dashboard.brain_drift(parent, child)[0]["change"], 0)

    def test_legacy_defaults_and_malformed_metadata(self):
        legacy = brain()
        legacy["version"] = 1
        del legacy["modules"][0]["structure"]
        module = dashboard.summarize_structure(legacy)["modules"][0]
        self.assertEqual(module["enabled_connections"], 6)
        malformed = brain()
        malformed["modules"][0]["structure"]["connections"][0][0].pop()
        self.assertFalse(dashboard.summarize_structure(malformed)["available"])

    def test_legacy_history_is_not_invented_and_future_events_are_hidden(self):
        value = brain()
        saved = {"base": value, "best": value}
        history = dashboard.structure_history(saved, 3)
        self.assertFalse(history["structural_history_available"])
        saved.update(structure_snapshots={"parent": value}, structural_history=[
            {"generation": 5, "proposal_sha256": "missing", "parent_sha256": "parent"}])
        history = dashboard.structure_history(saved, 3)
        self.assertEqual(history["structural_events"], [])
        saved["structural_history"][0]["generation"] = 2
        history = dashboard.structure_history(saved, 3)
        self.assertFalse(history["structural_events"][0]["available"])

    def test_embedded_labels_cannot_close_script(self):
        with tempfile.TemporaryDirectory() as directory:
            dashboard.make_demo(directory, 5)
            model = dashboard.load_model(directory)
            model["tracks"][0]["name"] = "</script><script>throw 42</script>"
            self.assertNotIn("</script><script>throw 42", dashboard.render(model))

    def test_demo_snapshots_match_the_performance_promotion_records(self):
        with tempfile.TemporaryDirectory() as directory:
            dashboard.make_demo(directory, 40)
            state = json.loads((Path(directory) / "run/state.json").read_text(encoding="utf-8"))
            for track in state["tracks"]:
                expected = [(g["generation"], island["name"], kind)
                            for g in state["history"] for row in g["tracks"] if row["name"] == track["name"]
                            for island in row["islands"] for kind in island["promoted"]]
                actual = [(p["generation"], p["island"], p["candidate"]) for p in track["promotion_history"]]
                self.assertEqual(actual, expected)


if __name__ == "__main__":
    unittest.main()
