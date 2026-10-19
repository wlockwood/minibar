# Training progress dashboard

`scripts/campaign_dashboard.py` turns a campaign output folder into one self-contained HTML page that answers
"is training making progress?". It is read-only (it never writes into the campaign directory), uses only the Python
standard library, and the page needs no network. Template: `viewer/campaign_dashboard.template.html`.

```powershell
python scripts/campaign_dashboard.py data/brains/campaign            # writes data/brains/campaign/dashboard.html
python scripts/campaign_dashboard.py data/brains/campaign --watch 20 # live: rewrites every 20 s, page reloads itself
python scripts/campaign_dashboard.py --demo some/dir                 # synthetic 40-generation run, for a preview
```

Open the file in a browser. `?theme=light|dark` forces a theme; `#tactical/wide/sample` selects a track, island and
candidate kind. Every chart has a "Table view"; arrow keys step through a focused line chart.

## What each panel answers

| Panel | Question | Where the numbers come from |
| --- | --- | --- |
| Verdict banner | In one sentence: better, within noise, stalled, worse? | champion vs starting brain, 95% Wilson win-rate intervals, holdout vs its starting score, generations since the last champion |
| Fresh-seed retest (tactical, optional) | Is the champion (or an island's latest brain) really better than the start on new seeds, and on which scenario family? | `fresh-retest.<track>.json` beside `campaign.json`, written by `scripts/fresh_retest.py` |
| Tiles | Generation, champion fitness (+sparkline), win rate / projects, holdout, last champion, optimizer step size | `state.json`, `summary.json` |
| Is the champion getting better? | Champion step line vs each island's center and best-sample validation, with the starting brain as a baseline | `history` records, `initial_report` |
| Do the wins show it? | Win / draw / timeout / loss share per validated generation (tactical); economy per game (strategic) | validation tallies and trials |
| Is the search alive? | Candidate fitness mean and worst-to-best band, optimizer step size | `optimizer` block |
| Is it forgetting anything? | Validation fitness per curriculum stage | `center_validation.stages` |
| Where is it winning / stuck? | Win rate by scenario x opponent now, and change since the untrained brain | `best_report`, `initial_report` rows |
| What has the brain changed? | Per labelled module, how far the champion's weights moved; which input sensors moved most | `base` vs `best` brain in `state.json` |
| Learning signal | Spread of candidate fitness per stage; flags a stage where every candidate scored the same | `stage_diagnostics` |
| What happened? | New champions, restarts, stalls (10+ generations), timeouts above 25%, rejected build orders, forgetting beyond `max_stage_regression`, validation up but holdout down | derived |

## Fresh-seed retest

The fixed validation set is scored again every validation round, so a champion's stored score contains
selection luck (the best of many noisy readings) and a later, equally good brain can look worse. With
`fresh_screening` on, promotion no longer uses that comparison, which makes the "champion validation vs start"
sentence misleading. `scripts/fresh_retest.py <campaign root> --seeds 150` re-scores the starting brain, the
champion and each island's latest brain on new seed bases, paired across brains, for three scenario families:
training (what the optimizer fits), validation, and the holdout. It writes `fresh-retest.<track>.json` next to
`campaign.json` (never inside `run/`). When that file exists the dashboard leads the verdict with it, uses
|t| >= 2 as "real", adds the retest card, and flags "gains on validation but loses on the holdout" as overfitting.
It needs the simulator build that produced the run (default `<campaign root>/bin/visigoth-simulator.exe`, or
`--exe`); the training family is evaluated through a temporary copy of the config that is removed afterwards.

Reading it honestly: validation cells hold only a few battles each, so the page shows intervals and calls an
overlapping win-rate change "within noise". Weight movement is not improvement, especially in the first generations.
Fitness is a weighted raw objective whose scale differs by stage and map.

## Not shown, because the campaign does not record it

* Wall time per generation, evaluation counts, ETA. (Watch mode could timestamp generations; it does not yet.)
* The champion's per-scenario report at each promotion; only the latest champion's and the starting brain's are kept,
  so scenario heatmaps compare "now" with "start", not with every generation.
* Per-generation validation seed counts (needed for tighter intervals on the fitness lines).
* Holdout score per generation (it is evaluated at the end and for the initial brain only).

Each of these is a small addition to the history record; they are Rust-side changes (`campaign.rs`) and were left to
its owner. The page ignores fields it does not know, so it keeps working as the record grows.

Tests: `python -m pytest scripts/test_campaign_dashboard.py` (extraction, events, verdict wording, hostile text,
atomic write, JS syntax). Ready-made pages: `data/dashboards/`.
