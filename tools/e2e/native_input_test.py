import importlib.util
import json
from pathlib import Path
import socket
import threading
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

    def make_socket_desktop(self, port):
        desktop = MODULE.Desktop.__new__(MODULE.Desktop)
        desktop.address = f"127.0.0.1:{port}"
        desktop.channel = None
        desktop.channel_reader = None
        return desktop



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

    def test_camera_reconnects_after_channel_eof(self):
        received = []
        server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        server.bind(("127.0.0.1", 0))
        server.listen(4)
        server.settimeout(5)
        port = server.getsockname()[1]

        def serve():
            try:
                for index in range(2):
                    conn, _ = server.accept()
                    with conn:
                        conn.settimeout(5)
                        reader = conn.makefile("r", encoding="utf-8")
                        try:
                            line = reader.readline()
                            if line:
                                received.append(json.loads(line))
                        finally:
                            reader.close()
                        if index == 1:
                            conn.sendall((json.dumps({"center": [2816.0, 1024.0], "height": 3000.0}) + "\n").encode())
            except OSError:
                return

        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        desktop = self.make_socket_desktop(port)
        try:
            with self.assertRaisesRegex(RuntimeError, "closed the suite input channel before replying"):
                desktop.camera(view="overview-map", center=[100.0, 200.0], height=180.0)
            self.assertIsNone(desktop.channel)
            self.assertIsNone(desktop.channel_reader)
            reply = desktop.camera(view="overview-map", reset=True)
            self.assertEqual(reply["center"], [2816.0, 1024.0])
            self.assertEqual(reply["height"], 3000.0)
        finally:
            if hasattr(desktop, "close_channel"):
                desktop.close_channel()
            server.close()
            thread.join(timeout=5)
        self.assertEqual(
            received,
            [
                {"camera": {"view": "overview-map", "center": [100.0, 200.0], "height": 180.0}},
                {"camera": {"view": "overview-map", "reset": True}},
            ],
        )

    def test_camera_reconnects_after_channel_timeout(self):
        sent_first = []
        received = []

        class TimedOutChannel:
            def __init__(self):
                self.closed = False

            def sendall(self, data):
                sent_first.append(json.loads(data.decode("utf-8")))

            def close(self):
                self.closed = True

        class TimedOutReader:
            def __init__(self):
                self.timed_out = False
                self.closed = False

            def readline(self):
                if self.timed_out:
                    raise OSError("cannot read from timed out object")
                self.timed_out = True
                raise TimeoutError("timed out")

            def close(self):
                self.closed = True

        server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        server.bind(("127.0.0.1", 0))
        server.listen(4)
        server.settimeout(5)
        port = server.getsockname()[1]

        def serve():
            try:
                conn, _ = server.accept()
                with conn:
                    conn.settimeout(5)
                    reader = conn.makefile("r", encoding="utf-8")
                    try:
                        line = reader.readline()
                        if line:
                            received.append(json.loads(line))
                    finally:
                        reader.close()
                    conn.sendall((json.dumps({"center": [2816.0, 1024.0], "height": 3000.0}) + "\n").encode())
            except OSError:
                return

        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        desktop = self.make_socket_desktop(port)
        first_channel = TimedOutChannel()
        first_reader = TimedOutReader()
        desktop.channel = first_channel
        desktop.channel_reader = first_reader
        try:
            with self.assertRaises(TimeoutError):
                desktop.camera(view="overview-map", center=[100.0, 200.0], height=180.0)
            self.assertTrue(first_channel.closed)
            self.assertTrue(first_reader.closed)
            self.assertIsNone(desktop.channel)
            self.assertIsNone(desktop.channel_reader)
            reply = desktop.camera(view="overview-map", reset=True)
            self.assertIsInstance(desktop.channel, socket.socket)
            self.assertEqual(reply["center"], [2816.0, 1024.0])
            self.assertEqual(reply["height"], 3000.0)
        finally:
            if hasattr(desktop, "close_channel"):
                desktop.close_channel()
            server.close()
            thread.join(timeout=5)
        self.assertEqual(
            sent_first,
            [{"camera": {"view": "overview-map", "center": [100.0, 200.0], "height": 180.0}}],
        )
        self.assertEqual(
            received,
            [{"camera": {"view": "overview-map", "reset": True}}],
        )


if __name__ == "__main__":
    unittest.main()
