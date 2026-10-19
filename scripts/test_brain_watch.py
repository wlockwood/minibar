"""Exercise decision presentation and candidate selection using the actual viewer functions."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class BrainWatchTest(unittest.TestCase):
    def test_air_signals_and_target_selection_preserve_ground_decisions(self):
        node = shutil.which("node")
        if not node:
            self.skipTest("node is not installed")
        template = (Path(__file__).resolve().parents[1] / "viewer" / "brain_watch.html").read_text(encoding="utf-8")
        functions = "function currentRun" + template.split("function currentRun", 1)[1].split("// ---------- battlefield", 1)[0]
        source = r'''
const assert = require('node:assert/strict');
const nodes=new Map();
const $=id=>{if(!nodes.has(id))nodes.set(id,{textContent:'',innerHTML:'',style:{},classList:{toggle(){}},querySelectorAll(){return [];}});return nodes.get(id);};
const document={activeElement:null};
let S,decision,selectedInputs;
const frameAt=()=>0, selectedUnit=()=>({id:7}), decisionOf=()=>decision;
const forward=(mod,input)=>({acts:[input,input]});
const updateGraph=values=>{selectedInputs=values.acts[0];},drawField=()=>{};
const fmt=x=>x.toFixed(2);
''' + functions + r'''
function setup(kind){
  const air=kind==='tactical-air-v1';
  const target={name:air?'air_target':'target_selector',inputs:['distance'],outputs:['priority']};
  const flight={name:air?'air_flight':'spacing',inputs:['distance'],outputs:air?['turn','throttle','extend_run']:['advance']};
  S={meta:{brain:{kind,modules:[target,flight]},input_ranges:{distance:[0,1]}},graph:{mod:target},battle:{_duration:3},t:0,cand:12,mod:0,probe:null};
  decision={target:11,throttle:0.8,commit:0.4,advance:0.5,orbit:0.1,regroup:0,retreat:0,bloom:0,
    runs:[{module:0,candidate:11,inputs:[0.1],outputs:[0.1]},{module:0,candidate:12,inputs:[0.9],outputs:[0.9]},
          {module:1,candidate:11,inputs:[0.1],outputs:[0.25,-0.5,0.75]}]};
}
setup('tactical-air-v1');refresh();
assert.equal($('postH').textContent,'Flight controller');
assert.ok($('postS').textContent.includes('throttle 0.80'));
assert.ok(!$('postS').textContent.includes('platoon'));
for(const label of ['turn signal','throttle signal','extend_run signal','0.25','-0.50','0.75'])assert.ok($('meters').innerHTML.includes(label),label);
assert.ok(!$('meters').innerHTML.includes('advance'));
assert.deepEqual(selectedInputs,[0.9]);
assert.ok($('cands').innerHTML.includes('enemy #12'));
assert.ok($('cands').innerHTML.includes(' sel'));
setup('tactical_v4');refresh();
assert.equal($('postH').textContent,'Closing in');
assert.ok($('postS').textContent.includes('platoon commit 0.40'));
for(const label of ['advance','orbit','regroup','retreat','bloom'])assert.ok($('meters').innerHTML.includes(label));
assert.deepEqual(selectedInputs,[0.9]);
'''
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "brain-watch-test.js"
            path.write_text(source, encoding="utf-8")
            result = subprocess.run([node, str(path)], capture_output=True, text=True, encoding="utf-8")
            self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
