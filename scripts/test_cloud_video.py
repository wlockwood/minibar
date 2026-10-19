import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cloud_video as cv  # noqa: E402


def battle_document(seed=1, outcome="win", fitness=0.25, frames=11, learner_side=0, dead_after=None):
    """A minimal `--watch-battle` document: one learner unit walks right, one enemy stands still."""
    out = []
    for k in range(frames):
        enemy_hp = 0.0 if dead_after is not None and k > dead_after else 100.0
        out.append({"tick": 3 * k, "time": round(k * 0.1, 3),
                    "u": [[1, 10.0 * k, 5.0, 0.0, 370.0], [2, 200.0, 5.0, 3.14, enemy_hp]],
                    "w": [], "f": [], "d": []})
    return {"scenario": 0, "opponent": 0, "seed": seed, "learner_side": learner_side, "outcome": outcome,
            "fitness": fitness, "exchange": 0.0, "range": 100.0, "sample_every_ticks": 3,
            "units": [{"id": 1, "side": learner_side, "name": "corsumo", "max_hp": 370.0, "radius": 10.0},
                      {"id": 2, "side": 1 - learner_side, "name": "armpw", "max_hp": 100.0, "radius": 10.0}],
            "frames": out}


def checkpoint(label="Generation 1", generation=1, population=None):
    return cv.Checkpoint(label, Path("x.json"), generation, population)


def scene_of(count=4, **kwargs):
    battles = [cv.parse_battle(battle_document(seed=s, **kwargs)) for s in range(count)]
    return cv.build_scene(checkpoint(), battles)


