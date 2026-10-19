"""Locally reassess returned champions with the existing native evaluation CLI."""
from __future__ import annotations

import math
from pathlib import Path
import subprocess

import branch_transfer as transfer


def aggregate(stages):
    if not stages or any(not math.isfinite(s["fitness"]) or not math.isfinite(s["weight"]) or s["weight"] <= 0 for s in stages):
        raise ValueError("evaluation needs finite scores and positive stage weights")
    weight = max(s["weight"] for s in stages); scale = max(abs(s["fitness"]) for s in stages)
    denominator = sum(s["weight"] / weight for s in stages)
    value = sum((s["weight"] / weight) / denominator * (s["fitness"] / scale) for s in stages) if scale else 0
    return {"fitness": max(-1, min(1, value)) * scale, "stages": stages}


def assess(candidate, incumbent, min_gain, slack, min_t=0.0):
    if not math.isfinite(min_t) or min_t < 0: raise ValueError("invalid paired t threshold")
    if not candidate or len(candidate) != len(incumbent) or not math.isfinite(min_gain) or not math.isfinite(slack) or min_gain < 0 or slack < 0:
        raise ValueError("invalid paired comparison dimensions or guards")
    reference = candidate[0]["stages"]
    for plan in candidate + incumbent:
        if not math.isfinite(plan["fitness"]) or len(plan["stages"]) != len(reference): raise ValueError("invalid comparison score")
        for a, b in zip(plan["stages"], reference):
            if any(a[k] != b[k] for k in ("name", "difficulty", "weight")): raise ValueError("comparison metadata differ")
        if not math.isclose(aggregate(plan["stages"])["fitness"], plan["fitness"], rel_tol=1e-10, abs_tol=1e-10):
            raise ValueError("inconsistent comparison aggregate")
    gains = [a["fitness"] - b["fitness"] for a, b in zip(candidate, incumbent)]
    if not all(math.isfinite(g) for g in gains): raise ValueError("nonfinite paired gain")
    mean = sum(g / len(gains) for g in gains); wins = sum(g > 0 for g in gains); losses = sum(g < 0 for g in gains)
    protected = all(sum(p["stages"][i]["fitness"] / len(candidate) for p in candidate) >=
                    sum(p["stages"][i]["fitness"] / len(incumbent) for p in incumbent) - slack for i in range(len(reference)))
    error = math.sqrt(sum((g-mean)**2 for g in gains)/(len(gains)-1)/len(gains)) if len(gains)>1 else 0
    t = (max(-1e6, min(1e6, mean/error)) if error>1e-12 else (1e6 if mean>0 else 0)) if len(gains)>1 else 0
    return {"accepted": mean > min_gain and wins > len(gains) // 2 and protected and (min_t == 0 or t >= min_t), "mean_gain": mean,
            "t_statistic": t,
            "wins": wins, "losses": losses, "ties": len(gains) - wins - losses, "seeds": len(gains),
            "candidate": candidate, "incumbent": incumbent}


def load_registry(root):
    root = Path(root)
    payload = transfer.safe_file(root / "current.json").read_bytes()
    document = transfer.parse_json(payload)
    if document.get("format") != "minibar.best_brains" or document.get("version") != 1:
        raise ValueError("unsupported best-brain registry")
    tracks = []
    for item in document["tracks"]:
        values = {}
        for field in ("brain", "initial_brain"):
            transfer.relative_name(item[field]); path = transfer.safe_file(root / item[field], root.resolve())
            brain = transfer.read_json(path)
            if transfer.digest(transfer.encoded(brain)) != item[field + "_sha256"]: raise ValueError("best-brain registry hash mismatch")
            values[field] = brain
        tracks.append({"name": item["name"], "best": values["brain"], "base": values["initial_brain"]})
    return {"kind": "best_registry", "state_sha256": transfer.digest(payload),
            "backend_identity": document["backend_identity"], "tracks": tracks}


