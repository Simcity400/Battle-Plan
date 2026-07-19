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
        "full_replacement": true,
        "completeness": {
            "definitions": true, "lifecycle": true, "membership": true,
            "owner": true, "ui_occurrences": true
        },
        "division_templates": [{
            "catalog_scan_ordinal": 0, "definition": {"name": "Infantry"},
            "definition_complete": true, "id": 8, "obsolete": false,
            "owner_tag": "GER", "type": 52
        }],
        "variants": [
            {
                "catalog_scan_ordinal": 0,
                "definition": {
                    "archetype_token": "light_tank_chassis_1",
                    "cached_stats": [], "cached_stats_complete": true,
                    "display_name": "Seven", "modules": [], "modules_complete": true,
                    "upgrades": [], "upgrades_complete": true, "variant_name": null
                },
                "definition_complete": true, "id": 7, "obsolete": false,
                "owner_tag": "GER", "type": 70
            },
            {
                "catalog_scan_ordinal": 1,
                "definition": {
                    "archetype_token": "light_tank_chassis_1",
                    "cached_stats": [], "cached_stats_complete": true,
                    "display_name": "Fifty Five", "modules": [], "modules_complete": true,
                    "upgrades": [], "upgrades_complete": true, "variant_name": null
                },
                "definition_complete": true, "id": 55, "obsolete": true,
                "owner_tag": "GER", "type": 70
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
    fs::remove_file(unknown).unwrap();

    let partial = temp_file("design_catalog_partial");
    let mut value: serde_json::Value = serde_json::from_slice(&catalog()).unwrap();
    value["completeness"]["owner"] = false.into();
    write_replay(&partial, &[7], serde_json::to_vec(&value).unwrap());
    assert!(open_recording(&partial).is_err());
    fs::remove_file(partial).unwrap();
}
