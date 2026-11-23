"""Input to the native map, on the suite's own desktop: one JSON command per stdin line, one JSON reply each.

    python tools/e2e/native_input.py --desktop <name> --app-pid <pid> --input <127.0.0.1:port>

Points are the app window's client pixels, as the page measures them times its device pixel ratio.
Pointer and key input goes through the viewer's suite input channel (crates/viewer/src/suite_input.rs,
served at `--input`, which the suite set in the app's BATTLEPLAN_E2E_INPUT): on a desktop of the suite's
own, where no cursor is, the windowing library would drop real pointer input. Points become client
pixels of the map's window under them. The user's own input and cursor are never touched. Each reply
carries the epoch milliseconds each event was sent at, the clock the viewer stamps its frames with.
    {"op": "move", "points": [[x, y], ...], "interval_ms": 16}
    {"op": "wheel", "x": x, "y": y, "notches": n, "interval_ms": 16}   (negative notches zoom out)
    {"op": "keys", "x": x, "y": y, "keys": ["ArrowRight"], "ms": 1500}  (holds keys with the pointer over the map)
    {"op": "drag", "points": [[x, y], ...], "interval_ms": 16}          (a middle-button drag)
    {"op": "click", "x": x, "y": y}                                     (a primary click on the map)
    {"op": "mods", "ctrl": b, "shift": b, "alt": b}                     (the modifiers held from then on)
    {"op": "camera", "view": id, "center": [x, z], "height": h}         (sets a repeatable map camera pose; or "reset": true)
    {"op": "save_dialog", "path": "C:/...png", "seconds": 20}          (fills and confirms the host's save dialog)
    {"op": "color_dialog", "rgb": [r, g, b], "seconds": 10}            (fills and confirms a colour input's dialog)
"""

import argparse
import ctypes as c
import json
import math
import socket
import sys
import time
from ctypes import wintypes as w

u = c.WinDLL("user32", use_last_error=True)
u.SetProcessDpiAwarenessContext.argtypes = [c.c_void_p]
u.SetProcessDpiAwarenessContext(c.c_void_p(-4))
ENUM = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
u.OpenDesktopW.argtypes = [w.LPCWSTR, w.DWORD, w.BOOL, w.DWORD]
u.OpenDesktopW.restype = w.HANDLE
u.SetThreadDesktop.argtypes = [w.HANDLE]
u.EnumDesktopWindows.argtypes = [w.HANDLE, ENUM, w.LPARAM]
u.EnumChildWindows.argtypes = [w.HWND, ENUM, w.LPARAM]
u.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
u.IsWindowVisible.argtypes = [w.HWND]
u.GetWindowRect.argtypes = [w.HWND, c.POINTER(w.RECT)]
u.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
u.GetClassNameW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
u.GetDlgCtrlID.argtypes = [w.HWND]
u.ClientToScreen.argtypes = [w.HWND, c.POINTER(w.POINT)]
u.ScreenToClient.argtypes = [w.HWND, c.POINTER(w.POINT)]
u.SendMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
u.SendMessageW.restype = w.LPARAM
WM_SETTEXT, BM_CLICK = 0x000C, 0x00F5
APP_TITLES = ("BattlePlan (Alpha)", "BattlePlan (Nightly)")
MAP_TITLE = "BattlePlan map"


def checked(value):
    if not value:
        raise c.WinError(c.get_last_error())
    return value


def epoch_ms():
    return time.time() * 1000.0


