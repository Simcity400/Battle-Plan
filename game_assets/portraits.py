#!/usr/bin/env python3
"""the game character-portrait extraction for the MP-Stats replay UI.

Companion to extract.py. The save parsers (save_oracle/output/characters.json,
intel.json) reference leader / general / admiral / advisor / operative faces by
sprite NAME (`portrait_gfx`, `gfx`, e.g. `GFX_portrait_GER_*`,
`GFX_idea_*_advisor`, operative `gfx`). This module:

  1. Parses the portrait spriteType definitions (name -> texturefile) across the
     WHOLE effective interface load set: vanilla + every integrated_dlc/* and
     dlc/* pack + the mod overlay (later roots override earlier by sprite name).
  2. Converts the referenced portrait textures (.tga/.dds incl. DXT + DX10) to
     web PNGs under assets/portraits/ -- REUSING extract.py's load_texture /
     slice_and_save / resolve helpers (nothing re-implemented here).
  3. Emits portraits.json mapping each portrait sprite name -> PNG path.
  4. Cross-checks coverage vs the names actually used in characters.json +
     intel.json and reports resolved / missing counts. Missing art is reported,
     never faked.

Scope discipline: writes ONLY under game_assets/ (assets/portraits/ + the
manifest + a gitignored diagnostics file). The converted art under assets/ stays
gitignored (Paradox IP, private offline use); only this script + portraits.json
are committed. No personal home path is baked into any committed file -- install
paths come from CLI args / the documented defaults shared with extract.py.

Run `python portraits.py --help`.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

# Reuse the existing, already-built pipeline pieces. Do NOT re-implement the
# Clausewitz parser, the .gfx walker, the TGA/DDS(DXT+DX10) loader, or the
# slice/save logic -- import them.
import extract as ex
from extract import (
    HERE,
    DEFAULT_VANILLA,
    DEFAULT_MOD,
    parse_gfx_file,
    resolve_texture,
    load_texture,
    slice_and_save,
    safe_basename,
)
from paradox_parser import parse


# --------------------------------------------------------------------------
# Effective load-set roots (mod-overrides-dlc-overrides-vanilla, by rel path /
# by sprite name). Order matters: later roots win. Within vanilla, integrated
# DLC and stand-alone DLC packs each carry their own interface/ + gfx/ subtree.
# --------------------------------------------------------------------------
def discover_roots(vanilla: Path, mod: Path | None) -> list[Path]:
    """Return the ordered list of asset roots to overlay.

    Order (earliest = lowest priority): vanilla base, then each integrated_dlc
    pack, then each stand-alone dlc pack, then the mod overlay last. DLC packs
    are sorted by directory name for determinism. Each returned path is a
    directory that may contain interface/ and/or gfx/ subtrees.
    """
    roots: list[Path] = [vanilla]
    for group in ("integrated_dlc", "dlc"):
        gdir = vanilla / group
        if gdir.is_dir():
            for sub in sorted(p for p in gdir.iterdir() if p.is_dir()):
                roots.append(sub)
    if mod is not None:
        roots.append(mod)
    return roots


def build_overlay_map(roots: list[Path], subdir: str, exts) -> dict:
    """{relative_posix_path -> absolute Path} across all roots; later roots
    override earlier ones at the same relative path (engine semantics).

    `relative_posix_path` is relative to each root's `subdir` so a vanilla
    interface/foo.gfx and a dlc interface/foo.gfx that share the name collide
    only if they truly share the same in-engine relative path -- which is the
    correct override behaviour. For gfx textures the rel path is relative to the
    root itself (texturefile strings are root-relative, e.g. "gfx/leaders/..").
    """
    eff: dict[str, Path] = {}
    for base in roots:
        root = base / subdir
        if not root.is_dir():
            continue
        for p in root.rglob("*"):
            if p.is_file() and p.suffix.lower() in exts:
                if subdir == "gfx":
                    # texturefile strings are root-relative ("gfx/...") -> key
                    # by path relative to the ROOT so they line up with .gfx refs
                    rel = p.relative_to(base).as_posix()
                else:
                    rel = p.relative_to(base / subdir).as_posix()
                eff[rel] = p
    return eff


def build_portrait_sprite_map(roots: list[Path]) -> tuple[dict, list]:
    """Parse every interface/*.gfx across all roots into one sprite manifest.

    Reuses extract.parse_gfx_file (handles spriteType / frameAnimated /
    corneredTile / masked / progressbar tolerantly). Later roots override
    earlier definitions of the same sprite name (DLC/mod overriding vanilla).
    """
    sprites: dict = {}
    dup_log: list = []
    for base in roots:
        idir = base / "interface"
        if not idir.is_dir():
            continue
        for path in sorted(idir.rglob("*.gfx")):
            try:
                text = path.read_bytes().decode("latin-1")
            except Exception as e:  # noqa: BLE001
                dup_log.append({"name": "<read-error>", "file": str(path), "error": str(e)})
                continue
            try:
                root = parse(text)
                # source_rel = root-relative for traceability
                src_rel = path.relative_to(base).as_posix()
                parse_gfx_file(root, src_rel, sprites, dup_log)
            except Exception as e:  # noqa: BLE001
                dup_log.append({"name": "<parse-error>", "file": str(path), "error": str(e)})
    return sprites, dup_log


# --------------------------------------------------------------------------
# Referenced-name harvest from the save-oracle outputs
# --------------------------------------------------------------------------
_PORTRAIT_KEYS = ("portrait_gfx", "gfx")


def harvest_referenced_names(output_dir: Path) -> dict:
    """Collect every distinct portrait sprite name referenced by the save
    outputs. Returns {name: [source_files...]} so misses can be attributed.

    Walks characters.json + intel.json, picking up any `portrait_gfx` / `gfx`
    string value (the operative portrait field is `gfx`).
    """
    refs: dict[str, set] = {}
    for fname in ("characters.json", "intel.json"):
        fp = output_dir / fname
        if not fp.exists():
            continue
        try:
            data = json.loads(fp.read_text(encoding="utf-8"))
        except Exception:  # noqa: BLE001
            continue

        def walk(o):
            if isinstance(o, dict):
                for k, v in o.items():
                    if k in _PORTRAIT_KEYS and isinstance(v, str) and v:
                        refs.setdefault(v, set()).add(fname)
                    else:
                        walk(v)
            elif isinstance(o, list):
                for x in o:
                    walk(x)

        walk(data)
    return {k: sorted(v) for k, v in refs.items()}


# --------------------------------------------------------------------------
# Name -> texture resolution
# --------------------------------------------------------------------------
def _ci_sprite_lookup(name: str, sprite_index: dict):
    """Case-insensitive sprite-name lookup. characters.json mixes casing
    (`GFX_Portrait_Asia_Generic_3` vs `GFX_portrait_asia_generic_4`); the engine
    treats sprite names case-insensitively, so we must too. Exact hit first."""
    sp = sprite_index.get(name)
    if sp is not None:
        return sp
    return sprite_index.get(name.lower())


def resolve_reference(name: str, sprite_index: dict, sprite_index_lc: dict,
                      gfx_map: dict):
    """Resolve a referenced portrait name to (texture_abs_path, how, texrel).

    Two name shapes occur in the save outputs:
      * a sprite NAME (`GFX_portrait_*`, `GFX_idea_*`, bare `Portrait_*`) -> look
        up its texturefile in the parsed sprite manifest, then resolve that to a
        file on disk (trying alternate image extensions, exactly as extract.py).
      * a RAW texturefile path used directly as the portrait id (a handful of
        ministers/leaders come through as `gfx/leaders/.../foo.dds`) -> resolve
        the path straight against the texture map.
    Returns (Path|None, reason_str, texrel_or_None).
    """
    # Raw-path form: value already looks like a gfx/... texture path.
    low = name.lower()
    if low.endswith((".dds", ".tga", ".png")) or low.startswith("gfx/"):
        texrel = ex.normalize_texpath(name)
        src = resolve_texture(texrel, gfx_map) if texrel else None
        if src is not None and src.exists():
            return src, "raw_path", texrel
        return None, "raw_path_texture_not_found", texrel

    # Sprite-name form. Consult the case-insensitive index FIRST: it is built
    # last-wins per lowercased name across load order, so a DLC/mod definition
    # correctly overrides a differently-cased vanilla one (e.g. vanilla
    # GFX_Portrait_Foo vs DLC GFX_portrait_foo). Exact-match is only a redundant
    # fallback (identical result when no case collision exists).
    sp = sprite_index_lc.get(low) or sprite_index.get(name)
    if sp is None:
        return None, "sprite_name_undefined", None
    texrel = sp.get("texturefile")
    if not texrel:
        return None, "sprite_has_no_texture", None
    src = resolve_texture(texrel, gfx_map)
    if src is not None and src.exists():
        return src, "sprite", texrel
    return None, "texture_not_found", texrel


# --------------------------------------------------------------------------
# Main build
# --------------------------------------------------------------------------
def run(args) -> int:
    from PIL import Image  # noqa: F401  ensure Pillow present early

    vanilla = Path(args.vanilla)
    mod = Path(args.mod) if args.mod else None
    output_dir = Path(args.save_output) if args.save_output else \
        (HERE.parent / "save_oracle" / "output")
    out_assets = HERE / "assets" / "portraits"

    if not vanilla.is_dir():
        print(f"ERROR: vanilla install not found: {vanilla}")
        return 2
    if mod and not mod.is_dir():
        print(f"WARNING: mod overlay not found, using vanilla only: {mod}")
        mod = None

    roots = discover_roots(vanilla, mod)
    print(f"[*] vanilla = {vanilla}")
    print(f"[*] mod     = {mod}")
    print(f"[*] {len(roots)} asset roots in load order "
          f"(vanilla + {len(roots) - (2 if mod else 1)} dlc packs"
          f"{' + mod' if mod else ''})")

    # ---- 1. parse all portrait sprite defs --------------------------------
    sprites, dup_log = build_portrait_sprite_map(roots)
    # case-insensitive index for tolerant lookup
    sprite_index_lc = {k.lower(): v for k, v in sprites.items()}
    print(f"[*] parsed sprite definitions across all roots -> {len(sprites)} "
          f"unique sprite names ({len(dup_log)} dup/override events)")

    # ---- 2. texture overlay map (gfx/** across all roots) -----------------
    gfx_map = build_overlay_map(roots, "gfx", (".dds", ".tga", ".png"))
    print(f"[*] texture overlay map: {len(gfx_map)} gfx files")

    # ---- 3. harvest referenced names --------------------------------------
    refs = harvest_referenced_names(output_dir)
    print(f"[*] referenced portrait names from save outputs: {len(refs)} "
          f"(from {output_dir})")

    # ---- 4. resolve + convert ---------------------------------------------
    manifest_portraits: dict = {}
    misses: list = []
    converted = 0
    failures: list = []
    frame_anomalies: list = []
    # de-dupe by resolved texture so the ~1500 names sharing a few dozen generic
    # textures don't each re-decode/re-write the same PNG. Cache the conversion
    # result (png paths + dims) keyed by absolute source path.
    tex_cache: dict[str, dict] = {}

    # We only convert textures that are actually referenced (keeps output to the
    # faces the replay needs, not all ~10k portrait art files on disk).
    for name in sorted(refs):
        src, how, texrel = resolve_reference(
            name, sprites, sprite_index_lc, gfx_map)
        if src is None:
            misses.append({
                "name": name, "reason": how, "texturefile": texrel,
                "referenced_in": refs[name],
            })
            continue

        src_key = str(src)
        cached = tex_cache.get(src_key)
        if cached is None:
            if args.manifest_only:
                # resolve + report coverage only; record metadata, skip bytes.
                cached = {"png": [], "width": None, "height": None,
                          "source_file": _root_rel(src, roots), "skipped": True}
                tex_cache[src_key] = cached
            else:
                safe = safe_basename(name)
                out_base = out_assets / safe
                try:
                    im = load_texture(src)
                    # Portraits are single-frame; pass 1 explicitly.
                    pngs, anomaly = slice_and_save(im, 1, out_base, safe)
                    if anomaly:
                        frame_anomalies.append(anomaly)
                    cached = {
                        "png": [f"assets/portraits/{p}" for p in pngs],
                        "width": im.size[0], "height": im.size[1],
                        "source_file": _root_rel(src, roots),
                    }
                    tex_cache[src_key] = cached
                    converted += 1
                except Exception as e:  # noqa: BLE001
                    failures.append({"name": name, "texture": texrel,
                                     "reason": f"{type(e).__name__}: {e}"})
                    continue

        entry = {
            "png": cached["png"],
            "texturefile": texrel,
            "resolved_via": how,
            "referenced_in": refs[name],
            "source_file": cached["source_file"],
        }
        if cached.get("width") is not None:
            entry["width"], entry["height"] = cached["width"], cached["height"]
        manifest_portraits[name] = entry

    # ---- 5. coverage report -----------------------------------------------
    total_refs = len(refs)
    resolved = len(manifest_portraits)
    miss_by_reason: dict[str, int] = {}
    for m in misses:
        miss_by_reason[m["reason"]] = miss_by_reason.get(m["reason"], 0) + 1

    manifest = {
        "portraits": manifest_portraits,
        "_meta": {
            "referenced_names": total_refs,
            "resolved_names": resolved,
            "missing_names": len(misses),
            "coverage_pct": round(100.0 * resolved / total_refs, 1) if total_refs else 0.0,
            "unique_textures_converted": converted,
            "conversion_failures": len(failures),
            "frame_anomalies": len(frame_anomalies),
            "missing_by_reason": miss_by_reason,
            "asset_roots": len(roots),
            "note": "PNG paths are relative to game_assets/; assets/ is gitignored. "
                    "Portrait names matched case-insensitively against the effective "
                    "(vanilla+DLC+mod) sprite load set.",
        },
    }
    (HERE / "portraits.json").write_text(json.dumps(manifest, indent=1),
                                         encoding="utf-8")

    diag = {
        "missing": misses,
        "conversion_failures": failures,
        "frame_anomalies": frame_anomalies,
        "dup_override_events": dup_log[:1000],
    }
    (HERE / "portraits_report.json").write_text(json.dumps(diag, indent=1),
                                                encoding="utf-8")

    # ---- console summary (ASCII only) -------------------------------------
    print("\n==== PORTRAIT EXTRACTION SUMMARY ====")
    print(f"referenced portrait names : {total_refs}")
    print(f"resolved -> manifest      : {resolved}  "
          f"({manifest['_meta']['coverage_pct']}% coverage)")
    print(f"unique textures converted : {converted}")
    print(f"missing (no art / undef)  : {len(misses)}")
    for reason, n in sorted(miss_by_reason.items(), key=lambda x: -x[1]):
        print(f"    {reason:28s}: {n}")
    print(f"conversion failures       : {len(failures)}")
    print(f"frame anomalies           : {len(frame_anomalies)}")
    if misses[:8]:
        print("  sample misses:")
        for m in misses[:8]:
            print(f"    {m['name']}  ({m['reason']})")
    print("\nwrote portraits.json + portraits_report.json (gitignored diag)")
    return 0


# small helper kept module-level so run() stays readable ------------------
def _root_rel(src: Path, roots: list[Path]):
    """Return src as a path relative to whichever asset root contains it
    (root-relative, e.g. 'gfx/leaders/GER/foo.dds'); never an absolute/personal
    path so it is safe even though portraits.json is committed... but to be
    extra safe we still strip to the gfx/ tail."""
    s = src.as_posix()
    idx = s.lower().rfind("/gfx/")
    if idx != -1:
        return s[idx + 1:]
    for r in roots:
        try:
            return src.relative_to(r).as_posix()
        except ValueError:
            continue
    return src.name


def build_argparser():
    ap = argparse.ArgumentParser(
        description="Extract the game character portraits for the replay UI.")
    ap.add_argument("--vanilla", default=DEFAULT_VANILLA,
                    help="Path to the vanilla the game install (default: documented Steam path)")
    ap.add_argument("--mod", default=DEFAULT_MOD,
                    help="Path to the mod overlay (default: ../../test). Use '' for vanilla-only.")
    ap.add_argument("--save-output", default=None,
                    help="save_oracle/output dir holding characters.json + intel.json "
                         "(default: ../save_oracle/output).")
    ap.add_argument("--manifest-only", action="store_true",
                    help="Resolve + report coverage only; skip PNG conversion.")
    return ap


def main(argv=None):
    args = build_argparser().parse_args(argv)
    if args.mod == "":
        args.mod = None
    return run(args)


if __name__ == "__main__":
    sys.exit(main())
