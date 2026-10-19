"""Independent, resumable training-quality experiments from one committed snapshot."""
from __future__ import annotations
import argparse
import copy
import concurrent.futures
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import time
import package_training
import training_compare as comparison
import training_report

REPO=Path(__file__).resolve().parents[1]
read=comparison.read
write=comparison.write

def default_manifest():
    return {"format":"minibar.training_experiment","version":1,
            "campaign":str(REPO/"examples/campaign/replay-informed.json"),
            "simulator":str(REPO/"target/training-quality/release/visigoth-simulator.exe"),
            "source_run":str(REPO/"data/brains/campaign"),
            "training_seeds":[2026101701,2026101702,2026101703],
            "pilot_generations":1,"population":2,"threads":2,"workers":2,
            "validation_layouts":4,"validation_seeds":2,"final_layouts":24,
            "comparison_seed_base":12_000_000_000_000,
            "arms":[{"name":"legacy-v1","production_mode":"count","brain_kind":"strategic_v1","sigma_scale":1.0},
                    {"name":"no-count-v1","production_mode":"none","brain_kind":"strategic_v1","sigma_scale":1.0},
                    {"name":"no-count-v2","production_mode":"none","brain_kind":"strategic_v2","sigma_scale":1.0},
                    {"name":"smaller-mutations","production_mode":"none","brain_kind":"strategic_v2","sigma_scale":0.5},
                    {"name":"selected-modules","production_mode":"none","brain_kind":"strategic_v2","sigma_scale":1.0,"modules":{"tactical":["engagement","mission"],"strategic":["production"]}}]}

def snapshot_source(run,out):
    out=Path(out);out.mkdir(parents=True,exist_ok=True)
    # A single read of an atomically replaced file: no lock on the active trainer.
    payload=(Path(run)/"state.json").read_bytes();state=json.loads(payload)
    if state.get("format")!="minibar-campaign-state" or state.get("version")!=1 or not state.get("tracks") or any(t.get("best_report") is None or t.get("initial_report") is None for t in state["tracks"]):raise ValueError("source needs committed validated champions")
    if (out/"state.json").exists():raise ValueError("source snapshot already exists")
    (out/"state.json").write_bytes(payload)
    source={"state_sha256":comparison.digest(payload),"backend_identity":state["backend_identity"],"completed_generations":state["completed_generations"],"tracks":state["tracks"]}
    for track in state["tracks"]:
        if not re.fullmatch(r"[A-Za-z0-9_-]+",track["name"]):raise ValueError("unsafe source track name")
        write(out/(track["name"]+"-champion.json"),track["best"])
        write(out/(track["name"]+"-reference.json"),track.get("reference") or track["base"])
    write(out/"snapshot.json",{k:v for k,v in source.items() if k!="tracks"})
    return source

def _validate_manifest(m):
    if m.get("format")!="minibar.training_experiment" or m.get("version")!=1:raise ValueError("unsupported experiment manifest")
    if not m["training_seeds"] or len(set(m["training_seeds"]))!=len(m["training_seeds"]) or any(type(s)!=int or not 0<=s<2**63 for s in m["training_seeds"]):raise ValueError("independent training seeds required")
    names=[a["name"] for a in m["arms"]]
    if not names or len(set(names))!=len(names) or any(not re.fullmatch(r"[A-Za-z0-9_-]+",n) for n in names):raise ValueError("safe distinct arm names required")
    if not 1<=m["pilot_generations"]<=1_000_000 or m["population"]<2 or m["population"]%2 or not 1<=m["threads"]<=64 or not 1<=m["workers"]<=16:raise ValueError("invalid experiment work budget")
    for a in m["arms"]:
        if a["production_mode"] not in ("none","count","value") or a["brain_kind"] not in ("strategic_v1","strategic_v2") or a["sigma_scale"] not in (0.5,1.0):raise ValueError("invalid arm treatment")

