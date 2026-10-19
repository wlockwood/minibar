#!/usr/bin/env python3
"""Progress dashboard for a MiniBAR training campaign (read-only, standard library only).

Reads a campaign output directory (the folder that holds `run/`, or the `run/` folder itself),
extracts what answers "is training making progress?", and writes one self-contained HTML file.

    python scripts/campaign_dashboard.py data/brains/campaign-validation-20261017
    python scripts/campaign_dashboard.py data/brains/campaign --watch 20      # live: rewrite every 20 s
    python scripts/campaign_dashboard.py --demo data/brains/demo-campaign      # synthetic 40-generation run

Inputs (all optional except `state.json` or `history.jsonl`):
  run/state.json     committed history, champions, structural snapshots/lineage, initial/best/holdout reports
  run/history.jsonl  appended per generation (used when state.json is absent)
  run/summary.json   status and target generations
  ../campaign.json   max_stage_regression, validation cadence, promotion thresholds
  ../holdout.<track>.initial.json   holdout score of the starting brain

Nothing in the campaign directory is modified. The page embeds its data, so it opens from disk.
"""
import argparse
import json
import math
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
TEMPLATE = os.path.join(os.path.dirname(HERE), "viewer", "campaign_dashboard.template.html")

STALL_GENERATIONS = 10        # generations without a new champion before a stall warning
TIMEOUT_WARN = 0.25           # fraction of validation battles that time out
FLAT_STD = 1e-9               # across-population std below this: no learning signal on that stage
PACKAGE_PROFILES = {"production": "campaign.json", "pilot": "pilot.json", "cohesion": "pilot-cohesion.json"}
OUTCOME_KEYS = ("wins", "draws", "timeouts", "losses", "battles", "underdog_wins", "underdog_battles",
                "duration_sum", "exchange_sum")
TRIAL_KEYS = ("fitness", "projects_completed", "units_produced", "actions_rejected", "metal_income",
              "energy_income", "metal_overflow", "energy_overflow", "unmet_energy", "army_value",
              "economy_value", "spent_metal")


# ----------------------------------------------------------------------------- loading

def _read_snapshot(path):
    with open(path, "rb") as handle:
        payload = handle.read()
    return payload.decode("utf-8")


def read_json(path):
    try:
        # Decode after closing: large checkpoints can take seconds to parse.
        return json.loads(_read_snapshot(path))
    except (OSError, ValueError):
        return None


def read_jsonl(path):
    rows = []
    try:
        for line in _read_snapshot(path).split("\n"):
            line = line.strip()
            if line:
                try:
                    rows.append(json.loads(line))
                except ValueError:
                    break          # a half-written final line while training is running
    except OSError:
        pass
    return rows


def locate(path):
    """Return (campaign_root, run_dir) for a campaign root or a run directory."""
    path = os.path.abspath(path)
    if os.path.exists(os.path.join(path, "run", "state.json")) or os.path.exists(os.path.join(path, "run", "history.jsonl")):
        return path, os.path.join(path, "run")
    if os.path.exists(os.path.join(path, "state.json")) or os.path.exists(os.path.join(path, "history.jsonl")):
        parent = os.path.dirname(path)
        package = os.path.dirname(parent)
        if os.path.basename(parent).lower() == "runs" and os.path.basename(path).lower() in PACKAGE_PROFILES:
            receipt = read_json(os.path.join(package, "receipt.json")) or {}
            if receipt.get("format") == "minibar.diverse_successor":
                return package, path
        return os.path.dirname(path), path
    raise SystemExit(f"no campaign state found under {path} (expected run/state.json or run/history.jsonl)")


def load_retest(root, name):
    """The optional fresh-seed retest written by scripts/fresh_retest.py (read-only here)."""
    data = read_json(os.path.join(root, f"fresh-retest.{name}.json"))
    if not isinstance(data, dict) or data.get("format") != "minibar.fresh_retest" or not isinstance(data.get("families"), list):
        return None
    return data


def wilson(wins, n, z=1.96):
    """95% Wilson interval for a proportion; (0, 1) when there is no data."""
    if n <= 0:
        return 0.0, 1.0
    p = wins / n
    denom = 1 + z * z / n
    centre = (p + z * z / (2 * n)) / denom
    half = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / denom
    return max(0.0, centre - half), min(1.0, centre + half)


# ----------------------------------------------------------------------------- reports

def evaluation_details(value):
    """Visit evaluation leaves inside layout/opponent wrappers, never arbitrary brain data."""
    if not isinstance(value, dict):
        return
    if "matchup_records" in value or "families" in value:
        yield value
    for key in ("report", "baseline", "historical", "scripted", "learned"):
        yield from evaluation_details(value.get(key))
    for layout in value.get("layouts", []) or []:
        yield from evaluation_details(layout)


def family_fitness(details):
    """Keep native pooled scores; only average legacy wrapper levels when needed."""
    if not isinstance(details, dict):
        return {}
    if "families" in details:
        return {name: value for name, value in (details.get("families") or {}).items()
                if isinstance(value, (int, float)) and math.isfinite(value)}
    if isinstance(details.get("report"), dict):
        return family_fitness(details["report"])
    branches = [details[key] for key in ("baseline", "historical", "scripted", "learned")
                if isinstance(details.get(key), dict)]
    if not branches:
        branches = details.get("layouts", []) or []
    scores = {}
    for branch in branches:
        for family, fitness in family_fitness(branch).items():
            scores.setdefault(family, []).append(fitness)
    return {family: sum(values) / len(values) for family, values in scores.items()}


def resource_coverage(details, with_rows):
    records, seen = [], set()
    for evaluation in evaluation_details(details):
        for record in evaluation.get("matchup_records", []) or []:
            if not isinstance(record, dict):
                continue
            identity = json.dumps(record, sort_keys=True)
            if identity not in seen:
                seen.add(identity)
                records.append(record)
    coverage = {}
    for record in records:
        family = record.get("family") or "unlabelled"
        coverage[family] = coverage.get(family, 0) + 1
    if isinstance(details.get("coverage_counts"), dict):
        coverage = dict(details["coverage_counts"])
    result = {"coverage": coverage,
              "family_fitness": family_fitness(details)}
    if details.get("matchup_records_sampled"):
        result["matchup_records_sampled"] = True
    if with_rows:
        result["matchup_records"] = records
    return result


