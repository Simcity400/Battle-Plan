"""Single-frame renderer: a Frame -> an HxWx3 RGB image.

Mirrors the recolor semantics of ``map_view/app.js`` (owner fill; controller
diagonal hatch for contested states via ``((x+y)&7)<4``) so PNG frames and the
HTML scrubber stay visually identical, then adds two replay layers:

  * front lines  -- derived from controller adjacency (a pixel whose controller
                    differs from a 4-neighbour's controller is drawn dark).
  * division markers -- dots at province centroids, sized by strength, coloured
                    by owner, drawn on top.

Pure numpy. The same colour-resolution chain as app.js / map_pipeline:
    pixel -> province id -> state id -> Frame owner/controller TAG -> colour.
"""

from __future__ import annotations

import math
import struct
import zlib
from typing import Dict, Optional, Tuple

import numpy as np

from .frames import Frame
from .geometry import MapGeometry

# colours match map_view/app.js
WATER = (38, 50, 66)
NO_OWNER = (120, 120, 120)
UNKNOWN = (70, 60, 60)
FRONT_LINE = (12, 12, 14)
MARKER_EDGE = (12, 12, 14)


# --------------------------------------------------------------------------
# PNG writer -- same stdlib technique as map_pipeline/build_map_assets.py
# (color type 2, 8-bit, filter 0). Kept local so replay_render has no import
# dependency on map_pipeline (which we must not edit/import per the boundary).
# --------------------------------------------------------------------------

def write_png_rgb(path, rgb: np.ndarray) -> None:
    h, w, _ = rgb.shape
    rgb = np.ascontiguousarray(rgb, dtype=np.uint8)

    def chunk(tag: bytes, payload: bytes) -> bytes:
        out = struct.pack(">I", len(payload)) + tag + payload
        crc = zlib.crc32(tag + payload) & 0xFFFFFFFF
        return out + struct.pack(">I", crc)

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)
    raw = np.empty((h, w * 3 + 1), dtype=np.uint8)
    raw[:, 0] = 0
    raw[:, 1:] = rgb.reshape(h, w * 3)
    compressed = zlib.compress(raw.tobytes(), 6)
    with open(path, "wb") as fp:
        fp.write(sig)
        fp.write(chunk(b"IHDR", ihdr))
        fp.write(chunk(b"IDAT", compressed))
        fp.write(chunk(b"IEND", b""))


# --------------------------------------------------------------------------
# helpers
# --------------------------------------------------------------------------

def _resolve_color(val, colors: Dict[str, Tuple[int, int, int]]) -> Tuple[int, int, int]:
    """A Frame owner value is a TAG or a literal (r,g,b)."""
    if isinstance(val, (tuple, list)):
        return (int(val[0]), int(val[1]), int(val[2]))
    return colors.get(val, UNKNOWN)


def _darken(rgb, f: float):
    return (min(255, int(rgb[0] * f)), min(255, int(rgb[1] * f)), min(255, int(rgb[2] * f)))


def color_coverage(frame: Frame,
                   colors: Dict[str, Tuple[int, int, int]]) -> Dict[str, object]:
    """Diagnostic: which owner/controller TAGs in ``frame`` lack a REAL game colour.

    Purely informational (the render path is unchanged): a TAG not in
    ``country_colors.json`` -- typically a dynamically released tag (``D76+``)
    whose colour the map pipeline never emitted -- renders as ``UNKNOWN``. Per
    the no-fabrication rule we DON'T invent a colour for it; this surfaces the
    gap so a caller can log it / extend the colour asset, instead of silently
    showing grey. Literal ``(r,g,b)`` owners (the synthetic demo) are not TAGs
    and never "unknown".

    Returns ``{unknown_owner_tags, unknown_controller_tags, unknown_division_tags,
    no_state_controllers}``. ``no_state_controllers`` flags the minimized-snapshot
    case Codex raised: owner is known (e.g. from ``map.owner_by_state``) but
    ``states{}`` carried no controller, so only province control (if any) is
    available.
    """
    def _missing(tags) -> list:
        out = []
        for t in tags:
            if isinstance(t, str) and t and t not in colors:
                out.append(t)
        return sorted(set(out))

    owner_tags = [v for v in frame.owner.values() if isinstance(v, str)]
    ctrl_tags = list((frame.controller or {}).values())
    prov_tags = list((frame.control_by_prov or {}).values())
    div_tags = [d.owner for d in (frame.divisions or [])]
    return {
        "unknown_owner_tags": _missing(owner_tags),
        "unknown_controller_tags": _missing(ctrl_tags + prov_tags),
        "unknown_division_tags": _missing(div_tags),
        # owner known but NO state-level controller captured (Codex weak-point #1)
        "no_state_controllers": bool(frame.owner) and not frame.controller,
    }


