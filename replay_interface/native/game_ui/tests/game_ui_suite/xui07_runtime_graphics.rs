use game_ui::runtime_graphics::*;

fn px(values: [f32; 4]) -> RgbaF32 {
    RgbaF32(values)
}

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

fn request(path: ShadingPath) -> RuntimeGraphicsRequest {
    RuntimeGraphicsRequest {
        source_record_id: "frame-1".into(),
        sprite_name: "GFX_test".into(),
        frame_count: 4,
        one_based_frame: 1,
        interaction_effect: ButtonVisualState::Up,
        shading_path: path,
        producer_resolved_rgba: None,
        color: RgbaF32([1.0; 4]),
        time: 0.0,
        animation_time: 0.0,
        next_one_based_frame: (path == ShadingPath::ButtonStateBlendFrames).then_some(2),
        clip: ClipRect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 50,
        },
        opacity: 1.0,
    }
}

#[test]
fn button_interaction_effect_is_independent_of_animation_frame() {
    assert_eq!(ButtonVisualState::from_raw(0), Some(ButtonVisualState::Up));
    assert_eq!(
        ButtonVisualState::from_raw(1),
        Some(ButtonVisualState::Down)
    );
    assert_eq!(
        ButtonVisualState::from_raw(2),
        Some(ButtonVisualState::Over)
    );
    assert_eq!(
        ButtonVisualState::from_raw(3),
        Some(ButtonVisualState::Disable)
    );
    assert_eq!(ButtonVisualState::from_raw(4), None);
    let mut down = request(ShadingPath::ButtonState);
    down.interaction_effect = ButtonVisualState::Down;
    down.one_based_frame = 3;
    let admitted = admit_runtime_graphics(&down).unwrap();
    assert_eq!(admitted.interaction_effect(), ButtonVisualState::Down);
    assert_eq!(admitted.frame().one_based_frame, 3);
}

#[test]
fn frame_selection_is_one_based_and_fail_closed() {
    assert_eq!(select_one_based_frame(1, 4).unwrap().zero_based_index, 0);
    assert_eq!(select_one_based_frame(4, 4).unwrap().zero_based_index, 3);
    assert_eq!(select_one_based_frame(0, 4), None);
    assert_eq!(select_one_based_frame(5, 4), None);
}

#[test]
fn static_button_modulates_sample_by_color_after_frame_selection() {
    assert_eq!(
        shade_static_button(px([0.5, 0.25, 1.0, 0.8]), px([0.2, 0.4, 0.5, 0.5])),
        px([0.1, 0.1, 0.5, 0.4])
    );
}

#[test]
fn static_button_all_four_effect_names_share_the_shipped_pixel_shader() {
    let base = px([0.5, 0.25, 1.0, 0.8]);
    let color = px([0.2, 0.4, 0.5, 0.5]);
    let expected = shade_static_button(base, color);
    for state in [
        ButtonVisualState::Up,
        ButtonVisualState::Down,
        ButtonVisualState::Over,
        ButtonVisualState::Disable,
    ] {
        let mut value = request(ShadingPath::StaticButton);
        value.interaction_effect = state;
        assert!(admit_runtime_graphics(&value).is_ok());
        assert_eq!(shade_static_button(base, color), expected);
    }
}

#[test]
fn standard_down_and_over_use_distinct_transition_speeds() {
    let base = px([0.25, 0.5, 0.75, 0.8]);
    let color = px([1.0; 4]);
    let down = shade_buttonstate(base, color, ButtonVisualState::Down, 0.025, 0.0);
    let over = shade_buttonstate(base, color, ButtonVisualState::Over, 0.025, 0.0);
    assert!(down.0[0] < base.0[0]);
    assert!(over.0[0] > base.0[0]);
    assert_ne!(base.0[0] - down.0[0], over.0[0] - base.0[0]);
    close(down.0[3], 0.8);
}

#[test]
fn standard_disable_grayscale_precedes_color_modulation() {
    let out = shade_buttonstate(
        px([1.0, 0.0, 0.0, 0.5]),
        px([0.5, 1.0, 0.25, 0.5]),
        ButtonVisualState::Disable,
        0.0,
        0.0,
    );
    close(out.0[0], 0.212671 * 0.5);
    close(out.0[1], 0.212671);
    close(out.0[2], 0.212671 * 0.25);
    close(out.0[3], 0.25);
}

#[test]
fn blendframes_lerps_before_state_shading() {
    let out = shade_buttonstate_blendframes(
        px([0.0, 0.0, 0.0, 0.0]),
        px([1.0, 1.0, 1.0, 1.0]),
        0.25,
        px([1.0; 4]),
        ButtonVisualState::Up,
    );
    assert_eq!(out, px([0.25; 4]));
}

