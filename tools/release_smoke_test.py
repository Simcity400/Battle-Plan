import os
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release_smoke as rs

FIVE = ["battleplan-recorder.exe", "collector.dll", "dxcompiler.dll", "dxil.dll", "replay-desktop.exe"]
NOTICES = ["LICENSE", "THIRD-PARTY-NOTICES.txt"]


def png(path, width, height, pixel, filters=(0,)):
    """An RGB PNG whose rows use `filters` in turn, `pixel(x, y)` each pixel's colour."""
    raw = bytearray()
    previous = bytes(width * 3)
    for y in range(height):
        kind = filters[y % len(filters)]
        line = bytes(channel for x in range(width) for channel in pixel(x, y))
        if kind == 2:
            encoded = bytes((line[i] - previous[i]) & 0xFF for i in range(len(line)))
        elif kind == 1:
            encoded = bytes((line[i] - (line[i - 3] if i >= 3 else 0)) & 0xFF for i in range(len(line)))
        else:
            encoded = line
        raw += bytes([kind]) + encoded
        previous = line

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(bytes(raw))) + chunk(b"IEND", b""))


class ReleaseSmoke(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def tree(self, *names):
        for name in names:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"x")
        return self.root


    def test_exactly_the_five_beside_each_other_pass_with_licences_and_installer_internals(self):
        installed = self.tree(*FIVE, *NOTICES, "$PLUGINSDIR/nsis_tauri_utils.dll")
        self.assertEqual(rs.layout_problems(installed, FIVE, NOTICES), [])

    def test_the_old_licences_folder_or_a_missing_combined_notice_is_named(self):
        installed = self.tree(*FIVE, "LICENSE", "LICENSES/SDL-LICENSE.txt")
        self.assertEqual(rs.layout_problems(installed, FIVE, NOTICES), [
            "THIRD-PARTY-NOTICES.txt is not in the install folder",
            "the old LICENSES folder is installed",
        ])
        (installed / "LICENSES" / "SDL-LICENSE.txt").unlink()
        self.tree("THIRD-PARTY-NOTICES.txt")
        self.assertEqual(rs.layout_problems(installed, FIVE, NOTICES), ["the old LICENSES folder is installed"])

    def test_an_old_viewer_or_compiler_executable_or_a_missing_or_moved_dll_is_named(self):
        installed = self.tree("replay-desktop.exe", "battleplan-recorder.exe", "collector.dll", "viewer.exe",
                              "shader-tools/dxc/dxc.exe", "shader-tools/dxc/dxcompiler.dll", "shader-tools/dxc/dxil.dll")
        self.assertEqual(rs.layout_problems(installed, FIVE, []), [
            "dxcompiler.dll is not in the install folder",
            "dxil.dll is not in the install folder",
            "shader-tools/dxc/dxc.exe is installed but not shipped",
            "shader-tools/dxc/dxcompiler.dll is installed but not shipped",
            "shader-tools/dxc/dxil.dll is installed but not shipped",
            "viewer.exe is installed but not shipped",
        ])

    def test_a_drawn_frame_has_many_colours_and_a_blank_one_has_one(self):
        drawn, blank = self.root / "drawn.png", self.root / "blank.png"
        png(drawn, 64, 32, lambda x, y: (x * 4, y * 8, (x + y) % 256), filters=(0, 1, 2))
        png(blank, 64, 32, lambda x, y: (12, 34, 56), filters=(2, 1))
        self.assertGreaterEqual(rs.png_colours(drawn), 16)
        self.assertEqual(rs.png_colours(blank), 1)
        (self.root / "text.png").write_bytes(b"not a picture")
        with self.assertRaises(ValueError):
            rs.png_colours(self.root / "text.png")

    def test_every_path_is_the_one_the_caller_named_though_the_app_runs_in_out(self):
        caller = self.root / "caller"
        caller.mkdir()
        here = Path.cwd()
        os.chdir(caller)
        try:
            args = rs.arguments(["--installer", "setup.exe", "--out", "smoke", "--recording", "campaign.battleplan",
                                 "--game-dir", "game", "--user-dir", "../user"])
        finally:
            os.chdir(here)
        self.assertEqual(
            [args.installer, args.out, args.recording, args.game_dir, args.user_dir],
            [caller / "setup.exe", caller / "smoke", caller / "campaign.battleplan", caller / "game", self.root / "user"],
        )


if __name__ == "__main__":
    unittest.main()
