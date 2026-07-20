use game_ui::national_focus::{
    admit_exact_national_focus_presentation, admit_national_focus_presentation,
    FocusAdmissionCode, FocusCatalog, FocusDefinitionKind,
    FocusNodeDefinition, FocusNodeState, FocusStateBackgroundSprites, FocusTreeTopology,
    DEFAULT_FOCUS_ICON_SPRITE, STANDARD_FOCUS_AVAILABLE_SPRITE, STANDARD_FOCUS_COMPLETED_SPRITE,
    STANDARD_FOCUS_CURRENT_SPRITE, STANDARD_FOCUS_UNAVAILABLE_SPRITE,
};
use game_ui::research_canvas::{
    FocusCanvasRequest, LocalizedTextWitness, ProducerResolvedFocusPositions,
    RecordedFocusTransform, ResearchCanvasRequest,
};
use replay_core::{
    FocusView, Leaf, NationalFocusPresentationExact, NationalFocusPresentationExactView,
    NationalFocusPresentationItemExact, ViewStatus,
};

fn ids() -> Vec<String> {
    ["focus_a", "focus_b", "focus_c", "focus_d"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn focus() -> FocusView {
    FocusView {
        focus_tree: Leaf::Measured("germany_focus".into()),
        current: Leaf::Measured("focus_b".into()),
        current_set: Leaf::Measured(vec!["focus_b".into()]),
        current_continuous: Leaf::Unobtained,
        current_continuous_set: Leaf::Measured(vec![]),
        completed_count: Leaf::Measured(1),
        progress: Leaf::Measured(12.5),
        completed: Leaf::Measured(vec!["focus_a".into()]),
        available: Leaf::Measured(vec!["focus_c".into()]),
        visible_order: Leaf::Measured(ids()),
        paused: Leaf::Measured(true),
    }
}

fn topology() -> FocusTreeTopology {
    FocusTreeTopology {
        tree_id: "germany_focus".into(),
        effective_node_ids: ids().into_iter().collect(),
    }
}

fn catalog() -> FocusCatalog {
    let nodes = ids()
        .into_iter()
        .enumerate()
        .map(|(index, id)| {
            let kind = match index {
                2 => FocusDefinitionKind::Shared,
                3 => FocusDefinitionKind::Joint,
                _ => FocusDefinitionKind::Local,
            };
            let icon_sprite = (id != "focus_b").then(|| format!("GFX_goal_{id}"));
            let definition = FocusNodeDefinition {
                id: id.clone(),
                kind,
                icon_sprite,
                overlay_sprite: (id == "focus_a").then(|| "GFX_focus_overlay_test".into()),
                title_loc_key: id.clone(),
                joint_state_backgrounds: (kind == FocusDefinitionKind::Joint).then(|| {
                    FocusStateBackgroundSprites {
                        unavailable: "GFX_focus_unavailable_joint_TEST".into(),
                        available: "GFX_focus_can_start_joint_TEST".into(),
                        current: "GFX_focus_current_joint_TEST".into(),
                        completed: "GFX_focus_completed_joint_TEST".into(),
                    }
                }),
            };
            (id, definition)
        })
        .collect();
    FocusCatalog { nodes }
}

fn canvas(order: Vec<String>) -> ResearchCanvasRequest {
    let positions = order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.clone(), [index as f64 * 96.0, index as f64 * 130.0]))
        .collect();
    let resolved_strings = order
        .iter()
        .map(|id| (id.clone(), format!("Title {id}")))
        .collect();
    ResearchCanvasRequest::Focus(FocusCanvasRequest {
        viewport: [1920, 1080],
        previous_viewport: [1920, 1080],
        requested_transform_state_id: "focus-state-1".into(),
        recorded_transform: Some(RecordedFocusTransform {
            source_record_id: "capture-focus-1".into(),
            state_id: "focus-state-1".into(),
            viewport: [1920, 1080],
            scale: 1.0,
            offset_pixels: [100.0, 80.0],
        }),
        node_ids: order.clone(),
        resolved_positions: Some(ProducerResolvedFocusPositions {
            source_record_id: "capture-focus-1".into(),
            positions,
        }),
        text: LocalizedTextWitness {
            locale: Some("english".into()),
            required_keys: order,
            resolved_strings,
        },
    })
}

