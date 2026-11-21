"""The release smoke (Python 3.11+): the installer's layout holds exactly the shipped files, and the
installed BattlePlan opens a recording and draws its map. Main's CI runs the layout on each release
candidate (`--layout-only`: a hosted runner has no game to draw from), and tools/release.py runs it
again on the signed installer it publishes.

    release_smoke.py --installer <setup.exe> --out <new folder> [--layout-only | [--recording <file>] [--hour H]
        [--game-dir <folder>] [--user-dir <folder>]]

The installer is opened with 7-Zip into <out>/installed, as tools/leak-gate.py opens it, so nothing is
installed on the machine. Its executables and libraries must be exactly tools/shipped.json's: the app,
the recorder, the collector and the shader compiler's two DLLs, beside each other, with LICENSE and the
combined THIRD-PARTY-NOTICES.txt and no LICENSES folder. Then the installed replay-desktop.exe draws one
frame of the recording's map through its own window (`--window-dump`), with LOCALAPPDATA and APPDATA in
<out>, so no user data or saved preference is read or written. The recording and folders default to
BATTLE_PLAN_RELEASE_SMOKE_RECORDING, BATTLE_PLAN_GAME_DIR and BATTLE_PLAN_USER_DIR, or their older HOI4_ names.
"""
import argparse
import json
import os
import shutil
import struct
import subprocess
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NSIS_INTERNAL = ("uninstall.exe",)
FRAME = (1280, 720)
SECRET_PREFIXES = ("TAURI_SIGNING_", "TAURI_PRIVATE_KEY", "AZURE_", "LEAK_GATE_TERMS", "GH_TOKEN", "GITHUB_TOKEN")
DRAW_SECONDS = 600


class SmokeFailed(Exception):
    """Why the smoke did not pass."""


def fail(message):
    raise SmokeFailed(message)


def shipped_binaries(root=ROOT):
    """The executables and libraries the installer must place, all in the install folder."""
    shipped = json.loads((root / "tools" / "shipped.json").read_text(encoding="utf-8"))
    return sorted([shipped["main"], *shipped["built"], *shipped["compiler"]], key=str.lower)


def shipped_notices():
    """The licence files the installer must place in the install folder."""
    shipped = json.loads((ROOT / "tools" / "shipped.json").read_text(encoding="utf-8"))
    return sorted([*shipped["repository"], shipped["notices"]["file"]], key=str.lower)


def layout_problems(installed, expected, notices):
    """What is wrong with the installed folder: an expected binary or notice missing or not beside the
    app, any other executable or library of ours, or a notice left in the old LICENSES folder."""
    found, present = [], []
    for path in sorted(installed.rglob("*")):
        relative = path.relative_to(installed)
        if not path.is_file() or relative.parts[0] in NSIS_INTERNAL or relative.parts[0].startswith("$"):
            continue
        if path.suffix.lower() in (".exe", ".dll"):
            found.append(relative.as_posix())
        present.append(relative.as_posix())
    problems = [f"{name} is not in the install folder" for name in [*expected, *notices] if name not in present]
    problems += [f"{name} is installed but not shipped" for name in found if name not in expected]
    if (installed / "LICENSES").exists():
        problems.append("the old LICENSES folder is installed")
    return problems


def open_installer(installer, into):
    tool = shutil.which("7z") or r"C:\Program Files\7-Zip\7z.exe"
    if not Path(tool).exists():
        fail("7-Zip was not found, so the installer could not be opened.")
    opened = subprocess.run([tool, "x", "-y", f"-o{into}", str(installer)], capture_output=True)
    if opened.returncode != 0:
        fail(f"The installer could not be opened (7-Zip exit {opened.returncode}).")


