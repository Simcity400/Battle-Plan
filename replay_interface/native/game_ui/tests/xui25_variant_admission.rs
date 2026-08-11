use game_ui::runtime_graphics::ClipRect;
use game_ui::variant_admission::*;
use replay_core::world::DisplayVariantMetadata;
use std::collections::BTreeMap;

const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn key(
    width: u32,
    height: u32,
    scale: f32,
    language: &str,
    input: InputVariant,
) -> DisplayVariantKey {
    DisplayVariantKey {
        physical_viewport: PixelExtent { width, height },
        min_gui: PixelExtent {
            width: 1280,
            height: 720,
        },
        gui_scale_f32_bits: scale.to_bits(),
        display_mode: DisplayMode::BorderlessFullscreen,
        localisation_language: language.to_string(),
        input_variant: input,
    }
}

fn envelope(key: DisplayVariantKey) -> VariantEnvelope {
    VariantEnvelope {
        source_record_id: "session:42:variant:7".into(),
        evidence_manifest_sha256: HASH_A.into(),
        logical_root: PixelExtent {
            width: 1920,
            height: 1080,
        },
        logical_layout_sha256: HASH_B.into(),
        pixel_oracle_sha256: HASH_A.into(),
        font_runs: vec![FontRunBinding {
            font_id: "hoi_16b".into(),
            font_file_sha256: HASH_A.into(),
            glyph_atlas_sha256: HASH_B.into(),
            glyph_set_sha256: HASH_A.into(),
            first_glyph: 32,
            glyph_count: 96,
        }],
        clips: vec![ResolvedClip {
            element_id: "main/list".into(),
            clip: ClipRect {
                left: 0,
                top: 4,
                right: key.physical_viewport.width as i32,
                bottom: 100,
            },
        }],
        key,
        focus: None,
        resize_transition: None,
    }
}

#[test]
fn exact_resolution_aspect_scale_language_and_input_select() {
    let wanted = key(2752, 1152, 1.0, "l_english", InputVariant::MouseKeyboard);
    let catalog = VariantCatalog::admit(vec![envelope(wanted.clone())]).unwrap();
    assert_eq!(catalog.select(&wanted).unwrap().key, wanted);
}

#[test]
fn unsupported_resolution_or_aspect_never_uses_nearest_variant() {
    let admitted = key(1920, 1080, 1.0, "l_english", InputVariant::MouseKeyboard);
    let catalog = VariantCatalog::admit(vec![envelope(admitted)]).unwrap();
    let wanted = key(1921, 1080, 1.0, "l_english", InputVariant::MouseKeyboard);
    assert_eq!(
        catalog.select(&wanted),
        Err(VariantRejection::UnsupportedExactVariant(wanted))
    );
}

#[test]
fn scale_is_selected_by_exact_f32_bits_and_language_has_no_fallback() {
    let admitted = key(1920, 1080, 1.0, "l_english", InputVariant::MouseKeyboard);
    let catalog = VariantCatalog::admit(vec![envelope(admitted)]).unwrap();
    for wanted in [
        key(
            1920,
            1080,
            f32::from_bits(1.0f32.to_bits() + 1),
            "l_english",
            InputVariant::MouseKeyboard,
        ),
        key(1920, 1080, 1.0, "l_french", InputVariant::MouseKeyboard),
    ] {
        assert!(matches!(
            catalog.select(&wanted),
            Err(VariantRejection::UnsupportedExactVariant(_))
        ));
    }
}

#[test]
fn resize_requires_exact_post_transition_envelope() {
    let previous = key(1920, 1080, 1.0, "l_english", InputVariant::MouseKeyboard);
    let current = key(2752, 1152, 1.0, "l_english", InputVariant::MouseKeyboard);
    let mut target = envelope(current.clone());
    let catalog = VariantCatalog::admit(vec![target.clone()]).unwrap();
    assert_eq!(
        catalog.select_after_transition(&previous, &current),
        Err(VariantRejection::ResizeTransitionRequired)
    );
    target.resize_transition = Some(ResizeTransition {
        previous_key: previous.clone(),
        current_key: current.clone(),
        transition_ordinal: 1,
        post_resize_layout_sha256: HASH_B.into(),
    });
    let catalog = VariantCatalog::admit(vec![target]).unwrap();
    assert_eq!(
        catalog
            .select_after_transition(&previous, &current)
            .unwrap()
            .key,
        current
    );
}

