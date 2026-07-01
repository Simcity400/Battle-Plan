#!/usr/bin/env python3
"""game-asset extraction pipeline for the MP-Stats replay UI.

Reads a (read-only) vanilla the game install plus an optional mod overlay and emits:
  * sprites.json        -- sprite-name -> texture/frame metadata manifest
  * gui/<view>.json     -- parsed window layouts (geometry/sprite/text tree)
  * assets/...          -- converted PNGs (GITIGNORED; never committed)

The engine's mod-overrides-vanilla-by-relative-path rule is replicated at the
FILE level: a mod interface/foo.gfx replaces vanilla interface/foo.gfx whole; a
mod gfx/.../bar.dds replaces the vanilla texture at the same relative path.

Run `python extract.py --help`. The vanilla game path comes from the BATTLE_PLAN_VANILLA
environment variable (or --vanilla), falling back to a generic documented Steam
location; the mod overlay defaults to the sibling `test` mod. No personal/machine-specific
path is baked into committed files -- set BATTLE_PLAN_VANILLA or pass --vanilla / --mod.

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
# Defaults (documented; no personal home path -- mod/game paths via CLI/config)
# --------------------------------------------------------------------------
# Vanilla game path is ENV/CLI-driven so NO machine-specific drive path is baked
# into the committed script. Resolution order:
#   1. --vanilla CLI flag (highest priority; see build_argparser)
#   2. BATTLE_PLAN_VANILLA environment variable
#   3. the generic, documented default Steam install location below
# A Steam library on another drive is selected by setting BATTLE_PLAN_VANILLA or
# passing --vanilla, never by editing this file.
_DEFAULT_VANILLA_FALLBACK = (
    r"C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV"
)
DEFAULT_VANILLA = os.environ.get("BATTLE_PLAN_VANILLA", os.environ.get("HOI4_VANILLA")) or _DEFAULT_VANILLA_FALLBACK
HERE = Path(__file__).resolve().parent          # .../battle-plan/replay_interface/game_assets
REPLAY_INTERFACE = HERE.parent                  # .../battle-plan/replay_interface
REPO = REPLAY_INTERFACE.parent                  # .../battle-plan
# The the game user-data dir holds dlc_load.json + the .mod descriptors. It is the
# grandparent of the mod/ dir this repo lives in: .../the game/.
DEFAULT_USERDATA = str(REPO.parent.parent.resolve())  # .../the game
# Single-mod default overlay: sibling "test" mod next to this repo.
DEFAULT_MOD = str((REPO.parent / "test").resolve())  # .../mod/test

# Canonical ideology suffixes. Anything else after the last underscore is part
# of a cosmetic-tag flag name, NOT an ideology (e.g. SPR_republic, ISR_zion).
IDEOLOGIES = {"fascism", "communism", "democratic", "neutrality"}

# Priority .gui views to parse into layout JSON (filename stem -> output name).
# Scope = the §C.4 core in-game replay-target views (group 2). EXCLUDES the ~28
# frontend/lobby files (group 0) and is independent of the ~19 nation-specific
# *_scripted_gui overlays (group 1, degrade/selective). The parser extends
# mechanically; adding a stem here parses it on the next run.
PRIORITY_GUI = {
    # --- Topbar / clock / minimap / alerts / menu / outliner ---
    "topbar": "topbar",
    "menubar": "menubar",
    "ingameclock": "ingame_clock",
    "ingamemenu": "ingame_menu",
    "minimap": "minimap",
    "alerts": "alerts",
    "findview": "find_view",
    "rightclickmenu": "rightclick_menu",
    # --- Map-mode chrome ---
    "mapmodes_interface": "mapmodes",
    "mapicons": "mapicons",
    # --- State / province / occupation ---
    "countrystateview": "province_state",
    "countryoccupationview": "occupation",
    # --- Politics / government ---
    "countrypoliticsview": "politics",
    "powerbalanceview": "power_balance",
    "command_character_view": "command_character",
    "advisorroleselectionview": "advisor_role_selection",
    # --- Focus tree ---
    "nationalfocusview": "national_focus",
    # --- Research / doctrine (side panel + the two TREE views) ---
    "countrytechnologyview": "research",
    "countrytechtreeview": "tech_tree",
    "countrydoctrinetreeview": "doctrine_tree",
    # --- Production / construction / deployment / equipment overview ---
    "countryproductionlineview": "production",
    "countryconstructionsview": "construction",
    "countrydeploymentview": "deployment",
    "equipmentoverview": "equipment_overview",
    # --- Designers ---
    "tank_designer_view": "tank_designer",
    "plane_designer_view": "plane_designer",
    "divisiondesignerview": "division_designer",
    "equipmentdesignerview": "equipment_designer",
    "equipmentupgradedesignerwindow": "equipment_upgrade_designer",
    # --- Decisions / operations ---
    "countrydecisionview": "decisions",
    "operationoverview": "operation_overview",
    # --- Diplomacy / trade / peace / tension ---
    "countrydiplomacyview": "diplomacy",
    "countrytradeview": "trade",
    "peaceconferencewindow": "peace_conference",
    "worldtensionwindowpopup": "world_tension_popup",
    # --- Intel / operatives / MIO ---
    "countryintelligenceagencyview": "intel_agency",
    "countryintelledger": "intel_ledger",
    "operative": "operative",
    "operativeleader": "operative_leader",
    # --- Officer corps / high command / leaders / traits ---
    "countryofficercorpview": "officer_corps",
    "unitleaderwindow": "unit_leader",
    "navyleaderview": "navy_leader",
    "leadergroups": "leader_groups",
    "acesview": "aces",
    "armytraittreewindow": "army_trait_tree",
    "navytraittreewindow": "navy_trait_tree",
    "traittreewindow_common": "trait_tree_common",
    # --- Army / navy / air mgmt + plans ---
    "countryarmyview": "army",
    "naviesview": "navies",
    "airgroupview": "air_group",
    "airselectionview": "air_selection",
    "airwingdetails": "air_wing_details",
    "airwingreorganization": "air_wing_reorg",
    "carrierairwingcompositionwindow": "carrier_air_wing",
    "strategicairview": "strategic_air",
    "battleplantools": "battle_plan_tools",
    "abilitylist": "ability_list",
    "theatreselector": "theatre_selector",
    "divisions_summary": "divisions_summary",
    "railway_gun": "railway_gun",
    # --- Logistics / supply ---
    "countrylogisticsview": "logistics",
    "garrisonlogview": "garrison_log",
    # --- Combat ---
    "landcombat": "land_combat",
    "navalcombat": "naval_combat",
    "combatlogview": "combat_log",
    "navalcombatresultswindow": "naval_combat_results",
    "destroyedshipswindow": "destroyed_ships",
    "naval_losses": "naval_losses",
    "naval_strategic_region_view": "naval_strategic_region",
    # --- Unit / division detail ---
    "unitview": "unit_view",
    # --- Events / popups / notifications ---
    "eventwindow": "event_window",
    "popupwindow": "popup_window",
    "detailedwindows": "detailed_windows",
    "core": "core",
    # --- War / faction / insignia ---
    "waroverview": "war_overview",
    "insigniaselection": "insignia_selection",
    "insrevolutionwindow": "ins_revolution",
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
# Active mod-set resolution (Clausewitz mod-over-vanilla, ordered playset)
# ==========================================================================
# A Layer is one source in the load order: vanilla (base) or a mod. `root` is the
# absolute dir to enumerate; `replace_paths` is the mod's list of replace_path
# directories (each REPLACES one exact directory's lower-layer contents).
class Layer:
    __slots__ = ("label", "root", "replace_paths", "meta")

    def __init__(self, label, root: Path, replace_paths=None, meta=None):
        self.label = label
        self.root = root
        self.replace_paths = list(replace_paths or [])
        self.meta = meta or {}


def _norm_reldir(d: str) -> str:
    """Normalize a replace_path directory: backslashes->/, strip, collapse '//',
    drop leading/trailing slashes. Returns a clean relative posix dir (no '.')."""
    s = (d or "").replace("\\", "/").strip().strip('"')
    while "//" in s:
        s = s.replace("//", "/")
    s = s.strip("/")
    parts = [p for p in s.split("/") if p not in ("", ".")]
    return "/".join(parts)


def parse_mod_descriptor(mod_file: Path) -> dict:
    """Parse a `.mod` descriptor (Clausewitz key=value) into a dict.

    Returns: {name, path, archive, version, supported_version, replace_path:[...],
              dependencies:[...], tags:[...], remote_file_id, descriptor: <abs>}.
    `path` may be absolute or relative; `archive` (zip) mods are flagged, not
    unzipped. Repeated replace_path lines are all collected.
    """
    text = mod_file.read_bytes().decode("latin-1")
    root = parse(text)
    out: dict = {
        "descriptor": str(mod_file),
        "name": None, "path": None, "archive": None,
        "version": None, "supported_version": None, "remote_file_id": None,
        "replace_path": [], "dependencies": [], "tags": [],
    }
    for k, v in root.entries:
        kl = k.lower()
        if kl == "replace_path" and isinstance(v, str):
            nd = _norm_reldir(v)
            if nd:
                out["replace_path"].append(nd)
        elif kl == "name" and isinstance(v, str):
            out["name"] = v
        elif kl == "path" and isinstance(v, str):
            out["path"] = v
        elif kl == "archive" and isinstance(v, str):
            out["archive"] = v
        elif kl == "version" and isinstance(v, str):
            out["version"] = v
        elif kl == "supported_version" and isinstance(v, str):
            out["supported_version"] = v
        elif kl == "remote_file_id" and isinstance(v, str):
            out["remote_file_id"] = v
        elif kl == "dependencies" and isinstance(v, Node):
            out["dependencies"] = [s for kk, s in v.entries
                                   if kk == "" and isinstance(s, str)]
        elif kl == "tags" and isinstance(v, Node):
            out["tags"] = [s for kk, s in v.entries
                           if kk == "" and isinstance(s, str)]
    return out


def resolve_mod_root(descr: dict, userdata: Path) -> Path | None:
    """Resolve a descriptor's mod root directory. `path` may be absolute or
    relative to the game user-data dir. Returns None for zip-only / missing."""
    p = descr.get("path")
    if p:
        pp = Path(p.replace("\\", "/"))
        if not pp.is_absolute():
            pp = (userdata / pp)
        if pp.is_dir():
            return pp.resolve()
    return None


def build_modset_layers(vanilla: Path, mod_files: list[Path],
                        userdata: Path) -> tuple[list, list]:
    """Build the ordered [vanilla, mod1, mod2, ...] Layer list from descriptors.

    Returns (layers, warnings). Each mod in `mod_files` is in load order (lowest
    priority first). Skips zip-only / unresolvable mods with a warning so the
    extraction is still valid (vanilla + the resolvable mods)."""
    layers = [Layer("vanilla", vanilla)]
    warnings: list = []
    for mf in mod_files:
        if not mf.is_file():
            warnings.append(f"mod descriptor not found: {mf}")
            continue
        descr = parse_mod_descriptor(mf)
        root = resolve_mod_root(descr, userdata)
        if root is None:
            if descr.get("archive"):
                warnings.append(
                    f"archive(zip)-only mod skipped (not unzipped): "
                    f"{descr.get('name') or mf.name}")
            else:
                warnings.append(
                    f"mod path unresolvable, skipped: "
                    f"{descr.get('name') or mf.name} (path={descr.get('path')})")
            continue
        layers.append(Layer(descr.get("name") or mf.stem, root,
                            descr.get("replace_path"), descr))
    return layers, warnings


def _immediate_parent(rel: str) -> str:
    """Immediate parent directory of a relative posix file path ('' for top)."""
    i = rel.rfind("/")
    return rel[:i] if i != -1 else ""


def build_file_map_layers(layers: list, subdir: str, exts) -> dict:
    """Resolve {relative_posix_path: abs Path} across an ordered Layer list,
    honoring Clausewitz mod-over-vanilla rules:

    * file-level override by exact (case-folded) relative path -- later layer wins
    * replace_path REPLACES one EXACT directory: before adding a mod's files,
      every accumulated lower-layer file whose IMMEDIATE PARENT dir == a
      replace_path dir is removed (NON-recursive: `common/units` does NOT touch
      `common/units/names_divisions`; that needs its own replace_path).

    Keys keep the on-disk RELATIVE path (real casing, so flag tags / source_gfx
    stay correct); override + replace_path matching are case-INSENSITIVE via a
    lowercased shadow index (Windows paths are case-insensitive; the game relpaths are
    ASCII so str.lower() suffices).

    `subdir` restricts enumeration (e.g. "interface", "gfx"); `exts` filters by
    lowercase suffix. replace_path dirs are matched against the full relpath, so
    a mod's replace_path="interface" correctly nukes lower interface/*.gfx here.
    """
    eff: dict[str, Path] = {}         # real-case relpath -> abs Path
    fold: dict[str, str] = {}         # case-folded relpath -> real-case relpath
    for layer in layers:
        # 1) apply this layer's replace_path: drop lower-layer files whose
        #    immediate parent dir is exactly one of the replace_path dirs.
        rps = {rp.lower() for rp in layer.replace_paths}
        if rps:
            for fkey in list(fold.keys()):
                if _immediate_parent(fkey) in rps:
                    real = fold.pop(fkey)
                    eff.pop(real, None)
        # 2) add this layer's own files under `subdir`
        root = layer.root / subdir
        if not root.is_dir():
            continue
        for p in root.rglob("*"):
            if p.is_file() and p.suffix.lower() in exts:
                rel = p.relative_to(layer.root).as_posix()
                fkey = rel.lower()
                # a later layer overriding a differently-cased lower-layer file
                # must displace the old real-case key, not leave a stale dup.
                prev_real = fold.get(fkey)
                if prev_real is not None and prev_real != rel:
                    eff.pop(prev_real, None)
                eff[rel] = p
                fold[fkey] = rel
    return eff


def build_file_map(vanilla: Path, mod: Path, subdir: str, exts) -> dict:
    """Back-compat single-mod overlay (vanilla + one mod, no replace_path).

    Retained for callers/tests that pass two roots directly. The mod-set path
    (build_file_map_layers) is the general resolver used by run().
    """
    layers = [Layer("vanilla", vanilla)]
    if mod is not None and Path(mod) != Path(vanilla):
        layers.append(Layer("mod", Path(mod)))
    return build_file_map_layers(layers, subdir, exts)


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


def gui_define_table(root: Node) -> dict:
    """Extract numeric Clausewitz GUI `@name = number` layout constants.

    The runtime resolves position anchors against the JSON `defines` table.
    Clausewitz files may place these constants at file root, inside `guiTypes`,
    or inside a root/template block. Emit a file-wide define only when the name
    has a single numeric value in this file; conflicting scoped values stay
    unresolved rather than being flattened to the wrong coordinate.
    """
    seen: dict[str, set] = {}

    def walk(node: Node) -> None:
        for key, value in node.entries:
            if isinstance(key, str) and key.startswith("@") and not isinstance(value, Node):
                parsed = coerce_scalar(value) if isinstance(value, str) else value
                if not isinstance(parsed, bool) and isinstance(parsed, (int, float)):
                    name = key[1:].strip()
                    if name:
                        seen.setdefault(name, set()).add(parsed)
            if isinstance(value, Node):
                walk(value)

    walk(root)
    return {name: next(iter(values)) for name, values in seen.items() if len(values) == 1}


def parse_gui_file(path: Path) -> tuple[list, dict, dict]:
    """Parse one .gui file into a list of top-level window descriptors.

    Returns (windows, defines, stats) where defines holds top-level `@name`
    layout constants and stats counts element types seen / skipped.
    """
    text = path.read_bytes().decode("latin-1")
    root = parse(text)
    defines = gui_define_table(root)
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

    return windows, defines, {"element_type_counts": type_counts}


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
def enumerate_flags(layers: list) -> dict:
    """Return {relative: abs Path} for flag .tga/.dds, resolved across the
    ordered mod-set (mod over vanilla, replace_path honored)."""
    return build_file_map_layers(layers, "gfx/flags", (".tga", ".dds"))


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


# ==========================================================================
# Font map (core.gfx bitmapfont name -> glyph atlas path)
# ==========================================================================
def parse_font_map(iface_map: dict) -> dict:
    """Extract the bitmapfont name->path map from the effective core.gfx.

    `bitmapfonts = { bitmapfont = { name=.. path=.. fontfiles={..} } ... }`. We
    record name -> {path, fontfiles[], color, language_overrides[]}. Resolved
    mod-over-vanilla because `iface_map` is already the resolved interface set.
    """
    fonts: dict = {}
    overrides: list = []
    src = _lc_lookup("interface/core.gfx", iface_map)
    if src is None:
        return {"fonts": {}, "overrides": [], "source": None}
    try:
        root = parse(src.read_bytes().decode("latin-1"))
    except Exception:
        return {"fonts": {}, "overrides": [], "source": str(src)}

    def fontfiles(blk):
        ff = blk.get("fontfiles")
        if isinstance(ff, Node):
            return [s for k, s in ff.entries if k == "" and isinstance(s, str)]
        return []

    def langs(blk):
        lg = blk.get("languages")
        if isinstance(lg, Node):
            return [s for k, s in lg.entries if k == "" and isinstance(s, str)]
        return []

    for k, v in root.entries:
        if not (isinstance(v, Node) and k.lower() == "bitmapfonts"):
            continue
        for kk, vv in v.entries:
            if not isinstance(vv, Node):
                continue
            kl = kk.lower()
            name = vv.get("name")
            if not isinstance(name, str):
                continue
            path = vv.get("path") if isinstance(vv.get("path"), str) else None
            entry = {
                "name": name,
                "path": normalize_texpath(path) if path else None,
                "fontfiles": [normalize_texpath(f) for f in fontfiles(vv)],
            }
            col = vv.get("color")
            if isinstance(col, str):
                entry["color"] = col
            if kl == "bitmapfont":
                fonts[name] = entry
            elif kl == "bitmapfont_override":
                entry["languages"] = langs(vv)
                overrides.append(entry)
    return {"fonts": fonts, "overrides": overrides,
            "source": "interface/core.gfx"}


# ==========================================================================
# Localisation resolution (mod-over-vanilla; resolved file LIST only)
# ==========================================================================
def resolve_loc(layers: list, language: str = "english") -> dict:
    """Resolve the effective localisation/<language>/*.yml set (mod over vanilla).

    Returns {relfile: abs Path}. We record the resolved file set + counts; full
    $VAR$/markup resolution is the runtime's job (Phase 0 loc layer). Mods rarely
    replace_path localisation, so file-level override is the norm here too.
    """
    return build_file_map_layers(layers, f"localisation/{language}", (".yml",))


# ==========================================================================
# Active mod-set assembly (from CLI: dlc_load.json playset OR explicit mods)
# ==========================================================================
def build_active_modset(args, vanilla: Path):
    """Return (layers, modset_manifest, warnings).

    Resolution sources, in priority:
      1. --mods A.mod,B.mod : explicit ordered playset (paths or names under
         the user-data mod/ dir).
      2. --dlc-load <dlc_load.json> or --userdata <dir> : read enabled_mods in
         launcher load order (authoritative).
      3. --mod <dir> : legacy single-mod overlay.
      4. none : vanilla-only (valid).
    """
    userdata = Path(args.userdata) if getattr(args, "userdata", None) else Path(DEFAULT_USERDATA)
    warnings: list = []
    mod_files: list[Path] = []
    source = None

    def as_modfile(token: str) -> Path:
        t = token.strip().replace("\\", "/")
        p = Path(t)
        if not p.is_absolute():
            p = userdata / p
        # accept either a .mod descriptor or a "mod/foo.mod" relative token
        return p

    if getattr(args, "mods", None):
        source = "explicit --mods playset"
        for tok in args.mods.split(","):
            tok = tok.strip()
            if tok:
                mod_files.append(as_modfile(tok))
    elif getattr(args, "dlc_load", None) or getattr(args, "use_dlc_load", False):
        dlc_path = Path(args.dlc_load) if getattr(args, "dlc_load", None) else (userdata / "dlc_load.json")
        # do NOT bake the absolute dlc_load path into the committed manifest
        source = "launcher dlc_load.json"
        try:
            dl = json.loads(Path(dlc_path).read_text(encoding="utf-8"))
            for rel in dl.get("enabled_mods", []):
                mod_files.append(as_modfile(rel))
        except Exception as e:
            warnings.append(f"could not read dlc_load.json {_sanitize_path(dlc_path, userdata)}: {e}")
    elif args.mod:
        source = "legacy single --mod overlay"
        modp = Path(args.mod)
        # find the descriptor next to the mod dir (mod/<name>.mod) if present
        descr = modp.parent / f"{modp.name}.mod"
        if descr.is_file():
            mod_files.append(descr)
        else:
            # no descriptor: synthesize a path-only layer (no replace_path)
            layers = [Layer("vanilla", vanilla), Layer(modp.name, modp)]
            manifest = _modset_manifest(layers, source, warnings, userdata)
            return layers, manifest, warnings
    else:
        source = "vanilla-only (no mods)"

    layers, w2 = build_modset_layers(vanilla, mod_files, userdata)
    warnings.extend(w2)
    manifest = _modset_manifest(layers, source, warnings, userdata)
    return layers, manifest, warnings


def _sanitize_path(p, userdata: Path):
    """Return a path RELATIVE to the game user-data dir (e.g. 'mod/test.mod') so
    no absolute personal home path is baked into the committed manifest. Falls
    back to just the basename if the path is outside user-data."""
    if p is None:
        return None
    try:
        pp = Path(str(p).replace("\\", "/"))
        return pp.relative_to(userdata).as_posix()
    except (ValueError, TypeError):
        return Path(str(p).replace("\\", "/")).name


def _modset_manifest(layers: list, source: str, warnings: list,
                     userdata: Path) -> dict:
    """Serialize the resolved load order for the app (active_modset.json).

    Paths are stored RELATIVE to the user-data dir to keep personal home paths
    out of the committed file (privacy/copyright rule)."""
    order = []
    for i, ly in enumerate(layers):
        m = ly.meta or {}
        order.append({
            "load_index": i,
            "label": ly.label,
            "kind": "vanilla" if ly.label == "vanilla" else "mod",
            "name": m.get("name"),
            "workshop_id": m.get("remote_file_id"),
            "version": m.get("version"),
            "supported_version": m.get("supported_version"),
            "descriptor": _sanitize_path(m.get("descriptor"), userdata),
            "replace_path": ly.replace_paths,
            "dependencies": m.get("dependencies", []),
        })
    return {
        "schema": 1,
        "selection_source": source,
        "load_order": order,
        "mod_count": sum(1 for o in order if o["kind"] == "mod"),
        "warnings": warnings,
        "note": ("Ordered vanilla->modN per Clausewitz: file-level override by "
                 "relpath (later wins); replace_path replaces one EXACT dir's "
                 "lower-layer files (non-recursive). The app auto-selects the "
                 "matching set per replay and warns on mismatch (plan §K)."),
    }


def run(args):
    if not args.manifest_only:
        from PIL import Image  # noqa: ensure Pillow present early
    vanilla = Path(args.vanilla)
    out_assets = HERE / "assets"
    out_gui = HERE / "gui"
    out_gui.mkdir(parents=True, exist_ok=True)

    if not vanilla.is_dir():
        print(f"ERROR: vanilla install not found: {vanilla}")
        return 2

    # ---- 0. active mod-set (ordered playset over vanilla) ------------------
    layers, modset_manifest, modset_warnings = build_active_modset(args, vanilla)
    print(f"[*] vanilla = {vanilla}")
    print(f"[*] mod-set ({modset_manifest['selection_source']}):")
    for o in modset_manifest["load_order"]:
        if o["kind"] == "mod":
            print(f"      [{o['load_index']}] {o['name'] or o['label']}"
                  f"  (replace_path x{len(o['replace_path'])}"
                  f"{', ws='+o['workshop_id'] if o['workshop_id'] else ''})")
    for w in modset_warnings:
        print(f"    [!] {w}")
    (HERE / "active_modset.json").write_text(
        json.dumps(modset_manifest, indent=1), encoding="utf-8")

    # ---- 1. sprite manifest ------------------------------------------------
    iface_map = build_file_map_layers(layers, "interface", (".gfx",))
    sprites, dup_log = build_sprite_manifest(iface_map)
    print(f"[*] parsed {len(iface_map)} effective .gfx files -> "
          f"{len(sprites)} sprites ({len(dup_log)} duplicate-name events)")

    # texture overlay map (gfx/**). Used to resolve sprite + flag paths.
    # Include .png: some .gfx reference a .png that ships as .dds (engine tries
    # alternate extensions), and some textures ship as real .png.
    gfx_map = build_file_map_layers(layers, "gfx", (".dds", ".tga", ".png"))

    # ---- 1b. font map + loc set (mod-resolved) -----------------------------
    font_map = parse_font_map(iface_map)
    loc_map = resolve_loc(layers, getattr(args, "language", "english"))
    (HERE / "fonts.json").write_text(
        json.dumps({"_meta": {"font_count": len(font_map["fonts"]),
                              "override_count": len(font_map["overrides"]),
                              "source": font_map["source"]},
                    **font_map}, indent=1), encoding="utf-8")
    print(f"[*] font map: {len(font_map['fonts'])} bitmapfonts "
          f"({len(font_map['overrides'])} language overrides); "
          f"loc/{getattr(args,'language','english')}: {len(loc_map)} .yml")

    # ---- 2. gui layouts ----------------------------------------------------
    gui_iface = build_file_map_layers(layers, "interface", (".gui",))
    gui_index = {}
    gui_parsed = 0
    gui_skipped = []
    for rel in sorted(gui_iface):
        stem = Path(rel).stem.lower()
        if stem not in PRIORITY_GUI:
            continue
        out_name = PRIORITY_GUI[stem]
        try:
            windows, defines, stats = parse_gui_file(gui_iface[rel])
            payload = {
                "view": out_name,
                "source": rel,
                "defines": defines,
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
    do_flags = ("flags" in args.only or "all" in args.only) and not args.manifest_only

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
        flag_map = enumerate_flags(layers)
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
        "font_count": len(font_map["fonts"]),
        "loc_yml_count": len(loc_map),
        "modset": {
            "selection_source": modset_manifest["selection_source"],
            "mods": [o["name"] or o["label"] for o in modset_manifest["load_order"]
                     if o["kind"] == "mod"],
        },
        "note": "PNG paths are relative to game_assets/; assets/ is gitignored. "
                "Asset set is resolved against the active mod-set "
                "(see active_modset.json).",
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
                    help="Path to the vanilla the game install. Default comes from the "
                         "BATTLE_PLAN_VANILLA env var, else the documented generic Steam path. "
                         "Set BATTLE_PLAN_VANILLA or pass this flag for a non-default drive.")
    # ---- active mod-set selection (Clausewitz playset over vanilla) --------
    ap.add_argument("--mod", default=DEFAULT_MOD,
                    help="Legacy single-mod overlay dir (default: sibling test mod). "
                         "Use '' for vanilla-only. Superseded by --mods / --use-dlc-load.")
    ap.add_argument("--mods", default=None,
                    help="Explicit ORDERED playset: comma list of .mod descriptors "
                         "(abs paths, or 'mod/foo.mod' relative to --userdata), "
                         "lowest priority first. Overrides --mod.")
    ap.add_argument("--use-dlc-load", action="store_true",
                    help="Resolve the active mod-set from the launcher "
                         "dlc_load.json under --userdata (authoritative load order).")
    ap.add_argument("--dlc-load", default=None,
                    help="Explicit path to a dlc_load.json (implies --use-dlc-load).")
    ap.add_argument("--userdata", default=DEFAULT_USERDATA,
                    help="the game user-data dir holding dlc_load.json + mod/*.mod "
                         "(default: the 'the game' dir above this repo).")
    ap.add_argument("--language", default="english",
                    help="Localisation language subdir to resolve (default: english).")
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