class Desktop:
    def __init__(self, name, app_pid, address):
        self.handle = checked(u.OpenDesktopW(name, 0, False, 0x1FF))
        checked(u.SetThreadDesktop(self.handle))
        self.app_pid = app_pid
        self.address = address
        self.channel = None
        self.channel_reader = None

    def close_channel(self):
        reader, channel = self.channel_reader, self.channel
        self.channel_reader = None
        self.channel = None
        for handle in (reader, channel):
            if handle is not None:
                try:
                    handle.close()
                except OSError:
                    pass

    def send(self, *lines):
        """Sends events to the viewer's suite input channel; the epoch milliseconds the first left at."""
        try:
            if self.channel is None:
                host, port = self.address.rsplit(":", 1)
                self.channel = socket.create_connection((host, int(port)), timeout=10)
                self.channel.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                self.channel_reader = self.channel.makefile("r", encoding="utf-8")
            at = epoch_ms()
            self.channel.sendall("".join(json.dumps(line) + "\n" for line in lines).encode())
            return at
        except Exception:
            self.close_channel()
            raise

    def send_and_receive(self, line):
        at = self.send(line)
        try:
            raw = self.channel_reader.readline()
            if not raw:
                raise RuntimeError("the viewer closed the suite input channel before replying")
            return at, json.loads(raw)
        except Exception:
            self.close_channel()
            raise

    def windows(self):
        """Every visible window on the desktop, children included: (hwnd, pid, rect, title, class, id)."""
        found = []

        def collect(hwnd, _):
            if u.IsWindowVisible(hwnd):
                pid, rect = w.DWORD(), w.RECT()
                u.GetWindowThreadProcessId(hwnd, c.byref(pid))
                u.GetWindowRect(hwnd, c.byref(rect))
                title, kind = c.create_unicode_buffer(256), c.create_unicode_buffer(128)
                u.GetWindowTextW(hwnd, title, 256)
                u.GetClassNameW(hwnd, kind, 128)
                found.append((hwnd, pid.value, (rect.left, rect.top, rect.right, rect.bottom), title.value, kind.value, u.GetDlgCtrlID(hwnd)))
            return True

        callback = ENUM(collect)
        u.EnumDesktopWindows(self.handle, callback, 0)
        for hwnd, *_ in list(found):
            u.EnumChildWindows(hwnd, callback, 0)
        return list({entry[0]: entry for entry in found}.values())

    def point(self, x, y):
        """The app client point (x, y) in client pixels of the map's window under it."""
        main = next((hwnd for hwnd, pid, _, title, _, _ in self.windows() if pid == self.app_pid and title in APP_TITLES), None)
        if main is None:
            raise RuntimeError("the app's window is gone")
        point = w.POINT(round(x), round(y))
        checked(u.ClientToScreen(main, c.byref(point)))
        under = [(hwnd, (r[2] - r[0]) * (r[3] - r[1])) for hwnd, pid, r, title, *_ in self.windows()
                 if r[0] <= point.x < r[2] and r[1] <= point.y < r[3] and pid == self.app_pid and title == MAP_TITLE]
        if not under:
            raise RuntimeError(f"no map window under screen point {point.x},{point.y}")
        checked(u.ScreenToClient(min(under, key=lambda item: item[1])[0], c.byref(point)))
        return [point.x, point.y]

    def move(self, points, interval_ms):
        posted = []
        for x, y in points:
            posted.append(self.send({"move": self.point(x, y)}))
            time.sleep(interval_ms / 1000)
        return {"posted": posted}

    def wheel(self, x, y, notches, interval_ms):
        self.send({"move": self.point(x, y)})
        posted = []
        for _ in range(abs(notches)):
            posted.append(self.send({"wheel": 1 if notches > 0 else -1}))
            time.sleep(interval_ms / 1000)
        return {"posted": posted}

    def keys(self, x, y, keys, ms):
        """Holds `keys` for `ms` with the pointer over the map: pressed together, then released."""
        self.send({"move": self.point(x, y)})
        posted = [self.send(*({"key": key, "pressed": True} for key in keys))]
        time.sleep(ms / 1000)
        self.send(*({"key": key, "pressed": False} for key in keys))
        return {"posted": posted}

    def drag(self, points, interval_ms):
        """A middle-button drag through `points`."""
        start = self.point(*points[0])
        posted = [self.send({"move": start}, {"button": "middle", "at": start, "pressed": True})]
        at = start
        for x, y in points[1:]:
            time.sleep(interval_ms / 1000)
            at = self.point(x, y)
            posted.append(self.send({"move": at}))
        self.send({"button": "middle", "at": at, "pressed": False})
        return {"posted": posted}

    def click(self, x, y):
        at = self.point(x, y)
        posted = self.send({"move": at}, {"button": "primary", "at": at, "pressed": True})
        time.sleep(0.05)
        self.send({"button": "primary", "at": at, "pressed": False})
        return {"posted": [posted]}

    def mods(self, ctrl=False, shift=False, alt=False):
        return {"posted": [self.send({"ctrl": ctrl, "shift": shift, "alt": alt})]}

    def camera(self, view=None, center=None, height=None, reset=False):
        payload = {}
        if view is not None:
            payload["view"] = str(view)
        if reset:
            if center is not None or height is not None:
                raise ValueError("camera reset takes neither center nor height")
            payload["reset"] = True
        else:
            if not isinstance(center, (list, tuple)) or len(center) != 2 or height is None:
                raise ValueError("camera placement requires center [x, z] and height")
            cx, cz, h = float(center[0]), float(center[1]), float(height)
            if not (math.isfinite(cx) and math.isfinite(cz) and math.isfinite(h) and h > 0):
                raise ValueError("camera placement requires finite center and positive height")
            payload["center"] = [cx, cz]
            payload["height"] = h
        at, reply = self.send_and_receive({"camera": payload})
        if not isinstance(reply, dict):
            raise RuntimeError("viewer returned a non-object camera reply")
        if reply.get("error"):
            raise RuntimeError(str(reply["error"]))
        resolved_center = reply.get("center")
        resolved_height = reply.get("height")
        if (
            not isinstance(resolved_center, (list, tuple))
            or len(resolved_center) != 2
            or not all(isinstance(v, (int, float)) and math.isfinite(v) for v in resolved_center)
            or not isinstance(resolved_height, (int, float))
            or not math.isfinite(resolved_height)
            or resolved_height <= 0
        ):
            raise RuntimeError(f"viewer returned an invalid resolved camera pose: {reply!r}")
        return {"posted": [at], "center": [float(resolved_center[0]), float(resolved_center[1])], "height": float(resolved_height)}

    def save_dialog(self, path, seconds):
        """Fills the host's save dialog's file name and confirms it, as a user does."""
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            dialogs = [entry for entry in self.windows() if entry[4] == "#32770"]
            if dialogs:
                children = [entry for entry in self.windows() if entry[0] != dialogs[0][0]]
                field = next((entry for entry in children if entry[4] == "Edit" and entry[5] in (1001, 1148)), None)
                button = next((entry for entry in children if entry[4] == "Button" and entry[5] == 1), None)
                if field and button:
                    buffer = c.create_unicode_buffer(path)
                    u.SendMessageW(field[0], WM_SETTEXT, 0, c.cast(buffer, c.c_void_p).value)
                    u.SendMessageW(button[0], BM_CLICK, 0, 0)
                    return {"saved": path}
            time.sleep(0.1)
        raise RuntimeError(f"no save dialog within {seconds} s")


    def color_dialog(self, rgb, seconds):
        """Fills the colour dialog's red, green and blue fields and confirms it, as a user does. The dialog opens with its
        custom colour fields folded away: they are opened through its own "Define Custom Colors" button. A dialog that
        shows and offers no fields fails at once, and is cancelled so the app is not left behind a modal dialog."""
        deadline = time.monotonic() + seconds
        expanded = False
        settled = None
        while time.monotonic() < deadline:
            dialogs = [entry for entry in self.windows() if entry[4] == "#32770"]
            if not dialogs:
                settled = None
            else:
                settled = settled or time.monotonic() + 1.0
                children = [entry for entry in self.windows() if entry[0] != dialogs[0][0]]
                fields = {entry[5]: entry for entry in children if entry[4] == "Edit" and entry[5] in (706, 707, 708)}
                button = next((entry for entry in children if entry[4] == "Button" and entry[5] == 1), None)
                if len(fields) == 3 and button:
                    for control, value in zip((706, 707, 708), rgb):
                        buffer = c.create_unicode_buffer(str(int(value)))
                        u.SendMessageW(fields[control][0], WM_SETTEXT, 0, c.cast(buffer, c.c_void_p).value)
                    u.SendMessageW(button[0], BM_CLICK, 0, 0)
                    return {"chosen": list(rgb)}
                mix = next((entry for entry in children if entry[4] == "Button" and entry[5] == 719), None)
                if mix and not expanded:
                    u.SendMessageW(mix[0], BM_CLICK, 0, 0)
                    expanded = True
                    settled = time.monotonic() + 1.0
                elif button and time.monotonic() > settled:
                    cancel = next((entry for entry in children if entry[4] == "Button" and entry[5] == 2), None)
                    if cancel:
                        u.SendMessageW(cancel[0], BM_CLICK, 0, 0)
                    shown = sorted({f"{entry[4]} {entry[5]}" for entry in children})
                    raise RuntimeError(f"the colour dialog showed no red, green and blue fields{' after Define Custom Colors' if expanded else ''} (it holds {', '.join(shown[:12])})")
            time.sleep(0.05)
        raise RuntimeError(f"no colour dialog with red, green and blue fields within {seconds} s")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--desktop", required=True)
    parser.add_argument("--app-pid", type=int, required=True)
    parser.add_argument("--input", required=True, help="the viewer's suite input address")
    args = parser.parse_args()
    desktop = Desktop(args.desktop, args.app_pid, args.input)
    try:
        for line in sys.stdin:
            if not line.strip():
                continue
            try:
                command = json.loads(line)
                op = command.pop("op")
                reply = {"move": desktop.move, "wheel": desktop.wheel, "keys": desktop.keys, "drag": desktop.drag, "click": desktop.click,
                         "mods": desktop.mods, "camera": desktop.camera, "save_dialog": desktop.save_dialog, "color_dialog": desktop.color_dialog}[op](**command)
            except Exception as error:
                reply = {"error": f"{type(error).__name__}: {error}"}
            print(json.dumps(reply), flush=True)
    finally:
        desktop.close_channel()


if __name__ == "__main__":
    main()
