//! S1 carrier admission into the unified production D3D operation stream.
//!
//! Portable-render validation authenticates target descriptors, true-writer
//! edges, L03 draw arguments, and exact cross-family order against the selected
//! submit. This layer converts that carrier to the recovered shadow law, checks
//! exact H/V constant-buffer bytes, and seals the materialized operations that
//! the native executor will consume. Execution rechecks the seal before any
//! bridge call, so post-preparation mutation fails without partial GPU work.

use super::PreparedProductionD3dFrame;
use crate::light_depth_projection::{
    L03DrawArguments, L03LightDepthProjection, L03LightDepthSubmission,
    L03LightDepthTarget, L03SelectedLight, L03ShadowPipeline, L03TypedOwner,
};
use crate::shadow_pass_plan::{
    plan_s1_shadow_screen_chain, ShadowScreenChainPlan, ShadowScreenPassKind,
    ShadowScreenTarget,
};
use crate::{PreparedRuntimeDraw, Result};
use replay_core::{
    ConstantBufferBinding, RenderExecutionRef, S1LightDepthSubmission, S1SelectedLight,
    S1ShadowCarrier, S1ShadowPipeline, S1TypedOwner, StaticMapCommand,
};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedS1ShadowExecution {
    pub carrier: S1ShadowCarrier,
    pub plan: Option<ShadowScreenChainPlan>,
    /// Hash of canonical references plus the materialized operation/draw bytes.
    pub materialized_execution_sha256: Option<String>,
}

pub fn prepare_s1_shadow_execution(
    carrier: &S1ShadowCarrier,
    frame: &PreparedProductionD3dFrame,
) -> Result<PreparedS1ShadowExecution> {
    let plan = build_plan(carrier)?;
    let materialized_execution_sha256 = match &plan {
        None => None,
        Some(plan) => {
            validate_blur_constant_buffers(carrier, frame, plan)?;
            Some(materialized_execution_sha256(carrier, frame)?)
        }
    };
    Ok(PreparedS1ShadowExecution {
        carrier: carrier.clone(),
        plan,
        materialized_execution_sha256,
    })
}

pub fn validate_s1_shadow_execution(
    prepared: &PreparedS1ShadowExecution,
    frame: &PreparedProductionD3dFrame,
) -> Result<()> {
    if prepared.carrier.computed_payload_sha256()? != prepared.carrier.payload_sha256 {
        return Err("prepared S1 carrier payload SHA-256 drifted after admission".into());
    }
    let rebuilt_plan = build_plan(&prepared.carrier)?;
    if rebuilt_plan != prepared.plan {
        return Err("prepared S1 pass plan drifted after admission".into());
    }
    match (&prepared.plan, &prepared.materialized_execution_sha256) {
        (None, None) => Ok(()),
        (Some(plan), Some(expected)) => {
            validate_blur_constant_buffers(&prepared.carrier, frame, plan)?;
            let actual = materialized_execution_sha256(&prepared.carrier, frame)?;
            if &actual != expected {
                return Err("prepared S1 materialized execution stream drifted".into());
            }
            Ok(())
        }
        _ => Err("prepared S1 zero-fade/execution seal nullability mismatch".into()),
    }
}

fn build_plan(carrier: &S1ShadowCarrier) -> Result<Option<ShadowScreenChainPlan>> {
    let projection = portable_projection(carrier)?;
    let producer = carrier
        .producer_and_final_target
        .as_ref()
        .map(portable_screen_target);
    let intermediate = carrier
        .blur_intermediate_target
        .as_ref()
        .map(portable_screen_target);
    plan_s1_shadow_screen_chain(
        &projection,
        carrier.camera_y_f32_bits,
        producer,
        intermediate,
    )
}