fn exact() -> NationalFocusPresentationExactView {
    let states = [1u8, 3, 2, 0];
    NationalFocusPresentationExactView {
        status: ViewStatus::Present,
        presentation: Some(NationalFocusPresentationExact {
            schema_version: 1,
            authority: "exact_a729_national_focus_view_items".into(),
            executable_sha256:
                "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
                    .into(),
            function_manifest_sha256:
                "303351ed65468aa7655b904998819c96ca1c325f906b5a3bf32c4f18decc7969"
                    .into(),
            job_receipt_sha256:
                "c6c3fb002bc05f1e092d7eb9b436aa720ca2cf19ad08f055747216dfa5871b4f"
                    .into(),
            elapsed_wall_ms: 3,
            frame_hour: 900,
            source_sequence: 11,
            producer_complete: true,
            country_tag: "GER".into(),
            view_id: "nationalfocusview:GER".into(),
            producer_record_identity: "a729:a31:900:GER:11".into(),
            expected_visible_item_count: 4,
            items: ids()
                .into_iter()
                .enumerate()
                .map(|(ordinal, focus_id)| NationalFocusPresentationItemExact {
                    focus_id,
                    source_ordinal: ordinal as u32,
                    presentation_state_code: states[ordinal],
                    display_progress_percent: match states[ordinal] {
                        1 => 100,
                        3 => 25,
                        _ => 0,
                    },
                    continuous_glow_visible: states[ordinal] == 3,
                    highlight_glow_visible: ordinal == 2,
                    historical_marker_visible: ordinal == 0,
                    joint_viewing_country_tag: (ordinal == 3).then(|| "ITA".into()),
                })
                .collect(),
        }),
    }
}

#[test]
fn exact_sets_icons_order_pause_and_progress_are_admitted() {
    let admitted =
        admit_national_focus_presentation(&focus(), &topology(), &catalog(), &canvas(ids()))
            .expect("complete exact focus presentation");

    assert_eq!(admitted.tree_id, "germany_focus");
    assert_eq!(admitted.current_national.as_deref(), Some("focus_b"));
    assert_eq!(admitted.current_continuous, None);
    assert_eq!(admitted.completed, ["focus_a"]);
    assert_eq!(admitted.available, ["focus_c"]);
    assert_eq!(admitted.progress_days, Some(12.5));
    assert!(admitted.paused);
    assert_eq!(
        admitted
            .nodes
            .iter()
            .map(|node| node.state)
            .collect::<Vec<_>>(),
        vec![
            FocusNodeState::Completed,
            FocusNodeState::Current,
            FocusNodeState::Available,
            FocusNodeState::Unavailable,
        ]
    );
    assert_eq!(admitted.nodes[1].icon_sprite, DEFAULT_FOCUS_ICON_SPRITE);
    assert_eq!(
        admitted.nodes[0].overlay_sprite.as_deref(),
        Some("GFX_focus_overlay_test")
    );
    assert_eq!(admitted.nodes[1].overlay_sprite, None);
    assert_eq!(
        admitted
            .nodes
            .iter()
            .map(|node| node.background_sprite.as_str())
            .collect::<Vec<_>>(),
        vec![
            STANDARD_FOCUS_COMPLETED_SPRITE,
            STANDARD_FOCUS_CURRENT_SPRITE,
            STANDARD_FOCUS_AVAILABLE_SPRITE,
            "GFX_focus_unavailable_joint_TEST",
        ]
    );
    assert_eq!(
        admitted.nodes[2].definition_kind,
        FocusDefinitionKind::Shared
    );
    assert_eq!(
        admitted.nodes[3].definition_kind,
        FocusDefinitionKind::Joint
    );
}

#[test]
fn exact_d89_leaves_are_admitted_without_pointer_or_inference() {
    let admitted = admit_exact_national_focus_presentation(
        &focus(),
        &exact(),
        "GER",
        &topology(),
        &catalog(),
        &canvas(ids()),
    )
    .expect("exact d89 presentation");
    assert_eq!(admitted.nodes[0].display_progress_percent, Some(100));
    assert_eq!(admitted.nodes[1].continuous_glow_visible, Some(true));
    assert_eq!(admitted.nodes[2].highlight_glow_visible, Some(true));
    assert_eq!(admitted.nodes[0].historical_marker_visible, Some(true));
    assert_eq!(
        admitted.nodes[3].joint_viewing_country_tag,
        Some(Some("ITA".into()))
    );

    let mut wrong = exact();
    wrong.presentation.as_mut().unwrap().items[1].focus_id = "focus_other".into();
    let error = admit_exact_national_focus_presentation(
        &focus(),
        &wrong,
        "GER",
        &topology(),
        &catalog(),
        &canvas(ids()),
    )
    .expect_err("cross-identity d89 item must reject");
    assert_eq!(error.code, FocusAdmissionCode::ExactPresentation);
}

