"""Provenance manifest + staleness gate for the frozen map_pipeline tables.

The seven JSON tables under ``replay_interface/map_pipeline/assets`` were
committed frozen: their Python generators (``build_map_assets.py``,
``build_strategic_regions.py``) were deleted when the map checkpoint moved to
Rust (commit c5120b3d), so the tables are UNREGENERABLE from this repo. Their
source provenance is explicitly UNRECORDED -- the deleted generator's default
input was ``--mod ../test``, an unrecorded sibling checkout, and no game
version or content hash of the inputs was captured at freeze time. This module
therefore records what CAN still be pinned:

  * a content hash (sha256 + fnv1a64) of each frozen table as committed, so
    any silent edit or corruption of the frozen data fails the test gate; and
  * an opt-in parity check against a locally generated Rust map bundle
    (``map_viewer`` ``bundle_gen``), the successor contract that generates
    ``province_to_state.json`` / ``state_meta.json`` fresh from installed content
    with real provenance (``definition_csv_fnv1a64``). Any difference means
    the frozen tables have drifted from the current game/mod data.

Fail-closed doctrine: ``derive/layers.py`` uses the frozen
``province_to_state.json`` as the authoritative complete base for state-level
derived layers. Because the tables cannot be regenerated, drift can only be
DETECTED (via the bundle parity gate), never silently repaired; a mismatch
must be treated as data-version drift requiring a re-freeze from the bundle,
not papered over.

The parity gate is opt-in (needs a locally generated bundle): set
``BATTLE_PLAN_BUNDLE_DIR`` to a bundle directory. When unset the test SKIPS with an
explicit reason -- it never passes silently.
"""

from __future__ import annotations

import hashlib
import json
import os
from typing import Any, Dict, List, Optional

# ---------------------------------------------------------------------------
# Locations
# ---------------------------------------------------------------------------
_HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS_DIR = os.path.normpath(
    os.path.join(_HERE, os.pardir, "map_pipeline", "assets"))

#: The frozen committed tables (the gitignored ``province_ids*.png`` rasters
#: are machine-local and are NOT part of the committed manifest).
FROZEN_TABLES = (
    "country_colors.json",
    "province_ids_meta.json",
    "province_to_state.json",
    "province_to_strategic_region.json",
    "state_meta.json",
    "strategic_region_meta.json",
    "strategic_region_to_provinces.json",
)

#: Machine-readable source-provenance record. The honest answer is UNRECORDED:
#: do not invent a game version for data whose generator and inputs are gone.
SOURCE_PROVENANCE: Dict[str, Any] = {
    "game_version": "UNRECORDED",
    "mod_identity": "UNRECORDED (generator default input was '--mod ../test', "
                    "an unrecorded sibling checkout)",
    "generator": "map_pipeline/build_map_assets.py + build_strategic_regions.py "
                 "(deleted in commit c5120b3d, 'Move map checkpoint to Rust')",
    "frozen_in_commit": "1d639adb",
    "regenerable": False,
    "note": "Regeneration is impossible until downstream callers migrate to "
            "the Rust bundle contract (map_viewer bundle_gen), which records "
            "real provenance (definition_csv_fnv1a64) per bundle.",
}

#: Content hashes of each frozen table AS COMMITTED. If a table on disk stops
#: matching, either the data was silently edited (fail: restore or re-freeze
#: deliberately with updated hashes) or the checkout is corrupt.
FROZEN_TABLE_HASHES: Dict[str, Dict[str, Any]] = {
    "country_colors.json": {
        "bytes": 5405,
        "sha256": "83a02a762a56b2a6398083270106becc73123fb420eab7f3562c52c23e0802c0",
        "fnv1a64": "1b8f18449d066cd8",
    },
    "province_ids_meta.json": {
        "bytes": 1110,
        "sha256": "6557d3a0a0bf34b1640a2f486ecd27db5e88fcffc45f771d5eb7c1da915dffc6",
        "fnv1a64": "c0560b794c195df9",
    },
    "province_to_state.json": {
        "bytes": 115036,
        "sha256": "a6cd13291f972935deceef003bc9f1c7204056103e025185ea88dcb84455f5a7",
        "fnv1a64": "f0f363d6f239707e",
    },
    "province_to_strategic_region.json": {
        "bytes": 144869,
        "sha256": "1f6e387f795ab43d7e4f067085e5f07590e7d50b4c4a513b918fecc9f9e8942d",
        "fnv1a64": "0ed226a1169cbbef",
    },
    "state_meta.json": {
        "bytes": 305576,
        "sha256": "c2a8ec09ec1b7fce374325ec462f9a7d48f3c3a9b48c5f2f8ebee6b90875dbf1",
        "fnv1a64": "17bd52c2483cc098",
    },
    "strategic_region_meta.json": {
        "bytes": 22282,
        "sha256": "8116cfcb7a7c7bacf741a92ecbb538560ca90b39c5fac1b338f242c4becc9cc3",
        "fnv1a64": "60c11b50d8700f65",
    },
    "strategic_region_to_provinces.json": {
        "bytes": 71699,
        "sha256": "d5c4d34a9b408d05843be78201609ec275b9657ef18f81070e747cf672ad296d",
        "fnv1a64": "bcafe4bf27352f3f",
    },
}

