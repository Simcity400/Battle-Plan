import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("native_input", Path(__file__).with_name("native_input.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class NativeInputCameraTest(unittest.TestCase):
    def make_desktop(self, response=None):
        desktop = MODULE.Desktop.__new__(MODULE.Desktop)
        sent = []

        def fake_send_and_receive(line):
            sent.append(line)
            if callable(response):
                return 1234.5, response(line)
            return 1234.5, response or {"center": [2816.0, 1024.0], "height": 250.0}

        desktop.send_and_receive = fake_send_and_receive
        return desktop, sent

    def test_camera_place_returns_resolved_pose_from_viewer(self):
        desktop, sent = self.make_desktop({"center": [2816.0, 1024.0], "height": 250.0})
        reply = desktop.camera(view="overview-map", center=[2816, 1024], height=180)
        self.assertEqual(reply, {"posted": [1234.5], "center": [2816.0, 1024.0], "height": 250.0})
        self.assertEqual(sent, [{"camera": {"view": "overview-map", "center": [2816.0, 1024.0], "height": 180.0}}])

    def test_camera_reset_returns_resolved_fit_pose(self):
        desktop, sent = self.make_desktop({"center": [2816.0, 1024.0], "height": 3000.0})
        reply = desktop.camera(view="overview-map", reset=True)
        self.assertEqual(reply, {"posted": [1234.5], "center": [2816.0, 1024.0], "height": 3000.0})
        self.assertEqual(sent, [{"camera": {"view": "overview-map", "reset": True}}])

    def test_camera_propagates_viewer_error_and_rejects_invalid_arguments(self):
        desktop, _ = self.make_desktop()
        with self.assertRaises(ValueError):
            desktop.camera(view="overview-map")
        with self.assertRaises(ValueError):
            desktop.camera(view="overview-map", center=[1.0], height=180.0)
        with self.assertRaises(ValueError):
            desktop.camera(view="overview-map", center=[1.0, 2.0], height=0.0)
        with self.assertRaises(ValueError):
            desktop.camera(view="overview-map", center=[float("nan"), 2.0], height=180.0)

        failing, _ = self.make_desktop({"error": "viewer timed out"})
        with self.assertRaisesRegex(RuntimeError, "viewer timed out"):
            failing.camera(view="overview-map", reset=True)


if __name__ == "__main__":
    unittest.main()