def prepare(manifest,out):
    manifest=read(manifest) if isinstance(manifest,(Path,str)) else copy.deepcopy(manifest)
    _validate_manifest(manifest);out=Path(out).resolve()
    for field in ("campaign","simulator","source_run"):
        manifest[field]=str((REPO/manifest[field]).resolve())
    if out.exists():raise ValueError("experiment output already exists; use run/compare to resume")
    simulator=package_training.safe_file(manifest["simulator"]);campaign=package_training.safe_file(manifest["campaign"])
    original=read(campaign);out.mkdir(parents=True)
    source=snapshot_source(Path(manifest["source_run"]),out/"source")
    by_name={t["name"]:t for t in source["tracks"]}
    if set(by_name)!={t["name"] for t in original["tracks"]}:raise ValueError("source/experiment track names differ")
    package=out/"package"
    package_training.build_package(campaign,simulator,package,source_root=REPO)
    packaged=read(package/"campaign.json")
    def make_config(destination,arm,seed,comparison_only=False):
        cfg=copy.deepcopy(packaged)
        cfg["name"]="quality / "+arm["name"]+" / "+str(seed)
        cfg["seed"]=seed;cfg["generations"]=manifest["pilot_generations"]
        cfg["validation_every"]=1;cfg["frontier_limit"]=8;cfg["max_faction_regression"]=0.05
        # Keep the source's existing promotion/recentring decisions constant in every arm.
        for ti,track in enumerate(cfg["tracks"]):
            name=track["name"];kind=arm["brain_kind"] if name=="strategic" else by_name[name]["best"]["kind"]
            for role,field in [("champion","brain"),("reference","reference_brain")]:
                path=out/"source"/(name+"-"+role+".json")
                path=comparison.ensure_kind(path,kind,simulator,out)
                track[field]=str(path)
            track["train_modules"]=arm.get("modules",{}).get(name,[])
            for ii,island in enumerate(track["islands"]):
                island["es"]["population"]=manifest["population"]
                island["es"]["sigma"]*=arm["sigma_scale"]
                island["es"]["seed"]=(seed+ti*1000+ii*100)%2**64
            for purpose in ("training","validation","holdout"):
                if purpose not in track or track[purpose] is None:continue
                plan=track[purpose];plan["catalog"]=str((package/plan["catalog"]).resolve())
                plan["layouts"]=1 if purpose=="training" else manifest["validation_layouts"]
                plan["seeds"]=1 if purpose=="training" else manifest["validation_seeds"]
                for si,stage in enumerate(plan["stages"]):
                    path=(package/stage["task"]["config"]).resolve();data=read(path)
                    # Resolve all runtime file dependencies relative to the copied stage.
                    def resolve(value):
                        if isinstance(value,dict):
                            for k,v in value.items():
                                if k in package_training.PATH_KEYS and isinstance(v,str) and v:value[k]=str((path.parent/v).resolve())
                                else:resolve(v)
                        elif isinstance(value,list):
                            for v in value:resolve(v)
                    resolve(data)
                    if name=="strategic":
                        data["brain_kind"]=kind;data["reward"]={"production_mode":arm["production_mode"]}
                    else:
                        for scenario in data.get("scenarios",[]):
                            # Fixture source metadata is added to generated configs only.
                            units=scenario.get("learner",[])
                            prefix=units[0]["unit"][:3] if units else ""
                            if prefix not in {"arm","cor","leg"}:raise ValueError("quality scenario needs explicit faction metadata")
                            scenario["faction"]={"arm":"armada","cor":"cortex","leg":"legion"}[prefix]
                    target=destination/"stages"/f"{name}-{purpose}-{si}.json";write(target,data);stage["task"]["config"]=str(target)
            if not comparison_only:track["holdout"]=None # Zed is not inspected or used by the pilot.
        write(destination/"campaign.json",cfg)
    runs=[]
    for arm in manifest["arms"]:
        for seed in manifest["training_seeds"]:
            directory=out/"arms"/arm["name"]/f"seed-{seed}"
            make_config(directory,arm,seed)
            runs.append({"arm":arm["name"],"seed":seed,"directory":str(directory),"status":"pending","completed_generations":0})
    # One common audited profile and opponent schema for every resulting brain.
    audited={"name":"audited-comparison","production_mode":"none","brain_kind":"strategic_v2","sigma_scale":1.0}
    make_config(out/"comparison-profile",audited,manifest["training_seeds"][0],True)
    manifest.update({"source_snapshot":{k:v for k,v in source.items() if k!="tracks"},"simulator":str(simulator),"simulator_sha256":comparison.digest(simulator.read_bytes()),"campaign":str(campaign),"prepared_at":"2026-10-17","comparison_campaign":str(out/"comparison-profile/campaign.json"),"seed_bands":{"training":"campaign generation bands (native)","development":8_000_000_000_000,"final":manifest["comparison_seed_base"]}})
    write(out/"experiment.json",manifest);write(out/"runs.json",runs)
    write(out/"input-hashes.json",{str(d):comparison.dependency_hashes(Path(d)/"campaign.json") for d in [*(r["directory"] for r in runs),str(out/"comparison-profile")]})
    write(out/"progress.json",{"status":"prepared","runs":len(runs)})
    return manifest

