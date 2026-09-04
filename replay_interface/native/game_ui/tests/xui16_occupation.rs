use game_ui::compose::ComposeOptions;
use game_ui::occupation_binding::{
    admit_occupation_fixture_from_world, admit_occupation_fixture_surface,
    apply_occupation_state_to_compose_options, fnv1a32,
    occupation_map_mode_inputs, GarrisonActivityRow, GarrisonEquipmentLossRow,
    GarrisonEquipmentRow, GarrisonShortageEquipmentRow, GarrisonTemplateDefinition,
    ObserverSemanticCollection, ObserverSemanticRow, OccupationAlertRow,
    OccupationFixtureSurfaceRequest, OccupationLawDefinition, OccupationObserverCollections,
    OccupationRejection, OccupationSurfaceRequest, OccupationVisibleText,
    OCCUPATION_COLLECTION_ACTIVITY, OCCUPATION_COLLECTION_ALERTS,
    OCCUPATION_COLLECTION_EQUIPMENT_LOSSES, OCCUPATION_COLLECTION_STATES,
};
use replay_core::{open_recording, Leaf, OccupationEquipmentExactRow, OccupationRow, WorldTimeline};

mod identity_support;
use identity_support::identity;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn collection(
    collection_id: &'static str,
    ids: &[&str],
) -> ObserverSemanticCollection {
    ObserverSemanticCollection {
        collection_id,
        frame_hour: 42,
        context_country: None,
        rows: ids
            .iter()
            .enumerate()
            .map(|(ordinal, id)| ObserverSemanticRow {
                stable_item_id: (*id).into(),
                source_ordinal: ordinal as u32,
            })
            .collect(),
    }
}

fn collections(
    states: &[&str],
    activities: &[&str],
    equipment_losses: &[&str],
    alerts: &[&str],
) -> OccupationObserverCollections {
    OccupationObserverCollections {
        states: collection(OCCUPATION_COLLECTION_STATES, states),
        activities: collection(OCCUPATION_COLLECTION_ACTIVITY, activities),
        equipment_losses: collection(
            OCCUPATION_COLLECTION_EQUIPMENT_LOSSES,
            equipment_losses,
        ),
        alerts: collection(OCCUPATION_COLLECTION_ALERTS, alerts),
    }
}

fn occupation() -> OccupationRow {
    OccupationRow {
        resistance: Leaf::Measured(22.3),
        resistance_speed: Leaf::Measured(0.025),
        compliance: Leaf::Measured(65.0),
        compliance_speed: Leaf::Measured(-0.012),
        occupier_tag: Leaf::Measured(identity("GER")),
        occupied_country_tag: Leaf::Measured(identity("FRA")),
        operational_status: Leaf::Absent,
        occupation_law: Leaf::Unobtained,
        occupation_law_token_id: Leaf::Measured(304_778_569),
        occupation_law_token_encoding: Leaf::Measured("fnv1a32_of_law_token".into()),
        occupation_law_token_source: Leaf::Absent,
        occupation_law_status: Leaf::Absent,
        garrison_template_id: Leaf::Measured(4183),
        garrison_army_manpower_need: Leaf::Measured(14_000),
        garrison_strength_ratio: Leaf::Measured(0.875),
        garrison_required: Leaf::Measured(2.5),
        garrison_signals_status: Leaf::Absent,
        garrison_manpower_source_tag_db_id: Leaf::Measured(47),
        occupation_controller_tag_db_id: Leaf::Measured(47),
        occupied_tag_db_id: Leaf::Measured(180),
        garrison_manpower_pool: Leaf::Absent,
        garrison_equipment: Leaf::Measured(vec![OccupationEquipmentExactRow {
            source_ordinal: Leaf::Measured(0),
            equipment_id: Leaf::Measured(12),
            equipment_type: Leaf::Measured(4),
            equipment_token: Leaf::Measured("infantry_equipment_1".into()),
            amount: Leaf::Measured(800.0),
        }]),
        garrison_equipment_complete: Leaf::Measured(true),
        garrison_equipment_need: Leaf::Absent,
        garrison_reinforcement_requests: Leaf::Absent,
    }
}

