use game_ui::design_catalog::{
    admit_design_catalog, admit_observer_design_catalogue, DesignCatalogRejection, DesignKind,
    DesignLifecycleState, DesignModule, DesignRecord, DesignStat, DesignUpgrade,
    DisplayOrderSnapshot, FieldCompleteness,
};
use game_ui::readonly_design_peace::DesignerViewKind;
use replay_core::{
    open_recording, WorldTimeline, A27_DESIGN_CATALOG_ARTIFACT_PATH, A27_LIVE_CATALOG_SCHEMA,
};
use serde_json::json;
use snapshot::{pack, PackEntry, PackIdentity};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn record(id: &str, kind: DesignKind) -> DesignRecord {
    DesignRecord {
        design_id: id.into(),
        source_record_id: format!("fixture:{id}"),
        kind,
        owner: Some("GER".into()),
        name: Some(format!("name:{id}")),
        archetype: Some(format!("archetype:{id}")),
        lifecycle_state: Some(DesignLifecycleState::Active),
        modules: vec![DesignModule {
            slot_id: "slot_1".into(),
            module_token: "module_1".into(),
            source_record_id: format!("fixture:{id}:module"),
        }],
        stats: vec![DesignStat {
            stat_id: "build_cost_ic".into(),
            value: 5.5,
            source_record_id: format!("fixture:{id}:stat"),
        }],
        upgrades: vec![DesignUpgrade {
            upgrade_id: "engine".into(),
            level: 2,
            source_record_id: format!("fixture:{id}:upgrade"),
        }],
        definition_completeness: FieldCompleteness::Complete,
        owner_completeness: FieldCompleteness::Complete,
        lifecycle_completeness: FieldCompleteness::Complete,
        module_completeness: FieldCompleteness::Complete,
        stat_completeness: FieldCompleteness::Complete,
        upgrade_completeness: FieldCompleteness::Complete,
    }
}

fn order(ids: &[&str]) -> DisplayOrderSnapshot {
    DisplayOrderSnapshot {
        source_record_id: "fixture:display_order".into(),
        completeness: FieldCompleteness::Complete,
        ordered_design_ids: ids.iter().map(|value| (*value).into()).collect(),
    }
}

#[test]
fn all_five_design_families_preserve_exact_producer_order() {
    let records = vec![
        record("division", DesignKind::DivisionTemplate),
        record("ship", DesignKind::ShipVariant),
        record("tank", DesignKind::TankVariant),
        record("aircraft", DesignKind::AircraftVariant),
        record("equipment", DesignKind::EquipmentVariant),
    ];
    let admitted = admit_design_catalog(
        &records,
        &order(&["tank", "division", "aircraft", "ship", "equipment"]),
    )
    .unwrap();
    let ids = admitted
        .ordered()
        .iter()
        .map(|row| row.design_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["tank", "division", "aircraft", "ship", "equipment"]);
    assert_eq!(admitted.get("ship").unwrap().kind, DesignKind::ShipVariant);
}

#[test]
fn unknown_field_completeness_never_promotes() {
    let mut row = record("tank", DesignKind::TankVariant);
    row.stat_completeness = FieldCompleteness::Unknown;
    assert_eq!(
        admit_design_catalog(&[row], &order(&["tank"])),
        Err(DesignCatalogRejection::UnknownCompleteness)
    );
}

#[test]
fn display_order_must_be_an_exact_permutation() {
    let records = vec![
        record("ship", DesignKind::ShipVariant),
        record("aircraft", DesignKind::AircraftVariant),
    ];
    assert_eq!(
        admit_design_catalog(&records, &order(&["ship"])),
        Err(DesignCatalogRejection::DisplayOrderMissingId)
    );
    assert_eq!(
        admit_design_catalog(&records, &order(&["ship", "unknown"])),
        Err(DesignCatalogRejection::DisplayOrderUnknownId)
    );
    assert_eq!(
        admit_design_catalog(&records, &order(&["ship", "ship"])),
        Err(DesignCatalogRejection::DuplicateDisplayOrderId)
    );
}

#[test]
fn modules_stats_and_upgrades_fail_closed() {
    let mut duplicate_slot = record("ship", DesignKind::ShipVariant);
    duplicate_slot
        .modules
        .push(duplicate_slot.modules[0].clone());
    assert_eq!(
        admit_design_catalog(&[duplicate_slot], &order(&["ship"])),
        Err(DesignCatalogRejection::DuplicateModuleSlot)
    );

    let mut bad_stat = record("aircraft", DesignKind::AircraftVariant);
    bad_stat.stats[0].value = f64::NAN;
    assert_eq!(
        admit_design_catalog(&[bad_stat], &order(&["aircraft"])),
        Err(DesignCatalogRejection::InvalidStat)
    );

    let mut duplicate_upgrade = record("tank", DesignKind::TankVariant);
    duplicate_upgrade
        .upgrades
        .push(duplicate_upgrade.upgrades[0].clone());
    assert_eq!(
        admit_design_catalog(&[duplicate_upgrade], &order(&["tank"])),
        Err(DesignCatalogRejection::DuplicateUpgrade)
    );
}

