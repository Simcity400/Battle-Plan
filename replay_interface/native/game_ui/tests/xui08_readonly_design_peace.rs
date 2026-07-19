use game_ui::design_catalog::{
    admit_design_catalog, DesignKind, DesignLifecycleState, DesignModule, DesignRecord, DesignStat,
    DisplayOrderSnapshot, FieldCompleteness,
};
use game_ui::readonly_design_peace::*;

fn atom(id: &str, raw: &str) -> DisplayAtom {
    DisplayAtom {
        source_record_id: id.into(),
        raw_value: raw.into(),
        rendered_text: raw.into(),
    }
}
fn catalog(kind: DesignKind, id: &str) -> game_ui::design_catalog::AdmittedDesignCatalog {
    admit_design_catalog(
        &[DesignRecord {
            design_id: id.into(),
            source_record_id: "catalog:1".into(),
            kind,
            owner: Some("GER".into()),
            name: Some(id.into()),
            archetype: Some("base".into()),
            lifecycle_state: Some(DesignLifecycleState::Active),
            modules: if kind == DesignKind::DivisionTemplate {
                vec![]
            } else {
                vec![DesignModule {
                    slot_id: "slot_1".into(),
                    module_token: "module_1".into(),
                    source_record_id: "catalog:module:1".into(),
                }]
            },
            stats: vec![DesignStat {
                stat_id: "build_cost_ic".into(),
                value: 5.5,
                source_record_id: "catalog:stat:1".into(),
            }],
            upgrades: vec![],
            definition_completeness: FieldCompleteness::Complete,
            owner_completeness: FieldCompleteness::Complete,
            lifecycle_completeness: FieldCompleteness::Complete,
            module_completeness: FieldCompleteness::Complete,
            stat_completeness: FieldCompleteness::Complete,
            upgrade_completeness: FieldCompleteness::Complete,
        }],
        &DisplayOrderSnapshot {
            source_record_id: "order:1".into(),
            completeness: FieldCompleteness::Complete,
            ordered_design_ids: vec![id.into()],
        },
    )
    .unwrap()
}
fn designer(view: DesignerViewKind, id: &str) -> ReadOnlyDesignerSnapshot {
    ReadOnlyDesignerSnapshot {
        source_record_id: "designer:1".into(),
        view,
        owner: "GER".into(),
        ordered_design_ids: vec![id.into()],
        selected_design_id: id.into(),
        name: atom("name:1", id),
        archetype: "base".into(),
        icon_token: Some("GFX_icon".into()),
        obsolete: false,
        assigned_mio_id: None,
        module_slots: if view == DesignerViewKind::Division {
            vec![]
        } else {
            vec![DesignerModuleSlot {
                source_record_id: "slot:1".into(),
                ordinal: 0,
                slot_id: "slot_1".into(),
                module_token: Some("module_1".into()),
                available: true,
            }]
        },
        stat_rows: vec![DesignerStatRow {
            source_record_id: "stat:1".into(),
            ordinal: 0,
            section_id: "base_stats".into(),
            stat_id: "build_cost_ic".into(),
            value: atom("value:1", "5.50"),
        }],
        upgrade_rows: vec![],
        division_slots: vec![],
        cost_rows: vec![atom("cost:1", "5")],
        controls_interactive: false,
        complete: true,
    }
}
fn peace() -> PeaceConferenceSnapshot {
    PeaceConferenceSnapshot::Active(ActivePeaceConferenceSnapshot {
        source_record_id: "peace:1".into(),
        conference_id: "pc_1".into(),
        phase: "bid".into(),
        turn: 2,
        remaining_score_turns: 3,
        current_participant_id: "p1".into(),
        current_score: atom("score:1", "100"),
        score_this_turn: atom("score:turn", "25"),
        available_state_ids: vec![1],
        contested_state_ids: vec![2],
        participants: vec![PeaceParticipantRow {
            source_record_id: "participant:1".into(),
            ordinal: 0,
            participant_id: "p1".into(),
            country_tag: "GER".into(),
            role: PeaceParticipantRole::Victor,
            score: atom("p1:score", "100"),
            score_ratio: atom("p1:ratio", "60%"),
        }],
        allocations: vec![PeaceAllocationRow {
            source_record_id: "allocation:1".into(),
            ordinal: 0,
            allocation_id: "a1".into(),
            turn: 2,
            action_kind: "take_states".into(),
            actor_participant_id: "p1".into(),
            beneficiary_country_tag: Some("GER".into()),
            target_country_tag: Some("POL".into()),
            target_state_ids: vec![1],
            target_ship_ids: vec![],
            cost: atom("a1:cost", "20"),
            bid: atom("a1:bid", "25"),
            status: "submitted".into(),
            contested_by_participant_ids: vec![],
        }],
        results: vec![PeaceResultRow {
            source_record_id: "result:1".into(),
            ordinal: 0,
            result_id: "r1".into(),
            allocation_id: "a1".into(),
            action_kind: "take_states".into(),
            actor_country_tag: "GER".into(),
            beneficiary_country_tag: Some("GER".into()),
            target_country_tag: Some("POL".into()),
            target_state_ids: vec![1],
            target_ship_ids: vec![],
            outcome: "awarded".into(),
        }],
        complete: true,
    })
}

