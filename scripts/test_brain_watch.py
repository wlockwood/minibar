"""Exercise decision presentation and candidate selection using the actual viewer functions."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class BrainWatchTest(unittest.TestCase):
    def test_late_wreck_metadata_renders_neutral_and_cannot_be_followed(self):
        node = shutil.which("node")
        if not node:
            self.skipTest("node is not installed")
        template = (Path(__file__).resolve().parents[1] / "viewer" / "brain_watch.html").read_text(encoding="utf-8")
        functions = "function setBattle" + template.split("function setBattle", 1)[1].split("async function loadLive", 1)[0]
        functions += "function selectedUnit" + template.split("function selectedUnit", 1)[1].split("// Outputs", 1)[0]
        functions += "function drawField" + template.split("function drawField", 1)[1].split("cv.onclick", 1)[0]
        source = r'''
const assert = require('node:assert/strict');
const nodes = new Map();
const $ = id => {if(!nodes.has(id))nodes.set(id,{value:0,style:{},innerHTML:'',append(){}});return nodes.get(id);};
const S={meta:{scenarios:[{name:'Test',learner:'A',enemy:'B'}],opponents:['charge']},playing:false};
const refresh=()=>{},fit=()=>[400,300],updateCam=()=>{},proj=()=>({s:1,p:(x,z)=>[x,z]});
const clamp=(x,a,b)=>Math.max(a,Math.min(x,b)),lerp=(a,b,t)=>a+(b-a)*t,lerpAngle=lerp,fmt=x=>x.toFixed(2);
const fills=[],labels=[];
const cx=new Proxy({}, {get(target,key){if(key==='fill')return()=>fills.push(target.fillStyle);if(key==='fillText')return text=>labels.push(text);return target[key]??(()=>{});},set(target,key,value){target[key]=value;return true;}});
const document={createElement:()=>({})};
''' + functions + r'''
const b={learner_side:0,scenario:0,opponent:0,seed:1,range:0,outcome:'win',fitness:1,exchange:0,
 units:[{id:1,side:0,max_hp:10,radius:4},{id:9,side:255,max_hp:20,radius:5,reclaim_metal:25}],
 frames:[{time:0,u:[[1,0,0,0,10]],f:[],d:[]},{time:1,u:[[1,0,0,0,0],[9,20,0,0,20]],f:[],d:[]}]};
setBattle(b);
assert.equal(b.frames[1]._by.get(9).reclaim_metal,25);
S.follow=9;
assert.equal(selectedUnit(1),null,'Neutral wreck must not become a followed learner');
S.t=0.5;drawField(1,null,null);
assert.ok(fills.includes('rgba(150,133,120,0.9)'),'Wreck body must use neutral brown');
assert.ok(!fills.includes('rgba(255,93,93,0.9)'),'No enemy-red wreck');
assert.ok(labels.includes('25.00 M'));
'''
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "wreck-watch-test.js"
            path.write_text(source, encoding="utf-8")
            result = subprocess.run([node, str(path)], capture_output=True, text=True, encoding="utf-8")
            self.assertEqual(result.returncode, 0, result.stderr)

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
