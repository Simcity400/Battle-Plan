from __future__ import annotations

import datetime as dt
import json
import sys
from pathlib import Path

import pytest


HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))

import live_binding_resolver as R  # noqa: E402
import static_bundle as SB  # noqa: E402


def test_runtime_log_requires_exact_counts(tmp_path: Path):
    log = tmp_path / "system.log"
    log.write_text(
        "[18:24:52][no_game_date][main.cpp:1]: start\n"
        "[18:25:40][no_game_date][gameapplication.cpp:1]: Version: Test v1\n"
        "[18:25:40][no_game_date][gameapplication.cpp:1]: Active DLC Count: 2\n"
        "[18:25:40][no_game_date][gameapplication.cpp:1]: Active Mod Count: 0\n"
        "[18:25:40][no_game_date][gameapplication.cpp:1]: Active DLC: One\n",
        encoding="utf-8",
    )
    with pytest.raises(R.ResolutionError, match="DLC count mismatch"):
        R.parse_runtime_log(log)


def test_descriptor_accepts_directory_or_archive_and_requires_identity(tmp_path: Path):
    directory = tmp_path / "directory.dlc"
    directory.write_text('name="Directory"\npath=dlc/directory\n', encoding="utf-8")
    archive = tmp_path / "archive.dlc"
    archive.write_text('name="Archive"\narchive="dlc/archive.zip"\n', encoding="utf-8")
    assert R.parse_descriptor(directory)["path"] == "dlc/directory"
    assert R.parse_descriptor(archive)["archive"] == "dlc/archive.zip"
    broken = tmp_path / "broken.dlc"
    broken.write_text('name="Broken"\n', encoding="utf-8")
    with pytest.raises(R.ResolutionError, match="path/archive"):
        R.parse_descriptor(broken)


def test_hash_tree_is_deterministic_and_content_sensitive(tmp_path: Path):
    root = tmp_path / "content"
    (root / "common").mkdir(parents=True)
    (root / "common/a.txt").write_text("a", encoding="utf-8")
    (root / "common/b.txt").write_text("b", encoding="utf-8")
    first, rows = R.hash_tree(root, layer_kind="mod", layer_id="fixture")
    second, rows_again = R.hash_tree(root, layer_kind="mod", layer_id="fixture")
    assert first == second
    assert rows == rows_again
    (root / "common/b.txt").write_text("changed", encoding="utf-8")
    changed, _ = R.hash_tree(root, layer_kind="mod", layer_id="fixture")
    assert changed != first


def _catalog(role: str, mod_name: str | None, enabled_dlcs: list[str] | None = None):
    layers = [{"label": "vanilla", "meta": {"enabled_dlcs": enabled_dlcs or []}}]
    if mod_name:
        layers.append({"label": mod_name, "meta": {"kind": "mod"}})
    return {"catalog": {"layers": layers}}


def test_catalog_compatibility_rejects_mod_and_dlc_drift():
    validated = {
        spec.role: _catalog(
            spec.role,
            "AMSB - DEV",
            ["dlc_old"] if spec.role == "gfx_icon_asset_manifest_static" else None,
        )
        for spec in SB.DEFAULT_SPECS
    }
    blockers = R.catalog_compatibility(validated, [], ["dlc_live"])
    assert len(blockers) == len(SB.DEFAULT_SPECS) + 2
    assert any("AMSB - DEV" in blocker for blocker in blockers)
    assert any("missing runtime active DLC ids" in blocker for blocker in blockers)
    assert any("unclassified extra DLC asset layers" in blocker for blocker in blockers)


def test_catalog_compatibility_accepts_active_product_subset_with_classified_asset_extra():
    validated = {
        spec.role: _catalog(
            spec.role,
            None,
            ["dlc_product", "dlc_descriptorless"]
            if spec.role == "gfx_icon_asset_manifest_static"
            else None,
        )
        for spec in SB.DEFAULT_SPECS
    }
    assert R.catalog_compatibility(
        validated,
        [],
        ["dlc_product"],
        {"dlc_descriptorless"},
    ) == []
    blockers = R.catalog_compatibility(
        validated,
        [],
        ["dlc_product", "dlc_missing"],
        {"dlc_descriptorless"},
    )
    assert any("missing runtime active DLC ids" in blocker for blocker in blockers)


def test_descriptorless_asset_layers_are_classified_and_content_hashed(tmp_path: Path):
    vanilla = tmp_path / "the game"
    plain = vanilla / "dlc/dlc_plain"
    shadow = vanilla / "dlc/dlc_shadow"
    descriptor_backed = vanilla / "dlc/dlc_product"
    for path in (plain, shadow, descriptor_backed):
        path.mkdir(parents=True)
    (plain / "asset.txt").write_text(plain.name, encoding="utf-8")
    (descriptor_backed / "asset.txt").write_text(descriptor_backed.name, encoding="utf-8")
    (vanilla / "integrated_dlc/dlc_shadow").mkdir(parents=True)

    layers, inventory = R.classify_descriptorless_asset_layers(vanilla, {"dlc_product"})

    assert [row["id"] for row in layers] == ["dlc_plain", "dlc_shadow"]
    assert layers[0]["classification"] == "descriptorless_asset_layer"
    assert layers[1]["classification"] == "integrated_shadow_asset_layer"
    assert all(SB._valid_sha(row["content_sha256"]) for row in layers)
    assert {row["relpath"] for row in inventory} == {
        "dlc/dlc_plain/asset.txt",
    }
    assert not any("dlc_product" in row["relpath"] for row in inventory)


def test_runtime_log_must_match_process_creation_time(tmp_path: Path):
    runtime = {"first_time": "18:24:52"}
    process = {"created_utc": dt.datetime(2026, 7, 14, 8, 25, 10, tzinfo=dt.timezone.utc)}
    with pytest.raises(R.ResolutionError, match="does not match process creation"):
        R._runtime_log_matches_process(runtime, process)


def test_launcher_state_rejects_unknown_schema(tmp_path: Path):
    (tmp_path / "dlc_load.json").write_text(
        json.dumps({"enabled_mods": [], "disabled_dlcs": [], "invented": True}),
        encoding="utf-8",
    )
    with pytest.raises(R.ResolutionError, match="unsupported schema"):
        R._launcher_state(tmp_path)