def brain_domain(saved):
    kind = ((saved.get("best") or saved.get("base") or {}).get("kind") or "").lower()
    if "air" in kind:
        return "air"
    if "strategic" in kind:
        return "strategic"
    return "ground" if kind.startswith("tactical") or "ground" in kind else "unknown"


def summarize_report(report, with_rows=False):
    """Compress one validation PlanReport to what the dashboard plots."""
    if not report:
        return None
    out = {"fitness": report.get("fitness"), "stages": [], "outcome": None, "trials": None}
    total = {key: 0.0 for key in OUTCOME_KEYS}
    have_outcome, trials = False, []
    for stage in report.get("stages", []):
        details = stage.get("details") or {}
        entry = {"name": stage.get("name"), "weight": stage.get("weight"), "fitness": stage.get("fitness")}
        entry.update(resource_coverage(details, with_rows))
        tally = details.get("total")
        if tally:
            have_outcome = True
            entry["outcome"] = {key: tally.get(key, 0) for key in OUTCOME_KEYS}
            for key in OUTCOME_KEYS:
                total[key] += tally.get(key, 0)
            if with_rows:
                entry["rows"] = [
                    {"scenario": row.get("scenario"), "opponent": row.get("opponent"),
                     **{key: (row.get("tally") or {}).get(key, 0) for key in OUTCOME_KEYS}}
                    for row in details.get("rows", [])
                ]
        for trial in details.get("trials", []) or []:
            trials.append({"stage": stage.get("name"), "map": trial.get("map"), "faction": trial.get("faction"),
                           "side": trial.get("learner_side"),
                           **{key: trial.get(key) for key in TRIAL_KEYS}})
        out["stages"].append(entry)
    if have_outcome:
        out["outcome"] = total
    if trials:
        out["trials"] = trials
        out["trial_means"] = {key: sum((t.get(key) or 0) for t in trials) / len(trials) for key in TRIAL_KEYS}
        out["trial_means"]["rejected_total"] = sum((t.get("actions_rejected") or 0) for t in trials)
    return out


def summarize_stage_file(data):
    """Initial-holdout files hold one stage's details (tactical: total/rows/robust_fitness, strategic:
    fitness/trials), not a full report. Wrap it so it reads like a one-stage report."""
    if not data:
        return None
    if "stages" in data:
        return summarize_report(data)
    fitness = data.get("fitness", data.get("robust_fitness"))
    return summarize_report({"fitness": fitness, "stages": [
        {"name": "holdout", "weight": 1.0, "fitness": fitness, "details": data}]})


def summarize_structure(brain):
    """Describe actual enabled paths; counts are structure, never behavioral importance."""
    if not brain:
        return {"available": False, "error": "Brain snapshot is unavailable.", "modules": []}
    modules = []
    try:
        for mod in brain.get("modules", []):
            layers = mod["layers"]
            sizes = [len(mod["inputs"])] + [len(layer["weights"]) for layer in layers]
            if len(sizes) < 2 or any(not n for n in sizes):
                raise ValueError("empty network layer")
            structure = mod.get("structure")
            if structure is None:
                if brain.get("version", 1) != 1:
                    raise ValueError("structural metadata is missing")
                structure = {"inputs": [True] * sizes[0], "hidden": [["tanh"] * n for n in sizes[1:-1]],
                             "connections": [[[True] * sizes[i] for _ in range(sizes[i+1])] for i in range(len(layers))]}
            inputs, edges, activations = structure["inputs"], structure["connections"], structure["hidden"]
            if len(inputs) != sizes[0] or any(type(on) is not bool for on in inputs):
                raise ValueError("input mask shape is invalid")
            if len(edges) != len(layers) or len(activations) != len(layers)-1:
                raise ValueError("structure layer counts are invalid")
            for i, layer in enumerate(layers):
                if len(layer["biases"]) != sizes[i+1] or any(len(row) != sizes[i] for row in layer["weights"]):
                    raise ValueError("weight matrix shape is invalid")
                if len(edges[i]) != sizes[i+1] or any(len(row) != sizes[i] or any(type(on) is not bool for on in row) for row in edges[i]):
                    raise ValueError("connection mask shape is invalid")
                if i < len(activations) and (len(activations[i]) != sizes[i+1] or any(a not in ("tanh", "softsign", "leaky_relu") for a in activations[i])):
                    raise ValueError("hidden activation shape or name is invalid")
            reaches_output = [True] * sizes[-1]
            for matrix in reversed(edges):
                reaches_output = [any(matrix[row][col] and reaches_output[row] for row in range(len(matrix))) for col in range(len(matrix[0]))]
            nodes, graph_edges = [], []
            outputs = mod.get("outputs", [f"output {i+1}" for i in range(sizes[-1])])
            if len(outputs) != sizes[-1]:
                raise ValueError("output names do not match output layer")
            for layer, count in enumerate(sizes):
                for neuron in range(count):
                    is_input, is_output = layer == 0, layer == len(sizes)-1
                    label = mod["inputs"][neuron] if is_input else outputs[neuron] if is_output else f"H{layer}.{neuron+1}"
                    identity = "input:" + label if is_input else "output:" + label if is_output else f"hidden:{layer}:{neuron}"
                    nodes.append({"id": identity, "layer": layer, "neuron": neuron, "label": label,
                                  "enabled": inputs[neuron] if is_input else True,
                                  "connected": bool(inputs[neuron] and reaches_output[neuron]) if is_input else None,
                                  "activation": None if is_input else "tanh" if is_output else activations[layer-1][neuron]})
            node_by_position = {(n["layer"], n["neuron"]): n for n in nodes}
            for layer, matrix in enumerate(edges):
                for row, columns in enumerate(matrix):
                    for col, enabled in enumerate(columns):
                        source, target = node_by_position[layer, col], node_by_position[layer+1, row]
                        graph_edges.append({"id": source["id"] + "->" + target["id"], "source": source["id"], "target": target["id"],
                                            "enabled": enabled, "effective": enabled and (layer != 0 or inputs[col]),
                                            "layer": layer, "row": row, "col": col, "weight": layers[layer]["weights"][row][col]})
            counts = {a: sum(layer.count(a) for layer in activations) for a in ("tanh", "softsign", "leaky_relu")}
            modules.append({"name": mod["name"], "role": mod.get("role", ""), "sizes": sizes,
                            "nodes": nodes, "edges": graph_edges, "activation_counts": counts,
                            "enabled_inputs": sum(inputs), "available_inputs": len(inputs),
                            "connected_inputs": sum(on and path for on, path in zip(inputs, reaches_output)),
                            "enabled_connections": sum(e["enabled"] for e in graph_edges),
                            "possible_connections": len(graph_edges)})
        return {"available": bool(modules), "modules": modules}
    except (KeyError, IndexError, TypeError, ValueError) as error:
        return {"available": False, "error": f"Cannot display this snapshot: {error}", "modules": []}


