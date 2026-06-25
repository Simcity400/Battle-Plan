#!/usr/bin/env python3
"""game-asset extraction pipeline for the MP-Stats replay UI.

Reads a (read-only) vanilla the game install plus an optional mod overlay and emits:
  * sprites.json        -- sprite-name -> texture/frame metadata manifest
  * gui/<view>.json     -- parsed window layouts (geometry/sprite/text tree)
  * assets/...          -- converted PNGs (GITIGNORED; never committed)

The engine's mod-overrides-vanilla-by-relative-path rule is replicated at the
FILE level: a mod interface/foo.gfx replaces vanilla interface/foo.gfx whole; a
mod gfx/.../bar.dds replaces the vanilla texture at the same relative path.

Run `python extract.py --help`. Install paths default to the documented Steam
location and `../../test`; override with --vanilla / --mod so no personal path
is baked into committed files.

Copyright: these are the user's own Paradox installed content, extracted for a private
offline tool. Converted art lives under assets/ and is gitignored -- only this
script, the manifest, and the parsed layouts are committed.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import struct
import sys
from pathlib import Path

from paradox_parser import Node, parse, parse_xy, coerce_scalar

# --------------------------------------------------------------------------
# Defaults (documented; no personal home path -- mod default is repo-relative)
# --------------------------------------------------------------------------
DEFAULT_VANILLA = r"X:/SteamLibrary/steamapps/common/Hearts of Iron IV"
# Mod overlay default: sibling "test" mod next to this repo (../../test).
HERE = Path(__file__).resolve().parent          # .../battle-plan/game_assets
REPO = HERE.parent                              # .../battle-plan
DEFAULT_MOD = str((REPO.parent / "test").resolve())  # .../mod/test

# Canonical ideology suffixes. Anything else after the last underscore is part
# of a cosmetic-tag flag name, NOT an ideology (e.g. SPR_republic, ISR_zion).
IDEOLOGIES = {"fascism", "communism", "democratic", "neutrality"}

# Priority .gui views to parse into layout JSON (filename stem -> output name).
PRIORITY_GUI = {
    "topbar": "topbar",
    "countrytechnologyview": "research",
    "countryproductionlineview": "production",
    "nationalfocusview": "national_focus",
    "countrydiplomacyview": "diplomacy",
    "countrydecisionview": "decisions",
    "countrystateview": "province_state",
    "countryconstructionsview": "construction",
    "landcombat": "land_combat",
    "navalcombat": "naval_combat",
    "combatlogview": "combat_log",
    "menubar": "menubar",
}

# Sprite-name prefixes that constitute the CORE UI chrome priority set (cat a).
CORE_CHROME_HINTS = (
    "GFX_topbar", "GFX_fuel", "GFX_supply", "GFX_pp_", "GFX_power",
    "GFX_stability", "GFX_war_support", "GFX_manpower", "GFX_factory",
    "GFX_resource", "GFX_generic_box", "GFX_tiled_window", "GFX_shield",
    "GFX_ideology", "GFX_button", "GFX_combat", "GFX_armyoverview",
)

PNG_DIRS = ("flags", "icons", "frames", "window_chrome", "goals", "ideas", "misc")


# ==========================================================================
# File overlay map (mod replaces vanilla by relative path)
# ==========================================================================
def build_file_map(vanilla: Path, mod: Path, subdir: str, exts) -> dict:
    """Return {relative_posix_path: absolute Path} with mod overriding vanilla.

    `subdir` is e.g. "interface" or "gfx"; `exts` a tuple of lowercase suffixes.
    """
    eff: dict[str, Path] = {}
    for base in (vanilla, mod):
        root = base / subdir
        if not root.is_dir():
            continue
        for p in root.rglob("*"):
            if p.is_file() and p.suffix.lower() in exts:
                rel = p.relative_to(base).as_posix()
                eff[rel] = p  # mod parsed second -> overrides
    return eff


def _lc_lookup(rel: str, gfx_map: dict):
    """Case-insensitive lookup into gfx_map (Windows is case-insensitive)."""
    if rel in gfx_map:
        return gfx_map[rel]
    low = rel.lower()
    for k, v in gfx_map.items():
        if k.lower() == low:
            return v
    return None


def resolve_texture(texrel: str, gfx_map: dict):
    """Resolve a sprite's texturefile to an actual file, trying alternate image
    extensions (the engine resolves .png<->.dds<->.tga for the same stem)."""
    src = _lc_lookup(texrel, gfx_map)
    if src is not None:
        return src
    base, _, ext = texrel.rpartition(".")
    if base:
        for alt in (".dds", ".tga", ".png"):
            cand = _lc_lookup(base + alt, gfx_map)
            if cand is not None:
                return cand
    return None


def normalize_texpath(raw: str) -> str:
    """Collapse '//' -> '/', backslashes -> '/', strip to a RELATIVE path.

    Defensive: a drive-rooted/absolute path in a .gfx (e.g. an accidental
    drive-letter path like '<DRIVE>:/some/abs/foo.dds') is reduced to its
    relative gfx/ tail so a local absolute path can never reach the committed
    manifest. Such textures simply won't resolve and get logged.
    """
    if raw is None:
        return ""
    s = raw.replace("\\", "/").strip()
    # Reject absolute / drive-rooted paths (defense against personal-path leak).
    if s[1:3] == ":/" or s.startswith("/"):
        # keep only the relative tail under a known gfx/ root if present
        idx = s.lower().find("gfx/")
        s = s[idx:] if idx != -1 else ""
    while "//" in s:
        s = s.replace("//", "/")
    return s.strip().lstrip("/")


_SAFE_RE = re.compile(r"[^A-Za-z0-9._-]")


def safe_basename(name: str) -> str:
    """Make a sprite/flag name safe as a single filename component (no path
    traversal). Strips directory separators and '..'."""
    base = name.replace("\\", "/").split("/")[-1]
    base = base.replace("..", "_")
    base = _SAFE_RE.sub("_", base)
    return base or "_unnamed"


# ==========================================================================
# .gfx sprite manifest
# ==========================================================================
SPRITE_BLOCK_TYPES = {
    "spritetype", "frameanimatedspritetype", "corneredtilespritetype",
    "textspritetype",  # generic button/list frames (GFX_button_*, list_button, ...)
}
# Special texture-backed types the topbar/UI reference but that aren't plain
# single-texture strips: masked shields (player flags) and progress bars
# (fuel/stability/war-support/resource bars). They use textureFile1/2.
SPECIAL_BLOCK_TYPES = {"maskedshieldtype", "progressbartype"}


def parse_gfx_file(node: Node, source_rel: str, sprites: dict, dup_log: list):
    """Walk a parsed .gfx root, collecting sprite definitions into `sprites`.

    Sprites live inside `spriteTypes = { ... }`. We also scan the root directly
    for tolerance. Last definition of a name wins (effective-file order); dups
    are logged.
    """
    def colorlist(blk, key):
        c = blk.get(key)
        if isinstance(c, Node):
            return [coerce_scalar(v) for k, v in c.entries if k == ""]
        return None

    def handle_block(key: str, blk: Node):
        kl = key.lower()
        if kl in SPECIAL_BLOCK_TYPES:
            handle_special(key, kl, blk)
            return
        if kl not in SPRITE_BLOCK_TYPES:
            return
        name = blk.get("name")
        if not isinstance(name, str) or not name:
            return
        tex = blk.get("texturefile")  # case-insensitive in Node.get
        entry = {
            "name": name,
            "type": key,
            "texturefile": normalize_texpath(tex) if isinstance(tex, str) else None,
            "source_gfx": source_rel,
        }
        if kl == "corneredtilespritetype":
            # 9-slice metadata, NOT a frame strip.
            entry["nine_slice"] = True
            entry["size"] = parse_xy(blk.get("size")) if isinstance(blk.get("size"), Node) else None
            entry["border_size"] = parse_xy(blk.get("borderSize")) if isinstance(blk.get("borderSize"), Node) else None
            entry["no_of_frames"] = 1
        else:
            nof = blk.get("noOfFrames")
            try:
                entry["no_of_frames"] = int(nof) if nof is not None else 1
            except (TypeError, ValueError):
                entry["no_of_frames"] = 1
        if name in sprites:
            dup_log.append({
                "name": name,
                "kept": source_rel,
                "shadowed": sprites[name]["source_gfx"],
            })
        sprites[name] = entry

    def handle_special(key: str, kl: str, blk: Node):
        name = blk.get("name")
        if not isinstance(name, str) or not name:
            return
        t1 = blk.get("textureFile1")
        t2 = blk.get("textureFile2")
        entry = {
            "name": name, "type": key, "no_of_frames": 1,
            "source_gfx": source_rel,
            # primary texture for conversion = textureFile1 (shield/bar fill)
            "texturefile": normalize_texpath(t1) if isinstance(t1, str) else None,
            "texturefile2": normalize_texpath(t2) if isinstance(t2, str) else None,
        }
        if kl == "maskedshieldtype":
            entry["masked_shield"] = True  # t1=flag/shield, t2=mask
        elif kl == "progressbartype":
            entry["progress_bar"] = True
            entry["color"] = colorlist(blk, "color")
            entry["color_two"] = colorlist(blk, "colortwo")
            entry["size"] = parse_xy(blk.get("size")) if isinstance(blk.get("size"), Node) else None
        if name in sprites:
            dup_log.append({"name": name, "kept": source_rel,
                            "shadowed": sprites[name]["source_gfx"]})
        sprites[name] = entry

    # spriteTypes wrapper(s)
    for k, v in node.entries:
        if isinstance(v, Node) and k.lower() == "spritetypes":
            for kk, vv in v.entries:
                if isinstance(vv, Node):
                    handle_block(kk, vv)
        elif isinstance(v, Node):
            # tolerate sprites defined at root level
            handle_block(k, v)


def build_sprite_manifest(iface_map: dict) -> tuple[dict, list]:
    """Parse all effective interface/*.gfx files into a sprite manifest."""
    sprites: dict = {}
    dup_log: list = []
    # Deterministic order: sort by relative path so output is stable.
    for rel in sorted(iface_map):
        if not rel.lower().endswith(".gfx"):
            continue
        path = iface_map[rel]
        try:
            text = path.read_bytes().decode("latin-1")  # the game files are 1252/latin-1ish
        except Exception as e:
            dup_log.append({"name": "<read-error>", "kept": rel, "error": str(e)})
            continue
        root = parse(text)
        parse_gfx_file(root, rel, sprites, dup_log)
    return sprites, dup_log


# ==========================================================================
# .gui layout parsing
# ==========================================================================
# Known geometry / sprite / text fields we lift to first-class keys.
# NOTE: buttonText is a localization KEY (text), not a sprite reference.
_SPRITE_REF_KEYS = ("spriteType", "quadTextureSprite", "background")
_TEXT_KEYS = ("text", "buttonText", "font", "format", "maxWidth", "maxHeight")


def gui_node_to_json(key: str, blk: Node) -> dict:
    """Convert a guiType block (containerWindowType/iconType/buttonType/...) to a
    JSON-friendly dict. Unknown element types are kept as generic nodes so the
    UI agent still sees their geometry. Child order is preserved (draw order)."""
    out: dict = {"type": key}
    name = blk.get("name")
    if isinstance(name, str):
        out["name"] = name

    pos = blk.get("position")
    if isinstance(pos, Node):
        out["position"] = parse_xy(pos)
    size = blk.get("size")
    if isinstance(size, Node):
        out["size"] = parse_xy(size)
    orient = blk.get("Orientation") or blk.get("orientation")
    if isinstance(orient, str):
        out["orientation"] = orient

    # sprite reference (icon/button/background)
    for sk in _SPRITE_REF_KEYS:
        v = blk.get(sk)
        if isinstance(v, str):
            out["sprite"] = v
            break

    # text fields
    text_fields = {}
    for tk in _TEXT_KEYS:
        v = blk.get(tk)
        if v is not None and not isinstance(v, Node):
            text_fields[tk] = coerce_scalar(v) if isinstance(v, str) else v
    if text_fields:
        out["text"] = text_fields

    # tooltip
    tip = blk.get("pdx_tooltip")
    if isinstance(tip, str):
        out["tooltip"] = tip

    # nested children: any child Node becomes a child element. Preserve order.
    children = []
    for ck, cv in blk.entries:
        if isinstance(cv, Node):
            # skip the geometry sub-blocks we already consumed
            if ck.lower() in ("position", "size", "show_position"):
                continue
            children.append(gui_node_to_json(ck, cv))
    if children:
        out["children"] = children
    return out


def parse_gui_file(path: Path) -> tuple[list, dict]:
    """Parse one .gui file into a list of top-level window descriptors.

    Returns (windows, stats) where stats counts element types seen / skipped.
    """
    text = path.read_bytes().decode("latin-1")
    root = parse(text)
    windows = []
    type_counts: dict[str, int] = {}

    def count(n):
        type_counts[n.get("type", "?")] = type_counts.get(n.get("type", "?"), 0) + 1
        for c in n.get("children", []):
            count(c)

    # guiTypes = { containerWindowType = {...} ... }
    containers = []
    for k, v in root.entries:
        if isinstance(v, Node) and k.lower() == "guitypes":
            for kk, vv in v.entries:
                if isinstance(vv, Node):
                    containers.append((kk, vv))
        elif isinstance(v, Node) and k.lower().endswith("type"):
            containers.append((k, v))

    for k, v in containers:
        node = gui_node_to_json(k, v)
        windows.append(node)
        count(node)

    return windows, {"element_type_counts": type_counts}


# ==========================================================================
# Texture conversion (TGA / DDS uncompressed / DXT1-3-5 via Pillow / DX10 raw)
# ==========================================================================
def _decode_dx10_uncompressed(path: Path):
    """Decode an uncompressed DX10 DDS (DXGI 28/29 RGBA8, 87/91 BGRA8) that
    Pillow refuses. Returns a Pillow Image (RGBA) or raises."""
    from PIL import Image
    data = path.read_bytes()
    if data[:4] != b"DDS ":
        raise ValueError("not a DDS")
    height = struct.unpack_from("<I", data, 12)[0]
    width = struct.unpack_from("<I", data, 16)[0]
    fourcc = data[84:88]
    if fourcc != b"DX10":
        raise ValueError("not DX10")
    dxgi = struct.unpack_from("<I", data, 128)[0]
    res_dim = struct.unpack_from("<I", data, 132)[0]
    array_size = struct.unpack_from("<I", data, 140)[0]
    if res_dim != 3:  # D3D10_RESOURCE_DIMENSION_TEXTURE2D
        raise ValueError(f"unsupported resourceDimension {res_dim}")
    if array_size != 1:
        raise ValueError(f"unsupported arraySize {array_size}")
    pixel_off = 148
    need = pixel_off + width * height * 4
    if len(data) < need:
        raise ValueError(f"truncated DX10 ({len(data)} < {need})")
    raw = data[pixel_off:pixel_off + width * height * 4]
    if dxgi in (28, 29):       # R8G8B8A8_UNORM(_SRGB)
        im = Image.frombytes("RGBA", (width, height), raw)
    elif dxgi in (87, 91):     # B8G8R8A8_UNORM(_SRGB) -> swap to RGBA
        im = Image.frombytes("RGBA", (width, height), raw, "raw", "BGRA")
    else:
        raise ValueError(f"unsupported DXGI format {dxgi}")
    return im


def load_texture(path: Path):
    """Open a .tga/.dds as a Pillow RGBA Image, with a manual DX10 fallback."""
    from PIL import Image
    try:
        im = Image.open(path)
        im.load()
        return im.convert("RGBA")
    except NotImplementedError:
        # Pillow can't do DX10/RXGB -- try our uncompressed DX10 decoder.
        return _decode_dx10_uncompressed(path)


def slice_and_save(im, no_of_frames: int, out_base: Path, name: str):
    """Save `im` as PNG(s). If multi-frame, slice horizontal strip into N PNGs.

    Returns (list_of_relative_png_names, frame_anomaly_or_None).
    """
    out_base.parent.mkdir(parents=True, exist_ok=True)
    w, h = im.size
    if no_of_frames and no_of_frames > 1:
        if w % no_of_frames != 0:
            # Do NOT silently floor -- save whole strip and flag anomaly.
            whole = out_base.with_suffix(".png")
            im.save(whole)
            return [whole.name], {
                "sprite": name, "issue": "width_not_divisible_by_frames",
                "width": w, "no_of_frames": no_of_frames,
            }
        fw = w // no_of_frames
        names = []
        for fi in range(no_of_frames):
            frame = im.crop((fi * fw, 0, (fi + 1) * fw, h))
            fp = out_base.parent / f"{out_base.name}_f{fi}.png"
            frame.save(fp)
            names.append(fp.name)
        return names, None
    out = out_base.with_suffix(".png")
    im.save(out)
    return [out.name], None


# ==========================================================================
# Flag enumeration (no .gfx; loaded by filename convention)
# ==========================================================================
def enumerate_flags(vanilla: Path, mod: Path) -> dict:
    """Return {relative: abs Path} for flag .tga, mod overriding vanilla."""
    return build_file_map(vanilla, mod, "gfx/flags", (".tga", ".dds"))


def parse_flag_filename(stem: str):
    """(tag, ideology|None) from a flag filename stem. Ideology only if the last
    suffix is a canonical ideology; cosmetic-tag underscores are kept in tag."""
    if "_" in stem:
        head, _, tail = stem.rpartition("_")
        if tail in IDEOLOGIES:
            return head, tail
    return stem, None


# ==========================================================================
# Main build
# ==========================================================================
def categorize_sprite(name: str, texrel: str) -> str:
    t = (texrel or "").lower()
    if "/goals/" in t:
        return "goals"
    if "/ideas/" in t:
        return "ideas"
    if name.startswith(CORE_CHROME_HINTS):
        if "window" in name.lower() or "box" in name.lower() or "tiled" in name.lower() or "bg" in t:
            return "window_chrome"
        return "icons"
    if "tiles" in t or "window" in t or "background" in t or "_bg" in t:
        return "window_chrome"
    return "icons"


def run(args):
    from PIL import Image  # noqa: ensure Pillow present early
    vanilla = Path(args.vanilla)
    mod = Path(args.mod) if args.mod else None
    out_assets = HERE / "assets"
    out_gui = HERE / "gui"
    out_gui.mkdir(parents=True, exist_ok=True)

    if not vanilla.is_dir():
        print(f"ERROR: vanilla install not found: {vanilla}")
        return 2
    if mod and not mod.is_dir():
        print(f"WARNING: mod overlay not found, using vanilla only: {mod}")
        mod = None

    print(f"[*] vanilla = {vanilla}")
    print(f"[*] mod     = {mod}")

    # ---- 1. sprite manifest ------------------------------------------------
    iface_map = build_file_map(vanilla, mod or vanilla, "interface", (".gfx",))
    sprites, dup_log = build_sprite_manifest(iface_map)
    print(f"[*] parsed {len(iface_map)} effective .gfx files -> "
          f"{len(sprites)} sprites ({len(dup_log)} duplicate-name events)")

    # texture overlay map (gfx/**). Used to resolve sprite + flag paths.
    # Include .png: some .gfx reference a .png that ships as .dds (engine tries
    # alternate extensions), and some textures ship as real .png.
    gfx_map = build_file_map(vanilla, mod or vanilla, "gfx", (".dds", ".tga", ".png"))

    # ---- 2. gui layouts ----------------------------------------------------
    gui_iface = build_file_map(vanilla, mod or vanilla, "interface", (".gui",))
    gui_index = {}
    gui_parsed = 0
    gui_skipped = []
    for rel in sorted(gui_iface):
        stem = Path(rel).stem.lower()
        if stem not in PRIORITY_GUI:
            continue
        out_name = PRIORITY_GUI[stem]
        try:
            windows, stats = parse_gui_file(gui_iface[rel])
            payload = {
                "view": out_name,
                "source": rel,
                "windows": windows,
                "stats": stats,
            }
            (out_gui / f"{out_name}.json").write_text(
                json.dumps(payload, indent=1), encoding="utf-8")
            gui_index[out_name] = {"source": rel, "windows": len(windows),
                                   "element_type_counts": stats["element_type_counts"]}
            gui_parsed += 1
        except Exception as e:
            gui_skipped.append({"file": rel, "error": str(e)})
    print(f"[*] parsed {gui_parsed} priority .gui layouts "
          f"({len(gui_skipped)} failed)")

    # ---- 3. conversion -----------------------------------------------------
    converted = {c: 0 for c in PNG_DIRS}
    failures = []
    frame_anomalies = []
    manifest = {"sprites": {}, "flags": {}}

    do_sprites = "sprites" in args.only or "all" in args.only
    do_flags = "flags" in args.only or "all" in args.only

    # Merge mode: a partial pass (e.g. --only flags) keeps the other section's
    # metadata from a prior full run instead of wiping it.
    if args.merge and (HERE / "sprites.json").exists():
        try:
            prev = json.loads((HERE / "sprites.json").read_text(encoding="utf-8"))
            if not do_sprites and prev.get("sprites"):
                manifest["sprites"] = prev["sprites"]
            if not do_flags and prev.get("flags"):
                manifest["flags"] = prev["flags"]
        except Exception as e:
            print(f"[!] --merge: could not read prior manifest: {e}")

    # which sprite categories to convert
    cat_filter = None
    if args.categories:
        cat_filter = set(args.categories.split(","))

    # Always record sprite METADATA (so --manifest-only is useful); convert
    # bytes only when do_sprites and not --manifest-only.
    convert_bytes = do_sprites and not args.manifest_only
    if do_sprites or args.manifest_only:
        items = sorted(sprites.items())
        if args.limit and convert_bytes:
            items = items[: args.limit]
        for name, sp in items:
            texrel = sp.get("texturefile")
            cat = categorize_sprite(name, texrel)
            entry = {
                "type": sp["type"], "no_of_frames": sp.get("no_of_frames", 1),
                "source_gfx": sp["source_gfx"], "texturefile": texrel,
                "category": cat,
            }
            # carry forward special-type metadata
            for fld in ("nine_slice", "size", "border_size", "texturefile2",
                        "masked_shield", "progress_bar", "color", "color_two"):
                if fld in sp:
                    entry[fld] = sp[fld]
            if not convert_bytes:
                manifest["sprites"][name] = entry  # metadata only
                continue
            if cat_filter and cat not in cat_filter:
                manifest["sprites"][name] = entry
                continue
            if not texrel:
                entry["png"] = []
                manifest["sprites"][name] = entry
                continue
            src = resolve_texture(texrel, gfx_map)
            if src is None or not src.exists():
                failures.append({"sprite": name, "texture": texrel,
                                 "reason": "texture_not_found"})
                entry["png"] = []
                manifest["sprites"][name] = entry
                continue
            safe_name = safe_basename(name)
            out_base = out_assets / cat / safe_name
            try:
                im = load_texture(src)
                pngs, anomaly = slice_and_save(im, sp.get("no_of_frames", 1),
                                               out_base, safe_name)
                if anomaly:
                    frame_anomalies.append(anomaly)
                entry["png"] = [f"assets/{cat}/{p}" for p in pngs]
                entry["width"], entry["height"] = im.size
                converted[cat] += 1
            except Exception as e:
                failures.append({"sprite": name, "texture": texrel,
                                 "reason": f"{type(e).__name__}: {e}"})
                entry["png"] = []
            # Secondary texture (shield mask / progress-bar background): convert
            # so the UI renderer has both compositing layers as PNGs.
            tex2 = sp.get("texturefile2")
            if tex2:
                src2 = resolve_texture(tex2, gfx_map)
                if src2 is not None and src2.exists():
                    try:
                        im2 = load_texture(src2)
                        pngs2, _ = slice_and_save(im2, 1, out_assets / cat /
                                                  (safe_name + "__2"),
                                                  safe_name + "__2")
                        entry["png2"] = f"assets/{cat}/{pngs2[0]}"
                        converted[cat] += 1
                    except Exception as e:
                        failures.append({"sprite": name + "(texture2)",
                                         "texture": tex2,
                                         "reason": f"{type(e).__name__}: {e}"})
                else:
                    failures.append({"sprite": name + "(texture2)",
                                     "texture": tex2,
                                     "reason": "texture_not_found"})
            manifest["sprites"][name] = entry

    if do_flags:
        flag_map = enumerate_flags(vanilla, mod or vanilla)
        items = sorted(flag_map.items())
        if args.limit:
            items = items[: args.limit]
        for rel, src in items:
            stem = Path(rel).stem
            tag, ideology = parse_flag_filename(stem)
            # Flags ship at three resolutions: gfx/flags/<stem> (large, ~82x52),
            # gfx/flags/medium/<stem> (~41x26), gfx/flags/small/<stem> (~10x7).
            # Key them distinctly so the large flag is never clobbered by a
            # smaller one. The manifest entry per (tag,ideology) lists all res.
            parts = rel.split("/")
            res = parts[2] if len(parts) > 3 and parts[2] in ("medium", "small") else "large"
            key = f"{tag}_{ideology}" if ideology else tag
            out_name = safe_basename(f"{stem}__{res}" if res != "large" else stem)
            out_base = out_assets / "flags" / out_name
            try:
                im = load_texture(src)
                pngs, _ = slice_and_save(im, 1, out_base, out_name)
                entry = manifest["flags"].setdefault(
                    key, {"tag": tag, "ideology": ideology, "res": {}})
                entry["res"][res] = {
                    "png": f"assets/flags/{pngs[0]}",
                    "width": im.size[0], "height": im.size[1],
                    "source": rel,
                }
                # convenience: top-level png points at the largest available
                if res == "large" or "png" not in entry:
                    entry["png"] = f"assets/flags/{pngs[0]}"
                    entry["width"], entry["height"] = im.size
                converted["flags"] += 1
            except Exception as e:
                failures.append({"flag": stem, "res": res,
                                 "reason": f"{type(e).__name__}: {e}"})

    # ---- 4. write manifest + report ---------------------------------------
    manifest["_meta"] = {
        "sprite_count": len(manifest["sprites"]),
        "flag_count": len(manifest["flags"]),
        "duplicate_name_events": len(dup_log),
        "converted_by_category": converted,
        "failures": len(failures),
        "frame_anomalies": len(frame_anomalies),
        "gui_views": list(gui_index.keys()),
        "note": "PNG paths are relative to game_assets/; assets/ is gitignored.",
    }
    (HERE / "sprites.json").write_text(json.dumps(manifest, indent=1),
                                       encoding="utf-8")
    (out_gui / "_index.json").write_text(json.dumps(gui_index, indent=1),
                                         encoding="utf-8")

    # diagnostics (gitignored alongside assets) -- keep failures visible
    diag = {
        "duplicate_name_events": dup_log[:500],
        "conversion_failures": failures,
        "frame_anomalies": frame_anomalies,
        "gui_skipped": gui_skipped,
    }
    (HERE / "extract_report.json").write_text(json.dumps(diag, indent=1),
                                              encoding="utf-8")

    # ---- console summary (ASCII only) -------------------------------------
    print("\n==== EXTRACTION SUMMARY ====")
    print(f"sprites in manifest : {len(manifest['sprites'])}")
    print(f"flags  in manifest  : {len(manifest['flags'])}")
    print("converted PNGs by category:")
    for c in PNG_DIRS:
        print(f"   {c:14s}: {converted[c]}")
    print(f"gui layouts parsed  : {gui_parsed} -> {list(gui_index.keys())}")
    print(f"conversion failures : {len(failures)}  (see extract_report.json)")
    print(f"frame anomalies     : {len(frame_anomalies)}")
    if failures[:8]:
        print("  sample failures:")
        for f in failures[:8]:
            print("   ", f)
    return 0


def build_argparser():
    ap = argparse.ArgumentParser(description="Extract the game UI assets for the replay tool.")
    ap.add_argument("--vanilla", default=DEFAULT_VANILLA,
                    help="Path to the vanilla the game install (default: documented Steam path)")
    ap.add_argument("--mod", default=DEFAULT_MOD,
                    help="Path to the mod overlay (default: ../../test). Use '' for vanilla-only.")
    ap.add_argument("--only", nargs="+", default=["all"],
                    choices=["all", "sprites", "flags"],
                    help="Limit conversion to subsets.")
    ap.add_argument("--categories", default=None,
                    help="Comma list of sprite categories to convert bytes for "
                         "(flags,icons,frames,window_chrome,goals,ideas,misc). "
                         "Others get metadata only. Default: all.")
    ap.add_argument("--limit", type=int, default=0,
                    help="Convert at most N items per pass (smoke test).")
    ap.add_argument("--manifest-only", action="store_true",
                    help="Parse .gfx/.gui only; skip texture conversion.")
    ap.add_argument("--merge", action="store_true",
                    help="Keep the untouched section (sprites/flags) of an "
                         "existing sprites.json when doing a partial --only pass.")
    return ap


def main(argv=None):
    args = build_argparser().parse_args(argv)
    if args.mod == "":
        args.mod = None
    if args.manifest_only:
        args.only = []
    return run(args)


if __name__ == "__main__":
    sys.exit(main())
