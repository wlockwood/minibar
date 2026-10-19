"""Paired native comparisons on explicit independent layout bundles."""
from __future__ import annotations
import argparse
import concurrent.futures
import hashlib
import json
import math
import os
from pathlib import Path
import random
import statistics
import struct
import subprocess
import time
import training_report
import package_training

def encoded(value):
    return json.dumps(value,sort_keys=True,separators=(",",":"),allow_nan=False).encode("utf-8")

def digest(data):
    return hashlib.sha256(data).hexdigest()

def read(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))

def write(path,value):
    path=Path(path);path.parent.mkdir(parents=True,exist_ok=True)
    temporary=path.with_name(path.name+".tmp")
    temporary.write_bytes(json.dumps(value,indent=2,sort_keys=True,allow_nan=False).encode("utf-8"))
    # Windows readers may briefly deny FILE_SHARE_DELETE during atomic replace.
    # Keep the previous complete document visible; never fall back to truncation.
    for attempt in range(51):
        try:
            os.replace(temporary,path)
            return
        except PermissionError:
            if attempt==50:raise
            time.sleep(0.02)

def dependency_hashes(config):
    seen={};active=set()
    def visit(path):
        path=package_training.safe_file(path)
        key=str(path)
        if key in active:raise ValueError("cyclic evaluation dependency")
        if key in seen:return
        active.add(key);payload=path.read_bytes();seen[key]=digest(payload)
        if path.suffix.lower()==".json":
            def walk(value):
                if isinstance(value,dict):
                    for name,item in value.items():
                        if name in package_training.PATH_KEYS and isinstance(item,str) and item:
                            visit(path.parent/item)
                        elif name=="opponents" and isinstance(item,list):
                            for opponent in item:
                                if isinstance(opponent,str) and opponent.startswith("brain:"):visit(path.parent/opponent[6:])
                        else:walk(item)
                elif isinstance(value,list):
                    for item in value:walk(item)
            walk(json.loads(payload))
            if path.name.endswith(".map_source.json"):
                sidecar=path.with_name(path.name[:-len(".map_source.json")]+".acquisition.json")
                if sidecar.exists():visit(sidecar)
        active.remove(key)
    visit(Path(config).resolve());return seen

def summarize_pairs(candidate,reference,bootstrap_seed):
    if not candidate or len(candidate)!=len(reference):raise ValueError("paired samples must be nonempty and equal length")
    gains=[float(a)-float(b) for a,b in zip(candidate,reference)]
    if not all(math.isfinite(v) for v in gains):raise ValueError("paired samples must be finite")
    rng=random.Random(bootstrap_seed)
    boot=sorted(statistics.mean(rng.choices(gains,k=len(gains))) for _ in range(20_000))
    return {"mean_gain":statistics.mean(gains),"wins":sum(v>0 for v in gains),"losses":sum(v<0 for v in gains),"ties":sum(v==0 for v in gains),"pairs":gains,"interval_95":[boot[500],boot[19499]],"bootstrap_seed":bootstrap_seed,"resamples":20_000,"interpretation":"Paired layout-bundle bootstrap, conditional on these models and scenario family; not independent training-run uncertainty."}

def rescore(trial,reward):
    result=json.loads(json.dumps(trial));stats=result.get("reward_stats",{});components=result.get("reward_components")
    if components is None:raise ValueError("reevaluation required: old trial lacks reward components")
    mode=reward.get("production_mode","count");weight=reward.get("production_weight",0.02);reference=reward.get("production_reference_value",100.0);cap=reward.get("production_cap")
    if mode not in ("none","count","value") or not math.isfinite(weight) or weight<0 or not math.isfinite(reference) or reference<=0 or cap is not None and (not math.isfinite(cap) or cap<0):raise ValueError("invalid reward configuration")
    if mode=="none":bonus=0.0
    else:
        key="units_produced" if mode=="count" else "production_value"
        if key not in stats:raise ValueError("reevaluation required: missing production sufficient statistics")
        bonus=weight*stats[key]/(reference if mode=="value" else 1)
    if cap is not None:bonus=min(bonus,cap)
    result["fitness"]=result["fitness"]-components["production"]+bonus
    components["production"]=bonus;components["total"]=result["fitness"]
    result["interpretation"]="Rescoring existing trajectories; not a retrained policy."
    return result

def native(sim,args,threads=2):
    process=subprocess.run([str(sim),*map(str,args)],capture_output=True,text=True,encoding="utf-8",env={**os.environ,"RAYON_NUM_THREADS":str(threads)})
    if process.returncode:raise ValueError(f"native evaluation failed: {process.stderr[-3000:]}")
    return json.loads(process.stdout)