def structure_diff(parent, child):
    """Match stable named endpoints, rather than truncating mismatched matrices."""
    a, b = summarize_structure(parent), summarize_structure(child)
    if not a["available"] or not b["available"]:
        return []
    previous = {m["name"]: m for m in a["modules"]}
    changes = []
    for module in b["modules"]:
        old = previous.get(module["name"])
        if not old or old["sizes"] != module["sizes"]:
            changes.append({"kind": "shape", "module": module["name"], "text": "Layer sizes differ; weight comparison unavailable."})
            continue
        old_nodes = {n["id"]: n for n in old["nodes"]}
        for node in module["nodes"]:
            before = old_nodes.get(node["id"])
            if not before:
                continue
            kind, setting = ("input", "enabled") if node["layer"] == 0 else ("activation", "activation")
            if before[setting] != node[setting]:
                changes.append({"kind": kind, "module": module["name"], "target": node["id"], "label": node["label"], "before": before[setting], "after": node[setting]})
        old_edges = {e["id"]: e for e in old["edges"]}
        labels = {n["id"]: n["label"] for n in module["nodes"]}
        for edge in module["edges"]:
            before = old_edges.get(edge["id"])
            if before and before["enabled"] != edge["enabled"]:
                changes.append({"kind": "connection", "module": module["name"], "target": edge["id"],
                                "label": labels[edge["source"]] + " → " + labels[edge["target"]], "before": before["enabled"], "after": edge["enabled"]})
    return changes


def structure_history(saved, completed):
    """Read only committed snapshots/events; legacy endpoints are not a timeline."""
    raw = dict(saved.get("structure_snapshots") or {})
    def endpoint(label, value):
        if not value:
            return None
        found = next((key for key, brain in raw.items() if brain == value), None)
        if found is None:
            found = label
            raw[found] = value
        return found
    initial = endpoint("starting-brain", saved.get("base"))
    champion = endpoint("current-champion", saved.get("best"))
    snapshots = {key: summarize_structure(value) for key, value in raw.items()}
    events = []
    for event in saved.get("structural_history", []):
        if not 0 < event.get("generation", 0) <= completed:
            continue
        event = dict(event)
        parent, proposal = event.get("parent_sha256"), event.get("proposal_sha256")
        event["available"] = bool(snapshots.get(parent, {}).get("available") and snapshots.get(proposal, {}).get("available"))
        event["changes"] = structure_diff(raw.get(parent), raw.get(proposal)) if event["available"] else []
        events.append(event)
    promotions = [p for p in saved.get("promotion_history", []) if 0 < p.get("generation", 0) <= completed]
    comparisons = {}
    for event in events:
        comparisons[event.get("proposal_sha256")] = event.get("parent_sha256")
    for event in promotions:
        comparisons[event.get("brain_sha256")] = event.get("parent_sha256")
    diffs = {key: {"parent": structure_diff(raw.get(comparisons.get(key)), value),
                   "initial": structure_diff(raw.get(initial), value)} for key, value in raw.items()}
    return {"structure_snapshots": snapshots, "structural_events": events, "promotion_history": promotions,
            "champion_snapshot": champion, "initial_snapshot": initial, "snapshot_parents": comparisons,
            "structure_diffs": diffs, "structural_history_available": "structural_history" in saved}


def brain_drift(base, best):
    """How far the champion's weights moved from the starting brain, per labelled module and per input
    sensor (first-layer weights are stored hidden-unit by input). Magnitudes only: a sign on an MLP weight
    does not by itself say "more" or "less" of a behaviour."""
    if not base or not best:
        return []
    out = []
    base_modules = {m.get("name"): m for m in base.get("modules", [])}
    for module in best.get("modules", []):
        start = base_modules.get(module.get("name"))
        if not start or len(start.get("layers", [])) != len(module.get("layers", [])):
            continue
        a_summary, b_summary = summarize_structure({"modules": [start]}), summarize_structure({"modules": [module]})
        if not a_summary["available"] or not b_summary["available"]:
            continue
        a_graph, b_graph = a_summary["modules"][0], b_summary["modules"][0]
        if a_graph["sizes"] != b_graph["sizes"] or start.get("inputs") != module.get("inputs"):
            continue
        active_a = {e["id"]: e for e in a_graph["edges"] if e["effective"]}
        common = {e["id"] for e in b_graph["edges"] if e["effective"]} & active_a.keys()
        identities = {(e["layer"], e["row"], e["col"]): e["id"] for e in b_graph["edges"]}
        moved = norm = 0.0
        per_input = [0.0] * len(module.get("inputs", []))
        for layer_index, (a, b) in enumerate(zip(start["layers"], module["layers"])):
            for row_index, (row_a, row_b) in enumerate(zip(a["weights"], b["weights"])):
                for k, (x, y) in enumerate(zip(row_a, row_b)):
                    if identities[layer_index, row_index, k] not in common:
                        continue
                    moved += (y - x) ** 2
                    norm += x * x
                    if a is start["layers"][0] and k < len(per_input):
                        per_input[k] += (y - x) ** 2
            for x, y in zip(a["biases"], b["biases"]):
                moved += (y - x) ** 2
                norm += x * x
        top = sorted(zip(module.get("inputs", []), per_input), key=lambda p: -p[1])[:3]
        out.append({"module": module.get("name"), "role": module.get("role"),
                    "change": math.sqrt(moved), "relative": math.sqrt(moved) / (math.sqrt(norm) or 1.0),
                    "top_inputs": [{"input": name, "change": math.sqrt(v)} for name, v in top if v > 0]})
    return out