#: state_meta fields shared between the frozen table and the Rust bundle's
#: StateMeta. The bundle additionally serializes ``impassable`` (and its
#: ``state_category`` is an Option that may be null); the frozen table
#: predates that field.
SHARED_STATE_META_FIELDS = (
    "name_key", "provinces", "victory_points", "vp_total",
    "state_category", "manpower",
)
BUNDLE_ONLY_STATE_META_FIELDS = frozenset({"impassable"})


# ---------------------------------------------------------------------------
# Hashing
# ---------------------------------------------------------------------------
def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(64 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def fnv1a64_bytes(data: bytes) -> str:
    """FNV-1a 64-bit, identical to the Rust map_viewer util (hex, 16 digits)."""
    h = 0xcbf29ce484222325
    for b in data:
        h ^= b
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return format(h, "016x")


def fnv1a64_file(path: str) -> str:
    with open(path, "rb") as fh:
        return fnv1a64_bytes(fh.read())


def build_manifest(assets_dir: Optional[str] = None) -> Dict[str, Dict[str, Any]]:
    """Hash every frozen table currently on disk (missing files are reported
    with ``{"missing": True}`` rather than raising)."""
    a = assets_dir or ASSETS_DIR
    out: Dict[str, Dict[str, Any]] = {}
    for name in FROZEN_TABLES:
        p = os.path.join(a, name)
        if not os.path.isfile(p):
            out[name] = {"missing": True}
            continue
        out[name] = {
            "bytes": os.path.getsize(p),
            "sha256": sha256_file(p),
            "fnv1a64": fnv1a64_file(p),
        }
    return out


def verify_frozen_tables(assets_dir: Optional[str] = None) -> List[str]:
    """Compare on-disk tables against FROZEN_TABLE_HASHES.

    Returns a list of human-readable mismatch strings (empty == all good).
    """
    problems: List[str] = []
    actual = build_manifest(assets_dir)
    for name, expected in FROZEN_TABLE_HASHES.items():
        got = actual.get(name)
        if got is None or got.get("missing"):
            problems.append(f"{name}: missing from assets dir")
            continue
        for field in ("bytes", "sha256", "fnv1a64"):
            if got[field] != expected[field]:
                problems.append(
                    f"{name}: {field} mismatch (frozen {expected[field]!r}, "
                    f"on disk {got[field]!r})")
    return problems


# ---------------------------------------------------------------------------
# Bundle parity (opt-in staleness gate vs the Rust bundle contract)
# ---------------------------------------------------------------------------
def _norm_vp(vp: Any) -> Dict[str, int]:
    """Normalize a victory_points map to {str_province: int} for comparison
    (Rust BTreeMap<u32,_> and the frozen table both serialize string keys,
    but be tolerant of int-keyed json loaded through other paths)."""
    if not isinstance(vp, dict):
        return {}
    return {str(k): int(v) for k, v in vp.items()}


def compare_province_to_state(frozen: Dict[str, Any],
                              bundle: Dict[str, Any]) -> List[str]:
    """Exact equality of the two string-keyed province->state maps."""
    problems: List[str] = []
    fk, bk = set(frozen), set(bundle)
    only_f = sorted(fk - bk, key=lambda s: int(s))
    only_b = sorted(bk - fk, key=lambda s: int(s))
    if only_f:
        problems.append(
            f"province_to_state: {len(only_f)} provinces only in frozen table "
            f"(first: {only_f[:5]})")
    if only_b:
        problems.append(
            f"province_to_state: {len(only_b)} provinces only in bundle "
            f"(first: {only_b[:5]})")
    diff = [k for k in (fk & bk) if int(frozen[k]) != int(bundle[k])]
    if diff:
        diff.sort(key=lambda s: int(s))
        problems.append(
            f"province_to_state: {len(diff)} provinces map to different "
            f"states (first: "
            f"{[(k, frozen[k], bundle[k]) for k in diff[:5]]})")
    return problems


def compare_state_meta(frozen: Dict[str, Any],
                       bundle: Dict[str, Any]) -> List[str]:
    """Field-aware comparison over the shared state-id key set.

    The bundle's StateMeta additionally carries ``impassable`` (serde default)
    and serializes ``state_category`` as an Option; comparison is therefore
    restricted to SHARED_STATE_META_FIELDS, and any OTHER bundle-side extra
    field is reported as schema drift.
    """
    problems: List[str] = []
    fk, bk = set(frozen), set(bundle)
    only_f = sorted(fk - bk, key=lambda s: int(s))
    only_b = sorted(bk - fk, key=lambda s: int(s))
    if only_f:
        problems.append(f"state_meta: {len(only_f)} states only in frozen "
                        f"table (first: {only_f[:5]})")
    if only_b:
        problems.append(f"state_meta: {len(only_b)} states only in bundle "
                        f"(first: {only_b[:5]})")
    for sid in sorted(fk & bk, key=lambda s: int(s)):
        f, b = frozen[sid], bundle[sid]
        extra = set(b) - set(SHARED_STATE_META_FIELDS) - BUNDLE_ONLY_STATE_META_FIELDS
        if extra:
            problems.append(
                f"state_meta[{sid}]: unexpected bundle-side fields {sorted(extra)} "
                f"(only 'impassable' is a known bundle addition)")
        for field in SHARED_STATE_META_FIELDS:
            fv, bv = f.get(field), b.get(field)
            if field == "victory_points":
                fv, bv = _norm_vp(fv), _norm_vp(bv)
            if fv != bv:
                problems.append(
                    f"state_meta[{sid}].{field}: frozen {fv!r} != bundle {bv!r}")
    return problems


def check_bundle_parity(bundle_dir: str,
                        assets_dir: Optional[str] = None) -> Dict[str, Any]:
    """Compare the frozen tables against a generated Rust map bundle.

    Returns ``{"status": "ok"|"mismatch", "differences": [...]}``. On
    mismatch, the differences mean the frozen assets -- a vanilla snapshot of
    an UNRECORDED game version -- have drifted from the current game/mod data
    (e.g. after a the game patch): that indicates data-version drift requiring a
    deliberate re-freeze from the bundle, not necessarily a code bug.
    """
    a = assets_dir or ASSETS_DIR
    problems: List[str] = []
    with open(os.path.join(a, "province_to_state.json"), encoding="utf-8") as fh:
        frozen_pts = json.load(fh)
    with open(os.path.join(a, "state_meta.json"), encoding="utf-8") as fh:
        frozen_sm = json.load(fh)
    with open(os.path.join(bundle_dir, "province_to_state.json"),
              encoding="utf-8") as fh:
        bundle_pts = json.load(fh)
    with open(os.path.join(bundle_dir, "state_meta.json"),
              encoding="utf-8") as fh:
        bundle_sm = json.load(fh)
    problems += compare_province_to_state(frozen_pts, bundle_pts)
    problems += compare_state_meta(frozen_sm, bundle_sm)
    return {"status": "mismatch" if problems else "ok",
            "differences": problems}


def check_raster_parity(bundle_dir: str,
                        assets_dir: Optional[str] = None) -> Dict[str, Any]:
    """Pixel-identity check of the bundle ``province_ids.r32`` (raw LE u32,
    dimensions from the bundle ``manifest.json``) against the frozen full-res
    ``province_ids.png`` (id = R + G*256 + B*65536). Both are 5632x2048; NO
    scaling is applied (the x2 factor applies only to province_ids_half.png).

    Returns ``{"status": "ok"|"mismatch"|"skipped", ...}``; "skipped" (with a
    reason) when the gitignored PNG or Pillow is unavailable -- visible, never
    a silent pass.
    """
    a = assets_dir or ASSETS_DIR
    png_path = os.path.join(a, "province_ids.png")
    if not os.path.isfile(png_path):
        return {"status": "skipped",
                "reason": "province_ids.png absent (gitignored machine-local raster)"}
    try:
        from PIL import Image
    except ImportError:
        return {"status": "skipped", "reason": "Pillow not installed"}
    with open(os.path.join(bundle_dir, "manifest.json"), encoding="utf-8") as fh:
        manifest = json.load(fh)
    dims = manifest["dimensions"]
    width, height = int(dims["width"]), int(dims["height"])
    r32_name = manifest.get("files", {}).get("province_ids", "province_ids.r32")
    with open(os.path.join(bundle_dir, r32_name), "rb") as fh:
        raw = fh.read()
    if len(raw) != width * height * 4:
        return {"status": "mismatch",
                "differences": [
                    f"{r32_name}: {len(raw)} bytes, expected "
                    f"{width * height * 4} for {width}x{height} u32"]}
    img = Image.open(png_path).convert("RGB")
    if img.size != (width, height):
        return {"status": "mismatch",
                "differences": [
                    f"province_ids.png is {img.size[0]}x{img.size[1]}, bundle "
                    f"plane is {width}x{height} (no scaling is applied)"]}
    import struct
    bundle_ids = struct.unpack(f"<{width * height}I", raw)
    png = img.tobytes()  # RGBRGB...
    problems: List[str] = []
    n_diff = 0
    first: List[Any] = []
    for i, bid in enumerate(bundle_ids):
        j = i * 3
        pid = png[j] + png[j + 1] * 256 + png[j + 2] * 65536
        if pid != bid:
            n_diff += 1
            if len(first) < 5:
                x, y = i % width, i // width
                first.append((x, y, pid, bid))
    if n_diff:
        problems.append(
            f"province_ids: {n_diff} of {width * height} pixels differ "
            f"(first (x, y, png_id, bundle_id): {first})")
    return {"status": "mismatch" if problems else "ok",
            "differences": problems}


def bundle_dir_from_env() -> Optional[str]:
    """The opt-in bundle directory (BATTLE_PLAN_BUNDLE_DIR), or None when unset."""
    d = os.environ.get("BATTLE_PLAN_BUNDLE_DIR", os.environ.get("HOI4_BUNDLE_DIR", "")).strip()
    return d or None