#[test]
fn exact_state_log_alert_and_equipment_inputs_are_admitted_in_trace_order() {
    let rows = BTreeMap::from([(891, occupation())]);
    let text = BTreeMap::from([(
        891,
        OccupationVisibleText {
            state_name: "Angola".into(),
            occupied_country_name: "Angola".into(),
            resistance_text: "22.3%".into(),
            resistance_speed_text: "+0.025".into(),
            compliance_text: "65.0%".into(),
            compliance_speed_text: "-0.012".into(),
            garrison_manpower_need_text: "14.0K".into(),
            garrison_required_text: "2.50".into(),
        },
    )]);
    let law = OccupationLawDefinition {
        token: "local_police_force_garrison".into(),
        display_name: "Local Police Force".into(),
        icon_frame: 11,
        gui_order: 0,
    };
    assert_eq!(fnv1a32(&law.token), 304_778_569);
    let laws = BTreeMap::from([(304_778_569, law)]);
    let templates = BTreeMap::from([(
        4183,
        GarrisonTemplateDefinition {
            template_id: 4183,
            display_name: "Cavalry Garrison".into(),
            icon_sprite: "GFX_unit_cavalry_icon_medium".into(),
        },
    )]);
    let equipment = BTreeMap::from([(
        891,
        vec![GarrisonEquipmentRow {
            equipment_id: 12,
            equipment_type: 4,
            equipment_token: "infantry_equipment_1".into(),
            amount: 800.0,
        }],
    )]);
    let activities = vec![GarrisonActivityRow {
        stable_id: "activity:1".into(),
        controller_tag: "GER".into(),
        date_text: "1 May, 1944".into(),
        location_text: "Angola".into(),
        state_id: 891,
        occupied_country_tag: "FRA".into(),
        attack_blocked: false,
        resistance_activity: Some("resistance_attack".into()),
        lost_manpower: 20,
        lost_equipment: 15,
    }];
    let equipment_losses = vec![GarrisonEquipmentLossRow {
        stable_id: "equipment:12:4".into(),
        equipment_id: 12,
        equipment_type: 4,
        equipment_token: "infantry_equipment_1".into(),
        display_name: "Infantry Equipment I".into(),
        amount_lost: 15.0,
        contributing_activity_ids: vec!["activity:1".into()],
    }];
    let alerts = vec![OccupationAlertRow {
        stable_id: "alert:GER:not_enough_garrison".into(),
        controller_tag: "GER".into(),
        kind: "alert_not_enough_garrison".into(),
        severity: "HIGH".into(),
        registry_slot: 0x3f,
        predicate_returned: false,
        equipment_fixed_point_divisor: 100_000,
        visible_order: 0,
        missing_manpower: 4200,
        missing_equipment: vec![GarrisonShortageEquipmentRow {
            archetype_id: 12,
            whole_amount: 125,
        }],
        resolved_text: "Garrison reinforcement required".into(),
    }];
    let semantic_collections = collections(
        &["state:891"],
        &["activity:1"],
        &["equipment:12:4"],
        &["alert:GER:not_enough_garrison"],
    );

    let admitted = admit_occupation_fixture_surface(OccupationSurfaceRequest {
        source_record_id: "record:42",
        occupation_rows: &rows,
        visible_text_by_state: &text,
        law_definitions_by_hash: &laws,
        garrison_templates: &templates,
        equipment_by_state: &BTreeMap::new(),
        equipment_complete: false,
        collections: &semantic_collections,
        activities: &activities,
        activities_complete: true,
        equipment_losses: &equipment_losses,
        equipment_losses_complete: true,
        alerts: &alerts,
        alerts_complete: true,
    })
    .expect("complete exact XUI16 producer inputs");

    assert!(!admitted.is_product_authority());
    assert_eq!(admitted.states()[0].state_id, 891);
    assert_eq!(admitted.states()[0].law.icon_frame, 11);
    assert_eq!(admitted.states()[0].equipment[0].amount, 800.0);
    assert_eq!(admitted.activities()[0].stable_id, "activity:1");
    assert_eq!(admitted.equipment_losses()[0].amount_lost, 15.0);
    assert_eq!(admitted.alerts()[0].visible_order, 0);

    let mut compose = ComposeOptions::new(1280, 720);
    apply_occupation_state_to_compose_options(&admitted.states()[0], &mut compose);
    assert_eq!(compose.text_overrides["state_name"], "Angola");
    assert_eq!(compose.text_overrides["selected_law"], "Local Police Force");
    assert_eq!(compose.frame_overrides["select_law_icon"], 11);
    assert_eq!(compose.progress_overrides["bar_str"], 0.875);

    let map = occupation_map_mode_inputs(&rows);
    assert_eq!(map[0].state_id, 891);
    assert_eq!(map[0].resistance, Some(22.3));
    assert_eq!(map[0].compliance, Some(65.0));

    let mut bad_encoding = occupation();
    bad_encoding.occupation_law_token_encoding = Leaf::Measured("unproven_hash".into());
    let bad_encoding_rows = BTreeMap::from([(891, bad_encoding)]);
    let rejected = admit_occupation_fixture_surface(OccupationSurfaceRequest {
        source_record_id: "record:42",
        occupation_rows: &bad_encoding_rows,
        visible_text_by_state: &text,
        law_definitions_by_hash: &laws,
        garrison_templates: &templates,
        equipment_by_state: &equipment,
        equipment_complete: true,
        collections: &semantic_collections,
        activities: &activities,
        activities_complete: true,
        equipment_losses: &equipment_losses,
        equipment_losses_complete: true,
        alerts: &alerts,
        alerts_complete: true,
    });
    assert_eq!(rejected, Err(OccupationRejection::MissingLawDefinition));

    let mut overstrength = occupation();
    overstrength.garrison_strength_ratio = Leaf::Measured(1.001);
    let overstrength_rows = BTreeMap::from([(891, overstrength)]);
    let rejected = admit_occupation_fixture_surface(OccupationSurfaceRequest {
        source_record_id: "record:42",
        occupation_rows: &overstrength_rows,
        visible_text_by_state: &text,
        law_definitions_by_hash: &laws,
        garrison_templates: &templates,
        equipment_by_state: &equipment,
        equipment_complete: true,
        collections: &semantic_collections,
        activities: &activities,
        activities_complete: true,
        equipment_losses: &equipment_losses,
        equipment_losses_complete: true,
        alerts: &alerts,
        alerts_complete: true,
    });
    assert_eq!(
        rejected,
        Err(OccupationRejection::MissingOrInvalidOccupationValue)
    );

    let mut invalid_alerts = alerts.clone();
    invalid_alerts[0].equipment_fixed_point_divisor = 1;
    let rejected = admit_occupation_fixture_surface(OccupationSurfaceRequest {
        source_record_id: "record:42",
        occupation_rows: &rows,
        visible_text_by_state: &text,
        law_definitions_by_hash: &laws,
        garrison_templates: &templates,
        equipment_by_state: &equipment,
        equipment_complete: true,
        collections: &semantic_collections,
        activities: &activities,
        activities_complete: true,
        equipment_losses: &equipment_losses,
        equipment_losses_complete: true,
        alerts: &invalid_alerts,
        alerts_complete: true,
    });
    assert_eq!(rejected, Err(OccupationRejection::InvalidAlertRows));
}

