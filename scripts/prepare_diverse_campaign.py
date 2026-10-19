"""Prepare an independent, portable ground/air successor; never launch training."""
from __future__ import annotations
import argparse
import copy
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

import export_training_catalog as exporter
from package_training import refuse_reparse

ROOT = Path(__file__).resolve().parents[1]
CANDIDATE = ROOT / "data/experiments/tactical-plateau-20261018/cohesion-candidate.json"
REFERENCE = ROOT / "data/branches/tactical-recovery-20261018/run/tracks/tactical/brain.best.json"
STATE = ROOT / "data/branches/tactical-recovery-20261018/run/state.json"


def encoded(document):
    return (json.dumps(document, indent=2, sort_keys=True, allow_nan=False) + "\n").encode("utf-8")


def write(path, document):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(encoded(document))


def digest(path):
    with Path(path).open("rb") as handle:
        result = hashlib.sha256()
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def campaign_documents(matchups, seed, elite_brains=None):
    """Build profiles with identical train/validation distributions and isolated holdout."""
    if type(seed) is not int or not 0 <= seed < (1 << 64) - 2048:
        raise ValueError("seed must reserve room for all pilot optimizer streams")
    documents = {}
    elite_brains = elite_brains or {}
    families = {name: [] for name in ("ground", "air")}
    anchors = {a["name"]: a for a in matchups["anchors"]}
    for family, anchor_name in [("heavy_swarm", "Mammoth vs 40 Pawns"),
                                ("artillery", "Starlight vs Banisher"),
                                ("rare_turret", "Ragnarok vs 30 Mammoths")]:
        anchor = copy.deepcopy(anchors[anchor_name]); anchor["family"] = family
        families["ground"].append((family, [anchor], []))
    reverse = copy.deepcopy(anchors["Mammoth vs 40 Pawns"])
    reverse.update(name="40 Pawns vs Mammoth", family="swarm_heavy")
    reverse["learner"], reverse["enemy"] = reverse["enemy"], reverse["learner"]
    families["ground"].append(("swarm_heavy", [reverse], []))
    for family, a, b, domain in [("mixed_ground", "ground", "ground", "ground"),
                                  ("mobile_turret", "ground", "turrets", "ground"),
                                  ("ground_aa", "aa", "bombers", "ground"),
                                  ("air_air", "fighters", "fighters", "air"),
                                  ("bomber_aa", "bombers", "aa", "air")]:
        quota = {"family": "bomber_aa" if family == "ground_aa" else family,
                 "learner_type": a, "enemy_type": b}
        quotas = [quota]
        if family == "mobile_turret":
            quotas.append({"family": family,"learner_type": b,"enemy_type": a})
        families[domain].append((family, [], quotas))

    def stages(domain, profile):
        stages = []
        for family, anchor, quota in families[domain]:
            matchup = copy.deepcopy(matchups)
            matchup.update({"anchors": anchor, "families": quota, "count": 3 if profile == "pilot" else 6})
            matchup["starting_distance_mode"] = "unit_ranges"
            matchup["sensor_support"] = {"radars":["armrad","corrad"],
                "air_scouts":["armpeep","corfink"],"jammers":["armjamt","corjamt"],"jammer_share":1/3}
            for key in ("underdog_ratio", "even_ratio", "overdog_ratio"):
                matchup[key] = [0.98, 1.02]
            path = f"stages/{profile}/{domain}-{family}.json"
            other = "air" if domain == "ground" else "ground"
            config = {"name": f"{domain} {family} {profile}",
                      "brain_kind": "tactical_v4" if domain == "ground" else "tactical-air-v1",
                      "companion_brain": f"../../brains/{other}.initial.json",
                      "resource_supply": {"metal": 1000000, "energy": 1000000},
                      "objective": {"win_stake_floor": 0.0},
                      "opponents": ["charge", "kite", "hold", "surround", f"brain:../../brains/{domain}.frozen.json"],
                      "max_time": 30 if profile == "pilot" else 120,
                      "layout_jitter": 30, "seeds_per_battle": 1, "scenarios": [], "matchups": matchup}
            # Non-combat objective investment stays separate from combat turret packing.
            if family in ("mixed_ground", "mobile_turret", "bomber_aa"):
                config["matchups"].update({"attack_share": 0.25,"defend_share": 0.25,"targets": ["armsolar"]})
            documents[path] = config
            stages.append({"name": family,"difficulty": 2,"weight": 1.0,
                           "task": {"kind": "tactical", "config": path}})
        # Keep the requested composition comparisons near equal investment.
        # Unequal-force exercises remain explicit, lightly weighted controls.
        if profile != "pilot":
            base_family = "mixed_ground" if domain == "ground" else "air_air"
            for band, ratios in [("disadvantage",[1.25,1.6]), ("advantage",[0.6,0.8])]:
                path = f"stages/{profile}/{domain}-resource_{band}.json"
                config = copy.deepcopy(documents[f"stages/{profile}/{domain}-{base_family}.json"])
                config["name"] = f"{domain} resource {band} {profile}"
                config["matchups"]["count"] = 3
                for key in ("underdog_ratio", "even_ratio", "overdog_ratio"):
                    config["matchups"][key] = ratios
                documents[path] = config
                stages.append({"name":f"resource_{band}","difficulty":2,"weight":0.25,
                               "task":{"kind":"tactical","config":path}})
        return stages

    for name, profile in [("campaign.json", "production"), ("pilot.json", "pilot")]:
        pilot = profile == "pilot"
        campaign = {"format": "minibar-campaign", "version": 1,
                    "name": f"reconnaissance training {profile}", "seed": seed + int(pilot),
                    "generations": 3 if pilot else 200, "validation_every": 1 if pilot else 5,
                    "promotion_seeds": 4 if pilot else 12,"min_gain": 0.001,
                    "fresh_screening": True,"screen_min_gain": 0.0,"promotion_min_t": 2.0,
                    "max_stage_regression": 0.15,"recentre_on_promotion": True,
                    "module_recombination_every": 1 if pilot else 5,
                    "elite_retest_every": 1 if pilot else 5,"elite_size": 8,
                    "reanchor_after": 1 if pilot else 30,"archive_size": 0,"restart_after": 30,
                    "restart_scale": 0.25,"tracks": []}
        for index, domain in enumerate(("ground", "air")):
            stage_list = stages(domain, profile)
            def plan(stage_list, layouts=1, seeds=1):
                return {"catalog": "catalog/catalog.json","stages": copy.deepcopy(stage_list),"layouts": layouts,"seeds": seeds}
            def island(label, population, offset):
                return {"name": label,"es": {"population": population,"sigma": 0.04,"learning_rate": 0.01,
                                             "weight_decay": 0.0,"l1": 0.0,"seed": seed + int(pilot)*1000 + index * 100 + offset},
                        "restart": False,"follow_champion": label == "local"}
            islands = [island("local", 4 if pilot else 32, 10)]
            if not pilot:
                structural = island("structural", 24, 20)
                structural["structural_search"] = {"every": 30}
                islands.append(structural)
            track = {"name": domain,"brain": f"brains/{domain}.initial.json",
                     "elite_brains": list(elite_brains.get(domain, [])),
                     "reference_brain": f"brains/{domain}.reference.json","train_modules": [],
                     "training": plan(stage_list), "validation": plan(stage_list, 2),"islands": islands}
            if not pilot:
                track["holdout"] = plan(stages(domain, "holdout"), 2, 2)
            campaign["tracks"].append(track)
        documents[name] = campaign
    module = copy.deepcopy(documents["pilot.json"])
    module.update({"name": "ground cohesion-only plumbing pilot", "seed": seed + 2})
    module["tracks"] = module["tracks"][:1]
    module["tracks"][0]["train_modules"] = ["cohesion"]
    module["module_recombination_every"] = 0
    module["tracks"][0]["islands"][0]["es"]["seed"] = seed + 30
    documents["pilot-cohesion.json"] = module
    return documents


