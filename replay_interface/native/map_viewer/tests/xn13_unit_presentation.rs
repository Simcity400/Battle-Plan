use map_viewer::{
    admit_m01_runtime_rows, admit_xn13_rows, M01A14ControlState, M01AirBaseProvinceIfWing,
    M01NegativeRuntimeControl, M01PositiveRuntimeRow, M01RuntimeRowPair, M01UnitKind,
    M01VisibilityOrIntelState, Xn13AnimationBinding, Xn13DamageVariant, Xn13EffectKind,
    Xn13EffectTrigger, Xn13PassPoses, Xn13PoseBinding, Xn13PresentationRow, Xn13SelectionEffect,
};

fn admitted_m01() -> map_viewer::M01AdmittedRuntimeRows {
    let pair = M01RuntimeRowPair {
        positive: M01PositiveRuntimeRow {
            clock: 42,
            stable_unit_id: "division:1".into(),
            unit_kind: M01UnitKind::Division,
            a7_selected_entity: "infantry_entity".into(),
            a7_selected_model: "infantry_mesh".into(),
            a7_air_base_province_if_wing: M01AirBaseProvinceIfWing::NotWing,
            a14_pov_tag: "GER".into(),
            a14_visibility_or_intel_state: M01VisibilityOrIntelState::Visible,
            a14_spotted_decision: true,
            render_included: true,
            immutable_ib_hash64: "0123456789abcdef".into(),
            world_matrix_le_hex_64_bytes: "00".repeat(64),
            pre_color_draw_event: 100,
            shadow_draw_event: 200,
            post_color_draw_event: 300,
        },
        negative_control: M01NegativeRuntimeControl {
            same_stable_unit_id: "division:1".into(),
            changed_a14_pov_or_spotted_state: M01A14ControlState {
                pov_tag: "FRA".into(),
                visibility_or_intel_state: M01VisibilityOrIntelState::Hidden,
                spotted_decision: false,
            },
            render_included_false: false,
            explicit_exclusion_reason: "not visible to POV".into(),
            zero_draw_memberships_for_that_clock: true,
        },
    };
    admit_m01_runtime_rows(Some(&[pair])).unwrap()
}

fn palette() -> Xn13PoseBinding {
    Xn13PoseBinding::Palette {
        sha256: "5a312281df4bd8dfbb4d4a94ad0bf44d01bb8cfced1206b90e21b4ca0568cdb1".into(),
        le_hex_3200_bytes: "00".repeat(3200),
    }
}

fn row() -> Xn13PresentationRow {
    Xn13PresentationRow {
        clock: 42,
        stable_unit_id: "division:1".into(),
        entity_name: "infantry_entity".into(),
        model_name: "infantry_mesh".into(),
        state_name: "move".into(),
        state_enter_clock: 40,
        animation: Xn13AnimationBinding::Transition {
            from_state: "idle".into(),
            from_animation: "infantry_idle_animation".into(),
            from_phase_f32_bits: 0.25f32.to_bits(),
            to_state: "move".into(),
            to_animation: "infantry_move_animation".into(),
            to_phase_f32_bits: 0.5f32.to_bits(),
            elapsed_f32_bits: 0.1f32.to_bits(),
            duration_f32_bits: 0.2f32.to_bits(),
        },
        selected_lod_index: 0,
        camera_distance_f32_bits: 300.0f32.to_bits(),
        selected_loddist_f32_bits: 500.0f32.to_bits(),
        material_variant: "default".into(),
        damage_variant: Xn13DamageVariant::Intact,
        selection_effect: Xn13SelectionEffect::NotSelected,
        state_effect_triggers: vec![Xn13EffectTrigger {
            state_event_index: 0,
            effect_kind: Xn13EffectKind::Particle,
            effect_token: "dust_particle".into(),
            node: "root".into(),
            event_time_f32_bits: 0.0f32.to_bits(),
            keep_particle: true,
            trigger_once: false,
        }],
        pass_poses: Xn13PassPoses {
            color_a: palette(),
            light_depth: palette(),
            main_hdr: palette(),
        },
    }
}

#[test]
fn admits_exact_hash_verified_three_pass_pose() {
    let admitted = admit_xn13_rows(&[row()], &admitted_m01()).unwrap();
    assert_eq!(admitted.rows().len(), 1);
}

#[test]
fn rejects_palette_hash_mismatch() {
    let mut input = row();
    if let Xn13PoseBinding::Palette { sha256, .. } = &mut input.pass_poses.main_hdr {
        *sha256 = "00".repeat(32);
    }
    assert!(admit_xn13_rows(&[input], &admitted_m01()).is_err());
}

#[test]
fn rejects_mixed_skinned_pass_tags() {
    let mut input = row();
    input.pass_poses.light_depth = Xn13PoseBinding::NotSkinned;
    assert!(admit_xn13_rows(&[input], &admitted_m01()).is_err());
}

#[test]
fn rejects_xn13_m01_row_set_drift() {
    let mut input = row();
    input.stable_unit_id = "division:2".into();
    assert!(admit_xn13_rows(&[input], &admitted_m01()).is_err());
}

#[test]
fn rejects_transition_elapsed_after_duration() {
    let mut input = row();
    if let Xn13AnimationBinding::Transition {
        elapsed_f32_bits, ..
    } = &mut input.animation
    {
        *elapsed_f32_bits = 0.3f32.to_bits();
    }
    assert!(admit_xn13_rows(&[input], &admitted_m01()).is_err());
}
