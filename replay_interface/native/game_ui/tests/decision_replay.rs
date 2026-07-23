use game_ui::decision_replay::*;

fn catalog() -> DecisionStaticCatalog {
    let raw = br#"{
      "decisions": {
        "plain": {
          "id":"plain","category":"political","loc_key":"plain","icon":"generic",
          "source_layer":"fixture","source_file":"common/decisions/fixture.txt",
          "cost":{"has_custom_cost_trigger":false,"political_power":"50"},
          "targets":{"has_target_trigger":false,"has_target_root_trigger":false},
          "cooldowns":{"complete_effect_present":true,"timeout_effect_present":false,"remove_effect_present":false},
          "visibility":{"has_visible":true,"has_available":true,"has_allowed":true}
        },
        "mission": {
          "id":"mission","category":"political","loc_key":"mission","icon":null,
          "source_layer":"fixture","source_file":"common/decisions/fixture.txt",
          "cost":{"has_custom_cost_trigger":false},
          "targets":{"has_target_trigger":false,"has_target_root_trigger":false},
          "cooldowns":{"complete_effect_present":true,"timeout_effect_present":true,"remove_effect_present":true,"days_mission_timeout":"180"},
          "visibility":{"has_visible":true,"has_available":false,"has_allowed":true},
          "is_good":"no"
        },
        "targeted": {
          "id":"targeted","category":"political","loc_key":"targeted","icon":"GFX_decision_target",
          "source_layer":"fixture","source_file":"common/decisions/fixture.txt",
          "cost":{"has_custom_cost_trigger":true,"custom_cost_text":"CUSTOM_COST"},
          "targets":{"has_target_trigger":true,"has_target_root_trigger":false,"target_scope":"state"},
          "cooldowns":{"complete_effect_present":true,"timeout_effect_present":false,"remove_effect_present":true,"days_re_enable":"30"},
          "visibility":{"has_visible":true,"has_available":true,"has_allowed":true}
        }
      }
    }"#;
    DecisionStaticCatalog::from_extractor_json(raw).unwrap()
}

#[test]
fn targeted_definition_without_state_declaration_is_country_target() {
    let raw = br#"{"decisions":{"country_target":{"id":"country_target","category":"c","loc_key":"country_target","icon":null,"source_layer":"fixture","source_file":"d.txt","cost":{"has_custom_cost_trigger":false},"targets":{"has_target_trigger":true,"has_target_root_trigger":false},"cooldowns":{"complete_effect_present":false,"timeout_effect_present":false,"remove_effect_present":false},"visibility":{"has_visible":true,"has_available":true,"has_allowed":true}}}}"#;
    let catalog = DecisionStaticCatalog::from_extractor_json(raw).unwrap();
    assert_eq!(
        catalog.entries["country_target"].target_scope,
        Some(DecisionTargetScope::Country)
    );
}

