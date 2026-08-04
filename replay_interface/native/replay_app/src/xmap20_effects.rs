//! XMAP20 normal-product execution boundary.
//!
//! This is the only app-side entry point for particle/effect draws. It binds
//! the current viewer projection and XMAP09/XMAP10-composed SceneCB0/MainScene;
//! selected-Present camera, constants, viewport and targets are never inputs.

use map_viewer::{GameFileResolver, PreparedXmap20Frame};
use replay_core::{
    canonical_json_sha256, ConstantBufferBinding, PreparedRenderTarget, Xmap20DependencyReceipts,
};
use serde::{Deserialize, Serialize};

use crate::d3d::Gfx;
use crate::submitted_draw::{SubmittedDrawStats, SubmittedMainSceneTarget};
use crate::Result;

pub const XMAP20_EXECUTION_RECEIPT_SCHEMA: &str = "hoi4.xmap20.execution.a729.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap20ExecutionReceipt {
    pub schema: String,
    pub hour_index: i64,
    pub admission_sha256: String,
    pub provider_receipts_sha256: String,
    pub scene_cb0_sha256: String,
    pub projection_cb1_sha256: String,
    pub color_target: PreparedRenderTarget,
    pub depth_target: PreparedRenderTarget,
    pub particle_draw_count: u32,
    pub deterministic_sha256: String,
}

impl Xmap20ExecutionReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP20_EXECUTION_RECEIPT_SCHEMA
            || self.hour_index < 0
            || self.admission_sha256.len() != 64
            || self.provider_receipts_sha256.len() != 64
            || self.scene_cb0_sha256.len() != 64
            || self.projection_cb1_sha256.len() != 64
        {
            return Err("XMAP20 execution receipt identity is incomplete".into());
        }
        for digest in [
            &self.admission_sha256,
            &self.provider_receipts_sha256,
            &self.scene_cb0_sha256,
            &self.projection_cb1_sha256,
            &self.deterministic_sha256,
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
        unsigned.deterministic_sha256.clear();
        if self.deterministic_sha256 != canonical_json_sha256(&unsigned)? {
            return Err("XMAP20 execution receipt digest mismatch".into());
        }
        Ok(())
    }
}

/// # Safety
/// All native target views and provider buffers must belong to `gfx.device`
/// and remain alive for this call.
pub unsafe fn execute_xmap20_effects(
    gfx: &Gfx,
    resolver: &GameFileResolver,
    prepared: &PreparedXmap20Frame,
    dependencies: &Xmap20DependencyReceipts,
    scene_constant_buffers: &[ConstantBufferBinding],
    viewer_projection_cb1: &ConstantBufferBinding,
    target: &SubmittedMainSceneTarget,
) -> Result<Xmap20ExecutionReceipt> {
    prepared.validate()?;
    dependencies
        .validate()
        .map_err(|error| format!("XMAP20 execution providers: {error}"))?;
    let provider_receipts_sha256 = canonical_json_sha256(dependencies)?;
    if prepared.admission.provider_receipts_sha256 != provider_receipts_sha256
        || prepared.admission.hour_index != dependencies.hour_index
        || prepared.content_manifest_sha256 != dependencies.content_manifest_sha256
        || prepared.product_modset_sha256 != dependencies.product_modset_sha256
        || prepared.effective_vfs_manifest_sha256
            != dependencies.effective_vfs_manifest_sha256
    {
        return Err(
            "XMAP20 execution providers/content epoch differ from preparation admission".into(),
        );
    }
    let color_target = target
        .color_identity
        .clone()
        .ok_or("XMAP20 MainScene color identity is absent")?;
    let depth_target = target
        .depth_identity
        .clone()
        .ok_or("XMAP20 MainScene depth identity is absent")?;
    let stats: SubmittedDrawStats = crate::submitted_draw::execute_xmap20_particles(
        gfx,
        resolver,
        prepared,
        scene_constant_buffers,
        viewer_projection_cb1,
        target,
    )?;
    if stats.entity_draws != 0
        || stats.direct_visual_joins != 0
        || stats.weather_joins != 0
        || stats.particle_draws != prepared.ordered_particles.len()
    {
        return Err("XMAP20 executed draw census differs from its prepared particle set".into());
    }
    let scene = scene_constant_buffers
        .first()
        .ok_or("XMAP20 SceneCB0 provider is absent")?;
    let mut receipt = Xmap20ExecutionReceipt {
        schema: XMAP20_EXECUTION_RECEIPT_SCHEMA.into(),
        hour_index: prepared.admission.hour_index,
        admission_sha256: prepared.admission.deterministic_sha256.clone(),
        provider_receipts_sha256,
        scene_cb0_sha256: scene.sha256.clone(),
        projection_cb1_sha256: viewer_projection_cb1.sha256.clone(),
        color_target,
        depth_target,
        particle_draw_count: u32::try_from(stats.particle_draws)?,
        deterministic_sha256: String::new(),
    };
    receipt.deterministic_sha256 = canonical_json_sha256(&receipt)?;
    receipt.validate()?;
    Ok(receipt)
}