def summarize_generation(record, latest):
    islands = []
    for island in record.get("islands", []):
        optimizer = island.get("optimizer") or {}
        islands.append({
            "name": island.get("name"),
            "mean": optimizer.get("fitness_mean"), "best": optimizer.get("fitness_best"),
            "worst": optimizer.get("fitness_worst"), "update_norm": optimizer.get("update_norm"),
            "stage_diagnostics": island.get("stage_diagnostics") or [],
            "center": summarize_report(island.get("center_validation"), with_rows=latest),
            "sample": summarize_report(island.get("sample_validation"), with_rows=False),
            "promoted": island.get("promoted") or [],
            "decisions": island.get("decisions") or [],
            "restarted_module": island.get("restarted_module"),
            "challenges": [
                {"candidate": c.get("candidate"), "accepted": (c.get("report") or {}).get("accepted")}
                for c in island.get("promotion_challenges", []) or []
            ],
        })
    return {"generation": record.get("generation"), "champion_fitness": record.get("champion_fitness"),
            "best_generation": record.get("best_generation"), "islands": islands}


def load_model(path):
    root, run = locate(path)
    state = read_json(os.path.join(run, "state.json"))
    summary = read_json(os.path.join(run, "summary.json")) or {}
    config_name = "campaign.json"
    if os.path.normcase(os.path.dirname(run)) == os.path.normcase(os.path.join(root, "runs")):
        config_name = PACKAGE_PROFILES.get(os.path.basename(run).lower(), config_name)
    config = read_json(os.path.join(root, config_name)) or {}
    if state:
        history = state.get("history", [])
        state_tracks = {t["name"]: t for t in state.get("tracks", [])}
    else:
        history = read_jsonl(os.path.join(run, "history.jsonl"))
        state_tracks = {}
    # history.jsonl is appended per generation, so it can be ahead of state.json while a generation commits
    appended = read_jsonl(os.path.join(run, "history.jsonl"))
    if appended and (not history or appended[-1].get("generation", 0) > history[-1].get("generation", 0)):
        history = appended
    track_names = []
    for record in history:
        for track in record.get("tracks", []):
            if track["name"] not in track_names:
                track_names.append(track["name"])
    for name in state_tracks:
        if name not in track_names:
            track_names.append(name)
    tracks = []
    for name in track_names:
        records = []
        for record in history:
            for track in record.get("tracks", []):
                if track["name"] == name:
                    records.append({"generation": record.get("generation"), **track})
        generations = [summarize_generation(r, latest=(i == len(records) - 1)) for i, r in enumerate(records)]
        saved = state_tracks.get(name, {})
        summary_track = next((t for t in summary.get("tracks", []) if t.get("name") == name), {})
        initial = summarize_report(saved.get("initial_report"), with_rows=True)
        best = summarize_report(saved.get("best_report"), with_rows=True)
        holdout = summarize_report(saved.get("holdout"), with_rows=True)
        holdout_initial = summarize_stage_file(read_json(os.path.join(root, f"holdout.{name}.initial.json")))
        if initial is None and summary_track.get("initial_fitness") is not None:
            initial = {"fitness": summary_track["initial_fitness"], "stages": [], "outcome": None, "trials": None}
        if best is None and summary_track.get("best_fitness") is not None:
            best = {"fitness": summary_track["best_fitness"], "stages": [], "outcome": None, "trials": None}
        if holdout is None and summary_track.get("holdout_fitness") is not None:
            holdout = {"fitness": summary_track["holdout_fitness"], "stages": [], "outcome": None, "trials": None}
        islands_state = saved.get("islands", [])
        tracks.append({
            "name": name,
            "brain_domain": brain_domain(saved),
            "kind": "strategic" if ((best or {}).get("trials") or (initial or {}).get("trials")) else "tactical",
            "initial": initial, "best": best, "holdout": holdout, "holdout_initial": holdout_initial,
            "retest": load_retest(root, name),
            "best_generation": saved.get("best_generation", summary_track.get("best_generation")),
            "generations": generations,
            "search_islands": next((t.get("islands", []) for t in config.get("tracks", []) if t.get("name") == name), []),
            "islands_now": [{"name": i.get("name"), "last_progress": i.get("last_progress"), "restarts": i.get("restarts")}
                            for i in islands_state],
            "archive_size": len(saved.get("archive", [])),
            "diagnostic_challengers": len((saved.get("frontier") or {}).get("entries", [])),
            "drift": brain_drift(saved.get("base"), saved.get("best")),
            **structure_history(saved, (state or {}).get("completed_generations", 0)),
        })
    completed = (state or {}).get("completed_generations") or (history[-1].get("generation") if history else 0)
    meta = {
        "campaign": config.get("name") or os.path.basename(root),
        "directory": root,
        "status": summary.get("status") or ("running" if history else "unknown"),
        "completed_generations": completed,
        "target_generations": summary.get("target_generations") or config.get("generations"),
        "waiting_jobs": summary.get("waiting_jobs"),
        "validation_every": config.get("validation_every"),
        "max_stage_regression": config.get("max_stage_regression"),
        "min_gain": config.get("min_gain"),
        "fresh_screening": config.get("fresh_screening", False),
        "screen_min_gain": config.get("screen_min_gain", 0.0),
        "promotion_seeds": config.get("promotion_seeds", 4),
        "promotion_min_t": config.get("promotion_min_t", 0.0),
        "parked_tracks": (read_json(os.path.join(root, "branch.json")) or {}).get("parked_tracks", []),
        "restart_after": config.get("restart_after"),
        "generated_at": time.strftime("%Y-%m-%d %H:%M:%S"),
    }
    for track in tracks:
        track["events"] = build_events(track, meta)
        track["verdict"] = build_verdict(track, meta)
    return {"meta": meta, "tracks": tracks}


