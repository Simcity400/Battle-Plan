use game_ui::production_lines::{
    admit_xui12_production_view, checked_variant_display_name, CheckedVariantNameError,
    MissingLeafState, PreservedLeaf, ProductionCarrierField, ViewerRowPresentation,
    Xui12ProductionKind, Xui12ProductionLineRejection, Xui12RowIdentity, Xui12ViewerState,
};
use replay_core::{
    DesignKind, DesignRecord, EquipmentProductionLineRow, FieldCompleteness, Leaf,
    ProductionLicenseAgreementRow, ProductionLicenseVariantRow, ProductionView, TradeView,
};
use std::collections::{BTreeMap, BTreeSet};

mod identity_support;
use identity_support::identity;

fn line(id: i64, priority: i64, variant: &str) -> EquipmentProductionLineRow {
    EquipmentProductionLineRow {
        id: Leaf::Measured(id),
        priority: Leaf::Measured(priority),
        line_subtype: Leaf::Measured(format!("subtype-{id}")),
        equipment_variant_id: Leaf::Measured(variant.into()),
        active_factories: Leaf::Measured(9 + id),
        requested_factories: Leaf::Measured(14 + id),
        efficiency: Leaf::Measured(0.51 + id as f64 / 100.0),
        efficiency_cap: Leaf::Measured(0.81 + id as f64 / 100.0),
        efficiency_growth: Leaf::Measured(0.001 + id as f64 / 100_000.0),
        cost: Leaf::Measured(2.5 + id as f64),
        speed: Leaf::Measured(3.5 + id as f64),
        produced: Leaf::Measured(4.5 + id as f64),
        amount: Leaf::Measured(30 + id),
        progress_ratio: Leaf::Measured(0.25 + id as f64 / 100.0),
        daily_output: Leaf::Measured(5.5 + id as f64),
        days_to_next_unit: Leaf::Measured(6.5 + id as f64),
        mio_id: Leaf::Unobtained,
    }
}

fn view(military: Leaf<Vec<EquipmentProductionLineRow>>) -> ProductionView {
    ProductionView {
        stockpile_total: Leaf::Measured(90.0),
        stockpile: Leaf::Measured(BTreeMap::from([("stock-token".into(), 90.0)])),
        stockpile_complete: Leaf::Measured(true),
        military_lines: military,
        military_lines_complete: Leaf::Measured(true),
        naval_lines: Leaf::Measured(vec![line(3, 30, "103")]),
        naval_lines_complete: Leaf::Measured(true),
        construction_queue: Leaf::Measured(Vec::new()),
        railway_construction_lines: Leaf::Measured(Vec::new()),
        equipment_stockpile: Leaf::Measured(Vec::new()),
        convoys_complete: Leaf::Measured(true),
        convoys_owned_raw: Leaf::Measured(12_000_000),
        convoys_owned: Leaf::Measured(120.0),
        convoys_assigned: Leaf::Measured(40),
        convoys_available: Leaf::Measured(80),
        convoys_cache_valid: Leaf::Measured(true),
        convoy_variants: Leaf::Measured(Vec::new()),
        convoy_priority_reservations_complete: Leaf::Measured(true),
        convoy_priority_reservations: Leaf::Measured(vec![
            replay_core::ConvoyPriorityReservation {
                priority: Leaf::Measured(2),
                count: Leaf::Measured(7),
            },
        ]),
        deployment_queue: Leaf::Measured(Vec::new()),
    }
}

fn record(id: &str, name: Option<&str>) -> DesignRecord {
    DesignRecord {
        design_id: format!("{id}:{}", id.len()),
        source_record_id: format!("a27:{id}"),
        kind: DesignKind::EquipmentVariant,
        owner: Some("OWNER".into()),
        name: name.map(str::to_string),
        archetype: None,
        lifecycle_state: None,
        modules: Vec::new(),
        stats: Vec::new(),
        upgrades: Vec::new(),
        definition_completeness: FieldCompleteness::Complete,
        owner_completeness: FieldCompleteness::Complete,
        lifecycle_completeness: FieldCompleteness::Complete,
        module_completeness: FieldCompleteness::Complete,
        stat_completeness: FieldCompleteness::Complete,
        upgrade_completeness: FieldCompleteness::Complete,
    }
}

