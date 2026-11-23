import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import suite


class CoresAndProfileTests(unittest.TestCase):
    def test_parse_cores_count_prefers_even_logical_cpus(self):
        parsed = suite.parse_cores("4", system_mask=0xFFFF)
        self.assertEqual(parsed["count"], 4)
        self.assertEqual(parsed["mask"], 0x55)
        self.assertEqual(parsed["cpus"], [0, 2, 4, 6])

    def test_parse_cores_fills_odd_after_even(self):
        parsed = suite.parse_cores("10", system_mask=0xFFFF)
        self.assertEqual(parsed["count"], 10)
        self.assertEqual(parsed["cpus"], [0, 2, 4, 6, 8, 10, 12, 14, 1, 3])
        self.assertEqual(parsed["mask"].bit_count(), 10)

    def test_parse_cores_hex_mask(self):
        parsed = suite.parse_cores("0x55", system_mask=0xFFFF)
        self.assertEqual(parsed["mask"], 0x55)
        self.assertEqual(parsed["count"], 4)
        self.assertEqual(parsed["cpus"], [0, 2, 4, 6])

    def test_parse_cores_rejects_out_of_range(self):
        with self.assertRaises(ValueError):
            suite.parse_cores("0", system_mask=0xFFFF)
        with self.assertRaises(ValueError):
            suite.parse_cores("17", system_mask=0xFFFF)
        with self.assertRaises(ValueError):
            suite.parse_cores("0x10000", system_mask=0xFFFF)

    def test_parse_weak_profiles(self):
        base = ["--app", "C:/nonexistent/app", "--fixture", "C:/nonexistent/fix",
                "--game-dir", "C:/nonexistent/game", "--out", "C:/nonexistent/out",
                "--state", "C:/nonexistent/state"]
        weak = suite.parse(["run", *base, "--profile", "weak"])
        self.assertEqual(weak.profile, "weak")
        self.assertEqual(weak.cpu_throttle, 4.0)
        self.assertEqual(weak.cores["count"], 4)
        weak8 = suite.parse(["run", *base, "--profile", "weak8"])
        self.assertEqual(weak8.profile, "weak8")
        self.assertEqual(weak8.cpu_throttle, 8.0)
        self.assertEqual(weak8.cores["count"], 4)

    def test_parse_build_ui_ungated(self):
        stock = suite.parse(["build-ui", "--out", "C:/nonexistent/ui"])
        self.assertFalse(stock.ungated)
        ungated = suite.parse(["build-ui", "--out", "C:/nonexistent/ui", "--ungated"])
        self.assertTrue(ungated.ungated)


if __name__ == "__main__":
    unittest.main()
