import copy

from validate_live_producer import PROGRAM_SHA256, SCHEMA, STAT_FIELDS, validate


def _stats():
    return [
        {"source_ordinal": ordinal, "stat_id": name, "raw_i32": ordinal, "scale": 1000}
        for ordinal, name in enumerate(sorted(STAT_FIELDS))
    ]


def _variant(ident, owner, ordinal, pointer, occurrence=True):
    modules = [] if ident == 2 else [{
        "source_ordinal": 0, "slot_handle": 101, "module_definition_handle": 201,
        "module_token": "engine_1", "module_token_utf8_hex": "656e67696e655f31",
    }]
    upgrades = [] if ident == 2 else [{
        "source_ordinal": 0, "upgrade_handle": 301, "level": 2, "level_raw_u64": 2,
    }]
    return {
        "id": ident, "type": 70, "owner_tag": owner, "creator_tag": owner,
        "origin_override_tag": None, "selected_country_tag": owner, "obsolete": ident == 2,
        "definition_complete": True, "catalog_scan_ordinal": ordinal,
        "first_ui_discovery_ordinal": 0 if occurrence else None,
        "pointer_identity": pointer,
        "source_occurrences": ([{
            "source_country_tag": owner, "source_country_registry_index": ordinal,
            "source_list": "A", "source_list_ordinal": 0, "pointer_identity": pointer,
        }] if occurrence else []),
        "definition": {
            "equipment_definition_pointer": "0x3000", "equipment_token_raw": ident,
            "archetype_token": "fighter_equipment", "archetype_token_utf8_hex": "66",
            "display_name": "Fighter" if ident == 1 else "Zerstörer",
            "display_name_utf8_hex": "66", "variant_name": None, "variant_name_utf8_hex": "",
            "modules_complete": True, "modules": modules,
            "upgrades_complete": True, "upgrades": upgrades,
            "cached_stats_complete": True, "cached_stats": _stats(),
        },
    }


def _template(ident, ordinal, obsolete=False):
    return {
        "id": ident, "type": 52, "owner_tag": "GER", "source_country_tag": "GER",
        "source_country_registry_index": 0, "country_vector_ordinal": ordinal,
        "obsolete": obsolete, "definition_complete": True, "catalog_scan_ordinal": ordinal,
        "pointer_identity": f"0x{4000 + ident:x}",
        "definition": {
            "name": "Infantry", "name_utf8_hex": "49", "regiment_grid_columns": 5,
            "regiment_grid_rows": 5, "regiment_pointer_count": 1,
            "regiment_placeholder_ordinals": [],
            "regiments": [{"source_ordinal": 0, "x": 0, "y": 0, "subunit_definition_pointer": "0x5000", "subunit_definition_handle": 1, "type": "infantry", "type_utf8_hex": "69"}],
            "support_pointer_count": 0, "support_placeholder_ordinals": [], "support": [],
            "regimental_support_pointer_count": 0, "regimental_support_placeholder_ordinals": [], "regimental_support": [],
            "priority": 1, "lock_regime_snapshot_raw": 0, "is_locked": False, "obsolete": obsolete,
        },
    }


def fixture():
    oracle = {
        "equipment_variants": {
            "1:70": {"id": 1, "type": 70, "creator": "GER", "origin": "---", "obsolete": False, "archetype": "fighter_equipment", "name": "Fighter", "modules": {"engine": "engine_1"}, "upgrades": {"gun": 2}},
            "2:70": {"id": 2, "type": 70, "creator": "ITA", "origin": "---", "obsolete": True, "archetype": "fighter_equipment", "name": "ZerstÃ¶rer", "modules": {}, "upgrades": {}},
        },
        "division_templates": [
            {"id": 10, "type": 52, "country": "GER", "obsolete": False, "raw_name": "Infantry", "priority": 1, "is_locked": False, "regiments": [{"type": "infantry", "x": 0, "y": 0}], "support": [], "regimental_support": []},
            {"id": 11, "type": 52, "country": "GER", "obsolete": True, "raw_name": "Infantry", "priority": 1, "is_locked": False, "regiments": [{"type": "infantry", "x": 0, "y": 0}], "support": [], "regimental_support": []},
        ],
    }
    snapshot = {
        "schema": SCHEMA,
        "authority": {"program": "hoi4.exe 1.19.2.0.a729", "program_sha256": PROGRAM_SHA256},
        "full_replacement": True,
        "completeness": {"membership": True, "owner": True, "definitions": True, "lifecycle": True, "ui_occurrences": True},
        "variants": [_variant(1, "GER", 0, "0x1000"), _variant(2, "ITA", 1, "0x2000", occurrence=False)],
        "division_templates": [_template(10, 0), _template(11, 1, obsolete=True)],
        "ui_occurrence_counts": {"unique_variants": 1, "catalog_variants_without_occurrence": 1},
    }
    return oracle, snapshot


def test_exact_snapshot_with_variant_absent_from_ab_passes():
    oracle, snapshot = fixture()
    assert validate(snapshot, oracle) == []


def test_badge_only_definition_fails():
    oracle, snapshot = fixture()
    del snapshot["variants"][0]["definition"]
    errors = validate(snapshot, oracle)
    assert any("definition payload missing" in error for error in errors)


def test_catalog_and_ui_ordinals_are_independent_and_fail_closed():
    oracle, snapshot = fixture()
    snapshot["variants"][1]["catalog_scan_ordinal"] = 0
    snapshot["variants"][1]["first_ui_discovery_ordinal"] = 1
    errors = validate(snapshot, oracle)
    assert any("catalog scan ordinals" in error for error in errors)
    assert any("UI ordinal exists without occurrence" in error for error in errors)


def test_module_and_template_payload_mismatch_fail():
    oracle, snapshot = fixture()
    snapshot = copy.deepcopy(snapshot)
    snapshot["variants"][0]["definition"]["modules"][0]["module_token"] = "wrong"
    snapshot["division_templates"][0]["definition"]["regiments"][0]["x"] = 1
    errors = validate(snapshot, oracle)
    assert any("module token multiset mismatch" in error for error in errors)
    assert any("regiments mismatch" in error for error in errors)


def test_native_utf8_matches_mojibake_oracle_semantics():
    oracle, snapshot = fixture()
    assert snapshot["variants"][1]["definition"]["display_name"] == "Zerstörer"
    assert validate(snapshot, oracle) == []
