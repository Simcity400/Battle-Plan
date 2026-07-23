from __future__ import annotations

import datetime as dt
import json
import sys
import zipfile
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
    normalized = tmp_path / "normalized.dlc"
    normalized.write_text(
        'name="Normalized"\npath="dlc/normalized"\n'
        'replace_path="common//./units"\n',
        encoding="utf-8",
    )
    assert R.parse_descriptor(normalized)["replace_path"] == ["common/units"]
    unsafe = tmp_path / "unsafe.dlc"
    unsafe.write_text(
        'name="Unsafe"\npath="dlc/unsafe"\nreplace_path="../common"\n',
        encoding="utf-8",
    )
    with pytest.raises(R.ResolutionError, match="replace_path"):
        R.parse_descriptor(unsafe)


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


def test_source_inventory_expands_descriptor_backed_dlc_archive_namespace(
    tmp_path: Path,
):
    vanilla = tmp_path / "the game"
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
        zf.writestr("common/units/dlc.txt", b"dlc")
        zf.writestr("gfx/models/tank.asset", b"tank")

    inventory = R.recompute_source_inventory(
        vanilla,
        [],
        ["dlc003"],
        [],
        {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
    )
    namespace = next(
        row for row in inventory["namespaces"]
        if row["kind"] == "dlc" and row["id"] == "dlc003"
    )
    rows = [
        row for row in inventory["rows"]
        if row["namespace_ordinal"] == namespace["ordinal"]
    ]

    assert namespace["binding"] == "active_product"
    assert namespace["storage_kind"] == "zip_archive"
    assert namespace["source_container_sha256"] == SB.sha256_file(archive)
    assert namespace["source_container_bytes"] == archive.stat().st_size
    assert namespace["file_count"] == 2
    assert namespace["replace_paths"] == ["gfx/models"]
    assert namespace["binding_content_sha256"] != namespace["content_sha256"]
    assert {row["relpath"] for row in rows} == {
        "common/units/dlc.txt",
        "gfx/models/tank.asset",
    }
    assert all(row["layer_kind"] == "dlc" for row in rows)
    assert all(row["layer_id"] == "dlc003" for row in rows)
    archive_bytes = archive.read_bytes()
    for row in rows:
        start = row["source_byte_offset"]
        end = start + row["source_byte_count"]
        assert 0 <= start <= end <= len(archive_bytes)
        assert row["source_container_sha256"] == namespace["source_container_sha256"]
        assert row["source_range_sha256"] == SB.sha256_bytes(archive_bytes[start:end])
    assert not any(row["relpath"].endswith(".zip") for row in rows)
    assert R.validate_source_inventory_document(inventory) == inventory

    descriptor.write_text(
        'name="DLC Pack"\narchive="../escape.zip"\n',
        encoding="utf-8",
    )
    with pytest.raises(R.ResolutionError, match=r"unsafe descriptor-backed DLC archive path"):
        R.recompute_source_inventory(
            vanilla,
            [],
            ["dlc003"],
            [],
            {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
        )


def _write_loose_dlc(
    vanilla: Path,
    dlc_id: str,
    relpath: str,
    payload: str,
    *,
    replace_path: str | None = None,
    name: str | None = None,
    dependencies: list[str] | None = None,
) -> None:
    root = vanilla / "dlc" / dlc_id
    target = root / relpath
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(payload, encoding="utf-8")
    descriptor = f'name="{name or dlc_id}"\npath="dlc/{dlc_id}"\n'
    if replace_path is not None:
        descriptor += f'replace_path="{replace_path}"\n'
    if dependencies:
        descriptor += "dependencies={ " + " ".join(
            f'"{dependency}"' for dependency in dependencies
        ) + " }\n"
    (root / f"{dlc_id}.dlc").write_text(descriptor, encoding="utf-8")


def _inventory_winner(inventory: dict, relpath: str):
    parent = relpath.rsplit("/", 1)[0].casefold() if "/" in relpath else ""
    rows = {
        (row["namespace_ordinal"], row["relpath"].casefold()): row
        for row in inventory["rows"]
    }
    for namespace in reversed(inventory["namespaces"]):
        row = rows.get((namespace["ordinal"], relpath.casefold()))
        if row is not None:
            return namespace, row
        if any(path.casefold() == parent for path in namespace["replace_paths"]):
            return None
    return None


def test_source_inventory_uses_exact_descriptor_id_order_for_dlc_overlap(
    tmp_path: Path,
):
    vanilla = tmp_path / "the game"
    (vanilla / "common").mkdir(parents=True)
    (vanilla / "common/base.txt").write_text("base", encoding="utf-8")
    _write_loose_dlc(vanilla, "dlc_z", "common/shared.txt", "z")
    _write_loose_dlc(vanilla, "dlc_a", "common/shared.txt", "a")

    inventory = R.recompute_source_inventory(
        vanilla,
        [],
        ["dlc_z", "dlc_a"],
        [],
        {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
    )
    assert [row["id"] for row in inventory["namespaces"]] == [
        "vanilla", "dlc_a", "dlc_z"
    ]
    winner = _inventory_winner(inventory, "common/shared.txt")
    assert winner is not None and winner[0]["id"] == "dlc_z"


def test_source_inventory_applies_exact_later_dlc_replace_path(
    tmp_path: Path,
):
    vanilla = tmp_path / "the game"
    (vanilla / "common").mkdir(parents=True)
    (vanilla / "common/base.txt").write_text("base", encoding="utf-8")
    _write_loose_dlc(vanilla, "dlc_a", "common/shared.txt", "a")
    _write_loose_dlc(
        vanilla,
        "dlc_z",
        "gfx/unique.txt",
        "z",
        replace_path="common",
    )

    inventory = R.recompute_source_inventory(
        vanilla,
        [],
        ["dlc_z", "dlc_a"],
        [],
        {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
    )
    assert [row["id"] for row in inventory["namespaces"]] == [
        "vanilla", "dlc_a", "dlc_z"
    ]
    assert _inventory_winner(inventory, "common/shared.txt") is None


def test_dlc_dependency_lowers_named_layer_before_dependent(tmp_path: Path):
    vanilla = tmp_path / "the game"
    (vanilla / "common").mkdir(parents=True)
    (vanilla / "common/base.txt").write_text("base", encoding="utf-8")
    _write_loose_dlc(
        vanilla, "dlc_z", "common/shared.txt", "core", name="Core"
    )
    _write_loose_dlc(
        vanilla,
        "dlc_a",
        "common/shared.txt",
        "dependent",
        name="Dependent",
        dependencies=["Core"],
    )
    inventory = R.recompute_source_inventory(
        vanilla,
        [],
        ["dlc_a", "dlc_z"],
        [],
        {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
    )
    assert [row["id"] for row in inventory["namespaces"]] == [
        "vanilla", "dlc_z", "dlc_a"
    ]
    winner = _inventory_winner(inventory, "common/shared.txt")
    assert winner is not None and winner[0]["id"] == "dlc_a"


def test_integrated_dlc_uses_physical_descriptor_parent_and_selected_dlc_wins(
    tmp_path: Path,
):
    vanilla = tmp_path / "the game"
    (vanilla / "common").mkdir(parents=True)
    (vanilla / "common/base.txt").write_text("base", encoding="utf-8")
    integrated = vanilla / "integrated_dlc/dlc018_together_for_victory"
    (integrated / "common").mkdir(parents=True)
    (integrated / "dlc018.dlc").write_text(
        'name="Together for Victory"\npath="dlc/dlc018_together_for_victory"\n',
        encoding="utf-8",
    )
    (integrated / "common/shared.txt").write_text("physical-integrated", encoding="utf-8")
    stale = vanilla / "dlc/dlc018_together_for_victory/common"
    stale.mkdir(parents=True)
    (stale / "shared.txt").write_text("stale-path", encoding="utf-8")
    _write_loose_dlc(vanilla, "dlc_z", "common/shared.txt", "selected")

    inventory = R.recompute_source_inventory(
        vanilla,
        [],
        ["dlc_z"],
        [],
        {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
    )
    assert [row["id"] for row in inventory["namespaces"]] == [
        "vanilla", "dlc018_together_for_victory", "dlc_z"
    ]
    integrated_namespace = inventory["namespaces"][1]
    integrated_row = next(
        row for row in inventory["rows"]
        if row["namespace_ordinal"] == integrated_namespace["ordinal"]
        and row["relpath"] == "common/shared.txt"
    )
    assert integrated_row["sha256"] == SB.sha256_bytes(b"physical-integrated")
    winner = _inventory_winner(inventory, "common/shared.txt")
    assert winner is not None and winner[0]["id"] == "dlc_z"


def test_source_inventory_order_and_digest_survive_install_relocation(tmp_path: Path):
    inventories = []
    for machine in ("machine-z", "machine-a"):
        vanilla = tmp_path / machine / "the game"
        (vanilla / "common").mkdir(parents=True)
        (vanilla / "common/base.txt").write_text("base", encoding="utf-8")
        _write_loose_dlc(vanilla, "dlc_z", "common/z.txt", "z")
        _write_loose_dlc(vanilla, "dlc_a", "common/a.txt", "a")
        inventories.append(
            R.recompute_source_inventory(
                vanilla,
                [],
                ["dlc_z", "dlc_a"],
                [],
                {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
            )
        )
    assert inventories[0] == inventories[1]


def test_archive_mod_replace_path_inventory_survives_relocation(tmp_path: Path):
    inventories = []
    for machine in ("machine-z", "machine-a"):
        vanilla = tmp_path / machine / "the game"
        (vanilla / "common").mkdir(parents=True)
        (vanilla / "common/base.txt").write_text("base", encoding="utf-8")
        userdir = tmp_path / machine / "profile"
        descriptor = userdir / "mod/archive.mod"
        descriptor.parent.mkdir(parents=True)
        descriptor.write_text(
            'name="Archive Override"\narchive="mod/archive.zip"\nreplace_path="common"\n',
            encoding="utf-8",
        )
        archive = userdir / "mod/archive.zip"
        info = zipfile.ZipInfo("common/mod.txt", date_time=(2020, 1, 2, 3, 4, 6))
        info.compress_type = zipfile.ZIP_DEFLATED
        with zipfile.ZipFile(archive, "w") as zf:
            zf.writestr(info, b"archive override payload")
        content_sha, _rows, _container_sha, _container_bytes = R._zip_namespace_rows(
            archive, "mod/archive.mod"
        )
        inventories.append(
            R.recompute_source_inventory(
                vanilla,
                [
                    {
                        "id": "mod/archive.mod",
                        "ordinal": 0,
                        "name": "Archive Override",
                        "descriptor_path": str(descriptor),
                        "descriptor_sha256": R._hash_file_stable(descriptor)[0],
                        "content_kind": "archive",
                        "content_path": str(archive),
                        "content_sha256": content_sha,
                    }
                ],
                [],
                [],
                {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
            )
        )
    assert inventories[0] == inventories[1]
    assert _inventory_winner(inventories[0], "common/base.txt") is None
    winner = _inventory_winner(inventories[0], "common/mod.txt")
    assert winner is not None and winner[0]["storage_kind"] == "zip_archive"


def test_archive_member_dot_and_duplicate_separators_use_one_engine_key(tmp_path: Path):
    empty = tmp_path / "empty.zip"
    with zipfile.ZipFile(empty, "w"):
        pass
    with pytest.raises(R.ResolutionError, match=r"no regular files"):
        R._zip_namespace_rows(empty, "mod/empty.mod")

    archive = tmp_path / "normalized.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("common//./nested/value.txt", b"canonical payload")
    _digest, rows, _container_sha, _container_bytes = R._zip_namespace_rows(
        archive, "mod/normalized.mod"
    )
    assert [row["relpath"] for row in rows] == ["common/nested/value.txt"]

    collision = tmp_path / "collision.zip"
    with zipfile.ZipFile(collision, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("common//./nested/value.txt", b"first")
        zf.writestr("COMMON/nested/value.txt", b"second")
    with pytest.raises(R.ResolutionError, match=r"case-fold/duplicate"):
        R._zip_namespace_rows(collision, "mod/collision.mod")


def test_ui14_collision_join_covers_archive_members_and_loose_mods(tmp_path: Path):
    archive = tmp_path / "archive.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("common/shared.txt", b"archive winner")
    _digest, archive_rows, _container_sha, _container_bytes = R._zip_namespace_rows(
        archive, "mod/archive.mod"
    )
    loose = tmp_path / "loose"
    (loose / "common").mkdir(parents=True)
    (loose / "common/shared.txt").write_bytes(b"loose loser")
    _loose_sha, loose_rows = R.hash_tree(
        loose, layer_kind="mod", layer_id="mod/loose.mod"
    )
    winner = {
        "relpath": "COMMON\\SHARED.TXT",
        "winner_id": "mod/archive.mod",
        "sha256": archive_rows[0]["sha256"],
    }

    R.validate_ui14_mod_collision_winners(
        {
            "mod/loose.mod": loose_rows,
            "mod/archive.mod": archive_rows,
        },
        [winner],
    )
    with pytest.raises(R.ResolutionError, match=r"effective winner mismatch"):
        R.validate_ui14_mod_collision_winners(
            {
                "mod/loose.mod": loose_rows,
                "mod/archive.mod": archive_rows,
            },
            [{**winner, "sha256": "0" * 64}],
        )
    with pytest.raises(R.ResolutionError, match=r"exactly cover mod collisions"):
        R.validate_ui14_mod_collision_winners(
            {
                "mod/loose.mod": loose_rows,
                "mod/archive.mod": archive_rows,
            },
            [],
        )
    with pytest.raises(R.ResolutionError, match=r"winner row 0 is malformed"):
        R.validate_ui14_mod_collision_winners(
            {
                "mod/loose.mod": loose_rows,
                "mod/archive.mod": archive_rows,
            },
            [{**winner, "unexpected": True}],
        )
    with pytest.raises(R.ResolutionError, match=r"winner row 1 identity is invalid"):
        R.validate_ui14_mod_collision_winners(
            {
                "mod/loose.mod": loose_rows,
                "mod/archive.mod": archive_rows,
            },
            [winner, {**winner, "relpath": "common/shared.txt"}],
        )


def test_engine_mount_plan_rejects_dependency_cycle_instead_of_hanging():
    rows = [
        {
            "id": "dlc_a",
            "descriptor_id": "dlc/dlc_a/a.dlc",
            "name": "A",
            "dependencies": ["B"],
        },
        {
            "id": "dlc_b",
            "descriptor_id": "dlc/dlc_b/b.dlc",
            "name": "B",
            "dependencies": ["A"],
        },
    ]
    with pytest.raises(R.ResolutionError, match="priority cycle"):
        R._engine_mount_plan(rows, [])


def test_reversed_lexical_dlc_mod_overlap_keeps_authenticated_mod_winner(
    tmp_path: Path,
):
    vanilla = tmp_path / "the game"
    (vanilla / "common").mkdir(parents=True)
    (vanilla / "common/base.txt").write_text("base", encoding="utf-8")
    _write_loose_dlc(vanilla, "dlc_z", "common/shared.txt", "z")
    _write_loose_dlc(vanilla, "dlc_a", "common/other.txt", "a")

    userdir = tmp_path / "profile"
    content = userdir / "mod" / "override"
    (content / "common").mkdir(parents=True)
    (content / "common/shared.txt").write_text("mod", encoding="utf-8")
    descriptor = userdir / "mod" / "override.mod"
    descriptor.write_text(
        'name="Override"\npath="mod/override"\n', encoding="utf-8"
    )
    content_sha, _ = R.hash_tree(content, layer_kind="mod", layer_id="mod/override.mod")
    inventory = R.recompute_source_inventory(
        vanilla,
        [
            {
                "id": "mod/override.mod",
                "ordinal": 0,
                "name": "Override",
                "descriptor_path": str(descriptor),
                "descriptor_sha256": R._hash_file_stable(descriptor)[0],
                "content_kind": "directory",
                "content_path": str(content),
                "content_sha256": content_sha,
            }
        ],
        ["dlc_z", "dlc_a"],
        [],
        {"playset_id": "fixture-playset", "profile_id": "fixture-profile"},
    )

    namespaces = {row["ordinal"]: row for row in inventory["namespaces"]}
    shared = [
        row
        for row in inventory["rows"]
        if row["relpath"].casefold() == "common/shared.txt"
    ]
    winner = max(shared, key=lambda row: row["namespace_ordinal"])
    assert namespaces[winner["namespace_ordinal"]]["id"] == "mod/override.mod"
    assert [row["id"] for row in inventory["namespaces"]] == [
        "vanilla",
        "dlc_a",
        "dlc_z",
        "mod/override.mod",
    ]


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


def test_launcher_disabled_dlc013_is_recorded_but_runtime_active_remains_effective():
    effective_names, effective_ids, requested_ids, conflicts = (
        R._runtime_and_requested_dlc_selection(
            ["Sabaton Soundtrack", "Allied Speeches Music Pack"],
            {
                "Sabaton Soundtrack": "dlc013_sabaton",
                "Allied Speeches Music Pack": "dlc031_allied_speeches_pack",
            },
            ["dlc/dlc013_sabaton/dlc013.dlc"],
        )
    )

    assert effective_names == ["Sabaton Soundtrack", "Allied Speeches Music Pack"]
    assert effective_ids == ["dlc013_sabaton", "dlc031_allied_speeches_pack"]
    assert requested_ids == ["dlc031_allied_speeches_pack"]
    assert conflicts == ["dlc013_sabaton"]


def test_requested_profile_rejects_unknown_disabled_descriptor_layer():
    with pytest.raises(R.ResolutionError, match="no installed layer"):
        R._runtime_and_requested_dlc_selection(
            ["Known"],
            {"Known": "dlc001_known"},
            ["dlc/dlc999_missing/missing.dlc"],
        )


def test_effective_runtime_rejects_duplicate_display_name():
    with pytest.raises(R.ResolutionError, match="duplicates"):
        R._runtime_and_requested_dlc_selection(
            ["Known", "Known"],
            {"Known": "dlc001_known"},
            [],
        )
