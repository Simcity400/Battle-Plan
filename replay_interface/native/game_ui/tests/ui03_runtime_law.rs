use game_ui::gui_runtime_law::{
    advance_position_animation, animation_type_from_runtime_code, eased_position_progress,
    evaluate_render2d_node, gui_object_is_visible, propagate_container_visibility,
    propagate_fixed_window_visibility, write_gui_object_visible, GraphicalNodeState,
    GraphicalRootVisibility, PositionAnimationError, PositionAnimationState, PositionAnimationType,
    PositionAxis,
};

fn state(kind: PositionAnimationType, elapsed: f64, duration: f64) -> PositionAnimationState {
    PositionAnimationState {
        runtime_code: kind.runtime_code(),
        elapsed,
        duration,
        axes: [
            PositionAxis {
                target: 100,
                delta: 100,
            },
            PositionAxis {
                target: -100,
                delta: -100,
            },
        ],
        deferred_hide: false,
    }
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
}

#[test]
fn logical_visibility_writer_preserves_every_unrelated_bit() {
    let original = 0b1011_0101;
    let shown = write_gui_object_visible(original, true);
    assert!(gui_object_is_visible(shown));
    assert_eq!(shown & !0x08, original & !0x08);
    let hidden = write_gui_object_visible(shown, false);
    assert!(!gui_object_is_visible(hidden));
    assert_eq!(hidden & !0x08, original & !0x08);
}

#[test]
fn container_and_fixed_window_bridge_logical_and_graphical_state() {
    let mut flags = 0x80;
    let mut roots = [
        GraphicalRootVisibility {
            descendants_enabled: false,
        },
        GraphicalRootVisibility {
            descendants_enabled: false,
        },
    ];
    propagate_container_visibility(&mut flags, &mut roots, true);
    assert!(gui_object_is_visible(flags));
    assert!(roots.iter().all(|root| root.descendants_enabled));
    let mut root = GraphicalRootVisibility {
        descendants_enabled: true,
    };
    propagate_fixed_window_visibility(&mut flags, &mut root, false);
    assert!(!gui_object_is_visible(flags));
    assert!(!root.descendants_enabled);
}

#[test]
fn render2d_local_append_and_descendant_traversal_are_independent() {
    let structural = evaluate_render2d_node(GraphicalNodeState {
        locally_renderable: false,
        descendants_enabled: true,
        child_count: 2,
    });
    assert!(!structural.append_primary);
    assert!(structural.traverse_descendants);

    let local_only = evaluate_render2d_node(GraphicalNodeState {
        locally_renderable: true,
        descendants_enabled: false,
        child_count: 2,
    });
    assert!(local_only.append_primary);
    assert!(!local_only.traverse_descendants);
}

#[test]
fn render2d_non_renderable_leaf_is_pruned() {
    let decision = evaluate_render2d_node(GraphicalNodeState {
        locally_renderable: false,
        descendants_enabled: true,
        child_count: 0,
    });
    assert!(decision.is_pruned_leaf());
}

#[test]
fn authored_and_runtime_animation_codes_are_exact() {
    let rows = [
        ("accelerated", PositionAnimationType::Accelerated, 0, 1),
        ("decelerated", PositionAnimationType::Decelerated, 1, 2),
        ("smoothstep", PositionAnimationType::Smoothstep, 2, 3),
        ("linear", PositionAnimationType::Linear, 3, 4),
    ];
    for (text, kind, authored, runtime) in rows {
        assert_eq!(PositionAnimationType::parse_authored(text), kind);
        assert_eq!(kind.authored_code(), authored);
        assert_eq!(kind.runtime_code(), runtime);
        assert_eq!(animation_type_from_runtime_code(runtime), Some(kind));
    }
    assert_eq!(
        PositionAnimationType::parse_authored("unknown"),
        PositionAnimationType::Decelerated
    );
    assert_eq!(animation_type_from_runtime_code(0), None);
}

#[test]
fn all_four_easing_laws_match_the_proven_formulas() {
    let t: f64 = 0.25;
    close(
        eased_position_progress(PositionAnimationType::Accelerated, t),
        t.powi(4),
    );
    close(
        eased_position_progress(PositionAnimationType::Decelerated, t),
        (1.0 - (1.0 + 4.0 * t).powi(-2)) / 0.96,
    );
    let once = (3.0 - 2.0 * t) * t * t;
    let twice = (3.0 - 2.0 * once) * once * once;
    close(
        eased_position_progress(PositionAnimationType::Smoothstep, t),
        twice,
    );
    close(eased_position_progress(PositionAnimationType::Linear, t), t);
}

#[test]
fn smoothstep_clamps_before_both_hermite_applications() {
    assert_eq!(
        eased_position_progress(PositionAnimationType::Smoothstep, -1.0),
        0.0
    );
    assert_eq!(
        eased_position_progress(PositionAnimationType::Smoothstep, 2.0),
        1.0
    );
}

#[test]
fn coordinate_reconstruction_truncates_toward_zero_for_both_signs() {
    let sample =
        advance_position_animation(state(PositionAnimationType::Linear, 0.0, 3.0), 1.0).unwrap();
    // E=1/3; remaining deltas are +66.66 and -66.66, truncated to +66/-66.
    assert_eq!(sample.position, [34, -34]);
    assert!(!sample.complete);
}

#[test]
fn elapsed_equal_duration_is_not_complete_but_writes_target_via_easing() {
    let sample =
        advance_position_animation(state(PositionAnimationType::Linear, 0.5, 1.0), 0.5).unwrap();
    assert_eq!(sample.position, [100, -100]);
    assert_eq!(sample.elapsed, 1.0);
    assert!(!sample.complete);
}

#[test]
fn elapsed_greater_than_duration_completes_and_performs_deferred_hide() {
    let mut input = state(PositionAnimationType::Accelerated, 1.0, 1.0);
    input.deferred_hide = true;
    let sample = advance_position_animation(input, 0.001).unwrap();
    assert_eq!(sample.position, [100, -100]);
    assert!(sample.complete);
    assert!(sample.perform_deferred_hide);
}

#[test]
fn inactive_animation_writes_exact_target() {
    let mut input = state(PositionAnimationType::Linear, 0.5, 1.0);
    input.runtime_code = 0;
    let sample = advance_position_animation(input, 0.25).unwrap();
    assert_eq!(sample.position, [100, -100]);
    assert!(sample.complete);
}

#[test]
fn updater_rejects_states_that_cannot_follow_the_engine_path() {
    let mut input = state(PositionAnimationType::Linear, 0.0, 0.0);
    assert_eq!(
        advance_position_animation(input, 0.0),
        Err(PositionAnimationError::NonPositiveDuration)
    );
    input.duration = 1.0;
    input.runtime_code = 9;
    assert_eq!(
        advance_position_animation(input, 0.0),
        Err(PositionAnimationError::InvalidRuntimeCode)
    );
    input.runtime_code = PositionAnimationType::Linear.runtime_code();
    assert_eq!(
        advance_position_animation(input, -0.1),
        Err(PositionAnimationError::NegativeElapsedOrDelta)
    );
}
