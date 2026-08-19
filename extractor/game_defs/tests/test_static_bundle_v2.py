from __future__ import annotations

import sys
from pathlib import Path

import pytest


HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import static_bundle as SB


@pytest.mark.parametrize(
    ("payload", "detail"),
    [
        (b'{"schema":"first","schema":"second"}', "duplicate JSON object member"),
        (b'{"outer":{"value":1,"value":2}}', "duplicate JSON object member"),
        (b'{"value":NaN}', "non-finite JSON constant"),
        (b'{"value":Infinity}', "non-finite JSON constant"),
    ],
)
def test_shared_json_loader_rejects_ambiguous_or_nonstandard_objects(
    tmp_path: Path, payload: bytes, detail: str
):
    path = tmp_path / "ambiguous.json"
    path.write_bytes(payload)

    with pytest.raises(SB.BundleError, match=detail):
        SB.load_object(path, "fixture authority")


def test_emitted_token_inventory_uses_the_shared_strict_json_loader(tmp_path: Path):
    path = tmp_path / "emitted_tokens.json"
    path.write_bytes(
        b'{"schema":"extractor.emitted_token_inventory.v1",'
        b'"tokens":[],"tokens":[],'
        b'"provenance":{"capture_id":"capture",'
        b'"session_id":"session",'
        b'"stream_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
        b'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}'
    )

    with pytest.raises(SB.BundleError, match="duplicate JSON object member"):
        SB._load_token_inventory(
            path,
            "emitted token inventory",
            SB.EMITTED_INVENTORY_SCHEMA,
            require_capture_provenance=True,
        )


@pytest.mark.parametrize("encoder", [SB.canonical_json, SB.pretty_json])
def test_bundle_json_encoders_refuse_nonfinite_numbers(encoder):
    with pytest.raises(ValueError, match="Out of range float values"):
        encoder({"value": float("nan")})


def _source_row(
    namespace_ordinal: int,
    layer_kind: str,
    layer_id: str,
    relpath: str,
    payload: bytes,
) -> dict:
    payload_sha256 = SB.sha256_bytes(payload)
    return {
        "namespace_ordinal": namespace_ordinal,
        "layer_kind": layer_kind,
        "layer_id": layer_id,
        "relpath": relpath,
        "bytes": len(payload),
        "sha256": payload_sha256,
        "storage_kind": "directory",
        "source_container_sha256": None,
        "source_byte_offset": 0,
        "source_byte_count": len(payload),
        "source_range_sha256": payload_sha256,
    }


def _inventory_document(
    namespace_specs: list[tuple[str, str, str, list[str]]],
    rows: list[dict],
) -> dict:
    namespaces = []
    for ordinal, (kind, layer_id, label, replace_paths) in enumerate(namespace_specs):
        namespace_rows = [row for row in rows if row["namespace_ordinal"] == ordinal]
        content_rows = [
            {"relpath": row["relpath"], "bytes": row["bytes"], "sha256": row["sha256"]}
            for row in namespace_rows
        ]
        namespaces.append(
            {
                "ordinal": ordinal,
                "kind": kind,
                "binding": {
                    "vanilla": "base",
                    "integrated_dlc": "integrated",
                    "dlc": "active_product",
                    "mod": "active_mod",
                }[kind],
                "id": layer_id,
                "label": label,
                "replace_paths": replace_paths,
                "storage_kind": "directory",
                "source_container_sha256": None,
                "source_container_bytes": None,
                "file_count": len(namespace_rows),
                "content_sha256": SB.sha256_bytes(SB.canonical_json(content_rows)),
                "binding_content_sha256": SB.sha256_bytes(SB.canonical_json(content_rows)),
            }
        )
    core = {
        "schema": SB.SOURCE_INVENTORY_SCHEMA,
        "identity": {"playset_id": "playset-1", "profile_id": "profile-1"},
        "namespaces": namespaces,
        "rows": rows,
        "namespace_sha256": SB.sha256_bytes(SB.canonical_json(namespaces)),
        "rows_sha256": SB.sha256_bytes(SB.canonical_json(rows)),
    }
    return {**core, "inventory_sha256": SB.sha256_bytes(SB.canonical_json(core))}


