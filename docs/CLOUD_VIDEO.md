# Cloud video of a training run

`scripts/cloud_video.py` turns a run into an MP4 in which every scene is one checkpoint (one generation) and
shows many battles of that checkpoint on top of each other. The middle of the cloud is what the brain usually
does, its width is how much that varies between seeds, and the trend across scenes is training progress.

```powershell
$b = "data/branches/diverse-training-20261018"
python scripts/cloud_video.py `
    --run $b/runs/production --track ground --island local `
    --catalog $b/catalog/catalog.json `
    --train-config $b/stages/production/ground-heavy_swarm.json `
    --exe $b/bin/visigoth-simulator.exe `
    --out cloud.mp4
```

Needs Python with numpy and Pillow, and ffmpeg on PATH (or `--ffmpeg PATH`). Nothing else is installed or changed.

## What a scene is

The same scenario and opponent, one brain, `--samples` battles (default 48) that differ only in seed (layout
jitter and simulation noise). Blue is the brain's side, red the opponent. Each unit is a translucent blob, so
overlapping ghosts saturate; a short trail shows where they came from. Dead units disappear. The camera is one
fixed frame fitted to all scenes, so scenes are comparable. Scenes play at `--speed` simulated seconds per
video second (default 4) and hold their last frame for `--hold` seconds.

Corner labels, on every scene:

| Corner | Text |
| --- | --- |
| upper left | `Generation 5` (generation 0 is the track's initial brain) |
| lower left | `Fitness range -0.55 to -0.50`: lowest and highest fitness of the battles on screen. When `history.jsonl` records that generation, a second line gives the ES population's range and mean |
| lower right | `Sim time 14.3 s` (this scene) and `video total 1:26` (simulated time since the video began) |
| upper right | outcome tally of the battles on screen (`--no-outcomes` removes it) |

Two unit lists flank the map: **Council units** (blue) on the left and **Red units** (red) on the right. Each
line is one unit type with its starting count (`40x Pawns`, `1x Mammoth`), the most valuable line first. A line's
value is count x (metal + energy / 60), the same formula the matchup generator uses, with costs and display names
from the catalog. A dim `total value` line closes each list, and a list too long for the screen ends with
`+ N more types`. The map is fitted into the space between the lists. `--no-rosters` removes them and gives the
map the full width.

## Progressive mode: evolution as a stream

`--mode progressive` replaces one-scene-per-generation with a single continuous cloud.

1. For every checkpoint, `--pool` battles are played (default 2 x `--top`) and the best `--top` by fitness are
   kept (default 50). `--select random` keeps an unbiased sample instead; `best` shows each generation at its
   best, so use `random` when you want what it typically does.
2. The kept runs join one queue, generation by generation (inside a generation they go in seed order, so the
   best are not always first).
3. Every `--interval` seconds of video (default 1) the next `--per-second` runs (default 5) leave the front of the
   queue and start playing, each on its own clock, overlaid in the same frame. A finished run fades out over
   `--fade` seconds instead of freezing.

By default the launches are spread evenly (`--launch stream`: one run every interval / per-second, so every 0.2 s
at 5 per second), so runs arrive and leave one at a time. `--launch burst` starts each batch of 5 on the same frame;
it has the same average rate but pulses once per interval (measured on rendered frames, the 1 Hz component of the
frame-to-frame change was about 9 times the typical component with bursts and absent with the stream).

A battle that takes 24 simulated seconds takes 6 video seconds at 4x, so 5 runs per second means about 30 runs
on screen at once, and the cloud slides from early generations to late ones. `--speed` changes the run length and
therefore the crowd size.

```powershell
python scripts/cloud_video.py --mode progressive --run $b/runs/production --track ground --island local `
    --catalog $b/catalog/catalog.json --train-config $b/stages/production/ground-heavy_swarm.json --out evolution.mp4
```

**Video length is `generations x top / per-second x interval` seconds, plus one run's length.** At the defaults a
generation takes 10 s, so 4 checkpoints are 46 s but 200 generations would be 33 minutes. Shorten it with a smaller
`--top` (10 to 15 gives 2 to 3 s per generation), a larger `--per-second` (which also makes the cloud denser), or
`--max-scenes` to thin the generations log-spaced. The script prints the length before it starts. With only a
handful of checkpoints the cloud changes in steps; a smooth progression needs a brain saved every generation.

