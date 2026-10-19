import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bar_lua import _Parser, _tokens  # noqa: E402


def parse(text):
    return _Parser(_tokens(text)).value()


class BarLuaTest(unittest.TestCase):
    def test_long_string_and_division(self):
        table = parse('{ yardmap = [[h\n oo\n oo]], eff = 1 / 70, k = 3 * 2, [1] = "a", [2] = "b" }')
        self.assertEqual(table["yardmap"], "h\n oo\n oo")  # first newline only is dropped when leading
        self.assertAlmostEqual(table["eff"], 1 / 70)
        self.assertEqual(table["k"], 6.0)
        self.assertEqual((table[1], table[2]), ("a", "b"))

    def test_leveled_long_string_and_comments(self):
        table = parse("{ a = [==[x]]y]==], --[[ gone ]] b = 2 -- tail\n }")
        self.assertEqual(table["a"], "x]]y")
        self.assertEqual(table["b"], 2.0)


if __name__ == "__main__":
    unittest.main()