fn trade() -> TradeView {
    TradeView {
        resource_pools_complete: Leaf::Measured(true),
        produced: Leaf::Measured(BTreeMap::new()),
        imported: Leaf::Measured(BTreeMap::new()),
        used: Leaf::Measured(BTreeMap::new()),
        exported: Leaf::Measured(BTreeMap::new()),
        to_export: Leaf::Measured(BTreeMap::new()),
        base_export: Leaf::Measured(BTreeMap::new()),
        available_derived: Leaf::Measured(BTreeMap::new()),
        resource_rows: Leaf::Measured(Vec::new()),
        resource_rows_complete: Leaf::Measured(true),
        resource_deals: Leaf::Measured(Vec::new()),
        resource_deals_complete: Leaf::Measured(true),
        factories_total: Leaf::Measured(0),
        factories_used: Leaf::Measured(0),
        factories_available: Leaf::Measured(0),
        contracts: Leaf::Measured(Vec::new()),
        equipment_purchase_contracts: Leaf::Measured(Vec::new()),
        market_stockpile: Leaf::Measured(Vec::new()),
        market_stockpile_complete: Leaf::Measured(true),
        production_license_agreements: Leaf::Measured(vec![ProductionLicenseAgreementRow {
            parties_withheld: [None, None, None, None],
            agreement_id: Leaf::Measured("licence:7:1:60000000".into()),
            visible_order: Leaf::Measured(0),
            owner: Leaf::Measured(identity("OTHER")),
            owner_tag_db_id: Leaf::Measured(7),
            partner: Leaf::Measured(identity("ALLY")),
            partner_tag_db_id: Leaf::Measured(1),
            relation_first: Leaf::Measured(identity("OTHER")),
            relation_first_tag_db_id: Leaf::Measured(7),
            relation_second: Leaf::Measured(identity("ALLY")),
            relation_second_tag_db_id: Leaf::Measured(1),
            start_date_raw: Leaf::Measured(60_000_000),
            start_date: Leaf::Measured("1940.1.1".into()),
            end_date_raw: Leaf::Absent,
            end_date: Leaf::Absent,
            variants: Leaf::Measured(vec![ProductionLicenseVariantRow {
                variant_id: Leaf::Measured(101),
                visible_order: Leaf::Measured(0),
                creator_tag_db_id: Leaf::Measured(1),
                origin_tag_db_id: Leaf::Measured(1),
                source_tag_db_id: Leaf::Measured(1),
            }]),
            source: Leaf::Measured("exact-test".into()),
        }]),
        production_license_agreements_complete: Leaf::Measured(true),
    }
}

fn records() -> Vec<DesignRecord> {
    vec![
        record("101", Some("First name")),
        record("102", Some("Second name")),
        record("103", Some("Third name")),
    ]
}

#[test]
fn carrier_admission_preserves_source_order_figures_and_viewer_state() {
    let view = view(Leaf::Measured(vec![line(1, 20, "101"), line(2, 10, "102")]));
    let mut viewer = Xui12ViewerState {
        panel_open: true,
        scroll_offset_px: 37,
        ..Xui12ViewerState::default()
    };
    viewer.row_presentation.insert(
        Xui12RowIdentity::ExistingLine {
            kind: Xui12ProductionKind::Military,
            line_id: 2,
        },
        ViewerRowPresentation::Collapsed,
    );
    let admitted = admit_xui12_production_view(&view, &trade(), "OTHER", &records(), &viewer)
        .expect("complete measured carrier")
        .expect("open viewer panel");

    assert_eq!(admitted.selected_country, "OTHER");
    assert_eq!(admitted.scroll_offset_px, 37);
    assert_eq!(
        admitted
            .military_lines
            .iter()
            .map(|row| (row.id, row.priority, row.equipment_display_name.as_str()))
            .collect::<Vec<_>>(),
        [(1, 20, "First name"), (2, 10, "Second name")]
    );
    assert_eq!(
        admitted.military_lines[0].presentation,
        ViewerRowPresentation::Expanded
    );
    assert_eq!(
        admitted.military_lines[1].presentation,
        ViewerRowPresentation::Collapsed
    );
    assert_eq!(admitted.military_lines[1].active_factories, 11);
    assert_eq!(admitted.military_lines[1].requested_factories, 16);
    assert_eq!(admitted.military_lines[1].daily_output, 7.5);
    assert_eq!(admitted.military_lines[1].mio_id, PreservedLeaf::Unobtained);
    assert_eq!(admitted.stockpile_total, 90.0);
    assert_eq!(admitted.stockpile["stock-token"], 90.0);
    assert_eq!(
        (
            admitted.convoys_owned_raw,
            admitted.convoys_owned,
            admitted.convoys_assigned,
            admitted.convoys_available,
            admitted.convoys_cache_valid,
        ),
        (12_000_000, 120.0, 40, 80, true)
    );
    assert_eq!(
        admitted.production_license_agreements[0]
            .agreement_id
            .measured()
            .map(String::as_str),
        Some("licence:7:1:60000000")
    );
    assert_eq!(
        admitted.convoy_priority_reservations[0]
            .count
            .measured()
            .copied(),
        Some(7)
    );
    assert!(admitted.deployment_queue.is_empty());
    assert_eq!(
        admitted
            .naval_lines
            .iter()
            .map(|row| (row.id, row.equipment_display_name.as_str()))
            .collect::<Vec<_>>(),
        [(3, "Third name")]
    );
}

