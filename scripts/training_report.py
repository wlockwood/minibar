"""Standalone, escaped HTML evidence tables; no browser/network dependency."""
import html
import json

def render(assessment):
    escape=lambda value: html.escape(str(value), quote=True)
    parts=['<!doctype html><html lang="en"><meta charset="utf-8"><title>MiniBAR training comparison</title>',
           '<style>body{font:16px system-ui;margin:36px auto;padding:0 24px;max-width:1200px;color:#18212c;background:#f5f7fa}table{border-collapse:collapse;background:white;width:100%;margin:18px 0}th,td{padding:10px;text-align:left;border-bottom:1px solid #dbe1ea}th{background:#e5ecf5}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#fff;padding:18px}small{color:#526174}</style>',
           '<h1>MiniBAR training comparison</h1>',
           '<p>Measured simulator results. One-generation pilots screen execution and initial signal; they do not establish long-run learning or native BAR strength. Confidence intervals are conditional on the sampled scenarios and models.</p>']
    comparisons=assessment.get("comparisons",[assessment] if "brains" in assessment else [])
    for result in comparisons:
        parts.append('<h2>'+escape(result.get("track","Comparison"))+'</h2>')
        parts.append('<p>Reference: '+escape(result.get("reference",""))+'; paired layout bundles: '+escape(result.get("layouts",""))+'</p>')
        parts.append('<table><tr><th>Brain</th><th>Mean bundle fitness</th><th>Pooled fitness</th><th>Gain</th><th>Paired wins / ties / losses</th><th>Exploratory 95% interval</th></tr>')
        for brain in result.get("brains",[]):
            paired=brain.get("paired",{})
            row=[brain["label"],f'{brain["fitness"]:.6f}',f'{brain.get("pooled_fitness",brain["fitness"]):.6f}',f'{paired.get("mean_gain",0):+.6f}',f'{paired.get("wins",0)} / {paired.get("ties",0)} / {paired.get("losses",0)}',"["+", ".join(f"{v:+.6f}" for v in paired.get("interval_95",[]))+"]"]
            parts.append('<tr>'+''.join('<td>'+escape(v)+'</td>' for v in row)+'</tr>')
        parts.append('</table>')
        for brain in result.get("brains",[]):
            parts.append('<details><summary>'+escape(brain["label"])+': stages, factions, outcomes and reward components</summary><pre>'+escape(json.dumps({"metrics":brain.get("diagnostics",{}),"opponent_roster":brain.get("opponent_diagnostics",{}),"unavailable_dimensions":brain.get("unavailable_dimensions",[])},indent=2))+'</pre></details>')
    if "training_seed_summary" in assessment:
        parts.append('<h2>Independent training seeds</h2><p>Variation across training seeds. Treatment intervals pair matching seeds with legacy-v1. Three seeds support only an exploratory screen.</p><table><tr><th>Track / arm</th><th>Completed seeds</th><th>Mean</th><th>Sample SD</th><th>Range</th><th>Gain vs legacy</th><th>Exploratory 95% interval</th></tr>')
        number=lambda value: f"{value:.6f}" if value is not None else "unavailable"
        interval=lambda values: "["+", ".join(number(v) for v in values)+"]"
        for row in assessment["training_seed_summary"]:
            paired=row.get("versus_legacy",{})
            values=[row["track"]+" / "+row["arm"],str(row["completed_seeds"])+" / "+str(row["planned_seeds"]),number(row["mean"]),number(row["sample_sd"]),interval(row["range"]),number(paired.get("mean_gain")),interval(paired.get("interval_95",[]))]
            parts.append('<tr>'+''.join('<td>'+escape(v)+'</td>' for v in values)+'</tr>')
        parts.append('</table>')
    if "runs" in assessment:
        parts.append('<h2>Every training run</h2><table><tr><th>Arm</th><th>Seed</th><th>Status</th><th>Committed generations</th><th>Total seconds</th><th>Result</th></tr>')
        for run in assessment["runs"]:
            parts.append('<tr>'+''.join('<td>'+escape(run.get("total_elapsed_seconds",run.get("elapsed_seconds","")) if k=="elapsed_seconds" else run.get(k,""))+'</td>' for k in ["arm","seed","status","completed_generations","elapsed_seconds","error"])+'</tr>')
        parts.append('</table>')
    parts.append('<h2>Provenance and interpretation</h2><pre>'+escape(json.dumps({k:v for k,v in assessment.items() if k not in ("comparisons","brains","runs")},indent=2))+'</pre></html>')
    return '\n'.join(parts)
