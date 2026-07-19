from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest


HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import static_bundle as SB  # noqa: E402


_REAL_CREATE_BUNDLE = SB.create_bundle
_REAL_VERIFY_BUNDLE = SB.verify_bundle


def _v2_create_bundle(out_dir: Path, *args, **kwargs):
    kwargs.setdefault("source_inventory", Path(out_dir).parent / "source_inventory.json")
    kwargs.setdefault("require_current_sources", False)
    return _REAL_CREATE_BUNDLE(out_dir, *args, **kwargs)


def _v2_verify_bundle(bundle_dir: Path, *args, **kwargs):
    kwargs.setdefault("source_inventory", Path(bundle_dir).parent / "source_inventory.json")
    kwargs.setdefault("require_current_sources", False)
    kwargs.setdefault("require_token_gate", False)
    return _REAL_VERIFY_BUNDLE(bundle_dir, *args, **kwargs)


SB.create_bundle = _v2_create_bundle
SB.verify_bundle = _v2_verify_bundle


def _write_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(SB.pretty_json(value))


def _binding_document(role: str, playset="playset-1", profile="profile-1"):
    content = SB.sha256_bytes(f"{role}-content".encode())
    if role == "mod_state":
        rows = [
            {
                "id": "mod-1",
                "name": "Fixture Mod",
                "descriptor_sha256": SB.sha256_bytes(b"descriptor"),
                "content_sha256": content,
                "enabled": True,
            }
        ]
        return {
            "schema": SB.MOD_STATE_SCHEMA,
            "identity": {"playset_id": playset, "profile_id": profile},
            "mods": rows,
            "effective_content_sha256": SB._content_digest(rows),
        }
    if role == "dlc_state":
        rows = [{"id": "dlc-1", "name": "Fixture DLC", "content_sha256": content, "enabled": True}]
        return {
            "schema": SB.DLC_STATE_SCHEMA,
            "identity": {"playset_id": playset, "profile_id": profile},
            "dlc": rows,
            "effective_content_sha256": SB._content_digest(rows),
        }
    rows = [
        {"id": "vanilla", "kind": "vanilla", "ordinal": 0, "content_sha256": content},
        {
            "id": "dlc-1",
            "kind": "dlc",
            "ordinal": 1,
            "content_sha256": SB.sha256_bytes(b"dlc_state-content"),
        },
        {
            "id": "mod-1",
            "kind": "mod",
            "ordinal": 2,
            "content_sha256": SB.sha256_bytes(b"mod_state-content"),
        },
    ]
    return {
        "schema": SB.LOAD_ORDER_SCHEMA,
        "identity": {"playset_id": playset, "profile_id": profile},
        "entries": rows,
        "effective_content_sha256": SB._content_digest(rows),
    }


def _write_emitted(path: Path, tokens: list[dict], *, provenance=True):
    value = {"schema": SB.EMITTED_INVENTORY_SCHEMA, "tokens": tokens}
    if provenance:
        value["provenance"] = {
            "capture_id": "capture-1",
            "session_id": "session-1",
            "stream_sha256": SB.sha256_bytes(b"stream"),
        }
    _write_json(path, value)


def _rewrite_manifest(bundle: Path, mutate):
    path = bundle / "static_bundle_manifest.json"
    manifest = json.loads(path.read_text(encoding="utf-8"))
    mutate(manifest)
    manifest.pop("bundle_id", None)
    manifest["bundle_id"] = SB.sha256_bytes(SB.canonical_json(manifest))
    _write_json(path, manifest)


def _artifact(
    root: Path,
    role: str,
    schema: str,
    body: dict,
    *,
    loc_sha: str | None = None,
    layers: list[dict] | None = None,
) -> SB.ArtifactSpec:
    catalog = {
        "schema": schema,
        "layers": layers or [{"label": "vanilla", "replace_path": []}],
        "source_files": {
            "rows": [
                {
                    "relpath": f"common/{role}.txt",
                    "bytes": 1,
                    "sha256": SB.sha256_bytes(role.encode("utf-8")),
                }
            ]
        },
        **body,
    }
    if loc_sha is not None:
        catalog["localisation_source"] = {"source": "static_localisation_join_r3", "sha256": loc_sha}
    catalog["artifact_sha256"] = SB.sha256_bytes(SB.canonical_json(catalog))
    store = root / role
    catalog_path = store / "catalog.json"
    _write_json(catalog_path, catalog)
    catalog_file_sha = SB.sha256_file(catalog_path)
    snapshot = {"catalog_file": catalog_path.name, "catalog_sha256": catalog_file_sha}
    snapshot_path = store / "snapshot_000000.json"
    _write_json(snapshot_path, snapshot)
    index = {
        "snapshots": [
            {"file": catalog_path.name, "sha256": catalog_file_sha},
            {"file": snapshot_path.name, "sha256": SB.sha256_file(snapshot_path)},
        ]
    }
    _write_json(store / "index.json", index)
    report_path = root / f"{role}.gate.json"
    _write_json(report_path, {"passed": True, "artifact_sha256": catalog["artifact_sha256"]})
    return SB.ArtifactSpec(role, catalog_path, report_path, schema, f"{role}.json", loc_sha is not None)