### How the readouts change

Nothing is static any more, so each readout describes what is on screen at that moment:

| Corner | Scenes mode | Progressive mode |
| --- | --- | --- |
| upper left | the scene's generation | the newest generation to have left the queue, and `showing generations 0 to 5` while older runs are still playing |
| lower left | fitness range of the scene's battles | range and mean of the runs playing now, with a count; the newest generation's ES population (when recorded); and the selection (`top 50 of 100 runs per generation`), because a best-of selection is not a typical run |
| lower right | sim time of this scene, and of the whole video | `Sim time 2:47`: simulated time since the video began (video seconds x speed), and `run 135 of 200 started` as progress through the queue |
| upper right | outcome tally of the scene | outcome tally of the runs playing now |
| side lists | the roster | the same roster (one scenario throughout) |

The fitness range is the readout that carries the trend: it moves as better generations replace older ones. The ES
population line is on a different scale (it aggregates every training stage), which is why the label says so.

## Choosing what to show

* `--list` prints the config's scenarios and opponents; pick with `--scenario N --opponent M`. Opponent 0 is
  usually the `charge` baseline.
* `--generations 0,5,10-40` and `--max-scenes 12` (log-spaced, first and last kept) limit the scenes.
* `--kind sample` or `module_spacing` shows other retained candidate kinds; `--island` is required when a track has
  several islands with checkpoints.
* Not a campaign? `--brains a.json b.json ... [--labels "Start" "Night 1" ...]` shows any brain files in order
  (evolve.py hall of fame, hand-picked champions). Files named `generation-N...` are labelled automatically.
* Looks: `--size 1920x1080`, `--fps`, `--glow`, `--dot-scale`, `--trail`, `--view x0,z0,x1,z1`, `--crf`.
* `--stills DIR` writes one PNG per scene; with `--stills-only` it stops there. Use it to tune the look before
  encoding.

## Cost and caching

Battles come from the simulator's own `--watch-battle` (about 40 ms for a 41-unit battle), run in parallel
(`--jobs`). 192 battles took 5 s; 816 frames of 1280x720 took about 30 s to render.

**Temporary files.** Played battles are stored as one small `.npz` file each (hundreds per video), plus the brains
being played. By default they live in a fresh folder in the system temp directory that is removed when the run
ends, whether it finishes, fails or is interrupted, so nothing accumulates and nothing is written next to the
video or inside a repository. A run that fails also deletes its half-written video.

To make re-renders fast, name a folder with `--cache DIR`. It is kept, and later runs reuse any battle whose
brain, catalog, config (and its companion brain), simulator binary, scenario, opponent, side and seed are
unchanged, so changing the look, speed or labels re-renders without simulating. `--fresh` replays instead. Keep that
folder outside the repository, or ignore it in git: it is bulky and regenerable.

## Honest limits

* Seeds default to small integers, which training and validation never use, so the cloud shows battles the
  optimizer did not fit. Scenario 0 of a stage is still a scenario it trained on; use a holdout stage config
  (`stages/holdout/`) to show generalisation.
* The campaign keeps center/sample brains only at validation generations (1, 5, 10, ... in the current run), so
  a scene exists only for those. A scene per generation needs the campaign to save each generation's brain
  (about 19 KB each; today's candidate files are about 8 MB because they embed reports).
* `timeout 100%` can be the scenario, not the brain. Every stage config caps a battle at `max_time` 24 s (pilot: 6 s).
  In the Mammoth-versus-40-Pawns scenario the Mammoth wins steadily but needs about 48 s to kill all 40 Pawns, so
  every battle times out at 24 s with 16 to 18 Pawns dead. See the `timeout` outcome in
  [src/learn/objective.rs](../src/learn/objective.rs): an even-value matchup that times out scores a penalty, not a win.
* The cloud only shows change if behaviour changed. Early in a run, scenes can look almost identical; that is
  the finding, not a rendering fault.

Tests: `python -m pytest scripts/test_cloud_video.py` (checkpoint discovery, history, battle parsing and cache,
scene stacking, drawing, all four labels, the unit lists and their sort order, progressive-mode selection,
queue schedule, fading and readouts, simulator caching with a stubbed process, and an ffmpeg end-to-end).