def _write_inventory(path: Path, value: dict) -> None:
    path.write_bytes(SB.pretty_json(value))


def _load_inventory(tmp_path: Path, namespace_specs, rows) -> dict:
    path = tmp_path / "source_inventory.json"
    _write_inventory(path, _inventory_document(namespace_specs, rows))
    return SB.load_source_inventory(path)


def _catalog(layers: list[tuple[str, list[str]]], relpath: str, payload: bytes) -> dict:
    return {
        "layers": [
            {"label": label, "replace_path": replace_paths}
            for label, replace_paths in layers
        ],
        "source_files": {
            "rows": [
                {
                    "relpath": relpath,
                    "bytes": len(payload),
                    "sha256": SB.sha256_bytes(payload),
                }
            ]
        },
    }


@pytest.mark.parametrize("digest_field", ["namespace_sha256", "rows_sha256", "inventory_sha256"])
def test_load_source_inventory_rejects_tampered_digests(tmp_path, digest_field):
    rows = [_source_row(0, "vanilla", "vanilla", "common/ideas/base.txt", b"base")]
    document = _inventory_document(
        [("vanilla", "vanilla", "vanilla", [])],
        rows,
    )
    document[digest_field] = "0" * 64
    path = tmp_path / "source_inventory.json"
    _write_inventory(path, document)

    with pytest.raises(SB.BundleError, match="live source inventory digest mismatch"):
        SB.load_source_inventory(path)


@pytest.mark.parametrize(
    "relpath",
    [
        "../common/ideas/base.txt",
        "common\\ideas\\base.txt",
        "C:/common/ideas/base.txt",
    ],
)
def test_load_source_inventory_rejects_unsafe_paths(tmp_path, relpath):
    rows = [_source_row(0, "vanilla", "vanilla", relpath, b"base")]
    path = tmp_path / "source_inventory.json"
    _write_inventory(
        path,
        _inventory_document([("vanilla", "vanilla", "vanilla", [])], rows),
    )

    with pytest.raises(SB.BundleError, match=r"relpath is unsafe"):
        SB.load_source_inventory(path)


def test_load_source_inventory_rejects_case_colliding_rows(tmp_path):
    rows = [
        _source_row(0, "vanilla", "vanilla", "common/ideas/Base.txt", b"first"),
        _source_row(0, "vanilla", "vanilla", "COMMON/IDEAS/base.TXT", b"second"),
    ]
    path = tmp_path / "source_inventory.json"
    _write_inventory(
        path,
        _inventory_document([("vanilla", "vanilla", "vanilla", [])], rows),
    )

    with pytest.raises(SB.BundleError, match="duplicate/case-colliding live source row"):
        SB.load_source_inventory(path)


def test_bind_catalog_sources_uses_highest_precedence_case_insensitive_winner(tmp_path):
    relpath = "common/ideas/base.txt"
    rows = [
        _source_row(0, "vanilla", "vanilla", relpath, b"vanilla"),
        _source_row(1, "mod", "mod-1", "COMMON/IDEAS/BASE.TXT", b"modded"),
    ]
    inventory = _load_inventory(
        tmp_path,
        [
            ("vanilla", "vanilla", "vanilla", []),
            ("mod", "mod-1", "mod-1", []),
        ],
        rows,
    )
    catalogs = {
        "ideas": _catalog(
            [("vanilla", []), ("mod-1", [])],
            "Common/Ideas/Base.txt",
            b"modded",
        )
    }

    report = SB.bind_catalog_sources(catalogs, inventory)

    assert report["passed"] is True
    assert report["bindings"][0]["namespace_ordinal"] == 1
    assert report["bindings"][0]["layer_id"] == "mod-1"