def ensure_kind(path,kind,sim,out):
    path=Path(path).resolve();brain=read(path)
    if brain["kind"]==kind:return path
    if brain["kind"]!="strategic_v1" or kind!="strategic_v2":raise ValueError("comparison brain kind differs")
    target=Path(out)/"upgrades"/(digest(path.read_bytes())+".json")
    if not target.exists():
        target.parent.mkdir(parents=True,exist_ok=True)
        native(sim,["--upgrade-brain",path,"--brain-kind",kind,"--brain-out",target])
    receipt=read(target.with_suffix(".upgrade.json"))
    if receipt["source_sha256"]!=digest(path.read_bytes()) or receipt["output_sha256"]!=digest(target.read_bytes()):raise ValueError("upgraded brain identity differs")
    return target

def diagnostics(reports):
    groups={}
    def add(key,value):groups.setdefault(key,[]).append(float(value))
    for report in reports:
        for stage in report["stages"]:
            name=stage["name"];details=stage["details"];add(f"stage/{name}/fitness",stage["fitness"])
            for faction,score in details.get("facets",{}).items():add(f"stage/{name}/faction/{faction}",score)
            for trial in details.get("trials",[]):
                prefixes=[f"stage/{name}",*(f"stage/{name}/{axis}/{trial[axis]}" for axis in ("faction","map","learner_side"))]
                for prefix in prefixes:
                    add(prefix+"/fitness",trial["fitness"])
                    for component,value in trial.get("reward_components",{}).items():add(prefix+f"/reward/{component}",value)
                    for metric in ("units_produced","army_value","economy_value","projects_completed","actions_rejected","spent_metal","metal_income","energy_income","metal_overflow","energy_overflow","unmet_energy"):
                        if metric in trial:add(prefix+f"/metric/{metric}",trial[metric])
                    for metric,value in trial.get("telemetry",{}).items():
                        if isinstance(value,(float,int)) and not isinstance(value,bool):add(prefix+f"/telemetry/{metric}",value)
                    add(prefix+f'/outcome/{trial.get("telemetry",{}).get("terminal_outcome","unavailable")}',1)
                    add(prefix+"/combat_enabled",int(trial.get("telemetry",{}).get("combat_enabled",False)))
            for row in details.get("rows",[]):
                tally=row["tally"];n=max(1,tally["battles"])
                for axis in ("scenario","opponent"):
                    add(f"stage/{name}/{axis}/{row[axis]}/fitness",tally["fitness_sum"]/n)
                for metric in ("battles","wins","losses","timeouts"):
                    add(f"stage/{name}/outcomes/{metric}",tally[metric])
    return {key:{"mean":statistics.mean(values),"sum":sum(values),"count":len(values)} for key,values in sorted(groups.items())}

def unavailable_dimensions(reports):
    missing=set()
    for report in reports:
        for stage in report["stages"]:
            details=stage["details"]
            if "rows" in details:missing.add("tactical map and learner_side: native rows aggregate both sides and do not identify map")
            if "trials" in details:missing.add("strategic scenario: trials identify map and faction instead")
            if details.get("facets_complete") is not True:missing.add("faction: incomplete scenario metadata")
    return sorted(missing)

def model_identity(path):
    """Brain weights are native f32; JSON formatting/decimal spellings are immaterial."""
    brain=read(path)
    def weights(value):
        return [weights(v) for v in value] if isinstance(value,list) else struct.pack("!f",float(value)).hex()
    for module in brain.get("modules",[]):
        for layer in module.get("layers",[]):
            for field in ("weights","biases"):
                if field in layer:layer[field]=weights(layer[field])
    return digest(encoded(brain))

def freeze_file(path,out):
    """Capture one read; all native work uses this exact immutable model."""
    payload=package_training.safe_file(path).read_bytes()
    target=Path(out)/"inputs"/"brains"/(digest(payload)+".json")
    target.parent.mkdir(parents=True,exist_ok=True)
    if target.exists() and target.read_bytes()!=payload:raise ValueError("frozen brain changed")
    if not target.exists():target.write_bytes(payload)
    return target

def freeze_profile(campaign,sim,out):
    dependencies=dependency_hashes(campaign);exe=digest(sim.read_bytes())
    target=Path(out)/"inputs"/("profile-"+digest(encoded([dependencies,exe])))
    root=package_training.find_root(campaign)
    if not target.exists():package_training.build_package(campaign,sim,target,source_root=root)
    receipt=read(target/"manifest.json")
    for record in receipt["files"]:
        if digest((target/record["path"]).read_bytes())!=record["sha256"]:raise ValueError("frozen profile changed")
        if "source" in record and record["source_sha256"]!=dependencies.get(str((root/record["source"]).resolve())):
            raise ValueError("profile changed while freezing")
    binary=target/receipt["binary"]
    if digest(binary.read_bytes())!=exe:raise ValueError("executable changed while freezing")
    return target/"campaign.json",binary

