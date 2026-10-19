import random
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import evolve  # noqa: E402

BASE = {"es": {"population": 48, "sigma": 0.1, "learning_rate": 0.03}, "seeds_per_battle": 2, "layout_jitter": 30.0}


class TweakTests(unittest.TestCase):
    def test_tweaks_are_small_bounded_and_change_something(self):
        hp = evolve.default_hp(BASE)
        for seed in range(300):
            new, focus, notes = evolve.tweak(hp, random.Random(seed))
            self.assertEqual(len(notes), 2)
            for key in ("sigma", "learning_rate", "population"):
                low, high = evolve.BOUNDS[key]
                self.assertTrue(low <= new["es"][key] <= high, (key, new))
            self.assertEqual(new["es"]["population"] % 2, 0)
            self.assertIn(new["es"]["l1"], evolve.L1_CHOICES)
            low, high = evolve.BOUNDS["seeds_per_battle"]
            self.assertTrue(low <= new["seeds_per_battle"] <= high)
            self.assertTrue(set(focus) <= set(evolve.MODULES))
            self.assertNotIn("focus", new)  # focus is child-only, never adopted

    def test_tweak_does_not_mutate_the_incumbent(self):
        hp = evolve.default_hp(BASE)
        before = repr(hp)
        for seed in range(50):
            evolve.tweak(hp, random.Random(seed))
        self.assertEqual(repr(hp), before)

    def test_repeated_tweaks_stay_inside_bounds(self):
        hp, rng = evolve.default_hp(BASE), random.Random(0)
        for _ in range(2000):
            hp, _, _ = evolve.tweak(hp, rng, scale=0.5)
        self.assertTrue(0.02 <= hp["es"]["sigma"] <= 0.4)
        low, high = evolve.BOUNDS["population"]
        self.assertTrue(low <= hp["es"]["population"] <= high)


class SelectionTests(unittest.TestCase):
    def test_first_round_promotes_the_best_child(self):
        self.assertEqual(evolve.choose_winner({"c0": 0.1, "c1": 0.3}, None, 0.005), ("c1", True))

    def test_a_child_must_beat_the_champion_by_the_margin(self):
        scores = {"champion": 0.50, "c0": 0.503, "c1": 0.49}
        self.assertEqual(evolve.choose_winner(scores, "champion", 0.005), ("champion", False))
        scores["c0"] = 0.52
        self.assertEqual(evolve.choose_winner(scores, "champion", 0.005), ("c0", True))

    def test_champion_alone_is_kept(self):
        self.assertEqual(evolve.choose_winner({"champion": 0.2}, "champion", 0.0), ("champion", False))


class SummaryTests(unittest.TestCase):
    def test_baseline_summary_ignores_hall_of_fame_rows(self):
        def row(opp, w, t, l):
            return {"opponent": opp, "tally": {"wins": w, "timeouts": t, "losses": l, "battles": w + t + l}}

        ev = {
            "total": {"fitness_sum": 3.0, "battles": 20, "underdog_wins": 1, "underdog_battles": 4},
            "rows": [row("charge", 6, 2, 2), row("brain:/x.json", 0, 0, 10)],
        }
        s = evolve.baseline_summary(ev, ["charge"])
        self.assertAlmostEqual(s["baseline_win"], 0.6)
        self.assertAlmostEqual(s["baseline_timeout"], 0.2)
        self.assertAlmostEqual(s["underdog_win"], 0.25)
        self.assertAlmostEqual(evolve.mean_fitness(ev), 0.15)


