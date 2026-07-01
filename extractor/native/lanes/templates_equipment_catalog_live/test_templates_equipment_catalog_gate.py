from pathlib import Path

import templates_equipment_catalog_gate as gate


def test_division_template_grid_and_support_parse(tmp_path: Path):
    p = tmp_path / "GER_1936.txt"
    p.write_text(
        """
division_template = {
  name = "Infanterie-Division"
  regiments = {
    infantry = { x = 0 y = 0 }
    artillery_brigade = { x = 1 y = 0 }
  }
  support = {
    engineer = { x = 0 y = 0 }
  }
}
""",
        encoding="utf-8",
    )

    rows = gate.collect_division_templates({"history/units/GER_1936.txt": p})

    assert rows[0]["name"] == "Infanterie-Division"
    assert rows[0]["regiment_count"] == 2
    assert rows[0]["support_count"] == 1
    assert rows[0]["regiments"][0]["unit_type"] == "infantry"
    assert rows[0]["support"][0]["unit_type"] == "engineer"


def test_top_level_equipment_blocks_are_deterministic(tmp_path: Path):
    p = tmp_path / "infantry.txt"
    p.write_text(
        """
infantry_equipment = {
  year = 1936
  reliability = 0.9
}
support_equipment = {
  year = 1936
}
""",
        encoding="utf-8",
    )

    blocks = gate.collect_top_level_blocks({"common/units/equipment/infantry.txt": p})

    assert list(blocks) == ["infantry_equipment", "support_equipment"]
    assert blocks["infantry_equipment"]["fields"]["year"] == "1936"
    assert blocks["infantry_equipment"]["source_sha256"] == gate.sha256_bytes(p.read_bytes())


def make_vanilla_root(root: Path) -> Path:
    vanilla = root / "the game"
    (vanilla / "common").mkdir(parents=True)
    (vanilla / "history").mkdir()
    (vanilla / "hoi4.exe").write_bytes(b"exe")
    return vanilla


def test_discover_vanilla_root_from_running_process_path(tmp_path: Path):
    vanilla = make_vanilla_root(tmp_path)

    found, report = gate.discover_vanilla_root(
        None,
        env={},
        process_paths=[vanilla / "hoi4.exe"],
        steam_library_files=[],
        fallback=tmp_path / "missing",
    )

    assert found == vanilla.resolve()
    assert report["checked"][0]["source"] == "running hoi4.exe process"
    assert report["checked"][0]["valid"] is True


def test_discover_vanilla_root_from_steam_appmanifest(tmp_path: Path):
    library = tmp_path / "SteamLibrary"
    vanilla = make_vanilla_root(library / "steamapps" / "common")
    libraryfolders = tmp_path / "libraryfolders.vdf"
    escaped_library = str(library).replace("\\", "\\\\")
    libraryfolders.write_text(
        f'''"libraryfolders"
{{
  "0"
  {{
    "path" "{escaped_library}"
    "apps"
    {{
      "394360" "100"
    }}
  }}
}}
''',
        encoding="utf-8",
    )
    (library / "steamapps").mkdir(exist_ok=True)
    (library / "steamapps" / "appmanifest_394360.acf").write_text(
        '"AppState"\n{\n  "appid" "394360"\n  "installdir" "the game"\n}\n',
        encoding="utf-8",
    )

    found, report = gate.discover_vanilla_root(
        None,
        env={},
        process_paths=[],
        steam_library_files=[libraryfolders],
        fallback=tmp_path / "missing",
    )

    assert found == vanilla.resolve()
    assert report["checked"][0]["source"] == "Steam appmanifest_394360.acf"
    assert report["checked"][0]["valid"] is True