#[test]
fn not_applicable_is_explicit_but_unknown_is_not() {
    let mut division = record("division", DesignKind::DivisionTemplate);
    division.upgrades.clear();
    division.upgrade_completeness = FieldCompleteness::NotApplicable;
    assert!(admit_design_catalog(&[division], &order(&["division"])).is_ok());
}

#[test]
fn provenance_identity_and_duplicate_ids_are_rejected() {
    let mut missing_provenance = record("equipment", DesignKind::EquipmentVariant);
    missing_provenance.source_record_id.clear();
    assert_eq!(
        admit_design_catalog(&[missing_provenance], &order(&["equipment"])),
        Err(DesignCatalogRejection::MissingProvenance)
    );

    let mut missing_identity = record("equipment", DesignKind::EquipmentVariant);
    missing_identity.name = None;
    missing_identity.archetype = None;
    assert_eq!(
        admit_design_catalog(&[missing_identity], &order(&["equipment"])),
        Err(DesignCatalogRejection::MissingDefinitionIdentity)
    );

    let duplicate = record("equipment", DesignKind::EquipmentVariant);
    assert_eq!(
        admit_design_catalog(&[duplicate.clone(), duplicate], &order(&["equipment"])),
        Err(DesignCatalogRejection::DuplicateDesignId)
    );
}

#[test]
fn owner_lifecycle_and_display_order_completeness_fail_closed() {
    let mut missing_owner = record("tank", DesignKind::TankVariant);
    missing_owner.owner = None;
    assert_eq!(
        admit_design_catalog(&[missing_owner], &order(&["tank"])),
        Err(DesignCatalogRejection::MissingOwner)
    );

    let mut missing_lifecycle = record("ship", DesignKind::ShipVariant);
    missing_lifecycle.lifecycle_state = None;
    assert_eq!(
        admit_design_catalog(&[missing_lifecycle], &order(&["ship"])),
        Err(DesignCatalogRejection::MissingLifecycleState)
    );

    let row = record("aircraft", DesignKind::AircraftVariant);
    let mut incomplete_order = order(&["aircraft"]);
    incomplete_order.completeness = FieldCompleteness::Unknown;
    assert_eq!(
        admit_design_catalog(&[row], &incomplete_order),
        Err(DesignCatalogRejection::UnknownCompleteness)
    );
}

struct ObserverTestTempDir {
    path: PathBuf,
}

impl ObserverTestTempDir {
    fn new(tag: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(observer_temp_name(tag));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for ObserverTestTempDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound && !std::thread::panicking() {
                panic!("remove observer catalogue test temp directory: {error}");
            }
        }
    }
}

fn observer_temp_name(tag: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "game_ui_observer_catalogue_{tag}_{}_{serial}_{nanos}",
        std::process::id()
    )
}

