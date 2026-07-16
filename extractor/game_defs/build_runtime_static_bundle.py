#!/usr/bin/env python3
"""Rebuild all seven static catalogs for an already-resolved live binding set."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path
from typing import Any

import live_binding_resolver as LR
import static_bundle as SB


BUILD_SCHEMA = "extractor.runtime_static_catalog_build.v1"
FINGERPRINT_SCHEMA = "extractor.runtime_static_catalog_inputs.v1"
STAGING_SCHEMA = "extractor.runtime_static_mod_staging.v1"
HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parents[1]
TEMPLATES_GATE = (
    REPO_ROOT
    / "extractor/native/lanes/templates_equipment_catalog_live/templates_equipment_catalog_gate.py"
)


MOD_SOURCE_FIELDS = {
    "id", "ordinal", "name", "descriptor_path", "descriptor_sha256",
    "content_kind", "content_path", "content_sha256",
}
CONTENT_LINE_RE = re.compile(r"^\s*(?:path|archive)\s*=.*$", re.MULTILINE | re.IGNORECASE)


def _safe_launcher_token(token: str, destination: Path) -> Path:
    normalized = token.replace("\\", "/")
    candidate = Path(normalized)
    if (
        not normalized
        or "," in normalized
        or candidate.is_absolute()
        or candidate.suffix.casefold() != ".mod"
        or any(part in {"", ".", ".."} for part in candidate.parts)
    ):
        raise LR.ResolutionError(f"unsafe or ambiguous launcher mod token: {token!r}")
    resolved = (destination / candidate).resolve()
    try:
        resolved.relative_to(destination.resolve())
    except ValueError as exc:
        raise LR.ResolutionError(f"launcher mod token escapes staging userdir: {token!r}") from exc
    return resolved


def _load_resolved_mod_sources(
    bindings_dir: Path, mod_state: dict[str, Any]
) -> list[dict[str, Any]]:
    resolution = SB.load_object(bindings_dir / "resolution_report.json", "live resolution report")
    sources = resolution.get("resolved_mod_sources")
    if not isinstance(sources, list) or not all(isinstance(row, dict) for row in sources):
        raise LR.ResolutionError("live resolution report lacks resolved_mod_sources")
    load_order = SB._validate_binding_document(
        SB.load_object(bindings_dir / "load_order.json", "resolved load order"),
        "load_order",
    )
    enabled_by_id = {row["id"]: row for row in mod_state["mods"] if row["enabled"]}
    ordered_ids = [row["id"] for row in load_order["items"] if row["kind"] == "mod"]
    if set(ordered_ids) != set(enabled_by_id) or len(ordered_ids) != len(enabled_by_id):
        raise LR.ResolutionError("load order mod identities do not match enabled mod state")
    enabled = [enabled_by_id[item_id] for item_id in ordered_ids]
    if len(sources) != len(enabled):
        raise LR.ResolutionError("resolved mod source count does not match enabled mod count")
    normalized: list[dict[str, Any]] = []
    for ordinal, (source, binding) in enumerate(zip(sources, enabled, strict=True)):
        if set(source) != MOD_SOURCE_FIELDS:
            raise LR.ResolutionError(f"resolved mod source {ordinal} contains unknown or missing fields")
        if source.get("ordinal") != ordinal or source.get("id") != binding["id"]:
            raise LR.ResolutionError("resolved mod sources do not preserve launcher order")
        if source.get("name") != binding["name"]:
            raise LR.ResolutionError(f"resolved mod source identity mismatch: {binding['id']}")
        if source.get("descriptor_sha256") != binding["descriptor_sha256"]:
            raise LR.ResolutionError(f"resolved mod descriptor hash mismatch: {binding['id']}")
        if source.get("content_sha256") != binding["content_sha256"]:
            raise LR.ResolutionError(f"resolved mod content hash mismatch: {binding['id']}")
        if source.get("content_kind") not in {"directory", "archive"}:
            raise LR.ResolutionError(f"resolved mod content kind is invalid: {binding['id']}")
        descriptor = Path(SB._required_string(source.get("descriptor_path"), "descriptor_path")).resolve()
        content = Path(SB._required_string(source.get("content_path"), "content_path")).resolve()
        if SB.sha256_file(descriptor) != binding["descriptor_sha256"]:
            raise LR.ResolutionError(f"resolved mod descriptor changed: {binding['id']}")
        fields = LR.parse_descriptor(descriptor)
        if fields["name"] != binding["name"]:
            raise LR.ResolutionError(f"resolved mod descriptor name changed: {binding['id']}")
        content_ref = fields.get("path") or fields["archive"]
        derived = Path(content_ref)
        if not derived.is_absolute():
            derived = descriptor.parents[1] / derived
        if derived.resolve() != content:
            raise LR.ResolutionError(f"resolved mod content path is ambiguous: {binding['id']}")
        if source["content_kind"] == "directory":
            actual_sha, _ = LR.hash_tree(content, layer_kind="mod", layer_id=binding["id"])
        else:
            if not content.is_file():
                raise LR.ResolutionError(f"resolved mod archive is missing: {binding['id']}")
            actual_sha, _ = LR._hash_file_stable(content)
        if actual_sha != binding["content_sha256"]:
            raise LR.ResolutionError(f"resolved mod content changed: {binding['id']}")
        normalized.append({**source, "descriptor_path": str(descriptor), "content_path": str(content)})
    return normalized


def _extract_archive(source: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=False)
    seen: set[str] = set()
    try:
        archive = zipfile.ZipFile(source)
    except (OSError, zipfile.BadZipFile) as exc:
        raise LR.ResolutionError(f"invalid resolved mod archive: {source}") from exc
    with archive:
        for info in archive.infolist():
            name = info.filename.replace("\\", "/")
            path = Path(name)
            if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
                raise LR.ResolutionError(f"unsafe member in resolved mod archive: {name!r}")
            key = name.rstrip("/").casefold()
            if key in seen:
                raise LR.ResolutionError(f"duplicate member in resolved mod archive: {name!r}")
            seen.add(key)
            mode = (info.external_attr >> 16) & 0o170000
            if mode == 0o120000:
                raise LR.ResolutionError(f"symlink member in resolved mod archive: {name!r}")
            target = destination.joinpath(*path.parts)
            if info.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.open(info) as src, target.open("wb") as dst:
                shutil.copyfileobj(src, dst)


def _stage_mod_descriptor(source: dict[str, Any], destination: Path, content_root: Path) -> Path:
    target = _safe_launcher_token(source["id"], destination)
    text = Path(source["descriptor_path"]).read_text(encoding="utf-8-sig", errors="strict")
    content_text = content_root.resolve().as_posix()
    if any(char in content_text for char in ('"', "\r", "\n")):
        raise LR.ResolutionError(f"resolved mod content path cannot be staged: {source['id']}")
    rendered, count = CONTENT_LINE_RE.subn(f'path="{content_text}"', text)
    if count != 1:
        raise LR.ResolutionError(f"resolved mod descriptor content field is ambiguous: {source['id']}")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(rendered, encoding="utf-8")
    return target


def prepare_runtime_userdir(bindings_dir: Path, vanilla: Path, destination: Path) -> dict[str, Any]:
    mod_state = SB.load_object(bindings_dir / "mod_state.json", "resolved mod state")
    dlc_state = SB.load_object(bindings_dir / "dlc_state.json", "resolved DLC state")
    SB._validate_binding_document(mod_state, "mod_state")
    SB._validate_binding_document(dlc_state, "dlc_state")
    mod_sources = _load_resolved_mod_sources(bindings_dir, mod_state)
    descriptors = {
        path.parent.name: path.relative_to(vanilla).as_posix()
        for path in vanilla.glob("dlc/*/*.dlc")
    }
    disabled = []
    for row in dlc_state["dlc"]:
        descriptor = descriptors.get(row["id"])
        if descriptor is None:
            raise LR.ResolutionError(f"resolved DLC lacks installed descriptor: {row['id']}")
        if not row["enabled"]:
            disabled.append(descriptor)
    destination.mkdir(parents=True, exist_ok=False)
    staged_rows = []
    for source in mod_sources:
        content_root = Path(source["content_path"])
        if source["content_kind"] == "archive":
            content_root = destination / "_materialized_mods" / f"{source['ordinal']:04d}"
            _extract_archive(Path(source["content_path"]), content_root)
        staged_descriptor = _stage_mod_descriptor(source, destination, content_root)
        staged_content_sha, staged_inventory = LR.hash_tree(
            content_root, layer_kind="staged_mod", layer_id=source["id"]
        )
        staged_rows.append({
            "id": source["id"],
            "ordinal": source["ordinal"],
            "descriptor_relpath": staged_descriptor.relative_to(destination).as_posix(),
            "descriptor_sha256": SB.sha256_file(staged_descriptor),
            "content_path": str(content_root.resolve()),
            "content_sha256": staged_content_sha,
            "content_files": len(staged_inventory),
            "source_content_sha256": source["content_sha256"],
        })
    launcher = {
        "enabled_mods": [source["id"] for source in mod_sources],
        "disabled_dlcs": sorted(disabled),
    }
    (destination / "dlc_load.json").write_bytes(SB.pretty_json(launcher))
    staging = {"schema": STAGING_SCHEMA, "mods": staged_rows}
    staging["sha256"] = SB.sha256_bytes(SB.canonical_json(staging))
    (destination / "static_mod_staging_manifest.json").write_bytes(SB.pretty_json(staging))
    return launcher


def _validate_runtime_userdir(
    bindings_dir: Path, vanilla: Path, destination: Path
) -> dict[str, Any]:
    mod_state = SB.load_object(bindings_dir / "mod_state.json", "resolved mod state")
    dlc_state = SB.load_object(bindings_dir / "dlc_state.json", "resolved DLC state")
    SB._validate_binding_document(mod_state, "mod_state")
    SB._validate_binding_document(dlc_state, "dlc_state")
    sources = _load_resolved_mod_sources(bindings_dir, mod_state)
    descriptors = {
        path.parent.name: path.relative_to(vanilla).as_posix()
        for path in vanilla.glob("dlc/*/*.dlc")
    }
    disabled = []
    for row in dlc_state["dlc"]:
        descriptor = descriptors.get(row["id"])
        if descriptor is None:
            raise LR.ResolutionError(f"resolved DLC lacks installed descriptor: {row['id']}")
        if not row["enabled"]:
            disabled.append(descriptor)
    expected_launcher = {
        "enabled_mods": [row["id"] for row in sources],
        "disabled_dlcs": sorted(disabled),
    }
    actual_launcher = SB.load_object(destination / "dlc_load.json", "runtime staging launcher state")
    if actual_launcher != expected_launcher:
        raise LR.ResolutionError("runtime staging launcher state does not match resolved bindings")
    manifest = SB.load_object(
        destination / "static_mod_staging_manifest.json", "runtime mod staging manifest"
    )
    claimed = manifest.get("sha256")
    unhashed = dict(manifest)
    unhashed.pop("sha256", None)
    if (
        manifest.get("schema") != STAGING_SCHEMA
        or claimed != SB.sha256_bytes(SB.canonical_json(unhashed))
        or not isinstance(manifest.get("mods"), list)
        or len(manifest["mods"]) != len(sources)
    ):
        raise LR.ResolutionError("runtime mod staging manifest is malformed")
    expected_descriptor_relpaths = {row["id"] for row in sources}
    actual_descriptor_relpaths = {
        path.relative_to(destination).as_posix()
        for path in destination.rglob("*.mod")
    }
    if actual_descriptor_relpaths != expected_descriptor_relpaths:
        raise LR.ResolutionError("runtime mod staging descriptor namespace changed")
    for source, row in zip(sources, manifest["mods"], strict=True):
        if row.get("id") != source["id"] or row.get("ordinal") != source["ordinal"]:
            raise LR.ResolutionError("runtime mod staging order changed")
        descriptor = (destination / str(row.get("descriptor_relpath"))).resolve()
        if SB.sha256_file(descriptor) != row.get("descriptor_sha256"):
            raise LR.ResolutionError(f"runtime staged descriptor changed: {source['id']}")
        fields = LR.parse_descriptor(descriptor)
        content = Path(fields["path"]).resolve()
        if str(content) != row.get("content_path"):
            raise LR.ResolutionError(f"runtime staged mod content path changed: {source['id']}")
        digest, inventory = LR.hash_tree(content, layer_kind="staged_mod", layer_id=source["id"])
        if digest != row.get("content_sha256") or len(inventory) != row.get("content_files"):
            raise LR.ResolutionError(f"runtime staged mod content changed: {source['id']}")
    return expected_launcher


def _implementation_inventory() -> list[dict[str, Any]]:
    paths = sorted(HERE.glob("*.py"), key=lambda path: path.name.casefold())
    paths.extend([
        TEMPLATES_GATE,
        REPO_ROOT / "replay_interface/game_assets/extract.py",
        REPO_ROOT / "replay_interface/game_assets/paradox_parser.py",
        HERE / "gfx_icon_missing_texture_allowlist.json",
    ])
    return [
        {"path": str(path.resolve()), "sha256": SB.sha256_file(path)}
        for path in paths
    ]


def _build_input_fingerprint(
    bindings_dir: Path, vanilla: Path, exe: Path, source_inventory: dict[str, Any]
) -> dict[str, Any]:
    binding_files = [
        "mod_state.json", "dlc_state.json", "load_order.json",
        "resolution_report.json", "source_inventory.json",
    ]
    value = {
        "schema": FINGERPRINT_SCHEMA,
        "vanilla_root": str(vanilla.resolve()),
        "executable_path": str(exe.resolve()),
        "executable_sha256": SB.sha256_file(exe),
        "binding_files": [
            {"name": name, "sha256": SB.sha256_file(bindings_dir / name)}
            for name in binding_files
        ],
        "source_inventory_sha256": source_inventory["inventory_sha256"],
        "implementation_files": _implementation_inventory(),
    }
    value["fingerprint_sha256"] = SB.sha256_bytes(SB.canonical_json(value))
    return value


def _run(name: str, command: list[str], cwd: Path, logs: Path) -> dict[str, Any]:
    proc = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
    )
    (logs / f"{name}.stdout.txt").write_text(proc.stdout, encoding="utf-8")
    (logs / f"{name}.stderr.txt").write_text(proc.stderr, encoding="utf-8")
    result = {
        "name": name,
        "command": command,
        "returncode": proc.returncode,
        "stdout_sha256": SB.sha256_bytes(proc.stdout.encode("utf-8")),
        "stderr_sha256": SB.sha256_bytes(proc.stderr.encode("utf-8")),
    }
    if proc.returncode != 0:
        tail = "\n".join((proc.stderr or proc.stdout).splitlines()[-30:])
        raise LR.ResolutionError(f"static generator {name} failed ({proc.returncode}):\n{tail}")
    return result


def runtime_specs(root: Path) -> tuple[SB.ArtifactSpec, ...]:
    return (
        SB.ArtifactSpec(
            "templates_equipment_catalog",
            root / "templates/templates_equipment_catalog_live_domain59/domain59_static_catalog.json",
            root / "templates/domain59_static_gate_report.json",
            "templates_equipment_catalog_live.static_artifact.v1",
            "templates_equipment_catalog.json",
        ),
        SB.ArtifactSpec(
            "localisation_join",
            root / "localisation/static_localisation_join_r3/localisation_catalog.json",
            root / "localisation/localisation_gate_report.json",
            "static_localisation_join_r3.static_artifact.v1",
            "localisation_join.json",
        ),
        SB.ArtifactSpec(
            "idea_tech_loc_xref_static",
            root / "idea_tech/idea_tech_loc_xref_static/idea_tech_loc_xref_catalog.json",
            root / "idea_tech/idea_tech_static_gate_report.json",
            "idea_tech_loc_xref_static.static_artifact.v1",
            "idea_tech_loc_xref_static.json",
            True,
        ),
        SB.ArtifactSpec(
            "decisions_static_definitions",
            root / "decisions/decisions_static_definitions/decision_definitions_catalog.json",
            root / "decisions/decisions_static_gate_report.json",
            "decisions_static_definitions.static_artifact.v1",
            "decisions_static_definitions.json",
            True,
        ),
        SB.ArtifactSpec(
            "ship_definitions_static",
            root / "ships/ship_definitions_static/ship_definitions_catalog.json",
            root / "ships/ship_definitions_gate_report.json",
            "ship_definitions_static.static_artifact.v1",
            "ship_definitions_static.json",
            True,
        ),
        SB.ArtifactSpec(
            "gfx_icon_asset_manifest_static",
            root / "gfx/gfx_icon_asset_manifest_static/gfx_icon_manifest_catalog.json",
            root / "gfx/gfx_icon_manifest_gate_report.json",
            "gfx_icon_asset_manifest_static.static_artifact.v1",
            "gfx_icon_asset_manifest_static.json",
        ),
        SB.ArtifactSpec(
            "technology_sharing_static",
            root / "technology_sharing/technology_sharing_static/technology_sharing_catalog.json",
            root / "technology_sharing/technology_sharing_gate_report.json",
            "technology_sharing_static.static_artifact.v1",
            "technology_sharing_static.json",
            True,
        ),
    )


def _technology_sharing_command(
    common: list[str], loc_catalog: Path, catalogs: Path
) -> list[str]:
    return [
        sys.executable,
        str(HERE / "technology_sharing_static_gate.py"),
        *common,
        "--localisation-catalog", str(loc_catalog),
        "--out-dir", str(catalogs / "technology_sharing"),
    ]


def build(
    *, bindings_dir: Path, vanilla: Path, output: Path, exe: Path, resume: bool = False
) -> dict[str, Any]:
    vanilla = vanilla.resolve()
    exe = exe.resolve()
    if not vanilla.is_dir() or not exe.is_file():
        raise LR.ResolutionError("vanilla root and executable must both exist")
    try:
        same_install = vanilla.samefile(exe.parent)
    except OSError as exc:
        raise LR.ResolutionError("cannot prove vanilla root/executable installation identity") from exc
    if not same_install:
        raise LR.ResolutionError("vanilla root must be the executable parent directory")
    if output.exists() and not resume:
        raise LR.ResolutionError(f"output already exists: {output}")
    if resume and not output.is_dir():
        raise LR.ResolutionError("--resume requires an existing build output directory")
    mod_state_for_inventory = SB.load_object(bindings_dir / "mod_state.json", "resolved mod state")
    SB._validate_binding_document(mod_state_for_inventory, "mod_state")
    resolved_sources = _load_resolved_mod_sources(bindings_dir, mod_state_for_inventory)
    recorded_source_inventory = LR.validate_source_inventory_document(
        SB.load_object(bindings_dir / "source_inventory.json", "live source inventory")
    )
    active_dlc_ids_for_inventory = [
        row["id"] for row in recorded_source_inventory["namespaces"]
        if row["kind"] == "dlc" and row["binding"] == "active_product"
    ]
    extra_asset_dlc_ids = [
        row["id"] for row in recorded_source_inventory["namespaces"]
        if row["kind"] == "dlc" and row["binding"] == "asset_only"
    ]
    recomputed_before = LR.recompute_source_inventory(
        vanilla,
        resolved_sources,
        active_dlc_ids_for_inventory,
        extra_asset_dlc_ids,
        recorded_source_inventory["identity"],
    )
    if recomputed_before != recorded_source_inventory:
        raise LR.ResolutionError("live source inventory changed before static generation")
    fingerprint = _build_input_fingerprint(
        bindings_dir, vanilla, exe, recorded_source_inventory
    )
    fingerprint_path = output / "build_input_fingerprint.json"
    if resume:
        existing_fingerprint = SB.load_object(fingerprint_path, "build input fingerprint")
        if existing_fingerprint != fingerprint:
            raise LR.ResolutionError("resume input fingerprint mismatch")
    else:
        output.mkdir(parents=True, exist_ok=False)
        fingerprint_path.write_bytes(SB.pretty_json(fingerprint))
    logs = output / "logs"
    logs.mkdir(exist_ok=resume)
    staging_userdir = output / "runtime_userdir"
    if staging_userdir.exists():
        launcher = _validate_runtime_userdir(bindings_dir, vanilla, staging_userdir)
    else:
        launcher = prepare_runtime_userdir(bindings_dir, vanilla, staging_userdir)
    explicit_mods = list(launcher.get("enabled_mods", []))
    mod_arg = ",".join(explicit_mods)
    catalogs = output / "catalogs"
    catalogs.mkdir(exist_ok=resume)
    loc_out = catalogs / "localisation"
    localisation_command = [
        sys.executable,
        str(HERE / "localisation_gate.py"),
        "--vanilla", str(vanilla),
        "--userdata", str(staging_userdir),
        "--mods", mod_arg,
        "--strict-mods",
        "--source-inventory", str(bindings_dir / "source_inventory.json"),
        "--out-dir", str(loc_out),
    ]
    results = []
    loc_report = loc_out / "localisation_gate_report.json"
    loc_passed = loc_report.is_file() and SB.load_object(loc_report, "localisation report").get("passed") is True
    if not loc_passed:
        results.append(_run("localisation", localisation_command, REPO_ROOT, logs))
    template_command = [
        sys.executable,
        str(TEMPLATES_GATE),
        "--vanilla-root", str(vanilla),
        "--source-inventory", str(bindings_dir / "source_inventory.json"),
        "--out-dir", str(catalogs / "templates"),
    ]
    if explicit_mods:
        template_command.extend(
            ["--mods", mod_arg, "--userdata", str(staging_userdir), "--strict-mods"]
        )
    else:
        template_command.append("--vanilla-only")
    commands = [("templates", template_command)]
    loc_catalog = loc_out / "static_localisation_join_r3/localisation_catalog.json"
    common = [
        "--vanilla", str(vanilla),
        "--userdata", str(staging_userdir),
        "--mods", mod_arg,
        "--strict-mods",
        "--source-inventory", str(bindings_dir / "source_inventory.json"),
    ]
    gfx_allowlist = None
    failed_gfx_report = catalogs / "gfx/gfx_icon_manifest_gate_report.json"
    if failed_gfx_report.is_file():
        failed = SB.load_object(failed_gfx_report, "failed runtime GFX report")
        counts = failed.get("counts") if isinstance(failed.get("counts"), dict) else {}
        allowed = failed.get("allowed_missing_textures")
        if (
            failed.get("passed") is False
            and counts.get("unresolved_textures") == 0
            and isinstance(allowed, list)
            and allowed
        ):
            base = SB.load_object(
                HERE / "gfx_icon_missing_texture_allowlist.json",
                "canonical GFX missing-texture allowlist",
            )
            gfx_allowlist = output / "runtime_gfx_missing_texture_allowlist.json"
            gfx_allowlist.write_bytes(
                SB.pretty_json({"schema": base.get("schema"), "entries": allowed})
            )
    commands.extend(
        [
            (
                "idea_tech",
                [sys.executable, str(HERE / "idea_tech_static_gate.py"), *common,
                 "--localisation-catalog", str(loc_catalog), "--out-dir", str(catalogs / "idea_tech")],
            ),
            (
                "decisions",
                [sys.executable, str(HERE / "decisions_static_gate.py"), *common,
                 "--localisation-catalog", str(loc_catalog), "--out-dir", str(catalogs / "decisions")],
            ),
            (
                "ships",
                [sys.executable, str(HERE / "ship_definitions_gate.py"), *common,
                 "--localisation-catalog", str(loc_catalog), "--out-dir", str(catalogs / "ships")],
            ),
            (
                "gfx",
                [sys.executable, str(HERE / "gfx_icon_manifest_gate.py"), *common,
                 "--out-dir", str(catalogs / "gfx")]
                + (["--missing-texture-allowlist", str(gfx_allowlist)] if gfx_allowlist else []),
            ),
            (
                "technology_sharing",
                _technology_sharing_command(
                    common,
                    loc_catalog,
                    catalogs,
                ),
            ),
        ]
    )
    report_by_name = {
        "templates": catalogs / "templates/domain59_static_gate_report.json",
        "idea_tech": catalogs / "idea_tech/idea_tech_static_gate_report.json",
        "decisions": catalogs / "decisions/decisions_static_gate_report.json",
        "ships": catalogs / "ships/ship_definitions_gate_report.json",
        "gfx": catalogs / "gfx/gfx_icon_manifest_gate_report.json",
        "technology_sharing": catalogs / "technology_sharing/technology_sharing_gate_report.json",
    }
    commands = [
        (name, command)
        for name, command in commands
        if not (
            report_by_name[name].is_file()
            and SB.load_object(report_by_name[name], f"{name} report").get("passed") is True
        )
    ]
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, len(commands))) as pool:
        futures = {
            pool.submit(_run, name, command, REPO_ROOT, logs): name
            for name, command in commands
        }
        for future in concurrent.futures.as_completed(futures):
            results.append(future.result())
    results.sort(key=lambda row: row["name"])

    _validate_runtime_userdir(bindings_dir, vanilla, staging_userdir)
    recomputed_after = LR.recompute_source_inventory(
        vanilla,
        resolved_sources,
        active_dlc_ids_for_inventory,
        extra_asset_dlc_ids,
        recorded_source_inventory["identity"],
    )
    if recomputed_after != recorded_source_inventory:
        raise LR.ResolutionError("live source inventory changed during static generation")

    specs = runtime_specs(catalogs)
    validated = {spec.role: SB.validate_artifact(spec) for spec in specs}
    resolution = SB.load_object(bindings_dir / "resolution_report.json", "live resolution report")
    dlc_state = SB.load_object(bindings_dir / "dlc_state.json", "resolved DLC state")
    active_dlc_ids = [row["id"] for row in dlc_state["dlc"] if row["enabled"]]
    allowed_extra_asset_layers = {
        row["id"] for row in resolution.get("descriptorless_asset_layers", [])
    }
    mod_state = SB.load_object(bindings_dir / "mod_state.json", "resolved mod state")
    SB._validate_binding_document(mod_state, "mod_state")
    mod_sources = _load_resolved_mod_sources(bindings_dir, mod_state)
    active_mod_names = [row["name"] for row in mod_sources]
    blockers = LR.catalog_compatibility(
        validated, active_mod_names, active_dlc_ids, allowed_extra_asset_layers
    )
    if blockers:
        raise LR.ResolutionError("rebuilt catalog compatibility failed: " + "; ".join(blockers))
    deferred = output / "deferred_token_join.json"
    deferred.write_bytes(
        SB.pretty_json(
            {
                "schema": SB.DEFER_PROVENANCE_SCHEMA,
                "requested_by": "extractor-static-runtime-builder",
                "reason": "static bundle built before emitted-token inventory exists",
                "session_plan_id": resolution["binding_identity"]["profile_id"],
            }
        )
    )
    bundle = output / "bundle"
    SB.create_bundle(
        bundle,
        exe=exe,
        mod_state=bindings_dir / "mod_state.json",
        dlc_state=bindings_dir / "dlc_state.json",
        load_order=bindings_dir / "load_order.json",
        source_inventory=bindings_dir / "source_inventory.json",
        live_resolution_report=bindings_dir / "resolution_report.json",
        specs=specs,
        defer_token_gate=True,
        defer_provenance=deferred,
    )
    verified = SB.verify_bundle(
        bundle,
        exe=exe,
        mod_state=bindings_dir / "mod_state.json",
        dlc_state=bindings_dir / "dlc_state.json",
        load_order=bindings_dir / "load_order.json",
        source_inventory=bindings_dir / "source_inventory.json",
        live_resolution_report=bindings_dir / "resolution_report.json",
        require_token_gate=False,
    )
    report = {
        "schema": BUILD_SCHEMA,
        "passed": True,
        "runtime_launcher_state": launcher,
        "preserved_launcher_conflicts": resolution.get("launcher_dlc_conflicts", []),
        "generators": results,
        "bundle": verified,
        "bundle_path": str(bundle),
    }
    (output / "build_report.json").write_bytes(SB.pretty_json(report))
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bindings-dir", type=Path, required=True)
    parser.add_argument("--vanilla", type=Path, required=True)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args(argv)
    try:
        report = build(
            bindings_dir=args.bindings_dir,
            vanilla=args.vanilla,
            output=args.out,
            exe=args.exe,
            resume=args.resume,
        )
    except (LR.ResolutionError, SB.BundleError, OSError, json.JSONDecodeError) as exc:
        print(f"runtime static catalog build: FAIL: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