class FocusTests(unittest.TestCase):
    @staticmethod
    def _row(opp, wins, timeouts, losses, fitness):
        n = wins + timeouts + losses
        return {"opponent": opp, "tally": {"wins": wins, "timeouts": timeouts, "losses": losses,
                                           "draws": 0, "battles": n, "fitness_sum": fitness * n}}

    def test_by_opponent_aggregates_scenarios_and_skips_hall_of_fame(self):
        ev = {"rows": [self._row("orbit", 0, 10, 0, -0.3), self._row("orbit", 5, 5, 0, 0.5),
                       self._row("kite", 10, 0, 0, 1.0), self._row("brain:/x.json", 0, 0, 10, -1.0)]}
        per = evolve.by_opponent(ev, ["orbit", "kite"])
        self.assertEqual(set(per), {"orbit", "kite"})
        self.assertAlmostEqual(per["orbit"]["win"], 0.25)
        self.assertAlmostEqual(per["orbit"]["timeout"], 0.75)
        self.assertAlmostEqual(per["orbit"]["fitness"], 0.1)

    def test_weakest_baseline_picks_the_lowest_mean_fitness(self):
        ev = {"rows": [self._row("charge", 8, 0, 2, 0.6), self._row("kite", 10, 0, 0, 1.0),
                       self._row("orbit", 0, 10, 0, -0.3)]}
        self.assertEqual(evolve.weakest_baseline(ev, ["charge", "kite", "orbit"])["opponent"], "orbit")
        # The full ranking is kept so the focus can alternate between the weakest two.
        self.assertEqual(evolve.weakest_baseline(ev, ["charge", "kite", "orbit"])["ranked"],
                         ["orbit", "charge", "kite"])

    def test_nothing_to_prioritise_with_a_single_baseline(self):
        ev = {"rows": [self._row("orbit", 0, 10, 0, -0.3)]}
        self.assertIsNone(evolve.weakest_baseline(ev, ["orbit"]))


class SelectionScoreTests(unittest.TestCase):
    def test_selection_uses_the_robust_score_when_reported_else_the_plain_mean(self):
        plain = {"total": {"fitness_sum": 5.0, "battles": 10}}
        self.assertAlmostEqual(evolve.mean_fitness(plain), 0.5)
        robust = {"total": {"fitness_sum": 5.0, "battles": 10}, "robust_fitness": -0.2}
        self.assertAlmostEqual(evolve.mean_fitness(robust), -0.2)

    def test_digest_summarises_promotions_and_work(self):
        import json, tempfile
        with tempfile.TemporaryDirectory() as d:
            records = [
                {"round": 0, "winner": "c1", "promoted": True, "candidates": {"c1": {"score": 0.1}},
                 "work": {"battles": 10, "sim_seconds": 100.0, "wall_seconds": 2.0}},
                {"round": 1, "winner": "champion", "promoted": False, "candidates": {"champion": {"score": 0.1}},
                 "work": {"battles": 5, "sim_seconds": 40.0, "wall_seconds": 1.0},
                 "by_opponent": {"orbit": {"win": 0.5}}, "weakest": {"opponent": "orbit"}},
            ]
            lines = "\n".join(json.dumps(r) for r in records) + "\n"
            (Path(d) / "history.jsonl").write_text(lines, encoding="utf-8")
            digest = evolve.digest(d)
        self.assertEqual(digest["rounds"], 2)
        self.assertEqual(digest["promotions"], [{"round": 0, "winner": "c1", "score": 0.1}])
        self.assertEqual(digest["work"]["battles"], 15)
        self.assertEqual(digest["last_weakest"], {"opponent": "orbit"})


class WorkTests(unittest.TestCase):
    def test_training_work_sums_logs_and_ignores_missing_or_old_ones(self):
        import json, tempfile
        with tempfile.TemporaryDirectory() as d:
            a, b = Path(d) / "a.jsonl", Path(d) / "b.jsonl"
            lines = [json.dumps({"battles": 100, "sim_seconds": 50.0}),
                     json.dumps({"battles": 10, "sim_seconds": 5.0})]
            a.write_text("\n".join(lines) + "\n", encoding="utf-8")
            # A log from before accounting existed, plus a corrupt line.
            b.write_text(json.dumps({"generation": 1}) + "\nnot json\n", encoding="utf-8")
            work = evolve.training_work([a, b, Path(d) / "missing.jsonl"])
        self.assertEqual(work, {"battles": 110, "sim_seconds": 55.0})


if __name__ == "__main__":
    unittest.main()