def _refresh_artifact(spec: SB.ArtifactSpec, catalog: dict) -> None:
    catalog.pop("artifact_sha256", None)
    catalog["artifact_sha256"] = SB.sha256_bytes(SB.canonical_json(catalog))
    _write_json(spec.catalog, catalog)
    snapshot_path = spec.catalog.parent / "snapshot_000000.json"
    snapshot = {
        "catalog_file": spec.catalog.name,
        "catalog_sha256": SB.sha256_file(spec.catalog),
    }
    _write_json(snapshot_path, snapshot)
    _write_json(
        spec.catalog.parent / "index.json",
        {
            "snapshots": [
                {"file": spec.catalog.name, "sha256": SB.sha256_file(spec.catalog)},
                {"file": snapshot_path.name, "sha256": SB.sha256_file(snapshot_path)},
            ]
        },
    )
    _write_json(
        spec.gate_report,
        {"passed": True, "artifact_sha256": catalog["artifact_sha256"]},
    )


def _fixture(tmp_path: Path):
    src = tmp_path / "source"
    loc = _artifact(
        src,
        "localisation_join",
        "static_localisation_join_r3.static_artifact.v1",
        {"keys": {"LOC_OK": {"name": "Okay"}}},
    )
    loc_sha = SB.sha256_file(loc.catalog)
    specs = [
        _artifact(
            src,
            "templates_equipment_catalog",
            "templates_equipment_catalog_live.static_artifact.v1",
            {
                "catalog": {
                    "division_templates": [{"name": "Template"}],
                    "equipment_definitions": {"rifle": {}},
                    "unit_definitions": {"infantry": {}},
                }
            },
        ),
        loc,
        _artifact(
            src,
            "idea_tech_loc_xref_static",
            "idea_tech_loc_xref_static.static_artifact.v1",
            {"tokens": {"ideas": {"idea_ok": {}}, "technologies": {"tech_ok": {}}}},
            loc_sha=loc_sha,
        ),
        _artifact(
            src,
            "decisions_static_definitions",
            "decisions_static_definitions.static_artifact.v1",
            {"decisions": {"decision_ok": {}}, "categories": {"category_ok": {}}},
            loc_sha=loc_sha,
        ),
        _artifact(
            src,
            "ship_definitions_static",
            "ship_definitions_static.static_artifact.v1",
            {"ship_hulls": {"hull_ok": {}}, "ship_modules": {"module_ok": {}}},
            loc_sha=loc_sha,
        ),
        _artifact(
            src,
            "gfx_icon_asset_manifest_static",
            "gfx_icon_asset_manifest_static.static_artifact.v1",
            {"sprites": {"GFX_ok": {}}},
        ),
        _artifact(
            src,
            "technology_sharing_static",
            "technology_sharing_static.static_artifact.v1",
            {
                "join_keys": {
                    "definition_token_candidate": "definitions[*].runtime_token_candidate",
                    "runtime_hash_candidate": None,
                    "compatibility_status": SB.TECH_SHARING_COMPATIBILITY_STATUS,
                },
                "source_namespace_proof": {
                    "schema": SB.TECH_SHARING_SOURCE_PROOF_SCHEMA,
                    "status": "bound_to_live_source_inventory",
                    "source_inventory_sha256": "0" * 64,
                    "selected_namespace_labels": ["vanilla", "dlc/dlc-1", "Fixture Mod"],
                    "excluded_asset_only_namespace_labels": [],
                },
                "definitions": [
                    {
                        "id": "fixture_research_group",
                        "runtime_token_candidate": "fixture_research_group",
                        "runtime_hash_candidate": None,
                        "runtime_join_status": SB.TECH_SHARING_UNJOINED_STATUS,
                    }
                ],
            },
            loc_sha=loc_sha,
            layers=[
                {"label": "vanilla", "replace_path": []},
                {"label": "dlc/dlc-1", "replace_path": []},
                {"label": "Fixture Mod", "replace_path": []},
            ],
        ),
    ]
    exe = tmp_path / "hoi4.exe"
    exe.write_bytes(b"test executable")
    bindings = []
    for name, role in (("mod", "mod_state"), ("dlc", "dlc_state"), ("load", "load_order")):
        path = tmp_path / f"{name}.json"
        _write_json(path, _binding_document(role))
        bindings.append(path)
    source_rows = []
    for spec in specs:
        catalog = json.loads(spec.catalog.read_text(encoding="utf-8"))
        source_rows.extend(SB._iter_source_rows(catalog["source_files"]))
    unique_rows = {
        row["relpath"].casefold(): {
            "namespace_ordinal": 0,
            "layer_kind": "vanilla",
            "layer_id": "vanilla",
            **row,
            "storage_kind": "directory",
            "source_container_sha256": None,
            "source_byte_offset": 0,
            "source_byte_count": row["bytes"],
            "source_range_sha256": row["sha256"],
        }
        for row in source_rows
    }
    rows = [unique_rows[key] for key in sorted(unique_rows)]
    content_rows = [
        {"relpath": row["relpath"], "bytes": row["bytes"], "sha256": row["sha256"]}
        for row in rows
    ]
    empty_content_sha = SB.sha256_bytes(SB.canonical_json([]))
    namespaces = [
        {
            "ordinal": 0,
            "kind": "vanilla",
            "binding": "base",
            "id": "vanilla",
            "label": "vanilla",
            "replace_paths": [],
            "storage_kind": "directory",
            "source_container_sha256": None,
            "source_container_bytes": None,
            "file_count": len(rows),
            "content_sha256": SB.sha256_bytes(SB.canonical_json(content_rows)),
            "binding_content_sha256": SB.sha256_bytes(b"load_order-content"),
        },
        {
            "ordinal": 1,
            "kind": "dlc",
            "binding": "active_product",
            "id": "dlc-1",
            "label": "dlc/dlc-1",
            "replace_paths": [],
            "storage_kind": "directory",
            "source_container_sha256": None,
            "source_container_bytes": None,
            "file_count": 0,
            "content_sha256": empty_content_sha,
            "binding_content_sha256": SB.sha256_bytes(b"dlc_state-content"),
        },
        {
            "ordinal": 2,
            "kind": "mod",
            "binding": "active_mod",
            "id": "mod-1",
            "label": "Fixture Mod",
            "replace_paths": [],
            "storage_kind": "directory",
            "source_container_sha256": None,
            "source_container_bytes": None,
            "file_count": 0,
            "content_sha256": empty_content_sha,
            "binding_content_sha256": SB.sha256_bytes(b"mod_state-content"),
        },
    ]
    core = {
        "schema": SB.SOURCE_INVENTORY_SCHEMA,
        "identity": {"playset_id": "playset-1", "profile_id": "profile-1"},
        "namespaces": namespaces,
        "rows": rows,
        "namespace_sha256": SB.sha256_bytes(SB.canonical_json(namespaces)),
        "rows_sha256": SB.sha256_bytes(SB.canonical_json(rows)),
    }
    source_inventory = {**core, "inventory_sha256": SB.sha256_bytes(SB.canonical_json(core))}
    _write_json(tmp_path / "source_inventory.json", source_inventory)
    for spec in specs:
        catalog = json.loads(spec.catalog.read_text(encoding="utf-8"))
        catalog["layers"] = [
            {"label": "vanilla", "replace_path": []},
            {"label": "dlc/dlc-1", "replace_path": []},
            {"label": "Fixture Mod", "replace_path": []},
        ]
        if spec.role in SB.DEFINITION_SOURCE_PROOF_ROLES:
            catalog["source_namespace_proof"] = {
                "schema": SB.TECH_SHARING_SOURCE_PROOF_SCHEMA,
                "status": "bound_to_live_source_inventory",
                "source_inventory_sha256": source_inventory["inventory_sha256"],
                "selected_namespace_labels": ["vanilla", "dlc/dlc-1", "Fixture Mod"],
                "excluded_asset_only_namespace_labels": [],
            }
        _refresh_artifact(spec, catalog)
    refreshed_loc_sha = SB.sha256_file(loc.catalog)
    for spec in specs:
        if not spec.localisation_dependent:
            continue
        catalog = json.loads(spec.catalog.read_text(encoding="utf-8"))
        catalog["localisation_source"]["sha256"] = refreshed_loc_sha
        _refresh_artifact(spec, catalog)
    return tuple(specs), exe, bindings


