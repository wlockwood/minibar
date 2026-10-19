import math
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fresh_retest as fr  # noqa: E402


class PairedStatsTest(unittest.TestCase):
    def test_a_consistent_gain_has_a_large_t(self):
        base = [0.0, 0.1, 0.2, 0.3, 0.4]
        stats = fr.paired_stats(base, [b + 0.05 + (0.001 if i % 2 else -0.001) for i, b in enumerate(base)])
        self.assertAlmostEqual(stats["diff"], 0.05, places=3)
        self.assertGreater(stats["t"], 20)
        self.assertEqual(stats["n"], 5)

    def test_pairing_removes_the_seed_to_seed_spread(self):
        base = [0.0, 1.0, -1.0, 2.0, -2.0, 0.5]
        paired = fr.paired_stats(base, [b + 0.1 + 0.01 * (i % 2) for i, b in enumerate(base)])
        unpaired_spread = max(base) - min(base)
        self.assertGreater(paired["t"], 10)
        self.assertGreater(unpaired_spread, 3)

    def test_no_difference_is_t_zero_and_identical_runs_do_not_divide_by_zero(self):
        stats = fr.paired_stats([0.1, 0.2, 0.3], [0.1, 0.2, 0.3])
        self.assertEqual(stats["t"], 0.0)
        self.assertEqual(fr.paired_stats([0.1, 0.2], [0.2, 0.3])["t"], 999.0)
        self.assertTrue(math.isfinite(fr.paired_stats([0.1, 0.2], [0.0, 0.1])["t"]))

    def test_samples_must_be_paired(self):
        with self.assertRaises(ValueError):
            fr.paired_stats([0.1, 0.2], [0.1])
        with self.assertRaises(ValueError):
            fr.paired_stats([0.1], [0.2])


if __name__ == "__main__":
    unittest.main()
