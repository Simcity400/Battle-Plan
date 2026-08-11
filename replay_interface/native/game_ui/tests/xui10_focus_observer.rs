use game_ui::assets::AssetResolver;
use game_ui::focus_observer::{
    build_observer_focus_compose_scene, load_authored_focus_layout, prepare_observer_focus_scene,
    EvaluatedFocusLinkChildOperation, EvaluatedFocusLinkTile, EvaluatedFocusNode,
    EvaluatedFocusProgress, EvaluatedFocusScene, FocusViewerState,
};
use game_ui::loc::Localisation;
use game_ui::national_focus::{
    AdmittedNationalFocusPresentation, FocusDefinitionKind, FocusNodePresentation, FocusNodeState,
};
use std::fs;
use std::path::PathBuf;

fn semantic() -> AdmittedNationalFocusPresentation {
    AdmittedNationalFocusPresentation {
        tree_id: "test_tree".into(),
        current_national: Some("focus_a".into()),
        current_continuous: None,
        completed: vec![],
        available: vec!["focus_b".into()],
        progress_days: Some(25.0),
        paused: false,
        nodes: vec![
            FocusNodePresentation {
                id: "focus_a".into(),
                definition_kind: FocusDefinitionKind::Local,
                state: FocusNodeState::Current,
                background_sprite: "GFX_focus_current".into(),
                icon_sprite: "GFX_goal_a".into(),
                overlay_sprite: None,
                title_loc_key: "focus_a".into(),
                display_progress_percent: None,
                continuous_glow_visible: None,
                highlight_glow_visible: None,
                historical_marker_visible: None,
                joint_viewing_country_tag: None,
            },
            FocusNodePresentation {
                id: "focus_b".into(),
                definition_kind: FocusDefinitionKind::Shared,
                state: FocusNodeState::Available,
                background_sprite: "GFX_focus_can_start".into(),
                icon_sprite: "GFX_goal_b".into(),
                overlay_sprite: Some("GFX_overlay_b".into()),
                title_loc_key: "focus_b".into(),
                display_progress_percent: None,
                continuous_glow_visible: None,
                highlight_glow_visible: None,
                historical_marker_visible: None,
                joint_viewing_country_tag: None,
            },
        ],
    }
}

fn evaluated(gui_sha: &str) -> EvaluatedFocusScene {
    EvaluatedFocusScene {
        frame_hour: 900,
        country_tag: "GER".into(),
        tree_id: "test_tree".into(),
        effective_vfs_manifest_sha256: "a".repeat(64),
        authored_gui_sha256: gui_sha.into(),
        producer_complete: true,
        expected_node_count: 2,
        expected_link_tile_count: 0,
        progress: Some(EvaluatedFocusProgress {
            progress_raw: 25,
            effective_required_progress_raw: 100,
            progress_scale_raw: 1,
            display_progress_percent: 25,
        }),
        nodes: vec![
            EvaluatedFocusNode {
                focus_id: "focus_a".into(),
                source_ordinal: 0,
                canvas_position_pixels: [100, 200],
                presentation_state_code: 3,
                display_progress_percent: 25,
                continuous_glow_visible: true,
                highlight_glow_visible: false,
                historical_marker_visible: false,
                joint_viewing_country_tag: None,
                joint_viewing_flag_sprite: None,
            },
            EvaluatedFocusNode {
                focus_id: "focus_b".into(),
                source_ordinal: 1,
                canvas_position_pixels: [300, 400],
                presentation_state_code: 2,
                display_progress_percent: 0,
                continuous_glow_visible: false,
                highlight_glow_visible: true,
                historical_marker_visible: false,
                joint_viewing_country_tag: None,
                joint_viewing_flag_sprite: None,
            },
        ],
        link_tiles: vec![],
    }
}