class CheckpointDiscoveryTest(unittest.TestCase):
    def make_run(self, directory):
        run = Path(directory) / "run"
        candidates = run / "candidates" / "ground"
        candidates.mkdir(parents=True)
        for name in ("generation-1-local-center", "generation-10-local-center", "generation-5-local-center",
                     "generation-5-local-sample", "generation-5-local-module_spacing",
                     "generation-5-structural-center"):
            (candidates / f"{name}.json").write_text(json.dumps({"brain": {"modules": []}}), encoding="utf-8")
        (run / "tracks" / "ground").mkdir(parents=True)
        (run / "tracks" / "ground" / "brain.initial.json").write_text('{"modules": []}', encoding="utf-8")
        return run

    def test_orders_generations_numerically_and_adds_the_initial_brain(self):
        with tempfile.TemporaryDirectory() as d:
            found, island = cv.discover_run(self.make_run(d), "ground", "local")
            self.assertEqual(island, "local")
            self.assertEqual([c.generation for c in found], [0, 1, 5, 10])
            self.assertEqual([c.label for c in found],
                             ["Generation 0 (start)", "Generation 1", "Generation 5", "Generation 10"])

    def test_kind_filter_ignores_samples_and_module_candidates(self):
        with tempfile.TemporaryDirectory() as d:
            run = self.make_run(d)
            found, _ = cv.discover_run(run, "ground", "local", kind="sample", include_initial=False)
            self.assertEqual([c.generation for c in found], [5])
            found, _ = cv.discover_run(run, "ground", "local", kind="module_spacing", include_initial=False)
            self.assertEqual([c.generation for c in found], [5])

    def test_several_islands_must_be_chosen(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(ValueError, "pass --island"):
                cv.discover_run(self.make_run(d), "ground")

    def test_unknown_island_names_what_exists(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(ValueError, "has no 'center' checkpoints"):
                cv.discover_run(self.make_run(d), "ground", "nope")

    def test_receipt_and_bare_brain_files_both_load(self):
        with tempfile.TemporaryDirectory() as d:
            bare, receipt = Path(d) / "a.json", Path(d) / "b.json"
            bare.write_text('{"modules": [1]}', encoding="utf-8")
            receipt.write_text('{"format": "r", "brain": {"modules": [2]}}', encoding="utf-8")
            self.assertEqual(cv.Checkpoint("a", bare).load_brain()["modules"], [1])
            self.assertEqual(cv.Checkpoint("b", receipt).load_brain()["modules"], [2])
            bad = Path(d) / "c.json"
            bad.write_text('{"nothing": 1}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "not a brain file"):
                cv.Checkpoint("c", bad).load_brain()

    def test_explicit_brain_files_get_generation_labels_from_their_names(self):
        found = cv.checkpoints_from_files(["a/generation-12.json", "b/champion.json", "c/x.json"], ["", "Best"])
        self.assertEqual([c.label for c in found], ["", "Best", "x"])
        found = cv.checkpoints_from_files(["a/generation_12.json", "b/champion.json"])
        self.assertEqual([c.label for c in found], ["Generation 12", "champion"])
        self.assertEqual([c.generation for c in found], [12, None])


class HistoryTest(unittest.TestCase):
    def test_population_range_for_one_island_of_one_generation(self):
        history = [{"generation": 2, "tracks": [
            {"name": "ground", "islands": [{"name": "local", "candidate_fitness": [-0.3, -0.1, -0.2]},
                                            {"name": "other", "candidate_fitness": [5.0]}]},
            {"name": "air", "islands": [{"name": "local", "candidate_fitness": [9.0]}]}]}]
        low, high, mean, count = cv.population_range(history, 2, "ground", "local")
        self.assertEqual((low, high, count), (-0.3, -0.1, 3))
        self.assertAlmostEqual(mean, -0.2)
        self.assertIsNone(cv.population_range(history, 3, "ground", "local"))
        self.assertIsNone(cv.population_range(history, 2, "ground", "missing"))

    def test_unfinished_last_line_of_a_live_run_is_ignored(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "history.jsonl"
            path.write_text('{"generation": 1}\n{"generation": 2, "tra', encoding="utf-8")
            self.assertEqual(cv.read_history(path), [{"generation": 1}])
            self.assertEqual(cv.read_history(Path(d) / "missing.jsonl"), [])


class SelectionTest(unittest.TestCase):
    def test_generation_lists_and_ranges(self):
        self.assertEqual(cv.parse_generations("1, 5,10-12"), {1, 5, 10, 11, 12})

    def test_log_spacing_keeps_ends_and_bunches_early(self):
        items = list(range(100))
        picked = cv.thin_log_spaced(items, 10)
        self.assertEqual(len(picked), 10)
        self.assertEqual((picked[0], picked[-1]), (0, 99))
        self.assertLess(picked[4], 15)
        self.assertEqual(cv.thin_log_spaced(items, 500), items)
        self.assertEqual(cv.thin_log_spaced(items, 1), [99])
        self.assertEqual(len(cv.thin_log_spaced(list(range(5)), 4)), 4)


class BattleTest(unittest.TestCase):
    def test_parse_orders_units_by_id_and_marks_the_dead(self):
        b = cv.parse_battle(battle_document(dead_after=4, learner_side=1))
        self.assertEqual(b.pos.shape, (11, 2, 2))
        self.assertAlmostEqual(b.dt, 0.1)
        self.assertAlmostEqual(b.duration, 1.0)
        self.assertEqual(b.learner.tolist(), [True, False])
        self.assertTrue(b.alive[4, 1])
        self.assertFalse(b.alive[5, 1])
        self.assertTrue(b.alive[10, 0])
        self.assertEqual(b.pos[3, 0].tolist(), [30.0, 5.0])

    def test_cache_round_trip_keeps_everything(self):
        b = cv.parse_battle(battle_document(seed=7, outcome="loss", fitness=-0.5))
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "deep" / "b.npz"
            cv.save_battle(path, b)
            back = cv.load_battle(path)
        self.assertEqual((back.seed, back.outcome, back.fitness), (7, "loss", -0.5))
        self.assertTrue(np.array_equal(back.pos, b.pos) and np.array_equal(back.alive, b.alive))
        self.assertTrue(np.array_equal(back.learner, b.learner))
        self.assertEqual(back.names, ("corsumo", "armpw"))

    def test_empty_battle_is_an_error(self):
        doc = battle_document()
        doc["frames"] = []
        with self.assertRaises(ValueError):
            cv.parse_battle(doc)


class SceneTest(unittest.TestCase):
    def test_shorter_battles_hold_their_last_frame(self):
        long_b = cv.parse_battle(battle_document(seed=1, frames=21))
        short_b = cv.parse_battle(battle_document(seed=2, frames=6, dead_after=2))
        scene = cv.build_scene(checkpoint(), [long_b, short_b])
        self.assertEqual(scene.pos.shape, (2, 21, 2, 2))
        self.assertAlmostEqual(scene.sim_seconds, 2.0)
        self.assertEqual(scene.pos[1, 20, 0].tolist(), short_b.pos[-1, 0].tolist())
        self.assertFalse(scene.alive[1, 20, 1])

    def test_max_sim_seconds_cuts_the_scene(self):
        scene = cv.build_scene(checkpoint(), [cv.parse_battle(battle_document(frames=51))], max_sim_seconds=2.0)
        self.assertAlmostEqual(scene.sim_seconds, 2.0)

    def test_view_fits_the_live_units_inside_the_margins(self):
        scene = scene_of()
        view = cv.fit_view([scene], 640, 360, 20, pad=0.0, clip=0.0)
        px, py = view.to_pixels(np.array([[0.0, 5.0], [200.0, 5.0]]), 640, 360)
        self.assertGreaterEqual(px.min(), 19.9)
        self.assertLessEqual(px.max(), 620.1)
        self.assertAlmostEqual(py[0], 180.0, places=3)  # centred vertically

    def test_play_plan_accumulates_simulated_time(self):
        a, b = scene_of(), scene_of()
        total = cv.plan_scenes([a, b], speed=2.0, hold_seconds=0.5, fps=30)
        self.assertAlmostEqual(a.video_seconds, 1.0 / 2.0 + 0.5)
        self.assertAlmostEqual(b.sim_before, a.sim_seconds)
        self.assertAlmostEqual(total, a.video_seconds + b.video_seconds)


class RendererTest(unittest.TestCase):
    def renderer(self, scene, **kwargs):
        view = cv.fit_view([scene], 320, 180, 10, pad=0.0, clip=0.0)
        return cv.Renderer(320, 180, view, 30, 1.0, trail_seconds=0.0, **kwargs)

    def test_learner_is_blue_and_enemy_is_red_where_they_stand(self):
        scene = scene_of(count=3)
        r = self.renderer(scene)
        frame = r.draw(scene, 0.0)
        ex, ey = r.view.to_pixels(np.array([200.0, 5.0]), 320, 180)
        lx, ly = r.view.to_pixels(np.array([0.0, 5.0]), 320, 180)
        enemy = frame[int(round(ey)), int(round(ex))]
        learner = frame[int(round(ly)), min(int(round(lx)) + 2, 319)]
        self.assertGreater(enemy[0], enemy[2] + 30)
        self.assertGreater(learner[2], learner[0] + 30)

    def test_dead_units_leave_the_picture(self):
        scene = scene_of(count=3, dead_after=2)
        r = self.renderer(scene)
        ex, ey = r.view.to_pixels(np.array([200.0, 5.0]), 320, 180)
        before = r.draw(scene, 0.2)[int(round(ey)), int(round(ex))]
        after = r.draw(scene, 1.0)[int(round(ey)), int(round(ex))]
        self.assertGreater(before[0], after[0] + 30)

    def test_a_crowd_is_brighter_than_a_lone_ghost(self):
        scene = scene_of(count=16)
        lone = cv.build_scene(checkpoint(), [cv.parse_battle(battle_document())])
        crowd_px = self.renderer(scene)
        lone_px = self.renderer(lone)
        ex, ey = crowd_px.view.to_pixels(np.array([200.0, 5.0]), 320, 180)
        a = crowd_px.draw(scene, 0.0)[int(round(ey)), int(round(ex)), 0]
        ex2, ey2 = lone_px.view.to_pixels(np.array([200.0, 5.0]), 320, 180)
        b = lone_px.draw(lone, 0.0)[int(round(ey2)), int(round(ex2)), 0]
        self.assertGreater(a, 100)
        self.assertGreater(a, 0.9 * b - 1)  # never dimmer than one ghost at the same opacity

    def test_time_beyond_the_end_holds_the_last_frame(self):
        scene = scene_of()
        r = self.renderer(scene)
        a = r.draw(scene, scene.sim_seconds)
        b = r.draw(scene, scene.sim_seconds + 50)
        self.assertTrue(np.array_equal(a, b))


class LabelTest(unittest.TestCase):
    def test_every_corner_carries_the_requested_text(self):
        scene = scene_of(count=4, outcome="win", fitness=0.25)
        scene.checkpoint = checkpoint("Generation 7", 7, population=(-0.3, -0.1, -0.2, 32))
        cv.plan_scenes([scene], 4.0, 0.0, 30)
        scene.sim_before = 83.0
        ul, ll, lr, ur = cv.corner_text(scene, 12.34)
        self.assertEqual(ul[0][0], "Generation 7")
        self.assertIn("Fitness range", ll[0][0])
        self.assertIn("+0.25", ll[0][0])
        self.assertTrue(any("ES population, all stages (32)" in text and "-0.300" in text for text, _ in ll))
        self.assertIn("12.3 s", lr[0][0])
        self.assertIn("1:35", lr[1][0])  # 83 + 12.3 simulated seconds into the video
        self.assertIn("win 100%", ur[0][0])

    def test_no_population_line_without_history_and_outcomes_can_be_hidden(self):
        scene = scene_of()
        _, ll, _, ur = cv.corner_text(scene, 0.0, show_outcomes=False)
        self.assertFalse(any("population" in t for t, _ in ll))
        self.assertEqual(ur, [])

    def test_fitness_range_spans_the_battles_on_screen(self):
        self.assertEqual(cv.fitness_text(np.array([-0.55, -0.5, 0.2])), "-0.55  to  +0.20")

    def test_clock_formats(self):
        self.assertEqual(cv.format_clock(0), "0:00")
        self.assertEqual(cv.format_clock(3599.9), "59:59")

    def test_drawing_labels_changes_pixels_in_all_four_corners(self):
        scene = scene_of(count=4)
        labels = cv.Labels(640, 360)
        blank = cv.Image.new("RGB", (640, 360), cv.BACKGROUND_RGB)
        image = labels.draw(blank.copy(), *cv.corner_text(scene, 3.0))
        diff = np.abs(np.asarray(image, np.int16) - np.asarray(blank, np.int16)).sum(axis=2) > 0
        for ys, xs in ((slice(0, 120), slice(0, 320)), (slice(0, 120), slice(320, 640)),
                       (slice(240, 360), slice(0, 320)), (slice(240, 360), slice(320, 640))):
            self.assertTrue(diff[ys, xs].any())


def unit_info():
    return {"corsumo": cv.UnitInfo("Mammoth", 2200 + 35000 / 60), "armpw": cv.UnitInfo("Pawn", 54 + 900 / 60),
            "armham": cv.UnitInfo("Mace", 90 + 3700 / 60), "corak": cv.UnitInfo("Grunt", 30 + 1600 / 60)}


class RosterTest(unittest.TestCase):
    def test_catalog_costs_use_metal_plus_energy_over_sixty(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "catalog.json"
            path.write_text(json.dumps({"unit_catalog": {
                "corsumo": {"human_name": "Mammoth", "metal_cost": 2200.0, "energy_cost": 35000.0},
                "nameless": {"metal_cost": 10.0}, "broken": 5}}), encoding="utf-8")
            info = cv.load_unit_info(path)
            self.assertAlmostEqual(info["corsumo"].value, 2783.3333, places=3)
            self.assertEqual(info["corsumo"].name, "Mammoth")
            self.assertEqual((info["nameless"].name, info["nameless"].value), ("nameless", 10.0))
            self.assertNotIn("broken", info)
            self.assertEqual(cv.load_unit_info(Path(d) / "missing.json"), {})
            path.write_text("{not json", encoding="utf-8")
            self.assertEqual(cv.load_unit_info(path), {})

    def test_plurals(self):
        self.assertEqual([cv.pluralise(n) for n in ("Pawn", "Thug", "Mace", "Bass", "Fox", "Pony")],
                         ["Pawns", "Thugs", "Maces", "Basses", "Foxes", "Ponies"])

    def test_lines_are_counted_per_type_and_sorted_by_total_value(self):
        names = ["armpw"] * 6 + ["armham"] * 6 + ["corak"] * 11 + ["corsumo"]
        roster = cv.build_roster(names, [True] * len(names), unit_info())
        values = [6 * 69.0, 6 * (90 + 3700 / 60), 11 * (30 + 1600 / 60), 2200 + 35000 / 60]
        expected = sorted(zip(values, ["6x Pawns", "6x Maces", "11x Grunts", "1x Mammoth"]), reverse=True)
        self.assertEqual(roster.lines, [text for _, text in expected])
        self.assertEqual(roster.lines[0], "1x Mammoth")
        self.assertEqual(roster.values, sorted(roster.values, reverse=True))
        self.assertAlmostEqual(roster.total, sum(values), places=6)

    def test_a_cheap_but_numerous_type_can_lead(self):
        names = ["armpw"] * 40 + ["armham"]
        roster = cv.build_roster(names, [True] * 41, unit_info())
        self.assertEqual(roster.lines, ["40x Pawns", "1x Mace"])

    def test_only_the_flagged_side_is_listed_and_singular_has_no_s(self):
        names = ["corsumo", "armpw", "armpw"]
        mine = cv.build_roster(names, [True, False, False], unit_info())
        theirs = cv.build_roster(names, [False, True, True], unit_info())
        self.assertEqual((mine.lines, theirs.lines), (["1x Mammoth"], ["2x Pawns"]))

    def test_unknown_units_fall_back_to_their_definition_name_and_sort_by_count(self):
        roster = cv.build_roster(["mmm", "nnn", "nnn"], [True] * 3, {})
        self.assertEqual(roster.lines, ["2x nnns", "1x mmm"])
        self.assertEqual(roster.total, 0.0)

    def test_scene_carries_both_rosters(self):
        scene = cv.build_scene(checkpoint(), [cv.parse_battle(battle_document())], unit_info=unit_info())
        self.assertEqual(scene.rosters[0].lines, ["1x Mammoth"])
        self.assertEqual(scene.rosters[1].lines, ["1x Pawn"])
        panels = cv.roster_panels(scene)
        self.assertEqual([p["title"] for p in panels], ["Council units", "Red units"])
        self.assertEqual(panels[0]["footer"], "total value 2,783")

    def test_scene_without_unit_names_has_empty_lists(self):
        b = cv.parse_battle(battle_document())
        b.names = ()
        scene = cv.build_scene(checkpoint(), [b])
        self.assertEqual(scene.rosters[0].lines, [])
        self.assertEqual(cv.roster_panels(scene)[0]["footer"], "")


class PanelDrawingTest(unittest.TestCase):
    def scene(self):
        return cv.build_scene(checkpoint(), [cv.parse_battle(battle_document())], unit_info=unit_info())

    def changed(self, labels, scene, region):
        blank = cv.Image.new("RGB", (labels.width, labels.height), cv.BACKGROUND_RGB)
        image = labels.draw(blank.copy(), [], [], [], [], panels=cv.roster_panels(scene))
        diff = np.abs(np.asarray(image, np.int16) - np.asarray(blank, np.int16)).sum(axis=2) > 0
        return bool(diff[region].any())

    def test_lists_sit_on_the_left_and_right_and_the_map_keeps_clear_of_them(self):
        scene = self.scene()
        labels = cv.Labels(1280, 720, [scene])
        left, top, right, bottom = labels.reserved_margins()
        self.assertGreater(labels.panel_width, 100)
        self.assertEqual(left, right)
        self.assertGreater(left, labels.margin + labels.panel_width)
        self.assertGreater(bottom, top)
        width = labels.width
        self.assertTrue(self.changed(labels, scene, (slice(60, 300), slice(0, labels.margin + labels.panel_width))))
        self.assertTrue(self.changed(labels, scene,
                                     (slice(60, 300), slice(width - labels.margin - labels.panel_width, width))))
        self.assertFalse(self.changed(labels, scene, (slice(0, 720), slice(left + 5, width - right - 5))))

    def test_map_is_fitted_inside_the_reserved_area(self):
        scene = self.scene()
        labels = cv.Labels(640, 360, [scene])
        left, top, right, bottom = labels.reserved_margins()
        view = cv.fit_view([scene], 640, 360, labels.reserved_margins(), pad=0.0, clip=0.0)
        px, py = view.to_pixels(np.array([[0.0, 5.0], [200.0, 5.0]]), 640, 360)
        self.assertGreaterEqual(px.min(), left - 0.1)
        self.assertLessEqual(px.max(), 640 - right + 0.1)
        self.assertGreaterEqual(py.min(), top)
        self.assertLessEqual(py.max(), 360 - bottom)

    def test_renderer_grid_works_with_an_off_centre_map(self):
        scene = self.scene()
        labels = cv.Labels(320, 180, [scene])
        view = cv.fit_view([scene], 320, 180, labels.reserved_margins())
        renderer = cv.Renderer(320, 180, view, 30, 1.0)
        self.assertEqual(renderer.draw(scene, 0.0).shape, (180, 320, 3))

    def test_no_rosters_means_no_panels_and_no_reserved_sides(self):
        scene = self.scene()
        labels = cv.Labels(640, 360, [scene], rosters=False)
        self.assertEqual(labels.panel_width, 0)
        self.assertEqual(labels.reserved_margins()[0], labels.margin)
        self.assertFalse(self.changed(labels, scene, (slice(0, 360), slice(0, 640))))

    def test_both_lists_share_one_width_across_scenes(self):
        small = self.scene()
        big = cv.build_scene(checkpoint(), [cv.parse_battle(battle_document())],
                             unit_info={"corsumo": cv.UnitInfo("Extremely Long Unit Name", 1.0),
                                        "armpw": cv.UnitInfo("P", 1.0)})
        alone = cv.Labels(1280, 720, [small])
        both = cv.Labels(1280, 720, [small, big])
        self.assertGreater(both.panel_width, alone.panel_width)
        self.assertLessEqual(both.panel_width, 0.26 * 1280)

    def test_a_long_list_is_cut_with_a_count_of_what_is_hidden(self):
        scene = self.scene()
        scene.rosters = (cv.Roster([f"{i + 1}x Type{i}" for i in range(40)], [0.0] * 40, 0.0), scene.rosters[1])
        labels = cv.Labels(640, 360, [scene])
        blank = cv.Image.new("RGB", (640, 360), cv.BACKGROUND_RGB)
        image = labels.draw(blank.copy(), [], [], [], [], panels=cv.roster_panels(scene))
        diff = np.abs(np.asarray(image, np.int16) - np.asarray(blank, np.int16)).sum(axis=2) > 0
        lowest = int(np.where(diff[:, :labels.margin + labels.panel_width].any(axis=1))[0].max())
        self.assertLess(lowest, 360 - labels.margin - labels.bottom_reserve + labels.line_height)


class SimulatorTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        root = Path(self.dir.name)
        self.exe, self.catalog, self.config = root / "sim.exe", root / "catalog.json", root / "config.json"
        self.exe.write_bytes(b"x")
        self.catalog.write_text("{}", encoding="utf-8")
        self.config.write_text("{}", encoding="utf-8")
        self.cache = root / "cache"

    def tearDown(self):
        self.dir.cleanup()

    def simulator(self, **kwargs):
        return cv.Simulator(self.exe, self.catalog, self.config, self.cache, **kwargs)

    def fake_run(self, calls):
        def run(cmd, **_):
            calls.append(cmd)
            seed = int(cmd[cmd.index("--watch-seed") + 1])
            return type("Done", (), {"returncode": 0, "stdout": json.dumps(battle_document(seed=seed)), "stderr": ""})()
        return run

    def test_battles_are_cached_by_brain_and_seed(self):
        calls = []
        sim = self.simulator(scenario=2, opponent=1, unit="armpw")
        digest, path = sim.stage_brain({"modules": [1]})
        with patch.object(cv.subprocess, "run", self.fake_run(calls)):
            first = sim.battle(digest, path, 5)
            again = sim.battle(digest, path, 5)
            sim.battle(digest, path, 6)
        self.assertEqual(len(calls), 2)
        self.assertEqual(first.seed, again.seed)
        cmd = calls[0]
        for flag, value in (("--watch-scenario", "2"), ("--watch-opponent", "1"), ("--watch-seed", "5"),
                            ("--watch-side", "0"), ("--unit", "armpw")):
            self.assertEqual(cmd[cmd.index(flag) + 1], value)

    def test_a_different_brain_or_config_is_not_served_from_the_cache(self):
        calls = []
        sim = self.simulator()
        with patch.object(cv.subprocess, "run", self.fake_run(calls)):
            d1, p1 = sim.stage_brain({"modules": [1]})
            d2, p2 = sim.stage_brain({"modules": [2]})
            self.assertNotEqual(d1, d2)
            sim.battle(d1, p1, 1)
            sim.battle(d2, p2, 1)
            self.assertEqual(len(calls), 2)
            self.config.write_text('{"changed": true}', encoding="utf-8")
            other = self.simulator()
            self.assertNotEqual(other.environment, sim.environment)
            other.battle(d1, p1, 1)
            self.assertEqual(len(calls), 3)

    def test_a_cached_battle_from_before_unit_names_is_played_again(self):
        calls = []
        sim = self.simulator()
        digest, path = sim.stage_brain({"modules": []})
        old = cv.parse_battle(battle_document(seed=3))
        cached = sim.cache_dir / "battles" / sim.environment / digest / "s0-o0-l0-3.npz"
        cached.parent.mkdir(parents=True)
        np.savez_compressed(cached, meta=np.array(json.dumps(
            {"seed": 3, "outcome": "win", "fitness": 0.1, "dt": 0.1})), pos=old.pos, alive=old.alive,
            learner=old.learner, radius=old.radius)
        with patch.object(cv.subprocess, "run", self.fake_run(calls)):
            battle = sim.battle(digest, path, 3)
        self.assertEqual(len(calls), 1)
        self.assertEqual(battle.names, ("corsumo", "armpw"))

    def test_fresh_ignores_the_cache(self):
        calls = []
        sim = self.simulator(use_cache=False)
        digest, path = sim.stage_brain({"modules": []})
        with patch.object(cv.subprocess, "run", self.fake_run(calls)):
            sim.battle(digest, path, 1)
            sim.battle(digest, path, 1)
        self.assertEqual(len(calls), 2)

    def test_simulator_failure_reports_its_stderr(self):
        sim = self.simulator()
        digest, path = sim.stage_brain({"modules": []})
        failed = type("Done", (), {"returncode": 3, "stdout": "", "stderr": "no scenario 9"})()
        with patch.object(cv.subprocess, "run", return_value=failed):
            with self.assertRaisesRegex(RuntimeError, "no scenario 9"):
                sim.battle(digest, path, 1)

    def test_missing_binary_is_explained(self):
        with self.assertRaisesRegex(FileNotFoundError, "cargo build"):
            cv.Simulator(self.dir.name + "/nope.exe", self.catalog, self.config, self.cache)

    def test_play_all_returns_battles_per_checkpoint_in_seed_order(self):
        calls = []
        sim = self.simulator()
        brains = []
        for i in range(2):
            path = Path(self.dir.name) / f"b{i}.json"
            path.write_text(json.dumps({"modules": [i]}), encoding="utf-8")
            brains.append(cv.Checkpoint(f"G{i}", path, i))
        with patch.object(cv.subprocess, "run", self.fake_run(calls)):
            result = cv.play_all(sim, brains, range(10, 14), jobs=3, log=lambda *_: None)
        self.assertEqual({k: [b.seed for b in v] for k, v in result.items()},
                         {0: [10, 11, 12, 13], 1: [10, 11, 12, 13]})


def summary(seed, fitness=0.0, outcome="win", duration=24.0):
    return cv.RunSummary(seed, fitness, outcome, duration)


def make_timeline(generations=3, per_generation=4, per_second=2, interval=1.0, speed=4.0, spread=False, **kwargs):
    """Burst launches by default, which keep the hand-computed times in the tests below simple."""
    selected = [[summary(s, fitness=g + s / 10, outcome="win" if (g + s) % 2 else "loss", duration=8.0)
                 for s in range(per_generation)] for g in range(generations)]
    return cv.Timeline(selected, per_second, interval, speed, spread=spread, **kwargs)


def fake_checkpoints(count=3, population=True):
    return [checkpoint(f"Generation {g * 5}", g * 5, (-0.3, -0.1, -0.2, 32) if population and g == 1 else None)
            for g in range(count)]


class SelectionOfRunsTest(unittest.TestCase):
    def test_best_keeps_the_highest_fitness_and_restores_seed_order(self):
        runs = [summary(s, fitness=f) for s, f in enumerate([0.1, 0.9, 0.5, 0.7, 0.3])]
        self.assertEqual([r.seed for r in cv.select_runs(runs, 3, "best")], [1, 2, 3])

    def test_equal_fitness_prefers_the_lower_seed(self):
        runs = [summary(s, fitness=0.5) for s in range(6)]
        self.assertEqual([r.seed for r in cv.select_runs(runs, 2, "best")], [0, 1])

    def test_random_is_reproducible_and_unbiased_by_fitness(self):
        runs = [summary(s, fitness=float(s)) for s in range(100)]
        a = cv.select_runs(runs, 20, "random", salt=3)
        self.assertEqual(a, cv.select_runs(runs, 20, "random", salt=3))
        self.assertNotEqual([r.seed for r in a], [r.seed for r in cv.select_runs(runs, 20, "random", salt=4)])
        self.assertLess(sum(r.fitness for r in a) / 20, 80)  # not the top twenty
        self.assertEqual([r.seed for r in a], sorted(r.seed for r in a))

    def test_asking_for_more_than_exist_returns_all_and_a_bad_mode_is_refused(self):
        runs = [summary(s) for s in range(3)]
        self.assertEqual(len(cv.select_runs(runs, 50, "best")), 3)
        with self.assertRaisesRegex(ValueError, "unknown selection"):
            cv.select_runs(runs, 2, "worst")


class TimelineTest(unittest.TestCase):
    def test_runs_queue_generation_by_generation_and_start_in_batches(self):
        tl = make_timeline(generations=3, per_generation=4, per_second=2, interval=1.0)
        self.assertEqual([r.checkpoint for r in tl.runs], [0] * 4 + [1] * 4 + [2] * 4)
        self.assertEqual(tl.starts, [0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5])
        self.assertEqual(tl.launched(0.0), 2)
        self.assertEqual(tl.launched(2.5), 6)
        self.assertEqual(tl.launched(99), 12)

    def test_streaming_is_the_default_and_spaces_launches_evenly(self):
        selected = [[summary(s, duration=8.0) for s in range(6)]]
        stream = cv.Timeline(selected, 2, 1.0, 4.0)
        self.assertTrue(stream.spread)
        self.assertEqual(stream.starts, [0.0, 0.5, 1.0, 1.5, 2.0, 2.5])
        burst = cv.Timeline(selected, 2, 1.0, 4.0, spread=False)
        self.assertEqual(burst.starts, [0.0, 0.0, 1.0, 1.0, 2.0, 2.0])

    def test_a_stream_launches_the_same_number_of_runs_per_interval_as_bursts(self):
        selected = [[summary(s, duration=8.0) for s in range(50)] for _ in range(3)]
        stream = cv.Timeline(selected, 5, 1.0, 4.0)
        burst = cv.Timeline(selected, 5, 1.0, 4.0, spread=False)
        for t in (10.0, 20.0, 29.0):
            self.assertAlmostEqual(stream.launched(t), burst.launched(t), delta=5)
        self.assertAlmostEqual(stream.total_seconds, burst.total_seconds, delta=1.0)

    def test_a_stream_starts_at_most_one_run_per_frame_and_keeps_the_screen_level(self):
        selected = [[summary(s, duration=24.0) for s in range(50)] for _ in range(4)]
        fps = 30

        def profile(spread):
            tl = cv.Timeline(selected, 5, 1.0, 4.0, spread=spread)
            times = np.arange(tl.frame_count(fps)) / fps
            steady = times[(times > tl.span) & (times < tl.total_seconds - tl.span)]
            level = np.array([sum(tl.opacity(tl.runs[i], t) for i in tl.active(t)) for t in steady])
            starts = np.diff([tl.launched(t) for t in steady])
            return level, starts

        level, starts = profile(True)
        self.assertLessEqual(starts.max(), 1)
        self.assertLess((level.max() - level.min()) / level.mean(), 0.02)
        _, burst_starts = profile(False)
        self.assertEqual(burst_starts.max(), 5)

    def test_a_run_lasts_its_battle_at_the_playback_speed_then_fades(self):
        tl = make_timeline(speed=4.0, fade_out=0.6)
        run = tl.runs[0]
        self.assertAlmostEqual(run.end - run.start, 2.0)  # 8 simulated s at 4x
        self.assertEqual(tl.total_seconds, tl.runs[-1].end + 0.6)
        self.assertEqual(tl.frame_count(30), int(np.ceil(tl.total_seconds * 30)))

    def test_active_runs_are_those_started_and_not_yet_gone(self):
        tl = make_timeline(speed=4.0, fade_out=0.6)  # a run is visible for 2.0 s + 0.6 s of fade
        self.assertEqual(tl.active(0.0), [0, 1])
        self.assertEqual(tl.active(1.5), [0, 1, 2, 3])
        self.assertEqual(tl.active(2.3), [0, 1, 2, 3, 4, 5])  # the first pair ended at 2.0 and is fading out
        self.assertEqual(tl.playing(tl.active(2.3), 2.3), [2, 3, 4, 5])
        self.assertEqual(tl.active(2.7), [2, 3, 4, 5])  # fade over at 2.6
        self.assertEqual(tl.active(tl.total_seconds + 1), [])

    def test_opacity_ramps_in_and_out(self):
        tl = make_timeline(speed=4.0, fade_out=1.0, fade_in=0.5)
        run = tl.runs[0]
        self.assertEqual(tl.opacity(run, 0.0), 0.0)
        self.assertAlmostEqual(tl.opacity(run, 0.25), 0.5)
        self.assertEqual(tl.opacity(run, 1.0), 1.0)
        self.assertEqual(tl.opacity(run, run.end), 1.0)
        self.assertAlmostEqual(tl.opacity(run, run.end + 0.5), 0.5)
        self.assertEqual(tl.opacity(run, run.end + 1.0), 0.0)

    def test_each_run_has_its_own_clock_and_it_stops_at_the_battle_end(self):
        tl = make_timeline(speed=4.0)
        second_batch = tl.runs[2]
        self.assertAlmostEqual(tl.sim_time(second_batch, 1.5), 2.0)  # started at 1 s
        self.assertEqual(tl.sim_time(second_batch, 50.0), 8.0)
        self.assertEqual(tl.sim_time(second_batch, 0.0), 0.0)

    def test_max_sim_cuts_runs_short(self):
        tl = make_timeline(speed=4.0, max_sim=4.0)
        self.assertAlmostEqual(tl.runs[0].end - tl.runs[0].start, 1.0)

    def test_typical_concurrency_is_launch_rate_times_run_length(self):
        tl = make_timeline(per_second=5, interval=1.0, speed=4.0)  # 2 s runs, 5 per second
        self.assertAlmostEqual(tl.typical_concurrency(), 10.0)

    def test_bad_settings_and_an_empty_queue(self):
        with self.assertRaises(ValueError):
            cv.Timeline([[summary(1)]], 0, 1.0, 4.0)
        with self.assertRaises(ValueError):
            cv.Timeline([[summary(1)]], 1, 0.0, 4.0)
        empty = cv.Timeline([[]], 5, 1.0, 4.0)
        self.assertEqual((empty.total_seconds, empty.active(0.0)), (0.6, []))
        self.assertGreaterEqual(empty.frame_count(30), 1)


class ProgressiveReadoutTest(unittest.TestCase):
    def text(self, t, **kwargs):
        tl = make_timeline(generations=3, per_generation=4, per_second=2, speed=4.0, **kwargs)
        return tl, cv.progressive_text(tl, fake_checkpoints(), t, "top 4 of 8 runs per generation")

    def test_title_names_the_newest_generation_started_and_the_span_on_screen(self):
        _, (ul, *_rest) = self.text(0.5)
        self.assertEqual(ul, [("Generation 0", "big")])
        # Generation 1 starts leaving the queue at 2.0 s while the last two generation-0 runs still play.
        _, (ul, *_rest) = self.text(2.3)
        self.assertEqual(ul, [("Generation 5", "big"), ("showing generations 0 to 5", "small")])
        _, (ul, *_rest) = self.text(3.1)  # by now only generation 1 is playing
        self.assertEqual(ul, [("Generation 5", "big")])

    def test_fitness_range_and_outcomes_cover_only_the_runs_playing(self):
        tl, (_, ll, _, ur) = self.text(0.5)
        playing = tl.playing(tl.active(0.5), 0.5)
        values = [tl.runs[i].summary.fitness for i in playing]
        self.assertIn(f"{min(values):+.2f}", ll[0][0])
        self.assertIn(f"{max(values):+.2f}", ll[0][0])
        self.assertIn(f"{len(values)} runs on screen", ll[1][0])
        outcomes = [tl.runs[i].summary.outcome for i in playing]
        for name in set(outcomes):
            self.assertIn(f"{name} {round(100 * outcomes.count(name) / len(outcomes))}%", ur[0][0])

    def test_the_range_moves_as_better_generations_arrive(self):
        tl = make_timeline(generations=3, per_generation=4, per_second=2, speed=4.0)
        early = cv.progressive_text(tl, fake_checkpoints(), 0.5, "")[1][0][0]
        late = cv.progressive_text(tl, fake_checkpoints(), tl.total_seconds - 1.0, "")[1][0][0]
        self.assertNotEqual(early, late)

    def test_clock_counts_simulated_time_since_the_video_began_and_runs_started(self):
        _, (_, _, lr, _) = self.text(2.5)
        self.assertEqual(lr[0][0], "Sim time  0:10")  # 2.5 s of video at 4x
        self.assertEqual(lr[1][0], "run 6 of 12 started")

    def test_population_line_belongs_to_the_newest_generation_and_the_note_is_last(self):
        _, (_, ll, _, _) = self.text(2.5)  # generation 1 (index 1) has a recorded population
        self.assertTrue(any("ES population, all stages (32)" in text for text, _ in ll))
        self.assertEqual(ll[-1], ("top 4 of 8 runs per generation", "small"))
        _, (_, ll, _, _) = self.text(0.5)
        self.assertFalse(any("population" in text for text, _ in ll))

    def test_outcome_tally_can_be_hidden_and_an_empty_screen_reads_n_a(self):
        tl = make_timeline()
        self.assertEqual(cv.progressive_text(tl, fake_checkpoints(), 0.5, "", show_outcomes=False)[3], [])
        late = cv.progressive_text(tl, fake_checkpoints(), tl.total_seconds + 5, "")
        self.assertIn("n/a", late[1][0][0])

    def test_span_text_uses_labels_when_checkpoints_have_no_generation(self):
        cps = [checkpoint("Start", None), checkpoint("Night 1", None)]
        self.assertEqual(cv.span_text(cps, 0, 1), "showing Start to Night 1")
        self.assertIsNone(cv.span_text(cps, 1, 1))

    def test_selection_wording(self):
        self.assertEqual(cv.selection_text(50, 100, "best"), "top 50 of 100 runs per generation")
        self.assertEqual(cv.selection_text(50, 50, "best"), "50 runs per generation")
        self.assertEqual(cv.selection_text(50, 100, "random"), "50 random runs per generation (of 100)")


class ProgressiveRenderTest(unittest.TestCase):
    def setup(self, **timeline_kwargs):
        tl = make_timeline(**timeline_kwargs)
        battles = {}
        calls = []

        def load(run):
            calls.append(run.summary.seed)
            doc = battle_document(seed=run.summary.seed, frames=81)
            return battles.setdefault((run.checkpoint, run.summary.seed), cv.parse_battle(doc))

        reference = load(tl.runs[0])
        calls.clear()
        rosters = cv.rosters_of(reference, unit_info())
        labels = cv.Labels(320, 180, [cv.SimpleNamespace(rosters=rosters)], bottom_lines=4)
        view = cv.view_from_points([cv.battle_points(reference)], 320, 180, labels.reserved_margins())
        renderer = cv.Renderer(320, 180, view, 10, 4.0)
        balance = cv.Renderer.balance_for(1, 1)
        return tl, load, calls, renderer, labels, rosters, balance

    def test_frames_cover_the_range_and_each_run_is_loaded_once(self):
        tl, load, calls, renderer, labels, rosters, balance = self.setup(speed=4.0)
        frames = list(cv.progressive_frames(tl, fake_checkpoints(), load, renderer, labels, rosters, balance,
                                            "note", 10, 0, 24))
        self.assertEqual([k for k, _ in frames], list(range(25)))
        self.assertEqual(frames[0][1].size, (320, 180))
        touched = {i for k in range(25) for i in tl.active(k / 10)}
        self.assertEqual(len(calls), len(touched))

    def test_every_run_is_loaded_exactly_once_over_the_whole_video(self):
        tl, load, calls, renderer, labels, rosters, balance = self.setup(speed=4.0)
        gen = cv.progressive_frames(tl, fake_checkpoints(), load, renderer, labels, rosters, balance, "n", 10, 0,
                                    tl.frame_count(10) - 1)
        for _ in gen:
            pass
        self.assertEqual(len(calls), len(tl.runs))

    def test_a_fading_run_is_dimmer_than_one_playing(self):
        tl, load, _, renderer, _, _, balance = self.setup(speed=4.0)
        battle = load(tl.runs[0])
        spot = renderer.view.to_pixels(np.array([200.0, 5.0]), 320, 180)
        x, y = int(round(spot[0])), int(round(spot[1]))
        full = renderer.draw_runs([(battle, 0.5, 1.0)], 4.0, balance)[y, x, 0]
        renderer.reset()
        half = renderer.draw_runs([(battle, 0.5, 0.4)], 4.0, balance)[y, x, 0]
        self.assertGreater(full, half + 5)

    def test_no_runs_draws_only_the_background(self):
        tl, _, _, renderer, _, _, balance = self.setup()
        self.assertTrue(np.array_equal(renderer.draw_runs([], 4.0, balance), renderer.background))

    def test_runs_on_different_clocks_are_drawn_where_each_one_is(self):
        tl, load, _, renderer, _, _, balance = self.setup()
        a, b = load(tl.runs[0]), load(tl.runs[1])
        frame = renderer.draw_runs([(a, 0.0, 1.0), (b, 0.8, 1.0)], 2.0, balance)
        for t in (0.0, 0.8):
            x, y = renderer.view.to_pixels(np.array([10.0 * (t / 0.1), 5.0]), 320, 180)
            pixel = frame[int(round(y)), int(round(x))]
            self.assertGreater(pixel[2], cv.BACKGROUND_RGB[2] + 20)  # a blue ghost at its own position

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg not installed")
    def test_progressive_video_encodes_to_the_planned_length(self):
        tl, load, _, renderer, labels, rosters, balance = self.setup(speed=4.0)
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "p.mp4"
            frames = cv.render_progressive_video(tl, fake_checkpoints(), load, renderer, labels, rosters, balance,
                                                 "note", out, shutil.which("ffmpeg"), 10, log=lambda *_: None)
            self.assertEqual(frames, tl.frame_count(10))
            self.assertEqual(out.read_bytes()[4:8], b"ftyp")

    def test_stills_are_spread_over_the_video(self):
        tl, load, _, renderer, labels, rosters, balance = self.setup(speed=4.0)
        with tempfile.TemporaryDirectory() as d:
            paths = cv.write_progressive_stills(tl, fake_checkpoints(), load, renderer, labels, rosters, balance,
                                                "note", d, 10, count=3)
            self.assertEqual([p.name for p in paths], ["still-000.png", "still-001.png", "still-002.png"])


class ProgressiveRunTest(unittest.TestCase):
    def test_summaries_replace_battles_so_thousands_fit_in_memory(self):
        calls = []

        class FakeSim:
            def battle(self, digest, path, seed):
                calls.append(seed)
                return cv.parse_battle(battle_document(seed=seed, fitness=seed / 10))

        result = cv.play_all(FakeSim(), [checkpoint(), checkpoint()], range(4), 2, log=lambda *_: None,
                             summaries=True, staged=[("d0", "p0"), ("d1", "p1")])
        self.assertTrue(all(isinstance(r, cv.RunSummary) for runs in result.values() for r in runs))
        self.assertEqual([r.seed for r in result[1]], [0, 1, 2, 3])
        self.assertAlmostEqual(result[0][3].duration, 1.0)

    def test_a_fresh_replay_still_refills_the_cache_that_progressive_mode_reloads_from(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for name in ("sim.exe", "catalog.json", "config.json"):
                (root / name).write_text("{}", encoding="utf-8")
            sim = cv.Simulator(root / "sim.exe", root / "catalog.json", root / "config.json", root / "cache",
                               use_cache=False)
            digest, path = sim.stage_brain({"modules": []})
            done = type("Done", (), {"returncode": 0, "stdout": json.dumps(battle_document(seed=9)), "stderr": ""})()
            with patch.object(cv.subprocess, "run", return_value=done):
                sim.battle(digest, path, 9)
            self.assertEqual(len(list((root / "cache" / "battles").rglob("*.npz"))), 1)

    def test_pool_smaller_than_top_is_refused_before_any_work(self):
        with self.assertRaises(SystemExit) as raised:
            cv.main(["--mode", "progressive", "--top", "50", "--pool", "10", "--catalog", "c",
                     "--train-config", "t", "--brains", "b.json"])
        self.assertIn("--pool", str(raised.exception))

    def test_progressive_options_default_to_fifty_and_five(self):
        args = cv.build_parser().parse_args(["--mode", "progressive"])
        self.assertEqual((args.top, args.per_second, args.interval, args.select), (50, 5, 1.0, "best"))
        self.assertIsNone(args.pool)
        self.assertEqual(args.launch, "stream")
        self.assertEqual(cv.build_parser().parse_args(["--launch", "burst"]).launch, "burst")


class CleanupTest(unittest.TestCase):
    def test_default_cache_is_a_temporary_folder_that_is_removed_afterwards(self):
        with cv.cache_scope() as cache:
            self.assertTrue(cache.is_dir())
            self.assertEqual(cache.parent, Path(tempfile.gettempdir()))
            (cache / "battles").mkdir()
            (cache / "battles" / "x.npz").write_bytes(b"x")
        self.assertFalse(cache.exists())

    def test_the_temporary_cache_is_removed_even_when_the_run_fails(self):
        with self.assertRaises(RuntimeError):
            with cv.cache_scope() as cache:
                (cache / "x.npz").write_bytes(b"x")
                raise RuntimeError("boom")
        self.assertFalse(cache.exists())

    def test_an_explicit_cache_is_kept(self):
        with tempfile.TemporaryDirectory() as d:
            keep = Path(d) / "mine"
            with cv.cache_scope(str(keep)) as cache:
                self.assertEqual(cache, keep)
                keep.mkdir()
                (keep / "x.npz").write_bytes(b"x")
            self.assertTrue((keep / "x.npz").is_file())

    def fake_simulator(self, seen):
        class FakeSimulator:
            def __init__(self, exe, catalog, config, cache_dir, *args, **kwargs):
                seen.append(Path(cache_dir))
                Path(cache_dir, "battles").mkdir(parents=True, exist_ok=True)
                Path(cache_dir, "battles", "x.npz").write_bytes(b"x")

            def stage_brain(self, brain):
                return "d", Path("p")

            def describe(self, path):
                return [], []
        return FakeSimulator

    def run_main(self, root, *extra):
        brain = Path(root) / "brain.json"
        brain.write_text('{"modules": []}', encoding="utf-8")
        seen = []
        with patch.object(cv, "Simulator", self.fake_simulator(seen)):
            cv.main(["--list", "--brains", str(brain), "--catalog", "c", "--train-config", "t",
                     "--out", str(Path(root) / "out" / "cloud.mp4"), *extra])
        return seen

    def test_a_default_run_leaves_nothing_next_to_the_video_and_removes_its_cache(self):
        with tempfile.TemporaryDirectory() as d:
            seen = self.run_main(d)
            self.assertFalse(seen[0].exists())
            self.assertEqual(sorted(p.name for p in Path(d).iterdir()), ["brain.json"])  # no <out>.cache, no out/

    def test_a_run_with_cache_keeps_it(self):
        with tempfile.TemporaryDirectory() as d:
            keep = Path(d) / "kept"
            seen = self.run_main(d, "--cache", str(keep))
            self.assertEqual(seen, [keep])
            self.assertTrue((keep / "battles" / "x.npz").is_file())

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg not installed")
    def test_aborting_the_encoder_removes_the_half_written_video(self):
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "half.mp4"
            encoder = cv.Encoder(shutil.which("ffmpeg"), out, 64, 64, 10)
            encoder.write(cv.Image.new("RGB", (64, 64), (10, 20, 30)))
            encoder.abort()
            self.assertFalse(out.exists())
            self.assertIsNotNone(encoder.process.poll())

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg not installed")
    def test_a_render_that_fails_midway_leaves_no_video_behind(self):
        scenes = [scene_of(count=3)]
        view = cv.fit_view(scenes, 160, 90, 8)
        renderer = cv.Renderer(160, 90, view, 10, 2.0)

        class Boom(cv.Labels):
            def draw(self, *args, **kwargs):
                raise RuntimeError("renderer failed")

        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "x.mp4"
            with self.assertRaisesRegex(RuntimeError, "renderer failed"):
                cv.render_video(scenes, renderer, Boom(160, 90), out, shutil.which("ffmpeg"), 10, 2.0, 0.5,
                                log=lambda *_: None)
            self.assertFalse(out.exists())
            self.assertEqual(list(Path(d).iterdir()), [])


class VideoTest(unittest.TestCase):
    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg not installed")
    def test_two_scenes_encode_to_an_mp4_of_the_planned_length(self):
        scenes = [scene_of(count=3), scene_of(count=3)]
        view = cv.fit_view(scenes, 160, 90, 8)
        renderer = cv.Renderer(160, 90, view, 10, 2.0)
        with tempfile.TemporaryDirectory() as d:
            out = Path(d) / "sub" / "cloud.mp4"
            frames = cv.render_video(scenes, renderer, cv.Labels(160, 90), out, shutil.which("ffmpeg"),
                                     fps=10, speed=2.0, hold_seconds=0.5, log=lambda *_: None)
            self.assertTrue(out.is_file() and out.stat().st_size > 0)
            # Each scene: 1.0 s of battle at 2x speed plus 0.5 s held, at 10 fps.
            self.assertEqual(frames, 2 * round((0.5 + 0.5) * 10))
            self.assertEqual(out.read_bytes()[4:8], b"ftyp")

    def test_stills_are_written_one_per_scene(self):
        scenes = [scene_of(count=3), scene_of(count=3)]
        view = cv.fit_view(scenes, 160, 90, 8)
        renderer = cv.Renderer(160, 90, view, 10, 2.0)
        with tempfile.TemporaryDirectory() as d:
            paths = cv.write_stills(scenes, renderer, cv.Labels(160, 90), d, 10, 2.0)
            self.assertEqual([p.name for p in paths], ["scene-000.png", "scene-001.png"])
            self.assertEqual(cv.Image.open(paths[0]).size, (160, 90))

    def test_odd_video_size_is_refused_before_any_work(self):
        with self.assertRaises(SystemExit):
            cv.main(["--size", "641x360", "--catalog", "c", "--train-config", "t", "--brains", "b.json"])

    def test_run_and_brains_are_mutually_exclusive(self):
        args = cv.build_parser().parse_args(["--catalog", "c", "--train-config", "t"])
        with self.assertRaisesRegex(ValueError, "exactly one"):
            cv.gather_checkpoints(args)


if __name__ == "__main__":
    unittest.main()
