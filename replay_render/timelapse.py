"""Time-lapse builder: a sequence of Frames -> a PNG frame sequence (+ optional
GIF / ffmpeg MP4 assembly).

Outputs land under ``replay_render/output/`` (gitignored -- large generated
media must never be committed). The PNG sequence is the primary, always-emitted
deliverable; GIF/MP4 are best-effort conveniences:

  * GIF  -- built with pure stdlib if Pillow is present (animated GIF). Skipped
            with a note if Pillow is missing (frames are still on disk).
  * MP4  -- built by shelling out to ``ffmpeg`` IF it is on PATH; skipped with a
            note otherwise. We never bundle or require ffmpeg.

The builder reuses ONE ``FrameRenderer`` across all frames (geometry decoded
once) so a hundreds-of-frame replay only repeats the cheap recolor pass.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path
from typing import Iterable, List, Optional

import numpy as np

from .frames import Frame
from .geometry import MapGeometry, load_geometry
from .render import FrameRenderer, write_png_rgb


def _output_dir() -> Path:
    return Path(__file__).resolve().parent / "output"


# Buffering all rendered frames for a GIF is ~8.65 MB/frame at half-res, so a
# full-game (~365 daily frames) would hold ~3 GB in RAM before Pillow. GIF is
# therefore OPT-IN and capped; the PNG sequence is always the safe primary
# output. Above this many frames we refuse to buffer for a GIF and tell the user
# to assemble from the PNG sequence (or use the streaming ffmpeg MP4 path).
GIF_FRAME_CAP = 120


def build_timelapse(frames: Iterable[Frame],
                    geom: Optional[MapGeometry] = None,
                    out_dir: Optional[Path] = None,
                    name: str = "replay",
                    show_front_lines: bool = True,
                    show_divisions: bool = False,
                    control_view: bool = False,
                    make_gif: bool = False,
                    gif_fps: int = 4,
                    make_mp4: bool = False,
                    mp4_fps: int = 8,
                    half_res: bool = True,
                    progress: bool = True) -> dict:
    """Render ``frames`` to ``out_dir/<name>/frame_####.png`` and optional media.

    Returns a manifest dict (frame paths, timestamps, sizes, gif/mp4 paths or the
    reason they were skipped). The manifest is also written as ``manifest.json``
    so a viewer / the dashboard can pick the sequence up later.

    ``make_gif`` is OPT-IN (default False) because it buffers all frames in RAM;
    it is auto-skipped past ``GIF_FRAME_CAP`` frames. The PNG sequence is always
    written. For long replays prefer the MP4 path (ffmpeg streams from disk).
    """
    frames = list(frames)
    if not frames:
        raise ValueError("build_timelapse: no frames")

    if geom is None:
        geom = load_geometry(half_res=half_res, with_centroids=show_divisions)
    renderer = FrameRenderer(geom, show_front_lines=show_front_lines,
                             show_divisions=show_divisions,
                             control_view=control_view)

    root = (out_dir or _output_dir()) / name
    root.mkdir(parents=True, exist_ok=True)

    gif_too_long = make_gif and len(frames) > GIF_FRAME_CAP
    buffer_for_gif = make_gif and not gif_too_long

    frame_files: List[str] = []
    rendered: List[np.ndarray] = []
    for i, fr in enumerate(frames):
        img = renderer.render(fr)
        fname = f"frame_{i:04d}.png"
        write_png_rgb(root / fname, img)
        frame_files.append(fname)
        if buffer_for_gif:
            rendered.append(img)
        if progress:
            print(f"  [{i + 1}/{len(frames)}] {fr.timestamp} -> {fname}")

    manifest = {
        "name": name,
        "width": geom.width,
        "height": geom.height,
        "half_res": geom.half_res,
        "frame_count": len(frames),
        "timestamps": [fr.timestamp for fr in frames],
        "frames": frame_files,
        "front_lines": show_front_lines,
        "divisions": show_divisions,
        "gif": None,
        "mp4": None,
    }

    if make_gif:
        if gif_too_long:
            manifest["gif"] = None
            manifest["gif_skipped_reason"] = (
                f"{len(frames)} frames > GIF_FRAME_CAP={GIF_FRAME_CAP}; "
                f"assemble from the PNG sequence or use the MP4 path")
        else:
            gif_path = root / f"{name}.gif"
            ok, reason = _write_gif(gif_path, rendered, fps=gif_fps)
            manifest["gif"] = str(gif_path.name) if ok else None
            manifest["gif_skipped_reason"] = None if ok else reason

    if make_mp4:
        mp4_path = root / f"{name}.mp4"
        ok, reason = _write_mp4(mp4_path, root, frame_files, fps=mp4_fps)
        manifest["mp4"] = str(mp4_path.name) if ok else None
        manifest["mp4_skipped_reason"] = None if ok else reason

    import json
    (root / "manifest.json").write_text(json.dumps(manifest, indent=1), encoding="utf-8")
    return manifest


def _write_gif(path: Path, images: List[np.ndarray], fps: int) -> tuple:
    """Animated GIF via Pillow if available; else skip with a reason."""
    try:
        from PIL import Image
    except Exception:
        return False, "Pillow not installed (PNG frames written; GIF skipped)"
    if not images:
        return False, "no frames buffered"
    pil = [Image.fromarray(im, mode="RGB").convert("P", palette=Image.ADAPTIVE)
           for im in images]
    duration_ms = max(1, int(1000 / max(1, fps)))
    pil[0].save(path, save_all=True, append_images=pil[1:],
                duration=duration_ms, loop=0, disposal=2)
    return True, ""


def _write_mp4(path: Path, frame_dir: Path, frame_files: List[str], fps: int) -> tuple:
    """MP4 via ffmpeg IF on PATH; else skip with a reason (never required)."""
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        return False, "ffmpeg not on PATH (PNG frames written; MP4 skipped)"
    pattern = str(frame_dir / "frame_%04d.png")
    cmd = [ffmpeg, "-y", "-framerate", str(fps), "-i", pattern,
           "-c:v", "libx264", "-pix_fmt", "yuv420p", str(path)]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
    except Exception as e:  # noqa: BLE001
        return False, f"ffmpeg failed to launch: {e}"
    if res.returncode != 0:
        return False, f"ffmpeg exit {res.returncode}: {res.stderr[-300:]}"
    return True, ""