fn temp_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "xui10_focus_observer_{}_{}",
        std::process::id(),
        line!()
    ));
    fs::create_dir_all(root.join("interface")).unwrap();
    fs::write(
        root.join("interface/nationalfocusview.gui"),
        br#"guiTypes = {
            containerWindowType = {
                name = "nationalfocusview"
                position = { x = -3 y = 78 }
                size = { width = 100%% height = 100%% }
                containerWindowType = {
                    name = "tree"
                    containerWindowType = {
                        name = "grid_window"
                        gridBoxType = { name = "grid" position = { x = 50 y = 50 } }
                    }
                }
            }
            containerWindowType = {
                name = "national_focus_item"
                position = { x = 0 y = 0 }
                size = { width = 165 height = 128 }
                iconType = { name = "continuous_glow" }
                iconType = { name = "highlight_glow" }
                iconType = { name = "bg" }
                iconType = { name = "symbol" }
                iconType = { name = "overlay" }
                iconType = { name = "historical" }
                instantTextBoxType = { name = "name" text = "" }
                iconType = { name = "viewing_flag" }
                iconType = { name = "viewing_flag_border" }
            }
            containerWindowType = {
                name = "national_focus_link"
                position = { x = -2 y = 0 }
                size = { width = 16 height = 16 }
                iconType = { name = "link" }
            }
            containerWindowType = {
                name = "national_focus_exclusive_item"
                position = { x = 0 y = 0 }
                size = { width = 1 height = 16 }
                iconType = { name = "link1" position = { x = 16 y = 10 } }
                iconType = { name = "link2" position = { x = 16 y = 10 } }
                iconType = { name = "left" position = { x = 0 y = 0 } }
                iconType = { name = "right" position = { x = 0 y = 0 } }
                iconType = { name = "mid" position = { x = 0 y = 0 } }
            }
            positionType = { name = "focus_spacing" position = { x = 96 y = 130 } }
            positionType = { name = "national_focus_center" position = { x = 130 y = 32 } }
            positionType = { name = "link_spacing" position = { x = 16 y = 16 } }
            positionType = { name = "link_offsets" position = { x = 0 y = 32 } }
            positionType = { name = "link_begin" position = { x = 80 y = 64 } }
            positionType = { name = "link_end" position = { x = 80 y = 0 } }
        }"#,
    )
    .unwrap();
    root
}

#[test]
fn complete_post_carrier_scene_joins_d12_and_preserves_engine_integer_percent() {
    let scene = evaluated(&"b".repeat(64));
    let prepared = prepare_observer_focus_scene(
        &semantic(),
        &scene,
        900,
        "GER",
        &"a".repeat(64),
        &"b".repeat(64),
    )
    .expect("complete evaluated scene");
    assert_eq!(prepared.nodes().len(), 2);
    assert_eq!(prepared.nodes()[0].canvas_position_pixels, [100, 200]);
    assert_eq!(prepared.progress().unwrap().display_progress_percent, 25);
}

#[test]
fn incomplete_or_cross_identity_scene_rejects_before_composition() {
    let mut scene = evaluated(&"b".repeat(64));
    scene.producer_complete = false;
    assert!(prepare_observer_focus_scene(
        &semantic(),
        &scene,
        900,
        "GER",
        &"a".repeat(64),
        &"b".repeat(64),
    )
    .is_err());

    let mut scene = evaluated(&"b".repeat(64));
    scene.nodes[1].focus_id = "foreign".into();
    assert!(prepare_observer_focus_scene(
        &semantic(),
        &scene,
        900,
        "GER",
        &"a".repeat(64),
        &"b".repeat(64),
    )
    .is_err());
}

#[test]
fn progress_law_preserves_engine_truncation_and_rejects_signed_64_overflow() {
    let mut scene = evaluated(&"b".repeat(64));
    scene.progress = Some(EvaluatedFocusProgress {
        progress_raw: 100_000,
        effective_required_progress_raw: 400_000,
        progress_scale_raw: 75_000,
        display_progress_percent: 25,
    });
    scene.nodes[0].display_progress_percent = 25;
    assert!(prepare_observer_focus_scene(
        &semantic(),
        &scene,
        900,
        "GER",
        &"a".repeat(64),
        &"b".repeat(64),
    )
    .is_err());

    scene.progress.as_mut().unwrap().display_progress_percent = 24;
    scene.nodes[0].display_progress_percent = 24;
    assert_eq!(
        prepare_observer_focus_scene(
            &semantic(),
            &scene,
            900,
            "GER",
            &"a".repeat(64),
            &"b".repeat(64),
        )
        .unwrap()
        .progress()
        .unwrap()
        .display_progress_percent,
        24
    );

    scene
        .progress
        .as_mut()
        .unwrap()
        .effective_required_progress_raw = i64::MAX;
    scene.progress.as_mut().unwrap().display_progress_percent = 0;
    scene.nodes[0].display_progress_percent = 0;
    assert!(prepare_observer_focus_scene(
        &semantic(),
        &scene,
        900,
        "GER",
        &"a".repeat(64),
        &"b".repeat(64),
    )
    .is_err());
}