#[test]
fn blendframes_down_and_over_do_not_apply_color_constant() {
    let color = px([0.0; 4]);
    let down = shade_buttonstate_blendframes(
        px([0.4; 4]),
        px([0.4; 4]),
        0.0,
        color,
        ButtonVisualState::Down,
    );
    let over = shade_buttonstate_blendframes(
        px([0.4; 4]),
        px([0.4; 4]),
        0.0,
        color,
        ButtonVisualState::Over,
    );
    assert!(down.0[0] > 0.0);
    assert!(over.0[0] > 0.4);
}

#[test]
fn sprite_animation_add_and_multiply_match_shader() {
    let dest = px([0.2, 0.3, 0.4, 0.5]);
    let source = px([0.8, 0.6, 0.4, 0.25]);
    let add = blend_sprite_animation(dest, source, SpriteAnimationBlendMode::Add);
    close(add.0[0], 0.4);
    close(add.0[3], 0.5);
    let multiply = blend_sprite_animation(dest, source, SpriteAnimationBlendMode::Multiply);
    close(multiply.0[0], 0.16);
    close(multiply.0[3], 0.125);
}

#[test]
fn sprite_animation_overlay_branches_per_channel() {
    let out = blend_sprite_animation(
        px([0.25, 0.75, 0.5, 0.2]),
        px([0.4, 0.4, 0.4, 0.8]),
        SpriteAnimationBlendMode::Overlay,
    );
    close(out.0[0], 0.2);
    close(out.0[1], 0.7);
    close(out.0[2], 0.4);
    close(out.0[3], 0.8);
}

#[test]
fn sprite_animation_normal_supports_transparent_destination() {
    let out = blend_sprite_animation(
        px([0.8, 0.4, 0.2, 0.5]),
        px([0.2, 0.6, 1.0, 0.25]),
        SpriteAnimationBlendMode::Normal,
    );
    close(out.0[0], 0.35);
    close(out.0[3], 0.5);
}

#[test]
fn clip_intersection_uses_half_open_pixel_bounds() {
    let a = ClipRect {
        left: 0,
        top: 0,
        right: 10,
        bottom: 10,
    };
    let b = ClipRect {
        left: 5,
        top: -2,
        right: 12,
        bottom: 6,
    };
    let clip = a.intersect(b).unwrap();
    assert_eq!(
        clip,
        ClipRect {
            left: 5,
            top: 0,
            right: 10,
            bottom: 6
        }
    );
    assert!(clip.contains_pixel(5, 0));
    assert!(!clip.contains_pixel(10, 0));
}

#[test]
fn opacity_changes_source_alpha_before_fixed_function_blend() {
    let source = apply_opacity(px([1.0, 0.0, 0.0, 0.8]), 0.5);
    let out = blend_src_alpha_inv_src_alpha(px([0.0, 0.0, 1.0, 1.0]), source);
    close(out.0[0], 0.4);
    close(out.0[2], 0.6);
    close(out.0[3], 0.4);
}

#[test]
fn admitted_builtin_request_carries_exact_frame() {
    let admitted = admit_runtime_graphics(&request(ShadingPath::ButtonState)).unwrap();
    assert_eq!(admitted.frame().zero_based_index, 0);
    assert_eq!(admitted.shading_path(), ShadingPath::ButtonState);
    assert_eq!(admitted.clip().right, 100);
    assert_eq!(admitted.opacity(), 1.0);
    assert_eq!(admitted.producer_resolved_rgba(), None);
    assert_eq!(admitted.interaction_effect(), ButtonVisualState::Up);
}

#[test]
fn unknown_effect_requires_producer_resolved_finite_rgba() {
    let mut value = request(ShadingPath::ProducerResolved);
    assert_eq!(
        admit_runtime_graphics(&value),
        Err(RuntimeGraphicsRejection::MissingProducerResolvedShading)
    );
    value.producer_resolved_rgba = Some(px([0.1, 0.2, 0.3, 0.4]));
    assert!(admit_runtime_graphics(&value).is_ok());
}

#[test]
fn admission_rejects_missing_provenance_frame_clip_and_opacity() {
    let mut value = request(ShadingPath::StaticButton);
    value.source_record_id.clear();
    assert_eq!(
        admit_runtime_graphics(&value),
        Err(RuntimeGraphicsRejection::MissingProvenance)
    );
    value = request(ShadingPath::StaticButton);
    value.one_based_frame = 0;
    assert_eq!(
        admit_runtime_graphics(&value),
        Err(RuntimeGraphicsRejection::InvalidFrame)
    );
    value = request(ShadingPath::StaticButton);
    value.clip.right = 0;
    assert_eq!(
        admit_runtime_graphics(&value),
        Err(RuntimeGraphicsRejection::InvalidClip)
    );
    value = request(ShadingPath::StaticButton);
    value.opacity = 1.1;
    assert_eq!(
        admit_runtime_graphics(&value),
        Err(RuntimeGraphicsRejection::InvalidOpacity)
    );
}