#[test]
fn admits_all_six_readonly_designer_surfaces() {
    let rows = [
        (DesignerViewKind::Division, DesignKind::DivisionTemplate),
        (DesignerViewKind::Equipment, DesignKind::EquipmentVariant),
        (DesignerViewKind::Tank, DesignKind::TankVariant),
        (DesignerViewKind::Plane, DesignKind::AircraftVariant),
        (DesignerViewKind::Ship, DesignKind::ShipVariant),
        (
            DesignerViewKind::VariantUpgrade,
            DesignKind::EquipmentVariant,
        ),
    ];
    for (view, kind) in rows {
        let mut d = designer(view, "d");
        if view == DesignerViewKind::Division {
            d.division_slots.push(DivisionSlotRow {
                source_record_id: "divslot:1".into(),
                ordinal: 0,
                column: 0,
                row: 0,
                subunit_token: "infantry".into(),
                support: false,
            });
        }
        assert_eq!(
            admit_readonly_designer(&catalog(kind, "d"), &d)
                .unwrap()
                .view(),
            view
        );
    }
}
#[test]
fn designer_requires_inert_complete_state() {
    let c = catalog(DesignKind::TankVariant, "d");
    let mut d = designer(DesignerViewKind::Tank, "d");
    d.controls_interactive = true;
    assert_eq!(
        admit_readonly_designer(&c, &d),
        Err(Xui08Rejection::InteractiveControl)
    );
    d.controls_interactive = false;
    d.complete = false;
    assert_eq!(
        admit_readonly_designer(&c, &d),
        Err(Xui08Rejection::Incomplete)
    );
}
#[test]
fn designer_order_selection_and_kind_are_exact() {
    let c = catalog(DesignKind::TankVariant, "d");
    let mut d = designer(DesignerViewKind::Tank, "d");
    d.ordered_design_ids = vec!["other".into()];
    assert_eq!(
        admit_readonly_designer(&c, &d),
        Err(Xui08Rejection::CatalogOrderMismatch)
    );
    let mut d = designer(DesignerViewKind::Tank, "missing");
    d.ordered_design_ids = vec!["d".into()];
    assert_eq!(
        admit_readonly_designer(&c, &d),
        Err(Xui08Rejection::UnknownSelectedDesign)
    );
    let d = designer(DesignerViewKind::Plane, "d");
    assert_eq!(
        admit_readonly_designer(&c, &d),
        Err(Xui08Rejection::DesignerKindMismatch)
    );
}
#[test]
fn designer_rows_require_exact_order_and_display_atoms() {
    let c = catalog(DesignKind::ShipVariant, "d");
    let mut d = designer(DesignerViewKind::Ship, "d");
    d.stat_rows[0].ordinal = 1;
    assert_eq!(
        admit_readonly_designer(&c, &d),
        Err(Xui08Rejection::InvalidOrderedRows)
    );
    let mut d = designer(DesignerViewKind::Ship, "d");
    d.cost_rows[0].rendered_text.clear();
    assert_eq!(
        admit_readonly_designer(&c, &d),
        Err(Xui08Rejection::InvalidDisplayAtom)
    );
}
#[test]
fn designer_catalog_identity_and_lifecycle_are_exact() {
    let c = catalog(DesignKind::ShipVariant, "d");
    let mut d = designer(DesignerViewKind::Ship, "d");
    d.owner = "ITA".into();
    assert_eq!(
        admit_readonly_designer(&c, &d),
        Err(Xui08Rejection::CatalogBindingMismatch)
    );
    let mut d = designer(DesignerViewKind::Ship, "d");
    d.obsolete = true;
    assert_eq!(
        admit_readonly_designer(&c, &d),
        Err(Xui08Rejection::CatalogBindingMismatch)
    );
    let mut d = designer(DesignerViewKind::Ship, "d");
    d.stat_rows[0].value.raw_value = "5.51".into();
    assert_eq!(
        admit_readonly_designer(&c, &d),
        Err(Xui08Rejection::CatalogBindingMismatch)
    );
}
#[test]
fn admitted_designer_is_the_lossless_render_model() {
    let c = catalog(DesignKind::AircraftVariant, "d");
    let d = designer(DesignerViewKind::Plane, "d");
    let admitted = admit_readonly_designer(&c, &d).unwrap();
    assert_eq!(admitted.render_model(), &d);
    assert_eq!(
        admitted.chrome().gui_file,
        "interface/plane_designer_view.gui"
    );
    assert_eq!(
        designer_chrome(DesignerViewKind::Ship).gui_file,
        "interface/equipmentdesignerview.gui"
    );
    assert_eq!(
        designer_chrome(DesignerViewKind::Equipment).root_widget,
        "countryequipmentdesignerview"
    );
    assert_eq!(
        designer_chrome(DesignerViewKind::Tank).root_widget,
        "tank_designer_view"
    );
    assert_eq!(
        designer_chrome(DesignerViewKind::Plane).root_widget,
        "plane_designer_view"
    );
}
#[test]
fn duplicate_designer_display_identity_fails_closed() {
    let c = catalog(DesignKind::ShipVariant, "d");
    let mut d = designer(DesignerViewKind::Ship, "d");
    let mut duplicate = d.stat_rows[0].clone();
    duplicate.ordinal = 1;
    d.stat_rows.push(duplicate);
    assert_eq!(
        admit_readonly_designer(&c, &d),
        Err(Xui08Rejection::DuplicateDisplayIdentity)
    );
}
#[test]
fn division_composition_is_required_only_for_divisions() {
    let c = catalog(DesignKind::DivisionTemplate, "d");
    assert_eq!(
        admit_readonly_designer(&c, &designer(DesignerViewKind::Division, "d")),
        Err(Xui08Rejection::InvalidDivisionComposition)
    );
}
#[test]
fn admits_inactive_and_complete_active_peace_state() {
    assert!(!admit_peace_conference(&PeaceConferenceSnapshot::Inactive {
        source_record_id: "peace:none".into()
    })
    .unwrap()
    .active());
    let admitted = admit_peace_conference(&peace()).unwrap();
    assert!(admitted.active());
    assert_eq!(admitted.conference_id(), Some("pc_1"));
    assert_eq!(admitted.render_model(), &peace());
}
#[test]
fn peace_participant_and_reference_sets_fail_closed() {
    let mut p = peace();
    if let PeaceConferenceSnapshot::Active(v) = &mut p {
        v.current_participant_id = "unknown".into();
    }
    assert_eq!(
        admit_peace_conference(&p),
        Err(Xui08Rejection::InvalidParticipantSet)
    );
    let mut p = peace();
    if let PeaceConferenceSnapshot::Active(v) = &mut p {
        v.allocations[0].target_state_ids = vec![999];
    }
    assert_eq!(
        admit_peace_conference(&p),
        Err(Xui08Rejection::InvalidAllocation)
    );
}
#[test]
fn peace_results_must_bind_an_allocation() {
    let mut p = peace();
    if let PeaceConferenceSnapshot::Active(v) = &mut p {
        v.results[0].allocation_id = "missing".into();
    }
    assert_eq!(
        admit_peace_conference(&p),
        Err(Xui08Rejection::InvalidResult)
    );
}
#[test]
fn peace_result_payload_must_exactly_match_its_allocation() {
    let mut p = peace();
    if let PeaceConferenceSnapshot::Active(v) = &mut p {
        v.results[0].target_state_ids = vec![2];
    }
    assert_eq!(
        admit_peace_conference(&p),
        Err(Xui08Rejection::InvalidResult)
    );
}
#[test]
fn peace_action_control_order_and_groups_are_frozen() {
    assert_eq!(PEACE_ACTION_CONTROLS.len(), 9);
    assert!(PEACE_ACTION_CONTROLS
        .iter()
        .enumerate()
        .all(|(ordinal, law)| law.ordinal as usize == ordinal));
    assert!(PEACE_ACTION_CONTROLS[..5]
        .iter()
        .all(|law| law.group == PeaceActionGroup::Primary));
    assert!(PEACE_ACTION_CONTROLS[5..]
        .iter()
        .all(|law| law.group == PeaceActionGroup::Stackable));
    assert_eq!(PEACE_ACTION_CONTROLS[0].action_kind, "take_states");
    assert_eq!(PEACE_ACTION_CONTROLS[0].widget_id, "take_state_action");
    assert_eq!(PEACE_ACTION_CONTROLS[0].controller_offset, 0x5190);
    assert_eq!(PEACE_ACTION_CONTROLS[6].widget_id, "resource_rights_action");
    assert_eq!(PEACE_ACTION_CONTROLS[6].controller_offset, 0x51c8);
    assert_eq!(PEACE_ACTION_CONTROLS[7].widget_id, "war_reparation_action");
    assert_eq!(PEACE_ACTION_CONTROLS[7].controller_offset, 0x51c0);
}
#[test]
fn peace_unknown_action_and_duplicate_targets_fail_closed() {
    let mut p = peace();
    if let PeaceConferenceSnapshot::Active(v) = &mut p {
        v.allocations[0].action_kind = "unknown".into();
    }
    assert_eq!(
        admit_peace_conference(&p),
        Err(Xui08Rejection::UnsupportedPeaceAction)
    );
    let mut p = peace();
    if let PeaceConferenceSnapshot::Active(v) = &mut p {
        v.allocations[0].target_state_ids = vec![1, 1];
    }
    assert_eq!(
        admit_peace_conference(&p),
        Err(Xui08Rejection::InvalidAllocation)
    );
}