def test_bind_catalog_sources_rejects_catalog_layer_order_drift(tmp_path):
    rows = [
        _source_row(0, "vanilla", "vanilla", "common/ideas/base.txt", b"vanilla"),
        _source_row(1, "mod", "mod-1", "common/ideas/base.txt", b"modded"),
    ]
    inventory = _load_inventory(
        tmp_path,
        [
            ("vanilla", "vanilla", "vanilla", []),
            ("mod", "mod-1", "mod-1", []),
        ],
        rows,
    )
    catalogs = {
        "ideas": _catalog(
            [("mod-1", []), ("vanilla", [])],
            "common/ideas/base.txt",
            b"modded",
        )
    }

    with pytest.raises(SB.BundleError, match="catalog layer precedence does not match"):
        SB.bind_catalog_sources(catalogs, inventory)


def test_bind_catalog_sources_rejects_omitted_active_dlc_definition_layer(tmp_path):
    rows = [
        _source_row(0, "vanilla", "vanilla", "common/ideas/base.txt", b"base"),
        _source_row(1, "dlc", "dlc-1", "common/ideas/dlc.txt", b"dlc"),
    ]
    inventory = _load_inventory(
        tmp_path,
        [
            ("vanilla", "vanilla", "vanilla", []),
            ("dlc", "dlc-1", "dlc/dlc-1", []),
        ],
        rows,
    )
    catalogs = {
        "ideas": _catalog(
            [("vanilla", [])],
            "common/ideas/base.txt",
            b"base",
        )
    }

    with pytest.raises(SB.BundleError, match="exact live definition namespace roster"):
        SB.bind_catalog_sources(catalogs, inventory)


def test_bind_catalog_sources_rejects_replace_path_metadata_drift(tmp_path):
    rows = [_source_row(0, "vanilla", "vanilla", "common/ideas/base.txt", b"base")]
    inventory = _load_inventory(
        tmp_path,
        [("vanilla", "vanilla", "vanilla", ["common/ideas"])],
        rows,
    )
    catalogs = {
        "ideas": _catalog(
            [("vanilla", [])],
            "common/ideas/base.txt",
            b"base",
        )
    }

    with pytest.raises(SB.BundleError, match="replace_path mismatch for layer vanilla"):
        SB.bind_catalog_sources(catalogs, inventory)


def test_bind_catalog_sources_replace_path_clears_lower_layer_winner(tmp_path):
    rows = [_source_row(0, "vanilla", "vanilla", "common/ideas/base.txt", b"base")]
    inventory = _load_inventory(
        tmp_path,
        [
            ("vanilla", "vanilla", "vanilla", []),
            ("mod", "mod-1", "mod-1", ["common/ideas"]),
        ],
        rows,
    )
    catalogs = {
        "ideas": _catalog(
            [("vanilla", []), ("mod-1", ["common/ideas"])],
            "common/ideas/base.txt",
            b"base",
        )
    }

    with pytest.raises(SB.BundleError, match="has no effective live namespace row"):
        SB.bind_catalog_sources(catalogs, inventory)


def test_bind_catalog_sources_rejects_missing_live_row(tmp_path):
    inventory = _load_inventory(
        tmp_path,
        [("vanilla", "vanilla", "vanilla", [])],
        [_source_row(0, "vanilla", "vanilla", "common/ideas/other.txt", b"other")],
    )
    catalogs = {
        "ideas": _catalog(
            [("vanilla", [])],
            "common/ideas/base.txt",
            b"base",
        )
    }

    with pytest.raises(SB.BundleError, match="has no effective live namespace row"):
        SB.bind_catalog_sources(catalogs, inventory)


@pytest.mark.parametrize("field", ["bytes", "sha256"])
def test_bind_catalog_sources_rejects_wrong_winner_bytes(tmp_path, field):
    relpath = "common/ideas/base.txt"
    inventory = _load_inventory(
        tmp_path,
        [("vanilla", "vanilla", "vanilla", [])],
        [_source_row(0, "vanilla", "vanilla", relpath, b"live")],
    )
    catalog = _catalog([("vanilla", [])], relpath, b"live")
    occurrence = catalog["source_files"]["rows"][0]
    if field == "bytes":
        occurrence["bytes"] += 1
    else:
        occurrence["sha256"] = SB.sha256_bytes(b"different")

    with pytest.raises(SB.BundleError, match="bytes do not match live namespace winner"):
        SB.bind_catalog_sources({"ideas": catalog}, inventory)