#[test]
fn clip_boundaries_are_half_open_and_must_be_inside_viewport() {
    let mut value = envelope(key(
        1920,
        1080,
        1.0,
        "l_english",
        InputVariant::MouseKeyboard,
    ));
    value.clips[0].clip.right = 1921;
    assert!(matches!(
        VariantCatalog::admit(vec![value]),
        Err(VariantRejection::InvalidEnvelope(_))
    ));
}

#[test]
fn fonts_and_glyph_sets_are_authenticated_and_non_pointer_focus_is_refused() {
    let mut no_font = envelope(key(
        1920,
        1080,
        1.0,
        "l_english",
        InputVariant::MouseKeyboard,
    ));
    no_font.font_runs.clear();
    assert!(matches!(
        VariantCatalog::admit(vec![no_font]),
        Err(VariantRejection::InvalidEnvelope(_))
    ));

    let non_pointer = envelope(key(1920, 1080, 1.0, "l_english", InputVariant::Keyboard));
    assert_eq!(
        VariantCatalog::admit(vec![non_pointer]),
        Err(VariantRejection::UnsupportedInputVariant(
            InputVariant::Keyboard
        ))
    );
}

#[test]
fn launch_bound_session_metadata_builds_the_exact_key_without_rounding() {
    let metadata = DisplayVariantMetadata {
        physical_width: 2752,
        physical_height: 1152,
        min_gui_width: 1280,
        min_gui_height: 720,
        gui_scale_f32_bits: f32::from_bits(1.0f32.to_bits() + 1).to_bits(),
        refresh_rate_hz: 60,
        max_refresh_rate_hz: Some(75),
        fullscreen: false,
        borderless: true,
        display_mode: "borderless_fullscreen".into(),
        settings_file_sha256: HASH_A.into(),
    };
    let exact = DisplayVariantKey::from_session_metadata(
        &metadata,
        "l_english",
        InputVariant::MouseKeyboard,
    )
    .unwrap();
    assert_eq!(
        exact.physical_viewport,
        PixelExtent {
            width: 2752,
            height: 1152
        }
    );
    assert_eq!(
        exact.min_gui,
        PixelExtent {
            width: 1280,
            height: 720
        }
    );
    assert_eq!(exact.gui_scale_f32_bits, metadata.gui_scale_f32_bits);
    assert_eq!(exact.display_mode, DisplayMode::BorderlessFullscreen);
    assert_eq!(
        DisplayVariantKey::from_session_metadata(&metadata, "l_english", InputVariant::Controller,),
        Err(VariantRejection::UnsupportedInputVariant(
            InputVariant::Controller
        ))
    );
}

#[test]
fn duplicate_exact_variant_is_rejected() {
    let wanted = key(1920, 1080, 1.0, "l_english", InputVariant::MouseKeyboard);
    assert_eq!(
        VariantCatalog::admit(vec![envelope(wanted.clone()), envelope(wanted.clone())]),
        Err(VariantRejection::DuplicateKey(wanted))
    );
}

#[test]
fn mechanically_narrow_product_contract_selects_only_its_exact_key() {
    let admitted = key(1920, 1080, 1.0, "l_english", InputVariant::MouseKeyboard);
    let contract = ExactKeyVariantContract::admit(admitted.clone()).unwrap();
    assert_eq!(contract.select(&admitted), Ok(&admitted));

    let mut unsupported = admitted;
    unsupported.localisation_language = "l_french".into();
    assert_eq!(
        contract.select(&unsupported),
        Err(VariantRejection::UnsupportedExactVariant(unsupported))
    );
    assert_eq!(
        ExactKeyVariantContract::admit(key(
            1920,
            1080,
            1.0,
            "l_english",
            InputVariant::Controller,
        )),
        Err(VariantRejection::UnsupportedInputVariant(
            InputVariant::Controller
        ))
    );
}

