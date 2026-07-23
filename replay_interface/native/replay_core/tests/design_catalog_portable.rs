use replay_core::{
    open_recording, DesignCatalogJoinError, DesignKind, FieldCompleteness, WorldTimeline,
    A27_DESIGN_CATALOG_ARTIFACT_PATH, A27_LIVE_CATALOG_SCHEMA,
};
use serde_json::json;
use snapshot::{pack, PackEntry, PackIdentity};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const GAME: &str = "a27_portable_catalog";

fn temp_file(tag: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "replay_core_{tag}_{}_{serial}_{nanos}.battleplan",
        std::process::id()
    ))
}

fn catalog() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema": A27_LIVE_CATALOG_SCHEMA,
        "authority": {
            "program": "hoi4.exe 1.19.2.0.a729",
            "program_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
            "current_date_raw": 43_800_899,
            "capture_hour_index": 900,
            "coherence": "double_read_all_roots_fields_and_membership"
        },
        "full_replacement": true,
        "completeness": {
            "definitions": true, "lifecycle": true, "membership": true,
            "owner": true, "ui_occurrences": true
        },
        "country_count": 1,
        "countries": [{"registry_index": 0, "tag": "GER"}],
        "division_templates": [{
            "catalog_scan_ordinal": 0, "country_vector_ordinal": 0,
            "definition": {
                "name": "Infantry", "name_utf8_hex": "496e66616e747279",
                "regiment_grid_columns": 5, "regiment_grid_rows": 5,
                "regiment_pointer_count": 0, "regiment_placeholder_ordinals": [],
                "regiments": [], "support_pointer_count": 0,
                "support_placeholder_ordinals": [], "support": [],
                "regimental_support_pointer_count": 0,
                "regimental_support_placeholder_ordinals": [],
                "regimental_support": [], "priority": 0,
                "lock_regime_snapshot_raw": 0, "is_locked": false,
                "obsolete": false
            },
            "definition_complete": true, "id": 8, "obsolete": false,
            "owner_tag": "GER", "source_country_registry_index": 0,
            "source_country_tag": "GER", "type": 52
        }],
        "variants": [
            {
                "catalog_scan_ordinal": 0, "creator_tag": "GER",
                "definition": {
                    "archetype_token": "light_tank_chassis_1",
                    "archetype_token_utf8_hex": "6c696768745f74616e6b5f636861737369735f31",
                    "cached_stats": [], "cached_stats_complete": true,
                    "display_name": "Seven", "display_name_utf8_hex": "536576656e",
                    "equipment_token_raw": 71, "modules": [], "modules_complete": true,
                    "upgrades": [], "upgrades_complete": true, "variant_name": null,
                    "variant_name_utf8_hex": ""
                },
                "definition_complete": true, "first_ui_discovery_ordinal": null,
                "id": 7, "obsolete": false, "origin_override_tag": null,
                "owner_tag": "GER", "selected_country_tag": "GER",
                "source_occurrences": [], "type": 70
            },
            {
                "catalog_scan_ordinal": 1, "creator_tag": "GER",
                "definition": {
                    "archetype_token": "light_tank_chassis_1",
                    "archetype_token_utf8_hex": "6c696768745f74616e6b5f636861737369735f31",
                    "cached_stats": [], "cached_stats_complete": true,
                    "display_name": "Fifty Five", "display_name_utf8_hex": "46696674792046697665",
                    "equipment_token_raw": 71, "modules": [], "modules_complete": true,
                    "upgrades": [], "upgrades_complete": true, "variant_name": null,
                    "variant_name_utf8_hex": ""
                },
                "definition_complete": true, "first_ui_discovery_ordinal": null,
                "id": 55, "obsolete": true, "origin_override_tag": null,
                "owner_tag": "GER", "selected_country_tag": "GER",
                "source_occurrences": [], "type": 70
            }
        ]
    }))
    .unwrap()
}

