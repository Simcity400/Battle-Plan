use game_ui::design_catalog::{
    admit_current_designer_catalog, admit_design_catalog, admit_designer_view,
    DesignCatalogRejection, DesignKind, DesignLifecycleState, DesignModule, DesignRecord,
    DesignStat, DesignUpgrade, DesignerViewRejection, DisplayOrderSnapshot, FieldCompleteness,
};
use replay_core::world::{DesignCatalogContext, DesignCatalogContextView, ViewStatus};
use replay_core::CurrentDesignCatalog;

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

fn live_context(ids: &[u32]) -> DesignCatalogContextView {
    DesignCatalogContextView {
        status: ViewStatus::Present,
        context: Some(DesignCatalogContext {
            schema_version: 1,
            authority: "exact_a729_designer_view".into(),
            elapsed_wall_ms: 1,
            frame_hour: 2,
            completeness_record_identity: "a27:a729:transition:9".into(),
            transition_sequence: 9,
            active: true,
            mode: 1,
            family: "tank_designer_view".into(),
            our_filter_enabled: true,
            foreign_filter_enabled: false,
            obsolete_filter_enabled: false,
            current_country_tag_id: 17,
            selected_country_tag_ids: vec![17, 42],
            ordered_design_ids: ids.to_vec(),
        }),
    }
}

#[test]
fn d78_exact_filtered_order_binds_by_stable_variant_identity() {
    let records = vec![
        record("7:70", DesignKind::TankVariant),
        record("55:70", DesignKind::TankVariant),
        record("100:70", DesignKind::TankVariant),
    ];
    let catalog = admit_design_catalog(&records, &order(&["7:70", "55:70", "100:70"])).unwrap();
    let admitted = admit_designer_view(&catalog, &live_context(&[100, 7])).unwrap();
    assert_eq!(
        admitted
            .ordered
            .iter()
            .map(|row| row.design_id.as_str())
            .collect::<Vec<_>>(),
        ["100:70", "7:70"]
    );
    assert_eq!(admitted.selected_country_tag_ids, [17, 42]);
}

#[test]
fn portable_current_catalog_admits_without_heap_or_d59_order() {
    let current = CurrentDesignCatalog {
        // This is already the d78 order, not the full A27 heap-scan order.
        records: vec![
            record("100:70", DesignKind::EquipmentVariant),
            record("7:70", DesignKind::EquipmentVariant),
        ],
        display_order: order(&["100:70", "7:70"]),
    };
    let admitted = admit_current_designer_catalog(&current, &live_context(&[100, 7])).unwrap();
    assert_eq!(
        admitted
            .ordered
            .iter()
            .map(|row| row.design_id.as_str())
            .collect::<Vec<_>>(),
        ["100:70", "7:70"]
    );
}

#[test]
fn missing_or_unknown_d78_context_fails_closed() {
    let row = record("7:70", DesignKind::TankVariant);
    let catalog = admit_design_catalog(&[row], &order(&["7:70"])).unwrap();
    let unavailable = DesignCatalogContextView {
        status: ViewStatus::DomainUnavailable,
        context: None,
    };
    assert_eq!(
        admit_designer_view(&catalog, &unavailable),
        Err(DesignerViewRejection::MissingContext)
    );
    assert_eq!(
        admit_designer_view(&catalog, &live_context(&[999])),
        Err(DesignerViewRejection::UnknownDesignId)
    );
}