def pooled_fitness(reports,worst_weights):
    """Pool tactical row tallies before applying the nonlinear worst-quarter term."""
    total=weight=0.0
    for index,stage in enumerate(reports[0]["stages"]):
        details=[r["stages"][index]["details"] for r in reports]
        def score(items):
            if "baseline" in items[0]:return .5*score([d["baseline"] for d in items])+.5*score([d["historical"] for d in items])
            if "rows" in items[0]:
                rows={}
                for d in items:
                    for row in d["rows"]:
                        key=(row["scenario"],row["opponent"]);v=rows.setdefault(key,[0.0,0])
                        v[0]+=row["tally"]["fitness_sum"];v[1]+=row["tally"]["battles"]
                means=sorted(v[0]/v[1] if v[1] else 0.0 for v in rows.values())
                w=worst_weights.get(stage["name"],.3);n=max(1,math.ceil(len(means)/4))
                return (1-w)*statistics.mean(means)+w*statistics.mean(means[:n])
            return statistics.mean(t["fitness"] for d in items for t in d["trials"])
        w=stage["weight"];total+=w*score(details);weight+=w
    return total/weight

def compare(manifest,out):
    manifest=read(manifest) if isinstance(manifest,(str,Path)) else json.loads(json.dumps(manifest))
    out=Path(out).resolve();out.mkdir(parents=True,exist_ok=True)
    sim=Path(manifest["simulator"]).resolve();campaign=Path(manifest["campaign"]).resolve();track=manifest["track"]
    campaign,sim=freeze_profile(campaign,sim,out)
    config=read(campaign);track_cfg=next((t for t in config["tracks"] if t["name"]==track),None)
    if track_cfg is None:raise ValueError("unknown comparison track")
    expected=read((campaign.parent/track_cfg["brain"]).resolve())["kind"]
    layouts=manifest.get("layouts",24);seeds=manifest.get("seeds",1);base=manifest.get("seed_base",12_000_000_000_000)
    if not isinstance(layouts,int) or not 1<=layouts<=1000 or not isinstance(seeds,int) or not 1<=seeds<=1_000_000 or base<0 or base+layouts*100_000_000+64_000_000>=2**64:raise ValueError("invalid comparison sample band")
    brains=manifest["brains"];labels=[b["label"] for b in brains]
    if len(set(labels))!=len(labels) or manifest["reference"] not in labels:raise ValueError("duplicate labels or missing reference")
    for b in brains:b["path"]=str(ensure_kind(freeze_file(b["path"],out),expected,sim,out))
    opponents=manifest.get("opponents",[{"label":"configured"}])
    if not opponents or len({o["label"] for o in opponents})!=len(opponents):raise ValueError("invalid opponent roster")
    for o in opponents:
        if o.get("brain"):o["brain"]=str(ensure_kind(freeze_file(o["brain"],out),expected,sim,out))
    deps=dependency_hashes(campaign);exe_hash=digest(sim.read_bytes())
    identity={"protocol":1,"executable_sha256":exe_hash,"dependencies":deps,"campaign":str(campaign),"track":track,"layouts":layouts,"seeds":seeds,"seed_base":base,"opponents":[{**o,"sha256":digest(Path(o["brain"]).read_bytes()) if o.get("brain") else None} for o in opponents]}
    model_hashes={str(Path(p).resolve()):digest(Path(p).read_bytes()) for p in [*(b["path"] for b in brains),*(o["brain"] for o in opponents if o.get("brain"))]}
    def verify_models():
        if digest(sim.read_bytes())!=exe_hash:raise ValueError("frozen executable changed during evaluation")
        if any(digest(Path(p).read_bytes())!=h for p,h in model_hashes.items()):raise ValueError("frozen model changed during evaluation")
    worst_weights={s["name"]:read(campaign.parent/s["task"]["config"]).get("objective",{}).get("worst_case_weight",.3) for s in track_cfg["validation"]["stages"]}
    write(out/"manifest.json",manifest)
    tasks=[(b,o,l) for b in brains for o in opponents for l in range(layouts)]
    started=time.monotonic();completed=0
    def evaluate(task):
        b,o,l=task;brain_hash=model_identity(b["path"]);sample=base+l*100_000_000
        key=digest(encoded({"comparison":identity,"brain":brain_hash,"opponent":o,"layout":l}))
        path=out/"cache"/(key+".json")
        if path.exists():
            cached=read(path)
            if cached.get("identity")!=key or cached.get("report_sha256")!=digest(encoded(cached.get("report"))):raise ValueError("cached evaluation identity/content differs")
            report=cached["report"]
        else:
            args=["--evaluate-campaign",b["path"],"--campaign-config",campaign,"--campaign-track",track,"--campaign-plan","validation","--eval-layouts","1","--eval-seeds",seeds,"--eval-seed-base",sample]
            if o.get("brain"):args += ["--against-brain",o["brain"]]
            verify_models()
            if dependency_hashes(campaign)!=deps:raise ValueError("frozen profile changed")
            report=native(sim,args,manifest.get("threads",2))
            verify_models()
            if dependency_hashes(campaign)!=deps:raise ValueError("frozen profile changed")
            if not math.isfinite(report["fitness"]):raise ValueError("nonfinite native score")
            write(path,{"identity":key,"report_sha256":digest(encoded(report)),"report":report})
        return b["label"],o["label"],l,report
    results=[]
    # Identical labels/weights may share work, but never duplicate concurrent writes.
    unique={}
    aliases={}
    for task in tasks:
        b,o,l=task;k=(model_identity(b["path"]),o["label"],l)
        if k in unique:aliases.setdefault(k,[]).append(b["label"])
        else:unique[k]=task
    with concurrent.futures.ThreadPoolExecutor(max_workers=manifest.get("workers",2)) as pool:
        pending={pool.submit(evaluate,t):k for k,t in unique.items()}
        try:
            while pending:
                done,_=concurrent.futures.wait(pending,timeout=30,return_when=concurrent.futures.FIRST_COMPLETED)
                for future in done:
                    k=pending.pop(future);result=future.result();results.append(result)
                    for alias in aliases.get(k,[]):results.append((alias,*result[1:]))
                    completed+=1
                progress={"status":"comparing","track":track,"completed":completed,"total":len(unique),"elapsed_seconds":round(time.monotonic()-started,2)}
                write(out/"progress.json",progress)
                if not done or completed==len(unique) or completed%max(1,len(unique)//10)==0:print(f'Comparison {track}: {completed}/{len(unique)} layout/opponent evaluations, {progress["elapsed_seconds"]:.1f}s',flush=True)
        except BaseException as error:
            active={future for future in pending if not future.cancel()}
            print(f"Comparison {track} failed; cancelling queued work and draining {len(active)} active evaluations: {error}",flush=True)
            failure={"status":"failed","error":str(error),"active_evaluations":len(active),"completed":completed,"total":len(unique)}
            try:write(out/"progress.json",failure)
            except OSError:pass
            while active:
                _,active=concurrent.futures.wait(active,timeout=30,return_when=concurrent.futures.FIRST_COMPLETED)
                failure["active_evaluations"]=len(active)
                try:write(out/"progress.json",failure)
                except OSError:pass
                print(f"Comparison {track}: failed, {len(active)} evaluations still draining",flush=True)
            raise
    verify_models()
    if dependency_hashes(campaign)!=deps or digest(sim.read_bytes())!=exe_hash:raise ValueError("comparison inputs changed during evaluation")
    scores={};reports={}
    for label in labels:
        rows=[r for r in results if r[0]==label];reports[label]=[r[3] for r in sorted(rows,key=lambda r:(r[2],r[1]))]
        scores[label]=[statistics.mean(r[3]["fitness"] for r in rows if r[2]==l) for l in range(layouts)]
    result={"format":"minibar.training_comparison","version":1,"track":track,"reference":manifest["reference"],"layouts":layouts,"identity":identity,"analysis_sources":{p.name:digest(p.read_bytes()) for p in [Path(__file__),Path(training_report.__file__),Path(package_training.__file__)]},"elapsed_seconds":time.monotonic()-started,"brains":[{"label":b["label"],"brain_sha256":digest(Path(b["path"]).read_bytes()),"model_identity":model_identity(b["path"]),"fitness":statistics.mean(scores[b["label"]]),"pooled_fitness":statistics.mean(pooled_fitness([r[3] for r in results if r[0]==b["label"] and r[1]==o["label"]],worst_weights) for o in opponents),"paired":summarize_pairs(scores[b["label"]],scores[manifest["reference"]],manifest.get("bootstrap_seed",20261017)),"diagnostics":diagnostics(reports[b["label"]]),"opponent_diagnostics":{o["label"]:diagnostics([r[3] for r in results if r[0]==b["label"] and r[1]==o["label"]]) for o in opponents},"unavailable_dimensions":unavailable_dimensions(reports[b["label"]])} for b in brains]}
    write(out/"assessment.json",result);write(out/"progress.json",{"status":"completed","completed":len(unique),"total":len(unique)})
    (out/"report.html").write_text(training_report.render(result),encoding="utf-8")
    return result

if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("manifest",type=Path);parser.add_argument("--out",required=True,type=Path)
    args=parser.parse_args();compare(args.manifest,args.out)