#[test]
fn missing_engine_available_set_is_rejected_not_inferred() {
    let mut state = focus();
    state.available = Leaf::Unobtained;
    let error = admit_national_focus_presentation(&state, &topology(), &catalog(), &canvas(ids()))
        .expect_err("static prerequisites must not stand in for evaluated availability");
    assert_eq!(error.code, FocusAdmissionCode::MissingMeasuredState);
}

#[test]
fn canvas_sequence_must_equal_engine_visible_order() {
    let mut reversed = ids();
    reversed.reverse();
    let error =
        admit_national_focus_presentation(&focus(), &topology(), &catalog(), &canvas(reversed))
            .expect_err("authored or sorted order cannot replace engine visible order");
    assert_eq!(error.code, FocusAdmissionCode::TopologyMismatch);
}

#[test]
fn exact_empty_current_sets_preserve_no_active_focus() {
    let mut state = focus();
    state.current = Leaf::Unobtained;
    state.current_set = Leaf::Measured(vec![]);
    state.progress = Leaf::Unobtained;
    let admitted =
        admit_national_focus_presentation(&state, &topology(), &catalog(), &canvas(ids()))
            .expect("measured-empty current set");
    assert_eq!(admitted.current_national, None);
    assert_eq!(admitted.progress_days, None);
}

#[test]
fn completed_focus_outside_current_tree_is_retained_when_catalogued() {
    let mut state = focus();
    state.completed = Leaf::Measured(vec!["old_tree_focus".into()]);
    state.completed_count = Leaf::Measured(1);
    let mut definitions = catalog();
    definitions.nodes.insert(
        "old_tree_focus".into(),
        FocusNodeDefinition {
            id: "old_tree_focus".into(),
            kind: FocusDefinitionKind::Local,
            icon_sprite: Some("GFX_goal_old_tree_focus".into()),
            overlay_sprite: None,
            title_loc_key: "old_tree_focus".into(),
            joint_state_backgrounds: None,
        },
    );
    let admitted =
        admit_national_focus_presentation(&state, &topology(), &definitions, &canvas(ids()))
            .expect("historical completion from a previous selected tree");
    assert_eq!(admitted.completed, ["old_tree_focus"]);
}

#[test]
fn joint_focus_without_exact_background_family_rejects() {
    let mut definitions = catalog();
    definitions
        .nodes
        .get_mut("focus_d")
        .unwrap()
        .joint_state_backgrounds = None;
    let error =
        admit_national_focus_presentation(&focus(), &topology(), &definitions, &canvas(ids()))
            .expect_err("joint background families cannot be guessed from generic sprites");
    assert_eq!(error.code, FocusAdmissionCode::InvalidBackground);
}

#[test]
fn standard_unavailable_background_is_exact_file_token() {
    let admitted =
        admit_national_focus_presentation(&focus(), &topology(), &catalog(), &canvas(ids()))
            .expect("complete focus presentation");
    assert_eq!(
        admitted.nodes[3].background_sprite,
        "GFX_focus_unavailable_joint_TEST"
    );
    let mut state = focus();
    state.available = Leaf::Measured(vec![]);
    let admitted =
        admit_national_focus_presentation(&state, &topology(), &catalog(), &canvas(ids()))
            .expect("standard unavailable state");
    assert_eq!(
        admitted.nodes[2].background_sprite,
        STANDARD_FOCUS_UNAVAILABLE_SPRITE
    );
}

#[test]
fn empty_authored_overlay_rejects() {
    let mut definitions = catalog();
    definitions.nodes.get_mut("focus_a").unwrap().overlay_sprite = Some("".into());
    let error =
        admit_national_focus_presentation(&focus(), &topology(), &definitions, &canvas(ids()))
            .expect_err("an authored overlay token cannot be empty");
    assert_eq!(error.code, FocusAdmissionCode::InvalidIcon);
}