# ----------------------------------------------------------------------------- interpretation

def fmt(value, places=3):
    return "n/a" if value is None else f"{value:+.{places}f}"


T_SIGNIFICANT = 2.0           # |t| of a paired fresh-seed difference treated as a real effect


def retest_rows(retest, brain):
    """{family: row} for one brain of a fresh-seed retest."""
    rows = {}
    for family in (retest or {}).get("families", []):
        row = next((b for b in family.get("brains", []) if b.get("name") == brain), None)
        if row and row.get("t") is not None:
            rows[family["name"]] = row
    return rows


def retest_effect(row):
    if row["t"] >= T_SIGNIFICANT:
        return "up"
    return "down" if row["t"] <= -T_SIGNIFICANT else "none"


def retest_reading(retest):
    """(level, sentence) from the champion's paired fresh-seed differences against the starting brain."""
    champion = retest_rows(retest, "champion")
    if not champion:
        return None
    effects = {name: retest_effect(row) for name, row in champion.items()}
    parts = [f"{name} {fmt(row['diff'])} (t {row['t']:+.1f})" for name, row in champion.items()]
    n = next((f.get("n") for f in retest["families"] if f.get("n")), None)
    text = f"Fresh-seed retest ({n} paired seeds per family), champion vs the starting brain: " + ", ".join(parts) + "."
    ups = [k for k, v in effects.items() if v == "up"]
    downs = [k for k, v in effects.items() if v == "down"]
    if ups and downs:
        text += f" It gains on {', '.join(ups)} but loses on {', '.join(downs)}: it looks tuned to the scenarios that selected it."
        level = "mixed"
    elif ups and (effects.get("holdout") in ("up", None)):
        level = "progress"
    elif ups:
        text += " The holdout shows no clear gain."
        level = "early"
    elif downs:
        level = "regress"
    else:
        text += " No family shows a clear difference."
        level = "flat"
    better = []
    for family in retest["families"]:
        if family["name"] != "holdout":
            continue
        for row in family.get("brains", []):
            if row["name"] != "champion" and row.get("t") is not None and row["t"] >= T_SIGNIFICANT and effects.get("holdout") != "up":
                better.append(f"{row['name']} {fmt(row['diff'])}")
    if better:
        text += " Not champion, but ahead on the holdout: " + ", ".join(better) + "."
    return level, text


def build_verdict(track, meta):
    """A cautious one-line reading of the numbers; never claims more than the data supports."""
    initial, best = track.get("initial"), track.get("best")
    done = len(track["generations"])
    if not initial or not best or initial.get("fitness") is None or best.get("fitness") is None:
        return {"level": "waiting", "text": "No validated champion yet."}
    gain = best["fitness"] - initial["fitness"]
    fixed_note = " on the fixed validation set"
    if meta.get("fresh_screening"):
        fixed_note += " (promotion no longer uses it, and its start score is selection-biased)"
    pieces = [f"Champion validation {best['fitness']:.3f} vs {initial['fitness']:.3f} at the start ({fmt(gain)}){fixed_note}."]
    level = "progress" if gain > 0 else "flat" if gain == 0 else "regress"
    outcome_i, outcome_b = initial.get("outcome"), best.get("outcome")
    if outcome_i and outcome_b and outcome_i["battles"] and outcome_b["battles"]:
        lo_i, hi_i = wilson(outcome_i["wins"], outcome_i["battles"])
        lo_b, hi_b = wilson(outcome_b["wins"], outcome_b["battles"])
        rate_i, rate_b = outcome_i["wins"] / outcome_i["battles"], outcome_b["wins"] / outcome_b["battles"]
        overlap = not (lo_b > hi_i or lo_i > hi_b)
        pieces.append(f"Win rate {rate_i:.0%} -> {rate_b:.0%} ({int(outcome_b['wins'])}/{int(outcome_b['battles'])} battles)"
                      + ("; the intervals overlap, so this is within noise." if overlap else "; the intervals do not overlap."))
        if overlap and level == "progress":
            level = "early"
    holdout, holdout_initial = track.get("holdout"), track.get("holdout_initial")
    if holdout and holdout_initial and holdout.get("fitness") is not None and holdout_initial.get("fitness") is not None:
        pieces.append(f"Unselected holdout {fmt(holdout['fitness'] - holdout_initial['fitness'])} vs the starting brain.")
    if done < 3:
        pieces.append(f"Only {done} generation(s) so far; too early to read a trend.")
        level = "early" if level == "progress" else level
    since = (track["generations"][-1]["generation"] or 0) - (track.get("best_generation") or 0) if done else 0
    if since >= STALL_GENERATIONS:
        pieces.append(f"No new champion for {since} generations.")
        level = "stalled"
    reading = retest_reading(track.get("retest"))
    if reading:
        # Fresh seeds are the better evidence: they lead the text and decide the level.
        level = reading[0] if level != "waiting" else level
        pieces.insert(0, reading[1])
    elif meta.get("fresh_screening") and track.get("kind") == "tactical":
        pieces.append("No fresh-seed retest found: run scripts/fresh_retest.py to compare champion and start on new seeds.")
    return {"level": level, "text": " ".join(pieces)}