fn portable_projection(carrier: &S1ShadowCarrier) -> Result<L03LightDepthProjection> {
    if carrier.is_zero_fade() {
        return Ok(L03LightDepthProjection {
            selected_light: None,
            light_depth_target: None,
            zero_fade_excluded: true,
            excluded_particle_rows: 0,
            submissions: Vec::new(),
        });
    }
    let selected_light = carrier
        .selected_light
        .map(|light| match light {
            S1SelectedLight::Sun => L03SelectedLight::Sun,
            S1SelectedLight::Moon => L03SelectedLight::Moon,
        })
        .ok_or("active S1 carrier has no selected light")?;
    let light = carrier
        .light_depth_target
        .as_ref()
        .ok_or("active S1 carrier has no light-depth target")?;
    let submissions = carrier
        .ordered_light_depth_submissions
        .iter()
        .map(portable_submission)
        .collect::<Result<Vec<_>>>()?;
    Ok(L03LightDepthProjection {
        selected_light: Some(selected_light),
        light_depth_target: Some(L03LightDepthTarget {
            stable_resource_id: light.stable_resource_id.clone(),
            width: light.width,
            height: light.height,
            dxgi_format: light.dxgi_format_u32,
            mip_levels: light.mip_levels,
        }),
        zero_fade_excluded: false,
        excluded_particle_rows: 0,
        submissions,
    })
}

fn portable_submission(row: &S1LightDepthSubmission) -> Result<L03LightDepthSubmission> {
    let map_pipeline = |pipeline| match pipeline {
        S1ShadowPipeline::StandardUnskinned => L03ShadowPipeline::StandardUnskinned,
        S1ShadowPipeline::StandardSkinned => L03ShadowPipeline::StandardSkinned,
        S1ShadowPipeline::FixedMapFurnitureBorder => {
            L03ShadowPipeline::FixedMapFurnitureBorder
        }
        S1ShadowPipeline::TreeShadow => L03ShadowPipeline::TreeShadow,
    };
    let map_draw = |draw: replay_core::S1DrawArguments| L03DrawArguments {
        index_count: draw.index_count,
        start_index: draw.start_index,
        base_vertex: draw.base_vertex,
        instance_count: draw.instance_count,
        start_instance: draw.start_instance,
    };
    Ok(match row {
        S1LightDepthSubmission::Pdxmesh {
            partition_mask,
            stable_typed_upstream_id,
            owner,
            world_matrix_f32_bits,
            selected_mesh,
            selected_submesh_material,
            group_ordinal,
            submesh_ordinal,
            pipeline,
            draw,
            ..
        } => L03LightDepthSubmission::Pdxmesh {
            partition_mask: *partition_mask,
            stable_typed_upstream_id: stable_typed_upstream_id.clone(),
            owner: match owner {
                S1TypedOwner::M01Unit => L03TypedOwner::M01Unit,
                S1TypedOwner::O04City => L03TypedOwner::O04City,
                S1TypedOwner::O05StaticFixed => L03TypedOwner::O05StaticFixed,
                S1TypedOwner::M04RailwayBridge => L03TypedOwner::M04RailwayBridge,
            },
            world_matrix_f32_bits: *world_matrix_f32_bits,
            selected_mesh: selected_mesh.clone(),
            selected_submesh_material: selected_submesh_material.clone(),
            group_ordinal: *group_ordinal,
            submesh_ordinal: *submesh_ordinal,
            pipeline: map_pipeline(*pipeline),
            draw: map_draw(*draw),
        },
        S1LightDepthSubmission::Tree {
            partition_mask,
            source_row_ordinal,
            selected_lod_index,
            selected_mesh,
            selected_material,
            pipeline,
            draw,
            ..
        } => L03LightDepthSubmission::Tree {
            partition_mask: *partition_mask,
            source_row_ordinal: *source_row_ordinal,
            selected_lod_index: *selected_lod_index,
            selected_mesh: selected_mesh.clone(),
            selected_material: selected_material.clone(),
            pipeline: map_pipeline(*pipeline),
            draw: map_draw(*draw),
        },
    })
}

fn portable_screen_target(target: &replay_core::S1TargetBinding) -> ShadowScreenTarget {
    ShadowScreenTarget {
        stable_resource_id: target.stable_resource_id.clone(),
        width: target.width,
        height: target.height,
        dxgi_format: target.dxgi_format_u32,
        mip_levels: target.mip_levels,
        bind_flags: target.bind_flags_u32,
    }
}