TRAIN_PS1 = r'''param(
    [ValidateSet('production','pilot','cohesion')][string]$Profile = 'pilot',
    [ValidateRange(1,256)][int]$Threads = 32,
    [switch]$Resume,
    [switch]$Stop,
    [double]$MaxSeconds = 0
)
$ErrorActionPreference = 'Stop'
function Get-TaskSha256([string]$Path) {
    $taskStream = [IO.File]::OpenRead($Path)
    $taskHasher = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($taskHasher.ComputeHash($taskStream))).Replace('-', '').ToLowerInvariant() }
    finally { $taskHasher.Dispose(); $taskStream.Dispose() }
}
$taskRoot = $PSScriptRoot
$receipt = Get-Content -LiteralPath (Join-Path $taskRoot 'receipt.json') -Raw | ConvertFrom-Json
foreach ($entry in $receipt.files) {
    $taskFile = Join-Path $taskRoot $entry.path
    if ((Get-TaskSha256 $taskFile) -ne $entry.sha256) {
        throw "Package identity mismatch: $($entry.path). Prepare a new package for changed inputs."
    }
}
$configs = @{production='campaign.json'; pilot='pilot.json'; cohesion='pilot-cohesion.json'}
$run = Join-Path $taskRoot "runs/$Profile"
if ($Stop) { New-Item -ItemType Directory -Path $run -Force | Out-Null; New-Item -ItemType File -Path (Join-Path $run 'STOP') -Force | Out-Null; return }
if ((Test-Path -LiteralPath (Join-Path $run 'state.json')) -and -not $Resume) { throw 'Checkpoint exists. Use -Resume to continue with these exact inputs.' }
if ($Resume -and (Test-Path -LiteralPath (Join-Path $run 'STOP'))) { Remove-Item -LiteralPath (Join-Path $run 'STOP') }
$nativeArgs = @('--train-campaign', (Join-Path $taskRoot $configs[$Profile]), '--train-out', $run, '--threads', "$Threads")
if ($Resume) { $nativeArgs += '--resume' }
if ($MaxSeconds -gt 0) { $nativeArgs += @('--max-seconds', "$MaxSeconds") }
& (Join-Path $taskRoot $receipt.binary) @nativeArgs
if ($LASTEXITCODE -ne 0) { throw "Native training exited with code $LASTEXITCODE" }
'''


