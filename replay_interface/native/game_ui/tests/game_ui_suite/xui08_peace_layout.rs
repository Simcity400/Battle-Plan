use game_ui::assets::AssetResolver;
use game_ui::peace_conference_ui::{
    load_peace_conference_authored_layout, PeaceConferenceScrollRegion, PeaceConferenceViewerState,
};
use game_ui::readonly_design_peace::{
    admit_peace_conference, ActivePeaceConferenceSnapshot, DisplayAtom, PeaceConferenceReference,
    PeaceConferenceSnapshot, PeaceParticipantRole, PeaceParticipantRow,
};
use std::fs;
use std::path::PathBuf;

fn fixture_root(suffix: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "xui08_peace_layout_{}_{}_{}",
        std::process::id(),
        line!(),
        suffix
    ));
    fs::create_dir_all(root.join("interface")).unwrap();
    root
}

fn fixture_gui() -> String {
    let actions = [
        "take_state_action",
        "liberate_action",
        "puppet_action",
        "force_government_action",
        "take_navy_action",
        "demilitarize_zone_action",
        "resource_rights_action",
        "war_reparation_action",
        "dismantle_industry_action",
    ]
    .into_iter()
    .map(|name| format!("buttonType = {{ name = {name} }}"))
    .collect::<Vec<_>>()
    .join("\n");
    let templates = [
        "aggregated_action_item",
        "available_action_item",
        "conference_winner_biddings",
        "conference_beneficiary_biddings",
        "conference_bidding_aggregated_action",
        "conference_bidding_aggregated_country",
        "conference_bidding_entry",
        "conference_bidding_quick_access_button",
        "peace_expandable_separator",
        "biddings_popup_item",
    ]
    .into_iter()
    .map(|name| format!("containerWindowType = {{ name = {name} }}"))
    .collect::<Vec<_>>()
    .join("\n");
    format!(
        r#"guiTypes = {{
            containerWindowType = {{
                name = peaceconference_full_window
                containerWindowType = {{
                    name = available_actions
                    verticalScrollbar = right_vertical_slider
                    containerWindowType = {{ name = available_actions_list }}
                }}
                containerWindowType = {{
                    name = current_and_history_actions
                    verticalScrollbar = right_vertical_slider
                    containerWindowType = {{ name = country_biddings_list }}
                }}
                {actions}
            }}
            containerWindowType = {{
                name = biddings_popup_window
                containerWindowType = {{
                    name = biddings_grid
                    verticalScrollbar = right_vertical_slider
                    scroll_wheel_factor = 44
                    smooth_scrolling = yes
                }}
            }}
            {templates}
        }}"#
    )
}

fn fixture_core_gui() -> &'static str {
    r#"guiTypes = {
        extendedScrollbarType = {
            name = right_vertical_slider
            position = { x = -5 y = 0 }
            size = { width = 18 height = 18 }
            orientation = upper_right
            origo = upper_right
            smooth_scrolling = 0.25
        }
    }"#
}

fn write_fixture(root: &PathBuf, peace: &str) {
    fs::write(root.join("interface/peaceconferencewindow.gui"), peace).unwrap();
    fs::write(root.join("interface/core.gui"), fixture_core_gui()).unwrap();
}

#[test]
fn effective_file_supplies_every_dynamic_template_and_scroll_law() {
    let root = fixture_root("complete");
    write_fixture(&root, &fixture_gui());
    let layout = load_peace_conference_authored_layout(&AssetResolver::new(vec![root.clone()]))
        .expect("complete effective XUI08 layout");
    assert_eq!(layout.templates.len(), 11);
    assert_eq!(layout.contested_popup_wheel_factor, 44);
    assert!(layout.contested_popup_smooth_scrolling);
    assert_eq!(layout.available_actions_wheel_factor, None);
    assert_eq!(layout.history_wheel_factor, None);
    assert_eq!(layout.available_actions_smooth_factor_fixed, 25_000);
    assert!(layout.template("conference_bidding_entry").is_some());
    fs::remove_dir_all(root).ok();
}

