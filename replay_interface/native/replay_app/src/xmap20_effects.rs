use map_viewer::{NativeProductionMainSceneTarget, Xmap10PreParticleSceneReceipt};
use replay_core::{canonical_json_sha256, AssignedStage, PreparedRenderTarget};
use serde::{Deserialize, Serialize};

use crate::Result;

pub const XMAP20_EXECUTION_RECEIPT_SCHEMA: &str = "hoi4.xmap20.authored-execution.a729.v1";
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
    pub authored_content_sha256: String,
    pub scene_providers_sha256: String,
    pub scene_cb0_sha256: String,
    pub projection_cb1_sha256: String,
    pub lifecycle_sha256: String,
    pub submission_sha256: String,
    pub color_target: PreparedRenderTarget,
    pub depth_target: PreparedRenderTarget,
    pub particle_draw_count: u64,
    pub deterministic_sha256: String,
}

impl Xmap20ExecutionReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP20_EXECUTION_RECEIPT_SCHEMA
            || self.hour_index < 0
            || self.operation_id != XMAP20_OPERATION_NAME
            || self.main_scene_allocation_generation == 0
            || self.authored_content_sha256.len() != 64
            || self.scene_providers_sha256.len() != 64
            || self.scene_cb0_sha256.len() != 64
            || self.projection_cb1_sha256.len() != 64
        {
            return Err("XMAP20 execution receipt identity is incomplete".into());
        }
        for digest in [
            &self.authored_content_sha256,
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
            &self.scene_providers_sha256,
            &self.scene_cb0_sha256,
            &self.projection_cb1_sha256,
            &self.lifecycle_sha256,
            &self.submission_sha256,
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

pub fn seal_authored_execution(
    hour_index: i64,
    assigned_stage: &AssignedStage,
    shared: &Xmap20SharedExecutionIdentities,
    pre_particle_scene: &Xmap10PreParticleSceneReceipt,
    projection_cb1_sha256: String,
    target: &NativeProductionMainSceneTarget,
    submissions: AuthoredSubmissions,
) -> Result<Xmap20ExecutionReceipt> {
    assigned_stage.validate()?;
    shared.validate()?;
    pre_particle_scene.validate()?;
    target.desc().validate()?;
    if assigned_stage.stage_name != XMAP20_STAGE_NAME
        || assigned_stage.operation_name != XMAP20_OPERATION_NAME
        || shared.main_scene_target_sha256 != canonical_json_sha256(target.desc())?
        || shared.main_scene_resource_identity_sha256 != target.resource_identity_sha256()
        || shared.main_scene_allocation_generation != target.allocation_generation()
        || shared.pre_particle_scene_receipt_sha256 != pre_particle_scene.deterministic_sha256
    {
        return Err("XMAP20 authored execution target/stage/pre-particle join mismatch".into());
    }
    let mut receipt = Xmap20ExecutionReceipt {
        schema: XMAP20_EXECUTION_RECEIPT_SCHEMA.into(),
        foundation_envelope_sha256: shared.foundation_envelope_sha256.clone(),
        hour_index,
        operation_id: assigned_stage.operation_name.clone(),
        topology_sha256: assigned_stage.topology_sha256.clone(),
        assignment_sha256: assigned_stage.assignment_sha256.clone(),
        content_manifest_sha256: shared.content_manifest_sha256.clone(),
        foundation_content_operation_sha256: shared.foundation_content_operation_sha256.clone(),
        viewer_state_sha256: shared.viewer_state_sha256.clone(),
        main_scene_target_sha256: shared.main_scene_target_sha256.clone(),
        main_scene_resource_identity_sha256: shared.main_scene_resource_identity_sha256.clone(),
        main_scene_allocation_generation: shared.main_scene_allocation_generation,
        pre_particle_scene_receipt_sha256: shared.pre_particle_scene_receipt_sha256.clone(),
        map_copy_receipt_set_sha256: shared.map_copy_receipt_set_sha256.clone(),
        authored_content_sha256: shared.content_manifest_sha256.clone(),
        scene_providers_sha256: canonical_json_sha256(&(
            &shared.main_scene_resource_identity_sha256,
            &shared.pre_particle_scene_receipt_sha256,
            &shared.map_copy_receipt_set_sha256,
            &projection_cb1_sha256,
        ))?,
        scene_cb0_sha256: pre_particle_scene.scene_cb0.sha256.clone(),
        projection_cb1_sha256,
        lifecycle_sha256: canonical_json_sha256(&(
            &shared.foundation_envelope_sha256,
            &shared.viewer_state_sha256,
            submissions.instances,
            &submissions.digest,
        ))?,
        submission_sha256: submissions.digest,
        color_target: PreparedRenderTarget::OpaqueRenderTarget {
            attachment_ordinal: target.desc().color_attachment_ordinal,
        },
        depth_target: PreparedRenderTarget::OpaqueRenderTarget {
            attachment_ordinal: target.desc().depth_attachment_ordinal,
        },
        particle_draw_count: submissions.counts.draw_calls,
        deterministic_sha256: String::new(),
    };
    receipt.deterministic_sha256 = canonical_json_sha256(&receipt)?;
    receipt.validate()?;
    Ok(receipt)
}

pub struct AuthoredSubmissions {
    instances: u64,
    digest: String,
    pub counts: map_viewer::ParticleDrawCounts,
}

impl Default for AuthoredSubmissions {
    fn default() -> Self {
        Self {
            instances: 0,
            digest: snapshot::sha256::Sha256::hex(b"XMAP20.authored-submissions.a729.v1"),
            counts: Default::default(),
        }
    }
}

impl AuthoredSubmissions {
    pub fn record(
        &mut self,
        instance_id: &str,
        capacity: &map_viewer::particle_authored::ParticleDrawCapacity,
        view: map_viewer::particle_runtime::ParticleGpuView<'_>,
        submission: map_viewer::ParticleDrawReceipt,
    ) -> Result<()> {
        if instance_id.is_empty()
            || view.presentation_id.is_none()
            || view.sorted_presentation_id() != view.presentation_id
        {
            return Err("XMAP20 authored submission identity/phase is unavailable".into());
        }
        let counts = submission.counts();
        self.instances = self
            .instances
            .checked_add(1)
            .ok_or("XMAP20 instance count overflow")?;
        for (total, count) in [
            (&mut self.counts.packed_subsystems, counts.packed_subsystems),
            (&mut self.counts.draw_calls, counts.draw_calls),
            (&mut self.counts.normal_instances, counts.normal_instances),
            (&mut self.counts.trail_vertices, counts.trail_vertices),
        ] {
            *total = total
                .checked_add(count)
                .ok_or("XMAP20 draw count overflow")?;
        }
        self.digest = canonical_json_sha256(&(
            &self.digest,
            self.instances,
            instance_id,
            view.definition().name(),
            view.presentation_id,
            view.world_matrix_f32_bits(),
            (capacity.packets, capacity.particles),
            counts,
            submission.submission_sha256(),
        ))?;
        Ok(())
    }
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