def _check_inputs(out,m):
    if comparison.digest(Path(m["simulator"]).read_bytes())!=m["simulator_sha256"]:raise ValueError("experiment executable changed")
    if m.get("evaluation_simulator") and comparison.digest(Path(m["evaluation_simulator"]).read_bytes())!=m["evaluation_simulator_sha256"]:raise ValueError("evaluation executable changed")
    if comparison.digest((out/"source/state.json").read_bytes())!=m["source_snapshot"]["state_sha256"]:raise ValueError("experiment source snapshot changed")
    sealed=read(out/"input-hashes.json")
    if str(out/"comparison-profile") not in sealed:raise ValueError("comparison profile has no sealed input receipt")
    for directory,hashes in sealed.items():
        if comparison.dependency_hashes(Path(directory)/"campaign.json")!=hashes:raise ValueError("experiment inputs changed")

def run(out):
    out=Path(out).resolve();m=read(out/"experiment.json");_check_inputs(out,m);runs=read(out/"runs.json")
    started=time.monotonic()
    def launch(record):
        r=copy.deepcopy(record);directory=Path(r["directory"]);target=directory/"run"
        if (target/"STOP").exists():r.update(status="stopped",error="STOP file present");return r
        args=[m["simulator"],"--train-campaign",str(directory/"campaign.json"),"--train-out",str(target),"--threads",str(m["threads"]),"--generations",str(m["pilot_generations"])]
        if (target/"state.json").exists():args.append("--resume")
        start=time.monotonic()
        with (directory/"stdout.json").open("w",encoding="utf-8") as stdout,(directory/"stderr.log").open("a",encoding="utf-8") as stderr:
            result=subprocess.run(args,stdout=stdout,stderr=stderr)
        summary=read(target/"summary.json") if (target/"summary.json").exists() else {}
        r.update(status=summary.get("status","failed") if result.returncode==0 else "failed",completed_generations=summary.get("completed_generations",0),elapsed_seconds=round(time.monotonic()-start,2),returncode=result.returncode)
        if result.returncode==0:r.pop("error",None)
        if result.returncode:r["error"]=(directory/"stderr.log").read_text(encoding="utf-8")[-2000:]
        if r["status"]=="completed" and r["completed_generations"]!=m["pilot_generations"]:raise ValueError("incomplete generation reported completed")
        r.setdefault("attempts",[]).append({k:v for k,v in r.items() if k!="attempts"})
        print(f'Pilot {r["arm"]}/{r["seed"]}: {r["status"]}, {r["completed_generations"]} generations, {r["elapsed_seconds"]:.1f}s',flush=True)
        return r
    with concurrent.futures.ThreadPoolExecutor(max_workers=m["workers"]) as pool:
        pending={pool.submit(launch,r):i for i,r in enumerate(runs) if r["status"]!="completed"}
        while pending:
            done,_=concurrent.futures.wait(pending,timeout=30,return_when=concurrent.futures.FIRST_COMPLETED)
            for future in done:runs[pending.pop(future)]=future.result()
            finished=[r for r in runs if r["status"]=="completed"]
            seconds=statistics_mean([r.get("elapsed_seconds",0) for r in finished])
            status={"status":"training","completed_runs":len(finished),"total_runs":len(runs),"elapsed_seconds":round(time.monotonic()-started,2),"estimated_remaining_seconds":round(seconds*len(pending)/m["workers"],1) if finished else None}
            write(out/"runs.json",runs);write(out/"progress.json",status)
            print(f'Experiment: {len(finished)}/{len(runs)} pilots committed; elapsed {status["elapsed_seconds"]:.0f}s; estimated remaining {str(status["estimated_remaining_seconds"])+"s" if finished else "pending first completed pilot"}',flush=True)
    _check_inputs(out,m)
    result={"experiment":m,"runs":runs,"interpretation":"One complete full-horizon generation per independent arm/seed; preliminary screen only."}
    write(out/"pilot-results.json",result);(out/"report.html").write_text(training_report.render(result),encoding="utf-8")
    write(out/"progress.json",{"status":"pilots_complete" if all(r["status"]=="completed" for r in runs) else "incomplete","runs":len(runs)})
    return result

def statistics_mean(values):
    return sum(values)/len(values) if values else 0.0