fn validate_blur_constant_buffers(
    carrier: &S1ShadowCarrier,
    frame: &PreparedProductionD3dFrame,
    plan: &ShadowScreenChainPlan,
) -> Result<()> {
    let horizontal_ref = carrier
        .horizontal_blur_draw
        .as_ref()
        .ok_or("active S1 carrier has no horizontal blur reference")?;
    let vertical_ref = carrier
        .vertical_blur_draw
        .as_ref()
        .ok_or("active S1 carrier has no vertical blur reference")?;
    let horizontal_bits = plan
        .passes
        .iter()
        .find(|pass| pass.kind == ShadowScreenPassKind::HorizontalBlur)
        .and_then(|pass| pass.cb0_f32_bits)
        .ok_or("S1 plan has no horizontal blur CB")?;
    let vertical_bits = plan
        .passes
        .iter()
        .find(|pass| pass.kind == ShadowScreenPassKind::VerticalBlur)
        .and_then(|pass| pass.cb0_f32_bits)
        .ok_or("S1 plan has no vertical blur CB")?;
    validate_blur_cb(frame, horizontal_ref, horizontal_bits, "horizontal")?;
    validate_blur_cb(frame, vertical_ref, vertical_bits, "vertical")
}

fn validate_blur_cb(
    frame: &PreparedProductionD3dFrame,
    reference: &RenderExecutionRef,
    expected_bits: [u32; 8],
    role: &str,
) -> Result<()> {
    let operation = static_operation(frame, reference, role)?;
    let StaticMapCommand::Draw { draw } = &operation.command else {
        return Err(format!("S1 {role} blur reference is not a draw").into());
    };
    let matches = draw
        .ordered_constant_buffers
        .iter()
        .filter(|binding| blur_cb_matches(binding, expected_bits))
        .count();
    if matches != 1 {
        return Err(format!(
            "S1 {role} blur draw does not carry exactly one exact PS-CB0 payload"
        )
        .into());
    }
    Ok(())
}

fn blur_cb_matches(binding: &ConstantBufferBinding, expected_bits: [u32; 8]) -> bool {
    binding.stage_mask_u8 == 0x02
        && binding.slot == 0
        && binding.byte_offset == 0
        && binding.byte_count == 32
        && binding.raw_bytes.len() == 32
        && binding
            .raw_bytes
            .chunks_exact(4)
            .zip(expected_bits)
            .all(|(bytes, bits)| bytes == bits.to_le_bytes())
}

fn static_operation<'a>(
    frame: &'a PreparedProductionD3dFrame,
    reference: &RenderExecutionRef,
    role: &str,
) -> Result<&'a replay_core::StaticMapOperation> {
    let RenderExecutionRef::Static { operation_ordinal } = reference else {
        return Err(format!("S1 {role} reference is not static").into());
    };
    let mut matches = frame
        .ordered_static_programs
        .iter()
        .flat_map(|program| program.ordered_operations.iter())
        .filter(|operation| operation.operation_ordinal == *operation_ordinal);
    let operation = matches
        .next()
        .ok_or_else(|| format!("S1 {role} operation is absent after materialization"))?;
    if matches.next().is_some() {
        return Err(format!("S1 {role} operation ordinal is duplicated").into());
    }
    Ok(operation)
}

#[derive(Serialize)]
struct MaterializedExecutionRow<'a> {
    reference: &'a RenderExecutionRef,
    payload: MaterializedExecutionPayload<'a>,
}