fn valid_snapshot() -> DecisionPanelSnapshot {
    DecisionPanelSnapshot {
        producer_complete: true,
        frame_hour: 100,
        country_tag: "GER".into(),
        decision_catalog_sha256: catalog().source_sha256,
        grid: DecisionGridState {
            clip_left: 0,
            clip_top: 0,
            clip_right: 502,
            clip_bottom: 600,
            content_origin_x: 0,
            content_origin_y: 0,
            content_extent_width: 502,
            content_extent_height: 123,
            scroll_min: 0,
            scroll_max: 0,
            scroll_target: 0,
            scroll_current: 0,
        },
        children: vec![
            DecisionPanelChild { presentation_index: 0, kind: DecisionPanelChildKind::Decision, decision_row_index: Some(0), identity: "plain".into(), name_text: Some("War Propaganda".into()), secondary_text: None, expanded: None, x: 0, y: 0, width: 502, height: 41 },
            DecisionPanelChild { presentation_index: 1, kind: DecisionPanelChildKind::Decision, decision_row_index: Some(1), identity: "mission".into(), name_text: Some("Mission".into()), secondary_text: Some("\u{11}H24 days\u{11}!".into()), expanded: None, x: 0, y: 41, width: 502, height: 41 },
            DecisionPanelChild { presentation_index: 2, kind: DecisionPanelChildKind::Decision, decision_row_index: Some(2), identity: "targeted".into(), name_text: Some("Targeted".into()), secondary_text: Some("\u{11}H30 days\u{11}!".into()), expanded: None, x: 0, y: 82, width: 502, height: 41 },
        ],
        available_alert_count: 1,
        timeout_alert_count: 1,
        rows: vec![
            DecisionPanelRow {
                decision_id: "plain".into(),
                category_id: "political".into(),
                presentation_index: 0,
                availability: DecisionAvailability::Available,
                lifecycle: DecisionLifecycle::Pending,
                interface_state: Some(1),
                on_map_entry_ordinal: None,
                target: None,
                cost: Some(DecisionCostState {
                    political_power: Some(50),
                    custom_cost_available: None,
                    formatted_text: Some("\u{11}Y50 PP\u{11}!".into()),
                }),
                remaining_timer_days: None,
                cooldown_remaining_days: None,
                name_text: "War Propaganda".into(),
                timer_text: None,
                alert_to_take: true,
                alert_timeout: false,
            },
            DecisionPanelRow {
                decision_id: "mission".into(),
                category_id: "political".into(),
                presentation_index: 1,
                availability: DecisionAvailability::Active,
                lifecycle: DecisionLifecycle::Running,
                interface_state: Some(3),
                on_map_entry_ordinal: None,
                target: None,
                cost: None,
                remaining_timer_days: Some(24),
                cooldown_remaining_days: None,
                name_text: "Mission".into(),
                timer_text: Some("\u{11}H24 days\u{11}!".into()),
                alert_to_take: false,
                alert_timeout: true,
            },
            DecisionPanelRow {
                decision_id: "targeted".into(),
                category_id: "political".into(),
                presentation_index: 2,
                availability: DecisionAvailability::OnCooldown,
                lifecycle: DecisionLifecycle::Pending,
                interface_state: Some(12),
                on_map_entry_ordinal: None,
                target: Some(DecisionTarget::State { state_id: 64 }),
                cost: Some(DecisionCostState {
                    political_power: None,
                    custom_cost_available: Some(false),
                    formatted_text: None,
                }),
                remaining_timer_days: None,
                cooldown_remaining_days: Some(30),
                name_text: "Targeted".into(),
                timer_text: Some("\u{11}H30 days\u{11}!".into()),
                alert_to_take: false,
                alert_timeout: false,
            },
        ],
    }
}

#[test]
fn complete_snapshot_binds_exact_order_alerts_and_presentation() {
    let binding = bind_decision_panel(&catalog(), &valid_snapshot()).unwrap();
    assert_eq!(binding.cards.len(), 3);
    assert_eq!(binding.available_alert_count, 1);
    assert_eq!(binding.timeout_alert_count, 1);
    assert_eq!(binding.alert_tooltip_row_limit, 5);
    assert_eq!(binding.cards[0].template, "decision_item");
    assert_eq!(binding.cards[1].template, "timed_decision_item");
    assert_eq!(binding.cards[2].template, "targeted_decision_item");
    assert_eq!(binding.cards[0].icon_sprite, "GFX_decision_generic");
    assert_eq!(binding.cards[1].icon_sprite, DECISION_UNKNOWN_ICON);
    assert_eq!(binding.cards[1].progress_sprite, DECISION_PROGRESS_BAD);
    assert_eq!(
        binding.cards[1].timer_text.as_deref(),
        Some("\u{11}H24 days\u{11}!")
    );
}