def audit_runs(runs):
    checks=[]
    for r in runs:
        if r["status"]=="completed":
            state=read(Path(r["directory"])/"run/state.json")
            cfg=read(Path(r["directory"])/"campaign.json")
            for track in cfg["tracks"]:
                saved=next(t for t in state["tracks"] if t["name"]==track["name"])
                selected=track.get("train_modules",[])
                frozen=[m["name"] for m in saved["base"]["modules"] if selected and m["name"] not in selected]
                before={m["name"]:m for m in saved["base"]["modules"]}
                outputs=[saved["best"],*(read(Path(r["directory"])/"run/tracks"/track["name"]/island["name"]/"brain.latest.json") for island in saved["islands"])]
                for brain in outputs:
                    after={m["name"]:m for m in brain["modules"]}
                    if any(before[name]!=after[name] for name in frozen):raise ValueError("pilot changed a frozen module")
                checks.append({"arm":r["arm"],"seed":r["seed"],"track":track["name"],"frozen_modules_verified":frozen,"verified_outputs":1+len(saved["islands"]),"committed_generations":state["completed_generations"]})
            if r.get("error")=="STOP file present":r.pop("error")
        r["total_elapsed_seconds"]=round(sum(a.get("attempt_elapsed_seconds",a.get("elapsed_seconds",0)) for a in r.get("attempts",[])),2)
    return checks

def run_summary(results,runs):
    """Training seeds, rather than layout bundles, are the replication units here."""
    summaries=[]
    for result in results:
        scores={b["label"]:b["fitness"] for b in result["brains"]}
        control={r["seed"]:scores[r["arm"]+"/"+str(r["seed"])] for r in runs if r["arm"]=="legacy-v1" and r["status"]=="completed"}
        for arm in sorted({r["arm"] for r in runs}):
            members=[r for r in runs if r["arm"]==arm]
            values=[scores[arm+"/"+str(r["seed"])] for r in members if r["status"]=="completed"]
            paired=[(scores[arm+"/"+str(r["seed"])],control[r["seed"]]) for r in members if r["status"]=="completed" and r["seed"] in control]
            row={"track":result["track"],"arm":arm,"completed_seeds":len(values),"planned_seeds":len(members),"mean":statistics.mean(values) if values else None,"sample_sd":statistics.stdev(values) if len(values)>1 else None,"range":[min(values),max(values)] if values else [],"missing_seeds":[r["seed"] for r in members if r["status"]!="completed"]}
            if paired:
                row["versus_legacy"]=comparison.summarize_pairs([p[0] for p in paired],[p[1] for p in paired],20261017)
                row["versus_legacy"]["interpretation"]="Exploratory paired training-seed bootstrap; only three seeds, insufficient for a firm treatment ranking. Shared evaluation layouts are held fixed."
            summaries.append(row)
    return summaries

def compare(out):
    out=Path(out).resolve();m=read(out/"experiment.json");_check_inputs(out,m);runs=read(out/"runs.json")
    run_integrity=audit_runs(runs)
    write(out/"runs.json",runs)
    config=read(m["comparison_campaign"]);results=[]
    for track in config["tracks"]:
        name=track["name"]
        brains=[{"label":"original-reference","path":str(out/"source"/(name+"-reference.json"))},{"label":"captured-champion","path":str(out/"source"/(name+"-champion.json"))}]
        for r in runs:
            if r["status"]=="completed":brains.append({"label":r["arm"]+"/"+str(r["seed"]),"path":str(Path(r["directory"])/"run/tracks"/name/"brain.best.json")})
        manifest={"simulator":m.get("evaluation_simulator",m["simulator"]),"campaign":m["comparison_campaign"],"track":name,"reference":"captured-champion","brains":brains,"layouts":m["final_layouts"],"seeds":1,"seed_base":m["comparison_seed_base"],"threads":m["threads"],"workers":m["workers"],"opponents":[{"label":"configured"},{"label":"captured-champion","brain":str(out/"source"/(name+"-champion.json"))}]}
        results.append(comparison.compare(manifest,out/"comparisons"/name))
    result={"experiment":m,"runs":runs,"comparisons":results,"analysis_sha256":comparison.digest(Path(__file__).read_bytes()),"training_seed_summary":run_summary(results,runs),"run_integrity":run_integrity,"pilot_matrix_complete":all(r["status"]=="completed" for r in runs),"interpretation":"All scores are common-profile audited reward. Individual battles are correlated within layouts; three training seeds per arm are preliminary. No winner deployed."}
    write(out/"assessment.json",result);(out/"report.html").write_text(training_report.render(result),encoding="utf-8")
    write(out/"progress.json",{"status":"completed" if result["pilot_matrix_complete"] else "comparison_complete_with_incomplete_pilots","runs":len(runs)})
    return result

if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__);sub=parser.add_subparsers(dest="command",required=True)
    init=sub.add_parser("prepare");init.add_argument("manifest",type=Path);init.add_argument("--out",type=Path,required=True)
    for name in ("run","compare"):
        p=sub.add_parser(name);p.add_argument("out",type=Path)
    args=parser.parse_args()
    if args.command=="prepare":prepare(args.manifest,args.out)
    elif args.command=="run":run(args.out)
    else:compare(args.out)