def evaluate(package, track, brain, seeds, seed_base, directory, label):
    directory = Path(directory); directory.mkdir(parents=True, exist_ok=True)
    brain_path = directory / f"{label}.brain.json"; transfer.write_json(brain_path, brain)
    manifest = transfer.read_json(package / "manifest.json"); plan = track["validation"]
    if plan.get("layouts", 1) > 1 or track.get("reference_brain") or track.get("_native_evaluation"):
        cmd=[str(package / manifest["binary"]), "--evaluate-campaign", str(brain_path.resolve()),
             "--campaign-config", str(package / "campaign.json"), "--campaign-track", track["name"],
             "--campaign-plan", "validation", "--eval-layouts", str(plan.get("layouts",1)),
             "--eval-seeds", str(seeds), "--eval-seed-base", str(seed_base)]
        result=subprocess.run(cmd,capture_output=True,text=True)
        if result.returncode: raise ValueError(f"local comparison failed: {result.stderr[-2000:]}")
        import json
        report=json.loads(result.stdout)
        transfer.write_json(directory / f"{label}.report.json",report)
        return report

    stages = []
    for index, stage in enumerate(plan["stages"]):
        kind = stage["task"]["kind"]
        if kind not in ("tactical", "strategic"): raise ValueError(f"unsupported stage kind {kind}")
        cmd = [str(package / manifest["binary"]), "--catalog", str((package / plan["catalog"]).resolve()),
               "--evaluate-strategy" if kind == "strategic" else "--evaluate", str(brain_path.resolve()),
               "--train-config", str((package / stage["task"]["config"]).resolve()),
               "--eval-seeds", str(seeds), "--eval-seed-base", str(seed_base + index * 1_000_000)]
        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode: raise ValueError(f"local comparison failed: {result.stderr[-2000:]}")
        import json
        report = json.loads(result.stdout)
        score = report["fitness" if kind == "strategic" else "robust_fitness"]
        stages.append({"name": stage["name"], "difficulty": stage["difficulty"], "weight": stage["weight"],
                       "fitness": score, "details": report})
    report = aggregate(stages); transfer.write_json(directory / f"{label}.report.json", report)
    return report


def faction_protected(candidates,references,slack):
    def dimensions(report):
        result={}
        for stage in report["stages"]:
            details=stage["details"]
            if details.get("facets_complete") is not True or not details.get("facets"):raise ValueError("faction protection needs complete native metadata")
            for faction,score in details["facets"].items():
                if not math.isfinite(score):raise ValueError("nonfinite faction score")
                result[(stage["name"],faction)]=score
        return result
    a=[dimensions(r) for r in candidates];b=[dimensions(r) for r in references]
    if not a or len(a)!=len(b) or any(d.keys()!=a[0].keys() for d in a+b):raise ValueError("faction comparison dimensions differ")
    return all(sum(d[k] for d in a)/len(a)>=sum(d[k] for d in b)/len(b)-slack for k in a[0])

