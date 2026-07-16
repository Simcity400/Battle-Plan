from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import pytest

_HERE = Path(__file__).resolve().parent
_DEFS = _HERE.parent
sys.path.insert(0, str(_DEFS))

import parse_technology_sharing as P  # noqa: E402
import resolver as R  # noqa: E402
import technology_sharing_static_gate as G  # noqa: E402
import build_all as B  # noqa: E402
import live_binding_resolver as LR  # noqa: E402
from static_artifact_common import canonical_json, sha256_bytes, source_inventory  # noqa: E402


def _write(root: Path, rel: str, text: str) -> Path:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


def _layer(label: str, root: Path, *, kind: str | None = None, dlc_name: str | None = None):
    meta = {key: value for key, value in {"kind": kind, "dlc_name": dlc_name}.items() if value}
    return R._ex.Layer(label, root, meta=meta)


def _group(
    token: str,
    *,
    bonus: str = "0.1",
    categories: str | None = None,
    name: str | None = None,
    desc: str | None = None,
) -> str:
    category_line = "" if categories is None else f"categories = {{ {categories} }}"
    return f"""
technology_sharing_group = {{
  id = {token}
  name = {name or token + '_name'}
  desc = {desc or token + '_desc'}
  picture = GFX_technology_sharing_default
  research_sharing_per_country_bonus = {bonus}
  {category_line}
}}
"""


def _localisation_catalog(layers, keys):
    file_map = R._ex.resolve_loc(layers, "english")
    payload = {
        "schema": "static_localisation_join_r3.static_artifact.v1",
        "layers": [
            {
                "label": layer.label,
                "root": Path(layer.root).name,
                "replace_path": list(layer.replace_paths),
            }
            for layer in layers
        ],
        "meta": {
            "key_count": len(keys),
            "files_read": len(file_map),
            "files_total": len(file_map),
        },
        "source_files": {"localisation_files": source_inventory(list(file_map.items()))},
        "keys": keys,
    }
    payload["artifact_sha256"] = sha256_bytes(canonical_json(payload))
    return payload


def test_parser_preserves_source_and_definition_order_and_category_scope(tmp_path):
    vanilla = tmp_path / "vanilla"
    _write(
        vanilla,
        "common/technology_sharing/00_groups.txt",
        _group("unrestricted") + _group("empty_restriction", categories=""),
    )
    _write(
        vanilla,
        "common/technology_sharing/01_groups.txt",
        _group("naval_only", bonus="0.05", categories="naval_equipment naval_doctrine"),
    )

    out = P.parse_technology_sharing([_layer("vanilla", vanilla, kind="vanilla")])

    assert [row["id"] for row in out["definitions"]] == [
        "unrestricted",
        "empty_restriction",
        "naval_only",
    ]
    assert out["definitions"][0]["category_scope"] == "unrestricted_by_absent_categories"
    assert out["definitions"][0]["categories_present"] is False
    assert (
        out["definitions"][1]["category_scope"]
        == "explicit_empty_categories_semantics_unproven"
    )
    assert out["definitions"][1]["categories"] == []
    assert out["definitions"][2]["categories"] == ["naval_equipment", "naval_doctrine"]
    assert out["definitions"][2]["category_scope"] == "explicit_category_list"
    assert out["definitions"][2]["base_research_sharing_per_country_bonus"] == 0.05
    assert [row["source_provenance"]["catalog_order"] for row in out["definitions"]] == [0, 1, 2]
    assert [row["source_provenance"]["file_order"] for row in out["definitions"]] == [0, 0, 1]
    assert [row["source_provenance"]["block_order"] for row in out["definitions"]] == [0, 1, 0]


def test_mod_file_override_and_provenance_are_effective_not_duplicated(tmp_path):
    vanilla = tmp_path / "vanilla"
    mod = tmp_path / "mod"
    rel = "common/technology_sharing/00_groups.txt"
    _write(vanilla, rel, _group("vanilla_definition"))
    _write(mod, rel, _group("mod_definition"))

    out = P.parse_technology_sharing(
        [_layer("vanilla", vanilla, kind="vanilla"), _layer("Fixture Mod", mod, kind="mod")]
    )

    assert list(out["definitions_by_id"]) == ["mod_definition"]
    row = out["definitions"][0]
    assert row["source_layer"] == "Fixture Mod"
    assert row["source_provenance"]["layer_kind"] == "mod"
    assert row["source_provenance"]["layer_ordinal"] == 1
    assert row["source_provenance"]["mod_name"] == "Fixture Mod"
    assert row["source_provenance"]["dlc_name"] is None