#[test]
fn heterogeneous_panel_materializes_structural_and_decision_items_in_exact_order() {
    let mut snapshot = valid_snapshot();
    for (offset, child) in snapshot.children.iter_mut().enumerate() {
        child.presentation_index = u32::try_from(offset + 3).unwrap();
        child.y += 111;
    }
    for row in &mut snapshot.rows {
        row.presentation_index += 3;
    }
    snapshot.grid.content_extent_height += 111;
    snapshot.children.splice(
        0..0,
        [
            DecisionPanelChild {
                presentation_index: 0,
                kind: DecisionPanelChildKind::Category,
                decision_row_index: None,
                identity: "political".into(),
                name_text: Some("Political Actions".into()),
                secondary_text: Some("Exact category description".into()),
                expanded: Some(true),
                x: 0,
                y: 0,
                width: 502,
                height: 58,
            },
            DecisionPanelChild {
                presentation_index: 1,
                kind: DecisionPanelChildKind::FixedHeader,
                decision_row_index: None,
                identity: "event_header".into(),
                name_text: Some("Events".into()),
                secondary_text: None,
                expanded: None,
                x: 0,
                y: 58,
                width: 502,
                height: 20,
            },
            DecisionPanelChild {
                presentation_index: 2,
                kind: DecisionPanelChildKind::Event,
                decision_row_index: None,
                identity: "event_42".into(),
                name_text: Some("An exact event".into()),
                secondary_text: Some("3 days".into()),
                expanded: None,
                x: 0,
                y: 78,
                width: 502,
                height: 33,
            },
        ],
    );

    let binding = bind_decision_panel(&catalog(), &snapshot).unwrap();
    assert_eq!(binding.items.len(), 6);
    assert!(matches!(&binding.items[0], DecisionPanelItem::Category(_)));
    assert!(matches!(&binding.items[1], DecisionPanelItem::FixedHeader { .. }));
    assert!(matches!(&binding.items[2], DecisionPanelItem::Event(_)));
    assert!(matches!(&binding.items[3], DecisionPanelItem::Decision { .. }));
    assert_eq!(binding.cards.len(), 3);
    assert_eq!(binding.grid.content_extent_height, 234);
}

#[test]
fn captured_widget_name_is_never_used_to_synthesize_cost() {
    let binding = bind_decision_panel(&catalog(), &valid_snapshot()).unwrap();
    assert_eq!(binding.cards[0].name_text, "War Propaganda");
    assert_eq!(
        binding.cards[0]
            .cost
            .as_ref()
            .and_then(|cost| cost.formatted_text.as_deref()),
        Some("\u{11}Y50 PP\u{11}!")
    );
}

#[test]
fn incomplete_producer_is_rejected() {
    let mut snapshot = valid_snapshot();
    snapshot.producer_complete = false;
    assert!(bind_decision_panel(&catalog(), &snapshot).is_err());
}

#[test]
fn noncontiguous_or_reordered_rows_are_rejected() {
    let mut snapshot = valid_snapshot();
    snapshot.rows.swap(0, 1);
    assert!(bind_decision_panel(&catalog(), &snapshot).is_err());
}

#[test]
fn static_target_scope_is_mandatory_and_exact() {
    let mut snapshot = valid_snapshot();
    snapshot.rows[2].target = Some(DecisionTarget::Country { tag: "ENG".into() });
    assert!(bind_decision_panel(&catalog(), &snapshot).is_err());
}

#[test]
fn active_and_cooldown_state_invariants_fail_closed() {
    let mut active = valid_snapshot();
    active.rows[1].lifecycle = DecisionLifecycle::Failed;
    assert!(bind_decision_panel(&catalog(), &active).is_err());

    let mut cooldown = valid_snapshot();
    cooldown.rows[2].cooldown_remaining_days = Some(0);
    assert!(bind_decision_panel(&catalog(), &cooldown).is_err());
}

#[test]
fn timer_text_and_numeric_timer_must_be_complete_together() {
    let mut snapshot = valid_snapshot();
    snapshot.rows[1].timer_text = None;
    assert!(bind_decision_panel(&catalog(), &snapshot).is_err());
}

#[test]
fn alert_counts_and_semantics_must_match_rows() {
    let mut count = valid_snapshot();
    count.available_alert_count = 2;
    assert!(bind_decision_panel(&catalog(), &count).is_err());

    let mut threshold = valid_snapshot();
    threshold.rows[1].remaining_timer_days = Some(31);
    threshold.rows[1].timer_text = Some("\u{11}H31 days\u{11}!".into());
    assert!(bind_decision_panel(&catalog(), &threshold).is_err());
}