def build_events(track, meta):
    events = []
    previous_best = None
    previous_fitness = None
    generations = track["generations"]
    for record in generations:
        gen = record["generation"]
        if previous_best is not None and record.get("best_generation") != previous_best:
            gain = ""
            if previous_fitness is not None and record.get("champion_fitness") is not None:
                gain = f" ({fmt(record['champion_fitness'] - previous_fitness)})"
            events.append({"generation": gen, "kind": "champion", "level": "good",
                           "text": f"New champion, validation {record['champion_fitness']:.3f}{gain}"})
        elif previous_best is None and record.get("best_generation") == gen:
            events.append({"generation": gen, "kind": "champion", "level": "good",
                           "text": f"First champion, validation {record['champion_fitness']:.3f}"})
        previous_best, previous_fitness = record.get("best_generation"), record.get("champion_fitness")
        for island in record["islands"]:
            if island.get("restarted_module"):
                events.append({"generation": gen, "kind": "restart", "level": "info",
                               "text": f"{island['name']} island restarted module {island['restarted_module']}"})
            explanations = {
                "screen_below_champion": "scored at or below the champion in screening",
                "screen_gain_too_small": "screening gain was too small to advance",
                "stage_regression": "lost too much on an individual stage",
                "faction_regression": "lost too much against a protected faction",
                "fresh_gain_too_small": "passed screening, but its gain on independent fresh tests was too small",
                "fresh_majority_failed": "passed screening, but did not win a majority of independent fresh tests",
                "fresh_evidence_too_weak": "passed screening, but the fresh-test gains were too inconsistent",
                "fresh_stage_regression": "passed screening, but lost too much on a fresh-test stage",
                "fresh_faction_regression": "passed screening, but lost too much against a faction in fresh tests",
                "fixed_audit_rejected": "did not pass the fixed validation screen",
            }
            for decision in island.get("decisions", []):
                reason = decision.get("reason")
                if reason == "promoted":
                    continue
                text = explanations.get(reason, reason or "decision unavailable")
                gain = decision.get("screen_gain")
                suffix = f" (screening gain {gain:+.4f})" if isinstance(gain, (int, float)) else ""
                events.append({"generation": gen, "kind": "decision", "level": "info",
                               "text": f"{island['name']} {decision.get('candidate')}: {text}{suffix}."})
            for challenge in ([] if island.get("decisions") else island.get("challenges", [])):
                if challenge.get("accepted") is False:
                    events.append({"generation": gen, "kind": "promotion", "level": "warn",
                                   "text": f"{island['name']} {challenge.get('candidate')} candidate won validation but lost the paired promotion check"})
            outcome = (island.get("center") or {}).get("outcome")
            if outcome and outcome.get("battles") and outcome["timeouts"] / outcome["battles"] >= TIMEOUT_WARN:
                events.append({"generation": gen, "kind": "timeouts", "level": "warn",
                               "text": f"{island['name']}: {outcome['timeouts'] / outcome['battles']:.0%} of validation battles timed out (stalling, not winning)"})
            trial_means = (island.get("center") or {}).get("trial_means")
            if trial_means and trial_means.get("rejected_total"):
                events.append({"generation": gen, "kind": "rejections", "level": "warn",
                               "text": f"{island['name']}: {int(trial_means['rejected_total'])} rejected build orders in validation"})
    if generations:
        last = generations[-1]
        for island in last["islands"]:
            flat = [d["name"] for d in island.get("stage_diagnostics", [])
                    if (d.get("std_dev") or 0) < FLAT_STD or (d.get("unique_fitness") or 0) <= 1]
            if flat:
                events.append({"generation": last["generation"], "kind": "signal", "level": "warn",
                               "text": f"{island['name']}: no spread between candidates on {', '.join(flat)}, so no learning signal there this generation"})
        since = (last["generation"] or 0) - (track.get("best_generation") or 0)
        if since >= STALL_GENERATIONS:
            events.append({"generation": last["generation"], "kind": "stall", "level": "warn",
                           "text": f"No new champion for {since} generations (last at generation {track.get('best_generation')})"})
    initial, best = track.get("initial"), track.get("best")
    limit = meta.get("max_stage_regression")
    if initial and best and limit is not None:
        for stage_i in initial.get("stages", []):
            stage_b = next((s for s in best.get("stages", []) if s["name"] == stage_i["name"]), None)
            if stage_b and None not in (stage_i.get("fitness"), stage_b.get("fitness")):
                drop = stage_i["fitness"] - stage_b["fitness"]
                if drop > limit:
                    events.append({"generation": generations[-1]["generation"] if generations else 0, "kind": "forgetting",
                                   "level": "warn", "text": f"Champion is {drop:.3f} below the starting brain on the {stage_i['name']} stage (limit {limit})"})
    holdout, holdout_initial = track.get("holdout"), track.get("holdout_initial")
    if holdout and holdout_initial and None not in (holdout.get("fitness"), holdout_initial.get("fitness")) and best and initial:
        if best["fitness"] > initial["fitness"] and holdout["fitness"] < holdout_initial["fitness"]:
            events.append({"generation": generations[-1]["generation"] if generations else 0, "kind": "overfit", "level": "warn",
                           "text": "Validation improved but the unselected holdout got worse: possible overfitting to the training scenarios"})
    champion = retest_rows(track.get("retest"), "champion")
    if champion.get("validation") and champion.get("holdout")             and retest_effect(champion["validation"]) == "up" and retest_effect(champion["holdout"]) == "down":
        events.append({"generation": generations[-1]["generation"] if generations else 0, "kind": "overfit", "level": "warn",
                       "text": f"Fresh-seed retest: champion beats the start on validation ({fmt(champion['validation']['diff'])}) "
                               f"but is worse on the unselected holdout ({fmt(champion['holdout']['diff'])})"})
    events.sort(key=lambda e: (-(e["generation"] or 0), e["kind"]))
    return events[:60]


# ----------------------------------------------------------------------------- output

def render(model, live_seconds=None):
    with open(TEMPLATE, encoding="utf-8") as handle:
        template = handle.read()
    model = dict(model)
    model["meta"] = dict(model["meta"], live_seconds=live_seconds)
    payload = json.dumps(model, separators=(",", ":")).replace("</", "<\\/")
    return template.replace("/*__DATA__*/null", payload)


def write_dashboard(source, out, live_seconds=None):
    html = render(load_model(source), live_seconds)
    temporary = out + ".tmp"
    with open(temporary, "w", encoding="utf-8") as handle:
        handle.write(html)
    os.replace(temporary, out)      # the browser never sees a half-written page


# ----------------------------------------------------------------------------- demo data