def prepare(out, simulator, bar_root, *, candidate=CANDIDATE, reference=REFERENCE, air_brain=None,
            checkpoint=STATE, seed=2026101829, elite_brains=()):
    out = Path(out).absolute()
    refuse_reparse(out)
    if out.exists():
        raise ValueError(f"output already exists: {out}")
    simulator, candidate, reference, bar_root = map(lambda p: Path(p).resolve(), (simulator,candidate,reference,bar_root))
    for path in (simulator,candidate,reference):
        refuse_reparse(path)
        if not path.is_file():
            raise ValueError(f"missing source input: {path}")
    for path in (candidate,reference):
        if json.loads(path.read_text(encoding="utf-8"))["kind"] != "tactical_v4":
            raise ValueError(f"ground anchor must have tactical_v4 contract: {path}")
    if type(seed) is not int or not 0 <= seed < (1 << 64) - 2048:
        raise ValueError("seed must leave room for independent bounded u64 streams")
    source_inputs = [{"role": role,"source": str(path),"sha256": digest(path)}
                     for role,path in [("ground_anchor",candidate),("ground_gen70_reference",reference),("executable",simulator)]]
    for script in (Path(__file__), Path(exporter.__file__), ROOT / "scripts/prepare_from_replays.py", ROOT / "scripts/make_catalog.py", ROOT / "scripts/bar_lua.py"):
        source_inputs.append({"role":"preparation_source","source":str(script.resolve()),"sha256":digest(script)})
    if checkpoint is not None:
        checkpoint = Path(checkpoint).resolve()
        state = json.loads(checkpoint.read_text(encoding="utf-8"))
        track = next(t for t in state["tracks"] if t["name"] == "tactical")
        if track["best"] != json.loads(reference.read_text(encoding="utf-8")):
            raise ValueError("reference does not match the preserved checkpoint champion")
        source_inputs.append({"role":"completed_checkpoint","source":str(checkpoint),"sha256":digest(checkpoint),
                              "completed_generations":state["completed_generations"],"best_generation":track["best_generation"]})
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=f".{out.name}.prepare-", dir=out.parent) as td:
        staged = Path(td) / "package"; staged.mkdir()
        manifest = exporter.export(staged / "catalog", bar_root, [*exporter.DEFAULT_UNITS, "armsolar"])
        if manifest["rejected"]:
            raise ValueError(f"catalog source rejections: {manifest['rejected']}")
        brains = staged / "brains"; brains.mkdir()
        shutil.copyfile(candidate, brains / "ground.initial.json")
        shutil.copyfile(reference, brains / "ground.reference.json")
        binary = Path("bin") / simulator.name
        (staged / binary).parent.mkdir()
        shutil.copy2(simulator, staged / binary)
        if air_brain is None:
            subprocess.run([str(staged / binary), "--seed-brain", "tactical-air-v1", "--brain-out", str(brains / "air.initial.json")], check=True, capture_output=True, text=True)
            air_origin = "fresh native tactical-air-v1 template; no inherited ground weights or optimizer state"
        else:
            air_brain = Path(air_brain).resolve()
            if json.loads(air_brain.read_text(encoding="utf-8"))["kind"] != "tactical-air-v1":
                raise ValueError("air anchor has wrong brain contract")
            shutil.copyfile(air_brain, brains / "air.initial.json")
            source_inputs.append({"role":"air_anchor","source":str(air_brain),"sha256":digest(air_brain)})
            air_origin = "preserved explicit air checkpoint weights; optimizer initialized independently"
        shutil.copyfile(brains / "air.initial.json", brains / "air.reference.json")
        for domain in ("ground", "air"):
            shutil.copyfile(brains / f"{domain}.initial.json", brains / f"{domain}.frozen.json")
        elite_paths = {"ground": [], "air": []}
        elite_hashes = set()
        for source in elite_brains:
            source = Path(source).resolve()
            refuse_reparse(source)
            elite = json.loads(source.read_text(encoding="utf-8"))
            kind = elite.get("kind")
            if kind not in ("tactical_v4", "tactical-air-v1"):
                raise ValueError(f"elite brain has unsupported contract: {source}")
            domain = "air" if kind == "tactical-air-v1" else "ground"
            identity = digest(source)
            if identity in elite_hashes:
                continue
            if len(elite_paths[domain]) >= 8:
                raise ValueError(f"at most eight imported elites per domain: {domain}")
            elite_hashes.add(identity)
            target = f"brains/elites/{domain}-{len(elite_paths[domain])+1}.json"
            (staged / target).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, staged / target)
            elite_paths[domain].append(target)
            source_inputs.append({"role":f"{domain}_elite", "source":str(source), "sha256":identity})
        source_receipt = candidate.with_name("candidate-receipt.json")
        if source_receipt.exists():
            if json.loads(source_receipt.read_text(encoding="utf-8"))["candidate_sha256"] != digest(candidate):
                raise ValueError("diagnostic receipt hash does not match ground anchor")
            shutil.copyfile(source_receipt, brains / "ground-diagnostic-receipt.json")
            source_inputs.append({"role":"diagnostic_evaluation_receipt","source":str(source_receipt),"sha256":digest(source_receipt)})
        matchup = json.loads((staged / "catalog/matchups.json").read_text(encoding="utf-8"))
        for path, document in campaign_documents(matchup, seed, elite_paths).items():
            write(staged / path, document)
        (staged / "Train.ps1").write_text(TRAIN_PS1, encoding="utf-8")
        (staged / "README.md").write_text(PACKAGE_README, encoding="utf-8")
        files = [{"path":p.relative_to(staged).as_posix(),"sha256":digest(p),"bytes":p.stat().st_size}
                 for p in sorted(staged.rglob("*")) if p.is_file()]
        receipt = {"format":"minibar.diverse_successor","version":1,"binary":binary.as_posix(),
                   "seed":seed,"sources":source_inputs,"files":files,"air_initialization":air_origin,
                   "ground_initialization":("confirmed cohesion diagnostic candidate; not a historical campaign promotion"
                       if candidate == CANDIDATE.resolve() else "copied explicit ground checkpoint weights; see ground_anchor source hash"),
                   "optimizer_initialization":"fresh independent streams and moments; no optimizer merge or exact continuation",
                   "opponent_policy":"fixed charge/kite/hold/surround scripts plus copied domain-specific learned brain; archive disabled",
                   "companion_policy":"opposite-domain initial brain frozen across comparisons; no live companion updates",
                   "elite_policy":"bounded eight-brain pool; one fresh challenge per track every five production generations; imported scores never reused",
                   "sensor_support_policy": "seeded thirds: neither, one or both sides; radar/scout/both providers; independent one-third jammer chance per side; all support costs scored; support survival excluded from main-force elimination",
                   "starting_distance_policy":"unit-derived LOS/weapon bounds with air acceleration and turn runway; explicit approach-time allowance",
                   "holdout_status":"production-only plan prepared, not evaluated",
                   "qualification_status":"prepared inputs; no training launched or improvement claimed"}
        write(staged / "receipt.json",receipt)
        staged.rename(out)
    return receipt


