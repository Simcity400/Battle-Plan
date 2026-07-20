//! Pointer-free S1 screen-shadow pass wiring.
//!
//! L02 owns the selected-light matrices and resource order; L03 owns the
//! ordered light-depth submissions. This module joins that admitted projection
//! to the exact W23 ping-pong screen-target chain and the recovered per-frame
//! blur constant-buffer producer. It plans resources and dependencies only;
//! it does not invent caster rows, terrain geometry, or texture contents.

use crate::light_depth_projection::{
    L03LightDepthProjection, L03LightDepthTarget, L03_LIGHT_DEPTH_DXGI_FORMAT,
    L03_LIGHT_DEPTH_HEIGHT, L03_LIGHT_DEPTH_MIP_LEVELS, L03_LIGHT_DEPTH_WIDTH,
};
use crate::shadow_law::{shadow_blur_constant_buffers, ShadowBlurConstantBuffers};
use crate::Result;
use serde::{Deserialize, Serialize};

/// W23 screen-shadow targets are `DXGI_FORMAT_B8G8R8A8_UNORM`.
pub const SHADOW_SCREEN_DXGI_FORMAT: u32 = 87;
pub const SHADOW_SCREEN_MIP_LEVELS: u32 = 1;
/// `D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET`.
pub const SHADOW_SCREEN_BIND_FLAGS: u32 = 0x28;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShadowScreenTarget {
    pub stable_resource_id: String,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub mip_levels: u32,
    pub bind_flags: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowScreenPassKind {
    TerrainUnlitProducer,
    HorizontalBlur,
    VerticalBlur,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShadowScreenPassPlan {
    pub kind: ShadowScreenPassKind,
    pub source_resource_id: String,
    pub target_resource_id: String,
    /// Present only for the two fullscreen blur passes.
    pub cb0_f32_bits: Option<[u32; 8]>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ShadowScreenChainPlan {
    pub light_depth_target: L03LightDepthTarget,
    pub producer_and_final_target: ShadowScreenTarget,
    pub blur_intermediate_target: ShadowScreenTarget,
    pub passes: Vec<ShadowScreenPassPlan>,
}

fn fail(message: impl Into<String>) -> Box<dyn std::error::Error> {
    message.into().into()
}

fn validate_screen_target(target: &ShadowScreenTarget, label: &str) -> Result<()> {
    if target.stable_resource_id.trim().is_empty() {
        return Err(fail(format!("S1 shadow chain blocked: {label} resource ID is missing")));
    }
    if target.width == 0 || target.height == 0 {
        return Err(fail(format!("S1 shadow chain blocked: {label} dimensions are empty")));
    }
    if target.dxgi_format != SHADOW_SCREEN_DXGI_FORMAT
        || target.mip_levels != SHADOW_SCREEN_MIP_LEVELS
        || target.bind_flags != SHADOW_SCREEN_BIND_FLAGS
    {
        return Err(fail(format!(
            "S1 shadow chain blocked: {label} must be format {} mip {} bind {:#x}",
            SHADOW_SCREEN_DXGI_FORMAT, SHADOW_SCREEN_MIP_LEVELS, SHADOW_SCREEN_BIND_FLAGS
        )));
    }
    Ok(())
}

/// Join an admitted L03 projection to the exact screen-shadow ping-pong chain.
///
/// A zero-fade L03 result emits no S1 chain, matching all 68 retained W23
/// skips. An active result requires two distinct, descriptor-identical screen
/// targets. The terrain-unlit producer writes the first target, horizontal blur
/// writes the second, and vertical blur writes back to the first; that first
/// target is therefore the resource consumed by main terrain and later passes.
pub fn plan_s1_shadow_screen_chain(
    projection: &L03LightDepthProjection,
    camera_y_f32_bits: Option<u32>,
    producer_and_final_target: Option<ShadowScreenTarget>,
    blur_intermediate_target: Option<ShadowScreenTarget>,
) -> Result<Option<ShadowScreenChainPlan>> {
    if projection.zero_fade_excluded {
        if projection.selected_light.is_some()
            || projection.light_depth_target.is_some()
            || !projection.submissions.is_empty()
        {
            return Err(fail(
                "S1 shadow chain blocked: zero-fade L03 projection contains active shadow work",
            ));
        }
        return Ok(None);
    }

    if projection.selected_light.is_none() {
        return Err(fail(
            "S1 shadow chain blocked: active L03 projection has no selected light",
        ));
    }
    let light_depth_target = projection.light_depth_target.as_ref().ok_or_else(|| {
        fail("S1 shadow chain blocked: active L03 projection has no light-depth target")
    })?;
    if light_depth_target.stable_resource_id.trim().is_empty()
        || light_depth_target.width != L03_LIGHT_DEPTH_WIDTH
        || light_depth_target.height != L03_LIGHT_DEPTH_HEIGHT
        || light_depth_target.dxgi_format != L03_LIGHT_DEPTH_DXGI_FORMAT
        || light_depth_target.mip_levels != L03_LIGHT_DEPTH_MIP_LEVELS
    {
        return Err(fail(
            "S1 shadow chain blocked: active L03 light-depth descriptor is not admitted",
        ));
    }
    let camera_y_bits = camera_y_f32_bits
        .ok_or_else(|| fail("S1 shadow chain blocked: active frame camera_y bits are missing"))?;
    let camera_y = f32::from_bits(camera_y_bits);
    if !camera_y.is_finite() {
        return Err(fail("S1 shadow chain blocked: active frame camera_y is not finite"));
    }
    let producer = producer_and_final_target.ok_or_else(|| {
        fail("S1 shadow chain blocked: producer/final screen target is missing")
    })?;
    let intermediate = blur_intermediate_target
        .ok_or_else(|| fail("S1 shadow chain blocked: blur intermediate target is missing"))?;
    validate_screen_target(&producer, "producer/final target")?;
    validate_screen_target(&intermediate, "blur intermediate target")?;
    if producer.width != intermediate.width || producer.height != intermediate.height {
        return Err(fail(
            "S1 shadow chain blocked: ping-pong screen target dimensions differ",
        ));
    }
    if producer.stable_resource_id == intermediate.stable_resource_id {
        return Err(fail(
            "S1 shadow chain blocked: ping-pong screen targets must have distinct identities",
        ));
    }
    if light_depth_target.stable_resource_id == producer.stable_resource_id
        || light_depth_target.stable_resource_id == intermediate.stable_resource_id
    {
        return Err(fail(
            "S1 shadow chain blocked: light-depth and screen targets alias",
        ));
    }

    let ShadowBlurConstantBuffers {
        horizontal_f32_bits,
        vertical_f32_bits,
    } = shadow_blur_constant_buffers(camera_y, producer.width, producer.height)?;
    let passes = vec![
        ShadowScreenPassPlan {
            kind: ShadowScreenPassKind::TerrainUnlitProducer,
            source_resource_id: light_depth_target.stable_resource_id.clone(),
            target_resource_id: producer.stable_resource_id.clone(),
            cb0_f32_bits: None,
        },
        ShadowScreenPassPlan {
            kind: ShadowScreenPassKind::HorizontalBlur,
            source_resource_id: producer.stable_resource_id.clone(),
            target_resource_id: intermediate.stable_resource_id.clone(),
            cb0_f32_bits: Some(horizontal_f32_bits),
        },
        ShadowScreenPassPlan {
            kind: ShadowScreenPassKind::VerticalBlur,
            source_resource_id: intermediate.stable_resource_id.clone(),
            target_resource_id: producer.stable_resource_id.clone(),
            cb0_f32_bits: Some(vertical_f32_bits),
        },
    ];

    Ok(Some(ShadowScreenChainPlan {
        light_depth_target: light_depth_target.clone(),
        producer_and_final_target: producer,
        blur_intermediate_target: intermediate,
        passes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light_depth_projection::L03SelectedLight;

    fn active_projection() -> L03LightDepthProjection {
        L03LightDepthProjection {
            selected_light: Some(L03SelectedLight::Sun),
            light_depth_target: Some(L03LightDepthTarget {
                stable_resource_id: "light-depth".into(),
                width: 2048,
                height: 2048,
                dxgi_format: 41,
                mip_levels: 1,
            }),
            zero_fade_excluded: false,
            excluded_particle_rows: 0,
            submissions: Vec::new(),
        }
    }

    fn screen_target(id: &str) -> ShadowScreenTarget {
        ShadowScreenTarget {
            stable_resource_id: id.into(),
            width: 1920,
            height: 1080,
            dxgi_format: SHADOW_SCREEN_DXGI_FORMAT,
            mip_levels: SHADOW_SCREEN_MIP_LEVELS,
            bind_flags: SHADOW_SCREEN_BIND_FLAGS,
        }
    }

    #[test]
    fn gate_s1_plan_preserves_exact_ping_pong_order() {
        let plan = plan_s1_shadow_screen_chain(
            &active_projection(),
            Some(50.000_007_629_394_53f32.to_bits()),
            Some(screen_target("screen-a")),
            Some(screen_target("screen-b")),
        )
        .unwrap()
        .unwrap();
        assert_eq!(plan.passes.len(), 3);
        assert_eq!(
            plan.passes
                .iter()
                .map(|pass| {
                    (
                        pass.kind,
                        pass.source_resource_id.as_str(),
                        pass.target_resource_id.as_str(),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (ShadowScreenPassKind::TerrainUnlitProducer, "light-depth", "screen-a"),
                (ShadowScreenPassKind::HorizontalBlur, "screen-a", "screen-b"),
                (ShadowScreenPassKind::VerticalBlur, "screen-b", "screen-a"),
            ]
        );
        assert!(plan.passes[0].cb0_f32_bits.is_none());
        assert!(plan.passes[1].cb0_f32_bits.is_some());
        assert!(plan.passes[2].cb0_f32_bits.is_some());
    }

    #[test]
    fn gate_s1_plan_skips_exact_zero_fade_and_rejects_aliasing() {
        let skipped = L03LightDepthProjection {
            selected_light: None,
            light_depth_target: None,
            zero_fade_excluded: true,
            excluded_particle_rows: 0,
            submissions: Vec::new(),
        };
        assert!(plan_s1_shadow_screen_chain(&skipped, None, None, None)
            .unwrap()
            .is_none());
        assert!(plan_s1_shadow_screen_chain(
            &active_projection(),
            Some(50.0f32.to_bits()),
            Some(screen_target("same")),
            Some(screen_target("same")),
        )
        .is_err());
    }
}
