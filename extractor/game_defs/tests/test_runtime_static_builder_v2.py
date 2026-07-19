from __future__ import annotations

import json
import sys
import zipfile
from pathlib import Path

import pytest


HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import build_runtime_static_bundle as B  # noqa: E402
import build_all as BUILD_ALL  # noqa: E402
import live_binding_resolver as LR  # noqa: E402
import source_inventory_layers as SIL  # noqa: E402
import static_bundle as SB  # noqa: E402


def test_all_static_entrypoints_treat_empty_explicit_modset_as_vanilla_only():
    assert BUILD_ALL.mods_from_arg("") == []


def test_definition_layers_ignores_generated_dlc_staging_as_active_mod(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    vanilla = tmp_path / "game"
    vanilla.mkdir()
    inventory = {
        "inventory_sha256": "1" * 64,
        "namespaces": [
            {
                "binding": "base",
                "kind": "base",
                "id": "base",
                "label": "vanilla",
                "replace_paths": [],
            }
        ],
    }
    monkeypatch.setattr(SIL.LR, "validate_source_inventory_document", lambda raw: raw)
    inventory_path = tmp_path / "source_inventory.json"
    inventory_path.write_text(json.dumps(inventory), encoding="utf-8")
    base = SIL.R._ex.Layer("vanilla", vanilla, [], {"kind": "base"})
    staged_dlc = SIL.R._ex.Layer(
        "dlc003_rocket_launcher_unit_pack",
        tmp_path / "runtime_userdir/_materialized_dlc/0002",
        [],
        {"kind": "mod", "generated_dlc_staging": True},
    )

    selected, proof = SIL.definition_layers([base, staged_dlc], inventory_path)

    assert selected == [base]
    assert proof["selected_namespace_labels"] == ["vanilla"]


def _write_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(SB.pretty_json(value))


def _write_mod_source(
    root: Path,
    *,
    token: str,
    name: str,
    content_ref: str,
    payload: str,
    archive: bool = False,
) -> tuple[dict, dict]:
    descriptor = root / token
    descriptor.parent.mkdir(parents=True, exist_ok=True)
    content = root / content_ref
    if archive:
        descriptor.write_text(f'name="{name}"\narchive="{content_ref}"\n', encoding="utf-8")
        content.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(content, "w", zipfile.ZIP_DEFLATED) as zf:
            zf.writestr("common/fixture.txt", payload.encode("utf-8"))
        content_sha = SB.sha256_file(content)
    else:
        descriptor.write_text(f'name="{name}"\npath="{content_ref}"\n', encoding="utf-8")
        (content / "common").mkdir(parents=True)
        (content / "common" / "fixture.txt").write_text(payload, encoding="utf-8")
        content_sha, _ = LR.hash_tree(content, layer_kind="mod", layer_id=token)
    descriptor_sha = SB.sha256_file(descriptor)
    binding = {
        "id": token,
        "name": name,
        "descriptor_sha256": descriptor_sha,
        "content_sha256": content_sha,
        "enabled": True,
    }
    source = {
        "id": token,
        "ordinal": 0,
        "name": name,
        "descriptor_path": str(descriptor),
        "descriptor_sha256": descriptor_sha,
        "content_kind": "archive" if archive else "directory",
        "content_path": str(content),
        "content_sha256": content_sha,
    }
    return binding, source


def _binding_fixture(
    tmp_path: Path, *, two_mods: bool = False, archive_mod: bool = False
):
    userdir = tmp_path / "source_userdir"
    specs = [
        ("mod/z-last.mod", "Zed Mod", "mods/zed", "zed"),
    ]
    if two_mods:
        specs.append(("mod/a-first.mod", "Alpha Mod", "mods/alpha", "alpha"))
    pairs = [
        _write_mod_source(
            userdir,
            token=token,
            name=name,
            content_ref=(content_ref + ".zip") if archive_mod and index == 0 else content_ref,
            payload=payload,
            archive=archive_mod and index == 0,
        )
        for index, (token, name, content_ref, payload) in enumerate(specs)
    ]
    bindings = [pair[0] for pair in pairs]
    sources = [{**pair[1], "ordinal": ordinal} for ordinal, pair in enumerate(pairs)]
    identity = {"playset_id": "fixture-playset", "profile_id": "fixture-profile"}
    mod_state = {
        "schema": SB.MOD_STATE_SCHEMA,
        "identity": identity,
        "mods": sorted(bindings, key=lambda row: row["id"]),
        "effective_content_sha256": SB._content_digest(
            sorted(bindings, key=lambda row: row["id"])
        ),
    }
    dlc_rows: list[dict] = []
    dlc_state = {
        "schema": SB.DLC_STATE_SCHEMA,
        "identity": identity,
        "dlc": dlc_rows,
        "effective_content_sha256": SB._content_digest(dlc_rows),
    }
    bindings_dir = tmp_path / "bindings"
    _write_json(bindings_dir / "mod_state.json", mod_state)
    _write_json(bindings_dir / "dlc_state.json", dlc_state)
    load_rows = [{
        "id": "vanilla",
        "kind": "vanilla",
        "ordinal": 0,
        "content_sha256": SB.sha256_bytes(b"vanilla"),
    }]
    for ordinal, source in enumerate(sources, start=1):
        load_rows.append({
            "id": source["id"],
            "kind": "mod",
            "ordinal": ordinal,
            "content_sha256": source["content_sha256"],
        })
    _write_json(
        bindings_dir / "load_order.json",
        {
            "schema": SB.LOAD_ORDER_SCHEMA,
            "identity": identity,
            "entries": load_rows,
            "effective_content_sha256": SB._content_digest(load_rows),
        },
    )
    _write_json(bindings_dir / "resolution_report.json", {"resolved_mod_sources": sources})
    winner_rows = []
    if len(sources) > 1:
        winner_path = Path(sources[-1]["content_path"]) / "common/fixture.txt"
        winner_rows.append({
            "relpath": "common/fixture.txt",
            "winner_id": sources[-1]["id"],
            "sha256": SB.sha256_file(winner_path),
        })
    _write_json(
        bindings_dir / B.UI14_ENGINE_VFS_FILE,
        {
            "schema": LR.UI14_ENGINE_VFS_SCHEMA,
            "executable_sha256": LR.EXPECTED_EXE_SHA256,
            "layers": [
                {
                    "id": source["id"],
                    "descriptor_sha256": source["descriptor_sha256"],
                    "engine_descriptor_ordinal": ordinal * 15,
                    "engine_mount_ordinal": ordinal,
                }
                for ordinal, source in enumerate(sources)
            ],
            "effective_winners": winner_rows,
        },
    )
    return bindings_dir, mod_state, sources


def test_descriptor_rejects_both_directory_and_archive_content_fields(tmp_path: Path):
    descriptor = tmp_path / "ambiguous.mod"
    descriptor.write_text(
        'name="Ambiguous"\npath="mods/ambiguous"\narchive="mods/ambiguous.zip"\n',
        encoding="utf-8",
    )

    with pytest.raises(LR.ResolutionError, match=r"exactly one path/archive"):
        LR.parse_descriptor(descriptor)


@pytest.mark.parametrize("field", ["name", "path", "archive"])
def test_descriptor_rejects_duplicate_identity_or_content_field(tmp_path: Path, field: str):
    values = {
        "name": ['name="One"', 'name="Two"', 'path="mods/one"'],
        "path": ['name="One"', 'path="mods/one"', 'path="mods/two"'],
        "archive": ['name="One"', 'archive="mods/one.zip"', 'archive="mods/two.zip"'],
    }
    descriptor = tmp_path / f"duplicate-{field}.mod"
    descriptor.write_text("\n".join(values[field]) + "\n", encoding="utf-8")

    with pytest.raises(LR.ResolutionError, match=rf"duplicate {field}"):
        LR.parse_descriptor(descriptor)


def test_resolved_mod_sources_retain_recorded_engine_order_not_binding_sort_order(tmp_path: Path):
    bindings_dir, mod_state, expected = _binding_fixture(tmp_path, two_mods=True)

    loaded = B._load_resolved_mod_sources(bindings_dir, mod_state)

    assert [row["id"] for row in loaded] == [row["id"] for row in expected]
    assert [row["ordinal"] for row in loaded] == [0, 1]


def test_runtime_staging_and_resume_reject_unattested_archive_mod_before_writes(
    tmp_path: Path,
):
    bindings_dir, mod_state, sources = _binding_fixture(tmp_path, archive_mod=True)
    vanilla = tmp_path / "vanilla"
    (vanilla / "common").mkdir(parents=True)
    (vanilla / "common/base.txt").write_text("vanilla", encoding="utf-8")
    _write_json(
        bindings_dir / "source_inventory.json",
        LR.recompute_source_inventory(vanilla, sources, [], [], mod_state["identity"]),
    )
    destination = tmp_path / "runtime_userdir"
    error = r"unattested archive mod is not admitted without a proven native-collision attestation"

    with pytest.raises(LR.ResolutionError, match=error):
        B.prepare_runtime_userdir(bindings_dir, vanilla, destination)
    assert not destination.exists()

    destination.mkdir()
    with pytest.raises(LR.ResolutionError, match=error):
        B._validate_runtime_userdir(bindings_dir, vanilla, destination)


@pytest.mark.parametrize(
    "token",
    [
        "../escape.mod",
        "mod/../../escape.mod",
        "mod/not-a-descriptor.txt",
        "mod/ambiguous,second.mod",
        "",
    ],
)
def test_staging_rejects_unsafe_or_ambiguous_launcher_tokens(tmp_path: Path, token: str):
    with pytest.raises(LR.ResolutionError, match=r"unsafe or ambiguous launcher mod token"):
        B._safe_launcher_token(token, tmp_path / "runtime_userdir")


def test_resolved_mod_source_rejects_descriptor_content_root_mismatch(tmp_path: Path):
    bindings_dir, mod_state, sources = _binding_fixture(tmp_path)
    wrong_root = tmp_path / "wrong_content_root"
    (wrong_root / "common").mkdir(parents=True)
    (wrong_root / "common" / "fixture.txt").write_text("zed", encoding="utf-8")
    wrong_sha, _ = LR.hash_tree(
        wrong_root, layer_kind="mod", layer_id=sources[0]["id"]
    )
    sources[0]["content_path"] = str(wrong_root)
    sources[0]["content_sha256"] = wrong_sha
    mod_state["mods"][0]["content_sha256"] = wrong_sha
    mod_state["effective_content_sha256"] = SB._content_digest(mod_state["mods"])
    _write_json(bindings_dir / "resolution_report.json", {"resolved_mod_sources": sources})

    with pytest.raises(LR.ResolutionError, match=r"content path is ambiguous"):
        B._load_resolved_mod_sources(bindings_dir, mod_state)


def test_input_fingerprint_binds_the_resolved_vanilla_root(tmp_path: Path, monkeypatch):
    bindings = tmp_path / "bindings"
    for name in (
        "mod_state.json",
        "dlc_state.json",
        "load_order.json",
        "resolution_report.json",
        "source_inventory.json",
        B.UI14_ENGINE_VFS_FILE,
    ):
        path = bindings / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(name, encoding="utf-8")
    executable = tmp_path / "hoi4.exe"
    executable.write_bytes(b"fixture executable")
    vanilla_a = tmp_path / "vanilla-a"
    vanilla_b = tmp_path / "vanilla-b"
    vanilla_a.mkdir()
    vanilla_b.mkdir()
    monkeypatch.setattr(
        B,
        "_implementation_inventory",
        lambda: [{"path": "fixture.py", "sha256": SB.sha256_bytes(b"fixture")}],
    )
    source_inventory = {"inventory_sha256": SB.sha256_bytes(b"source inventory")}

    first = B._build_input_fingerprint(bindings, vanilla_a, executable, source_inventory)
    second = B._build_input_fingerprint(bindings, vanilla_b, executable, source_inventory)

    assert first["vanilla_root"] == str(vanilla_a.resolve())
    assert first["fingerprint_sha256"] == SB.sha256_bytes(
        SB.canonical_json({key: value for key, value in first.items() if key != "fingerprint_sha256"})
    )
    assert first["fingerprint_sha256"] != second["fingerprint_sha256"]


def test_runtime_userdir_validation_detects_staged_descriptor_tampering(tmp_path: Path):
    bindings_dir, mod_state, sources = _binding_fixture(tmp_path)
    vanilla = tmp_path / "vanilla"
    (vanilla / "common").mkdir(parents=True)
    (vanilla / "common/base.txt").write_text("vanilla", encoding="utf-8")
    _write_json(
        bindings_dir / "source_inventory.json",
        LR.recompute_source_inventory(
            vanilla, sources, [], [], mod_state["identity"]
        ),
    )
    destination = tmp_path / "runtime_userdir"

    expected_launcher = B.prepare_runtime_userdir(bindings_dir, vanilla, destination)
    assert B._validate_runtime_userdir(bindings_dir, vanilla, destination) == expected_launcher

    manifest = json.loads(
        (destination / "static_mod_staging_manifest.json").read_text(encoding="utf-8")
    )
    assert manifest["mods"][0]["content_sha256"] == manifest["mods"][0][
        "source_namespace_sha256"
    ]
    handoff = json.loads(
        (destination / "renderer_game_file_handoff.json").read_text(encoding="utf-8")
    )
    assert handoff["schema"] == B.RENDERER_HANDOFF_SCHEMA
    assert handoff["game_dir"] == str(vanilla.resolve())
    assert handoff["userdata"] == str(destination.resolve())
    assert handoff["use_dlc_load"] is True
    assert handoff["cli_args"] == [
        "--game", str(vanilla.resolve()),
        "--use-dlc-load", "--userdata", str(destination.resolve()),
    ]
    staged_descriptor = destination / manifest["mods"][0]["descriptor_relpath"]
    staged_descriptor.write_text(
        staged_descriptor.read_text(encoding="utf-8") + "# tampered\n",
        encoding="utf-8",
    )

    with pytest.raises(LR.ResolutionError, match=r"runtime (staged descriptor|UI14 engine VFS receipt) changed"):
        B._validate_runtime_userdir(bindings_dir, vanilla, destination)


def test_descriptor_backed_dlc_archive_is_materialized_and_handed_to_renderer(
    tmp_path: Path,
):
    vanilla = tmp_path / "vanilla"
    (vanilla / "common").mkdir(parents=True)
    (vanilla / "common/base.txt").write_text("vanilla", encoding="utf-8")
    dlc_root = vanilla / "dlc/dlc003"
    dlc_root.mkdir(parents=True)
    descriptor = dlc_root / "dlc003.dlc"
    descriptor.write_text(
        'name="DLC Pack"\n'
        'archive="dlc/dlc003/dlc003.zip"\n'
        'replace_path="gfx/models"\n',
        encoding="utf-8",
    )
    archive = dlc_root / "dlc003.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("gfx/models/tank.asset", b"tank")
        zf.writestr("common/units/dlc.txt", b"dlc")

    identity = {"playset_id": "fixture-playset", "profile_id": "fixture-profile"}
    mod_state = {
        "schema": SB.MOD_STATE_SCHEMA,
        "identity": identity,
        "mods": [],
        "effective_content_sha256": SB._content_digest([]),
    }
    binding_sha, _ = LR.hash_tree(
        dlc_root, layer_kind="dlc", layer_id="dlc003", allow_empty=True
    )
    dlc_rows = [{
        "id": "dlc003",
        "name": "DLC Pack",
        "content_sha256": binding_sha,
        "enabled": True,
    }]
    dlc_state = {
        "schema": SB.DLC_STATE_SCHEMA,
        "identity": identity,
        "dlc": dlc_rows,
        "effective_content_sha256": SB._content_digest(dlc_rows),
    }
    load_rows = [
        {
            "id": "vanilla",
            "kind": "vanilla",
            "ordinal": 0,
            "content_sha256": SB.sha256_bytes(b"vanilla"),
        },
        {
            "id": "dlc003",
            "kind": "dlc",
            "ordinal": 1,
            "content_sha256": binding_sha,
        },
    ]
    bindings_dir = tmp_path / "bindings"
    _write_json(bindings_dir / "mod_state.json", mod_state)
    _write_json(bindings_dir / "dlc_state.json", dlc_state)
    _write_json(
        bindings_dir / "load_order.json",
        {
            "schema": SB.LOAD_ORDER_SCHEMA,
            "identity": identity,
            "entries": load_rows,
            "effective_content_sha256": SB._content_digest(load_rows),
        },
    )
    _write_json(bindings_dir / "resolution_report.json", {"resolved_mod_sources": []})
    _write_json(
        bindings_dir / B.UI14_ENGINE_VFS_FILE,
        {
            "schema": LR.UI14_ENGINE_VFS_SCHEMA,
            "executable_sha256": LR.EXPECTED_EXE_SHA256,
            "layers": [],
            "effective_winners": [],
        },
    )
    source_inventory = LR.recompute_source_inventory(
        vanilla, [], ["dlc003"], [], identity
    )
    _write_json(bindings_dir / "source_inventory.json", source_inventory)

    destination = tmp_path / "runtime_userdir"
    launcher = B.prepare_runtime_userdir(bindings_dir, vanilla, destination)
    assert launcher["enabled_mods"] == ["_ui15_dlc/0000-dlc003.mod"]
    assert B._validate_runtime_userdir(bindings_dir, vanilla, destination) == launcher

    manifest = json.loads(
        (destination / "static_mod_staging_manifest.json").read_text(encoding="utf-8")
    )
    row = manifest["dlc_archives"][0]
    source_namespace = next(
        item for item in source_inventory["namespaces"]
        if item["kind"] == "dlc" and item["id"] == "dlc003"
    )
    assert row["source_namespace_sha256"] == source_namespace["content_sha256"]
    assert row["content_sha256"] == source_namespace["content_sha256"]
    assert row["source_archive_sha256"] == SB.sha256_file(archive)
    staged_fields = LR.parse_descriptor(destination / row["descriptor_relpath"])
    assert "path" in staged_fields and "archive" not in staged_fields
    assert staged_fields["replace_path"] == ["gfx/models"]
    handoff = json.loads(
        (destination / "renderer_game_file_handoff.json").read_text(encoding="utf-8")
    )
    assert handoff["userdata"] == str(destination.resolve())
    assert handoff["cli_args"][-2:] == ["--userdata", str(destination.resolve())]

    staged_root = Path(row["content_path"])
    (staged_root / "common/units/dlc.txt").write_bytes(b"tampered")
    with pytest.raises(
        LR.ResolutionError, match=r"runtime staged DLC namespace changed"
    ):
        B._validate_runtime_userdir(bindings_dir, vanilla, destination)


def test_runtime_specs_include_canonical_technology_sharing_role(tmp_path: Path):
    specs = {spec.role: spec for spec in B.runtime_specs(tmp_path)}

    assert set(specs) == set(SB.EXPECTED_SPECS)
    technology_sharing = specs["technology_sharing_static"]
    assert technology_sharing.schema == "technology_sharing_static.static_artifact.v1"
    assert technology_sharing.bundle_name == "technology_sharing_static.json"
    assert technology_sharing.localisation_dependent is True
    assert technology_sharing.catalog == (
        tmp_path
        / "technology_sharing/technology_sharing_static/technology_sharing_catalog.json"
    )
    assert technology_sharing.gate_report == (
        tmp_path / "technology_sharing/technology_sharing_gate_report.json"
    )


def test_technology_sharing_command_binds_source_inventory_and_resume_report(tmp_path: Path):
    catalogs = tmp_path / "catalogs"
    source_inventory = tmp_path / "bindings/source_inventory.json"
    localisation = catalogs / "localisation/localisation_catalog.json"
    common = [
        "--vanilla", "fixture-vanilla", "--mods", "", "--strict-mods",
        "--source-inventory", str(source_inventory),
    ]

    command = B._technology_sharing_command(common, localisation, catalogs)

    assert command[0] == sys.executable
    assert command[1] == str(B.HERE / "technology_sharing_static_gate.py")
    assert command[2 : 2 + len(common)] == common
    assert command[-4:] == [
        "--localisation-catalog", str(localisation),
        "--out-dir", str(catalogs / "technology_sharing"),
    ]
    spec = next(
        spec for spec in B.runtime_specs(catalogs) if spec.role == "technology_sharing_static"
    )
    assert spec.gate_report == (
        catalogs / "technology_sharing/technology_sharing_gate_report.json"
    )
