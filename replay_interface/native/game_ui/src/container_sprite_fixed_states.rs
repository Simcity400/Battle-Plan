//! Fixed state for the reached a729 D3D11 ordinary GUI sprite operation.
//! The caller owns authenticated, ordered blend-row selection and the actual
//! surrounding GUI state schedule. This module does not select effect depth or
//! raster states, create a source locale witness, or infer AA from the target.
//! Existing fixed-state exports are pinned by portable_engine_evidence::map_scene_completion.
//! Candidate caller selection/order and remaining gaps are recorded in
//! extractor/native/runs/arrow_native_ui_20260908/CANDIDATE_EFFECT_CLOCK_LIFECYCLES.json.

use portable_render_stream::{
    BlendDesc, BlendFactor, BlendOperation, Comparison, CullMode, DepthDesc, FillMode,
    RasterizerDesc, StencilFaceDesc, StencilOperation,
};

/// An owning source-policy admission must establish this behavior. The viewer's
/// Rust ASCII functions do not themselves establish the native CRT locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeShaderStateTextPolicy {
    AsciiCaseFoldAndUppercase,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FixedStateRefusal {
    ScissorSelectionUnknown,
    RendererCreationAaUnknown,
    RendererCreationAaCapabilitiesUnknown { requested: u32 },
    NativeStateTextPolicyUnknown,
    NonAsciiStateText { field: String },
    UnknownStateField { field: String },
    InvalidBoolean { field: String, value: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OrdinaryUiFixedStates {
    pub blend: BlendDesc,
    pub depth: DepthDesc,
    pub rasterizer: RasterizerDesc,
    /// Native enum scanners report an error but leave the destination intact.
    /// Keep that distinction instead of assigning a guessed replacement.
    pub unchanged_invalid_enum_fields: Vec<(String, String)>,
}

/// `rows` must be the first native-comparison match from the original ordered
/// blend declarations, or empty after a witnessed unsuccessful name lookup.
/// None means witnessed no matching row, never an unavailable source census.
/// EffectFile's BTreeMap alone cannot prove selection when names repeat; the
/// caller may instead use its exact hash-admitted builtin no-duplicates law.
pub(crate) fn ordinary_ui_fixed_states(
    rows: Option<&[(String, String)]>,
    scissor: Option<bool>,
    renderer_creation_aa_request: Option<u32>,
    text_policy: Option<NativeShaderStateTextPolicy>,
) -> Result<OrdinaryUiFixedStates, FixedStateRefusal> {
    let rows = rows.unwrap_or(&[]);
    let scissor_enabled = scissor.ok_or(FixedStateRefusal::ScissorSelectionUnknown)?;
    let aa_request =
        renderer_creation_aa_request.ok_or(FixedStateRefusal::RendererCreationAaUnknown)?;
    // The settings reader compares its stored 32 bits as signed i32.
    if (aa_request as i32) >= 2 {
        return Err(FixedStateRefusal::RendererCreationAaCapabilitiesUnknown {
            requested: aa_request,
        });
    }
    if !rows.is_empty() && text_policy.is_none() {
        return Err(FixedStateRefusal::NativeStateTextPolicyUnknown);
    }
    // 1423f7110, followed by 1423fcb30. Alpha sentinel resolution occurs only
    // after all ordered assignments, even when explicitly reassigned to 1/0/0.
    let mut enabled = false;
    let mut color = (BlendFactor::One, BlendFactor::Zero, BlendOperation::Add);
    let mut alpha = (BlendFactor::One, BlendFactor::Zero, BlendOperation::Add);
    let mut write_mask = 15u8;
    let mut unchanged_invalid_enum_fields = Vec::new();
    for (key, value) in rows {
        if !key.is_ascii() || !value.is_ascii() {
            return Err(FixedStateRefusal::NonAsciiStateText { field: key.clone() });
        }
        match key.to_ascii_lowercase().as_str() {
            "blendenable" => enabled = native_bool(key, value)?,
            // Both native tokens assign the second byte. D3D11 lowering
            // deliberately does not read it; coverage is written as zero.
            "alphatocoverage" | "alphatest" => {
                native_bool(key, value)?;
            }
            "sourceblend" => {
                assign_factor(&mut color.0, key, value, &mut unchanged_invalid_enum_fields)
            }
            "destblend" => {
                assign_factor(&mut color.1, key, value, &mut unchanged_invalid_enum_fields)
            }
            "blendop" => {
                assign_operation(&mut color.2, key, value, &mut unchanged_invalid_enum_fields)
            }
            "sourcealpha" => {
                assign_factor(&mut alpha.0, key, value, &mut unchanged_invalid_enum_fields)
            }
            "destalpha" => {
                assign_factor(&mut alpha.1, key, value, &mut unchanged_invalid_enum_fields)
            }
            "blendopalpha" => {
                assign_operation(&mut alpha.2, key, value, &mut unchanged_invalid_enum_fields)
            }
            "writemask" => {
                // 1423f81b0: native uppercase, then substring searches, not
                // tokenization by '|'. ALL anywhere wins; unknown text => 0.
                let text = value.to_ascii_uppercase();
                write_mask = if text.contains("ALL") {
                    15
                } else {
                    u8::from(text.contains("RED"))
                        | (u8::from(text.contains("GREEN")) << 1)
                        | (u8::from(text.contains("BLUE")) << 2)
                        | (u8::from(text.contains("ALPHA")) << 3)
                };
            }
            _ => return Err(FixedStateRefusal::UnknownStateField { field: key.clone() }),
        }
    }
    if alpha == (BlendFactor::One, BlendFactor::Zero, BlendOperation::Add) {
        alpha = color;
    }
    let face = StencilFaceDesc {
        fail_operation: StencilOperation::Keep,
        depth_fail_operation: StencilOperation::Keep,
        pass_operation: StencilOperation::Keep,
        comparison: Comparison::Always,
    };
    Ok(OrdinaryUiFixedStates {
        blend: BlendDesc {
            alpha_to_coverage_enabled: false,
            independent_blend_enabled: false,
            enabled,
            source_color: color.0,
            destination_color: color.1,
            color_operation: color.2,
            source_alpha: alpha.0,
            destination_alpha: alpha.1,
            alpha_operation: alpha.2,
            write_mask_rgba_u8: write_mask,
            additional_targets: Vec::new(),
            // 142400090 passes NULL BlendFactor: D3D11 stores/uses ones.
            blend_factor_rgba_f32_bits: [0x3f80_0000; 4],
            sample_mask_u32: u32::MAX,
        },
        depth: DepthDesc {
            enabled: false,
            write_enabled: false,
            comparison: Comparison::LessEqual,
            stencil_enabled: false,
            stencil_read_mask_u8: 255,
            stencil_write_mask_u8: 255,
            front_face: face.clone(),
            back_face: face,
            stencil_reference_u32: 0,
        },
        rasterizer: RasterizerDesc {
            fill: FillMode::Solid,
            cull: CullMode::Back,
            front_counter_clockwise: false,
            depth_clip_enabled: true,
            scissor_enabled,
            depth_bias_i32: 0,
            depth_bias_clamp_f32_bits: 0,
            slope_scaled_depth_bias_f32_bits: 0,
            multisample_enabled: false,
            antialiased_line_enabled: false,
        },
        unchanged_invalid_enum_fields,
    })
}

fn native_bool(key: &str, value: &str) -> Result<bool, FixedStateRefusal> {
    // 1424b06e0 compares exact current token bytes with yes\0/no\0.
    match value {
        "yes" => Ok(true),
        "no" => Ok(false),
        _ => Err(FixedStateRefusal::InvalidBoolean {
            field: key.into(),
            value: value.into(),
        }),
    }
}

fn assign_factor(
    destination: &mut BlendFactor,
    key: &str,
    value: &str,
    invalid: &mut Vec<(String, String)>,
) {
    let factor = match value.to_ascii_lowercase().as_str() {
        "zero" => BlendFactor::Zero,
        "one" => BlendFactor::One,
        "src_color" => BlendFactor::SourceColor,
        "inv_src_color" => BlendFactor::InverseSourceColor,
        "src_alpha" => BlendFactor::SourceAlpha,
        "inv_src_alpha" => BlendFactor::InverseSourceAlpha,
        "dest_alpha" => BlendFactor::DestinationAlpha,
        "inv_dest_alpha" => BlendFactor::InverseDestinationAlpha,
        "dest_color" => BlendFactor::DestinationColor,
        "inv_dest_color" => BlendFactor::InverseDestinationColor,
        "src_alpha_sat" => BlendFactor::SourceAlphaSaturate,
        "blend_factor" => BlendFactor::BlendFactor,
        "inv_blend_factor" => BlendFactor::InverseBlendFactor,
        "src1_color" => BlendFactor::Source1Color,
        "inv_src1_color" => BlendFactor::InverseSource1Color,
        "src1_alpha" => BlendFactor::Source1Alpha,
        "inv_src1_alpha" => BlendFactor::InverseSource1Alpha,
        _ => {
            invalid.push((key.into(), value.into()));
            return;
        }
    };
    *destination = factor;
}

fn assign_operation(
    destination: &mut BlendOperation,
    key: &str,
    value: &str,
    invalid: &mut Vec<(String, String)>,
) {
    let operation = match value.to_ascii_lowercase().as_str() {
        "blend_op_add" => BlendOperation::Add,
        "blend_op_subtract" => BlendOperation::Subtract,
        "blend_op_rev_subtract" => BlendOperation::ReverseSubtract,
        "blend_op_min" => BlendOperation::Minimum,
        "blend_op_max" => BlendOperation::Maximum,
        _ => {
            invalid.push((key.into(), value.into()));
            return;
        }
    };
    *destination = operation;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(values: &[(&str, &str)]) -> Vec<(String, String)> {
        values
            .iter()
            .map(|(a, b)| ((*a).into(), (*b).into()))
            .collect()
    }
    fn build(values: &[(&str, &str)]) -> OrdinaryUiFixedStates {
        ordinary_ui_fixed_states(
            Some(&rows(values)),
            Some(false),
            Some(0),
            Some(NativeShaderStateTextPolicy::AsciiCaseFoldAndUppercase),
        )
        .unwrap()
    }

    #[test]
    fn missing_selected_blend_constructs_native_values_without_text_policy() {
        let state = ordinary_ui_fixed_states(None, Some(false), Some(1), None).unwrap();
        assert!(!state.blend.enabled);
        assert_eq!(state.blend.source_alpha, BlendFactor::One);
        assert_eq!(state.blend.destination_alpha, BlendFactor::Zero);
        assert_eq!(state.blend.write_mask_rgba_u8, 15);
        assert_eq!(state.blend.blend_factor_rgba_f32_bits, [1f32.to_bits(); 4]);
        assert_eq!(state.blend.sample_mask_u32, u32::MAX);
        assert!(!state.depth.enabled && !state.depth.write_enabled);
        assert_eq!(state.rasterizer.cull, CullMode::Back);
    }

    #[test]
    fn static_button_and_buttonstate_assignments_copy_alpha_sentinel() {
        let s = build(&[
            ("BlendEnable", "yes"),
            ("AlphaTest", "no"),
            ("SourceBlend", "src_alpha"),
            ("DestBlend", "inv_src_alpha"),
        ]);
        assert!(s.blend.enabled);
        assert_eq!(s.blend.source_color, BlendFactor::SourceAlpha);
        assert_eq!(s.blend.destination_color, BlendFactor::InverseSourceAlpha);
        assert_eq!(s.blend.source_alpha, s.blend.source_color);
        assert_eq!(s.blend.destination_alpha, s.blend.destination_color);
        assert_eq!(s.blend.alpha_operation, BlendOperation::Add);
    }

    #[test]
    fn exact_alpha_names_override_and_explicit_sentinel_still_copies() {
        let s = build(&[
            ("SourceBlend", "src_alpha"),
            ("SourceAlpha", "one"),
            ("DestAlpha", "zero"),
            ("BlendOpAlpha", "blend_op_add"),
        ]);
        assert_eq!(s.blend.source_alpha, BlendFactor::SourceAlpha);
        let s = build(&[
            ("SourceAlpha", "dest_alpha"),
            ("BlendOpAlpha", "blend_op_max"),
        ]);
        assert_eq!(s.blend.source_alpha, BlendFactor::DestinationAlpha);
        assert_eq!(s.blend.alpha_operation, BlendOperation::Maximum);
        assert!(matches!(
            ordinary_ui_fixed_states(
                Some(&rows(&[("SourceBlendAlpha", "one")])),
                Some(false),
                Some(0),
                Some(NativeShaderStateTextPolicy::AsciiCaseFoldAndUppercase)
            ),
            Err(FixedStateRefusal::UnknownStateField { .. })
        ));
    }

    #[test]
    fn both_legacy_alpha_tokens_leave_d3d_coverage_disabled() {
        let s = build(&[("AlphaToCoverage", "yes"), ("AlphaTest", "yes")]);
        assert!(!s.blend.alpha_to_coverage_enabled && !s.blend.independent_blend_enabled);
    }

    #[test]
    fn native_invalid_enum_retains_previous_assignment_in_physical_order() {
        let s = build(&[
            ("SourceBlend", "dest_color"),
            ("SourceBlend", "invalid"),
            ("BlendOp", "blend_op_max"),
            ("BlendOp", "ADD"),
        ]);
        assert_eq!(s.blend.source_color, BlendFactor::DestinationColor);
        assert_eq!(s.blend.color_operation, BlendOperation::Maximum);
        assert_eq!(
            s.unchanged_invalid_enum_fields,
            rows(&[("SourceBlend", "invalid"), ("BlendOp", "ADD")])
        );
    }

    #[test]
    fn native_write_mask_is_uppercase_substring_and_can_be_measured_zero() {
        assert_eq!(
            build(&[("WriteMask", "REDGREEN")]).blend.write_mask_rgba_u8,
            3
        );
        assert_eq!(
            build(&[("WriteMask", "SMALL")]).blend.write_mask_rgba_u8,
            15
        );
        assert_eq!(
            build(&[("WriteMask", "not-a-color")])
                .blend
                .write_mask_rgba_u8,
            0
        );
    }

    #[test]
    fn unknown_creation_scissor_and_text_policy_are_distinct() {
        assert_eq!(
            ordinary_ui_fixed_states(None, None, Some(0), None),
            Err(FixedStateRefusal::ScissorSelectionUnknown)
        );
        assert_eq!(
            ordinary_ui_fixed_states(None, Some(false), None, None),
            Err(FixedStateRefusal::RendererCreationAaUnknown)
        );
        assert_eq!(
            ordinary_ui_fixed_states(None, Some(false), Some(2), None),
            Err(FixedStateRefusal::RendererCreationAaCapabilitiesUnknown { requested: 2 })
        );
        assert_eq!(
            ordinary_ui_fixed_states(
                Some(&rows(&[("BlendEnable", "yes")])),
                Some(false),
                Some(0),
                None
            ),
            Err(FixedStateRefusal::NativeStateTextPolicyUnknown)
        );
        let s = ordinary_ui_fixed_states(None, Some(true), Some(0), None).unwrap();
        assert!(s.rasterizer.scissor_enabled);
        assert!(!s.rasterizer.multisample_enabled && !s.rasterizer.antialiased_line_enabled);
        let negative = ordinary_ui_fixed_states(None, Some(false), Some(u32::MAX), None).unwrap();
        assert!(!negative.rasterizer.multisample_enabled);
    }
}