#[test]
fn display_mode_and_min_gui_are_exact_key_dimensions() {
    let admitted = key(1920, 1080, 1.0, "l_english", InputVariant::MouseKeyboard);
    let catalog = VariantCatalog::admit(vec![envelope(admitted.clone())]).unwrap();

    let mut windowed = admitted.clone();
    windowed.display_mode = DisplayMode::Windowed;
    assert!(matches!(
        catalog.select(&windowed),
        Err(VariantRejection::UnsupportedExactVariant(_))
    ));

    let mut different_min_gui = admitted;
    different_min_gui.min_gui.width += 1;
    assert!(matches!(
        catalog.select(&different_min_gui),
        Err(VariantRejection::UnsupportedExactVariant(_))
    ));
}

#[test]
fn invalid_scale_layout_or_pixel_identity_rejects() {
    for bits in [0.0f32.to_bits(), (-1.0f32).to_bits(), f32::NAN.to_bits()] {
        let mut value = envelope(key(
            1920,
            1080,
            1.0,
            "l_english",
            InputVariant::MouseKeyboard,
        ));
        value.key.gui_scale_f32_bits = bits;
        assert!(matches!(
            VariantCatalog::admit(vec![value]),
            Err(VariantRejection::InvalidEnvelope(_))
        ));
    }

    for clear_layout in [true, false] {
        let mut value = envelope(key(
            1920,
            1080,
            1.0,
            "l_english",
            InputVariant::MouseKeyboard,
        ));
        if clear_layout {
            value.logical_layout_sha256.clear();
        } else {
            value.pixel_oracle_sha256 = "not-a-hash".into();
        }
        assert!(matches!(
            VariantCatalog::admit(vec![value]),
            Err(VariantRejection::InvalidEnvelope(_))
        ));
    }
}

#[test]
fn exact_font_atlas_and_glyph_set_identity_is_required() {
    for field in 0..5 {
        let mut value = envelope(key(
            1920,
            1080,
            1.0,
            "l_english",
            InputVariant::MouseKeyboard,
        ));
        let run = &mut value.font_runs[0];
        match field {
            0 => run.font_file_sha256.clear(),
            1 => run.glyph_atlas_sha256.clear(),
            2 => run.glyph_set_sha256.clear(),
            3 => run.glyph_count = 0,
            _ => {
                run.first_glyph = u32::MAX;
                run.glyph_count = 2;
            }
        }
        assert!(matches!(
            VariantCatalog::admit(vec![value]),
            Err(VariantRejection::InvalidEnvelope(_))
        ));
    }
}

#[test]
fn clips_reject_empty_negative_and_duplicate_boundaries() {
    for clip in [
        ClipRect {
            left: 10,
            top: 10,
            right: 10,
            bottom: 11,
        },
        ClipRect {
            left: -1,
            top: 0,
            right: 1,
            bottom: 1,
        },
        ClipRect {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1081,
        },
    ] {
        let mut value = envelope(key(
            1920,
            1080,
            1.0,
            "l_english",
            InputVariant::MouseKeyboard,
        ));
        value.clips[0].clip = clip;
        assert!(matches!(
            VariantCatalog::admit(vec![value]),
            Err(VariantRejection::InvalidEnvelope(_))
        ));
    }

    let mut duplicate = envelope(key(
        1920,
        1080,
        1.0,
        "l_english",
        InputVariant::MouseKeyboard,
    ));
    duplicate.clips.push(duplicate.clips[0].clone());
    assert!(matches!(
        VariantCatalog::admit(vec![duplicate]),
        Err(VariantRejection::InvalidEnvelope(_))
    ));
}

#[test]
fn resize_transition_must_match_keys_ordinal_and_selected_layout() {
    let previous = key(1920, 1080, 1.0, "l_english", InputVariant::MouseKeyboard);
    let current = key(2752, 1152, 1.0, "l_english", InputVariant::MouseKeyboard);
    for mutation in 0..4 {
        let mut value = envelope(current.clone());
        value.resize_transition = Some(ResizeTransition {
            previous_key: previous.clone(),
            current_key: current.clone(),
            transition_ordinal: 1,
            post_resize_layout_sha256: value.logical_layout_sha256.clone(),
        });
        let transition = value.resize_transition.as_mut().unwrap();
        match mutation {
            0 => transition.previous_key = current.clone(),
            1 => transition.current_key = previous.clone(),
            2 => transition.transition_ordinal = 0,
            _ => transition.post_resize_layout_sha256 = HASH_A.into(),
        }
        assert!(matches!(
            VariantCatalog::admit(vec![value]),
            Err(VariantRejection::InvalidEnvelope(_))
        ));
    }
}