fn observer_catalogue_bytes() -> Vec<u8> {
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
            "definitions": true,
            "lifecycle": true,
            "membership": true,
            "owner": true,
            "ui_occurrences": true
        },
        "country_count": 1,
        "countries": [{"registry_index": 0, "tag": "GER"}],
        "division_templates": [{
            "catalog_scan_ordinal": 0,
            "country_vector_ordinal": 0,
            "definition": {
                "name": "Infantry",
                "name_utf8_hex": "496e66616e747279",
                "regiment_grid_columns": 5,
                "regiment_grid_rows": 5,
                "regiment_pointer_count": 0,
                "regiment_placeholder_ordinals": [],
                "regiments": [],
                "support_pointer_count": 0,
                "support_placeholder_ordinals": [],
                "support": [],
                "regimental_support_pointer_count": 0,
                "regimental_support_placeholder_ordinals": [],
                "regimental_support": [],
                "priority": 0,
                "lock_regime_snapshot_raw": 0,
                "is_locked": false,
                "obsolete": false
            },
            "definition_complete": true,
            "id": 8,
            "obsolete": false,
            "owner_tag": "GER",
            "source_country_registry_index": 0,
            "source_country_tag": "GER",
            "type": 52
        }],
        "variants": [
            {
                "catalog_scan_ordinal": 0,
                "creator_tag": "GER",
                "definition": {
                    "archetype_token": "light_tank_chassis_1",
                    "archetype_token_utf8_hex": "6c696768745f74616e6b5f636861737369735f31",
                    "cached_stats": [],
                    "cached_stats_complete": true,
                    "display_name": "Seven",
                    "display_name_utf8_hex": "536576656e",
                    "equipment_token_raw": 71,
                    "modules": [],
                    "modules_complete": true,
                    "upgrades": [],
                    "upgrades_complete": true,
                    "variant_name": null,
                    "variant_name_utf8_hex": ""
                },
                "definition_complete": true,
                "first_ui_discovery_ordinal": null,
                "id": 7,
                "obsolete": false,
                "origin_override_tag": null,
                "owner_tag": "GER",
                "selected_country_tag": "GER",
                "source_occurrences": [],
                "type": 70
            },
            {
                "catalog_scan_ordinal": 1,
                "creator_tag": "GER",
                "definition": {
                    "archetype_token": "light_tank_chassis_1",
                    "archetype_token_utf8_hex": "6c696768745f74616e6b5f636861737369735f31",
                    "cached_stats": [],
                    "cached_stats_complete": true,
                    "display_name": "Fifty Five",
                    "display_name_utf8_hex": "46696674792046697665",
                    "equipment_token_raw": 71,
                    "modules": [],
                    "modules_complete": true,
                    "upgrades": [],
                    "upgrades_complete": true,
                    "variant_name": null,
                    "variant_name_utf8_hex": ""
                },
                "definition_complete": true,
                "first_ui_discovery_ordinal": null,
                "id": 55,
                "obsolete": true,
                "origin_override_tag": null,
                "owner_tag": "GER",
                "selected_country_tag": "GER",
                "source_occurrences": [],
                "type": 70
            }
        ]
    }))
    .unwrap()
}

fn write_observer_replay(path: &PathBuf, d78_ids: &[u32], our_filter_enabled: bool) {
    const GAME: &str = "observer_design_catalogue";
    let index = serde_json::to_vec(&json!({
        "game_id": GAME,
        "schema_version": "1",
        "snapshots": [{"hour_index": 900, "file": "000000000900.snapshot.json"}]
    }))
    .unwrap();
    let snapshot = serde_json::to_vec(&json!({
        "schema_version": "1",
        "game_id": GAME,
        "hour_index": 900,
        "_r2_design_catalog_context_live": {
            "schema_version": 1,
            "authority": "exact_a729_designer_view",
            "elapsed_wall_ms": 7,
            "frame_hour": 900,
            "completeness_record_identity": "a27:a729:transition:5",
            "transition_sequence": 5,
            "active": true,
            "mode": 1,
            "family": "tank_designer_view",
            "our_filter_enabled": our_filter_enabled,
            "foreign_filter_enabled": !our_filter_enabled,
            "obsolete_filter_enabled": our_filter_enabled,
            "current_country_tag_id": 17,
            "selected_country_tag_ids": [17],
            "ordered_design_ids": d78_ids
        }
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
            bytes: snapshot,
            role: "snapshot",
            hour_index: Some(900),
        },
        PackEntry {
            path: format!("payload/{GAME}/{A27_DESIGN_CATALOG_ARTIFACT_PATH}"),
            bytes: observer_catalogue_bytes(),
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
fn observer_catalogue_is_independent_of_d78_filter_state_and_rows() {
    let temp = ObserverTestTempDir::new("independent");
    let first = temp.file("first.battleplan");
    let second = temp.file("second.battleplan");
    write_observer_replay(&first, &[7], true);
    write_observer_replay(&second, &[55], false);

    let first_timeline = WorldTimeline::open(open_recording(&first).unwrap()).unwrap();
    let second_timeline = WorldTimeline::open(open_recording(&second).unwrap()).unwrap();
    let first_frame = first_timeline.frame(900).unwrap();
    let second_frame = second_timeline.frame(900).unwrap();
    let first_admitted =
        admit_observer_design_catalogue(&first_frame, "GER", DesignerViewKind::Equipment).unwrap();
    let second_admitted =
        admit_observer_design_catalogue(&second_frame, "GER", DesignerViewKind::Equipment).unwrap();

    assert_eq!(first_admitted, second_admitted);
    assert_eq!(
        first_admitted
            .ordered()
            .iter()
            .map(|record| record.design_id.as_str())
            .collect::<Vec<_>>(),
        ["55:70", "7:70"]
    );

    drop(first_frame);
    drop(second_frame);
    drop(first_timeline);
    drop(second_timeline);
    portable_test_fs_retry::remove_file(first).unwrap();
    portable_test_fs_retry::remove_file(second).unwrap();
}
