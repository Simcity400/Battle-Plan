use game_ui::runtime_graphics::ButtonVisualState;
use game_ui::widget_visual_state::{
    admit_widget_visual, AttentionCarrier, DragSnapshot, ProgressEffect, ProgressFill,
    ReadOnlyPresentation, ScrollbarAxis, WidgetFamily, WidgetVisualRejection, WidgetVisualRequest,
};

fn request(family: WidgetFamily) -> WidgetVisualRequest {
    WidgetVisualRequest {
        source_record_id: "fixture:widget".into(),
        family,
        presentation: ReadOnlyPresentation::Passive,
        interaction: None,
        is_current_hover_target: false,
        frame_count: usize::from(family.has_sprite_surface()) * 4,
        one_based_frame: None,
        selected: false,
        active: false,
        toggle: None,
        attention: None,
        progress: None,
        drag: None,
    }
}

#[test]
fn every_effective_widget_family_is_case_folded_without_coercing_generic_blocks() {
    let cases = [
        ("windowType", WidgetFamily::Window),
        ("containerWindowType", WidgetFamily::Container),
        ("iconType", WidgetFamily::Icon),
        ("ButtonType", WidgetFamily::Button),
        ("guiButtonType", WidgetFamily::GuiButton),
        ("checkBoxType", WidgetFamily::Checkbox),
        ("dropDownBoxType", WidgetFamily::DropDown),
        ("editBoxType", WidgetFamily::EditBox),
        ("browserType", WidgetFamily::Browser),
        ("instantTextboxType", WidgetFamily::InstantText),
        ("textBoxType", WidgetFamily::Text),
        ("listBoxType", WidgetFamily::List),
        ("smoothListBoxType", WidgetFamily::SmoothList),
        ("gridboxType", WidgetFamily::Grid),
        ("OverlappingElementsBoxType", WidgetFamily::Overlap),
        ("scrollbarType", WidgetFamily::LegacyScrollbar),
        ("extendedScrollbarType", WidgetFamily::ExtendedScrollbar),
        ("increaseButton", WidgetFamily::ScrollbarPart),
        ("slider", WidgetFamily::ScrollbarPart),
    ];
    for (raw, expected) in cases {
        assert_eq!(
            WidgetFamily::from_element_type(raw),
            Some(expected),
            "{raw}"
        );
    }
    for generic in ["background", "borderSize", "slotsize", "pdx_tooltip_anchor"] {
        assert_eq!(WidgetFamily::from_element_type(generic), None, "{generic}");
    }
}

#[test]
fn exact_button_frames_cover_up_pressed_hover_fallback_and_disabled() {
    let cases = [
        (ButtonVisualState::Up, true, 1),
        (ButtonVisualState::Down, true, 2),
        (ButtonVisualState::Over, true, 3),
        (ButtonVisualState::Over, false, 1),
        (ButtonVisualState::Disable, true, 4),
    ];
    for (state, hover_target, expected) in cases {
        let mut value = request(WidgetFamily::Button);
        value.presentation = if state == ButtonVisualState::Disable {
            ReadOnlyPresentation::Disabled
        } else {
            ReadOnlyPresentation::Navigation
        };
        value.interaction = Some(state);
        value.is_current_hover_target = hover_target;
        assert_eq!(
            admit_widget_visual(&value)
                .unwrap()
                .frame()
                .unwrap()
                .one_based_frame,
            expected
        );
    }
}

#[test]
fn selected_active_and_toggle_never_guess_a_sprite_frame() {
    for mutation in 0..3 {
        let mut value = request(WidgetFamily::Checkbox);
        match mutation {
            0 => value.selected = true,
            1 => value.active = true,
            _ => value.toggle = Some(true),
        }
        assert_eq!(
            admit_widget_visual(&value),
            Err(WidgetVisualRejection::MissingSemanticFrame)
        );
        value.one_based_frame = Some(2);
        assert_eq!(
            admit_widget_visual(&value)
                .unwrap()
                .frame()
                .unwrap()
                .zero_based_index,
            1
        );
    }
}

