import unittest
import tempfile
import subprocess
import sys
from pathlib import Path
from run_from_replays import configurations


class ConfigurationTests(unittest.TestCase):
    def test_refused_rerun_does_not_modify_existing_package(self):
        with tempfile.TemporaryDirectory() as directory:
            out=Path(directory);(out/'tactical/trial').mkdir(parents=True)
            sentinel=out/'tactical/baseline.json';sentinel.write_text('keep')
            result=subprocess.run([sys.executable,str(Path(__file__).with_name('run_from_replays.py')),
                                   '--out',str(out)],capture_output=True,text=True)
            self.assertNotEqual(result.returncode,0)
            self.assertIn('exists; use a fresh --run-name',result.stderr)
            self.assertEqual(sentinel.read_text(),'keep')

    def test_curricula_include_all_factions_objectives_and_real_maps(self):
        manifest={"factions":[{"name":n,"builder":n+"ck","mex":n+"mex","power":n+"solar","factory":n+"lab","combat":[n+"pw",n+"rock"]} for n in ("arm","cor","leg")],
                  "maps":[{"cache":"maps/real.vsgmap","name":"real","movement_class":"bot2","cell_size":64,"center":[1000,1000],"starts":[{"x":100,"z":100},{"x":1900,"z":1900}]}]*3}
        tactical,strategic,scale=configurations(manifest,3)
        self.assertEqual(tactical["brain_kind"],"tactical_v4")
        self.assertEqual(len(tactical["scenarios"]),9)
        self.assertEqual({s["objective"]["kind"] for s in tactical["scenarios"] if "objective" in s},{"attack","defend"})
        self.assertEqual(len(strategic["factions"]),3)
        self.assertEqual(strategic["generations"],3)
        self.assertEqual(scale["scenarios"][0]["learner"][0]["count"],64)
        self.assertTrue(tactical["arena"]["map"].startswith("../maps/"))


if __name__=="__main__":
    unittest.main()
