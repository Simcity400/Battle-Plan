
use map_viewer::{
    GameFileResolver, NativeProductionMainSceneTarget, PreparedXmap20Frame,
    Xmap10PreParticleSceneReceipt,
};
use replay_core::{
    canonical_json_sha256, AssignedStage, ConstantBufferBinding, PreparedRenderTarget,
    Xmap20DependencyReceipts,
};
use serde::{Deserialize, Serialize};

use crate::d3d::Gfx;
use crate::submitted_draw::{SubmittedDrawStats, SubmittedMainSceneTarget};
use crate::Result;

pub const XMAP20_EXECUTION_RECEIPT_SCHEMA: &str = "hoi4.xmap20.execution.a729.v1";
pub const XMAP20_STAGE_NAME: &str = "particles_effects";
pub const XMAP20_OPERATION_NAME: &str = "particles.effects.draw";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap20SharedExecutionIdentities {
    /// The foundation envelope this draw belongs to.
    ///
    /// The hour, content manifest, foundation content operation and viewer
    /// state identify a replay hour and what is being looked at; two DIFFERENT
    /// envelopes can agree on all four, so on their own they let a receipt
    /// produced for one frame be accepted into another frame's bundle (Codex
    /// review, round 14). The envelope digest is what says which frame.
    pub foundation_envelope_sha256: String,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub main_scene_resource_identity_sha256: String,
    pub main_scene_allocation_generation: u64,
    pub pre_particle_scene_receipt_sha256: String,
    pub map_copy_receipt_set_sha256: String,
}

