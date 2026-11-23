import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

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

    def test_build_ui_command_uses_dedicated_config_only_when_ungated(self):
        out = Path("C:/nonexistent/ui")
        stock_cmd = suite.build_ui_command(out, ungated=False)
        self.assertEqual(stock_cmd[stock_cmd.index("--config") + 1], str(suite.STOCK_VITE_CONFIG))
        self.assertEqual(stock_cmd[stock_cmd.index("--emptyOutDir") + 1], "false")
        ungated_cmd = suite.build_ui_command(out, ungated=True)
        self.assertEqual(ungated_cmd[ungated_cmd.index("--config") + 1], str(suite.UNGATED_VITE_CONFIG))
        self.assertEqual(ungated_cmd[ungated_cmd.index("--emptyOutDir") + 1], "false")

    def test_run_build_ui_guards_ownership_links_and_failed_builds(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            unmarked = root / "user-data"
            unmarked.mkdir()
            precious = unmarked / "campaign.battleplan"
            precious.write_text("keep", encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "battleplan-ui-bundle"):
                suite.run_build_ui(unmarked, runner=lambda *a, **k: SimpleNamespace(returncode=0))
            self.assertEqual(precious.read_text(encoding="utf-8"), "keep")

            if os.name == "nt":
                junction = root / "link-to-user-data"
                subprocess.run(["cmd", "/c", "mklink", "/J", str(junction), str(unmarked)], check=True, capture_output=True)
                try:
                    with self.assertRaisesRegex(RuntimeError, "symlink or junction"):
                        suite.validate_ui_output(junction)
                    with self.assertRaisesRegex(RuntimeError, "symlink or junction"):
                        suite.validate_ui_output(junction / "nested")
                finally:
                    subprocess.run(["cmd", "/c", "rmdir", str(junction)], check=False, capture_output=True)

            empty_dir = root / "empty-out"
            empty_dir.mkdir()
            self.assertEqual(suite.run_build_ui(empty_dir, runner=lambda *a, **k: SimpleNamespace(returncode=1)), 1)
            self.assertFalse((empty_dir / suite.UI_BUNDLE_MARKER).exists(), "failed first build must not mark an empty folder")

            def fake_ok(cmd, cwd=None, env=None):
                out_dir = Path(cmd[cmd.index("--outDir") + 1])
                (out_dir / "index.html").write_text("v1", encoding="utf-8")
                return SimpleNamespace(returncode=0)

            self.assertEqual(suite.run_build_ui(empty_dir, runner=fake_ok), 0)
            self.assertTrue((empty_dir / suite.UI_BUNDLE_MARKER).is_file())
            self.assertEqual((empty_dir / "index.html").read_text(encoding="utf-8"), "v1")

            self.assertEqual(suite.run_build_ui(empty_dir, runner=lambda *a, **k: SimpleNamespace(returncode=1)), 1)
            self.assertTrue((empty_dir / suite.UI_BUNDLE_MARKER).is_file(), "failed rebuild must keep marker and ownership")
            self.assertEqual((empty_dir / "index.html").read_text(encoding="utf-8"), "v1")

            def fake_v2(cmd, cwd=None, env=None):
                out_dir = Path(cmd[cmd.index("--outDir") + 1])
                (out_dir / "index.html").write_text("v2", encoding="utf-8")
                return SimpleNamespace(returncode=0)

            self.assertEqual(suite.run_build_ui(empty_dir, runner=fake_v2), 0)
            self.assertTrue((empty_dir / suite.UI_BUNDLE_MARKER).is_file())
            self.assertEqual((empty_dir / "index.html").read_text(encoding="utf-8"), "v2")


if __name__ == "__main__":
    unittest.main()