#[test]
fn incomplete_equipment_or_history_fails_closed() {
    let mut incomplete = occupation();
    incomplete.garrison_equipment_complete = Leaf::Measured(false);
    let rows = BTreeMap::from([(891, incomplete)]);
    let empty_text = BTreeMap::new();
    let empty_laws = BTreeMap::new();
    let empty_templates = BTreeMap::new();
    let empty_equipment = BTreeMap::new();
    let semantic_collections = collections(&["state:891"], &[], &[], &[]);
    let result = admit_occupation_fixture_surface(OccupationSurfaceRequest {
        source_record_id: "record:42",
        occupation_rows: &rows,
        visible_text_by_state: &empty_text,
        law_definitions_by_hash: &empty_laws,
        garrison_templates: &empty_templates,
        equipment_by_state: &empty_equipment,
        equipment_complete: false,
        collections: &semantic_collections,
        activities: &[],
        activities_complete: false,
        equipment_losses: &[],
        equipment_losses_complete: true,
        alerts: &[],
        alerts_complete: true,
    });
    assert_eq!(result, Err(OccupationRejection::IncompleteProducer));
}

#[test]
fn production_owner_fixture_crosses_typed_history_alert_and_xui16_constructor() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xui16_production_owner");
    let timeline = WorldTimeline::open(
        open_recording(&fixture)
            .unwrap_or_else(|error| panic!("open XUI16 production-owner fixture: {error}")),
    )
    .unwrap_or_else(|error| panic!("load XUI16 production-owner fixture: {error}"));
    assert_eq!(timeline.hours(), &[42]);
    let frame = timeline.frame(42).expect("XUI16 owner frame");

    let text = BTreeMap::from([(
        891,
        OccupationVisibleText {
            state_name: "Angola".into(),
            occupied_country_name: "France".into(),
            resistance_text: "22.3%".into(),
            resistance_speed_text: "+0.025".into(),
            compliance_text: "65.0%".into(),
            compliance_speed_text: "-0.012".into(),
            garrison_manpower_need_text: "14.0K".into(),
            garrison_required_text: "2.50".into(),
        },
    )]);
    let laws = BTreeMap::from([(
        304_778_569,
        OccupationLawDefinition {
            token: "local_police_force_garrison".into(),
            display_name: "Local Police Force".into(),
            icon_frame: 11,
            gui_order: 0,
        },
    )]);
    let templates = BTreeMap::from([(
        4183,
        GarrisonTemplateDefinition {
            template_id: 4183,
            display_name: "Cavalry Garrison".into(),
            icon_sprite: "GFX_unit_cavalry_icon_medium".into(),
        },
    )]);
    let activity_id = "GER|1942.8.15.1|891|0";
    let second_activity_id = "GER|1942.8.15.2|891|1";
    let loss_id = "equipment:438:70";
    let alert_id = "alert_not_enough_garrison:43800041:47";
    let semantic_collections = collections(
        &["state:891"],
        &[activity_id, second_activity_id],
        &[loss_id],
        &[alert_id],
    );
    let location_text = BTreeMap::from([
        (activity_id.into(), "Angola".into()),
        (second_activity_id.into(), "Angola".into()),
    ]);
    let equipment_names =
        BTreeMap::from([((438, 70), "Support Equipment".into())]);
    let alert_text =
        BTreeMap::from([("GER".into(), "Garrison reinforcement required".into())]);

    let admitted = admit_occupation_fixture_from_world(OccupationFixtureSurfaceRequest {
        source_record_id: "xui16-owner-fixture:42",
        frame: &frame,
        occupation_rows: frame.occupation_by_state(),
        visible_text_by_state: &text,
        law_definitions_by_hash: &laws,
        garrison_templates: &templates,
        equipment_by_state: &BTreeMap::new(),
        equipment_complete: false,
        collections: &semantic_collections,
        activity_location_text: &location_text,
        equipment_display_names: &equipment_names,
        alert_resolved_text: &alert_text,
        controller_supplement: None,
    })
    .expect("typed fixture must admit");

    assert!(!admitted.is_product_authority());
    assert_eq!(admitted.states()[0].state_id, 891);
    assert_eq!(admitted.activities()[0].stable_id, activity_id);
    assert_eq!(admitted.activities()[0].lost_equipment, 1);
    assert_eq!(admitted.activities()[1].stable_id, second_activity_id);
    assert_eq!(admitted.activities()[1].lost_equipment, 2);
    assert_eq!(admitted.equipment_losses()[0].stable_id, loss_id);
    assert_eq!(admitted.equipment_losses()[0].amount_lost, 3.0);
    assert_eq!(
        admitted.equipment_losses()[0].contributing_activity_ids,
        [activity_id.to_owned(), second_activity_id.to_owned()]
    );
    assert_eq!(admitted.alerts()[0].controller_tag, "GER");
    assert_eq!(admitted.alerts()[0].missing_manpower, 4_200);
    assert_eq!(admitted.alerts()[0].missing_equipment[0].whole_amount, 125);

    let mut wrong_hour_collections = semantic_collections.clone();
    wrong_hour_collections.activities.frame_hour = 41;
    let rejected = admit_occupation_fixture_from_world(OccupationFixtureSurfaceRequest {
        source_record_id: "xui16-owner-fixture:wrong-hour",
        frame: &frame,
        occupation_rows: frame.occupation_by_state(),
        visible_text_by_state: &text,
        law_definitions_by_hash: &laws,
        garrison_templates: &templates,
        equipment_by_state: &BTreeMap::new(),
        equipment_complete: false,
        collections: &wrong_hour_collections,
        activity_location_text: &location_text,
        equipment_display_names: &equipment_names,
        alert_resolved_text: &alert_text,
        controller_supplement: None,
    });
    assert_eq!(
        rejected,
        Err(OccupationRejection::InvalidSemanticCollection)
    );
}