def integrate(package, incoming, source, best_dir, evidence):
    """Publish only selected brain objects; never rewrite a campaign optimizer."""
    best_dir = Path(best_dir); evidence = Path(evidence)
    config = transfer.read_json(package / "campaign.json"); lineage = transfer.read_json(package / "branch.json")
    with transfer.exclusive_lock(best_dir / ".selection.lock"):
        if (best_dir / "current.json").exists(): source = load_registry(best_dir)
        if source is None: raise ValueError("initial champion integration needs --against-run")
        if source["backend_identity"] != lineage["backend_identity"]: raise ValueError("local best uses a different executable")
        baseline = {t["name"]: t for t in source["tracks"]}; remote = {t["name"]: t for t in incoming["tracks"]}
        if not set(remote) <= set(baseline) or set(remote) != {t["name"] for t in config["tracks"]}:
            raise ValueError("champion track names differ")
        results = []; chosen = []; gain = config.get("min_gain", .005); slack = config.get("max_stage_regression", .15)
        for track in config["tracks"]:
            name = track["name"]; original = baseline[name]; candidate = remote[name]["best"]; incumbent = original["best"]
            if not name or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-" for c in name): raise ValueError("unsafe track name")
            fixed_reference=transfer.read_json(package/track["reference_brain"]) if track.get("reference_brain") else original["base"]
            faction_slack=config.get("max_faction_regression")
            if faction_slack is not None and (not math.isfinite(faction_slack) or faction_slack<0):raise ValueError("invalid faction slack")
            if faction_slack is not None:track={**track,"_native_evaluation":True}
            summary = {"name": name, "accepted": False}
            if transfer.encoded(candidate) == transfer.encoded(incumbent): summary["reason"] = "identical_brain"
            elif remote[name].get("best_report") is None: summary["reason"] = "unvalidated_remote_champion"
            else:
                print(f"Reassessing {name} on common fixed validation...", flush=True)
                directory = evidence / name
                reports = {label: evaluate(package, track, brain, track["validation"].get("seeds", 1), 1_000_000_000, directory, label)
                           for label, brain in [("candidate-fixed", candidate), ("incumbent-fixed", incumbent), ("initial-fixed", original["base"])]}
                c, b, initial = (reports[k] for k in ("candidate-fixed", "incumbent-fixed", "initial-fixed"))
                summary["fixed"] = reports
                passes = (config.get("fresh_screening", False) or c["fitness"] > b["fitness"] + gain) and all(a["fitness"] >= z["fitness"] - slack and a["fitness"] >= i["fitness"] - slack
                    for a, z, i in zip(c["stages"], b["stages"], initial["stages"]))
                if faction_slack is not None:
                    reference_report=evaluate(package,track,fixed_reference,track["validation"].get("seeds",1),1_000_000_000,directory,"reference-fixed")
                    summary["reference_fixed"]=reference_report
                    passes = passes and faction_protected([c],[reference_report],faction_slack)
                summary["reason"] = "fixed_validation_rejected"
                if passes:
                    # Seeded by both brains and returned state; retrying an import
                    # cannot keep rolling fresh conditions until a candidate wins.
                    token = transfer.digest(transfer.encoded([incoming["state_sha256"], candidate, incumbent]))
                    seed = (1 << 62) + int(token[:14], 16)
                    candidates = []; incumbents = []; references=[]
                    samples = max(4, config.get("promotion_seeds", 4))
                    if samples > 64: raise ValueError("promotion sample limit exceeded")
                    for index in range(samples):
                        print(f"Reassessing {name}, fresh pair {index + 1}/{samples}...", flush=True)
                        for label, brain, output in [("candidate", candidate, candidates), ("incumbent", incumbent, incumbents)]:
                            output.append(evaluate(package, track, brain, 1, seed + index * 100_000_000, directory, f"{label}-fresh-{index}"))
                        if faction_slack is not None:references.append(evaluate(package,track,fixed_reference,1,seed+index*100_000_000,directory,f"reference-fresh-{index}"))
                    summary["paired"] = assess(candidates, incumbents, gain, slack, config.get("promotion_min_t", 0.0))
                    summary["accepted"] = summary["paired"]["accepted"]
                    if faction_slack is not None:
                        summary["fresh_reference"]=references
                        summary["accepted"] &= faction_protected(candidates,references,faction_slack)
                    summary["reason"] = "fresh_validation_accepted" if summary["accepted"] else "fresh_validation_rejected"
            chosen.append({"name": name, "best": candidate if summary["accepted"] else incumbent, "base": original["base"]})
            results.append(summary)
        parked = sorted(set(baseline) - set(remote))
        chosen.extend(baseline[name] for name in parked)
        records = []
        for track in chosen:
            item = {"name": track["name"]}
            for field, key in [("best", "brain"), ("base", "initial_brain")]:
                payload = transfer.encoded(track[field]); identity = transfer.digest(payload)
                relative = f"objects/{identity}.json"; path = best_dir / relative
                if path.exists() and path.read_bytes() != payload: raise ValueError("existing best object differs")
                if not path.exists(): transfer.write_json(path, track[field])
                item[key] = relative; item[key + "_sha256"] = identity
            records.append(item)
        report = {"branch_id": lineage["branch_id"], "source_state_sha256": incoming["state_sha256"], "tracks": results, "parked_tracks": parked,
                  "note": "Local validation selection; holdout never selects. Paired guards are not a significance test."}
        transfer.write_json(evidence / "selection.json", report)
        transfer.write_json(best_dir / "current.json", {"format": "minibar.best_brains", "version": 1,
            "backend_identity": lineage["backend_identity"], "tracks": records, "last_assessment": str((evidence / "selection.json").resolve())})
        return report
