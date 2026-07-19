use game_ui::compose::{ComposeOptions, Rect};
use game_ui::dynamic_collection::{
    CollectionItemTrace, CollectionTraceAuthority, DynamicCollectionTrace,
};
use game_ui::occupation_binding::{
    admit_occupation_surface, apply_occupation_state_to_compose_options, fnv1a32,
    occupation_map_mode_inputs, GarrisonActivityRow, GarrisonEquipmentLossRow,
    GarrisonEquipmentRow, GarrisonTemplateDefinition, OccupationAlertRow, OccupationLawDefinition,
    OccupationRejection, OccupationSurfaceRequest, OccupationVisibleText,
};
use replay_core::{Leaf, OccupationEquipmentExactRow, OccupationRow};
use std::collections::BTreeMap;

fn trace(ids: &[&str]) -> DynamicCollectionTrace {
    DynamicCollectionTrace {
        authority: CollectionTraceAuthority::ExactA729Runtime,
        collection_instance_id: "xui16-test".into(),
        frame_hour: 42,
        source_items: ids
            .iter()
            .enumerate()
            .map(|(ordinal, id)| CollectionItemTrace {
                stable_item_id: (*id).into(),
                source_ordinal: ordinal as u32,
                included: true,
                filter_reason: None,
            })
            .collect(),
        ordered_item_ids: ids.iter().map(|id| (*id).into()).collect(),
        group_boundaries: vec![],
        row_template_id: "occupied_territory_state_entry".into(),
        materialized_slots: vec![],
        clip_rect: Rect::new(0, 0, 640, 480),
        content_extent_pixels: (640, 480),
        viewport_extent_pixels: (640, 480),
        scroll_min_fixed: 0,
        scroll_max_fixed: 0,
        scroll_current_fixed: 0,
        scroll_target_fixed: 0,
    }
}

fn occupation() -> OccupationRow {
    OccupationRow {
        resistance: Leaf::Measured(22.3),
        resistance_speed: Leaf::Measured(0.025),
        compliance: Leaf::Measured(65.0),
        compliance_speed: Leaf::Measured(-0.012),
        occupier_tag: Leaf::Measured("47".into()),
        occupied_country_tag: Leaf::Measured("180".into()),
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
        date_text: "1 May, 1944".into(),
        location_text: "Angola".into(),
        state_id: 891,
        occupied_country_tag: "180".into(),
        attack_blocked: false,
        lost_manpower: 20,
        lost_equipment: 15,
    }];
    let equipment_losses = vec![GarrisonEquipmentLossRow {
        stable_id: "loss:12:4".into(),
        equipment_id: 12,
        equipment_type: 4,
        equipment_token: "infantry_equipment_1".into(),
        display_name: "Infantry Equipment I".into(),
        amount_lost: 15.0,
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
        missing_equipment: vec![GarrisonEquipmentRow {
            equipment_id: 12,
            equipment_type: 4,
            equipment_token: "infantry_equipment_1".into(),
            amount: 125.0,
        }],
        resolved_text: "Garrison reinforcement required".into(),
    }];
    let state_trace = trace(&["state:891"]);
    let activity_trace = trace(&["activity:1"]);
    let loss_trace = trace(&["loss:12:4"]);

    let admitted = admit_occupation_surface(OccupationSurfaceRequest {
        source_record_id: "record:42",
        occupation_rows: &rows,
        visible_text_by_state: &text,
        law_definitions_by_hash: &laws,
        garrison_templates: &templates,
        equipment_by_state: &BTreeMap::new(),
        equipment_complete: false,
        occupation_collection: &state_trace,
        activity_collection: &activity_trace,
        equipment_loss_collection: &loss_trace,
        activities: &activities,
        activities_complete: true,
        equipment_losses: &equipment_losses,
        equipment_losses_complete: true,
        alerts: &alerts,
        alerts_complete: true,
    })
    .expect("complete exact XUI16 producer inputs");

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
    let rejected = admit_occupation_surface(OccupationSurfaceRequest {
        source_record_id: "record:42",
        occupation_rows: &bad_encoding_rows,
        visible_text_by_state: &text,
        law_definitions_by_hash: &laws,
        garrison_templates: &templates,
        equipment_by_state: &equipment,
        equipment_complete: true,
        occupation_collection: &state_trace,
        activity_collection: &activity_trace,
        equipment_loss_collection: &loss_trace,
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
    let rejected = admit_occupation_surface(OccupationSurfaceRequest {
        source_record_id: "record:42",
        occupation_rows: &overstrength_rows,
        visible_text_by_state: &text,
        law_definitions_by_hash: &laws,
        garrison_templates: &templates,
        equipment_by_state: &equipment,
        equipment_complete: true,
        occupation_collection: &state_trace,
        activity_collection: &activity_trace,
        equipment_loss_collection: &loss_trace,
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
    let rejected = admit_occupation_surface(OccupationSurfaceRequest {
        source_record_id: "record:42",
        occupation_rows: &rows,
        visible_text_by_state: &text,
        law_definitions_by_hash: &laws,
        garrison_templates: &templates,
        equipment_by_state: &equipment,
        equipment_complete: true,
        occupation_collection: &state_trace,
        activity_collection: &activity_trace,
        equipment_loss_collection: &loss_trace,
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
    let trace = trace(&["state:891"]);
    let empty_trace = self::trace(&[]);
    let result = admit_occupation_surface(OccupationSurfaceRequest {
        source_record_id: "record:42",
        occupation_rows: &rows,
        visible_text_by_state: &empty_text,
        law_definitions_by_hash: &empty_laws,
        garrison_templates: &empty_templates,
        equipment_by_state: &empty_equipment,
        equipment_complete: false,
        occupation_collection: &trace,
        activity_collection: &empty_trace,
        equipment_loss_collection: &empty_trace,
        activities: &[],
        activities_complete: false,
        equipment_losses: &[],
        equipment_losses_complete: true,
        alerts: &[],
        alerts_complete: true,
    });
    assert_eq!(result, Err(OccupationRejection::IncompleteProducer));
}