PACKAGE_README = """# Diverse tactical successor

This is a new independent experiment. Ground starts from the supplied checkpoint
(the confirmed cohesion replacement candidate by default); the preserved
generation-70 champion is the reference. Exact sources are listed in receipt.json.
The air brain has its own explicit initialization and weights. Opposite-domain
companions remain frozen at package initialization. No optimizer state is inherited.

Run a short plumbing pilot in a visible PowerShell console:

```powershell
./Train.ps1 -Profile pilot -Threads 32
./Train.ps1 -Profile cohesion -Threads 32
```

These 3-generation pilots use 30 seconds of combat plus approach allowance to test the workflow. They cannot establish
learning improvement. They contain no holdout plan. After reviewing pilot evidence,
the production profile is available with `./Train.ps1 -Profile production -Threads 32`.
Production is 200 generations with a 120-second combat budget plus approach
allowance (resolved fights end early), a protected 40-Pawns-versus-Mammoth family alongside the reverse, and
charge/kite/hold/surround scripts plus a frozen learned opponent per domain.
The frozen opponents are copies of the initial brains and never follow promotions.
Both training and validation use this roster.

Starting front gaps vary from the largest participating LOS to 1.5 times the
longest offensive weapon range. If LOS exceeds that far end, the interval
collapses at LOS. Aircraft receive a conservative runway floor of
4 * (cruise acceleration distance + turn diameter); their far end is also at
least 1.5 times this floor. Travel allowance budgets for an opponent holding
position. The configured 30/120 seconds remains available as combat time after
that estimated approach. Preview records include all bounds and capped or
unreachable-contact flags. Fixed historical configurations remain unchanged.

Up to eight promising elites are retained per track. Every five production
generations (every pilot generation), one gets a fresh challenge through the
same screening, paired tests and promotion checks. Standalone imported elite
brains are frozen in this package and their old scores are not reused. Full
candidate receipts remain under runs/PROFILE/candidates. The dashboard shows
pool identities, provenance, recent retest gains and rejection reasons.

Reconnaissance support cycles through neither side, one side and both sides
in equal thirds, with seed-dependent rotation. On covered sides, radar towers,
radar-equipped scout aircraft or both are selected. Jammers vary independently
(one-third chance per side). Assets use source-backed sensor ranges, can be
shot down/destroyed and never grant omniscient vision. Scout aircraft fly a
fixed area sweep; combat aircraft keep their separate learned air controller.
Support investment is separate from the near-equal main combat armies, while
all costs and damage count in scoring. Surviving support cannot prevent a win
after the main force is eliminated. Actual coverage may disappear as assets die
or leave range, and jamming does not block visual scouting.

Production includes a separate untouched holdout plan;
running production evaluates its holdout according to the native campaign lifecycle.

Stop cooperatively with `./Train.ps1 -Profile pilot -Stop`; active work finishes
and caches. Resume using `./Train.ps1 -Profile pilot -Resume -Threads 32`.
`-MaxSeconds 60` sets a cooperative time budget, not a hard process kill.
Each profile has its own `runs/PROFILE` state. Keep package inputs unchanged.
The launcher verifies all prepared file hashes before running or resuming.

Preview exactly the native seeded scenarios without training:

```powershell
./bin/visigoth-simulator.exe --catalog ./catalog/catalog.json --preview-curriculum ./stages/production/ground-heavy_swarm.json --seed 17
```

`catalog/manifest.json` records source paths, hashes and bounded mechanics.
The extra Solar Collector is a non-combat objective asset; combat turret investment
is packed in the combat budget. One million metal and energy per side is finite
ammunition supply; no resupply is modeled. See `receipt.json` for executable,
source-brain, checkpoint and package identities. Source/catalog acceptance and
plumbing pilots do not establish native BAR equivalence or champion improvement.
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out",type=Path,default=ROOT / "data/branches/recon-training-20261019")
    parser.add_argument("--sim",type=Path,required=True)
    parser.add_argument("--bar-root",type=Path,default=Path(exporter.make_catalog.BAR))
    parser.add_argument("--candidate",type=Path,default=CANDIDATE)
    parser.add_argument("--reference",type=Path,default=REFERENCE)
    parser.add_argument("--checkpoint",type=Path,default=STATE)
    parser.add_argument("--air-brain",type=Path)
    parser.add_argument("--elite-brain",type=Path,action="append",default=[],help="standalone saved elite brain; repeatable, up to eight per domain")
    parser.add_argument("--seed",type=int,default=2026101829)
    args=parser.parse_args()
    receipt=prepare(args.out,args.sim,args.bar_root,candidate=args.candidate,reference=args.reference,
                    checkpoint=args.checkpoint,air_brain=args.air_brain,seed=args.seed,elite_brains=args.elite_brain)
    print(json.dumps({"out":str(args.out),"files":len(receipt["files"]),"status":receipt["qualification_status"]}))


if __name__ == "__main__":
    main()