impl Xmap20SharedExecutionIdentities {
    pub fn validate(&self) -> Result<()> {
        for digest in [
            &self.foundation_envelope_sha256,
            &self.content_manifest_sha256,
            &self.foundation_content_operation_sha256,
            &self.viewer_state_sha256,
            &self.main_scene_target_sha256,
            &self.main_scene_resource_identity_sha256,
            &self.pre_particle_scene_receipt_sha256,
            &self.map_copy_receipt_set_sha256,
        ] {
            validate_sha256(digest)?;
        }
        if self.main_scene_allocation_generation == 0 {
            return Err("XMAP20 shared MainScene allocation generation is zero".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap20ExecutionReceipt {
    pub schema: String,
    /// Which foundation envelope this draw belongs to. See
    /// `Xmap20SharedExecutionIdentities::foundation_envelope_sha256`.
    pub foundation_envelope_sha256: String,
    pub hour_index: i64,
    pub operation_id: String,
    pub topology_sha256: String,
    pub assignment_sha256: String,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub main_scene_resource_identity_sha256: String,
    pub main_scene_allocation_generation: u64,
    pub pre_particle_scene_receipt_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub admission_sha256: String,
    pub provider_receipts_sha256: String,
    pub scene_cb0_sha256: String,
    pub projection_cb1_sha256: String,
    pub prepared_particle_census_sha256: String,
    pub executed_particle_order_sha256: String,
    pub color_target: PreparedRenderTarget,
    pub depth_target: PreparedRenderTarget,
    pub particle_draw_count: u32,
    pub packet_draw_receipt_sha256: String,
}

impl Xmap20ExecutionReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP20_EXECUTION_RECEIPT_SCHEMA
            || self.hour_index < 0
            || self.operation_id != XMAP20_OPERATION_NAME
            || self.particle_draw_count == 0
            || self.main_scene_allocation_generation == 0
            || self.admission_sha256.len() != 64
            || self.provider_receipts_sha256.len() != 64
            || self.scene_cb0_sha256.len() != 64
            || self.projection_cb1_sha256.len() != 64
        {
            return Err("XMAP20 execution receipt identity is incomplete".into());
        }
        for digest in [
            &self.admission_sha256,
            &self.foundation_envelope_sha256,
            &self.topology_sha256,
            &self.assignment_sha256,
            &self.content_manifest_sha256,
            &self.foundation_content_operation_sha256,
            &self.viewer_state_sha256,
            &self.main_scene_target_sha256,
            &self.main_scene_resource_identity_sha256,
            &self.pre_particle_scene_receipt_sha256,
            &self.map_copy_receipt_set_sha256,
            &self.provider_receipts_sha256,
            &self.scene_cb0_sha256,
            &self.projection_cb1_sha256,
            &self.prepared_particle_census_sha256,
            &self.executed_particle_order_sha256,
            &self.packet_draw_receipt_sha256,
        ] {
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err("XMAP20 execution receipt contains an invalid digest".into());
            }
        }
        if !matches!(
            &self.color_target,
            PreparedRenderTarget::OpaqueRenderTarget { .. }
        ) || !matches!(
            &self.depth_target,
            PreparedRenderTarget::OpaqueRenderTarget { .. }
        ) {
            return Err("XMAP20 execution receipt targets are not MainScene attachments".into());
        }
        let mut unsigned = self.clone();
        unsigned.packet_draw_receipt_sha256.clear();
        if self.packet_draw_receipt_sha256 != canonical_json_sha256(&unsigned)? {
            return Err("XMAP20 execution receipt digest mismatch".into());
        }
        Ok(())
    }

    pub fn validate_for(
        &self,
        prepared: &PreparedXmap20Frame,
        dependencies: &Xmap20DependencyReceipts,
        assigned_stage: &AssignedStage,
        shared: &Xmap20SharedExecutionIdentities,
        target: &NativeProductionMainSceneTarget,
        pre_particle_scene: &Xmap10PreParticleSceneReceipt,
    ) -> Result<()> {
        self.validate()?;
        prepared.validate()?;
        dependencies
            .validate()
            .map_err(|error| format!("XMAP20 receipt providers: {error}"))?;
        assigned_stage
            .validate()
            .map_err(|error| format!("XMAP20 receipt assigned stage: {error}"))?;
        shared.validate()?;
        target.desc().validate()?;
        pre_particle_scene.validate()?;
        let target_desc = target.desc();
        if target_desc.color_format == 0 || target_desc.depth_format == 0 {
            return Err("XMAP20 receipt target has an unknown color/depth format".into());
        }
        let expected_color = PreparedRenderTarget::OpaqueRenderTarget {
            attachment_ordinal: target_desc.color_attachment_ordinal,
        };
        let expected_depth = PreparedRenderTarget::OpaqueRenderTarget {
            attachment_ordinal: target_desc.depth_attachment_ordinal,
        };
        if self.hour_index != prepared.admission.hour_index
            || self.operation_id != assigned_stage.operation_name
            || self.topology_sha256 != assigned_stage.topology_sha256
            || self.assignment_sha256 != assigned_stage.assignment_sha256
            || self.foundation_envelope_sha256 != shared.foundation_envelope_sha256
            || self.content_manifest_sha256 != prepared.content_manifest_sha256
            || self.content_manifest_sha256 != shared.content_manifest_sha256
            || self.foundation_content_operation_sha256
                != shared.foundation_content_operation_sha256
            || self.viewer_state_sha256 != shared.viewer_state_sha256
            || self.main_scene_target_sha256 != shared.main_scene_target_sha256
            || self.main_scene_target_sha256 != canonical_json_sha256(target_desc)?
            || self.main_scene_resource_identity_sha256
                != shared.main_scene_resource_identity_sha256
            || self.main_scene_resource_identity_sha256 != target.resource_identity_sha256()
            || self.main_scene_allocation_generation != shared.main_scene_allocation_generation
            || self.main_scene_allocation_generation != target.allocation_generation()
            || self.pre_particle_scene_receipt_sha256
                != shared.pre_particle_scene_receipt_sha256
            || self.pre_particle_scene_receipt_sha256
                != pre_particle_scene.deterministic_sha256
            || self.scene_cb0_sha256 != pre_particle_scene.scene_cb0.sha256
            || self.color_target != expected_color
            || self.depth_target != expected_depth
            || self.map_copy_receipt_set_sha256 != shared.map_copy_receipt_set_sha256
            || self.admission_sha256 != prepared.admission.deterministic_sha256
            || self.provider_receipts_sha256 != canonical_json_sha256(dependencies)?
            || self.projection_cb1_sha256 != dependencies.camera.descriptor_sha256
            || self.prepared_particle_census_sha256
                != prepared_particle_census_sha256(prepared)?
            || self.executed_particle_order_sha256 != particle_order_sha256(prepared)?
            || usize::try_from(self.particle_draw_count)? != prepared.ordered_particles.len()
        {
            return Err(
                "XMAP20 execution receipt differs from prepared/provider/schedule authority"
                    .into(),
            );
        }
        Ok(())
    }
}

pub unsafe fn execute_xmap20_effects(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    prepared: &PreparedXmap20Frame,
    dependencies: &Xmap20DependencyReceipts,
    assigned_stage: &AssignedStage,
    shared: &Xmap20SharedExecutionIdentities,
    pre_particle_scene: &Xmap10PreParticleSceneReceipt,
    viewer_projection_cb1: &ConstantBufferBinding,
    target: &NativeProductionMainSceneTarget,
) -> Result<Xmap20ExecutionReceipt> {
    prepared.validate()?;
    assigned_stage
        .validate()
        .map_err(|error| format!("XMAP20 assigned stage: {error}"))?;
    shared.validate()?;
    pre_particle_scene.validate()?;
    if target.device_identity() != gfx.device.as_raw() as usize
        || target.resource_identity_sha256() != shared.main_scene_resource_identity_sha256
        || target.allocation_generation() != shared.main_scene_allocation_generation
        || pre_particle_scene.deterministic_sha256
            != shared.pre_particle_scene_receipt_sha256
    {
        return Err("XMAP20 typed MainScene/pre-particle allocation join mismatch".into());
    }
    if assigned_stage.stage_name != XMAP20_STAGE_NAME
        || assigned_stage.operation_name != XMAP20_OPERATION_NAME
        || prepared.ordered_particles.is_empty()
    {
        return Err(
            "XMAP20 target execution requires its exact assigned stage and a non-empty census"
                .into(),
        );
    }
    dependencies
        .validate()
        .map_err(|error| format!("XMAP20 execution providers: {error}"))?;
    if viewer_projection_cb1.sha256 != dependencies.camera.descriptor_sha256 {
        return Err(
            "XMAP20 projection CB1 differs from the exact XMAP03 camera descriptor".into(),
        );
    }
    let provider_receipts_sha256 = canonical_json_sha256(dependencies)?;
    if prepared.admission.provider_receipts_sha256 != provider_receipts_sha256
        || prepared.admission.hour_index != dependencies.hour_index
        || prepared.content_manifest_sha256 != dependencies.content_manifest_sha256
        || prepared.product_modset_sha256 != dependencies.product_modset_sha256
        || prepared.effective_vfs_manifest_sha256
            != dependencies.effective_vfs_manifest_sha256
        || shared.content_manifest_sha256 != prepared.content_manifest_sha256
    {
        return Err(
            "XMAP20 execution providers/content epoch differ from preparation admission".into(),
        );
    }
    let submitted_target = SubmittedMainSceneTarget::from_native(target);
    let color_target = submitted_target
        .color_identity
        .clone()
        .ok_or("XMAP20 MainScene color identity is absent")?;
    let depth_target = submitted_target
        .depth_identity
        .clone()
        .ok_or("XMAP20 MainScene depth identity is absent")?;
    let color_attachment_ordinal = match &color_target {
        PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal } => *attachment_ordinal,
        PreparedRenderTarget::SwapchainBackBuffer => {
            return Err("XMAP20 MainScene color target is the swapchain".into())
        }
    };
    let depth_attachment_ordinal = match &depth_target {
        PreparedRenderTarget::OpaqueRenderTarget { attachment_ordinal } => *attachment_ordinal,
        PreparedRenderTarget::SwapchainBackBuffer => {
            return Err("XMAP20 MainScene depth target is the swapchain".into())
        }
    };
    let actual_target = target.desc();
    if color_attachment_ordinal != actual_target.color_attachment_ordinal
        || depth_attachment_ordinal != actual_target.depth_attachment_ordinal
        || shared.main_scene_target_sha256 != canonical_json_sha256(actual_target)?
    {
        return Err("XMAP20 shared MainScene digest differs from the actual native target".into());
    }
    let stats: SubmittedDrawStats = crate::submitted_draw::execute_xmap20_particles(
        gfx,
        resolver,
        prepared,
        std::slice::from_ref(&pre_particle_scene.scene_cb0),
        viewer_projection_cb1,
        &submitted_target,
    )?;
    if stats.entity_draws != 0
        || stats.weather_joins != 0
        || stats.particle_draws != prepared.ordered_particles.len()
    {
        return Err("XMAP20 executed draw census differs from its prepared particle set".into());
    }
    let scene = &pre_particle_scene.scene_cb0;
    let prepared_particle_census_sha256 = prepared_particle_census_sha256(prepared)?;
    let executed_particle_order_sha256 = particle_order_sha256(prepared)?;
    let mut receipt = Xmap20ExecutionReceipt {
        schema: XMAP20_EXECUTION_RECEIPT_SCHEMA.into(),
        foundation_envelope_sha256: shared.foundation_envelope_sha256.clone(),
        hour_index: prepared.admission.hour_index,
        operation_id: assigned_stage.operation_name.clone(),
        topology_sha256: assigned_stage.topology_sha256.clone(),
        assignment_sha256: assigned_stage.assignment_sha256.clone(),
        content_manifest_sha256: shared.content_manifest_sha256.clone(),
        foundation_content_operation_sha256: shared
            .foundation_content_operation_sha256
            .clone(),
        viewer_state_sha256: shared.viewer_state_sha256.clone(),
        main_scene_target_sha256: shared.main_scene_target_sha256.clone(),
        main_scene_resource_identity_sha256: shared
            .main_scene_resource_identity_sha256
            .clone(),
        main_scene_allocation_generation: shared.main_scene_allocation_generation,
        pre_particle_scene_receipt_sha256: shared
            .pre_particle_scene_receipt_sha256
            .clone(),
        map_copy_receipt_set_sha256: shared.map_copy_receipt_set_sha256.clone(),
        admission_sha256: prepared.admission.deterministic_sha256.clone(),
        provider_receipts_sha256,
        scene_cb0_sha256: scene.sha256.clone(),
        projection_cb1_sha256: viewer_projection_cb1.sha256.clone(),
        prepared_particle_census_sha256,
        executed_particle_order_sha256,
        color_target,
        depth_target,
        particle_draw_count: u32::try_from(stats.particle_draws)?,
        packet_draw_receipt_sha256: String::new(),
    };
    receipt.packet_draw_receipt_sha256 = canonical_json_sha256(&receipt)?;
    receipt.validate_for(
        prepared,
        dependencies,
        assigned_stage,
        shared,
        target,
        pre_particle_scene,
    )?;
    Ok(receipt)
}

fn prepared_particle_census_sha256(prepared: &PreparedXmap20Frame) -> Result<String> {
    canonical_json_sha256(&prepared.admission.ordered_particle_submissions).map_err(Into::into)
}

fn particle_order_sha256(prepared: &PreparedXmap20Frame) -> Result<String> {
    canonical_json_sha256(
        &prepared
            .ordered_particles
            .iter()
            .map(|packet| {
                (
                    packet.effect_instance_id,
                    packet.bucket,
                    packet.packet_ordinal,
                    packet.draw_order,
                    packet.canonical_packet_sha256.as_str(),
                )
            })
            .collect::<Vec<_>>(),
    )
    .map_err(Into::into)
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("XMAP20 shared execution identity is not lowercase SHA-256".into());
    }
    Ok(())
}