#[test]
fn selected_country_authority_does_not_consult_the_recorded_pov() {
    let recorded_pov = "REC";
    let selected_country = "OTHER";
    assert_ne!(selected_country, recorded_pov);
    let admitted = admit_xui12_production_view(
        &view(Leaf::Measured(vec![line(1, 20, "101")])),
        &trade(),
        selected_country,
        &records(),
        &Xui12ViewerState::default(),
    )
    .expect("selected-country carrier is complete")
    .expect("viewer opened the panel");
    assert_eq!(admitted.selected_country, "OTHER");
    assert_eq!(
        admitted
            .military_lines
            .iter()
            .map(|line| (line.id, line.equipment_display_name.as_str()))
            .collect::<Vec<_>>(),
        [(1, "First name")]
    );
    assert_eq!(
        admitted
            .naval_lines
            .iter()
            .map(|line| (line.id, line.equipment_display_name.as_str()))
            .collect::<Vec<_>>(),
        [(3, "Third name")]
    );
    assert_eq!(
        admitted.stockpile,
        BTreeMap::from([("stock-token".into(), 90.0)])
    );
    assert_eq!(
        (admitted.convoys_owned, admitted.convoys_available),
        (120.0, 80)
    );
    assert_eq!(
        admitted.production_license_agreements[0]
            .partner
            .measured()
            .map(replay_core::CountryIdentityRef::tag),
        Some("ALLY")
    );
}

#[test]
fn supporting_family_completeness_reaches_the_exact_typed_rejection() {
    let production = view(Leaf::Measured(vec![line(1, 20, "101")]));
    let mut trade = trade();
    trade.production_license_agreements_complete = Leaf::Unobtained;
    assert_eq!(
        admit_xui12_production_view(
            &production,
            &trade,
            "OTHER",
            &records(),
            &Xui12ViewerState::default(),
        ),
        Err(
            Xui12ProductionLineRejection::SupportingCompletenessNotMeasuredTrue {
                carrier_field: ProductionCarrierField::new(stringify!(
                    production_license_agreements
                )),
                state: MissingLeafState::Unobtained,
            }
        )
    );
}

#[test]
fn unmeasured_collection_and_viewer_closed_are_reachable_no_draw_outcomes() {
    let mut input = view(Leaf::Unobtained);
    assert_eq!(
        admit_xui12_production_view(
            &input,
            &trade(),
            "TAG",
            &records(),
            &Xui12ViewerState::default()
        ),
        Ok(None)
    );
    input.military_lines = Leaf::Measured(vec![line(1, 20, "101")]);
    let viewer = Xui12ViewerState {
        panel_open: false,
        ..Xui12ViewerState::default()
    };
    assert_eq!(
        admit_xui12_production_view(&input, &trade(), "TAG", &records(), &viewer),
        Ok(None)
    );
}

#[test]
fn completeness_guards_reach_and_name_the_exact_typed_branch() {
    let mut input = view(Leaf::Measured(vec![line(1, 20, "101")]));
    input.military_lines_complete = Leaf::Absent;
    assert_eq!(
        admit_xui12_production_view(
            &input,
            &trade(),
            "TAG",
            &records(),
            &Xui12ViewerState::default()
        ),
        Err(Xui12ProductionLineRejection::CompletenessNotMeasuredTrue {
            carrier_field: ProductionCarrierField::new(stringify!(military_lines)),
            state: MissingLeafState::Absent,
        })
    );
    input.military_lines_complete = Leaf::Measured(false);
    assert_eq!(
        admit_xui12_production_view(
            &input,
            &trade(),
            "TAG",
            &records(),
            &Xui12ViewerState::default()
        ),
        Err(Xui12ProductionLineRejection::CompletenessMeasuredFalse {
            carrier_field: ProductionCarrierField::new(stringify!(military_lines)),
        })
    );
}