def test_mod_named_vanilla_is_classified_by_layer_position_not_label(tmp_path):
    base = tmp_path / "base"
    mod = tmp_path / "mod"
    _write(mod, "common/technology_sharing/00.txt", _group("mod_group"))

    out = P.parse_technology_sharing([_layer("base", base), _layer("vanilla", mod)])

    provenance = out["definitions"][0]["source_provenance"]
    assert provenance["layer_kind"] == "mod"
    assert provenance["mod_name"] == "vanilla"


def test_dlc_provenance_is_retained_when_layer_model_provides_it(tmp_path):
    vanilla = tmp_path / "vanilla"
    dlc = tmp_path / "dlc043"
    _write(dlc, "common/technology_sharing/10_groups.txt", _group("dlc_group"))

    out = P.parse_technology_sharing(
        [
            _layer("vanilla", vanilla, kind="vanilla"),
            _layer("dlc/dlc043", dlc, kind="dlc", dlc_name="dlc043"),
        ]
    )

    provenance = out["definitions"][0]["source_provenance"]
    assert provenance["layer_kind"] == "dlc"
    assert provenance["dlc_name"] == "dlc043"
    assert provenance["layer_ordinal"] == 1


def test_gate_materializes_integrated_and_active_product_dlc_but_not_asset_only(
    tmp_path, monkeypatch
):
    vanilla = tmp_path / "vanilla"
    base_rel = "common/technology_sharing/00_base.txt"
    integrated_rel = "common/technology_sharing/10_integrated.txt"
    product_rel = "common/technology_sharing/20_product.txt"
    asset_rel = "common/technology_sharing/30_asset_only.txt"
    _write(vanilla, base_rel, _group("base_group"))
    _write(vanilla / "integrated_dlc/dlc_integrated", integrated_rel, _group("integrated_group"))
    _write(vanilla / "dlc/dlc_product", product_rel, _group("product_group"))
    _write(vanilla / "dlc/dlc_asset", asset_rel, _group("asset_only_group"))
    layers = [_layer("vanilla", vanilla, kind="vanilla")]
    inventory = LR.recompute_source_inventory(
        vanilla,
        [],
        ["dlc_product"],
        ["dlc_asset"],
        {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
    )
    inventory_path = tmp_path / "source_inventory.json"
    inventory_path.write_text(json.dumps(inventory), encoding="utf-8")
    monkeypatch.setattr(G.R, "build_layers", lambda **_kwargs: layers)
    args = argparse.Namespace(
        vanilla=str(vanilla),
        mods="",
        userdata=str(tmp_path),
        strict_mods=True,
        localisation_catalog=tmp_path / "missing_localisation_catalog.json",
        source_inventory=inventory_path,
    )

    catalog = G.build_catalog(args)

    assert [row["id"] for row in catalog["definitions"]] == [
        "base_group",
        "integrated_group",
        "product_group",
    ]
    assert "asset_only_group" not in catalog["definitions_by_id"]
    assert [row["label"] for row in catalog["layers"]] == [
        "vanilla",
        "integrated_dlc/dlc_integrated",
        "dlc/dlc_product",
    ]
    assert catalog["source_namespace_proof"] == {
        "schema": G.SOURCE_NAMESPACE_PROOF_SCHEMA,
        "status": "bound_to_live_source_inventory",
        "source_inventory_sha256": inventory["inventory_sha256"],
        "selected_namespace_labels": [
            "vanilla",
            "integrated_dlc/dlc_integrated",
            "dlc/dlc_product",
        ],
        "excluded_asset_only_namespace_labels": ["dlc/dlc_asset"],
    }


def test_runtime_join_candidate_does_not_invent_a_hash(tmp_path):
    vanilla = tmp_path / "vanilla"
    _write(vanilla, "common/technology_sharing/00.txt", _group("candidate"))

    parsed = P.parse_technology_sharing([_layer("vanilla", vanilla)])
    row = parsed["definitions"][0]

    assert row["runtime_token_candidate"] == "candidate"
    assert row["runtime_hash_candidate"] is None
    assert (
        row["runtime_join_status"]
        == "definition_id_candidate_only_runtime_token_is_intern_ordinal_mapping_unproven"
    )
    assert parsed["meta"]["runtime_token_proof"] == "engine_runtime_intern_ordinal_not_string_hash"


def test_category_id_may_literally_be_named_categories(tmp_path):
    vanilla = tmp_path / "vanilla"
    _write(
        vanilla,
        "common/technology_sharing/00.txt",
        _group("literal", categories="categories"),
    )

    parsed = P.parse_technology_sharing([_layer("vanilla", vanilla)])

    assert parsed["definitions"][0]["categories"] == ["categories"]


def test_duplicate_definition_id_across_effective_files_fails_closed(tmp_path):
    vanilla = tmp_path / "vanilla"
    _write(vanilla, "common/technology_sharing/00.txt", _group("duplicate"))
    _write(vanilla, "common/technology_sharing/01.txt", _group("duplicate"))

    with pytest.raises(RuntimeError, match="duplicate technology-sharing id"):
        P.parse_technology_sharing([_layer("vanilla", vanilla)])


@pytest.mark.parametrize(
    "text, error",
    [
        (_group("bad").replace("id = bad", "id = bad\nid = other"), "exactly one"),
        (_group("bad").replace("desc = bad_desc\n", ""), "exactly one"),
        (_group("bad", bonus="not_a_number"), "not numeric"),
        (_group("bad", categories="naval naval"), "duplicate categories"),
        (
            _group("bad").replace(
                "research_sharing_per_country_bonus = 0.1",
                "research_sharing_per_country_bonus = 0.1\ncategories = { naval = yes }",
            ),
            "anonymous scalar tokens",
        ),
        (
            _group("bad").replace(
                "research_sharing_per_country_bonus = 0.1",
                "research_sharing_per_country_bonus = 0.1\ncategories = { { naval } }",
            ),
            "anonymous scalar tokens",
        ),
        (
            _group("bad").replace(
                "research_sharing_per_country_bonus = 0.1",
                "research_sharing_per_country_bonus = 0.1\ncategories = { = }",
            ),
            "anonymous scalar tokens",
        ),
        (
            _group("bad").replace(
                "research_sharing_per_country_bonus = 0.1",
                "research_sharing_per_country_bonus = 0.1\ncategories { naval }",
            ),
            "must use categories",
        ),
        (
            _group("bad", categories="naval > army"),
            "invalid operator-like token",
        ),
        (
            _group("bad", categories="naval>army"),
            "invalid operator-like token",
        ),
        (_group("bad") + "}", "unmatched closing brace"),
    ],
)
def test_malformed_or_ambiguous_definitions_fail_closed(tmp_path, text, error):
    vanilla = tmp_path / "vanilla"
    _write(vanilla, "common/technology_sharing/00.txt", text)

    with pytest.raises(RuntimeError, match=error):
        P.parse_technology_sharing([_layer("vanilla", vanilla)])


def test_gate_enriches_localisation_and_hashes_deterministically(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    layers = [_layer("vanilla", vanilla, kind="vanilla")]
    _write(
        vanilla,
        "common/technology_sharing/00.txt",
        _group("fixture", name="fixture_title", desc="fixture_description"),
    )
    _write(
        vanilla,
        "localisation/english/technology_sharing_l_english.yml",
        'l_english:\n fixture_title:0 "Fixture research"\n fixture_description:0 "Fixture description"\n',
    )
    loc_path = tmp_path / "localisation_catalog.json"
    loc_path.write_text(
        json.dumps(_localisation_catalog(
            layers,
            {
                "fixture_title": {"name": "Fixture research"},
                "fixture_description": {"name": "Fixture description"},
            },
        )),
        encoding="utf-8",
    )
    monkeypatch.setattr(G.R, "build_layers", lambda **_kwargs: layers)
    args = argparse.Namespace(
        vanilla=None,
        mods=None,
        userdata=None,
        strict_mods=True,
        localisation_catalog=loc_path,
    )

    first = G.build_catalog(args)
    second = G.build_catalog(args)
    row = first["definitions"][0]

    assert row["localised_name"] == "Fixture research"
    assert row["localised_description"] == "Fixture description"
    assert first["artifact_sha256"] == second["artifact_sha256"]
    assert all(G.catalog_gate_checks(first).values())


def test_standalone_build_and_gate_always_require_complete_mod_resolution(
    tmp_path, monkeypatch
):
    vanilla = tmp_path / "vanilla"
    layers = [_layer("vanilla", vanilla, kind="vanilla")]
    _write(vanilla, "common/technology_sharing/00.txt", _group("fixture"))
    calls = []

    def capture_layers(**kwargs):
        calls.append(kwargs)
        return layers

    monkeypatch.setattr(P.R, "build_layers", capture_layers)
    P.build(vanilla=str(vanilla), mods=None, userdata=tmp_path)

    monkeypatch.setattr(G.R, "build_layers", capture_layers)
    args = argparse.Namespace(
        vanilla=str(vanilla),
        mods=None,
        userdata=str(tmp_path),
        strict_mods=False,
        localisation_catalog=tmp_path / "missing_localisation_catalog.json",
    )
    G.build_catalog(args)

    assert len(calls) == 2
    assert all(call["strict_mods"] is True for call in calls)


@pytest.mark.parametrize("drift", ["source_bytes", "layer_set"])
def test_gate_rejects_stale_or_wrong_mod_localisation_catalog(tmp_path, monkeypatch, drift):
    vanilla = tmp_path / "vanilla"
    mod = tmp_path / "mod"
    _write(vanilla, "common/technology_sharing/00.txt", _group("fixture"))
    loc_file = _write(
        vanilla,
        "localisation/english/technology_sharing_l_english.yml",
        'l_english:\n fixture_name:0 "Old name"\n fixture_desc:0 "Old description"\n',
    )
    vanilla_layer = _layer("vanilla", vanilla, kind="vanilla")
    original_layers = [vanilla_layer]
    payload = _localisation_catalog(
        original_layers,
        {
            "fixture_name": {"name": "Old name"},
            "fixture_desc": {"name": "Old description"},
        },
    )
    loc_path = tmp_path / "localisation_catalog.json"
    loc_path.write_text(json.dumps(payload), encoding="utf-8")
    current_layers = original_layers
    if drift == "source_bytes":
        loc_file.write_text(
            'l_english:\n fixture_name:0 "New name"\n fixture_desc:0 "New description"\n',
            encoding="utf-8",
        )
        expected_error = "source inventory does not match"
    else:
        _write(mod, "localisation/english/mod_l_english.yml", 'l_english:\n MOD_KEY:0 "Mod"\n')
        current_layers = [vanilla_layer, _layer("Fixture Mod", mod, kind="mod")]
        expected_error = "layers do not match"
    monkeypatch.setattr(G.R, "build_layers", lambda **_kwargs: current_layers)
    args = argparse.Namespace(
        vanilla=None,
        mods=None,
        userdata=None,
        strict_mods=True,
        localisation_catalog=loc_path,
    )

    with pytest.raises(RuntimeError, match=expected_error):
        G.build_catalog(args)


def test_gate_rejects_self_hashed_localisation_catalog_that_omits_current_keys(
    tmp_path, monkeypatch
):
    vanilla = tmp_path / "vanilla"
    layers = [_layer("vanilla", vanilla, kind="vanilla")]
    _write(
        vanilla,
        "common/technology_sharing/00.txt",
        _group("fixture", name="fixture_title", desc="fixture_description"),
    )
    _write(
        vanilla,
        "localisation/english/technology_sharing_l_english.yml",
        'l_english:\n fixture_title:0 "Fixture research"\n fixture_description:0 "Fixture description"\n',
    )
    payload = _localisation_catalog(layers, {})
    payload["meta"]["files_read"] = 0
    payload["artifact_sha256"] = sha256_bytes(
        canonical_json({key: value for key, value in payload.items() if key != "artifact_sha256"})
    )
    loc_path = tmp_path / "localisation_catalog.json"
    loc_path.write_text(json.dumps(payload), encoding="utf-8")
    monkeypatch.setattr(G.R, "build_layers", lambda **_kwargs: layers)
    args = argparse.Namespace(
        vanilla=None,
        mods=None,
        userdata=None,
        strict_mods=True,
        localisation_catalog=loc_path,
    )

    with pytest.raises(RuntimeError, match="does not match current effective files"):
        G.build_catalog(args)


def test_gate_rejects_same_count_forged_localisation_key_projection(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    layers = [_layer("vanilla", vanilla, kind="vanilla")]
    _write(vanilla, "common/technology_sharing/00.txt", _group("fixture"))
    _write(
        vanilla,
        "localisation/english/technology_sharing_l_english.yml",
        'l_english:\n fixture_name:0 "Correct name"\n fixture_desc:0 "Correct description"\n',
    )
    payload = _localisation_catalog(
        layers,
        {
            "fixture_name": {"name": "Correct name"},
            "forged_key": {"name": "Forged replacement"},
        },
    )
    loc_path = tmp_path / "localisation_catalog.json"
    loc_path.write_text(json.dumps(payload), encoding="utf-8")
    monkeypatch.setattr(G.R, "build_layers", lambda **_kwargs: layers)
    args = argparse.Namespace(
        vanilla=None,
        mods=None,
        userdata=None,
        strict_mods=True,
        localisation_catalog=loc_path,
    )

    with pytest.raises(RuntimeError, match="key contents do not match"):
        G.build_catalog(args)


def test_gate_accepts_legitimate_localisation_file_with_no_keys(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    layers = [_layer("vanilla", vanilla, kind="vanilla")]
    _write(vanilla, "common/technology_sharing/00.txt", _group("fixture"))
    _write(
        vanilla,
        "localisation/english/empty_l_english.yml",
        "l_english:\n",
    )
    payload = _localisation_catalog(layers, {})
    payload["meta"]["files_read"] = 0
    payload["artifact_sha256"] = sha256_bytes(
        canonical_json({key: value for key, value in payload.items() if key != "artifact_sha256"})
    )
    loc_path = tmp_path / "localisation_catalog.json"
    loc_path.write_text(json.dumps(payload), encoding="utf-8")
    monkeypatch.setattr(G.R, "build_layers", lambda **_kwargs: layers)
    args = argparse.Namespace(
        vanilla=None,
        mods=None,
        userdata=None,
        strict_mods=True,
        localisation_catalog=loc_path,
    )

    catalog = G.build_catalog(args)

    assert catalog["definitions"][0]["name_localisation_status"] == "missing"
    assert all(G.catalog_gate_checks(catalog).values())


def test_implicit_active_modset_missing_descriptor_fails_closed(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    vanilla.mkdir()
    userdata = tmp_path / "userdata"
    userdata.mkdir()
    manifest = tmp_path / "active_modset.json"
    manifest.write_text(
        json.dumps(
                {
                    "load_order": [
                        {"kind": "vanilla", "load_index": 0},
                        {
                            "kind": "mod",
                            "descriptor": "mod/missing_descriptor.mod",
                            "load_index": 1,
                        }
                    ]
                }
        ),
        encoding="utf-8",
    )
    monkeypatch.setattr(P.R, "_DEFAULT_MODSET", manifest)

    with pytest.raises(RuntimeError, match="strict mod resolution failed"):
        P.build(vanilla=str(vanilla), mods=None, userdata=userdata)


def test_implicit_active_modset_mod_row_without_descriptor_fails_closed(
    tmp_path, monkeypatch
):
    vanilla = tmp_path / "vanilla"
    vanilla.mkdir()
    manifest = tmp_path / "active_modset.json"
    manifest.write_text(
        json.dumps({"load_order": [{"kind": "mod", "load_index": 0}]}),
        encoding="utf-8",
    )
    monkeypatch.setattr(P.R, "_DEFAULT_MODSET", manifest)

    with pytest.raises(RuntimeError, match="requires a descriptor"):
        P.build(vanilla=str(vanilla), mods=None, userdata=tmp_path)


@pytest.mark.parametrize(
    "load_order, error",
    [
        ([], "exactly one vanilla"),
        (
            [
                {"kind": "vanilla", "load_index": 0},
                {"kind": "vanilla", "load_index": 1},
            ],
            "vanilla row must be first",
        ),
        (
            [{"kind": "vanilla", "descriptor": "mod/actual.mod", "load_index": 0}],
            "must not name a descriptor",
        ),
        (
            [
                {
                    "kind": "vanilla",
                    "descriptor": ["mod/actual.mod"],
                    "load_index": 0,
                }
            ],
            "must not name a descriptor",
        ),
        (
            [{"kind": "mod", "descriptor": "mod/actual.mod", "load_index": 0}],
            "exactly one vanilla",
        ),
        (
            [
                {"kind": "vanilla", "load_index": 0},
                {
                    "kind": "mod",
                    "descriptor": "mod/actual.mod",
                    "load_index": 3,
                },
            ],
            "non-contiguous load_index",
        ),
        (
            [{"kind": "vanilla", "load_index": 0.0}],
            "non-contiguous load_index",
        ),
    ],
)
def test_implicit_active_modset_structure_fails_closed(
    tmp_path, monkeypatch, load_order, error
):
    vanilla = tmp_path / "vanilla"
    vanilla.mkdir()
    manifest = tmp_path / "active_modset.json"
    manifest.write_text(json.dumps({"load_order": load_order}), encoding="utf-8")
    monkeypatch.setattr(P.R, "_DEFAULT_MODSET", manifest)

    with pytest.raises(RuntimeError, match=error):
        P.build(vanilla=str(vanilla), mods=None, userdata=tmp_path)


def test_implicit_active_modset_mod_count_mismatch_fails_closed(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    vanilla.mkdir()
    manifest = tmp_path / "active_modset.json"
    manifest.write_text(
        json.dumps(
            {
                "load_order": [{"kind": "vanilla", "load_index": 0}],
                "mod_count": 1,
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setattr(P.R, "_DEFAULT_MODSET", manifest)

    with pytest.raises(RuntimeError, match="mod_count does not match"):
        P.build(vanilla=str(vanilla), mods=None, userdata=tmp_path)


@pytest.mark.parametrize(
    "entry",
    [
        {"descriptor": "mod/example.mod"},
        {"kind": "mdo", "descriptor": "mod/example.mod"},
        {},
    ],
)
def test_implicit_active_modset_invalid_or_missing_kind_fails_closed(
    tmp_path, monkeypatch, entry
):
    vanilla = tmp_path / "vanilla"
    vanilla.mkdir()
    manifest = tmp_path / "active_modset.json"
    manifest.write_text(json.dumps({"load_order": [entry]}), encoding="utf-8")
    monkeypatch.setattr(P.R, "_DEFAULT_MODSET", manifest)

    with pytest.raises(RuntimeError, match="invalid or missing kind"):
        P.build(vanilla=str(vanilla), mods=None, userdata=tmp_path)


def test_explicit_missing_mod_fails_before_aggregate_output(tmp_path, monkeypatch):
    vanilla = tmp_path / "vanilla"
    vanilla.mkdir()
    out_dir = tmp_path / "out"
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "build_all.py",
            "--vanilla",
            str(vanilla),
            "--userdata",
            str(tmp_path),
            "--mods",
            "mod/missing_descriptor.mod",
            "--out",
            str(out_dir),
        ],
    )

    with pytest.raises(RuntimeError, match="strict mod resolution failed"):
        B.main()
    assert not out_dir.exists()


def test_real_installed_corpus_category_shape_remains_compatible():
    try:
        root = R.vanilla_root()
    except (FileNotFoundError, OSError):
        pytest.skip("the game install unavailable")
    layers = R.build_layers(vanilla=str(root), mods=[])

    out = P.parse_technology_sharing(layers)

    assert out["meta"]["definition_count"] > 0
    assert out["meta"]["source_files_parsed"] == out["meta"]["source_files_total"]
    assert not out["meta"]["warnings"]
    assert (
        out["meta"]["categories_absent_unrestricted_count"]
        + out["meta"]["categories_explicit_nonempty_count"]
        + out["meta"]["categories_explicit_empty_unproven_count"]
        == out["meta"]["definition_count"]
    )