#[test]
fn authored_layout_and_viewer_state_drive_scene_without_geometry_literals() {
    let root = temp_root();
    let resolver = AssetResolver::new(vec![root.clone()]);
    let layout = load_authored_focus_layout(&resolver).expect("authored focus layout");
    assert_eq!(layout.node_extent_pixels, [165, 128]);
    assert_eq!(layout.focus_spacing_pixels, [96, 130]);
    assert_eq!(layout.link_begin_pixels, [80, 64]);

    let prepared = prepare_observer_focus_scene(
        &semantic(),
        &evaluated(&layout.source_payload_sha256),
        900,
        "GER",
        &"a".repeat(64),
        &layout.source_payload_sha256,
    )
    .unwrap();
    let mut viewer = FocusViewerState::default();
    assert!(build_observer_focus_compose_scene(
        &layout,
        &prepared,
        &viewer,
        &Localisation::default(),
        1920,
        1080,
    )
    .unwrap()
    .is_none());
    viewer.open();
    viewer.pan_by([10, 20]).unwrap();
    assert_eq!(
        viewer.hit_test(&prepared, &layout, [111, 221]),
        Some("focus_a")
    );
    viewer.select(&prepared, Some("focus_a")).unwrap();
    let (_, options) = build_observer_focus_compose_scene(
        &layout,
        &prepared,
        &viewer,
        &Localisation::default(),
        1920,
        1080,
    )
    .unwrap()
    .expect("open focus scene");
    assert_eq!(
        options.sprite_name_overrides["xui10_node_0__bg"],
        "GFX_focus_current"
    );
    assert!(options.hidden_elements.contains("xui10_node_0__overlay"));
    assert!(!options
        .hidden_elements
        .contains("xui10_node_0__highlight_glow"));
    assert!(options.hidden_elements.contains("zoom_slider_container"));
    assert!(options.hidden_elements.contains("continuous_focus_window"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn exclusive_links_and_joint_flags_require_and_use_exact_carrier_identity() {
    let root = temp_root();
    let resolver = AssetResolver::new(vec![root.clone()]);
    let layout = load_authored_focus_layout(&resolver).expect("authored focus layout");
    let mut evaluated = evaluated(&layout.source_payload_sha256);
    evaluated.expected_link_tile_count = 1;
    evaluated.link_tiles.push(EvaluatedFocusLinkTile {
        source_ordinal: 0,
        from_focus_id: "focus_a".into(),
        to_focus_id: "focus_b".into(),
        mutually_exclusive: true,
        container_width_pixels: Some(96),
        canvas_position_pixels: [200, 300],
        child_operations: vec![
            EvaluatedFocusLinkChildOperation {
                child_name: "link1".into(),
                visible: true,
                sprite_name: "GFX_focus_exclusive_line1".into(),
                frame_index: 1,
                local_position_pixels: [16, 10],
            },
            EvaluatedFocusLinkChildOperation {
                child_name: "link2".into(),
                visible: false,
                sprite_name: "GFX_focus_exclusive_line2".into(),
                frame_index: 1,
                local_position_pixels: [16, 10],
            },
            EvaluatedFocusLinkChildOperation {
                child_name: "left".into(),
                visible: true,
                sprite_name: "GFX_focus_link_exclusive".into(),
                frame_index: 2,
                local_position_pixels: [0, 0],
            },
            EvaluatedFocusLinkChildOperation {
                child_name: "right".into(),
                visible: true,
                sprite_name: "GFX_focus_link_exclusive".into(),
                frame_index: 3,
                local_position_pixels: [80, 0],
            },
            EvaluatedFocusLinkChildOperation {
                child_name: "mid".into(),
                visible: true,
                sprite_name: "GFX_focus_link_exclusive".into(),
                frame_index: 1,
                local_position_pixels: [16, 0],
            },
        ],
    });
    evaluated.nodes[0].joint_viewing_country_tag = Some("USA".into());
    assert!(prepare_observer_focus_scene(
        &semantic(),
        &evaluated,
        900,
        "GER",
        &"a".repeat(64),
        &layout.source_payload_sha256,
    )
    .is_err());

    evaluated.nodes[0].joint_viewing_flag_sprite = Some("GFX_flag_USA".into());
    let prepared = prepare_observer_focus_scene(
        &semantic(),
        &evaluated,
        900,
        "GER",
        &"a".repeat(64),
        &layout.source_payload_sha256,
    )
    .expect("complete exclusive/joint scene");
    let mut viewer = FocusViewerState::default();
    viewer.open();
    let (_, options) = build_observer_focus_compose_scene(
        &layout,
        &prepared,
        &viewer,
        &Localisation::default(),
        1920,
        1080,
    )
    .unwrap()
    .expect("open focus scene");
    assert_eq!(
        options.sprite_name_overrides["xui10_link_0__left"],
        "GFX_focus_link_exclusive"
    );
    assert_eq!(options.frame_overrides["xui10_link_0__left"], 2);
    assert!(options.hidden_elements.contains("xui10_link_0__link2"));
    assert_eq!(
        options.sprite_name_overrides["xui10_node_0__viewing_flag"],
        "GFX_flag_USA"
    );
    fs::remove_dir_all(root).ok();
}