#[test]
fn missing_controller_widget_fails_closed() {
    let root = fixture_root("missing_widget");
    let broken = fixture_gui().replace(
        "buttonType = { name = take_navy_action }",
        "buttonType = { name = caller_invented_action }",
    );
    write_fixture(&root, &broken);
    let error = load_peace_conference_authored_layout(&AssetResolver::new(vec![root.clone()]))
        .expect_err("missing exact controller widget must refuse");
    assert!(error.message.contains("take_navy_action"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn viewer_scroll_is_nonnegative_and_clamped() {
    let mut state = PeaceConferenceViewerState::default();
    state
        .set_scroll(PeaceConferenceScrollRegion::AvailableActions, 90, 40)
        .unwrap();
    assert_eq!(state.available_actions_scroll_px, 40);
    assert!(state
        .set_scroll(PeaceConferenceScrollRegion::AvailableActions, -1, 40)
        .is_err());

    state
        .set_scroll(PeaceConferenceScrollRegion::CurrentAndHistory, 90, 100)
        .unwrap();
    state
        .clamp_scroll(PeaceConferenceScrollRegion::CurrentAndHistory, 12)
        .unwrap();
    assert_eq!(state.current_and_history_scroll_px, 12);
    assert_eq!(
        state.displayed_fixed(PeaceConferenceScrollRegion::CurrentAndHistory),
        1_200_000
    );

    state.select_allocation("allocation:one".into());
    state
        .set_scroll(PeaceConferenceScrollRegion::ContestedPopup, 70, 100)
        .unwrap();
    state.clear_contested_popup();
    assert!(state.selected_allocation_id.is_none());
    assert_eq!(state.contested_popup_scroll_px, 0);
    assert_eq!(
        state.displayed_fixed(PeaceConferenceScrollRegion::ContestedPopup),
        0
    );
}

fn atom(id: &str, value: &str) -> DisplayAtom {
    DisplayAtom {
        source_record_id: id.into(),
        raw_value: value.into(),
        rendered_text: value.into(),
    }
}

fn conference(reference_id: u32, snapshot_id: &str, participant: &str) -> PeaceConferenceSnapshot {
    let reference = PeaceConferenceReference {
        object_type: 9,
        object_id: reference_id,
    };
    PeaceConferenceSnapshot::Active(ActivePeaceConferenceSnapshot {
        source_record_id: snapshot_id.into(),
        conference_reference: reference,
        conference_id: reference.portable_id(),
        phase: "bid".into(),
        turn: 1,
        remaining_score_turns: 1,
        current_participant_id: participant.into(),
        current_score: atom("score", "10"),
        score_this_turn: atom("turn-score", "3"),
        available_state_ids: Vec::new(),
        contested_state_ids: Vec::new(),
        participants: vec![PeaceParticipantRow {
            source_record_id: "participant-row".into(),
            ordinal: 0,
            participant_id: participant.into(),
            country_tag: "GER".into(),
            role: PeaceParticipantRole::Victor,
            score: atom("participant-score", "10"),
            score_ratio: atom("participant-ratio", "100%"),
            allocation_ids: Vec::new(),
            result_ids: Vec::new(),
            bid_count: atom("participant-bid-count", "0"),
            contested_bid_count: atom("participant-contested-count", "0"),
        }],
        allocations: Vec::new(),
        allocation_groups: Vec::new(),
        contested_popups: Vec::new(),
        results: Vec::new(),
        complete: true,
    })
}

#[test]
fn viewer_selection_survives_only_the_same_conference_identity() {
    let first = admit_peace_conference(&conference(40, "refresh-1", "participant:GER")).unwrap();
    let refresh = admit_peace_conference(&conference(40, "refresh-2", "participant:GER")).unwrap();
    let replacement =
        admit_peace_conference(&conference(41, "refresh-3", "participant:USA")).unwrap();
    let mut state = PeaceConferenceViewerState::default();
    state.set_open(true);
    state.synchronize(&first);
    state.selected_participant_id = Some("participant:GER".into());
    state
        .expanded_participant_ids
        .insert("participant:GER".into());
    state
        .set_scroll(PeaceConferenceScrollRegion::CurrentAndHistory, 12, 30)
        .unwrap();

    state.synchronize(&refresh);
    assert_eq!(
        state.selected_participant_id.as_deref(),
        Some("participant:GER")
    );
    assert_eq!(state.current_and_history_scroll_px, 12);
    assert_eq!(state.snapshot_identity(), Some("refresh-2"));

    state.synchronize(&replacement);
    assert!(state.selected_participant_id.is_none());
    assert!(state.expanded_participant_ids.is_empty());
    assert_eq!(state.current_and_history_scroll_px, 0);
    assert!(
        state.open,
        "conference lifecycle does not own viewer open/close"
    );
}

#[test]
fn unavailable_frame_retains_intent_until_authenticated_reconciliation() {
    let first = admit_peace_conference(&conference(40, "first", "participant:GER")).unwrap();
    let mut state = PeaceConferenceViewerState::default();
    state.set_open(true);
    state.synchronize(&first);
    state.selected_participant_id = Some("participant:GER".into());
    state
        .expanded_participant_ids
        .insert("participant:GER".into());
    state.expanded_action_kinds.insert("take_state".into());
    state
        .set_scroll(PeaceConferenceScrollRegion::CurrentAndHistory, 12, 30)
        .unwrap();
    let intent = state.clone();
    state.suspend_unavailable_frame();
    assert!(state.snapshot_identity().is_none());
    assert_eq!(state.conference_id(), intent.conference_id());
    assert_eq!(
        state.selected_participant_id,
        intent.selected_participant_id
    );
    assert_eq!(
        state.expanded_participant_ids,
        intent.expanded_participant_ids
    );
    assert_eq!(state.expanded_action_kinds, intent.expanded_action_kinds);
    assert_eq!(state.current_and_history_scroll_px, 12);
    assert!(state.open);

    // An invalid candidate cannot supply the membership used for reconciliation.
    let mut invalid = conference(40, "invalid", "participant:USA");
    if let PeaceConferenceSnapshot::Active(active) = &mut invalid {
        active.complete = false;
    }
    assert!(admit_peace_conference(&invalid).is_err());
    state.suspend_unavailable_frame();
    assert_eq!(
        state.selected_participant_id.as_deref(),
        Some("participant:GER")
    );

    let same = admit_peace_conference(&conference(40, "same", "participant:GER")).unwrap();
    state.synchronize(&same);
    assert_eq!(state.snapshot_identity(), Some("same"));
    assert!(state.expanded_action_kinds.is_empty(), "authenticated current groups prune stale expansion keys");
    assert_eq!(state.current_and_history_scroll_px, 12);
    assert_eq!(
        state.expanded_participant_ids,
        intent.expanded_participant_ids
    );

    state.suspend_unavailable_frame();
    let changed = admit_peace_conference(&conference(40, "changed", "participant:USA")).unwrap();
    state.synchronize(&changed);
    assert!(state.selected_participant_id.is_none());
    assert!(state.expanded_participant_ids.is_empty());
    assert_eq!(state.current_and_history_scroll_px, 12);
    assert!(state.open);

    state.suspend_unavailable_frame();
    let replacement =
        admit_peace_conference(&conference(41, "replacement", "participant:USA")).unwrap();
    state.synchronize(&replacement);
    assert!(state.expanded_action_kinds.is_empty());
    assert_eq!(state.current_and_history_scroll_px, 0);
    assert!(state.open);
}