fn snapshot(ids: &[u32]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": "1", "game_id": GAME, "hour_index": 900,
        "_r2_design_catalog_context_live": {
            "schema_version": 1, "authority": "exact_a729_designer_view",
            "elapsed_wall_ms": 7, "frame_hour": 900,
            "completeness_record_identity": "a27:a729:transition:5",
            "transition_sequence": 5, "active": true, "mode": 1,
            "family": "tank_designer_view", "our_filter_enabled": true,
            "foreign_filter_enabled": false, "obsolete_filter_enabled": true,
            "current_country_tag_id": 17, "selected_country_tag_ids": [17],
            "ordered_design_ids": ids
        }
    }))
    .unwrap()
}

fn write_replay(path: &PathBuf, ids: &[u32], catalog_bytes: Vec<u8>) {
    let index = serde_json::to_vec(&json!({
        "game_id": GAME, "schema_version": "1",
        "snapshots": [{"hour_index": 900, "file": "000000000900.snapshot.json"}]
    }))
    .unwrap();
    let entries = vec![
        PackEntry {
            path: format!("payload/{GAME}/index.json"),
            bytes: index,
            role: "index",
            hour_index: None,
        },
        PackEntry {
            path: format!("payload/{GAME}/000000000900.snapshot.json"),
            bytes: snapshot(ids),
            role: "snapshot",
            hour_index: Some(900),
        },
        PackEntry {
            path: format!("payload/{GAME}/{A27_DESIGN_CATALOG_ARTIFACT_PATH}"),
            bytes: catalog_bytes,
            role: "static_artifact",
            hour_index: None,
        },
    ];
    fs::write(
        path,
        pack(GAME, &entries, &PackIdentity::default(), false).unwrap(),
    )
    .unwrap();
}

#[test]
fn portable_artifact_roundtrips_and_d78_is_the_only_display_order() {
    let path = temp_file("design_catalog_roundtrip");
    write_replay(&path, &[55, 7], catalog());
    let timeline = WorldTimeline::open(open_recording(&path).unwrap()).unwrap();
    assert_eq!(timeline.design_catalog().unwrap().records.len(), 3);
    let frame = timeline.frame(900).unwrap();
    let current = frame.current_design_catalog().unwrap();
    assert_eq!(
        current.display_order.completeness,
        FieldCompleteness::Complete
    );
    assert_eq!(current.display_order.ordered_design_ids, ["55:70", "7:70"]);
    assert_eq!(current.records[0].design_id, "55:70");
    assert_eq!(current.records[0].kind, DesignKind::EquipmentVariant);
    assert_eq!(current.records[1].design_id, "7:70");
    drop(current);
    drop(frame);
    drop(timeline);
    fs::remove_file(path).unwrap();
}

#[test]
fn portable_artifact_and_d78_join_fail_closed() {
    let unknown = temp_file("design_catalog_unknown");
    write_replay(&unknown, &[999], catalog());
    let timeline = WorldTimeline::open(open_recording(&unknown).unwrap()).unwrap();
    assert_eq!(
        timeline.frame(900).unwrap().current_design_catalog(),
        Err(DesignCatalogJoinError::UnknownContextId(999))
    );
    drop(timeline);
    fs::remove_file(unknown).unwrap();

    let partial = temp_file("design_catalog_partial");
    let mut value: serde_json::Value = serde_json::from_slice(&catalog()).unwrap();
    value["completeness"]["owner"] = false.into();
    write_replay(&partial, &[7], serde_json::to_vec(&value).unwrap());
    assert!(open_recording(&partial).is_err());
    fs::remove_file(partial).unwrap();

    let wrong_hour = temp_file("design_catalog_wrong_hour");
    let mut value: serde_json::Value = serde_json::from_slice(&catalog()).unwrap();
    value["authority"]["current_date_raw"] = 43_800_900_u64.into();
    value["authority"]["capture_hour_index"] = 901_u64.into();
    write_replay(
        &wrong_hour,
        &[7],
        serde_json::to_vec(&value).unwrap(),
    );
    assert!(open_recording(&wrong_hour).is_err());
    fs::remove_file(wrong_hour).unwrap();
}
