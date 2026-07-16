from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest


HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import build_runtime_static_bundle as B  # noqa: E402
import build_all as BUILD_ALL  # noqa: E402
import live_binding_resolver as LR  # noqa: E402
import static_bundle as SB  # noqa: E402


def test_all_static_entrypoints_treat_empty_explicit_modset_as_vanilla_only():
    assert BUILD_ALL.mods_from_arg("") == []


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
) -> tuple[dict, dict]:
    descriptor = root / token
    descriptor.parent.mkdir(parents=True, exist_ok=True)
    descriptor.write_text(f'name="{name}"\npath="{content_ref}"\n', encoding="utf-8")
    content = root / content_ref
    (content / "common").mkdir(parents=True)
    (content / "common" / "fixture.txt").write_text(payload, encoding="utf-8")
    descriptor_sha = SB.sha256_file(descriptor)
    content_sha, _ = LR.hash_tree(content, layer_kind="mod", layer_id=token)
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
        "content_kind": "directory",
        "content_path": str(content),
        "content_sha256": content_sha,
    }
    return binding, source


def _binding_fixture(tmp_path: Path, *, two_mods: bool = False):
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
            content_ref=content_ref,
            payload=payload,
        )
        for token, name, content_ref, payload in specs
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


def test_resolved_mod_sources_retain_launcher_order_not_binding_sort_order(tmp_path: Path):
    bindings_dir, mod_state, expected = _binding_fixture(tmp_path, two_mods=True)

    loaded = B._load_resolved_mod_sources(bindings_dir, mod_state)

    assert [row["id"] for row in loaded] == [row["id"] for row in expected]
    assert [row["ordinal"] for row in loaded] == [0, 1]


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
    bindings_dir, _mod_state, _sources = _binding_fixture(tmp_path)
    vanilla = tmp_path / "vanilla"
    vanilla.mkdir()
    destination = tmp_path / "runtime_userdir"

    expected_launcher = B.prepare_runtime_userdir(bindings_dir, vanilla, destination)
    assert B._validate_runtime_userdir(bindings_dir, vanilla, destination) == expected_launcher

    manifest = json.loads(
        (destination / "static_mod_staging_manifest.json").read_text(encoding="utf-8")
    )
    staged_descriptor = destination / manifest["mods"][0]["descriptor_relpath"]
    staged_descriptor.write_text(
        staged_descriptor.read_text(encoding="utf-8") + "# tampered\n",
        encoding="utf-8",
    )

    with pytest.raises(LR.ResolutionError, match=r"runtime staged descriptor changed"):
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