#[test]
fn read_only_mutation_targets_are_visually_exact_and_never_mutate_game_state() {
    let mut inert = request(WidgetFamily::Checkbox);
    inert.presentation = ReadOnlyPresentation::InertSnapshot;
    inert.toggle = Some(true);
    inert.one_based_frame = Some(2);
    let admitted = admit_widget_visual(&inert).unwrap();
    assert!(!admitted.allows_navigation_events);
    assert!(!admitted.allows_game_mutation);

    let mut disabled = request(WidgetFamily::Button);
    disabled.presentation = ReadOnlyPresentation::Disabled;
    disabled.interaction = Some(ButtonVisualState::Disable);
    let admitted = admit_widget_visual(&disabled).unwrap();
    assert_eq!(admitted.frame().unwrap().one_based_frame, 4);
    assert!(!admitted.allows_game_mutation);

    disabled.interaction = Some(ButtonVisualState::Up);
    assert_eq!(
        admit_widget_visual(&disabled),
        Err(WidgetVisualRejection::DisabledStateMismatch)
    );
}

#[test]
fn navigation_is_the_only_eventful_read_only_presentation() {
    let mut nav = request(WidgetFamily::Button);
    nav.presentation = ReadOnlyPresentation::Navigation;
    nav.interaction = Some(ButtonVisualState::Over);
    nav.is_current_hover_target = true;
    let admitted = admit_widget_visual(&nav).unwrap();
    assert!(admitted.allows_navigation_events);
    assert!(!admitted.allows_game_mutation);
}

#[test]
fn drag_requires_a_hashable_scrollbar_snapshot() {
    let snapshot = DragSnapshot {
        source_record_id: "ui07:drag:42".into(),
        axis: ScrollbarAxis::Vertical,
        value: 42.0,
        thumb_x: 800,
        thumb_y: 311,
        pointer_x: 807,
        pointer_y: 318,
    };
    let mut scrollbar = request(WidgetFamily::ExtendedScrollbar);
    scrollbar.presentation = ReadOnlyPresentation::InertSnapshot;
    scrollbar.drag = Some(snapshot.clone());
    assert_eq!(
        admit_widget_visual(&scrollbar).unwrap().drag,
        Some(snapshot.clone())
    );

    let mut icon = request(WidgetFamily::Icon);
    icon.drag = Some(snapshot);
    assert_eq!(
        admit_widget_visual(&icon),
        Err(WidgetVisualRejection::DragOnNonScrollbar)
    );
}

#[test]
fn shipped_progress_predicates_preserve_boundary_inequalities() {
    let linear = ProgressFill {
        effect: ProgressEffect::Linear,
        current_state: 0.5,
        circular_mix: None,
    };
    assert_eq!(linear.foreground_at(0.5), Some(true));
    assert_eq!(linear.foreground_at(0.5001), Some(false));

    let reverse = ProgressFill {
        effect: ProgressEffect::Reverse,
        current_state: 0.25,
        circular_mix: None,
    };
    assert_eq!(reverse.foreground_at(0.75), Some(true));
    assert_eq!(reverse.foreground_at(0.7499), Some(false));

    let minmax_color = ProgressFill {
        effect: ProgressEffect::MinMaxColor,
        current_state: 0.25,
        circular_mix: None,
    };
    assert_eq!(minmax_color.foreground_at(0.25), Some(true));
    assert_eq!(minmax_color.foreground_at(0.40), Some(false));
    assert_eq!(minmax_color.foreground_at(0.75), Some(true));

    let minmax_texture = ProgressFill {
        effect: ProgressEffect::MinMaxTexture,
        current_state: 0.25,
        circular_mix: None,
    };
    assert_eq!(minmax_texture.foreground_at(0.25), Some(true));
    assert_eq!(minmax_texture.foreground_at(0.10), Some(false));
    assert_eq!(minmax_texture.foreground_at(0.50), Some(false));
}

#[test]
fn radial_and_circular_progress_use_exact_blend_carriers() {
    let radial = ProgressFill {
        effect: ProgressEffect::Radial,
        current_state: 0.0,
        circular_mix: None,
    };
    assert_eq!(radial.foreground_at(0.5), None);
    assert_eq!(radial.texture_two_mix(0.5, 0.0), Some(1.0));

    let circular = ProgressFill {
        effect: ProgressEffect::Circular,
        current_state: 0.0,
        circular_mix: Some(0.375),
    };
    assert_eq!(circular.texture_two_mix(0.1, 0.9), Some(0.375));
    assert!(circular.valid());
}

#[test]
fn attention_is_explicit_and_provenance_bound() {
    let mut icon = request(WidgetFamily::Icon);
    icon.attention = Some(AttentionCarrier::PoliticalGoalPulse {
        trigger_event: true,
    });
    let admitted = admit_widget_visual(&icon).unwrap();
    assert_eq!(admitted.attention, icon.attention);

    icon.source_record_id.clear();
    assert_eq!(
        admit_widget_visual(&icon),
        Err(WidgetVisualRejection::MissingProvenance)
    );
}