def make_demo(out_dir, generations=40, seed=7):
    """Write a synthetic campaign directory shaped like the real one (for previews and tests)."""
    import random
    rng = random.Random(seed)
    run = os.path.join(out_dir, "run")
    os.makedirs(os.path.join(run, "tracks"), exist_ok=True)
    scenarios = ["armada duel", "cortex two v three", "legion three v three", "armada mixed roles", "legion asset defence"]
    opponents = ["charge", "hold", "kite"]
    stage_of = {"armada duel": "easy", "cortex two v three": "easy", "legion three v three": "mixed",
                "armada mixed roles": "mixed", "legion asset defence": "objectives"}
    base_skill = {"armada duel": 0.35, "cortex two v three": 0.2, "legion three v three": 0.45,
                  "armada mixed roles": 0.3, "legion asset defence": 0.1}

    def tactical_report(progress, noise):
        stages = []
        for stage in ("easy", "mixed", "objectives"):
            rows, total = [], {k: 0 for k in OUTCOME_KEYS}
            for scenario in [s for s in scenarios if stage_of[s] == stage]:
                for opponent in opponents:
                    p = min(0.97, max(0.02, base_skill[scenario] + progress * (0.55 if stage != "objectives" else 0.4)
                                      + rng.uniform(-noise, noise) - (0.1 if opponent == "kite" else 0)))
                    battles = 4
                    wins = sum(rng.random() < p for _ in range(battles))
                    timeouts = sum(1 for _ in range(battles - wins) if rng.random() < max(0.05, 0.5 - progress))
                    losses = battles - wins - timeouts
                    tally = {"battles": battles, "wins": wins, "draws": 0, "timeouts": timeouts, "losses": losses,
                             "underdog_battles": 2, "underdog_wins": min(wins, 1 if p > 0.4 else 0),
                             "duration_sum": 30.0 * battles, "exchange_sum": 0.1 * wins - 0.1 * losses,
                             "fitness_sum": 0.0, "key_term_sum": 0.0}
                    rows.append({"scenario": scenario, "opponent": opponent, "tally": tally})
                    for k in OUTCOME_KEYS:
                        total[k] += tally[k]
            fitness = (total["wins"] - total["losses"] - 0.6 * total["timeouts"]) / max(1, total["battles"])
            stages.append({"name": stage, "difficulty": len(stages) + 1, "weight": [0.25, 0.35, 0.4][len(stages)],
                           "fitness": fitness, "details": {"robust_fitness": fitness, "rows": rows, "total": total}})
        return {"fitness": sum(s["weight"] * s["fitness"] for s in stages), "stages": stages}

    def strategic_report(progress, noise):
        stages = []
        for stage in ("easy", "mixed", "objectives"):
            trials = []
            for faction in ("armada", "cortex", "legion"):
                for map_name in ("Synthetic flat 2048", "Center Command"):
                    projects = max(0, round(2 + 5 * progress + rng.uniform(-noise, noise) * 4))
                    trials.append({"map": map_name, "faction": faction, "learner_side": 0,
                                   "fitness": 0.2 + 0.7 * progress + rng.uniform(-noise, noise),
                                   "projects_completed": projects, "units_produced": projects * 2 + rng.randint(0, 3),
                                   "actions_rejected": 0 if progress > 0.35 or rng.random() < 0.6 else rng.randint(5, 40),
                                   "metal_income": 6 + 10 * progress, "energy_income": 30 + 40 * progress,
                                   "metal_overflow": max(0.0, 40 - 80 * progress) + rng.uniform(0, 10),
                                   "energy_overflow": rng.uniform(0, 60), "unmet_energy": rng.uniform(0, 200),
                                   "army_value": 200 + 900 * progress, "economy_value": 300 + 700 * progress,
                                   "spent_metal": 800 + 900 * progress})
            fitness = sum(t["fitness"] for t in trials) / len(trials)
            stages.append({"name": stage, "difficulty": len(stages) + 1, "weight": [0.25, 0.35, 0.4][len(stages)],
                           "fitness": fitness, "details": {"fitness": fitness, "trials": trials}})
        return {"fitness": sum(s["weight"] * s["fitness"] for s in stages), "stages": stages}

    tracks_state, history = [], []
    makers = {"tactical": tactical_report, "strategic": strategic_report}
    champion = {name: makers[name](0.0, 0.05) for name in makers}
    initial = {name: json.loads(json.dumps(champion[name])) for name in makers}
    best_generation = {name: 0 for name in makers}
    for gen in range(1, generations + 1):
        record = {"generation": gen, "tracks": []}
        for name, make in makers.items():
            progress = min(0.85, 0.9 * (1 - math.exp(-gen / 14.0))) * (1.0 if name == "tactical" else 0.8)
            islands = []
            for island, sigma in (("local", 0.04), ("wide", 0.16)):
                level = -0.4 + 1.2 * progress
                spread = 0.05 + (0.3 if island == "wide" else 0.1) * math.exp(-gen / 25.0)
                if gen > 28 and name == "strategic":
                    spread = 0.0           # exercise the "no learning signal" warning
                diag = [{"name": s, "mean": level + rng.uniform(-0.1, 0.1), "std_dev": spread * (0.5 + i * 0.3),
                         "minimum": level - spread, "maximum": level + spread, "unique_fitness": 8 if spread else 1}
                        for i, s in enumerate(("easy", "mixed", "objectives"))]
                entry = {"name": island, "restarted_module": ("mission" if island == "wide" and gen == 31 else None),
                         "optimizer": {"generation": gen, "fitness_mean": level, "fitness_best": level + spread,
                                       "fitness_worst": level - spread,
                                       "update_norm": 0.4 * math.exp(-gen / 20.0) + rng.uniform(0, 0.03)},
                         "candidate_fitness": [level], "candidate_stage_fitness": [[level]] ,
                         "stage_diagnostics": diag, "promoted": [], "center_validation": None, "sample_validation": None}
                if gen % 5 == 0 or gen == 1:
                    report = make(progress * (1.0 if island == "local" else 0.92), 0.12)
                    entry["center_validation"] = report
                    entry["sample_validation"] = make(progress, 0.12)
                    if report["fitness"] > champion[name]["fitness"] + 0.005:
                        champion[name], best_generation[name] = report, gen
                        entry["promoted"] = ["center"]
                islands.append(entry)
            record["tracks"].append({"name": name, "islands": islands,
                                     "champion_fitness": champion[name]["fitness"], "best_generation": best_generation[name]})
        history.append(record)
    holdout = {name: makers[name](0.45 * (1 if name == "tactical" else 0.7), 0.1) for name in makers}
    state = {"format": "minibar-campaign-state", "version": 1, "input_identity": "demo", "backend_identity": "demo",
             "completed_generations": generations, "history": history,
             "tracks": [{"name": name, "best_report": champion[name], "initial_report": initial[name],
                         "holdout": holdout[name], "best_generation": best_generation[name], "archive": [],
                         "islands": [{"name": "local", "last_progress": best_generation[name], "restarts": 0},
                                     {"name": "wide", "last_progress": best_generation[name], "restarts": 1}]}
                        for name in makers]}
    # Synthetic structures make the demo exercise each visible transformation.
    import copy
    for track in state["tracks"]:
        module = {"name": "spacing" if track["name"] == "tactical" else "production",
                  "role": "Advance or back away" if track["name"] == "tactical" else "Choose what to produce",
                  "inputs": ["own_health", "target_distance", "reload_remaining"], "hidden": [3],
                  "outputs": ["advance" if track["name"] == "tactical" else "utility"],
                  "layers": [{"weights": [[0.5, -1.0, 0.2], [0.1, 0.8, -0.3], [1.0, 0.1, 0.4]], "biases": [0., 0., 0.]},
                             {"weights": [[0.8, -0.5, 0.3]], "biases": [0.]}],
                  "structure": {"inputs": [True, True, True], "hidden": [["tanh"] * 3],
                                "connections": [[[True]*3 for _ in range(3)], [[True]*3]]}}
        base_brain = {"format": "minibar-brain", "version": 2, "kind": track["name"], "modules": [module]}
        track.update(base=base_brain, best=base_brain, structure_snapshots={"demo-initial": base_brain},
                     structural_history=[], promotion_history=[])
        parent_id = "demo-initial"
        proposal = None
        pending = None
        attempt = 0
        for generation in history:
            gen = generation["generation"]
            record = next(r for r in generation["tracks"] if r["name"] == track["name"])
            for island in record["islands"]:
                if island["name"] == "wide" and gen in (1, 6, 11):
                    attempt += 1
                    if pending and pending["outcome"] == "training":
                        pending["outcome"] = "superseded_without_evaluation"
                    proposal = copy.deepcopy(track["structure_snapshots"][parent_id])
                    structure = proposal["modules"][0]["structure"]
                    if attempt == 1: structure["hidden"][0][1] = "softsign"
                    elif attempt == 2: structure["inputs"][2] = False
                    else: structure["connections"][0][2][0] = False
                    key = f"demo-proposal-{attempt}"
                    track["structure_snapshots"][key] = proposal
                    pending = {"generation": gen, "island": "wide", "attempt": attempt,
                               "parent_sha256": parent_id, "proposal_sha256": key,
                               "outcome": "training", "accepted_generation": None}
                    track["structural_history"].append(pending)
                for candidate in island.get("promoted", []):
                    trained = copy.deepcopy(proposal if island["name"] == "wide" and proposal else base_brain)
                    trained["modules"][0]["layers"][1]["weights"][0][0] += 0.01 * gen
                    key = f"demo-champion-{gen}-{island['name']}-{candidate}"
                    track["structure_snapshots"][key] = trained
                    track["promotion_history"].append({"generation": gen, "island": island["name"],
                        "candidate": candidate, "parent_sha256": parent_id, "brain_sha256": key})
                    track["best"] = trained
                    parent_id = key
                    if island["name"] == "wide" and pending:
                        pending["outcome"] = "accepted"
                        pending["accepted_generation"] = gen
                if island["name"] == "wide" and pending and pending["outcome"] == "training" and (gen == 1 or gen % 5 == 0):
                    pending["outcome"] = "evaluated_without_promotion"
    with open(os.path.join(run, "state.json"), "w", encoding="utf-8") as handle:
        json.dump(state, handle)
    with open(os.path.join(run, "summary.json"), "w", encoding="utf-8") as handle:
        json.dump({"status": "completed", "completed_generations": generations, "target_generations": generations,
                   "waiting_jobs": 0, "tracks": []}, handle)
    with open(os.path.join(out_dir, "campaign.json"), "w", encoding="utf-8") as handle:
        json.dump({"name": "demo campaign (synthetic)", "generations": generations, "validation_every": 5,
                   "max_stage_regression": 0.15, "min_gain": 0.005, "restart_after": 30, "tracks": []}, handle)
    for name in makers:
        with open(os.path.join(out_dir, f"holdout.{name}.initial.json"), "w", encoding="utf-8") as handle:
            json.dump(makers[name](0.0, 0.05), handle)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("campaign", nargs="?", help="campaign output directory (or its run/ folder)")
    parser.add_argument("--out", help="HTML to write (default: <campaign>/dashboard.html)")
    parser.add_argument("--watch", type=float, metavar="SECONDS", help="keep rewriting the page and make it auto-refresh")
    parser.add_argument("--demo", metavar="DIR", help="write a synthetic 40-generation campaign to DIR and render it")
    args = parser.parse_args()
    source = args.campaign
    if args.demo:
        make_demo(args.demo)
        source = args.demo
    if not source:
        parser.error("give a campaign directory or --demo DIR")
    root, _ = locate(source)
    out = args.out or os.path.join(root, "dashboard.html")
    write_dashboard(source, out, args.watch)
    print(f"wrote {out}")
    while args.watch:
        time.sleep(args.watch)
        try:
            write_dashboard(source, out, args.watch)
        except (OSError, ValueError, SystemExit) as error:
            print(f"skipped a refresh: {error}", file=sys.stderr)


if __name__ == "__main__":
    main()