def png_colours(path, most=64):
    """How many colours an 8-bit RGB or RGBA PNG holds, counting no further than `most`."""
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    at, chunks, header = 8, [], None
    while at < len(data):
        length, kind = struct.unpack(">I4s", data[at:at + 8])
        body = data[at + 8:at + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            chunks.append(body)
        at += 12 + length
    width, height, depth, colour, _, _, interlace = header
    if depth != 8 or colour not in (2, 6) or interlace:
        raise ValueError("not an 8-bit RGB or RGBA PNG")
    stride = 3 if colour == 2 else 4
    raw = zlib.decompress(b"".join(chunks))
    row_bytes = width * stride
    previous = bytearray(row_bytes)
    colours = set()
    for row in range(height):
        start = row * (row_bytes + 1)
        kind, line = raw[start], bytearray(raw[start + 1:start + 1 + row_bytes])
        for index in range(row_bytes):
            left = line[index - stride] if index >= stride else 0
            up = previous[index]
            corner = previous[index - stride] if index >= stride else 0
            if kind == 1:
                line[index] = (line[index] + left) & 0xFF
            elif kind == 2:
                line[index] = (line[index] + up) & 0xFF
            elif kind == 3:
                line[index] = (line[index] + (left + up) // 2) & 0xFF
            elif kind == 4:
                guess = left + up - corner
                near = min((abs(guess - left), left), (abs(guess - up), up), (abs(guess - corner), corner), key=lambda pair: pair[0])
                line[index] = (line[index] + near[1]) & 0xFF
        for pixel in range(0, row_bytes, stride * 7):
            colours.add(bytes(line[pixel:pixel + 3]))
            if len(colours) >= most:
                return len(colours)
        previous = line
    return len(colours)


def draw(app, recording, out, hour, game_dir, user_dir):
    """The installed app draws one frame of the recording's map, given no signing key, release
    secret or token; the frame, or why not."""
    local, roaming = out / "local", out / "roaming"
    local.mkdir()
    roaming.mkdir()
    frame = out / "frame.png"
    command = [str(app), "--window-dump", str(recording), "--out", str(frame), "--width", str(FRAME[0]), "--height", str(FRAME[1])]
    if hour is not None:
        command += ["--hour", str(hour)]
    if game_dir:
        command += ["--game-dir", str(game_dir)]
    if user_dir:
        command += ["--user-dir", str(user_dir)]
    environment = {name: value for name, value in os.environ.items() if not name.upper().startswith(SECRET_PREFIXES)}
    environment.update(LOCALAPPDATA=str(local), APPDATA=str(roaming))
    try:
        drawn = subprocess.run(command, cwd=out, env=environment, capture_output=True, timeout=DRAW_SECONDS)
    except subprocess.TimeoutExpired:
        fail(f"The installed app drew no frame in {DRAW_SECONDS} s.")
    (out / "draw.log").write_bytes(drawn.stdout + drawn.stderr)
    if drawn.returncode != 0:
        fail(f"The installed app could not draw the map (exit {drawn.returncode}): "
             f"{drawn.stderr.decode('utf-8', 'replace').strip()[-2000:]}")
    if not frame.is_file():
        fail("The installed app exited without writing its frame.")
    return frame


def setting(name):
    return os.environ.get(f"BATTLE_PLAN_{name}") or os.environ.get(f"HOI4_{name}") or None


def arguments(argv=None):
    """The command line, every path in it as the caller's folder names it: the app runs elsewhere."""
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--installer", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--layout-only", action="store_true")
    parser.add_argument("--recording", type=Path, default=setting("RELEASE_SMOKE_RECORDING"))
    parser.add_argument("--hour", type=int)
    parser.add_argument("--game-dir", type=Path, default=setting("GAME_DIR"))
    parser.add_argument("--user-dir", type=Path, default=setting("USER_DIR"))
    args = parser.parse_args(argv)
    for name in ("installer", "out", "recording", "game_dir", "user_dir"):
        if getattr(args, name) is not None:
            setattr(args, name, Path(os.path.abspath(getattr(args, name))))
    return args


def installed_layout(installer, out, expected=None, notices=None):
    """The installer opened into <out>/installed and its layout checked against `expected`, by
    default this checkout's shipped binaries; the install folder."""
    opened = out / "installed"
    open_installer(installer, opened)
    installed = opened / "$INSTDIR" if (opened / "$INSTDIR").is_dir() else opened
    problems = layout_problems(installed, expected or shipped_binaries(), notices or shipped_notices())
    if problems:
        fail("The installer's layout is not the shipped one:\n" + "\n".join(problems))
    return installed


def drawn_map(installed, recording, out, hour=None, game_dir=None, user_dir=None):
    """The installed app draws the recording's map: the frame, and how many colours it holds, 16 or more."""
    frame = draw(installed / "replay-desktop.exe", recording, out, hour, game_dir, user_dir)
    try:
        colours = png_colours(frame)
    except (ValueError, struct.error, zlib.error, TypeError) as error:
        fail(f"The frame is not a picture the smoke reads: {error}")
    if colours < 16:
        fail(f"The frame holds {colours} colours: the map was not drawn.")
    return frame, colours


def smoke(args):
    if not args.layout_only and (not args.recording or not args.recording.is_file()):
        fail("Name the recording to draw: --recording or BATTLE_PLAN_RELEASE_SMOKE_RECORDING, or --layout-only.")
    if args.out.exists():
        fail(f"{args.out} exists; the smoke writes to a new folder.")
    args.out.mkdir(parents=True)
    installed = installed_layout(args.installer, args.out)
    print(f"Layout: exactly {', '.join(shipped_binaries())}, with {' and '.join(shipped_notices())}.")
    if args.layout_only:
        return
    frame, colours = drawn_map(installed, args.recording, args.out, args.hour, args.game_dir, args.user_dir)
    print(f"Drew the recording's map from the installed layout: {frame} ({colours}+ colours).")


def main():
    try:
        smoke(arguments())
    except SmokeFailed as error:
        print(error, file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
