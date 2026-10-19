"""Exercise visible launcher recovery without changing simulator identity."""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


@unittest.skipUnless(shutil.which("pwsh"), "PowerShell required")
class TrainingLauncherTest(unittest.TestCase):
    def run_launcher(self, mode, max_seconds=None):
        temporary = tempfile.TemporaryDirectory(prefix="minibar launcher ")
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        shutil.copyfile(Path(__file__).parent / "training_branch/Train.ps1", root / "Train.ps1")
        (root / "run").mkdir()
        (root / "run/state.json").write_text("{}", encoding="utf-8")
        (root / "Branch-Common.ps1").write_text('''
$ErrorActionPreference = 'Stop'
function Get-BranchPath([string]$Relative) { return Join-Path $PSScriptRoot $Relative }
function Test-BranchInputs {
    return [PSCustomObject]@{Manifest=@{binary='fake.ps1';config='campaign.json'};Lineage=@{branch_id='test'}}
}
''', encoding="utf-8")
        (root / "fake.ps1").write_text('''
$calls = Join-Path $PSScriptRoot 'calls.jsonl'
$number = if (Test-Path $calls) { @(Get-Content $calls).Count + 1 } else { 1 }
ConvertTo-Json -InputObject @($args) -Compress | Add-Content -LiteralPath $calls
$mode = Get-Content (Join-Path $PSScriptRoot 'mode.txt')
if ($number -eq 1 -or $mode -ne 'once') {
    $checkpoint = Join-Path $PSScriptRoot 'run/state.json'
    if ($mode -eq 'unrelated') { Write-Output 'error: campaign config/run: invalid optimizer' }
    elseif ($mode -eq 'other-path') { Write-Output 'error: campaign config/run: commit C:\other\state.json: Access is denied. (os error 5)' }
    else { Write-Output "error: campaign config/run: commit ${checkpoint}: Access is denied. (os error 5)" }
    exit 1
}
Write-Output '{"status":"stopped"}'
exit 0
''', encoding="utf-8")
        (root / "mode.txt").write_text(mode, encoding="utf-8")
        args = [shutil.which("pwsh"), "-NoProfile", "-File", str(root / "Train.ps1"), "-Threads", "1"]
        if max_seconds is not None:
            args.extend(["-MaxSeconds", str(max_seconds)])
        result = subprocess.run(args, capture_output=True, text=True, timeout=30)
        calls = [json.loads(line) for line in (root / "calls.jsonl").read_text(encoding="utf-8").splitlines()]
        return result, calls

    def test_busy_checkpoint_resumes_and_keeps_native_output_visible(self):
        result, calls = self.run_launcher("once")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 2)
        self.assertIn("--resume", calls[1])
        self.assertIn("Access is denied", result.stdout)
        self.assertIn("retry", result.stdout.lower())

    def test_other_errors_and_other_paths_do_not_retry(self):
        for mode in ["unrelated", "other-path"]:
            with self.subTest(mode=mode):
                result, calls = self.run_launcher(mode)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(len(calls), 1)

    def test_retry_consumes_remaining_time_budget_instead_of_resetting_it(self):
        for limit in [2, .001]:
            with self.subTest(limit=limit):
                result, calls = self.run_launcher("once", max_seconds=limit)
                self.assertEqual(result.returncode, 0, result.stderr)
                first = float(calls[0][calls[0].index("--max-seconds")+1])
                second = float(calls[1][calls[1].index("--max-seconds")+1])
                self.assertEqual(first, limit)
                self.assertGreater(second, 0)
                self.assertLess(second, limit)

    def test_persistent_access_denial_stops_after_bounded_retries(self):
        result, calls = self.run_launcher("always")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(calls), 6)
        self.assertIn("Native training failed", result.stderr)


if __name__ == "__main__":
    unittest.main()