#[test]
fn keyboard_controller_and_pointer_focus_are_all_fail_closed() {
    for input in [InputVariant::Keyboard, InputVariant::Controller] {
        let value = envelope(key(1920, 1080, 1.0, "l_english", input));
        assert_eq!(
            VariantCatalog::admit(vec![value]),
            Err(VariantRejection::UnsupportedInputVariant(input))
        );
    }

    let mut pointer = envelope(key(
        1920,
        1080,
        1.0,
        "l_english",
        InputVariant::MouseKeyboard,
    ));
    pointer.focus = Some(FocusVariantState {
        active_window_id: "main".into(),
        focused_widget_id: "button".into(),
        traversal: vec!["button".into()],
    });
    assert!(matches!(
        VariantCatalog::admit(vec![pointer]),
        Err(VariantRejection::InvalidEnvelope(_))
    ));
}

fn local_environment(language: &str) -> map_viewer::PlaybackEnvironment {
    map_viewer::PlaybackEnvironment {
        source_game: replay_core::SourceGameIdentity {
            version: "1.19.2".into(),
            build: "a729".into(),
            executable_sha256: HASH_A.into(),
            build_profile: "exact-a729".into(),
        },
        source_profile: replay_core::SourceProfileIdentity {
            profile_id: "local-profile".into(),
            playset_id: "local-playset".into(),
            selection_sha256: HASH_B.into(),
        },
        dlc_selection: Vec::new(),
        localisation_language: language.into(),
        settings: BTreeMap::from([
            ("physical_width".into(), "2560".into()),
            ("physical_height".into(), "1440".into()),
            ("min_gui_width".into(), "1280".into()),
            ("min_gui_height".into(), "720".into()),
            (
                "gui_scale_f32_bits".into(),
                1.25f32.to_bits().to_string(),
            ),
            ("display_mode".into(), "windowed".into()),
            ("settings_file_sha256".into(), HASH_A.into()),
        ]),
    }
}

#[test]
fn product_profile_is_local_and_does_not_consume_recorder_presentation() {
    let environment = local_environment("l_french");
    let profile = PlaybackUiProfile::admit(&environment, InputVariant::MouseKeyboard).unwrap();
    assert_eq!(
        profile.physical_viewport(),
        PixelExtent {
            width: 2560,
            height: 1440,
        }
    );
    assert_eq!(profile.key().min_gui, PixelExtent { width: 1280, height: 720 });
    assert_eq!(profile.key().gui_scale(), 1.25);
    assert_eq!(profile.key().display_mode, DisplayMode::Windowed);
    assert_eq!(profile.localisation_language(), "l_french");
    assert_eq!(profile.actual_client_input(), InputVariant::MouseKeyboard);
    assert_eq!(profile.settings_file_sha256(), HASH_A);
}

#[test]
fn product_profile_refuses_missing_settings_and_unimplemented_client_input() {
    let mut missing = local_environment("l_english");
    missing.settings.remove("gui_scale_f32_bits");
    assert!(matches!(
        PlaybackUiProfile::admit(&missing, InputVariant::MouseKeyboard),
        Err(VariantRejection::InvalidEnvelope(_))
    ));
    assert_eq!(
        PlaybackUiProfile::admit(&local_environment("l_english"), InputVariant::Controller),
        Err(VariantRejection::UnsupportedInputVariant(
            InputVariant::Controller
        ))
    );

    let mut missing_hash = local_environment("l_english");
    missing_hash.settings.remove("settings_file_sha256");
    assert!(matches!(
        PlaybackUiProfile::admit(&missing_hash, InputVariant::MouseKeyboard),
        Err(VariantRejection::InvalidEnvelope(_))
    ));
    for invalid_hash in [
        "not-a-sha256".to_string(),
        "g".repeat(64),
        HASH_A.to_uppercase(),
    ] {
        let mut invalid = local_environment("l_english");
        invalid
            .settings
            .insert("settings_file_sha256".into(), invalid_hash);
        assert!(matches!(
            PlaybackUiProfile::admit(&invalid, InputVariant::MouseKeyboard),
            Err(VariantRejection::InvalidEnvelope(_))
        ));
    }
}
