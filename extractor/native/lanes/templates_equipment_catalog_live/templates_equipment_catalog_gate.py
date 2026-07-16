#!/usr/bin/env python3
"""Build and verify the domain-59 static template/equipment catalog artifact.

This lane-local gate is intentionally separate from the shared collector and
recorder entrypoints. It produces a deterministic static artifact that can be
packed into a `.battleplan` container and byte-identity verified. Live-changing
template mutations still require the shared DLL registration plan documented in
this lane.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any


LANE_DIR = Path(__file__).resolve().parent
REPO_ROOT = LANE_DIR.parents[3]
GAME_ASSETS = REPO_ROOT / "replay_interface" / "game_assets"
GAME_DEFS = REPO_ROOT / "extractor" / "game_defs"
DEFAULT_VANILLA_FALLBACK = Path(r"C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV")
DEFAULT_MOD = (REPO_ROOT.parent / "test").resolve()
DEFAULT_DESCRIPTOR = DEFAULT_MOD.with_suffix(".mod")
DEFAULT_OUT = LANE_DIR / "proofs" / "domain59_static_gate"
DOMAIN_ID = 59
GAME_ID = "templates_equipment_catalog_live_domain59"
STEAM_APP_ID = "394360"

sys.path.insert(0, str(GAME_ASSETS))
sys.path.insert(0, str(GAME_DEFS))
from paradox_parser import Node, parse  # noqa: E402
from extract import (  # noqa: E402
    Layer,
    build_file_map_layers,
    build_modset_layers,
    parse_mod_descriptor,
)
import source_inventory_layers as SIL  # noqa: E402


STATIC_SUBDIRS = {
    "unit_definition_files": ("common/units", (".txt",)),
    "equipment_definition_files": ("common/units/equipment", (".txt",)),
    "division_template_files": ("history/units", (".txt",)),
}

DEFAULT_STEAM_LIBRARY_FILES = [
    Path("C:/Program Files (x86)/Steam/steamapps/libraryfolders.vdf"),
    Path("C:/Program Files/Steam/steamapps/libraryfolders.vdf"),
    Path("X:/SteamLibrary/steamapps/libraryfolders.vdf"),
    Path("X:/SteamLibrary/steamapps/libraryfolders.vdf"),
    Path("X:/SteamLibrary/steamapps/libraryfolders.vdf"),
    Path("X:/SteamLibrary/steamapps/libraryfolders.vdf"),
    Path("X:/SteamLibrary/steamapps/libraryfolders.vdf"),
    Path("X:/SteamLibrary/steamapps/libraryfolders.vdf"),
    Path("X:/SteamLibrary/steamapps/libraryfolders.vdf"),
    Path("X:/SteamLibrary/steamapps/libraryfolders.vdf"),
]


def canonical_json(data: Any) -> bytes:
    return json.dumps(data, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_text_lossless(path: Path) -> str:
    return path.read_bytes().decode("utf-8-sig", errors="replace")


def is_vanilla_root(path: Path) -> bool:
    return (
        path.is_dir()
        and (path / "hoi4.exe").is_file()
        and (path / "common").is_dir()
        and (path / "history").is_dir()
    )


def parse_vdf_string_values(text: str) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in text.splitlines():
        line = raw.strip()
        if not line.startswith('"'):
            continue
        parts = line.split('"')
        if len(parts) >= 4:
            values[parts[1]] = parts[3].replace("\\\\", "\\")
    return values


def steam_library_roots(library_file: Path) -> list[Path]:
    if not library_file.is_file():
        return []
    roots: list[Path] = []
    text = read_text_lossless(library_file)
    for raw in text.splitlines():
        line = raw.strip()
        if line.startswith('"path"'):
            values = parse_vdf_string_values(line)
            if "path" in values:
                roots.append(Path(values["path"]))
    return roots


def steam_install_dir(library_root: Path) -> str:
    manifest = library_root / "steamapps" / f"appmanifest_{STEAM_APP_ID}.acf"
    if not manifest.is_file():
        return "the game"
    values = parse_vdf_string_values(read_text_lossless(manifest))
    return values.get("installdir") or "the game"


def running_hoi4_process_paths() -> list[Path]:
    if os.name != "nt":
        return []
    cmd = [
        "powershell",
        "-NoProfile",
        "-Command",
        "Get-Process the game -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Path",
    ]
    proc = subprocess.run(cmd, text=True, capture_output=True)
    if proc.returncode != 0:
        return []
    return [Path(line.strip()) for line in proc.stdout.splitlines() if line.strip()]


def discover_vanilla_root(
    explicit: Path | None,
    *,
    env: dict[str, str] | os._Environ[str] = os.environ,
    process_paths: list[Path] | None = None,
    steam_library_files: list[Path] | None = None,
    fallback: Path = DEFAULT_VANILLA_FALLBACK,
) -> tuple[Path | None, dict[str, Any]]:
    checked: list[dict[str, Any]] = []

    def record(source: str, candidate: Path, detail: str) -> Path | None:
        resolved = candidate.resolve() if candidate.exists() else candidate
        valid = is_vanilla_root(resolved)
        checked.append(
            {
                "source": source,
                "path": str(resolved),
                "exists": resolved.exists(),
                "valid": valid,
                "detail": detail,
            }
        )
        return resolved if valid else None

    if explicit is not None:
        found = record("explicit --vanilla-root/--vanilla", explicit, "user supplied")
        return found, {"selected": str(found) if found else None, "checked": checked}

    env_value = env.get("BATTLE_PLAN_VANILLA")
    if env_value:
        found = record("BATTLE_PLAN_VANILLA", Path(env_value), "environment variable")
        if found:
            return found, {"selected": str(found), "checked": checked}

    proc_paths = process_paths if process_paths is not None else running_hoi4_process_paths()
    for proc_path in proc_paths:
        found = record("running hoi4.exe process", proc_path.parent, f"process path {proc_path}")
        if found:
            return found, {"selected": str(found), "checked": checked}

    for library_file in steam_library_files or DEFAULT_STEAM_LIBRARY_FILES:
        if not library_file.is_file():
            checked.append(
                {
                    "source": "Steam libraryfolders.vdf",
                    "path": str(library_file),
                    "exists": False,
                    "valid": False,
                    "detail": "library metadata file not found",
                }
            )
            continue
        for library_root in steam_library_roots(library_file):
            manifest = library_root / "steamapps" / f"appmanifest_{STEAM_APP_ID}.acf"
            if not manifest.is_file():
                checked.append(
                    {
                        "source": "Steam appmanifest_394360.acf",
                        "path": str(manifest),
                        "exists": False,
                        "valid": False,
                        "detail": f"library declared by {library_file}",
                    }
                )
                continue
            install_dir = steam_install_dir(library_root)
            found = record(
                "Steam appmanifest_394360.acf",
                library_root / "steamapps" / "common" / install_dir,
                f"library metadata {library_file}; installdir {install_dir}",
            )
            if found:
                return found, {"selected": str(found), "checked": checked}

    found = record("documented Steam fallback", fallback, "generic fallback path")
    return found, {"selected": str(found) if found else None, "checked": checked}


def node_to_canonical(value: Any) -> Any:
    if not isinstance(value, Node):
        return value
    out: dict[str, Any] = {}
    anonymous: list[Any] = []
    for key, child in value.entries:
        converted = node_to_canonical(child)
        if key == "":
            anonymous.append(converted)
            continue
        if key in out:
            if not isinstance(out[key], list) or (
                out[key] and isinstance(out[key], list) and out[key][0] == {"__not_array_marker__": True}
            ):
                out[key] = [out[key]]
            out[key].append(converted)
        else:
            out[key] = converted
    if anonymous:
        out["_values"] = anonymous
    return out


def entries_named(root: Node, name: str) -> list[Node]:
    wanted = name.lower()
    return [value for key, value in root.entries if key.lower() == wanted and isinstance(value, Node)]


def scalar(node: Node, key: str) -> str | None:
    value = node.get(key)
    return value if isinstance(value, str) else None


def grid_entries(node: Node | None) -> list[dict[str, Any]]:
    if not isinstance(node, Node):
        return []
    rows: list[dict[str, Any]] = []
    for unit_type, payload in node.entries:
        if not unit_type or not isinstance(payload, Node):
            continue
        rows.append(
            {
                "unit_type": unit_type,
                "x": scalar(payload, "x"),
                "y": scalar(payload, "y"),
                "raw": node_to_canonical(payload),
            }
        )
    rows.sort(key=lambda row: (str(row.get("y")), str(row.get("x")), row["unit_type"]))
    return rows


def collect_top_level_blocks(files: dict[str, Path], *, skip_names: set[str] | None = None) -> dict[str, Any]:
    skip = {name.lower() for name in (skip_names or set())}
    blocks: dict[str, Any] = {}
    duplicate_names: dict[str, int] = {}
    for rel, path in sorted(files.items()):
        root = parse(read_text_lossless(path))
        for key, value in root.entries:
            if not key or key.lower() in skip or not isinstance(value, Node):
                continue
            entry = {
                "token": key,
                "source": rel,
                "source_sha256": sha256_bytes(path.read_bytes()),
                "fields": node_to_canonical(value),
            }
            if key in blocks:
                duplicate_names[key] = duplicate_names.get(key, 1) + 1
                blocks[f"{key}#{duplicate_names[key]}"] = entry
            else:
                blocks[key] = entry
    return dict(sorted(blocks.items()))


def collect_division_templates(files: dict[str, Path]) -> list[dict[str, Any]]:
    templates: list[dict[str, Any]] = []
    for rel, path in sorted(files.items()):
        root = parse(read_text_lossless(path))
        for ordinal, tmpl in enumerate(entries_named(root, "division_template")):
            regiments = grid_entries(tmpl.get("regiments"))
            support = grid_entries(tmpl.get("support"))
            templates.append(
                {
                    "source": rel,
                    "source_sha256": sha256_bytes(path.read_bytes()),
                    "ordinal_in_file": ordinal,
                    "name": scalar(tmpl, "name"),
                    "division_names_group": scalar(tmpl, "division_names_group"),
                    "is_locked": scalar(tmpl, "is_locked"),
                    "regiment_count": len(regiments),
                    "support_count": len(support),
                    "regiments": regiments,
                    "support": support,
                    "raw": node_to_canonical(tmpl),
                }
            )
    templates.sort(key=lambda row: (row["source"], row["ordinal_in_file"], row.get("name") or ""))
    return templates


def make_layers(
    vanilla: Path | None,
    mod: Path | None,
    descriptor: Path | None,
    allow_missing_vanilla: bool,
    mods: list[str] | None = None,
    userdata: Path | None = None,
    strict_mods: bool = False,
) -> tuple[list[Layer], list[str], dict[str, Any]]:
    warnings: list[str] = []
    layers: list[Layer] = []
    discovered_vanilla, discovery = discover_vanilla_root(vanilla)
    if discovered_vanilla is not None:
        layers.append(Layer("vanilla", discovered_vanilla))
    elif allow_missing_vanilla:
        warnings.append("vanilla root missing; mod-only bounded proof")
    else:
        checked = "; ".join(f"{row['source']}={row['path']}" for row in discovery["checked"])
        raise SystemExit(
            "vanilla root missing after bounded discovery; "
            "set BATTLE_PLAN_VANILLA, pass --vanilla-root, or use --allow-missing-vanilla for bounded proof. "
            f"checked: {checked}"
        )

    if mods is not None:
        if discovered_vanilla is None:
            raise SystemExit("explicit mod layers require a readable vanilla root")
        user_root = userdata.resolve() if userdata else REPO_ROOT.parent.parent.resolve()
        descriptors = []
        for token in mods:
            candidate = Path(token.strip().replace("\\", "/"))
            descriptors.append(candidate if candidate.is_absolute() else user_root / candidate)
        resolved, mod_warnings = build_modset_layers(discovered_vanilla, descriptors, user_root)
        if strict_mods and (mod_warnings or len(resolved) != 1 + len(descriptors)):
            detail = "; ".join(mod_warnings) or "layer count mismatch"
            raise SystemExit(f"strict mod resolution failed: {detail}")
        layers = resolved
        warnings.extend(mod_warnings)
    elif mod and mod.is_dir():
        replace_paths: list[str] = []
        label = mod.name
        if descriptor and descriptor.is_file():
            meta = parse_mod_descriptor(descriptor)
            replace_paths = list(meta.get("replace_path") or [])
            label = meta.get("name") or label
        layers.append(Layer(label, mod, replace_paths, {"descriptor": str(descriptor) if descriptor else None}))
    elif mod:
        warnings.append(f"mod root missing and skipped: {mod}")

    if not layers:
        raise SystemExit("no readable vanilla or mod layers")
    return layers, warnings, discovery


def build_catalog(args: argparse.Namespace) -> dict[str, Any]:
    layers, warnings, discovery = make_layers(
        args.vanilla_root,
        args.mod,
        args.mod_descriptor,
        args.allow_missing_vanilla,
        getattr(args, "mods", None),
        getattr(args, "userdata", None),
        getattr(args, "strict_mods", False),
    )
    layers, source_namespace_proof = SIL.definition_layers(
        layers, getattr(args, "source_inventory", None)
    )
    args.vanilla_root_discovery = discovery
    maps = {
        key: build_file_map_layers(layers, subdir, exts)
        for key, (subdir, exts) in STATIC_SUBDIRS.items()
    }
    unit_definitions = collect_top_level_blocks(
        maps["unit_definition_files"],
        skip_names={"division_template", "sub_units", "group", "categories"},
    )
    equipment_definitions = collect_top_level_blocks(maps["equipment_definition_files"])
    templates = collect_division_templates(maps["division_template_files"])
    source_files = {
        key: [
            {
                "relpath": rel,
                "sha256": sha256_bytes(path.read_bytes()),
                "bytes": path.stat().st_size,
            }
            for rel, path in sorted(file_map.items())
        ]
        for key, file_map in maps.items()
    }

    catalog = {
        "schema": "templates_equipment_catalog_live.static_artifact.v1",
        "domain_id": DOMAIN_ID,
        "artifact_kind": "deterministic_static_replay_enrichment",
        "scope": {
            "closed_rows": [
                "unit templates and equipment definitions: static equipment archetype definition artifact",
                "division template catalog: scripted starting-template artifact",
            ],
            "not_closed_rows": [
                "live-created or edited division templates",
                "live division-template-id to catalog joins",
                "runtime equipment requirement deltas after template edits",
            ],
        },
        "layers": [
            {
                "label": layer.label,
                "root": str(layer.root),
                "replace_path": list(layer.replace_paths),
            }
            for layer in layers
        ],
        "source_namespace_proof": source_namespace_proof,
        "warnings": warnings,
        "source_files": source_files,
        "catalog": {
            "division_templates": templates,
            "unit_definitions": unit_definitions,
            "equipment_definitions": equipment_definitions,
        },
        "counts": {
            "division_template_files": len(maps["division_template_files"]),
            "division_templates": len(templates),
            "unit_definition_files": len(maps["unit_definition_files"]),
            "unit_definitions": len(unit_definitions),
            "equipment_definition_files": len(maps["equipment_definition_files"]),
            "equipment_definitions": len(equipment_definitions),
        },
    }
    catalog["artifact_sha256"] = sha256_bytes(canonical_json(catalog))
    return catalog


def write_store(out_dir: Path, catalog: dict[str, Any]) -> dict[str, Path]:
    game_dir = out_dir / GAME_ID
    game_dir.mkdir(parents=True, exist_ok=True)
    catalog_bytes = json.dumps(catalog, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8")
    catalog_path = game_dir / "domain59_static_catalog.json"
    catalog_path.write_bytes(catalog_bytes)
    snapshot = {
        "schema_version": "templates_equipment_catalog_live.static_snapshot.v1",
        "provenance": {
            "mod": {
                "domain_id": DOMAIN_ID,
                "artifact_sha256": sha256_bytes(catalog_bytes),
                "layers": catalog["layers"],
            }
        },
        "catalog_file": catalog_path.name,
        "catalog_sha256": sha256_bytes(catalog_bytes),
        "counts": catalog["counts"],
    }
    snapshot_bytes = json.dumps(snapshot, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8")
    snapshot_path = game_dir / "snapshot_000000.json"
    snapshot_path.write_bytes(snapshot_bytes)
    index = {
        "schema": "templates_equipment_catalog_live.static_index.v1",
        "game_id": GAME_ID,
        "snapshots": [
            {
                "hour_index": 0,
                "file": snapshot_path.name,
                "role": "domain59_static_catalog_manifest",
                "sha256": sha256_bytes(snapshot_bytes),
            },
            {
                "hour_index": 0,
                "file": catalog_path.name,
                "role": "domain59_static_catalog_payload",
                "sha256": sha256_bytes(catalog_bytes),
            },
        ],
    }
    index_path = game_dir / "index.json"
    index_path.write_bytes(json.dumps(index, ensure_ascii=False, sort_keys=True, indent=2).encode("utf-8"))
    return {"game_dir": game_dir, "catalog": catalog_path, "snapshot": snapshot_path, "index": index_path}


def run_pack_verify(out_dir: Path) -> dict[str, Any]:
    replay_path = out_dir / f"{GAME_ID}.battleplan"
    pack_cmd = [
        "cargo",
        "run",
        "--manifest-path",
        str(REPO_ROOT / "extractor/native" / "Cargo.toml"),
        "-p",
        "snapshot",
        "--bin",
        "pack_replay",
        "--",
        "pack",
        str(out_dir),
        GAME_ID,
        str(replay_path),
    ]
    verify_cmd = [
        "cargo",
        "run",
        "--manifest-path",
        str(REPO_ROOT / "extractor/native" / "Cargo.toml"),
        "-p",
        "snapshot",
        "--bin",
        "pack_replay",
        "--",
        "verify",
        str(replay_path),
        str(out_dir),
        GAME_ID,
    ]
    pack = subprocess.run(pack_cmd, cwd=REPO_ROOT, text=True, capture_output=True)
    verify = subprocess.run(verify_cmd, cwd=REPO_ROOT, text=True, capture_output=True) if pack.returncode == 0 else None
    return {
        "replay_path": str(replay_path),
        "pack": {
            "cmd": pack_cmd,
            "returncode": pack.returncode,
            "stdout": pack.stdout,
            "stderr": pack.stderr,
        },
        "verify": None
        if verify is None
        else {
            "cmd": verify_cmd,
            "returncode": verify.returncode,
            "stdout": verify.stdout,
            "stderr": verify.stderr,
        },
    }


def apply_layer_mode(args: argparse.Namespace) -> None:
    if args.vanilla_only and getattr(args, "mods", None) is not None:
        raise SystemExit("--vanilla-only and --mods are mutually exclusive")
    if args.vanilla_only:
        args.mod = None
        args.mod_descriptor = None
        args.mods = []
    elif getattr(args, "mods", None) is not None:
        args.mod = None
        args.mod_descriptor = None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--vanilla-root", "--vanilla", dest="vanilla_root", type=Path, default=None)
    parser.add_argument("--mod", type=Path, default=DEFAULT_MOD)
    parser.add_argument("--mod-descriptor", type=Path, default=DEFAULT_DESCRIPTOR)
    parser.add_argument("--mods", default=None, help="explicit comma-separated ordered .mod tokens")
    parser.add_argument("--userdata", type=Path, default=None)
    parser.add_argument("--strict-mods", action="store_true")
    parser.add_argument("--source-inventory", type=Path, default=None)
    parser.add_argument(
        "--vanilla-only",
        action="store_true",
        help="explicitly build without a mod layer; never inferred from a missing mod path",
    )
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--allow-missing-vanilla", action="store_true")
    parser.add_argument("--skip-pack", action="store_true")
    args = parser.parse_args()
    if args.mods is not None:
        args.mods = [part.strip() for part in args.mods.split(",") if part.strip()]
    apply_layer_mode(args)

    catalog = build_catalog(args)
    paths = write_store(args.out_dir, catalog)
    proof: dict[str, Any] = {
        "schema": "templates_equipment_catalog_live.static_gate_report.v1",
        "domain_id": DOMAIN_ID,
        "passed": False,
        "paths": {key: str(path) for key, path in paths.items()},
        "counts": catalog["counts"],
        "artifact_sha256": catalog["artifact_sha256"],
        "warnings": catalog["warnings"],
        "vanilla_root_discovery": getattr(args, "vanilla_root_discovery", None),
        "pack_verify": None,
    }
    if not args.skip_pack:
        proof["pack_verify"] = run_pack_verify(args.out_dir)
    verify_rc = (proof.get("pack_verify") or {}).get("verify", {}).get("returncode")
    proof["passed"] = bool(
        catalog["counts"]["division_templates"] > 0
        and catalog["counts"]["equipment_definitions"] > 0
        and (args.skip_pack or verify_rc == 0)
        and not catalog["warnings"]
    )
    proof_path = args.out_dir / "domain59_static_gate_report.json"
    proof_path.parent.mkdir(parents=True, exist_ok=True)
    proof_path.write_text(json.dumps(proof, ensure_ascii=False, sort_keys=True, indent=2), encoding="utf-8")
    print(json.dumps(proof, ensure_ascii=False, sort_keys=True, indent=2))
    return 0 if proof["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
