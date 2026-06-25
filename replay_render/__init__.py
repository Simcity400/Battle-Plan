"""replay_render -- offline map / division time-lapse renderer for Battle Plan.

Turns a sequence of replay frames into BOTH a PNG frame sequence (for video/GIF)
and an interactive HTML scrubber. Pure offline Python; the game need not be
running. Consumes the ``map_pipeline/assets/`` geometry read-only and is
schema-agnostic via the generic ``Frame`` interface in ``frames.py`` (a thin
adapter maps the eventual ``replay/SCHEMA.md`` snapshot onto it).

Public surface:
    Frame, DivisionMarker            -- the generic frame interface (frames.py)
    frame_from_ownership_dict        -- adapter for save_oracle ownership JSON
    frame_from_replay_snapshot       -- adapter for replay/SCHEMA.md (placeholder)
    load_geometry / MapGeometry      -- read-only map asset loader (geometry.py)
    FrameRenderer                    -- one Frame -> RGB ndarray (render.py)
    build_timelapse                  -- Frames -> PNG sequence (+GIF/MP4) (timelapse.py)
    build_scrubber_html              -- Frames -> interactive HTML (scrubber.py)
"""

from .frames import (
    Frame,
    DivisionMarker,
    frame_from_ownership_dict,
    frame_from_replay_snapshot,
    frames_from_replay_series,
)
from .geometry import MapGeometry, load_geometry
from .render import FrameRenderer, write_png_rgb
from .timelapse import build_timelapse
from .scrubber import build_scrubber_html

__all__ = [
    "Frame", "DivisionMarker",
    "frame_from_ownership_dict", "frame_from_replay_snapshot", "frames_from_replay_series",
    "MapGeometry", "load_geometry",
    "FrameRenderer", "write_png_rgb",
    "build_timelapse", "build_scrubber_html",
]