#[derive(Serialize)]
#[serde(tag = "family", rename_all = "snake_case")]
enum MaterializedExecutionPayload<'a> {
    Static(&'a StaticMapCommand),
    Runtime(&'a PreparedRuntimeDraw),
}

fn materialized_execution_sha256(
    carrier: &S1ShadowCarrier,
    frame: &PreparedProductionD3dFrame,
) -> Result<String> {
    let chain = carrier_execution_chain(carrier)?;
    let mut previous = None;
    let mut rows = Vec::with_capacity(chain.len());
    for reference in chain {
        let position = unique_execution_position(frame, reference)?;
        if previous.is_some_and(|prior| position <= prior) {
            return Err("S1 materialized operation order differs from admitted chain".into());
        }
        previous = Some(position);
        let payload = match reference {
            RenderExecutionRef::Static { .. } => {
                MaterializedExecutionPayload::Static(&static_operation(frame, reference, "chain")?.command)
            }
            RenderExecutionRef::Runtime { output } => {
                let mut matches = frame.ordered_runtime_draws.iter().filter(|draw| match draw {
                    PreparedRuntimeDraw::Particle { ordered_draw, .. }
                    | PreparedRuntimeDraw::Entity { ordered_draw, .. }
                    | PreparedRuntimeDraw::O04City { ordered_draw, .. }
                    | PreparedRuntimeDraw::O05Building { ordered_draw, .. } => {
                        ordered_draw == output
                    }
                });
                let draw = matches
                    .next()
                    .ok_or("S1 runtime operation is absent after materialization")?;
                if matches.next().is_some() {
                    return Err("S1 runtime operation is duplicated after materialization".into());
                }
                MaterializedExecutionPayload::Runtime(draw)
            }
            RenderExecutionRef::M02 { .. } | RenderExecutionRef::M03 { .. } => {
                return Err("S1 chain contains a UI or map-arrow execution family".into())
            }
        };
        rows.push(MaterializedExecutionRow { reference, payload });
    }
    Ok(Sha256::hex(&serde_json::to_vec(&rows)?))
}

fn carrier_execution_chain(carrier: &S1ShadowCarrier) -> Result<Vec<&RenderExecutionRef>> {
    if carrier.is_zero_fade() {
        return Ok(Vec::new());
    }
    let mut chain = Vec::new();
    chain.push(
        carrier
            .light_depth_clear
            .as_ref()
            .ok_or("active S1 light-depth clear is absent")?,
    );
    chain.extend(
        carrier
            .ordered_light_depth_submissions
            .iter()
            .map(S1LightDepthSubmission::execution),
    );
    chain.extend(carrier.ordered_terrain_unlit_draws.iter());
    chain.push(
        carrier
            .horizontal_blur_draw
            .as_ref()
            .ok_or("active S1 horizontal blur is absent")?,
    );
    chain.push(
        carrier
            .vertical_blur_draw
            .as_ref()
            .ok_or("active S1 vertical blur is absent")?,
    );
    chain.extend(carrier.ordered_shadow_consumers.iter());
    Ok(chain)
}

fn unique_execution_position(
    frame: &PreparedProductionD3dFrame,
    reference: &RenderExecutionRef,
) -> Result<usize> {
    let mut matches = frame
        .ordered_render_operations
        .iter()
        .enumerate()
        .filter(|(_, candidate)| *candidate == reference)
        .map(|(position, _)| position);
    let position = matches
        .next()
        .ok_or("S1 reference is absent from materialized unified order")?;
    if matches.next().is_some() {
        return Err("S1 reference repeats in materialized unified order".into());
    }
    Ok(position)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{
        S1_A729_EXECUTABLE_SHA256, S1_CHAIN_AUTHORITY_SHA256, S1_L02_AUTHORITY_SHA256,
        S1_L03_AUTHORITY_SHA256, S1_SHADOW_SCHEMA,
    };

    fn zero_fade_carrier() -> S1ShadowCarrier {
        let mut carrier = S1ShadowCarrier {
            schema: S1_SHADOW_SCHEMA.into(),
            executable_sha256: S1_A729_EXECUTABLE_SHA256.into(),
            l02_authority_sha256: S1_L02_AUTHORITY_SHA256.into(),
            l03_authority_sha256: S1_L03_AUTHORITY_SHA256.into(),
            s1_chain_authority_sha256: S1_CHAIN_AUTHORITY_SHA256.into(),
            shadow_fade_factor_f32_bits: 0.0f32.to_bits(),
            selected_light: None,
            light_view_projection_f32_bits: None,
            shadow_map_texture_matrix_f32_bits: None,
            camera_y_f32_bits: None,
            enabled_partition_mask: None,
            light_depth_target: None,
            producer_and_final_target: None,
            blur_intermediate_target: None,
            light_depth_clear: None,
            ordered_light_depth_submissions: Vec::new(),
            ordered_terrain_unlit_draws: Vec::new(),
            horizontal_blur_draw: None,
            vertical_blur_draw: None,
            ordered_shadow_consumers: Vec::new(),
            payload_sha256: String::new(),
        };
        carrier.payload_sha256 = carrier.computed_payload_sha256().unwrap();
        carrier
    }

    #[test]
    fn zero_fade_carrier_builds_an_exact_skip_plan() {
        let carrier = zero_fade_carrier();
        assert!(build_plan(&carrier).unwrap().is_none());
        assert!(carrier_execution_chain(&carrier).unwrap().is_empty());
    }
}
