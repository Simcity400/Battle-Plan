import sys
import tempfile
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

    def test_build_ui_command_uses_dedicated_mode_only_when_ungated(self):
        out = Path("C:/nonexistent/ui")
        stock_cmd = suite.build_ui_command(out, ungated=False)
        self.assertNotIn("--mode", stock_cmd)
        self.assertNotIn(suite.UNGATED_VITE_MODE, stock_cmd)
        ungated_cmd = suite.build_ui_command(out, ungated=True)
        mode_index = ungated_cmd.index("--mode")
        self.assertEqual(ungated_cmd[mode_index + 1], suite.UNGATED_VITE_MODE)

    def test_prepare_ui_output_refuses_unmarked_non_empty_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            unmarked = root / "user-data"
            unmarked.mkdir()
            precious = unmarked / "campaign.battleplan"
            precious.write_text("keep", encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "battleplan-ui-bundle"):
                suite.prepare_ui_output(unmarked)
            self.assertEqual(precious.read_text(encoding="utf-8"), "keep")

            empty_dir = root / "empty-out"
            empty_dir.mkdir()
            suite.prepare_ui_output(empty_dir)
            self.assertTrue(empty_dir.is_dir())

            marked = root / "marked-out"
            marked.mkdir()
            (marked / suite.UI_BUNDLE_MARKER).write_text("ui-bundle\n", encoding="utf-8")
            (marked / "stale.js").write_text("old", encoding="utf-8")
            suite.prepare_ui_output(marked)
            self.assertTrue((marked / suite.UI_BUNDLE_MARKER).is_file())


if __name__ == "__main__":
    unittest.main()