#[test]
fn nonmeasured_display_figure_reaches_exact_required_leaf_rejection() {
    let mut row = line(1, 20, "101");
    row.daily_output = Leaf::Unobtained;
    let input = view(Leaf::Measured(vec![row]));
    assert_eq!(
        admit_xui12_production_view(
            &input,
            &trade(),
            "TAG",
            &records(),
            &Xui12ViewerState::default()
        ),
        Err(Xui12ProductionLineRejection::RequiredLeafNotMeasured {
            carrier_field: ProductionCarrierField::new(stringify!(military_lines)),
            row: 0,
            field: "daily_output",
            state: MissingLeafState::Unobtained,
        })
    );
}

#[test]
fn checked_display_name_join_rejects_id_fallbacks_and_ambiguous_a27_rows() {
    let valid = record("7316", Some("Admitted display name"));
    assert_eq!(
        checked_variant_display_name("7316", std::slice::from_ref(&valid)),
        Ok("Admitted display name")
    );
    assert_eq!(
        checked_variant_display_name("07316", std::slice::from_ref(&valid)),
        Err(CheckedVariantNameError::InvalidProductionVariantId)
    );
    assert_eq!(
        checked_variant_display_name("7316", &[]),
        Err(CheckedVariantNameError::NoMatchingDesign)
    );
    assert_eq!(
        checked_variant_display_name(
            "7316",
            &[
                valid.clone(),
                DesignRecord {
                    source_record_id: "a27:duplicate-7316".into(),
                    ..valid.clone()
                }
            ]
        ),
        Err(CheckedVariantNameError::AmbiguousDesign)
    );
    assert_eq!(
        checked_variant_display_name("7316", &[record("7316", None)]),
        Err(CheckedVariantNameError::MissingDisplayName)
    );
}

#[test]
fn checked_display_name_join_ignores_same_numeric_id_of_another_design_kind() {
    let equipment = record("88", Some("Equipment name"));
    let division = DesignRecord {
        design_id: "88:52".into(),
        kind: DesignKind::DivisionTemplate,
        name: Some("Division name".into()),
        ..equipment.clone()
    };
    assert_eq!(
        checked_variant_display_name("88", &[division, equipment]),
        Ok("Equipment name")
    );
}

#[test]
fn carrier_admission_does_not_sort_on_priority_or_deduplicate_ids() {
    let input = view(Leaf::Measured(vec![line(1, 90, "101"), line(1, 4, "102")]));
    let admitted = admit_xui12_production_view(
        &input,
        &trade(),
        "TAG",
        &records(),
        &Xui12ViewerState::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        admitted
            .military_lines
            .iter()
            .map(|row| (row.source_ordinal, row.id, row.priority))
            .collect::<Vec<_>>(),
        [(0, 1, 90), (1, 1, 4)]
    );
    assert_eq!(
        admitted
            .military_lines
            .iter()
            .map(|row| row.equipment_variant_id.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["101", "102"])
    );
}

#[test]
fn military_and_naval_rows_with_the_same_numeric_id_keep_independent_viewer_state() {
    let mut input = view(Leaf::Measured(vec![line(3, 90, "101")]));
    input.naval_lines = Leaf::Measured(vec![line(3, 4, "103")]);
    let military_identity = Xui12RowIdentity::ExistingLine {
        kind: game_ui::production_lines::Xui12ProductionKind::Military,
        line_id: 3,
    };
    let naval_identity = Xui12RowIdentity::ExistingLine {
        kind: game_ui::production_lines::Xui12ProductionKind::Naval,
        line_id: 3,
    };
    let mut viewer = Xui12ViewerState::default();
    viewer.set_row_presentation(military_identity.clone(), ViewerRowPresentation::Collapsed);
    let admitted = admit_xui12_production_view(&input, &trade(), "TAG", &records(), &viewer)
        .unwrap()
        .unwrap();

    assert_ne!(military_identity, naval_identity);
    assert_eq!(admitted.military_lines[0].row_identity, military_identity);
    assert_eq!(admitted.naval_lines[0].row_identity, naval_identity);
    assert_eq!(
        admitted.military_lines[0].presentation,
        ViewerRowPresentation::Collapsed
    );
    assert_eq!(
        admitted.naval_lines[0].presentation,
        ViewerRowPresentation::Expanded
    );
}
