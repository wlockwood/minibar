"""Seed, train and evaluate the replay-informed MiniBAR baseline package.

Run from any directory. --setup-only writes templates and curricula without
training. Existing training output is refused unless --run-name selects a fresh
directory; baseline/config files are deterministic authored starting artifacts.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time


ROOT=Path(__file__).resolve().parents[1]


def write_json(path,value):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_text(json.dumps(value,indent=2,allow_nan=False)+"\n",encoding="utf-8")


def configurations(manifest,generations):
    factions=manifest["factions"]
    def roster(faction):
        return [{"unit":faction["combat"][0],"count":4},
                {"unit":faction["combat"][1],"count":2}]
    scenarios=[]
    for i,faction in enumerate(factions):
        enemy=factions[(i+1)%len(factions)]
        base={"name":faction["name"]+" skirmish","learner":roster(faction),
              "enemy":roster(enemy),"separation":350}
        scenarios.append(base)
        for kind,target in [("attack",enemy["factory"]),("defend",faction["factory"])]:
            s=copy.deepcopy(base);s["name"]=faction["name"]+" "+kind
            s["objective"]={"kind":kind,"target":target};scenarios.append(s)
    def arena(m):
        return {"map":"../"+m["cache"],"movement_class":m["movement_class"],
                "cell_size":m["cell_size"],"center":m["center"]}
    tactical={"name":"from_replays_tactical","brain_kind":"tactical_v4","hidden":8,
              "init_from":"baseline.json","generations":generations,
              "es":{"population":8,"sigma":0.08,"learning_rate":0.02,"seed":20261016},
              "seeds_per_battle":1,"validation_seeds":2,"validation_every":1,
              "opponents":["charge","kite","strike","guard"],"max_time":40,
              "layout_jitter":8,"scenarios":scenarios,"arena":arena(manifest["maps"][0])}
    strategic={"name":"from_replays_strategic","hidden":8,"init_from":"baseline.json",
               "generations":generations,"es":{"population":6,"sigma":0.08,"learning_rate":0.02,"seed":20261016},
               "maps":[{"map":"../"+m["cache"],"movement_class":m["movement_class"],
                        "cell_size":m["cell_size"],"starts":m["starts"]}
                       for m in manifest["maps"] if not m["name"].startswith("Synthetic") or "2048" in m["name"]],
               "factions":[{k:f[k] for k in ("name","builder","mex","power","factory","combat")} for f in factions],
               "max_ticks":5400,"cadence_ticks":30,"train_seeds":1,"validation_seeds":1,
               "starting_metal":1000,"starting_energy":1000,"base_metal_income":2,
               "base_energy_income":25,"combat":True}
    scale=copy.deepcopy(tactical)
    scale.update(name="from_replays_scale_128",init_from="baseline.json",opponents=["charge"],
                 max_time=15,layout_jitter=0,validation_seeds=1)
    scale["scenarios"]=[{"name":f["name"]+" 64v64","learner":[{"unit":f["combat"][0],"count":64}],
                         "enemy":[{"unit":factions[(i+1)%len(factions)]["combat"][0],"count":64}],
                         "separation":300} for i,f in enumerate(factions)]
    return tactical,strategic,scale


def run_command(args,stdout_file,stderr_file,receipts):
    started=time.perf_counter()
    with stdout_file.open("w",encoding="utf-8") as out,stderr_file.open("w",encoding="utf-8") as err:
        result=subprocess.run([str(a) for a in args],cwd=ROOT,stdout=out,stderr=err,check=False)
    receipt={"command":[str(a) for a in args],"seconds":time.perf_counter()-started,
             "exit_code":result.returncode,"stdout":str(stdout_file),"stderr":str(stderr_file)}
    receipts.append(receipt)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {args}\n{stderr_file.read_text(encoding='utf-8')[-6000:]}")
    return receipt


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out",type=Path,default=ROOT/"data/brains/from_replays")
    parser.add_argument("--sim",type=Path,default=ROOT/"target/release/visigoth-simulator.exe")
    parser.add_argument("--catalog",type=Path,help="Optional expanded prepared catalog; copied with explicit coarse-map leveling approximation")
    parser.add_argument("--generations",type=int,default=3)
    parser.add_argument("--run-name",default="trial")
    parser.add_argument("--setup-only",action="store_true")
    parser.add_argument("--threads",type=int,default=4)
    args=parser.parse_args()
    if args.generations<0 or args.threads<1:
        parser.error("generations must be nonnegative and threads positive")
    if Path(args.run_name).name!=args.run_name or args.run_name in (".",".."):
        parser.error("run-name must be a single directory name")
    out=args.out.resolve();sim=args.sim.resolve()
    if not args.setup_only:
        for folder in ("tactical","strategic","runs"):
            if (out/folder/args.run_name).exists():
                raise RuntimeError(f"{out/folder/args.run_name} exists; use a fresh --run-name")
    os.environ["RAYON_NUM_THREADS"]=str(args.threads)
    manifest=json.loads((out/"manifest.json").read_text(encoding="utf-8"))
    catalog=out/"catalog.json"
    if args.catalog:
        source=args.catalog.resolve()
        document=json.loads(source.read_text(encoding="utf-8"))
        adjusted=[]
        for name,definition in document["unit_catalog"].items():
            if definition.get("level_ground"):
                definition["level_ground"]=False;adjusted.append(name)
        catalog=out/"catalog.expanded.json"
        write_json(catalog,document)
        write_json(out/"catalog.expanded.provenance.json",{
            "source":str(source),"source_sha256":hashlib.sha256(source.read_bytes()).hexdigest(),
            "units":len(document["unit_catalog"]),"level_ground_adjusted":adjusted,
            "approximation":"Terrain leveling is not simulated; retain slope/water constraints, permit buildable coarse slopes rather than require mathematically flat interpolation."})
    receipts=[]
    for kind,folder in [("tactical_v4","tactical"),("strategic_v1","strategic")]:
        (out/folder).mkdir(parents=True,exist_ok=True)
        run_command([sim,"--seed-brain",kind,"--brain-out",out/folder/"baseline.json","--hidden","8"],
                    out/folder/"seed.json",out/folder/"seed.stderr.txt",receipts)
    tactical,strategic,scale=configurations(manifest,args.generations)
    write_json(out/"tactical/train_config.json",tactical)
    write_json(out/"strategic/train_config.json",strategic)
    write_json(out/"tactical/large_scale_128.json",scale)
    scale512=copy.deepcopy(scale)
    scale512["name"]="from_replays_scale_512"
    synthetic=next(m for m in manifest["maps"] if m["name"].startswith("Synthetic") and "2048" in m["name"])
    scale512["arena"]={"map":"../"+synthetic["cache"],"movement_class":synthetic["movement_class"],
                       "cell_size":synthetic["cell_size"],"center":synthetic["center"]}
    for scenario in scale512["scenarios"]:
        scenario["name"]=scenario["name"].replace("64v64","256v256")
        for side in ("learner","enemy"):
            unit=scenario[side][0]["unit"]
            scenario[side]=[{"unit":unit,"count":64,"platoon":i} for i in range(4)]
    write_json(out/"tactical/large_scale_512.json",scale512)
    for m in manifest["maps"][:2]:
        name=Path(m["cache"]).stem
        mapped=copy.deepcopy(tactical)
        mapped["arena"]={"map":"../"+m["cache"],"movement_class":m["movement_class"],"cell_size":m["cell_size"],"center":m["center"]}
        write_json(out/f"tactical/{name}.json",mapped)
    if args.setup_only:
        write_json(out/"setup_receipts.json",receipts)
        print(f"Wrote baseline brains and configs to {out}")
        return
    run_root=out/"runs"/args.run_name
    run_root.mkdir(parents=True)
    inputs=[sim,catalog,out/"manifest.json",Path(__file__),Path(__file__).with_name("prepare_from_replays.py"),out/"tactical/baseline.json",
            out/"strategic/baseline.json",out/"tactical/train_config.json",
            out/"strategic/train_config.json",out/"tactical/large_scale_128.json",out/"tactical/large_scale_512.json"]
    inputs.extend(out/m["cache"] for m in manifest["maps"])
    shutil.copyfile(catalog,run_root/"input_catalog.json")
    shutil.copyfile(out/"manifest.json",run_root/"assets_manifest.json")
    if args.catalog:
        shutil.copyfile(out/"catalog.expanded.provenance.json",run_root/"catalog.provenance.json")
    write_json(run_root/"input_provenance.json",{
        "catalog":str(catalog),"threads":args.threads,
        "sha256":{str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in inputs}})
    try:
        for folder,flag in [("tactical","--train"),("strategic","--train-strategy")]:
            run_dir=out/folder/args.run_name;run_dir.mkdir()
            print(f"Training {folder}: {args.generations} generations",flush=True)
            run_command([sim,"--catalog",catalog,flag,out/folder/"train_config.json","--train-out",run_dir],
                        run_dir/"stdout.json",run_dir/"stderr.txt",receipts)
            write_json(out/"run_receipts.json",receipts)
        for filename,config in [("scale",out/"tactical/large_scale_128.json"),
                                ("scale512",out/"tactical/large_scale_512.json"),
                                ("zed",out/"tactical/zed-remake.json")]:
            for checkpoint in ["baseline", "best"]:
                brain=out/"tactical/baseline.json" if checkpoint=="baseline" else out/f"tactical/{args.run_name}/brain.best.json"
                run_command([sim,"--catalog",catalog,"--evaluate",brain,"--train-config",config,
                             "--eval-seeds","1","--eval-seed-base","2000000000"],
                            run_root/f"{filename}.{checkpoint}.evaluation.json",run_root/f"{filename}.{checkpoint}.stderr.txt",receipts)
        hashes={str(p.relative_to(out)):hashlib.sha256(p.read_bytes()).hexdigest()
                for p in out.rglob("brain.*.json") if args.run_name in p.parts}
        write_json(out/"checkpoint_hashes.json",hashes)
        write_json(run_root/"checkpoint_hashes.json",hashes)
    finally:
        write_json(out/"run_receipts.json",receipts)
        write_json(run_root/"run_receipts.json",receipts)
    print(f"Training and evaluation saved under {out}")


if __name__=="__main__":
    main()