class FrameRenderer:
    """Holds the (immutable) geometry and renders any Frame against it.

    Build ONCE per game (decoding the raster + centroids is the expensive part);
    call ``render(frame)`` per timestamp. This is why a hundreds-of-frame
    time-lapse stays cheap: only the small per-province colour lookup tables and
    one pixel pass are redone per frame, exactly like app.js's ``recolor()``.
    """

    def __init__(self, geom: MapGeometry,
                 show_front_lines: bool = True,
                 show_divisions: bool = False,
                 control_view: bool = False):
        self.geom = geom
        self.show_front_lines = show_front_lines
        self.show_divisions = show_divisions
        # owner view (default): fill by owner, hatch toward controller.
        # control view: fill by controller, hatch toward a darkened owner.
        self.control_view = control_view
        # precompute the per-pixel hatch mask once ( ((x+y)&7)<4 ), reused every frame
        h, w = geom.height, geom.width
        xs = np.arange(w, dtype=np.int32)[None, :]
        ys = np.arange(h, dtype=np.int32)[:, None]
        self._hatch = (((xs + ys) & 7) < 4)            # HxW bool
        self._flat_ids = geom.id_raster.reshape(-1)    # HxW uint32, view

    # ----- per-province colour tables (mirror buildColorTables in app.js) ---
    def _color_tables(self, frame: Frame):
        """Build per-province packed-colour tables.

        Control resolution (SCHEMA §5/§8): province-level control
        (``frame.control_by_prov``) OVERRIDES the state-level controller for that
        province, so partial occupations / intra-state fronts render correctly.
        Where no province override exists, fall back to the state controller.
        """
        sb = self.geom.state_by_prov
        max_id = sb.shape[0] - 1
        primary = np.zeros((max_id + 1, 3), dtype=np.uint8)
        alt = np.zeros((max_id + 1, 3), dtype=np.uint8)
        contested = np.zeros(max_id + 1, dtype=bool)
        # controller per province (for front-line derivation); -1 == none
        ctrl_of_prov = np.full(max_id + 1, -1, dtype=np.int32)

        colors = self.geom.country_colors
        owner_map = frame.owner
        ctrl_map = frame.controller or {}
        prov_ctrl = frame.control_by_prov or {}

        # encode controller TAGs to small ints for the adjacency compare
        ctrl_code: Dict[str, int] = {}

        primary[0] = WATER
        alt[0] = WATER
        for prov in range(1, max_id + 1):
            st = int(sb[prov])
            if st < 0:
                primary[prov] = WATER
                alt[prov] = WATER
                continue
            own_val = owner_map.get(st)
            if not own_val:
                primary[prov] = NO_OWNER
                alt[prov] = NO_OWNER
                continue
            own_c = _resolve_color(own_val, colors)
            # A literal (r,g,b) owner gets a stable pseudo-tag '#rrggbb' (same as
            # the scrubber) so contested-hatch comparison and front-line codes
            # work uniformly for literal owners, not just TAG strings.
            owner_tag = (own_val if isinstance(own_val, str)
                         else "#%02x%02x%02x" % (own_c[0], own_c[1], own_c[2]))
            # province override first, then state controller, then owner
            ctrl_tag = prov_ctrl.get(prov)
            if ctrl_tag is None:
                sc = ctrl_map.get(st, owner_tag)
                ctrl_tag = sc if isinstance(sc, str) else owner_tag
            # control-view base = controller; owner-view base = owner. We render
            # owner-fill with a contested hatch toward controller (single image
            # carries both, matching the dashboard's combined look).
            if self.control_view and ctrl_tag:
                primary[prov] = colors.get(ctrl_tag, UNKNOWN)
            else:
                primary[prov] = own_c
            if ctrl_tag and owner_tag and ctrl_tag != owner_tag:
                contested[prov] = True
                ctrl_c = colors.get(ctrl_tag, UNKNOWN)
                # owner view: hatch toward controller colour; control view: hatch
                # toward a darkened owner ("owner under") -- matches app.js.
                alt[prov] = ctrl_c if not self.control_view else _darken(own_c, 0.55)
            else:
                alt[prov] = primary[prov]
            # controller code for front lines (effective, province-overridden,
            # controller; falls back to owner_tag which is now always set)
            key = ctrl_tag or owner_tag or "_"
            if key not in ctrl_code:
                ctrl_code[key] = len(ctrl_code) + 1
            ctrl_of_prov[prov] = ctrl_code[key]
        return primary, alt, contested, ctrl_of_prov

    # ----- the recolor pass -------------------------------------------------
    def render(self, frame: Frame) -> np.ndarray:
        geom = self.geom
        h, w = geom.height, geom.width
        primary, alt, contested, ctrl_of_prov = self._color_tables(frame)

        ids = self._flat_ids
        # base fill = primary[id]; where contested[id], use hatch to pick alt
        img = primary[ids].reshape(h, w, 3)
        if contested.any():
            cont_px = contested[ids].reshape(h, w)
            alt_img = alt[ids].reshape(h, w, 3)
            use_alt = cont_px & (~self._hatch)
            img[use_alt] = alt_img[use_alt]

        if self.show_front_lines:
            self._draw_front_lines(img, ctrl_of_prov)

        if self.show_divisions and frame.divisions:
            self._draw_divisions(img, frame)

        return img

    # ----- front lines: controller differs from a right/down neighbour ------
    def _draw_front_lines(self, img: np.ndarray, ctrl_of_prov: np.ndarray) -> None:
        ids2d = self.geom.id_raster
        ctrl = ctrl_of_prov[ids2d]                 # HxW controller code, -1 water/none
        valid = ctrl > 0
        edge = np.zeros(ctrl.shape, dtype=bool)
        # compare to right and down neighbours (cheap 4-ish connectivity)
        diff_r = valid[:, :-1] & valid[:, 1:] & (ctrl[:, :-1] != ctrl[:, 1:])
        edge[:, :-1] |= diff_r
        edge[:, 1:] |= diff_r
        diff_d = valid[:-1, :] & valid[1:, :] & (ctrl[:-1, :] != ctrl[1:, :])
        edge[:-1, :] |= diff_d
        edge[1:, :] |= diff_d
        img[edge] = FRONT_LINE

    # ----- division markers: dots at province centroids ---------------------
    def _draw_divisions(self, img: np.ndarray, frame: Frame) -> None:
        geom = self.geom
        h, w = geom.height, geom.width
        # aggregate divisions per province (sum strength, pick majority owner)
        per_prov: Dict[int, Dict] = {}
        for d in frame.divisions:
            p = per_prov.setdefault(d.province_id, {"strength": 0.0, "owners": {}})
            p["strength"] += d.strength
            p["owners"][d.owner] = p["owners"].get(d.owner, 0.0) + d.strength
        # Use the representative IN-province point (actual province pixel nearest
        # the centroid) so the marker never lands in the sea or outside a concave
        # province; fall back to the raw centroid only if points weren't computed.
        for prov, agg in per_prov.items():
            pt = geom.prov_points.get(prov)
            if pt is not None:
                cx, cy = pt
            else:
                cen = geom.prov_centroids.get(prov)
                if cen is None:
                    continue
                cx, cy = int(round(cen[0])), int(round(cen[1]))
            owner = max(agg["owners"].items(), key=lambda kv: kv[1])[0]
            col = geom.country_colors.get(owner, UNKNOWN)
            # radius scales with sqrt(strength), clamped for visibility
            r = int(max(2, min(9, round(1.5 + math.sqrt(agg["strength"]) * 0.25))))
            _draw_disc(img, cx, cy, r, col, MARKER_EDGE, w, h)


def _draw_disc(img, cx, cy, r, fill, edge, w, h) -> None:
    x0, x1 = max(0, cx - r), min(w - 1, cx + r)
    y0, y1 = max(0, cy - r), min(h - 1, cy + r)
    if x0 > x1 or y0 > y1:
        return
    ys = np.arange(y0, y1 + 1)[:, None]
    xs = np.arange(x0, x1 + 1)[None, :]
    dist2 = (xs - cx) ** 2 + (ys - cy) ** 2
    inside = dist2 <= r * r
    ring = inside & (dist2 >= (r - 1) * (r - 1))
    sub = img[y0:y1 + 1, x0:x1 + 1]
    sub[inside] = fill
    sub[ring] = edge
    img[y0:y1 + 1, x0:x1 + 1] = sub