def test_create_and_verify_bound_bundle_with_closed_token_join(tmp_path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    emitted = tmp_path / "emitted.json"
    omissions = tmp_path / "omissions.json"
    _write_emitted(
        emitted,
        [
            {"kind": "idea", "token": "idea_ok"},
            {"kind": "decision", "token": "decision_ok"},
            {"kind": "technology", "token": "known_missing_but_accepted"},
        ],
    )
    _write_json(
        omissions,
        {
            "schema": SB.OMISSION_INVENTORY_SCHEMA,
            "tokens": [
                {
                    "kind": "technology",
                    "token": "known_missing_but_accepted",
                    "reason": "engine-only token",
                }
            ],
        },
    )
    bundle = tmp_path / "bundle"
    manifest = SB.create_bundle(
        bundle,
        exe=exe,
        mod_state=mod,
        dlc_state=dlc,
        load_order=load,
        specs=specs,
        emitted_tokens=emitted,
        omissions=omissions,
    )
    assert manifest["token_join"]["status"] == "passed"
    report = json.loads((bundle / "token_join_report.json").read_text(encoding="utf-8"))
    assert report["counts"] == {"resolved": 2, "explicitly_omitted": 1, "unknown": 0}
    assert SB.verify_bundle(
        bundle, exe=exe, mod_state=mod, dlc_state=dlc, load_order=load
    )["artifact_count"] == 7


def test_technology_sharing_intern_ordinal_is_never_static_joined(tmp_path):
    specs, _exe, _bindings = _fixture(tmp_path)
    catalogs = {
        spec.role: json.loads(spec.catalog.read_text(encoding="utf-8")) for spec in specs
    }
    emitted = tmp_path / "technology-sharing-emitted.json"
    _write_emitted(
        emitted,
        [
            {"kind": "technology_sharing_group", "token": "fixture_research_group"},
            {"kind": "technology_sharing_group", "token": "7"},
        ],
    )

    report = SB.account_emitted_tokens(catalogs, emitted)

    assert report["passed"] is False
    assert report["counts"] == {"resolved": 0, "explicitly_omitted": 0, "unknown": 2}
    assert all(row["disposition"] == "unknown" for row in report["results"])


@pytest.mark.parametrize(
    "mutate,error",
    [
        (
            lambda catalog: catalog["join_keys"].update(runtime_hash_candidate="deadbeef"),
            "runtime join metadata drifted",
        ),
        (
            lambda catalog: catalog["join_keys"].update(definition_token_candidate="wrong"),
            "runtime join metadata drifted",
        ),
        (
            lambda catalog: catalog["join_keys"].update(compatibility_status="hash it"),
            "runtime join metadata drifted",
        ),
        (
            lambda catalog: catalog["definitions"][0].update(runtime_token_candidate="other"),
            "unjoined intern ordinal",
        ),
        (
            lambda catalog: catalog["definitions"][0].update(runtime_hash_candidate="deadbeef"),
            "unjoined intern ordinal",
        ),
        (
            lambda catalog: catalog["definitions"][0].update(runtime_join_status="resolved"),
            "unjoined intern ordinal",
        ),
    ],
)
def test_technology_sharing_join_metadata_drift_is_rejected(mutate, error):
    catalog = {
        "layers": [{"label": "vanilla", "replace_path": []}],
        "join_keys": {
            "definition_token_candidate": "definitions[*].runtime_token_candidate",
            "runtime_hash_candidate": None,
            "compatibility_status": SB.TECH_SHARING_COMPATIBILITY_STATUS,
        },
        "definitions": [
            {
                "id": "fixture",
                "runtime_token_candidate": "fixture",
                "runtime_hash_candidate": None,
                "runtime_join_status": SB.TECH_SHARING_UNJOINED_STATUS,
            }
        ],
        "source_namespace_proof": {
            "schema": SB.TECH_SHARING_SOURCE_PROOF_SCHEMA,
            "status": "bound_to_live_source_inventory",
            "source_inventory_sha256": "0" * 64,
            "selected_namespace_labels": ["vanilla"],
            "excluded_asset_only_namespace_labels": [],
        },
    }
    mutate(catalog)

    with pytest.raises(SB.BundleError, match=error):
        SB._validate_unjoined_technology_sharing(catalog)


@pytest.mark.parametrize(
    "field,error",
    [
        ("source_inventory_sha256", "source namespace proof binding mismatch"),
        ("selected_namespace_labels", "source namespace selection is malformed"),
    ],
)
def test_technology_sharing_source_namespace_proof_drift_is_rejected(
    tmp_path, field, error
):
    specs, _exe, _bindings = _fixture(tmp_path)
    spec = next(spec for spec in specs if spec.role == "technology_sharing_static")
    catalog = json.loads(spec.catalog.read_text(encoding="utf-8"))
    inventory = SB.load_source_inventory(tmp_path / "source_inventory.json")
    if field == "source_inventory_sha256":
        catalog["source_namespace_proof"][field] = "f" * 64
    else:
        catalog["source_namespace_proof"][field] = ["vanilla"]

    with pytest.raises(SB.BundleError, match=error):
        SB._validate_unjoined_technology_sharing(catalog, inventory)


def test_unknown_token_fails_closed(tmp_path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    emitted = tmp_path / "emitted.json"
    _write_emitted(emitted, [{"kind": "idea", "token": "not_in_catalog"}])
    with pytest.raises(SB.BundleError, match="1 unknown tokens"):
        SB.create_bundle(
            tmp_path / "bundle",
            exe=exe,
            mod_state=mod,
            dlc_state=dlc,
            load_order=load,
            specs=specs,
            emitted_tokens=emitted,
        )


def test_missing_token_inventory_requires_explicit_defer(tmp_path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    with pytest.raises(SB.BundleError, match="emitted token inventory is required"):
        SB.create_bundle(
            tmp_path / "bundle",
            exe=exe,
            mod_state=mod,
            dlc_state=dlc,
            load_order=load,
            specs=specs,
        )


def test_catalog_corruption_and_binding_drift_fail_closed(tmp_path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    emitted = tmp_path / "emitted.json"
    _write_emitted(emitted, [{"kind": "idea", "token": "idea_ok"}])
    bundle = tmp_path / "bundle"
    SB.create_bundle(
        bundle,
        exe=exe,
        mod_state=mod,
        dlc_state=dlc,
        load_order=load,
        specs=specs,
        emitted_tokens=emitted,
    )
    changed = _binding_document("mod_state")
    changed["mods"][0]["content_sha256"] = SB.sha256_bytes(b"changed")
    changed["effective_content_sha256"] = SB._content_digest(changed["mods"])
    _write_json(mod, changed)
    with pytest.raises(SB.BundleError, match="load_order enabled mod/DLC"):
        SB.verify_bundle(bundle, exe=exe, mod_state=mod, dlc_state=dlc, load_order=load)

    artifact = bundle / "artifacts/decisions_static_definitions.json"
    artifact.write_bytes(artifact.read_bytes() + b" ")
    with pytest.raises(SB.BundleError, match="missing/corrupt bundled artifact"):
        SB.verify_bundle(bundle, exe=exe, mod_state=mod, dlc_state=dlc, load_order=load)


def test_localisation_source_must_match_exact_catalog_bytes(tmp_path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    bad = list(specs)
    dependent = bad[2]
    catalog = json.loads(dependent.catalog.read_text(encoding="utf-8"))
    catalog["localisation_source"]["sha256"] = "0" * 64
    catalog.pop("artifact_sha256")
    catalog["artifact_sha256"] = SB.sha256_bytes(SB.canonical_json(catalog))
    _write_json(dependent.catalog, catalog)
    snapshot = json.loads((dependent.catalog.parent / "snapshot_000000.json").read_text(encoding="utf-8"))
    snapshot["catalog_sha256"] = SB.sha256_file(dependent.catalog)
    _write_json(dependent.catalog.parent / "snapshot_000000.json", snapshot)
    _write_json(
        dependent.catalog.parent / "index.json",
        {
            "snapshots": [
                {"file": dependent.catalog.name, "sha256": SB.sha256_file(dependent.catalog)},
                {
                    "file": "snapshot_000000.json",
                    "sha256": SB.sha256_file(dependent.catalog.parent / "snapshot_000000.json"),
                },
            ]
        },
    )
    _write_json(dependent.gate_report, {"passed": True, "artifact_sha256": catalog["artifact_sha256"]})
    emitted = tmp_path / "emitted.json"
    _write_emitted(emitted, [{"kind": "idea", "token": "idea_ok"}])
    with pytest.raises(SB.BundleError, match="localisation source hash"):
        SB.create_bundle(
            tmp_path / "bundle",
            exe=exe,
            mod_state=mod,
            dlc_state=dlc,
            load_order=load,
            specs=bad,
            emitted_tokens=emitted,
        )


@pytest.mark.parametrize(
    ("binding_index", "mutation", "message"),
    [
        (0, lambda doc: doc.update(schema="anything.v0"), "mod_state: schema must be"),
        (
            1,
            lambda doc: doc.update(effective_content_sha256="0" * 64),
            "dlc_state: effective_content_sha256",
        ),
        (2, lambda doc: doc["entries"][0].update(content_sha256=None), "content_sha256 must be"),
    ],
)
def test_binding_schemas_and_content_fields_fail_closed(tmp_path, binding_index, mutation, message):
    specs, exe, bindings = _fixture(tmp_path)
    path = bindings[binding_index]
    document = json.loads(path.read_text(encoding="utf-8"))
    mutation(document)
    _write_json(path, document)
    emitted = tmp_path / "emitted.json"
    _write_emitted(emitted, [{"kind": "idea", "token": "idea_ok"}])
    with pytest.raises(SB.BundleError, match=message):
        SB.create_bundle(
            tmp_path / "bundle",
            exe=exe,
            mod_state=bindings[0],
            dlc_state=bindings[1],
            load_order=bindings[2],
            specs=specs,
            emitted_tokens=emitted,
        )


def test_binding_identities_must_match(tmp_path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    _write_json(load, _binding_document("load_order", playset="different-playset"))
    emitted = tmp_path / "emitted.json"
    _write_emitted(emitted, [{"kind": "idea", "token": "idea_ok"}])
    with pytest.raises(SB.BundleError, match="playset identities do not match"):
        SB.create_bundle(
            tmp_path / "bundle",
            exe=exe,
            mod_state=mod,
            dlc_state=dlc,
            load_order=load,
            specs=specs,
            emitted_tokens=emitted,
        )


def test_empty_or_unprovenanced_emitted_inventory_cannot_pass(tmp_path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    emitted = tmp_path / "emitted.json"
    _write_emitted(emitted, [])
    with pytest.raises(SB.BundleError, match="inventory is empty"):
        SB.create_bundle(
            tmp_path / "empty",
            exe=exe,
            mod_state=mod,
            dlc_state=dlc,
            load_order=load,
            specs=specs,
            emitted_tokens=emitted,
        )
    _write_emitted(emitted, [{"kind": "idea", "token": "idea_ok"}], provenance=False)
    with pytest.raises(SB.BundleError, match="provenance must be an object"):
        SB.create_bundle(
            tmp_path / "unprovenanced",
            exe=exe,
            mod_state=mod,
            dlc_state=dlc,
            load_order=load,
            specs=specs,
            emitted_tokens=emitted,
        )


def test_deferred_join_requires_provenance_and_is_structurally_verified(tmp_path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    with pytest.raises(SB.BundleError, match="requires --defer-provenance"):
        SB.create_bundle(
            tmp_path / "missing-proof",
            exe=exe,
            mod_state=mod,
            dlc_state=dlc,
            load_order=load,
            specs=specs,
            defer_token_gate=True,
        )
    deferred = tmp_path / "deferred.json"
    _write_json(
        deferred,
        {
            "schema": SB.DEFER_PROVENANCE_SCHEMA,
            "requested_by": "extractor-orchestrator",
            "reason": "capture has not started",
            "session_plan_id": "profile-1",
        },
    )
    bundle = tmp_path / "bundle"
    SB.create_bundle(
        bundle,
        exe=exe,
        mod_state=mod,
        dlc_state=dlc,
        load_order=load,
        specs=specs,
        defer_token_gate=True,
        defer_provenance=deferred,
    )
    with pytest.raises(SB.BundleError, match="join gate was deferred"):
        SB.verify_bundle(
            bundle,
            exe=exe,
            mod_state=mod,
            dlc_state=dlc,
            load_order=load,
            require_token_gate=True,
        )
    admitted = SB.verify_bundle(
        bundle,
        exe=exe,
        mod_state=mod,
        dlc_state=dlc,
        load_order=load,
        require_token_gate=False,
    )
    assert admitted == {
        "passed": True,
        "bundle_id": admitted["bundle_id"],
        "artifact_count": 7,
        "token_join_status": "deferred",
        "verification_phase": "pre_injection_deferred_allowed",
    }


def test_deferred_join_session_plan_must_match_binding_profile(tmp_path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    deferred = tmp_path / "wrong-profile.json"
    _write_json(
        deferred,
        {
            "schema": SB.DEFER_PROVENANCE_SCHEMA,
            "requested_by": "test",
            "reason": "capture has not started",
            "session_plan_id": "different-profile",
        },
    )
    with pytest.raises(SB.BundleError, match="does not match current binding profile"):
        SB.create_bundle(
            tmp_path / "wrong-profile-bundle",
            exe=exe,
            mod_state=mod,
            dlc_state=dlc,
            load_order=load,
            specs=specs,
            defer_token_gate=True,
            defer_provenance=deferred,
        )


@pytest.mark.parametrize("bad_status", [None, "", "unknown", "PASSED"])
def test_unknown_or_null_token_join_status_is_always_rejected(tmp_path, bad_status):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    emitted = tmp_path / "emitted.json"
    _write_emitted(emitted, [{"kind": "idea", "token": "idea_ok"}])
    bundle = tmp_path / "bundle"
    SB.create_bundle(
        bundle,
        exe=exe,
        mod_state=mod,
        dlc_state=dlc,
        load_order=load,
        specs=specs,
        emitted_tokens=emitted,
    )
    _rewrite_manifest(bundle, lambda manifest: manifest["token_join"].update(status=bad_status))
    with pytest.raises(SB.BundleError, match="status must be exactly"):
        SB.verify_bundle(
            bundle,
            exe=exe,
            mod_state=mod,
            dlc_state=dlc,
            load_order=load,
            require_token_gate=False,
        )


def test_catalog_source_occurrences_are_recomputed_during_verify(tmp_path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    emitted = tmp_path / "emitted.json"
    _write_emitted(emitted, [{"kind": "idea", "token": "idea_ok"}])
    bundle = tmp_path / "bundle"
    SB.create_bundle(
        bundle,
        exe=exe,
        mod_state=mod,
        dlc_state=dlc,
        load_order=load,
        specs=specs,
        emitted_tokens=emitted,
    )
    _rewrite_manifest(
        bundle,
        lambda manifest: manifest["bindings"]["catalog_source_occurrences"].update(rows=999),
    )
    with pytest.raises(SB.BundleError, match="effective source inventory binding mismatch"):
        SB.verify_bundle(bundle, exe=exe, mod_state=mod, dlc_state=dlc, load_order=load)


def test_false_token_join_counts_are_rejected_even_with_updated_hashes(tmp_path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    emitted = tmp_path / "emitted.json"
    _write_emitted(emitted, [{"kind": "idea", "token": "idea_ok"}])
    bundle = tmp_path / "bundle"
    SB.create_bundle(
        bundle,
        exe=exe,
        mod_state=mod,
        dlc_state=dlc,
        load_order=load,
        specs=specs,
        emitted_tokens=emitted,
    )
    report_path = bundle / "token_join_report.json"
    report = json.loads(report_path.read_text(encoding="utf-8"))
    report["counts"]["resolved"] = 2
    _write_json(report_path, report)
    _rewrite_manifest(
        bundle,
        lambda manifest: manifest["token_join"].update(sha256=SB.sha256_file(report_path)),
    )
    with pytest.raises(SB.BundleError, match="counts do not match results"):
        SB.verify_bundle(bundle, exe=exe, mod_state=mod, dlc_state=dlc, load_order=load)


def _deferred_fixture_bundle(tmp_path: Path):
    specs, exe, (mod, dlc, load) = _fixture(tmp_path)
    deferred = tmp_path / "deferred.json"
    _write_json(
        deferred,
        {
            "schema": SB.DEFER_PROVENANCE_SCHEMA,
            "requested_by": "test",
            "reason": "capture has not started",
            "session_plan_id": "profile-1",
        },
    )
    bundle = tmp_path / "deferred_bundle"
    SB.create_bundle(
        bundle,
        exe=exe,
        mod_state=mod,
        dlc_state=dlc,
        load_order=load,
        specs=specs,
        defer_token_gate=True,
        defer_provenance=deferred,
    )
    return bundle, exe, mod, dlc, load


def test_finalize_deferred_bundle_binds_exact_capture_and_preserves_source(tmp_path):
    bundle, exe, mod, dlc, load = _deferred_fixture_bundle(tmp_path)
    source_manifest = (bundle / "static_bundle_manifest.json").read_bytes()
    emitted = tmp_path / "emitted-final.json"
    _write_emitted(emitted, [{"kind": "idea", "token": "idea_ok"}])
    out = tmp_path / "final_bundle"
    result = SB.finalize_deferred_bundle(
        bundle,
        out,
        exe=exe,
        mod_state=mod,
        dlc_state=dlc,
        load_order=load,
        source_inventory=tmp_path / "source_inventory.json",
        live_resolution_report=tmp_path / "unused-resolution.json",
        emitted_tokens=emitted,
        expected_capture_id="capture-1",
        expected_session_id="session-1",
        expected_stream_sha256=SB.sha256_bytes(b"stream"),
        require_current_sources=False,
    )
    assert result["passed"] is True
    assert result["token_join_status"] == "passed"
    assert result["verification_phase"] == "post_capture_finalized"
    assert (bundle / "static_bundle_manifest.json").read_bytes() == source_manifest
    assert _REAL_VERIFY_BUNDLE(
        out,
        exe=exe,
        mod_state=mod,
        dlc_state=dlc,
        load_order=load,
        source_inventory=tmp_path / "source_inventory.json",
        require_current_sources=False,
        expected_capture_id="capture-1",
        expected_session_id="session-1",
        expected_stream_sha256=SB.sha256_bytes(b"stream"),
    )["passed"]


def test_finalize_deferred_bundle_rejects_capture_provenance_mismatch(tmp_path):
    bundle, exe, mod, dlc, load = _deferred_fixture_bundle(tmp_path)
    emitted = tmp_path / "emitted-final.json"
    _write_emitted(emitted, [{"kind": "idea", "token": "idea_ok"}])
    out = tmp_path / "bad-final"
    with pytest.raises(SB.BundleError, match="does not match expected current capture"):
        SB.finalize_deferred_bundle(
            bundle,
            out,
            exe=exe,
            mod_state=mod,
            dlc_state=dlc,
            load_order=load,
            source_inventory=tmp_path / "source_inventory.json",
            live_resolution_report=tmp_path / "unused-resolution.json",
            emitted_tokens=emitted,
            expected_capture_id="wrong-capture",
            expected_session_id="session-1",
            expected_stream_sha256=SB.sha256_bytes(b"stream"),
            require_current_sources=False,
        )
    assert not out.exists()


def _verify_cli_args(tmp_path: Path) -> list[str]:
    return [
        "verify",
        "--bundle", str(tmp_path / "bundle"),
        "--exe", str(tmp_path / "hoi4.exe"),
        "--mod-state", str(tmp_path / "mod.json"),
        "--dlc-state", str(tmp_path / "dlc.json"),
        "--load-order", str(tmp_path / "load.json"),
        "--source-inventory", str(tmp_path / "sources.json"),
        "--live-resolution-report", str(tmp_path / "resolution.json"),
    ]


def test_verify_cli_explicit_deferred_phase_requires_deferred_result(tmp_path, monkeypatch):
    calls = []

    def fake_verify(*args, **kwargs):
        calls.append((args, kwargs))
        return {
            "passed": True,
            "bundle_id": "a" * 64,
            "artifact_count": 7,
            "token_join_status": "deferred",
            "verification_phase": "pre_injection_deferred_allowed",
        }

    monkeypatch.setattr(SB, "verify_bundle", fake_verify)
    assert SB.main([*_verify_cli_args(tmp_path), "--allow-deferred-token-gate"]) == 0
    assert calls[0][1]["require_token_gate"] is False


def test_verify_cli_deferred_phase_rejects_finalized_result(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(
        SB,
        "verify_bundle",
        lambda *args, **kwargs: {
            "passed": True,
            "bundle_id": "a" * 64,
            "artifact_count": 7,
            "token_join_status": "passed",
            "verification_phase": "post_capture_finalized",
        },
    )
    assert SB.main([*_verify_cli_args(tmp_path), "--allow-deferred-token-gate"]) == 1
    assert "only valid for a deferred pre-injection bundle" in capsys.readouterr().err


def test_verify_cli_deferred_phase_rejects_final_capture_provenance(tmp_path, capsys):
    args = [
        *_verify_cli_args(tmp_path),
        "--allow-deferred-token-gate",
        "--expected-capture-id", "capture-1",
    ]
    assert SB.main(args) == 1
    assert "cannot accept final capture provenance" in capsys.readouterr().err
