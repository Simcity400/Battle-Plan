import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("native_input", Path(__file__).with_name("native_input.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class NativeInputCameraTest(unittest.TestCase):
    def make_desktop(self):
        desktop = MODULE.Desktop.__new__(MODULE.Desktop)
        sent = []

        def fake_send(*lines):
            sent.extend(lines)
            return 1234.5

        desktop.send = fake_send
        return desktop, sent

    def test_camera_place_sends_view_center_and_height(self):
        desktop, sent = self.make_desktop()
        reply = desktop.camera(view="overview-map", center=[2816, 1024], height=180)
        self.assertEqual(reply, {"posted": [1234.5]})
        self.assertEqual(sent, [{"camera": {"view": "overview-map", "center": [2816.0, 1024.0], "height": 180.0}}])

    def test_camera_reset_sends_reset_flag(self):
        desktop, sent = self.make_desktop()
        reply = desktop.camera(view="overview-map", reset=True)
        self.assertEqual(reply, {"posted": [1234.5]})
        self.assertEqual(sent, [{"camera": {"view": "overview-map", "reset": True}}])

    def test_camera_rejects_invalid_arguments(self):
        desktop, _ = self.make_desktop()
        with self.assertRaises(ValueError):
            desktop.camera(view="overview-map")
        with self.assertRaises(ValueError):
            desktop.camera(view="overview-map", center=[1.0], height=180.0)
        with self.assertRaises(ValueError):
            desktop.camera(view="overview-map", center=[1.0, 2.0], height=0.0)
        with self.assertRaises(ValueError):
            desktop.camera(view="overview-map", center=[float("nan"), 2.0], height=180.0)


if __name__ == "__main__":
    unittest.main()
