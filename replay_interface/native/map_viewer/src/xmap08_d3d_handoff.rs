//! Real app-device execution for the XMAP08 river and strait programs.
//!
//! XMAP08 owns its static textures, geometry, shader programs, sampler state,
//! and fixed pipeline state. Runtime terrain, water, lighting, and
//! screen-shadow resources remain typed inputs from
//! XMAP04/XMAP07/XMAP09/XMAP10. The executor refuses a draw unless every
//! reflected resource and constant buffer is present and content-addressed.

use crate::{
    production_features::xmap08::{
        reflected_cbuffer_variable_is_used, reflected_constant_buffer_requirements,
        reflected_resource_requirements, validate_program_draw_dependencies,
        XMAP08_RIVER_OPERATION_ID, XMAP08_RIVER_STAGE_NAME, XMAP08_STRAIT_OPERATION_ID,
        XMAP08_STRAIT_STAGE_NAME,
    },
    AssignedStage, FoundationRenderEnvelope, GameFileResolver, MapConstantBufferBinding,
    MapDependentResourceBinding, NativeProductionMainSceneTarget, NativeXmap05PoliticalHandoff,
    PreparedMapEffectProgram, PreparedMapSamplerRequirement, PreparedRiversStraitsFrame,
    ProductionMainSceneTargetDesc, ProductionMapCameraProjection, ReadyRiversStraitsDrawContract,
    Result, ViewerMapCameraState, ViewerOwnedMapState,
};
use portable_content_manifest::foundation::FoundationTextureDescriptor;
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

const XMAP08_TARGET_BOUND_DRAW_SCHEMA: &str = "hoi4.xmap08.target-bound-draw.v1";
const XMAP08_COMPONENT_CLOSURE_SCHEMA: &str = "hoi4.xmap08.component-closure.v1";
const XMAP08_RIVER_COMPONENT_ID: &str = "xmap08.rivers";
const XMAP08_STRAIT_COMPONENT_ID: &str = "xmap08.straits";
fn validate_assigned_operation(
    operation: &AssignedStage,
    stage_name: &str,
    operation_id: &str,
) -> Result<()> {
    operation
        .validate()
        .map_err(|error| format!("XMAP08 assigned operation: {error}"))?;
    if operation.stage_name != stage_name || operation.operation_name != operation_id {
        return Err(format!(
            "XMAP08 assigned operation {:?}/{:?} does not match required {stage_name}/{operation_id}",
            operation.stage_name, operation.operation_name
        )
        .into());
    }
    Ok(())
}

fn validate_bound_assignment_fields(
    component_id: &str,
    stage_name: &str,
    operation_id: &str,
    assigned_ordinal: u32,
    topology_sha256: &str,
    assignment_sha256: &str,
) -> Result<()> {
    let (required_stage, required_operation) = match component_id {
        XMAP08_RIVER_COMPONENT_ID => (XMAP08_RIVER_STAGE_NAME, XMAP08_RIVER_OPERATION_ID),
        XMAP08_STRAIT_COMPONENT_ID => (XMAP08_STRAIT_STAGE_NAME, XMAP08_STRAIT_OPERATION_ID),
        _ => return Err(format!("XMAP08 component id {component_id:?} is unsupported").into()),
    };
    if stage_name != required_stage || operation_id != required_operation {
        return Err(format!(
            "XMAP08 component {component_id} is bound to {stage_name}/{operation_id}, expected {required_stage}/{required_operation}"
        )
        .into());
    }
    let expected = AssignedStage::seal(topology_sha256, stage_name, operation_id, assigned_ordinal)
        .map_err(|error| format!("XMAP08 bound assigned stage: {error}"))?;
    if expected.assignment_sha256 != assignment_sha256 {
        return Err("XMAP08 bound assigned-stage digest mismatch".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap08ExecutionFrameAuthority {
    pub hour_index: i64,
    pub replay_frame_identity_sha256: String,
    pub political_frame_sha256: String,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub main_scene_resource_identity_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub river_assignment: AssignedStage,
    pub strait_assignment: AssignedStage,
    pub deterministic_sha256: String,
}

impl Xmap08ExecutionFrameAuthority {
    pub fn from_validated_inputs(
        envelope: &FoundationRenderEnvelope,
        viewer: &ViewerOwnedMapState,
        target: &ProductionMainSceneTargetDesc,
        main_scene_resource_identity_sha256: &str,
        terrain: &crate::PreparedTerrainFrame,
        political: &crate::PreparedPoliticalFrame,
        prepared: &PreparedRiversStraitsFrame,
    ) -> Result<Self> {
        envelope
            .validate()
            .map_err(|error| format!("XMAP08 foundation envelope: {error}"))?;
        viewer.validate()?;
        target.validate()?;
        terrain.validate()?;
        political.validate()?;
        prepared.validate()?;
        if envelope.hour_index != political.hour_index
            || terrain.map_width != prepared.map_width
            || terrain.map_height != prepared.map_height
            || terrain.camera_frame_sha256 != prepared.camera_view_sha256
        {
            return Err(
                "XMAP08 execution inputs do not describe one replay hour, map, and camera".into(),
            );
        }
        let replay_frame_identity_sha256 = terrain.replay_frame_identity_sha256.clone();
        let map_copy_receipt_set_sha256 = terrain.map_copy_receipt_set_sha256()?;
        let river_assignment = prepared.river_assignment.clone();
        let strait_assignment = prepared.strait_assignment.clone();
        validate_assigned_operation(
            &river_assignment,
            XMAP08_RIVER_STAGE_NAME,
            XMAP08_RIVER_OPERATION_ID,
        )?;
        validate_assigned_operation(
            &strait_assignment,
            XMAP08_STRAIT_STAGE_NAME,
            XMAP08_STRAIT_OPERATION_ID,
        )?;
        if river_assignment.topology_sha256 != strait_assignment.topology_sha256
            || river_assignment.assigned_ordinal >= strait_assignment.assigned_ordinal
        {
            return Err(
                "XMAP08 assigned-stage authority does not place rivers before straits".into(),
            );
        }
        validate_canonical_sha256(
            &replay_frame_identity_sha256,
            "XMAP08 replay-frame identity",
        )?;
        validate_canonical_sha256(
            &map_copy_receipt_set_sha256,
            "XMAP08 execution map-copy set",
        )?;
        validate_canonical_sha256(
            main_scene_resource_identity_sha256,
            "XMAP08 execution MainScene resource identity",
        )?;
        let mut authority = Self {
            hour_index: envelope.hour_index,
            replay_frame_identity_sha256,
            political_frame_sha256: political.deterministic_sha256.clone(),
            content_manifest_sha256: envelope.content_manifest_sha256.clone(),
            foundation_content_operation_sha256: envelope
                .foundation_content_operation_sha256
                .clone(),
            viewer_state_sha256: Sha256::hex(&serde_json::to_vec(viewer)?),
            main_scene_target_sha256: Sha256::hex(&serde_json::to_vec(target)?),
            main_scene_resource_identity_sha256: main_scene_resource_identity_sha256.into(),
            map_copy_receipt_set_sha256,
            river_assignment,
            strait_assignment,
            deterministic_sha256: String::new(),
        };
        authority.deterministic_sha256 = execution_frame_authority_sha256(&authority)?;
        authority.validate()?;
        Ok(authority)
    }

    #[cfg(test)]
    pub(crate) fn from_test_identities(
        envelope: &FoundationRenderEnvelope,
        viewer: &ViewerOwnedMapState,
        target: &ProductionMainSceneTargetDesc,
        replay_frame_identity_sha256: String,
        map_copy_receipt_set_sha256: String,
        prepared: &PreparedRiversStraitsFrame,
    ) -> Result<Self> {
        envelope
            .validate()
            .map_err(|error| format!("XMAP08 test foundation envelope: {error}"))?;
        viewer.validate()?;
        target.validate()?;
        prepared.validate()?;
        let mut authority = Self {
            hour_index: envelope.hour_index,
            replay_frame_identity_sha256,
            political_frame_sha256: "a".repeat(64),
            content_manifest_sha256: envelope.content_manifest_sha256.clone(),
            foundation_content_operation_sha256: envelope
                .foundation_content_operation_sha256
                .clone(),
            viewer_state_sha256: Sha256::hex(&serde_json::to_vec(viewer)?),
            main_scene_target_sha256: Sha256::hex(&serde_json::to_vec(target)?),
            main_scene_resource_identity_sha256: "7".repeat(64),
            map_copy_receipt_set_sha256,
            river_assignment: prepared.river_assignment.clone(),
            strait_assignment: prepared.strait_assignment.clone(),
            deterministic_sha256: String::new(),
        };
        authority.deterministic_sha256 = execution_frame_authority_sha256(&authority)?;
        authority.validate()?;
        Ok(authority)
    }

    pub fn validate(&self) -> Result<()> {
        validate_canonical_sha256(
            &self.replay_frame_identity_sha256,
            "XMAP08 execution replay-frame identity",
        )?;
        validate_canonical_sha256(
            &self.political_frame_sha256,
            "XMAP08 execution political frame",
        )?;
        validate_canonical_sha256(
            &self.content_manifest_sha256,
            "XMAP08 execution content manifest",
        )?;
        validate_canonical_sha256(
            &self.foundation_content_operation_sha256,
            "XMAP08 execution foundation operation",
        )?;
        validate_canonical_sha256(&self.viewer_state_sha256, "XMAP08 execution viewer state")?;
        validate_canonical_sha256(
            &self.main_scene_target_sha256,
            "XMAP08 execution MainScene target",
        )?;
        validate_canonical_sha256(
            &self.main_scene_resource_identity_sha256,
            "XMAP08 execution MainScene resource identity",
        )?;
        validate_canonical_sha256(
            &self.map_copy_receipt_set_sha256,
            "XMAP08 execution map-copy set",
        )?;
        validate_assigned_operation(
            &self.river_assignment,
            XMAP08_RIVER_STAGE_NAME,
            XMAP08_RIVER_OPERATION_ID,
        )?;
        validate_assigned_operation(
            &self.strait_assignment,
            XMAP08_STRAIT_STAGE_NAME,
            XMAP08_STRAIT_OPERATION_ID,
        )?;
        if self.river_assignment.topology_sha256 != self.strait_assignment.topology_sha256
            || self.river_assignment.assigned_ordinal >= self.strait_assignment.assigned_ordinal
        {
            return Err(
                "XMAP08 execution authority does not bind one ordered assigned-stage topology"
                    .into(),
            );
        }
        validate_canonical_sha256(
            &self.deterministic_sha256,
            "XMAP08 execution frame authority",
        )?;
        let expected = execution_frame_authority_sha256(self)?;
        if self.deterministic_sha256 != expected {
            return Err(format!(
                "XMAP08 execution frame authority digest mismatch: expected {expected}, got {}",
                self.deterministic_sha256
            )
            .into());
        }
        Ok(())
    }
}

fn execution_frame_authority_sha256(authority: &Xmap08ExecutionFrameAuthority) -> Result<String> {
    let mut unsigned = authority.clone();
    unsigned.deterministic_sha256.clear();
    Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap08TargetBoundDrawReceipt {
    pub schema: String,
    pub component_id: String,
    pub stage_name: String,
    pub operation_id: String,
    pub assigned_ordinal: u32,
    pub topology_sha256: String,
    pub assignment_sha256: String,
    pub hour_index: i64,
    pub execution_authority_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub main_scene_resource_identity_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub draw_ordinal: u32,
    pub prepared_frame_sha256: String,
    pub program_sha256: String,
    pub render_view_mask: u8,
    pub map_copy_offset_f32_bits: u32,
    pub index_count: u32,
    pub start_index_location: u32,
    pub base_vertex: i32,
    pub bound_resource_set_sha256: String,
    pub bound_constant_buffer_set_sha256: String,
    pub packet_draw_receipt_sha256: String,
}

impl Xmap08TargetBoundDrawReceipt {
    pub fn seal(mut self) -> Result<Self> {
        self.packet_draw_receipt_sha256.clear();
        self.validate_body()?;
        self.packet_draw_receipt_sha256 = target_bound_draw_receipt_sha256(&self)?;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        validate_canonical_sha256(
            &self.packet_draw_receipt_sha256,
            "XMAP08 target-bound draw receipt",
        )?;
        let expected = target_bound_draw_receipt_sha256(self)?;
        if self.packet_draw_receipt_sha256 != expected {
            return Err(format!(
                "XMAP08 target-bound draw receipt digest mismatch: expected {expected}, got {}",
                self.packet_draw_receipt_sha256
            )
            .into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        if self.schema != XMAP08_TARGET_BOUND_DRAW_SCHEMA {
            return Err("XMAP08 target-bound draw receipt schema mismatch".into());
        }
        validate_bound_assignment_fields(
            &self.component_id,
            &self.stage_name,
            &self.operation_id,
            self.assigned_ordinal,
            &self.topology_sha256,
            &self.assignment_sha256,
        )?;
        if self.render_view_mask != 1 && self.render_view_mask != 2 && self.render_view_mask != 4 {
            return Err("XMAP08 target-bound draw render-view mask is invalid".into());
        }
        if self.index_count == 0 {
            return Err("XMAP08 target-bound draw has zero indices".into());
        }
        validate_canonical_sha256(&self.topology_sha256, "XMAP08 target-bound topology")?;
        validate_canonical_sha256(
            &self.assignment_sha256,
            "XMAP08 target-bound stage assignment",
        )?;
        validate_canonical_sha256(
            &self.execution_authority_sha256,
            "XMAP08 target-bound execution authority",
        )?;
        validate_canonical_sha256(
            &self.replay_frame_identity_sha256,
            "XMAP08 target-bound replay-frame identity",
        )?;
        validate_canonical_sha256(
            &self.content_manifest_sha256,
            "XMAP08 target-bound content manifest",
        )?;
        validate_canonical_sha256(
            &self.foundation_content_operation_sha256,
            "XMAP08 target-bound foundation operation",
        )?;
        validate_canonical_sha256(
            &self.viewer_state_sha256,
            "XMAP08 target-bound viewer state",
        )?;
        validate_canonical_sha256(
            &self.main_scene_target_sha256,
            "XMAP08 target-bound MainScene target",
        )?;
        validate_canonical_sha256(
            &self.main_scene_resource_identity_sha256,
            "XMAP08 target-bound MainScene resource",
        )?;
        validate_canonical_sha256(
            &self.map_copy_receipt_set_sha256,
            "XMAP08 target-bound map-copy set",
        )?;
        validate_canonical_sha256(
            &self.prepared_frame_sha256,
            "XMAP08 target-bound prepared frame",
        )?;
        validate_canonical_sha256(&self.program_sha256, "XMAP08 target-bound program")?;
        validate_canonical_sha256(
            &self.bound_resource_set_sha256,
            "XMAP08 target-bound resource set",
        )?;
        validate_canonical_sha256(
            &self.bound_constant_buffer_set_sha256,
            "XMAP08 target-bound constant-buffer set",
        )?;
        Ok(())
    }
}

fn target_bound_draw_receipt_sha256(receipt: &Xmap08TargetBoundDrawReceipt) -> Result<String> {
    let mut unsigned = receipt.clone();
    unsigned.packet_draw_receipt_sha256.clear();
    Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
}

fn validate_canonical_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} must be 64 lowercase hexadecimal characters").into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompositionComponentDisposition {
    Present {
        ordered_packet_draw_receipt_sha256s: Vec<String>,
    },
    AuthenticatedEmpty {
        enumerator_witness_sha256: String,
    },
    NotApplicable {
        applicability_witness_sha256: String,
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap08ComponentClosure {
    pub schema: String,
    pub component_id: String,
    pub stage_name: String,
    pub operation_id: String,
    pub assigned_ordinal: u32,
    pub topology_sha256: String,
    pub assignment_sha256: String,
    pub hour_index: i64,
    pub execution_authority_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub main_scene_resource_identity_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub prepared_frame_sha256: String,
    pub program_sha256: String,
    pub disposition: CompositionComponentDisposition,
    pub closure_sha256: String,
}

impl Xmap08ComponentClosure {
    pub fn seal(mut self) -> Result<Self> {
        self.closure_sha256.clear();
        self.validate_body()?;
        self.closure_sha256 = component_closure_sha256(&self)?;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        validate_canonical_sha256(&self.closure_sha256, "XMAP08 component closure")?;
        let expected = component_closure_sha256(self)?;
        if self.closure_sha256 != expected {
            return Err(format!(
                "XMAP08 component closure digest mismatch: expected {expected}, got {}",
                self.closure_sha256
            )
            .into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        if self.schema != XMAP08_COMPONENT_CLOSURE_SCHEMA {
            return Err("XMAP08 composition component closure schema mismatch".into());
        }
        validate_bound_assignment_fields(
            &self.component_id,
            &self.stage_name,
            &self.operation_id,
            self.assigned_ordinal,
            &self.topology_sha256,
            &self.assignment_sha256,
        )?;
        validate_canonical_sha256(&self.topology_sha256, "XMAP08 closure topology")?;
        validate_canonical_sha256(&self.assignment_sha256, "XMAP08 closure stage assignment")?;
        validate_canonical_sha256(
            &self.execution_authority_sha256,
            "XMAP08 closure execution authority",
        )?;
        validate_canonical_sha256(
            &self.replay_frame_identity_sha256,
            "XMAP08 closure replay-frame identity",
        )?;
        validate_canonical_sha256(
            &self.content_manifest_sha256,
            "XMAP08 closure content manifest",
        )?;
        validate_canonical_sha256(
            &self.foundation_content_operation_sha256,
            "XMAP08 closure foundation operation",
        )?;
        validate_canonical_sha256(&self.viewer_state_sha256, "XMAP08 closure viewer state")?;
        validate_canonical_sha256(
            &self.main_scene_target_sha256,
            "XMAP08 closure MainScene target",
        )?;
        validate_canonical_sha256(
            &self.main_scene_resource_identity_sha256,
            "XMAP08 closure MainScene resource",
        )?;
        validate_canonical_sha256(
            &self.map_copy_receipt_set_sha256,
            "XMAP08 closure map-copy set",
        )?;
        validate_canonical_sha256(&self.prepared_frame_sha256, "XMAP08 closure prepared frame")?;
        validate_canonical_sha256(&self.program_sha256, "XMAP08 closure program")?;
        match &self.disposition {
            CompositionComponentDisposition::Present {
                ordered_packet_draw_receipt_sha256s,
            } => {
                if ordered_packet_draw_receipt_sha256s.is_empty() {
                    return Err("XMAP08 Present component closure has no actual draws".into());
                }
                let mut unique = BTreeSet::new();
                for digest in ordered_packet_draw_receipt_sha256s {
                    validate_canonical_sha256(digest, "XMAP08 closure draw receipt")?;
                    if !unique.insert(digest.as_str()) {
                        return Err(
                            "XMAP08 Present component closure duplicates a draw receipt".into()
                        );
                    }
                }
            }
            CompositionComponentDisposition::AuthenticatedEmpty {
                enumerator_witness_sha256,
            } => validate_canonical_sha256(
                enumerator_witness_sha256,
                "XMAP08 authenticated-empty enumerator witness",
            )?,
            CompositionComponentDisposition::NotApplicable {
                applicability_witness_sha256,
                reason,
            } => {
                validate_canonical_sha256(
                    applicability_witness_sha256,
                    "XMAP08 not-applicable witness",
                )?;
                if reason.trim().is_empty() {
                    return Err("XMAP08 not-applicable closure reason is blank".into());
                }
            }
        }
        Ok(())
    }
}

fn component_closure_sha256(closure: &Xmap08ComponentClosure) -> Result<String> {
    let mut unsigned = closure.clone();
    unsigned.closure_sha256.clear();
    Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
}

fn target_bound_draw_receipt(
    authority: &Xmap08ExecutionFrameAuthority,
    assignment: &AssignedStage,
    component_id: &str,
    frame: &PreparedRiversStraitsFrame,
    program: &PreparedMapEffectProgram,
    draw_ordinal: u32,
    render_view_mask: u8,
    map_copy_offset_f32_bits: u32,
    index_count: u32,
    start_index_location: u32,
    base_vertex: i32,
    bound_resource_set_sha256: String,
    bound_constant_buffer_set_sha256: String,
) -> Result<Xmap08TargetBoundDrawReceipt> {
    authority.validate()?;
    validate_canonical_sha256(&bound_resource_set_sha256, "XMAP08 draw bound-resource set")?;
    validate_canonical_sha256(
        &bound_constant_buffer_set_sha256,
        "XMAP08 draw bound-constant-buffer set",
    )?;
    Xmap08TargetBoundDrawReceipt {
        schema: XMAP08_TARGET_BOUND_DRAW_SCHEMA.into(),
        component_id: component_id.into(),
        stage_name: assignment.stage_name.clone(),
        operation_id: assignment.operation_name.clone(),
        assigned_ordinal: assignment.assigned_ordinal,
        topology_sha256: assignment.topology_sha256.clone(),
        assignment_sha256: assignment.assignment_sha256.clone(),
        hour_index: authority.hour_index,
        execution_authority_sha256: authority.deterministic_sha256.clone(),
        replay_frame_identity_sha256: authority.replay_frame_identity_sha256.clone(),
        content_manifest_sha256: authority.content_manifest_sha256.clone(),
        foundation_content_operation_sha256: authority.foundation_content_operation_sha256.clone(),
        viewer_state_sha256: authority.viewer_state_sha256.clone(),
        main_scene_target_sha256: authority.main_scene_target_sha256.clone(),
        main_scene_resource_identity_sha256: authority.main_scene_resource_identity_sha256.clone(),
        map_copy_receipt_set_sha256: authority.map_copy_receipt_set_sha256.clone(),
        draw_ordinal,
        prepared_frame_sha256: frame.deterministic_sha256.clone(),
        program_sha256: program.deterministic_sha256.clone(),
        render_view_mask,
        map_copy_offset_f32_bits,
        index_count,
        start_index_location,
        base_vertex,
        bound_resource_set_sha256,
        bound_constant_buffer_set_sha256,
        packet_draw_receipt_sha256: String::new(),
    }
    .seal()
}

fn component_closure(
    authority: &Xmap08ExecutionFrameAuthority,
    assignment: &AssignedStage,
    component_id: &str,
    frame: &PreparedRiversStraitsFrame,
    program: &PreparedMapEffectProgram,
    enumerator_witness_sha256: &str,
    draw_receipts: &[Xmap08TargetBoundDrawReceipt],
) -> Result<Xmap08ComponentClosure> {
    authority.validate()?;
    validate_canonical_sha256(
        enumerator_witness_sha256,
        "XMAP08 component enumerator witness",
    )?;
    for (ordinal, receipt) in draw_receipts.iter().enumerate() {
        receipt.validate()?;
        if receipt.component_id != component_id
            || receipt.stage_name != assignment.stage_name
            || receipt.operation_id != assignment.operation_name
            || receipt.assigned_ordinal != assignment.assigned_ordinal
            || receipt.topology_sha256 != assignment.topology_sha256
            || receipt.assignment_sha256 != assignment.assignment_sha256
            || receipt.hour_index != authority.hour_index
            || receipt.execution_authority_sha256 != authority.deterministic_sha256
            || receipt.draw_ordinal != ordinal as u32
            || receipt.content_manifest_sha256 != authority.content_manifest_sha256
            || receipt.foundation_content_operation_sha256
                != authority.foundation_content_operation_sha256
            || receipt.viewer_state_sha256 != authority.viewer_state_sha256
            || receipt.main_scene_target_sha256 != authority.main_scene_target_sha256
            || receipt.main_scene_resource_identity_sha256
                != authority.main_scene_resource_identity_sha256
            || receipt.map_copy_receipt_set_sha256 != authority.map_copy_receipt_set_sha256
            || receipt.prepared_frame_sha256 != frame.deterministic_sha256
            || receipt.program_sha256 != program.deterministic_sha256
        {
            return Err("XMAP08 component closure received a mismatched draw receipt".into());
        }
    }
    let disposition = if draw_receipts.is_empty() {
        CompositionComponentDisposition::AuthenticatedEmpty {
            enumerator_witness_sha256: enumerator_witness_sha256.into(),
        }
    } else {
        CompositionComponentDisposition::Present {
            ordered_packet_draw_receipt_sha256s: draw_receipts
                .iter()
                .map(|receipt| receipt.packet_draw_receipt_sha256.clone())
                .collect(),
        }
    };
    Xmap08ComponentClosure {
        schema: XMAP08_COMPONENT_CLOSURE_SCHEMA.into(),
        component_id: component_id.into(),
        stage_name: assignment.stage_name.clone(),
        operation_id: assignment.operation_name.clone(),
        assigned_ordinal: assignment.assigned_ordinal,
        topology_sha256: assignment.topology_sha256.clone(),
        assignment_sha256: assignment.assignment_sha256.clone(),
        hour_index: authority.hour_index,
        execution_authority_sha256: authority.deterministic_sha256.clone(),
        replay_frame_identity_sha256: authority.replay_frame_identity_sha256.clone(),
        content_manifest_sha256: authority.content_manifest_sha256.clone(),
        foundation_content_operation_sha256: authority.foundation_content_operation_sha256.clone(),
        viewer_state_sha256: authority.viewer_state_sha256.clone(),
        main_scene_target_sha256: authority.main_scene_target_sha256.clone(),
        main_scene_resource_identity_sha256: authority.main_scene_resource_identity_sha256.clone(),
        map_copy_receipt_set_sha256: authority.map_copy_receipt_set_sha256.clone(),
        prepared_frame_sha256: frame.deterministic_sha256.clone(),
        program_sha256: program.deterministic_sha256.clone(),
        disposition,
        closure_sha256: String::new(),
    }
    .seal()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap08D3dHandoffReceipt {
    pub frame_sha256: String,
    pub uploaded_programs: u32,
    pub uploaded_vertex_buffers: u32,
    pub uploaded_index_buffers: u32,
    pub uploaded_static_resources: u32,
    pub retained_river_draws: u32,
    pub retained_strait_draws: u32,
    pub executed_river_draws: u32,
    pub executed_strait_draws: u32,
    pub river_ready_contract_sha256: Option<String>,
    pub strait_ready_contract_sha256: Option<String>,
    pub river_draw_receipts: Vec<Xmap08TargetBoundDrawReceipt>,
    pub strait_draw_receipts: Vec<Xmap08TargetBoundDrawReceipt>,
    pub river_component_closure: Option<Xmap08ComponentClosure>,
    pub strait_component_closure: Option<Xmap08ComponentClosure>,
    pub deterministic_sha256: String,
}

impl Xmap08D3dHandoffReceipt {
    /// Prove that both XMAP08 passes completed for the exact prepared frame.
    ///
    /// An empty admitted draw set is a completed pass with zero executions and
    /// no ready-draw contract. A non-empty set must execute every retained
    /// draw and publish the authenticated ready-draw contract generated from
    /// its actual bound resources and constant buffers.
    pub fn validate_complete_for_authority(
        &self,
        frame_sha256: &str,
        authority: &Xmap08ExecutionFrameAuthority,
    ) -> Result<()> {
        authority.validate()?;
        self.validate_complete_structure_for_frame(frame_sha256)?;
        let river = self
            .river_component_closure
            .as_ref()
            .ok_or("XMAP08 completion omitted the river component closure")?;
        let strait = self
            .strait_component_closure
            .as_ref()
            .ok_or("XMAP08 completion omitted the strait component closure")?;
        for (closure, assignment) in [
            (river, &authority.river_assignment),
            (strait, &authority.strait_assignment),
        ] {
            if closure.execution_authority_sha256 != authority.deterministic_sha256
                || closure.hour_index != authority.hour_index
                || closure.replay_frame_identity_sha256 != authority.replay_frame_identity_sha256
                || closure.content_manifest_sha256 != authority.content_manifest_sha256
                || closure.foundation_content_operation_sha256
                    != authority.foundation_content_operation_sha256
                || closure.viewer_state_sha256 != authority.viewer_state_sha256
                || closure.main_scene_target_sha256 != authority.main_scene_target_sha256
                || closure.main_scene_resource_identity_sha256
                    != authority.main_scene_resource_identity_sha256
                || closure.map_copy_receipt_set_sha256 != authority.map_copy_receipt_set_sha256
                || closure.topology_sha256 != assignment.topology_sha256
                || closure.assignment_sha256 != assignment.assignment_sha256
                || closure.assigned_ordinal != assignment.assigned_ordinal
            {
                return Err(
                    "XMAP08 completion receipt differs from the required execution authority"
                        .into(),
                );
            }
        }
        Ok(())
    }

    fn validate_complete_structure_for_frame(&self, frame_sha256: &str) -> Result<()> {
        if self.frame_sha256 != frame_sha256 {
            return Err(format!(
                "XMAP08 completion receipt frame {} does not match required frame {frame_sha256}",
                self.frame_sha256
            )
            .into());
        }
        validate_completed_pass(
            "river",
            self.retained_river_draws,
            self.executed_river_draws,
            self.river_ready_contract_sha256.as_deref(),
            &self.river_draw_receipts,
            self.river_component_closure.as_ref(),
        )?;
        validate_completed_pass(
            "strait",
            self.retained_strait_draws,
            self.executed_strait_draws,
            self.strait_ready_contract_sha256.as_deref(),
            &self.strait_draw_receipts,
            self.strait_component_closure.as_ref(),
        )?;
        let river_closure = self
            .river_component_closure
            .as_ref()
            .ok_or("XMAP08 completion omitted the river component closure")?;
        let strait_closure = self
            .strait_component_closure
            .as_ref()
            .ok_or("XMAP08 completion omitted the strait component closure")?;
        if river_closure.prepared_frame_sha256 != frame_sha256
            || strait_closure.prepared_frame_sha256 != frame_sha256
            || river_closure.hour_index != strait_closure.hour_index
            || river_closure.execution_authority_sha256 != strait_closure.execution_authority_sha256
            || river_closure.replay_frame_identity_sha256
                != strait_closure.replay_frame_identity_sha256
            || river_closure.content_manifest_sha256 != strait_closure.content_manifest_sha256
            || river_closure.foundation_content_operation_sha256
                != strait_closure.foundation_content_operation_sha256
            || river_closure.viewer_state_sha256 != strait_closure.viewer_state_sha256
            || river_closure.main_scene_target_sha256 != strait_closure.main_scene_target_sha256
            || river_closure.main_scene_resource_identity_sha256
                != strait_closure.main_scene_resource_identity_sha256
            || river_closure.map_copy_receipt_set_sha256
                != strait_closure.map_copy_receipt_set_sha256
            || river_closure.topology_sha256 != strait_closure.topology_sha256
            || river_closure.assigned_ordinal >= strait_closure.assigned_ordinal
        {
            return Err(
                "XMAP08 river/strait component closures do not share one ordered frame authority"
                    .into(),
            );
        }
        for receipt in &self.river_draw_receipts {
            validate_draw_receipt_against_closure(receipt, river_closure)?;
        }
        for receipt in &self.strait_draw_receipts {
            validate_draw_receipt_against_closure(receipt, strait_closure)?;
        }
        let expected = receipt_sha256(self)?;
        if self.deterministic_sha256 != expected {
            return Err(format!(
                "XMAP08 completion receipt digest mismatch: expected {expected}, got {}",
                self.deterministic_sha256
            )
            .into());
        }
        Ok(())
    }
}

fn validate_draw_receipt_against_closure(
    receipt: &Xmap08TargetBoundDrawReceipt,
    closure: &Xmap08ComponentClosure,
) -> Result<()> {
    if receipt.component_id != closure.component_id
        || receipt.stage_name != closure.stage_name
        || receipt.operation_id != closure.operation_id
        || receipt.assigned_ordinal != closure.assigned_ordinal
        || receipt.topology_sha256 != closure.topology_sha256
        || receipt.assignment_sha256 != closure.assignment_sha256
        || receipt.hour_index != closure.hour_index
        || receipt.execution_authority_sha256 != closure.execution_authority_sha256
        || receipt.replay_frame_identity_sha256 != closure.replay_frame_identity_sha256
        || receipt.content_manifest_sha256 != closure.content_manifest_sha256
        || receipt.foundation_content_operation_sha256
            != closure.foundation_content_operation_sha256
        || receipt.viewer_state_sha256 != closure.viewer_state_sha256
        || receipt.main_scene_target_sha256 != closure.main_scene_target_sha256
        || receipt.main_scene_resource_identity_sha256
            != closure.main_scene_resource_identity_sha256
        || receipt.map_copy_receipt_set_sha256 != closure.map_copy_receipt_set_sha256
        || receipt.prepared_frame_sha256 != closure.prepared_frame_sha256
        || receipt.program_sha256 != closure.program_sha256
    {
        return Err("XMAP08 target-bound draw receipt differs from its component closure".into());
    }
    Ok(())
}

fn validate_completed_pass(
    label: &str,
    retained: u32,
    executed: u32,
    ready_contract_sha256: Option<&str>,
    draw_receipts: &[Xmap08TargetBoundDrawReceipt],
    component_closure: Option<&Xmap08ComponentClosure>,
) -> Result<()> {
    if executed != retained {
        return Err(format!(
            "XMAP08 {label} pass executed {executed} draws but retained {retained}"
        )
        .into());
    }
    let closure = component_closure
        .ok_or_else(|| format!("XMAP08 completed {label} pass has no typed component closure"))?;
    closure.validate()?;
    let expected_component_id = match label {
        "river" => XMAP08_RIVER_COMPONENT_ID,
        "strait" => XMAP08_STRAIT_COMPONENT_ID,
        _ => return Err(format!("XMAP08 unknown completed pass label {label}").into()),
    };
    if closure.component_id != expected_component_id
        || draw_receipts
            .iter()
            .any(|receipt| receipt.component_id != expected_component_id)
    {
        return Err(format!(
            "XMAP08 {label} completion receipt carries the wrong component identity"
        )
        .into());
    }
    if draw_receipts.len() != executed as usize {
        return Err(format!(
            "XMAP08 {label} pass has {} target-bound receipts for {executed} draws",
            draw_receipts.len()
        )
        .into());
    }
    for (ordinal, receipt) in draw_receipts.iter().enumerate() {
        receipt.validate()?;
        if receipt.draw_ordinal != ordinal as u32 {
            return Err(format!(
                "XMAP08 {label} draw receipt ordinal {} is not engine-order position {ordinal}",
                receipt.draw_ordinal
            )
            .into());
        }
    }
    match (retained, ready_contract_sha256) {
        (0, None) => {
            if !draw_receipts.is_empty()
                || !matches!(
                    &closure.disposition,
                    CompositionComponentDisposition::AuthenticatedEmpty { .. }
                )
            {
                return Err(format!(
                    "XMAP08 empty {label} pass lacks an authenticated-empty closure"
                )
                .into());
            }
            Ok(())
        }
        (0, Some(_)) => Err(format!(
            "XMAP08 empty {label} pass must not publish a ready-draw contract"
        )
        .into()),
        (_, Some(digest))
            if digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
        {
            let CompositionComponentDisposition::Present {
                ordered_packet_draw_receipt_sha256s,
            } = &closure.disposition
            else {
                return Err(format!(
                    "XMAP08 non-empty {label} pass lacks a Present closure"
                )
                .into());
            };
            let expected = draw_receipts
                .iter()
                .map(|receipt| receipt.packet_draw_receipt_sha256.as_str())
                .collect::<Vec<_>>();
            if ordered_packet_draw_receipt_sha256s
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                != expected
            {
                return Err(format!(
                    "XMAP08 {label} component closure draw order does not match actual receipts"
                )
                .into());
            }
            Ok(())
        }
        (_, Some(digest)) => Err(format!(
            "XMAP08 {label} ready-draw contract digest must be 64 lowercase hex characters, got {digest}"
        )
        .into()),
        (_, None) => Err(format!(
            "XMAP08 non-empty {label} pass has no authenticated ready-draw contract"
        )
        .into()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap08RiverTemporalBinding {
    pub(crate) animation_time_f32_bits: u32,
    pub(crate) season_lerp_f32_bits: u32,
    pub(crate) camera_frame_sha256: String,
    pub(crate) replay_frame_identity_sha256: String,
    pub(crate) payload_sha256: String,
}

impl Xmap08RiverTemporalBinding {
    /// Bind XMAP08 to the exact viewer-time and replay-season values already
    /// authenticated by XMAP07 for the same replay and camera frame.
    pub fn from_xmap07(
        water: &crate::PreparedWaterFrame,
        camera_frame_sha256: &str,
        replay_frame_identity_sha256: &str,
    ) -> Result<Self> {
        water.validate()?;
        if water.camera_view_sha256 != camera_frame_sha256
            || water.replay_frame_identity_sha256 != replay_frame_identity_sha256
            || water.temporal.replay_frame_identity_sha256 != replay_frame_identity_sha256
        {
            return Err(
                "XMAP08 temporal provider belongs to another camera or replay frame".into(),
            );
        }
        let payload = water.temporal.map_object_time_season_bytes();
        Ok(Self {
            animation_time_f32_bits: water.temporal.animation_time_f32_bits,
            season_lerp_f32_bits: water.temporal.season_lerp_f32_bits,
            camera_frame_sha256: camera_frame_sha256.into(),
            replay_frame_identity_sha256: replay_frame_identity_sha256.into(),
            payload_sha256: Sha256::hex(&payload),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap08RiverShadowMatrixBinding {
    pub(crate) shadow_map_texture_matrix_f32_bits: [[u32; 4]; 4],
    pub(crate) camera_frame_sha256: String,
    pub(crate) replay_frame_identity_sha256: String,
    pub(crate) payload_sha256: String,
}

impl Xmap08RiverShadowMatrixBinding {
    /// Adapt XMAP09's authenticated shadow matrix only for an effective river
    /// shader whose reflection retains `ShadowMapTextureMatrix`.
    pub fn from_xmap09(frame: &crate::PreparedXmap09LightingFrame) -> Result<Self> {
        frame.validate()?;
        let binding = frame.shadow_map_texture_matrix()?;
        if binding.owner_packet != "XMAP09"
            || binding.symbol != "ShadowMapTextureMatrix"
            || binding.input_payload_sha256 != frame.input_payload_sha256
        {
            return Err("XMAP08 received an invalid XMAP09 shadow-matrix binding".into());
        }
        validate_canonical_sha256(
            &frame.camera_frame_sha256,
            "XMAP08 XMAP09 shadow camera frame",
        )?;
        validate_canonical_sha256(
            &frame.replay_frame_identity_sha256,
            "XMAP08 XMAP09 shadow replay frame",
        )?;
        let mut rows = [[0u32; 4]; 4];
        for (index, bits) in binding
            .shadow_map_texture_matrix_f32_bits
            .into_iter()
            .enumerate()
        {
            rows[index / 4][index % 4] = bits;
        }
        let payload = rows
            .iter()
            .flatten()
            .flat_map(|bits| bits.to_le_bytes())
            .collect::<Vec<_>>();
        if Sha256::hex(&payload) != binding.payload_sha256 {
            return Err("XMAP08 XMAP09 shadow-matrix payload digest mismatch".into());
        }
        Ok(Self {
            shadow_map_texture_matrix_f32_bits: rows,
            camera_frame_sha256: frame.camera_frame_sha256.clone(),
            replay_frame_identity_sha256: frame.replay_frame_identity_sha256.clone(),
            payload_sha256: binding.payload_sha256,
        })
    }
}

/// XMAP10-owned, per-map-copy aggregate SceneCB0. XMAP08 never patches or
/// reuploads this buffer: the exact compositor bytes are borrowed and bound
/// directly for the matching XMAP04 map-copy row.
///
/// # Safety
/// Implementors must keep `raw_buffer()` alive on the draw device and every
/// identity must describe that exact retained buffer.
#[cfg(windows)]
pub unsafe trait NativeXmap08SceneConstantBuffer:
    crate::NativeXmap04UnderwaterSceneConstantBuffer
{
    fn map_copy_receipt_set_sha256(&self) -> &str;
    fn render_view_mask(&self) -> u8;
    fn map_copy_offset_f32_bits(&self) -> u32;
    fn composition_receipt_sha256(&self) -> &str;
    fn reflected_layout_sha256(&self) -> &str;
    fn main_scene_resource_identity_sha256(&self) -> &str;
    fn raw_main_scene_color_texture(&self) -> *mut winapi::um::d3d11::ID3D11Texture2D;
}

/// Borrowed XMAP04/XMAP07/XMAP09/XMAP10 shader resource used by the optimized river
/// program. Water content resources are replay-scoped and deliberately have
/// no camera identity; lighting/shadow resources are camera-scoped.
///
/// # Safety
/// Implementors must keep `raw_shader_resource_view()` alive on the draw
/// device and every identity must describe that exact retained resource.
#[cfg(windows)]
pub unsafe trait NativeXmap08RiverShaderResource {
    fn owner_packet(&self) -> &str;
    fn symbol(&self) -> &str;
    fn shader_register(&self) -> u32;
    fn camera_frame_sha256(&self) -> Option<&str>;
    fn replay_frame_identity_sha256(&self) -> &str;
    fn resource_sha256(&self) -> &str;
    fn descriptor_sha256(&self) -> &str;
    fn raw_shader_resource_view(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView;
}

#[cfg(windows)]
unsafe impl NativeXmap08RiverShaderResource for crate::NativeXmap09ShaderResourceView<'_> {
    fn owner_packet(&self) -> &str {
        &self.binding.owner_packet
    }

    fn symbol(&self) -> &str {
        &self.binding.symbol
    }

    fn shader_register(&self) -> u32 {
        <Self as crate::NativeXmap04UnderwaterShaderResource>::shader_register(self)
    }

    fn camera_frame_sha256(&self) -> Option<&str> {
        Some(&self.binding.camera_frame_sha256)
    }

    fn replay_frame_identity_sha256(&self) -> &str {
        &self.binding.replay_frame_identity_sha256
    }

    fn resource_sha256(&self) -> &str {
        &self.binding.resource_sha256
    }

    fn descriptor_sha256(&self) -> &str {
        &self.binding.descriptor_sha256
    }

    fn raw_shader_resource_view(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.as_raw()
    }
}

#[cfg(windows)]
unsafe impl NativeXmap08RiverShaderResource for crate::NativeXmap04ShaderResourceView<'_> {
    fn owner_packet(&self) -> &str {
        "XMAP04"
    }

    fn symbol(&self) -> &str {
        self.symbol()
    }

    fn shader_register(&self) -> u32 {
        self.shader_register()
    }

    fn camera_frame_sha256(&self) -> Option<&str> {
        None
    }

    fn replay_frame_identity_sha256(&self) -> &str {
        self.replay_frame_identity_sha256()
    }

    fn resource_sha256(&self) -> &str {
        self.resource_sha256()
    }

    fn descriptor_sha256(&self) -> &str {
        self.descriptor_sha256()
    }

    fn raw_shader_resource_view(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.as_raw()
    }
}

#[cfg(windows)]
unsafe impl NativeXmap08RiverShaderResource for crate::NativeXmap07ResourceView<'_> {
    fn owner_packet(&self) -> &str {
        &self.binding.owner_packet
    }

    fn symbol(&self) -> &str {
        &self.binding.symbol
    }

    fn shader_register(&self) -> u32 {
        self.binding.shader_register
    }

    fn camera_frame_sha256(&self) -> Option<&str> {
        None
    }

    fn replay_frame_identity_sha256(&self) -> &str {
        &self.replay_frame_identity_sha256
    }

    fn resource_sha256(&self) -> &str {
        &self.binding.resource_sha256
    }

    fn descriptor_sha256(&self) -> &str {
        &self.binding.descriptor_sha256
    }

    fn raw_shader_resource_view(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.as_raw()
    }
}

#[cfg(windows)]
unsafe impl NativeXmap08RiverShaderResource for crate::NativeXmap10ShaderResourceView<'_> {
    fn owner_packet(&self) -> &str {
        &self.binding().owner_packet
    }

    fn symbol(&self) -> &str {
        &self.binding().symbol
    }

    fn shader_register(&self) -> u32 {
        <Self as crate::NativeXmap04UnderwaterShaderResource>::shader_register(self)
    }

    fn camera_frame_sha256(&self) -> Option<&str> {
        Some(self.frame_identity_sha256())
    }

    fn replay_frame_identity_sha256(&self) -> &str {
        crate::NativeXmap10ShaderResourceView::replay_frame_identity_sha256(self)
    }

    fn resource_sha256(&self) -> &str {
        crate::NativeXmap10ShaderResourceView::resource_sha256(self)
    }

    fn descriptor_sha256(&self) -> &str {
        crate::NativeXmap10ShaderResourceView::descriptor_sha256(self)
    }

    fn raw_shader_resource_view(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.as_raw()
    }
}

#[cfg(windows)]
unsafe impl NativeXmap08SceneConstantBuffer for crate::NativeXmap10MapCopySceneCb0View<'_> {
    fn map_copy_receipt_set_sha256(&self) -> &str {
        crate::NativeXmap10MapCopySceneCb0View::map_copy_receipt_set_sha256(self)
    }

    fn render_view_mask(&self) -> u8 {
        crate::NativeXmap10MapCopySceneCb0View::render_view_mask(self)
    }

    fn map_copy_offset_f32_bits(&self) -> u32 {
        crate::NativeXmap10MapCopySceneCb0View::map_copy_offset_f32_bits(self)
    }

    fn composition_receipt_sha256(&self) -> &str {
        crate::NativeXmap10MapCopySceneCb0View::composition_receipt_sha256(self)
    }

    fn reflected_layout_sha256(&self) -> &str {
        crate::NativeXmap10MapCopySceneCb0View::reflected_layout_sha256(self)
    }

    fn main_scene_resource_identity_sha256(&self) -> &str {
        crate::NativeXmap10MapCopySceneCb0View::main_scene_resource_identity_sha256(self)
    }

    fn raw_main_scene_color_texture(&self) -> *mut winapi::um::d3d11::ID3D11Texture2D {
        crate::NativeXmap10MapCopySceneCb0View::raw_main_scene_color_texture(self)
    }
}

#[cfg(windows)]
pub struct NativeXmap08RiverProviderInputs<'a> {
    /// Exact XMAP04/XMAP07/XMAP09/XMAP10 resources retained by the optimized river program.
    /// XMAP05 resources are borrowed directly from `political`.
    pub shader_resources: &'a [&'a dyn NativeXmap08RiverShaderResource],
    /// One XMAP10 aggregate CB0 per XMAP04 canonical map-copy row.
    pub scene_cb0_by_map_copy: &'a [&'a dyn NativeXmap08SceneConstantBuffer],
    /// Required only when the effective high-GFX program retains CB2.
    pub point_lights_cb2: Option<&'a dyn crate::NativeXmap04UnderwaterPointLightsConstantBuffer>,
    pub temporal: &'a Xmap08RiverTemporalBinding,
    pub shadow_matrix: Option<&'a Xmap08RiverShadowMatrixBinding>,
}

#[cfg(windows)]
struct NativeMapShaderResourceBinding<'a> {
    effect_name: String,
    symbol: String,
    shader_register: u32,
    resource_type: String,
    owner_packet: String,
    resource_sha256: String,
    descriptor_sha256: String,
    shader_resource_view: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
    _provider: std::marker::PhantomData<&'a ()>,
}

#[cfg(windows)]
pub struct NativeXmap08RiversStraitsHandoff {
    device: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Device>,
    river_program: NativeMapProgram,
    strait_program: NativeMapProgram,
    river_vertex: Option<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>>,
    river_indices: BTreeMap<String, crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>>,
    strait_vertex: Option<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>>,
    strait_index: Option<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>>,
    static_resources: BTreeMap<(String, String), NativeStaticResource>,
    frame: PreparedRiversStraitsFrame,
    receipt: Xmap08D3dHandoffReceipt,
}

#[cfg(windows)]
struct NativeMapProgram {
    vertex_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>,
    pixel_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>,
    input_layout: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>,
    blend_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>,
    depth_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    rasterizer_state: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
    samplers: BTreeMap<String, crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>>,
}

#[cfg(windows)]
struct NativeStaticResource {
    _texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    resource_sha256: String,
    descriptor_sha256: String,
}

#[cfg(windows)]
impl NativeXmap08RiversStraitsHandoff {
    /// Upload the complete XMAP08-owned portion of the river/strait pass.
    ///
    /// # Safety
    /// `device` must remain live for the lifetime of the returned handoff.
    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &GameFileResolver,
        frame: &PreparedRiversStraitsFrame,
    ) -> Result<Self> {
        if device.is_null() {
            return Err("XMAP08 D3D handoff received a null device".into());
        }
        (*(device as *mut winapi::um::unknwnbase::IUnknown)).AddRef();
        let retained_device =
            crate::d3d11_device::D3dPtr::from_raw(device, "XMAP08 retained device")?;
        frame.validate()?;
        let river_program = create_program(device, &frame.river_program)?;
        let strait_program = create_program(device, &frame.strait_program)?;
        let river_vertex = (!frame.river_draws.is_empty())
            .then(|| {
                create_buffer(
                    device,
                    &frame.river_vertices,
                    winapi::um::d3d11::D3D11_BIND_VERTEX_BUFFER,
                    "XMAP08 river vertex",
                )
            })
            .transpose()?;
        let mut river_indices = BTreeMap::new();
        for draw in &frame.river_draws {
            if !river_indices.contains_key(&draw.indices_sha256) {
                river_indices.insert(
                    draw.indices_sha256.clone(),
                    create_buffer(
                        device,
                        &draw.indices,
                        winapi::um::d3d11::D3D11_BIND_INDEX_BUFFER,
                        "XMAP08 river relative index",
                    )?,
                );
            }
        }
        let strait_vertex = (!frame.strait_vertices.is_empty())
            .then(|| {
                create_buffer(
                    device,
                    &frame.strait_vertices,
                    winapi::um::d3d11::D3D11_BIND_VERTEX_BUFFER,
                    "XMAP08 strait vertex",
                )
            })
            .transpose()?;
        let strait_index = (!frame.strait_indices.is_empty())
            .then(|| {
                create_buffer(
                    device,
                    &frame.strait_indices,
                    winapi::um::d3d11::D3D11_BIND_INDEX_BUFFER,
                    "XMAP08 strait index",
                )
            })
            .transpose()?;
        let mut static_resources = BTreeMap::new();
        for program in [&frame.river_program, &frame.strait_program] {
            for (_, sampler) in reflected_resource_requirements(program)? {
                if sampler.owner_packet != "XMAP08" {
                    continue;
                }
                let content = sampler
                    .admitted_static_content
                    .first()
                    .ok_or("XMAP08 retained static sampler has no admitted payload")?;
                let bytes = resolver.read_admitted_used_content(content)?;
                let descriptor = crate::mod_resolver::foundation_dds_descriptor(&bytes, content)?;
                let resource = create_static_texture(device, &bytes, &descriptor, content)?;
                let key = (program.effect_name.clone(), sampler.name.clone());
                if static_resources.insert(key, resource).is_some() {
                    return Err("XMAP08 static resource key is duplicated".into());
                }
            }
        }
        let receipt = initial_receipt(frame, river_indices.len(), static_resources.len())?;
        Ok(Self {
            device: retained_device,
            river_program,
            strait_program,
            river_vertex,
            river_indices,
            strait_vertex,
            strait_index,
            static_resources,
            frame: frame.clone(),
            receipt,
        })
    }

    /// Replace frame-dependent geometry/program uploads without accepting a
    /// stale handoff identity.
    ///
    /// # Safety
    /// `device` must be the same live device used by `create_on_device`.
    pub unsafe fn update_frame(
        &mut self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &GameFileResolver,
        frame: &PreparedRiversStraitsFrame,
    ) -> Result<&Xmap08D3dHandoffReceipt> {
        if device.is_null() || device != self.device.as_raw() {
            return Err("XMAP08 frame update received a different or null D3D11 device".into());
        }
        frame.validate()?;
        if frame.deterministic_sha256 == self.receipt.frame_sha256 {
            self.receipt =
                initial_receipt(frame, self.river_indices.len(), self.static_resources.len())?;
            return Ok(&self.receipt);
        }
        let replacement = Self::create_on_device(device, resolver, frame)?;
        *self = replacement;
        Ok(&self.receipt)
    }

    pub fn receipt(&self) -> &Xmap08D3dHandoffReceipt {
        &self.receipt
    }

    /// Execute every admitted strait view on the application's actual scene
    /// target. Vanilla's optimized strait program retains only XMAP08's
    /// admitted DiffuseTexture and camera CB0; effective mod shaders also
    /// receive any reflected XMAP05 FoW resources through the typed provider.
    ///
    /// # Safety
    /// `context` and `target` must belong to the device used for this handoff.
    pub unsafe fn execute_straits(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        target: &NativeProductionMainSceneTarget,
        viewer: &ViewerOwnedMapState,
        authority: &Xmap08ExecutionFrameAuthority,
        political: Option<&NativeXmap05PoliticalHandoff>,
    ) -> Result<&Xmap08D3dHandoffReceipt> {
        if context.is_null() {
            return Err("XMAP08 strait execution received a null context".into());
        }
        validate_context_device(context, self.device.as_raw())?;
        validate_target_device(target, self.device.as_raw())?;
        validate_completed_pass(
            "river",
            self.receipt.retained_river_draws,
            self.receipt.executed_river_draws,
            self.receipt.river_ready_contract_sha256.as_deref(),
            &self.receipt.river_draw_receipts,
            self.receipt.river_component_closure.as_ref(),
        )
        .map_err(|error| {
            format!("XMAP08 straits cannot execute before the river pass completes: {error}")
        })?;
        if self.receipt.strait_component_closure.is_some()
            || self.receipt.executed_strait_draws != 0
            || self.receipt.strait_ready_contract_sha256.is_some()
            || !self.receipt.strait_draw_receipts.is_empty()
        {
            return Err(
                "XMAP08 strait pass cannot execute more than once per retained frame".into(),
            );
        }
        let target_desc = target.desc();
        let camera = validate_execution_authority(&self.frame, viewer, target_desc, authority)?;
        let target_width = target_desc.color_width;
        let target_height = target_desc.color_height;
        validate_execution_camera(&self.frame, camera, target_width, target_height)?;
        validate_strait_cb0_used_fields(&self.frame.strait_program)?;
        if self.frame.strait_draws.is_empty() {
            self.receipt.executed_strait_draws = 0;
            self.receipt.strait_ready_contract_sha256 = None;
            self.receipt.strait_draw_receipts.clear();
            self.receipt.strait_component_closure = Some(component_closure(
                authority,
                &authority.strait_assignment,
                XMAP08_STRAIT_COMPONENT_ID,
                &self.frame,
                &self.frame.strait_program,
                &self.frame.strait_enumerator_witness_sha256,
                &[],
            )?);
            reseal_receipt(&mut self.receipt)?;
            return Ok(&self.receipt);
        }
        let vertex = self
            .strait_vertex
            .as_ref()
            .ok_or("XMAP08 strait draw has no uploaded vertex buffer")?;
        let index = self
            .strait_index
            .as_ref()
            .ok_or("XMAP08 strait draw has no uploaded index buffer")?;
        let mut planned_cb0 = Vec::with_capacity(self.frame.strait_draws.len());
        let mut cb0_draw_hashes = Vec::with_capacity(self.frame.strait_draws.len());
        let mut cb0_descriptor = None;
        for draw in &self.frame.strait_draws {
            let view_camera =
                camera_for_map_copy(camera, f32::from_bits(draw.map_copy_offset_f32_bits))?;
            let bytes = strait_camera_cb0_bytes(&self.frame.strait_program, view_camera, camera)?;
            let descriptor_sha256 =
                constant_buffer_descriptor_sha256(&self.frame.strait_program, 0, bytes.len())?;
            if cb0_descriptor
                .as_ref()
                .is_some_and(|current| current != &descriptor_sha256)
            {
                return Err("XMAP08 strait CB0 descriptor changed between map copies".into());
            }
            let payload_sha256 = Sha256::hex(&bytes);
            let buffer = create_constant_buffer_from_bytes(
                self.device.as_raw(),
                &bytes,
                "XMAP08 strait CB0",
            )?;
            cb0_draw_hashes.push(payload_sha256.clone());
            cb0_descriptor = Some(descriptor_sha256.clone());
            planned_cb0.push((buffer, payload_sha256, descriptor_sha256));
        }
        bind_target_and_pipeline(
            context,
            target.color_rtv(),
            target.depth_dsv(),
            &self.strait_program,
            vertex,
            index,
            target_width,
            target_height,
        );
        let mut logical_resources = Vec::new();
        bind_static_resources(
            context,
            &self.frame.strait_program,
            &self.static_resources,
            &self.strait_program,
            &mut logical_resources,
        )?;
        let downstream =
            strait_resource_bindings(&self.frame.strait_program, political, self.device.as_raw())?;
        bind_downstream_resources(
            context,
            &self.frame.strait_program,
            &downstream,
            &self.strait_program,
            &mut logical_resources,
        )?;
        let resource_set_sha256 = bound_resource_set_sha256(&logical_resources)?;
        let planned_draw_receipts = self
            .frame
            .strait_draws
            .iter()
            .zip(&planned_cb0)
            .enumerate()
            .map(
                |(ordinal, (draw, (_, payload_sha256, descriptor_sha256)))| {
                    let draw_constant_buffers = vec![MapConstantBufferBinding {
                        effect_name: self.frame.strait_program.effect_name.clone(),
                        shader_register: 0,
                        owner_packet: "XMAP03".into(),
                        payload_sha256: payload_sha256.clone(),
                        descriptor_sha256: descriptor_sha256.clone(),
                    }];
                    target_bound_draw_receipt(
                        authority,
                        &authority.strait_assignment,
                        XMAP08_STRAIT_COMPONENT_ID,
                        &self.frame,
                        &self.frame.strait_program,
                        u32::try_from(ordinal)?,
                        draw.render_view_mask,
                        draw.map_copy_offset_f32_bits,
                        draw.index_count,
                        0,
                        0,
                        resource_set_sha256.clone(),
                        bound_constant_buffer_set_sha256(&draw_constant_buffers)?,
                    )
                },
            )
            .collect::<Result<Vec<_>>>()?;
        for (draw, (buffer, _, _)) in self.frame.strait_draws.iter().zip(&planned_cb0) {
            let raw = buffer.as_raw();
            (*context).VSSetConstantBuffers(0, 1, &raw);
            (*context).PSSetConstantBuffers(0, 1, &raw);
            (*context).DrawIndexed(draw.index_count, 0, 0);
        }
        crate::d3d11_device::native_d3d11_check_void_upload_device(
            &self.device,
            "XMAP08 strait DrawIndexed submission",
        )?;
        let logical_cbuffers = vec![MapConstantBufferBinding {
            effect_name: self.frame.strait_program.effect_name.clone(),
            shader_register: 0,
            owner_packet: "XMAP03".into(),
            payload_sha256: aggregate_draw_payload_sha256(&cb0_draw_hashes),
            descriptor_sha256: cb0_descriptor
                .ok_or("XMAP08 strait execution produced no CB0 descriptor")?,
        }];
        let ready = ready_contract_for_executed_program(
            &self.frame,
            &self.frame.strait_program,
            logical_resources,
            logical_cbuffers,
        )?;
        let executed = u32::try_from(self.frame.strait_draws.len())
            .map_err(|_| "XMAP08 executed strait draw count exceeds u32")?;
        self.receipt.executed_strait_draws = executed;
        self.receipt.strait_ready_contract_sha256 = Some(ready.deterministic_sha256);
        self.receipt.strait_draw_receipts = planned_draw_receipts;
        self.receipt.strait_component_closure = Some(component_closure(
            authority,
            &authority.strait_assignment,
            XMAP08_STRAIT_COMPONENT_ID,
            &self.frame,
            &self.frame.strait_program,
            &self.frame.strait_enumerator_witness_sha256,
            &self.receipt.strait_draw_receipts,
        )?);
        reseal_receipt(&mut self.receipt)?;
        unbind_resources(context, &self.frame.strait_program)?;
        Ok(&self.receipt)
    }

    /// Execute every admitted river strip only after XMAP04/XMAP07/XMAP09/
    /// XMAP10 provide the real reflected runtime resources and full scene
    /// constants.
    ///
    /// # Safety
    /// All pointers must belong to the same device used for this handoff.
    pub unsafe fn execute_rivers(
        &mut self,
        context: *mut winapi::um::d3d11::ID3D11DeviceContext,
        target: &NativeProductionMainSceneTarget,
        viewer: &ViewerOwnedMapState,
        authority: &Xmap08ExecutionFrameAuthority,
        political: &NativeXmap05PoliticalHandoff,
        providers: &NativeXmap08RiverProviderInputs<'_>,
    ) -> Result<&Xmap08D3dHandoffReceipt> {
        if context.is_null() {
            return Err("XMAP08 river execution received a null context".into());
        }
        validate_context_device(context, self.device.as_raw())?;
        validate_target_device(target, self.device.as_raw())?;
        if self.receipt.river_component_closure.is_some()
            || self.receipt.strait_component_closure.is_some()
            || self.receipt.executed_river_draws != 0
            || self.receipt.executed_strait_draws != 0
            || self.receipt.river_ready_contract_sha256.is_some()
            || self.receipt.strait_ready_contract_sha256.is_some()
            || !self.receipt.river_draw_receipts.is_empty()
            || !self.receipt.strait_draw_receipts.is_empty()
        {
            return Err(
                "XMAP08 river pass must be the first and only first-pass execution for a retained frame"
                    .into(),
            );
        }
        let target_desc = target.desc();
        let camera = validate_execution_authority(&self.frame, viewer, target_desc, authority)?;
        let target_width = target_desc.color_width;
        let target_height = target_desc.color_height;
        validate_execution_camera(&self.frame, camera, target_width, target_height)?;
        if self.frame.river_draws.is_empty() {
            self.receipt.executed_river_draws = 0;
            self.receipt.river_ready_contract_sha256 = None;
            self.receipt.river_draw_receipts.clear();
            self.receipt.river_component_closure = Some(component_closure(
                authority,
                &authority.river_assignment,
                XMAP08_RIVER_COMPONENT_ID,
                &self.frame,
                &self.frame.river_program,
                &self.frame.river_enumerator_witness_sha256,
                &[],
            )?);
            reseal_receipt(&mut self.receipt)?;
            return Ok(&self.receipt);
        }
        validate_river_cb1_owned_variables(&self.frame.river_program)?;
        let river_vertex = self
            .river_vertex
            .as_ref()
            .ok_or("XMAP08 river draw has no uploaded vertex buffer")?;
        let first_index = self
            .river_indices
            .values()
            .next()
            .ok_or("XMAP08 river execution has no retained index buffer")?;
        if political.receipt().political_frame_sha256 != authority.political_frame_sha256 {
            return Err(
                "XMAP08 political resources belong to a different prepared replay frame".into(),
            );
        }
        let downstream_resources = river_resource_bindings(
            &self.frame.river_program,
            political,
            providers.shader_resources,
            &self.frame.camera_view_sha256,
            &authority.replay_frame_identity_sha256,
            self.device.as_raw(),
        )?;
        validate_downstream_resource_bindings(&self.frame.river_program, &downstream_resources)?;
        let scene_cb0_by_map_copy = validate_scene_cb0_provider_set(
            &self.frame.river_program,
            &self.frame,
            authority,
            providers.scene_cb0_by_map_copy,
            target_width,
            target_height,
            target.color_texture(),
            self.device.as_raw(),
        )?;
        let point_lights_cb2 = validate_point_lights_cb2_provider(
            &self.frame.river_program,
            &self.frame,
            authority,
            providers.point_lights_cb2,
            self.device.as_raw(),
        )?;
        let river_cb1_byte_len = reflected_cbuffer_size(&self.frame.river_program, 1)? as usize;
        let river_cb1_descriptor =
            constant_buffer_descriptor_sha256(&self.frame.river_program, 1, river_cb1_byte_len)?;
        let device = self.device.as_raw();
        let mut logical_cbuffer_draw_hashes = BTreeMap::<(u32, String, String), Vec<String>>::new();
        struct PlannedRiverDraw {
            scene_cb0: *mut winapi::um::d3d11::ID3D11Buffer,
            point_lights_cb2: Option<*mut winapi::um::d3d11::ID3D11Buffer>,
            cb1_buffer: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
            logical_cbuffers: Vec<MapConstantBufferBinding>,
        }
        let mut planned_draws = Vec::with_capacity(self.frame.river_draws.len());
        for draw in &self.frame.river_draws {
            self.river_indices
                .get(&draw.indices_sha256)
                .ok_or("XMAP08 river draw has no matching uploaded relative index buffer")?;
            let mut draw_logical_cbuffers = Vec::new();
            let scene_cb0 = scene_cb0_by_map_copy
                .iter()
                .copied()
                .find(|binding| {
                    binding.render_view_mask() == draw.render_view_mask
                        && binding.map_copy_offset_f32_bits() == draw.map_copy_offset_f32_bits
                })
                .ok_or("XMAP08 river draw has no matching XMAP10 SceneCB0 map-copy row")?;
            let cb0_owner = self
                .frame
                .river_program
                .constant_buffer_owners
                .iter()
                .find_map(|(slot, owner)| (*slot == 0).then_some(owner.clone()))
                .ok_or("XMAP08 river program omitted CB0 ownership")?;
            logical_cbuffer_draw_hashes
                .entry((0, cb0_owner.clone(), scene_cb0.descriptor_sha256().into()))
                .or_default()
                .push(scene_cb0.payload_sha256().into());
            draw_logical_cbuffers.push(MapConstantBufferBinding {
                effect_name: self.frame.river_program.effect_name.clone(),
                shader_register: 0,
                owner_packet: cb0_owner,
                payload_sha256: scene_cb0.payload_sha256().into(),
                descriptor_sha256: scene_cb0.descriptor_sha256().into(),
            });
            if let Some(point_lights) = point_lights_cb2 {
                let cb2_owner = self
                    .frame
                    .river_program
                    .constant_buffer_owners
                    .iter()
                    .find_map(|(slot, owner)| (*slot == 2).then_some(owner.clone()))
                    .ok_or("XMAP08 river program omitted retained CB2 ownership")?;
                logical_cbuffer_draw_hashes
                    .entry((
                        2,
                        cb2_owner.clone(),
                        point_lights.descriptor_sha256().into(),
                    ))
                    .or_default()
                    .push(point_lights.payload_sha256().into());
                draw_logical_cbuffers.push(MapConstantBufferBinding {
                    effect_name: self.frame.river_program.effect_name.clone(),
                    shader_register: 2,
                    owner_packet: cb2_owner,
                    payload_sha256: point_lights.payload_sha256().into(),
                    descriptor_sha256: point_lights.descriptor_sha256().into(),
                });
            }
            let river_cb1 = compose_river_cb1(
                &self.frame.river_program,
                providers.shadow_matrix,
                providers.temporal,
                &self.frame.camera_view_sha256,
                &authority.replay_frame_identity_sha256,
                draw.render_group,
            )?;
            let buffer = create_constant_buffer_from_bytes(
                device,
                &river_cb1,
                "XMAP08 river composite CB1",
            )?;
            let river_cb1_payload_sha256 = Sha256::hex(&river_cb1);
            let river_cb1_owner = self
                .frame
                .river_program
                .constant_buffer_owners
                .iter()
                .find_map(|(slot, owner)| (*slot == 1).then_some(owner.clone()))
                .ok_or("XMAP08 river program omitted CB1 ownership")?;
            logical_cbuffer_draw_hashes
                .entry((1, river_cb1_owner.clone(), river_cb1_descriptor.clone()))
                .or_default()
                .push(river_cb1_payload_sha256.clone());
            draw_logical_cbuffers.push(MapConstantBufferBinding {
                effect_name: self.frame.river_program.effect_name.clone(),
                shader_register: 1,
                owner_packet: river_cb1_owner,
                payload_sha256: river_cb1_payload_sha256,
                descriptor_sha256: river_cb1_descriptor.clone(),
            });
            planned_draws.push(PlannedRiverDraw {
                scene_cb0: scene_cb0.raw_buffer(),
                point_lights_cb2: point_lights_cb2.map(|binding| binding.raw_buffer()),
                cb1_buffer: buffer,
                logical_cbuffers: draw_logical_cbuffers,
            });
        }
        bind_target_and_pipeline(
            context,
            target.color_rtv(),
            target.depth_dsv(),
            &self.river_program,
            river_vertex,
            first_index,
            target_width,
            target_height,
        );
        let mut logical_resources = Vec::new();
        bind_static_resources(
            context,
            &self.frame.river_program,
            &self.static_resources,
            &self.river_program,
            &mut logical_resources,
        )?;
        bind_downstream_resources(
            context,
            &self.frame.river_program,
            &downstream_resources,
            &self.river_program,
            &mut logical_resources,
        )?;
        let resource_set_sha256 = bound_resource_set_sha256(&logical_resources)?;
        let planned_draw_receipts = self
            .frame
            .river_draws
            .iter()
            .zip(&planned_draws)
            .enumerate()
            .map(|(ordinal, (draw, planned))| {
                target_bound_draw_receipt(
                    authority,
                    &authority.river_assignment,
                    XMAP08_RIVER_COMPONENT_ID,
                    &self.frame,
                    &self.frame.river_program,
                    u32::try_from(ordinal)?,
                    draw.render_view_mask,
                    draw.map_copy_offset_f32_bits,
                    draw.index_count,
                    0,
                    draw.base_vertex,
                    resource_set_sha256.clone(),
                    bound_constant_buffer_set_sha256(&planned.logical_cbuffers)?,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        for (draw, planned) in self.frame.river_draws.iter().zip(&planned_draws) {
            let index = self
                .river_indices
                .get(&draw.indices_sha256)
                .ok_or("XMAP08 planned river index disappeared before submission")?;
            (*context).IASetIndexBuffer(
                index.as_raw(),
                winapi::shared::dxgiformat::DXGI_FORMAT_R16_UINT,
                0,
            );
            (*context).VSSetConstantBuffers(0, 1, &planned.scene_cb0);
            (*context).PSSetConstantBuffers(0, 1, &planned.scene_cb0);
            let raw = planned.cb1_buffer.as_raw();
            (*context).VSSetConstantBuffers(1, 1, &raw);
            (*context).PSSetConstantBuffers(1, 1, &raw);
            if let Some(cb2) = planned.point_lights_cb2 {
                (*context).VSSetConstantBuffers(2, 1, &cb2);
                (*context).PSSetConstantBuffers(2, 1, &cb2);
            }
            (*context).DrawIndexed(draw.index_count, 0, draw.base_vertex);
        }
        crate::d3d11_device::native_d3d11_check_void_upload_device(
            &self.device,
            "XMAP08 river DrawIndexed submission",
        )?;
        let logical_cbuffers = logical_cbuffer_draw_hashes
            .into_iter()
            .map(
                |((shader_register, owner_packet, descriptor_sha256), hashes)| {
                    MapConstantBufferBinding {
                        effect_name: self.frame.river_program.effect_name.clone(),
                        shader_register,
                        owner_packet,
                        payload_sha256: aggregate_draw_payload_sha256(&hashes),
                        descriptor_sha256,
                    }
                },
            )
            .collect();
        let ready = ready_contract_for_executed_program(
            &self.frame,
            &self.frame.river_program,
            logical_resources,
            logical_cbuffers,
        )?;
        let executed = u32::try_from(self.frame.river_draws.len())
            .map_err(|_| "XMAP08 executed river draw count exceeds u32")?;
        self.receipt.executed_river_draws = executed;
        self.receipt.river_ready_contract_sha256 = Some(ready.deterministic_sha256);
        self.receipt.river_draw_receipts = planned_draw_receipts;
        self.receipt.river_component_closure = Some(component_closure(
            authority,
            &authority.river_assignment,
            XMAP08_RIVER_COMPONENT_ID,
            &self.frame,
            &self.frame.river_program,
            &self.frame.river_enumerator_witness_sha256,
            &self.receipt.river_draw_receipts,
        )?);
        reseal_receipt(&mut self.receipt)?;
        unbind_resources(context, &self.frame.river_program)?;
        Ok(&self.receipt)
    }
}

#[cfg(windows)]
unsafe fn validate_context_device(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    expected: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<()> {
    let mut actual = std::ptr::null_mut();
    (*context).GetDevice(&mut actual);
    let actual = crate::d3d11_device::D3dPtr::from_raw(actual, "XMAP08 context device")?;
    if actual.as_raw() != expected {
        return Err("XMAP08 execution context belongs to a different D3D11 device".into());
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn validate_target_device(
    target: &NativeProductionMainSceneTarget,
    expected: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<()> {
    use winapi::um::d3d11::ID3D11Resource;

    let validate_view = |label: &str, resource: *mut ID3D11Resource| -> Result<()> {
        let resource =
            crate::d3d11_device::D3dPtr::from_raw(resource, &format!("XMAP08 {label} resource"))?;
        let mut actual = std::ptr::null_mut();
        (*resource.as_raw()).GetDevice(&mut actual);
        let actual =
            crate::d3d11_device::D3dPtr::from_raw(actual, &format!("XMAP08 {label} device"))?;
        if actual.as_raw() != expected {
            return Err(format!("XMAP08 {label} belongs to a different D3D11 device").into());
        }
        Ok(())
    };

    let mut color_resource = std::ptr::null_mut();
    (*target.color_rtv()).GetResource(&mut color_resource);
    validate_view("MainScene color target", color_resource)?;
    let mut depth_resource = std::ptr::null_mut();
    (*target.depth_dsv()).GetResource(&mut depth_resource);
    validate_view("MainScene depth target", depth_resource)
}

#[cfg(windows)]
unsafe fn validate_srv_device(
    view: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
    expected: *mut winapi::um::d3d11::ID3D11Device,
    label: &str,
) -> Result<()> {
    if view.is_null() {
        return Err(format!("XMAP08 {label} SRV is null").into());
    }
    let mut resource = std::ptr::null_mut();
    (*view).GetResource(&mut resource);
    let resource =
        crate::d3d11_device::D3dPtr::from_raw(resource, &format!("XMAP08 {label} SRV resource"))?;
    let mut actual = std::ptr::null_mut();
    (*resource.as_raw()).GetDevice(&mut actual);
    let actual =
        crate::d3d11_device::D3dPtr::from_raw(actual, &format!("XMAP08 {label} SRV device"))?;
    if actual.as_raw() != expected {
        return Err(format!("XMAP08 {label} SRV belongs to a different D3D11 device").into());
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn validate_buffer_device(
    buffer: *mut winapi::um::d3d11::ID3D11Buffer,
    expected: *mut winapi::um::d3d11::ID3D11Device,
    label: &str,
) -> Result<()> {
    if buffer.is_null() {
        return Err(format!("XMAP08 {label} buffer is null").into());
    }
    let mut actual = std::ptr::null_mut();
    (*(buffer as *mut winapi::um::d3d11::ID3D11DeviceChild)).GetDevice(&mut actual);
    let actual =
        crate::d3d11_device::D3dPtr::from_raw(actual, &format!("XMAP08 {label} buffer device"))?;
    if actual.as_raw() != expected {
        return Err(format!("XMAP08 {label} buffer belongs to a different D3D11 device").into());
    }
    Ok(())
}

fn river_flow_direction_f32_bits(render_group: u8) -> u32 {
    // through the largest retained `record + 0x24` value, writing -1.0 for an
    // even loop index and +1.0 for an odd index before drawing that group.
    // Evidence job 20260730T123519Z-xmap08-river-render-group-direction-fina-80bf83a9,
    // decompile SHA-256 9913e0a2c517408ed2c87eabf9a952eaadf445056f486dfc78e93b32eb70812e.
    if render_group & 1 == 0 {
        (-1.0f32).to_bits()
    } else {
        1.0f32.to_bits()
    }
}

fn validate_execution_camera(
    frame: &PreparedRiversStraitsFrame,
    camera: &ViewerMapCameraState,
    target_width: u32,
    target_height: u32,
) -> Result<()> {
    camera.validate()?;
    if camera.deterministic_sha256 != frame.camera_view_sha256
        || camera.map_width != frame.map_width
        || camera.map_height != frame.map_height
        || camera.viewport_width != target_width
        || camera.viewport_height != target_height
    {
        return Err(
            "XMAP08 execution camera/viewport does not match the prepared river/strait frame"
                .into(),
        );
    }
    Ok(())
}

fn validate_execution_authority<'a>(
    frame: &PreparedRiversStraitsFrame,
    viewer: &'a ViewerOwnedMapState,
    target: &ProductionMainSceneTargetDesc,
    authority: &Xmap08ExecutionFrameAuthority,
) -> Result<&'a ViewerMapCameraState> {
    authority.validate()?;
    viewer.validate()?;
    target.validate()?;
    let viewer_sha256 = Sha256::hex(&serde_json::to_vec(viewer)?);
    if viewer_sha256 != authority.viewer_state_sha256 {
        return Err(
            "XMAP08 execution viewer state differs from its authenticated authority".into(),
        );
    }
    let target_sha256 = Sha256::hex(&serde_json::to_vec(target)?);
    if target_sha256 != authority.main_scene_target_sha256 {
        return Err(
            "XMAP08 execution MainScene target differs from its authenticated authority".into(),
        );
    }
    if authority.river_assignment != frame.river_assignment
        || authority.strait_assignment != frame.strait_assignment
    {
        return Err(
            "XMAP08 execution assigned stages differ from the prepared frame authority".into(),
        );
    }
    let camera = viewer
        .camera
        .as_ref()
        .ok_or("XMAP08 execution authority has no viewer camera")?;
    if camera.deterministic_sha256 != frame.camera_view_sha256 {
        return Err("XMAP08 execution authority camera differs from the prepared frame".into());
    }
    Ok(camera)
}

#[cfg(windows)]
unsafe fn river_resource_bindings<'a>(
    program: &PreparedMapEffectProgram,
    political: &'a NativeXmap05PoliticalHandoff,
    external: &'a [&'a dyn NativeXmap08RiverShaderResource],
    camera_frame_sha256: &str,
    replay_frame_identity_sha256: &str,
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<Vec<NativeMapShaderResourceBinding<'a>>> {
    let expected = reflected_resource_requirements(program)?
        .into_iter()
        .filter(|(_, sampler)| sampler.owner_packet != "XMAP08")
        .collect::<Vec<_>>();
    let expected_external = expected
        .iter()
        .filter(|(_, sampler)| sampler.owner_packet != "XMAP05")
        .count();
    let supplied = external
        .iter()
        .map(|resource| {
            (
                resource.owner_packet(),
                resource.symbol(),
                resource.shader_register(),
            )
        })
        .collect::<BTreeSet<_>>();
    if external.len() != expected_external || supplied.len() != external.len() {
        return Err(
                "XMAP08 typed XMAP04/XMAP07/XMAP09/XMAP10 resource set is incomplete, duplicated, or contains extras"
                .into(),
        );
    }
    let mut bindings = Vec::with_capacity(expected.len());
    for (_, sampler) in expected {
        let binding = if sampler.owner_packet == "XMAP05" {
            let resource = political.shader_resource(&sampler.name)?;
            if resource.owner_packet != "XMAP05"
                || resource.symbol != sampler.name
                || resource.shader_resource_view.is_null()
            {
                return Err(
                    format!("XMAP08 typed XMAP05 resource {} is invalid", sampler.name).into(),
                );
            }
            validate_canonical_sha256(resource.resource_sha256, "XMAP08 XMAP05 resource")?;
            validate_canonical_sha256(
                resource.descriptor_sha256,
                "XMAP08 XMAP05 resource descriptor",
            )?;
            validate_srv_device(resource.shader_resource_view, device, &sampler.name)?;
            NativeMapShaderResourceBinding {
                effect_name: program.effect_name.clone(),
                symbol: sampler.name.clone(),
                shader_register: sampler.shader_register,
                resource_type: sampler.resource_type.clone(),
                owner_packet: "XMAP05".into(),
                resource_sha256: resource.resource_sha256.into(),
                descriptor_sha256: resource.descriptor_sha256.into(),
                shader_resource_view: resource.shader_resource_view,
                _provider: std::marker::PhantomData,
            }
        } else {
            let resource = external
                .iter()
                .copied()
                .find(|resource| {
                    resource.owner_packet() == sampler.owner_packet
                        && resource.symbol() == sampler.name
                        && resource.shader_register() == sampler.shader_register
                })
                .ok_or_else(|| {
                    format!(
                        "XMAP08 river draw lacks typed {} resource {} at t{}",
                        sampler.owner_packet, sampler.name, sampler.shader_register
                    )
                })?;
            validate_external_river_resource_identity(
                resource,
                &sampler.owner_packet,
                &sampler.name,
                sampler.shader_register,
                camera_frame_sha256,
                replay_frame_identity_sha256,
            )?;
            if resource.raw_shader_resource_view().is_null() {
                return Err(format!(
                    "XMAP08 {} resource {} has a null shader-resource view",
                    sampler.owner_packet, sampler.name
                )
                .into());
            }
            validate_srv_device(resource.raw_shader_resource_view(), device, &sampler.name)?;
            NativeMapShaderResourceBinding {
                effect_name: program.effect_name.clone(),
                symbol: sampler.name.clone(),
                shader_register: sampler.shader_register,
                resource_type: sampler.resource_type.clone(),
                owner_packet: sampler.owner_packet.clone(),
                resource_sha256: resource.resource_sha256().into(),
                descriptor_sha256: resource.descriptor_sha256().into(),
                shader_resource_view: resource.raw_shader_resource_view(),
                _provider: std::marker::PhantomData,
            }
        };
        bindings.push(binding);
    }
    Ok(bindings)
}

#[cfg(windows)]
unsafe fn strait_resource_bindings<'a>(
    program: &PreparedMapEffectProgram,
    political: Option<&'a NativeXmap05PoliticalHandoff>,
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<Vec<NativeMapShaderResourceBinding<'a>>> {
    let expected = reflected_resource_requirements(program)?
        .into_iter()
        .filter(|(_, sampler)| sampler.owner_packet != "XMAP08")
        .collect::<Vec<_>>();
    if expected
        .iter()
        .any(|(_, sampler)| sampler.owner_packet != "XMAP05")
    {
        return Err(
            "XMAP08 strait program retained a downstream resource without a typed provider".into(),
        );
    }
    let political = match (expected.is_empty(), political) {
        (true, _) => return Ok(Vec::new()),
        (false, Some(political)) => political,
        (false, None) => {
            return Err("XMAP08 strait program retained XMAP05 resources without a provider".into())
        }
    };
    let mut bindings = Vec::with_capacity(expected.len());
    for (_, sampler) in expected {
        let resource = political.shader_resource(&sampler.name)?;
        if resource.owner_packet != "XMAP05"
            || resource.symbol != sampler.name
            || resource.shader_resource_view.is_null()
        {
            return Err(format!(
                "XMAP08 typed XMAP05 strait resource {} is invalid",
                sampler.name
            )
            .into());
        }
        validate_canonical_sha256(resource.resource_sha256, "XMAP08 strait XMAP05 resource")?;
        validate_canonical_sha256(
            resource.descriptor_sha256,
            "XMAP08 strait XMAP05 resource descriptor",
        )?;
        validate_srv_device(resource.shader_resource_view, device, &sampler.name)?;
        bindings.push(NativeMapShaderResourceBinding {
            effect_name: program.effect_name.clone(),
            symbol: sampler.name.clone(),
            shader_register: sampler.shader_register,
            resource_type: sampler.resource_type.clone(),
            owner_packet: "XMAP05".into(),
            resource_sha256: resource.resource_sha256.into(),
            descriptor_sha256: resource.descriptor_sha256.into(),
            shader_resource_view: resource.shader_resource_view,
            _provider: std::marker::PhantomData,
        });
    }
    Ok(bindings)
}

#[cfg(windows)]
fn validate_external_river_resource_identity(
    resource: &dyn NativeXmap08RiverShaderResource,
    expected_owner: &str,
    expected_symbol: &str,
    expected_shader_register: u32,
    camera_frame_sha256: &str,
    replay_frame_identity_sha256: &str,
) -> Result<()> {
    let camera_identity_matches = match expected_owner {
        "XMAP04" | "XMAP07" => resource.camera_frame_sha256().is_none(),
        "XMAP09" | "XMAP10" => resource.camera_frame_sha256() == Some(camera_frame_sha256),
        _ => false,
    };
    if resource.owner_packet() != expected_owner
        || resource.symbol() != expected_symbol
        || resource.shader_register() != expected_shader_register
        || !camera_identity_matches
        || resource.replay_frame_identity_sha256() != replay_frame_identity_sha256
    {
        return Err(format!(
            "XMAP08 {expected_owner} resource {expected_symbol} has the wrong owner, slot, replay, or camera scope"
        )
        .into());
    }
    validate_canonical_sha256(
        resource.replay_frame_identity_sha256(),
        "XMAP08 downstream replay frame",
    )?;
    if let Some(camera) = resource.camera_frame_sha256() {
        validate_canonical_sha256(camera, "XMAP08 downstream camera frame")?;
    }
    validate_canonical_sha256(resource.resource_sha256(), "XMAP08 downstream resource")?;
    validate_canonical_sha256(
        resource.descriptor_sha256(),
        "XMAP08 downstream resource descriptor",
    )
}

fn aggregate_draw_payload_sha256(hashes: &[String]) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"XMAP08-DRAW-CBUFFER-V1\0");
    for hash in hashes {
        bytes.extend_from_slice(hash.as_bytes());
        bytes.push(0);
    }
    Sha256::hex(&bytes)
}

#[cfg(windows)]
fn ready_contract_for_executed_program(
    frame: &PreparedRiversStraitsFrame,
    program: &PreparedMapEffectProgram,
    resources: Vec<MapDependentResourceBinding>,
    constant_buffers: Vec<MapConstantBufferBinding>,
) -> Result<ReadyRiversStraitsDrawContract> {
    let expected_resources = reflected_resource_requirements(program)?;
    let expected_cbuffers = reflected_constant_buffer_requirements(program)?;
    if resources.len() != expected_resources.len()
        || constant_buffers.len() != expected_cbuffers.len()
    {
        return Err("XMAP08 executed program dependency set is incomplete".into());
    }
    validate_program_draw_dependencies(program, &resources, &constant_buffers)?;
    let mut identity = Vec::new();
    identity.extend_from_slice(frame.deterministic_sha256.as_bytes());
    identity.extend_from_slice(program.deterministic_sha256.as_bytes());
    identity.extend_from_slice(format!("{resources:?}").as_bytes());
    identity.extend_from_slice(format!("{constant_buffers:?}").as_bytes());
    Ok(ReadyRiversStraitsDrawContract {
        frame_sha256: frame.deterministic_sha256.clone(),
        ordered_resources: resources,
        ordered_constant_buffers: constant_buffers,
        deterministic_sha256: Sha256::hex(&identity),
    })
}

fn bound_resource_set_sha256(resources: &[MapDependentResourceBinding]) -> Result<String> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"hoi4.xmap08.bound-resource-set.v1\0");
    bytes.extend_from_slice(
        &u32::try_from(resources.len())
            .map_err(|_| "XMAP08 bound-resource set exceeds u32 rows")?
            .to_le_bytes(),
    );
    for resource in resources {
        push_len_prefixed(&mut bytes, resource.effect_name.as_bytes())?;
        push_len_prefixed(&mut bytes, resource.symbol.as_bytes())?;
        push_len_prefixed(&mut bytes, resource.owner_packet.as_bytes())?;
        bytes.extend_from_slice(resource.resource_sha256.as_bytes());
        bytes.extend_from_slice(resource.descriptor_sha256.as_bytes());
    }
    Ok(Sha256::hex(&bytes))
}

fn bound_constant_buffer_set_sha256(buffers: &[MapConstantBufferBinding]) -> Result<String> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"hoi4.xmap08.bound-constant-buffer-set.v1\0");
    bytes.extend_from_slice(
        &u32::try_from(buffers.len())
            .map_err(|_| "XMAP08 bound-constant-buffer set exceeds u32 rows")?
            .to_le_bytes(),
    );
    for buffer in buffers {
        push_len_prefixed(&mut bytes, buffer.effect_name.as_bytes())?;
        bytes.extend_from_slice(&buffer.shader_register.to_le_bytes());
        push_len_prefixed(&mut bytes, buffer.owner_packet.as_bytes())?;
        bytes.extend_from_slice(buffer.payload_sha256.as_bytes());
        bytes.extend_from_slice(buffer.descriptor_sha256.as_bytes());
    }
    Ok(Sha256::hex(&bytes))
}

fn push_len_prefixed(bytes: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    bytes.extend_from_slice(
        &u32::try_from(value.len())
            .map_err(|_| "XMAP08 receipt field exceeds u32 bytes")?
            .to_le_bytes(),
    );
    bytes.extend_from_slice(value);
    Ok(())
}

#[cfg(windows)]
unsafe fn create_program(
    device: *mut winapi::um::d3d11::ID3D11Device,
    program: &PreparedMapEffectProgram,
) -> Result<NativeMapProgram> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ffi::CString;
    use std::ptr;
    use winapi::shared::dxgiformat::DXGI_FORMAT_R32G32B32A32_FLOAT;
    use winapi::um::d3d11::{
        ID3D11InputLayout, ID3D11PixelShader, ID3D11VertexShader, D3D11_INPUT_ELEMENT_DESC,
        D3D11_INPUT_PER_VERTEX_DATA,
    };
    let mut raw_vertex = ptr::null_mut::<ID3D11VertexShader>();
    native_d3d11_hr(
        (*device).CreateVertexShader(
            program.vertex_bytecode.as_ptr() as *const _,
            program.vertex_bytecode.len(),
            ptr::null_mut(),
            &mut raw_vertex,
        ),
        &format!("CreateVertexShader(XMAP08 {})", program.effect_name),
    )?;
    let vertex_shader = D3dPtr::from_raw(raw_vertex, "XMAP08 vertex shader")?;
    let mut raw_pixel = ptr::null_mut::<ID3D11PixelShader>();
    native_d3d11_hr(
        (*device).CreatePixelShader(
            program.pixel_bytecode.as_ptr() as *const _,
            program.pixel_bytecode.len(),
            ptr::null_mut(),
            &mut raw_pixel,
        ),
        &format!("CreatePixelShader(XMAP08 {})", program.effect_name),
    )?;
    let pixel_shader = D3dPtr::from_raw(raw_pixel, "XMAP08 pixel shader")?;
    let position = CString::new("POSITION")?;
    let texcoord = CString::new("TEXCOORD")?;
    let elements = [
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: position.as_ptr(),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 0,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: texcoord.as_ptr(),
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            InputSlot: 0,
            AlignedByteOffset: 16,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        },
    ];
    let mut raw_layout = ptr::null_mut::<ID3D11InputLayout>();
    native_d3d11_hr(
        (*device).CreateInputLayout(
            elements.as_ptr(),
            elements.len() as u32,
            program.vertex_bytecode.as_ptr() as *const _,
            program.vertex_bytecode.len(),
            &mut raw_layout,
        ),
        &format!("CreateInputLayout(XMAP08 {})", program.effect_name),
    )?;
    let input_layout = D3dPtr::from_raw(raw_layout, "XMAP08 input layout")?;
    let blend_state = create_blend_state(device, program)?;
    let depth_state = create_depth_state(device, program)?;
    let rasterizer_state = create_rasterizer_state(device, program)?;
    let mut samplers = BTreeMap::new();
    for (_, requirement) in reflected_resource_requirements(program)? {
        samplers.insert(
            requirement.name.clone(),
            create_sampler_state(device, requirement)?,
        );
    }
    Ok(NativeMapProgram {
        vertex_shader,
        pixel_shader,
        input_layout,
        blend_state,
        depth_state,
        rasterizer_state,
        samplers,
    })
}

#[cfg(windows)]
unsafe fn create_buffer(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    bind_flags: u32,
    label: &str,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11Buffer, D3D11_BUFFER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_DEFAULT,
    };
    if bytes.is_empty() {
        return Err(format!("{label} payload is empty").into());
    }
    let descriptor = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(bytes.len())?,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind_flags,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let initial = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr() as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut raw = ptr::null_mut::<ID3D11Buffer>();
    native_d3d11_hr(
        (*device).CreateBuffer(&descriptor, &initial, &mut raw),
        &format!("CreateBuffer({label})"),
    )?;
    D3dPtr::from_raw(raw, label)
}

#[cfg(windows)]
unsafe fn create_constant_buffer_from_bytes(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
    label: &str,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>> {
    if bytes.is_empty() || bytes.len() % 16 != 0 {
        return Err(format!("{label} must be nonempty and 16-byte aligned").into());
    }
    create_buffer(
        device,
        bytes,
        winapi::um::d3d11::D3D11_BIND_CONSTANT_BUFFER,
        label,
    )
}

#[cfg(windows)]
unsafe fn create_static_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    dds: &[u8],
    descriptor: &FoundationTextureDescriptor,
    content: &portable_content_manifest::UsedContentKey,
) -> Result<NativeStaticResource> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    if descriptor.dimension != "texture2d"
        || descriptor.depth != 1
        || descriptor.array_size != 1
        || descriptor.cube
    {
        return Err("XMAP08 static sampler requires one non-array Texture2D".into());
    }
    let (format, block_bytes, bytes_per_pixel, mut offset) =
        crate::production_d3d::foundation_native_format(&descriptor.format, dds)?;
    let mut initial = Vec::with_capacity(descriptor.mip_levels as usize);
    for mip in 0..descriptor.mip_levels {
        let width = (descriptor.width >> mip).max(1);
        let height = (descriptor.height >> mip).max(1);
        let (row_pitch, slice_pitch) = if block_bytes != 0 {
            let row = ((width + 3) / 4)
                .max(1)
                .checked_mul(block_bytes)
                .ok_or("XMAP08 DDS row pitch overflow")?;
            (
                row,
                row.checked_mul(((height + 3) / 4).max(1))
                    .ok_or("XMAP08 DDS slice pitch overflow")?,
            )
        } else {
            let row = width
                .checked_mul(bytes_per_pixel)
                .ok_or("XMAP08 DDS row pitch overflow")?;
            (
                row,
                row.checked_mul(height)
                    .ok_or("XMAP08 DDS slice pitch overflow")?,
            )
        };
        let end = offset
            .checked_add(slice_pitch as usize)
            .ok_or("XMAP08 DDS payload range overflow")?;
        let payload = dds
            .get(offset..end)
            .ok_or("XMAP08 DDS mip payload is truncated")?;
        initial.push(D3D11_SUBRESOURCE_DATA {
            pSysMem: payload.as_ptr() as *const _,
            SysMemPitch: row_pitch,
            SysMemSlicePitch: slice_pitch,
        });
        offset = end;
    }
    let native = D3D11_TEXTURE2D_DESC {
        Width: descriptor.width,
        Height: descriptor.height,
        MipLevels: descriptor.mip_levels,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut raw_texture = ptr::null_mut::<ID3D11Texture2D>();
    native_d3d11_hr(
        (*device).CreateTexture2D(&native, initial.as_ptr(), &mut raw_texture),
        "CreateTexture2D(XMAP08 admitted DDS)",
    )?;
    let texture = D3dPtr::from_raw(raw_texture, "XMAP08 admitted texture")?;
    let mut raw_view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut raw_view),
        "CreateShaderResourceView(XMAP08 admitted DDS)",
    )?;
    Ok(NativeStaticResource {
        _texture: texture,
        view: D3dPtr::from_raw(raw_view, "XMAP08 admitted SRV")?,
        resource_sha256: content.payload_sha256.clone(),
        descriptor_sha256: descriptor.shader_resource_view_identity_sha256.clone(),
    })
}

#[cfg(windows)]
unsafe fn create_blend_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
    program: &PreparedMapEffectProgram,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11BlendState, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
        D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_ZERO, D3D11_COLOR_WRITE_ENABLE_ALL,
        D3D11_COLOR_WRITE_ENABLE_BLUE, D3D11_COLOR_WRITE_ENABLE_GREEN,
        D3D11_COLOR_WRITE_ENABLE_RED,
    };
    let values = state_map(&program.pipeline.blend_state);
    require_state(&values, "BlendEnable", "yes", &program.effect_name)?;
    require_state_any(
        &values,
        "SourceBlend",
        &["src_alpha", "SRC_ALPHA"],
        &program.effect_name,
    )?;
    require_state_any(
        &values,
        "DestBlend",
        &["inv_src_alpha", "INV_SRC_ALPHA"],
        &program.effect_name,
    )?;
    let write_mask = match values.get("WriteMask").copied() {
        None => D3D11_COLOR_WRITE_ENABLE_ALL as u8,
        Some("RED|GREEN|BLUE") => {
            (D3D11_COLOR_WRITE_ENABLE_RED
                | D3D11_COLOR_WRITE_ENABLE_GREEN
                | D3D11_COLOR_WRITE_ENABLE_BLUE) as u8
        }
        Some(other) => {
            return Err(format!(
                "XMAP08 {} has unsupported WriteMask {other}",
                program.effect_name
            )
            .into())
        }
    };
    let mut descriptor: D3D11_BLEND_DESC = std::mem::zeroed();
    descriptor.AlphaToCoverageEnable = 0;
    descriptor.IndependentBlendEnable = 0;
    descriptor.RenderTarget[0].BlendEnable = 1;
    descriptor.RenderTarget[0].SrcBlend = D3D11_BLEND_SRC_ALPHA;
    descriptor.RenderTarget[0].DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    descriptor.RenderTarget[0].BlendOp = D3D11_BLEND_OP_ADD;
    descriptor.RenderTarget[0].SrcBlendAlpha = D3D11_BLEND_ONE;
    descriptor.RenderTarget[0].DestBlendAlpha = D3D11_BLEND_ZERO;
    descriptor.RenderTarget[0].BlendOpAlpha = D3D11_BLEND_OP_ADD;
    descriptor.RenderTarget[0].RenderTargetWriteMask = write_mask;
    let mut raw = ptr::null_mut::<ID3D11BlendState>();
    native_d3d11_hr(
        (*device).CreateBlendState(&descriptor, &mut raw),
        &format!("CreateBlendState(XMAP08 {})", program.effect_name),
    )?;
    D3dPtr::from_raw(raw, "XMAP08 blend state")
}

#[cfg(windows)]
unsafe fn create_depth_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
    program: &PreparedMapEffectProgram,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11DepthStencilState, D3D11_COMPARISON_LESS, D3D11_DEPTH_STENCIL_DESC,
        D3D11_DEPTH_WRITE_MASK_ALL,
    };
    let values = state_map(&program.pipeline.depth_stencil_state);
    require_state(&values, "DepthEnable", "yes", &program.effect_name)?;
    let descriptor = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: 1,
        DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
        DepthFunc: D3D11_COMPARISON_LESS,
        StencilEnable: 0,
        StencilReadMask: 0xff,
        StencilWriteMask: 0xff,
        FrontFace: std::mem::zeroed(),
        BackFace: std::mem::zeroed(),
    };
    let mut raw = ptr::null_mut::<ID3D11DepthStencilState>();
    native_d3d11_hr(
        (*device).CreateDepthStencilState(&descriptor, &mut raw),
        &format!("CreateDepthStencilState(XMAP08 {})", program.effect_name),
    )?;
    D3dPtr::from_raw(raw, "XMAP08 depth state")
}

#[cfg(windows)]
unsafe fn create_rasterizer_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
    program: &PreparedMapEffectProgram,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11RasterizerState, D3D11_CULL_BACK, D3D11_FILL_SOLID, D3D11_RASTERIZER_DESC,
    };
    let values = state_map(&program.pipeline.rasterizer_state);
    require_state(&values, "FillMode", "FILL_SOLID", &program.effect_name)?;
    require_state(&values, "CullMode", "CULL_BACK", &program.effect_name)?;
    require_state(&values, "FrontCCW", "no", &program.effect_name)?;
    let descriptor = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_BACK,
        FrontCounterClockwise: 0,
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: 1,
        ScissorEnable: 0,
        MultisampleEnable: 0,
        AntialiasedLineEnable: 0,
    };
    let mut raw = ptr::null_mut::<ID3D11RasterizerState>();
    native_d3d11_hr(
        (*device).CreateRasterizerState(&descriptor, &mut raw),
        &format!("CreateRasterizerState(XMAP08 {})", program.effect_name),
    )?;
    D3dPtr::from_raw(raw, "XMAP08 rasterizer state")
}

#[cfg(windows)]
unsafe fn create_sampler_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
    sampler: &PreparedMapSamplerRequirement,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11SamplerState, D3D11_COMPARISON_NEVER, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        D3D11_FILTER_MIN_MAG_MIP_POINT, D3D11_SAMPLER_DESC, D3D11_TEXTURE_ADDRESS_CLAMP,
        D3D11_TEXTURE_ADDRESS_WRAP,
    };
    let filter = match (
        sampler.mag_filter.as_str(),
        sampler.min_filter.as_str(),
        sampler.mip_filter.as_str(),
    ) {
        ("Linear", "Linear", "Linear") => D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        ("Point", "Point", "Point") => D3D11_FILTER_MIN_MAG_MIP_POINT,
        other => {
            return Err(format!(
                "XMAP08 sampler {} has unsupported filter {other:?}",
                sampler.name
            )
            .into())
        }
    };
    let address = |value: &str| -> Result<u32> {
        match value {
            "Wrap" => Ok(D3D11_TEXTURE_ADDRESS_WRAP),
            "Clamp" => Ok(D3D11_TEXTURE_ADDRESS_CLAMP),
            other => Err(format!("XMAP08 sampler address mode {other} is unsupported").into()),
        }
    };
    let descriptor = D3D11_SAMPLER_DESC {
        Filter: filter,
        AddressU: address(&sampler.address_u)?,
        AddressV: address(&sampler.address_v)?,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        BorderColor: [0.0; 4],
        MinLOD: 0.0,
        MaxLOD: f32::MAX,
    };
    let mut raw = ptr::null_mut::<ID3D11SamplerState>();
    native_d3d11_hr(
        (*device).CreateSamplerState(&descriptor, &mut raw),
        &format!("CreateSamplerState(XMAP08 {})", sampler.name),
    )?;
    D3dPtr::from_raw(raw, "XMAP08 sampler")
}

#[cfg(windows)]
unsafe fn bind_target_and_pipeline(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    target: *mut winapi::um::d3d11::ID3D11RenderTargetView,
    depth: *mut winapi::um::d3d11::ID3D11DepthStencilView,
    program: &NativeMapProgram,
    vertex: &crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    index: &crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    width: u32,
    height: u32,
) {
    use std::ptr;
    use winapi::shared::dxgiformat::DXGI_FORMAT_R16_UINT;
    use winapi::um::d3d11::D3D11_VIEWPORT;
    use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
    (*context).OMSetRenderTargets(1, &target, depth);
    let viewport = D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: width.max(1) as f32,
        Height: height.max(1) as f32,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    };
    (*context).RSSetViewports(1, &viewport);
    (*context).RSSetScissorRects(0, ptr::null());
    (*context).RSSetState(program.rasterizer_state.as_raw());
    let blend_factor = [0.0; 4];
    (*context).OMSetBlendState(program.blend_state.as_raw(), &blend_factor, u32::MAX);
    (*context).OMSetDepthStencilState(program.depth_state.as_raw(), 0);
    (*context).IASetInputLayout(program.input_layout.as_raw());
    (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    let vertex_raw = vertex.as_raw();
    let stride = 32u32;
    let offset = 0u32;
    (*context).IASetVertexBuffers(0, 1, &vertex_raw, &stride, &offset);
    (*context).IASetIndexBuffer(index.as_raw(), DXGI_FORMAT_R16_UINT, 0);
    (*context).VSSetShader(program.vertex_shader.as_raw(), ptr::null_mut(), 0);
    (*context).PSSetShader(program.pixel_shader.as_raw(), ptr::null_mut(), 0);
}

#[cfg(windows)]
unsafe fn bind_static_resources(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    program: &PreparedMapEffectProgram,
    resources: &BTreeMap<(String, String), NativeStaticResource>,
    native: &NativeMapProgram,
    logical: &mut Vec<MapDependentResourceBinding>,
) -> Result<()> {
    for (_, sampler) in reflected_resource_requirements(program)? {
        if sampler.owner_packet != "XMAP08" {
            continue;
        }
        let key = (program.effect_name.clone(), sampler.name.clone());
        let resource = resources
            .get(&key)
            .ok_or_else(|| format!("XMAP08 static SRV {} is absent", sampler.name))?;
        let view = resource.view.as_raw();
        (*context).PSSetShaderResources(sampler.shader_register, 1, &view);
        let state = native
            .samplers
            .get(&sampler.name)
            .ok_or_else(|| format!("XMAP08 sampler state {} is absent", sampler.name))?;
        let state = state.as_raw();
        (*context).PSSetSamplers(sampler.shader_register, 1, &state);
        logical.push(MapDependentResourceBinding {
            effect_name: program.effect_name.clone(),
            symbol: sampler.name.clone(),
            owner_packet: sampler.owner_packet.clone(),
            resource_sha256: resource.resource_sha256.clone(),
            descriptor_sha256: resource.descriptor_sha256.clone(),
        });
    }
    logical.sort_by_key(|binding| {
        program
            .samplers
            .iter()
            .find(|sampler| sampler.name == binding.symbol)
            .map(|sampler| sampler.shader_register)
            .unwrap_or(u32::MAX)
    });
    Ok(())
}

#[cfg(windows)]
fn validate_downstream_resource_bindings(
    program: &PreparedMapEffectProgram,
    resources: &[NativeMapShaderResourceBinding<'_>],
) -> Result<()> {
    let expected = reflected_resource_requirements(program)?
        .into_iter()
        .filter(|(_, sampler)| sampler.owner_packet != "XMAP08")
        .collect::<Vec<_>>();
    let supplied_keys = resources
        .iter()
        .map(|binding| {
            (
                binding.effect_name.as_str(),
                binding.symbol.as_str(),
                binding.owner_packet.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    if resources.len() != expected.len() || supplied_keys.len() != resources.len() {
        return Err(
            "XMAP08 downstream resource set is incomplete, duplicated, or contains extras".into(),
        );
    }
    for (_, sampler) in expected {
        let binding = resources
            .iter()
            .find(|binding| {
                binding.effect_name == program.effect_name
                    && binding.symbol == sampler.name
                    && binding.owner_packet == sampler.owner_packet
            })
            .ok_or_else(|| {
                format!(
                    "XMAP08 river draw lacks {} provider resource {}",
                    sampler.owner_packet, sampler.name
                )
            })?;
        if binding.shader_resource_view.is_null()
            || binding.shader_register != sampler.shader_register
            || binding.resource_type != sampler.resource_type
        {
            return Err(format!(
                "XMAP08 downstream resource {} is null or has the wrong reflected binding",
                sampler.name
            )
            .into());
        }
        validate_canonical_sha256(&binding.resource_sha256, "XMAP08 downstream resource")?;
        validate_canonical_sha256(
            &binding.descriptor_sha256,
            "XMAP08 downstream resource descriptor",
        )?;
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn bind_downstream_resources(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    program: &PreparedMapEffectProgram,
    resources: &[NativeMapShaderResourceBinding<'_>],
    native: &NativeMapProgram,
    logical: &mut Vec<MapDependentResourceBinding>,
) -> Result<()> {
    validate_downstream_resource_bindings(program, resources)?;
    let expected_count = reflected_resource_requirements(program)?
        .into_iter()
        .filter(|(_, sampler)| sampler.owner_packet != "XMAP08")
        .count();
    let mut consumed = BTreeSet::new();
    for (_, sampler) in reflected_resource_requirements(program)? {
        if sampler.owner_packet == "XMAP08" {
            continue;
        }
        let binding = resources
            .iter()
            .find(|binding| {
                binding.effect_name == program.effect_name
                    && binding.symbol == sampler.name
                    && binding.owner_packet == sampler.owner_packet
            })
            .ok_or_else(|| {
                format!(
                    "XMAP08 river draw lacks {} provider resource {}",
                    sampler.owner_packet, sampler.name
                )
            })?;
        if binding.shader_resource_view.is_null()
            || binding.shader_register != sampler.shader_register
            || binding.resource_type != sampler.resource_type
        {
            return Err(format!(
                "XMAP08 downstream resource {} is null or unauthenticated",
                sampler.name
            )
            .into());
        }
        validate_canonical_sha256(&binding.resource_sha256, "XMAP08 downstream resource")?;
        validate_canonical_sha256(
            &binding.descriptor_sha256,
            "XMAP08 downstream resource descriptor",
        )?;
        if !consumed.insert((binding.effect_name.clone(), binding.symbol.clone())) {
            return Err("XMAP08 downstream resource is duplicated".into());
        }
        (*context).PSSetShaderResources(sampler.shader_register, 1, &binding.shader_resource_view);
        let state = native
            .samplers
            .get(&sampler.name)
            .ok_or_else(|| format!("XMAP08 sampler state {} is absent", sampler.name))?;
        let state = state.as_raw();
        (*context).PSSetSamplers(sampler.shader_register, 1, &state);
        logical.push(MapDependentResourceBinding {
            effect_name: binding.effect_name.clone(),
            symbol: binding.symbol.clone(),
            owner_packet: binding.owner_packet.clone(),
            resource_sha256: binding.resource_sha256.clone(),
            descriptor_sha256: binding.descriptor_sha256.clone(),
        });
    }
    logical.sort_by_key(|binding| {
        program
            .samplers
            .iter()
            .find(|sampler| sampler.name == binding.symbol)
            .map(|sampler| sampler.shader_register)
            .unwrap_or(u32::MAX)
    });
    if consumed.len() != expected_count {
        return Err("XMAP08 did not consume the exact downstream resource set".into());
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn validate_scene_cb0_provider_set<'a>(
    program: &PreparedMapEffectProgram,
    frame: &PreparedRiversStraitsFrame,
    authority: &Xmap08ExecutionFrameAuthority,
    bindings: &'a [&'a dyn NativeXmap08SceneConstantBuffer],
    target_width: u32,
    target_height: u32,
    target_color_texture: *mut winapi::um::d3d11::ID3D11Texture2D,
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<Vec<&'a dyn NativeXmap08SceneConstantBuffer>> {
    let required_size = reflected_cbuffer_size(program, 0)?;
    let required_layout_sha256 = crate::xmap10_scene_cb0_reflected_layout_sha256(
        &program.vertex_bytecode,
        &program.pixel_bytecode,
    )?;
    validate_scene_cb0_provider_identities(
        frame.map_width,
        &frame.camera_view_sha256,
        &authority.replay_frame_identity_sha256,
        &authority.map_copy_receipt_set_sha256,
        required_size,
        &required_layout_sha256,
        target_width,
        target_height,
        &authority.main_scene_resource_identity_sha256,
        bindings,
    )?;
    for binding in bindings {
        if binding.raw_main_scene_color_texture() != target_color_texture {
            return Err(
                "XMAP08 XMAP10 SceneCB0 is bound to a different MainScene allocation".into(),
            );
        }
        validate_buffer_device(binding.raw_buffer(), device, "SceneCB0")?;
    }
    Ok(bindings.to_vec())
}

#[cfg(windows)]
fn validate_scene_cb0_provider_identities(
    map_width: u32,
    camera_frame_sha256: &str,
    replay_frame_identity_sha256: &str,
    map_copy_receipt_set_sha256: &str,
    required_size: u32,
    required_layout_sha256: &str,
    target_width: u32,
    target_height: u32,
    main_scene_resource_identity_sha256: &str,
    bindings: &[&dyn NativeXmap08SceneConstantBuffer],
) -> Result<()> {
    let rows = bindings
        .iter()
        .map(|binding| crate::Xmap04MapCopyReceiptRow {
            render_view_mask: binding.render_view_mask(),
            map_copy_offset_f32_bits: binding.map_copy_offset_f32_bits(),
        })
        .collect::<Vec<_>>();
    if bindings.is_empty()
        || crate::xmap04_map_copy_receipt_set_sha256(map_width, &rows)?
            != map_copy_receipt_set_sha256
    {
        return Err(
            "XMAP08 XMAP10 SceneCB0 rows do not equal the canonical XMAP04 map-copy set".into(),
        );
    }
    let mut unique = BTreeSet::new();
    for binding in bindings {
        if binding.owner_packet() != "XMAP10"
            || binding.shader_register() != 0
            || binding.byte_len() != required_size
            || binding.reflected_layout_sha256() != required_layout_sha256
            || binding.viewport_width() != target_width
            || binding.viewport_height() != target_height
            || binding.frame_identity_sha256() != camera_frame_sha256
            || binding.replay_frame_identity_sha256() != replay_frame_identity_sha256
            || binding.map_copy_receipt_set_sha256() != map_copy_receipt_set_sha256
            || binding.main_scene_resource_identity_sha256() != main_scene_resource_identity_sha256
            || !unique.insert((
                binding.render_view_mask(),
                binding.map_copy_offset_f32_bits(),
            ))
        {
            return Err(
                "XMAP08 XMAP10 SceneCB0 has the wrong owner/layout/frame/map-copy identity".into(),
            );
        }
        validate_canonical_sha256(binding.payload_sha256(), "XMAP08 SceneCB0 payload")?;
        validate_canonical_sha256(binding.descriptor_sha256(), "XMAP08 SceneCB0 descriptor")?;
        validate_canonical_sha256(
            binding.composition_receipt_sha256(),
            "XMAP08 SceneCB0 composition receipt",
        )?;
        validate_canonical_sha256(
            binding.reflected_layout_sha256(),
            "XMAP08 SceneCB0 reflected layout",
        )?;
        validate_canonical_sha256(
            binding.main_scene_resource_identity_sha256(),
            "XMAP08 SceneCB0 MainScene resource",
        )?;
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn validate_point_lights_cb2_provider<'a>(
    program: &PreparedMapEffectProgram,
    frame: &PreparedRiversStraitsFrame,
    authority: &Xmap08ExecutionFrameAuthority,
    binding: Option<&'a dyn crate::NativeXmap04UnderwaterPointLightsConstantBuffer>,
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<Option<&'a dyn crate::NativeXmap04UnderwaterPointLightsConstantBuffer>> {
    let retained = reflected_constant_buffer_requirements(program)?
        .into_iter()
        .find(|(_, slot, _)| *slot == 2);
    match (retained, binding) {
        (None, None) => Ok(None),
        (None, Some(_)) => {
            Err("XMAP08 low-GFX river program received an unexpected XMAP09 CB2".into())
        }
        (Some((_, _, owner)), Some(binding))
            if owner == "XMAP09"
                && binding.owner_packet() == "XMAP09"
                && binding.shader_register() == 2
                && binding.byte_len() == reflected_cbuffer_size(program, 2)?
                && binding.frame_identity_sha256() == frame.camera_view_sha256
                && binding.replay_frame_identity_sha256()
                    == authority.replay_frame_identity_sha256 =>
        {
            validate_canonical_sha256(binding.payload_sha256(), "XMAP08 PointLightsCB2 payload")?;
            validate_canonical_sha256(
                binding.descriptor_sha256(),
                "XMAP08 PointLightsCB2 descriptor",
            )?;
            validate_buffer_device(binding.raw_buffer(), device, "PointLightsCB2")?;
            Ok(Some(binding))
        }
        (Some(_), _) => Err(
            "XMAP08 high-GFX river program lacks the exact XMAP09 PointLightsCB2 provider".into(),
        ),
    }
}

fn compose_river_cb1(
    program: &PreparedMapEffectProgram,
    shadow: Option<&Xmap08RiverShadowMatrixBinding>,
    temporal: &Xmap08RiverTemporalBinding,
    camera_frame_sha256: &str,
    replay_frame_identity_sha256: &str,
    render_group: u8,
) -> Result<Vec<u8>> {
    validate_river_cb1_owned_variables(program)?;
    let authenticated_input_bytes = [
        temporal.animation_time_f32_bits,
        temporal.season_lerp_f32_bits,
    ]
    .iter()
    .flat_map(|bits| bits.to_le_bytes())
    .collect::<Vec<_>>();
    if temporal.payload_sha256 != Sha256::hex(&authenticated_input_bytes)
        || temporal.camera_frame_sha256 != camera_frame_sha256
        || temporal.replay_frame_identity_sha256 != replay_frame_identity_sha256
        || validate_canonical_sha256(
            &temporal.camera_frame_sha256,
            "XMAP08 temporal camera frame",
        )
        .is_err()
        || validate_canonical_sha256(
            &temporal.replay_frame_identity_sha256,
            "XMAP08 temporal replay frame",
        )
        .is_err()
        || [
            temporal.animation_time_f32_bits,
            temporal.season_lerp_f32_bits,
        ]
        .iter()
        .any(|bits| !f32::from_bits(*bits).is_finite())
    {
        return Err("XMAP08 composite river CB1 inputs are unauthenticated or non-finite".into());
    }
    let temporal_bytes = [
        temporal.animation_time_f32_bits,
        river_flow_direction_f32_bits(render_group),
        temporal.season_lerp_f32_bits,
    ]
    .iter()
    .flat_map(|bits| bits.to_le_bytes())
    .collect::<Vec<_>>();
    let variables = reflected_cbuffer_variables(program, 1)?;
    let byte_len = reflected_cbuffer_size(program, 1)? as usize;
    let mut bytes = vec![0u8; byte_len.next_multiple_of(16)];
    let (temporal_offset, temporal_size) = variables
        .get("vTimeDirectionSeasonLerp")
        .copied()
        .ok_or("XMAP08 river CB1 omits vTimeDirectionSeasonLerp")?;
    if temporal_size < 12 || temporal_offset + 12 > bytes.len() {
        return Err("XMAP08 river CB1 reflection layout is invalid".into());
    }
    if reflected_cbuffer_variable_is_used(program, 1, "ShadowMapTextureMatrix")? {
        let shadow = shadow.ok_or(
            "XMAP08 effective river program uses ShadowMapTextureMatrix without XMAP09 input",
        )?;
        let shadow_bytes = shadow
            .shadow_map_texture_matrix_f32_bits
            .iter()
            .flatten()
            .flat_map(|bits| bits.to_le_bytes())
            .collect::<Vec<_>>();
        if shadow.payload_sha256 != Sha256::hex(&shadow_bytes)
            || shadow.camera_frame_sha256 != camera_frame_sha256
            || shadow.replay_frame_identity_sha256 != replay_frame_identity_sha256
            || shadow
                .shadow_map_texture_matrix_f32_bits
                .iter()
                .flatten()
                .any(|bits| !f32::from_bits(*bits).is_finite())
        {
            return Err("XMAP08 river shadow matrix is unauthenticated or non-finite".into());
        }
        let (matrix_offset, matrix_size) = variables
            .get("ShadowMapTextureMatrix")
            .copied()
            .ok_or("XMAP08 river CB1 omits used ShadowMapTextureMatrix")?;
        if matrix_size != 64 || matrix_offset + 64 > bytes.len() {
            return Err("XMAP08 river CB1 shadow matrix reflection layout is invalid".into());
        }
        bytes[matrix_offset..matrix_offset + 64].copy_from_slice(&shadow_bytes);
    }
    bytes[temporal_offset..temporal_offset + 12].copy_from_slice(&temporal_bytes);
    Ok(bytes)
}

fn validate_river_cb1_owned_variables(program: &PreparedMapEffectProgram) -> Result<()> {
    let permitted = ["vTimeDirectionSeasonLerp", "ShadowMapTextureMatrix"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    for reflection in [
        crate::parse_dxbc_reflection(&program.vertex_bytecode)?,
        crate::parse_dxbc_reflection(&program.pixel_bytecode)?,
    ] {
        let names = reflection
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 0 && resource.bind_point == 1)
            .map(|resource| resource.name.as_str())
            .collect::<BTreeSet<_>>();
        for variable in reflection
            .cbuffers
            .iter()
            .filter(|buffer| names.contains(buffer.name.as_str()))
            .flat_map(|buffer| &buffer.vars)
            .filter(|variable| variable.used)
        {
            if !permitted.contains(variable.name.as_str()) {
                return Err(format!(
                    "XMAP08 effective river CB1 retains unsupported variable {}",
                    variable.name
                )
                .into());
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
unsafe fn unbind_resources(
    context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    program: &PreparedMapEffectProgram,
) -> Result<()> {
    use std::ptr;
    for (_, sampler) in reflected_resource_requirements(program)? {
        let null_view = ptr::null_mut();
        (*context).PSSetShaderResources(sampler.shader_register, 1, &null_view);
    }
    Ok(())
}

fn validate_strait_cb0_used_fields(program: &PreparedMapEffectProgram) -> Result<()> {
    let permitted = ["ViewProjectionMatrix", "vCamPos", "vCamLookAtDir"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    for reflection in [
        crate::parse_dxbc_reflection(&program.vertex_bytecode)?,
        crate::parse_dxbc_reflection(&program.pixel_bytecode)?,
    ] {
        for buffer in reflection.cbuffers {
            for variable in buffer.vars.into_iter().filter(|variable| variable.used) {
                if !permitted.contains(variable.name.as_str()) {
                    return Err(format!(
                        "XMAP08 strait program retained non-camera CB0 field {}",
                        variable.name
                    )
                    .into());
                }
            }
        }
    }
    Ok(())
}

fn strait_camera_cb0_bytes(
    program: &PreparedMapEffectProgram,
    camera: crate::ProductionMapCamera,
    viewer: &ViewerMapCameraState,
) -> Result<Vec<u8>> {
    validate_strait_cb0_used_fields(program)?;
    let byte_len = reflected_cbuffer_size(program, 0)?.max(240) as usize;
    let mut bytes = vec![0u8; byte_len.next_multiple_of(16)];
    patch_camera_fields(program, &mut bytes, camera, viewer)?;
    Ok(bytes)
}

fn patch_camera_fields(
    program: &PreparedMapEffectProgram,
    bytes: &mut [u8],
    camera: crate::ProductionMapCamera,
    viewer: &ViewerMapCameraState,
) -> Result<()> {
    let variables = reflected_cbuffer_variables(program, 0)?;
    write_reflected_matrix(
        bytes,
        &variables,
        "ViewProjectionMatrix",
        camera.view_projection_matrix,
    )?;
    write_reflected_vec3(bytes, &variables, "vCamPos", camera.position)?;
    let eye = viewer.current_eye();
    let look = viewer.current_look_at();
    let direction = normalize3([look[0] - eye[0], look[1] - eye[1], look[2] - eye[2]])?;
    write_reflected_vec3(bytes, &variables, "vCamLookAtDir", direction)
}

fn reflected_cbuffer_variables(
    program: &PreparedMapEffectProgram,
    slot: u32,
) -> Result<BTreeMap<String, (usize, usize)>> {
    let mut out = BTreeMap::new();
    for reflection in [
        crate::parse_dxbc_reflection(&program.vertex_bytecode)?,
        crate::parse_dxbc_reflection(&program.pixel_bytecode)?,
    ] {
        let names = reflection
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 0 && resource.bind_point == slot)
            .map(|resource| resource.name.as_str())
            .collect::<BTreeSet<_>>();
        for buffer in reflection
            .cbuffers
            .iter()
            .filter(|buffer| names.contains(buffer.name.as_str()))
        {
            for variable in &buffer.vars {
                let value = (variable.offset as usize, variable.size as usize);
                if out
                    .insert(variable.name.clone(), value)
                    .is_some_and(|previous| previous != value)
                {
                    return Err(format!(
                        "XMAP08 CB{slot} variable {} has stage-inconsistent layout",
                        variable.name
                    )
                    .into());
                }
            }
        }
    }
    Ok(out)
}

fn reflected_cbuffer_size(program: &PreparedMapEffectProgram, slot: u32) -> Result<u32> {
    let mut sizes = BTreeSet::new();
    for reflection in [
        crate::parse_dxbc_reflection(&program.vertex_bytecode)?,
        crate::parse_dxbc_reflection(&program.pixel_bytecode)?,
    ] {
        let names = reflection
            .resources
            .iter()
            .filter(|resource| resource.resource_type == 0 && resource.bind_point == slot)
            .map(|resource| resource.name.as_str())
            .collect::<BTreeSet<_>>();
        for buffer in reflection
            .cbuffers
            .iter()
            .filter(|buffer| names.contains(buffer.name.as_str()))
        {
            sizes.insert(buffer.size);
        }
    }
    sizes
        .into_iter()
        .max()
        .ok_or_else(|| format!("XMAP08 program has no reflected CB{slot}").into())
}

fn write_reflected_matrix(
    bytes: &mut [u8],
    variables: &BTreeMap<String, (usize, usize)>,
    name: &str,
    value: [[f32; 4]; 4],
) -> Result<()> {
    let (offset, size) = variables
        .get(name)
        .copied()
        .ok_or_else(|| format!("XMAP08 reflected CB0 omits {name}"))?;
    if size != 64 || offset + size > bytes.len() {
        return Err(format!("XMAP08 reflected {name} has invalid matrix extent").into());
    }
    for (row_index, row) in value.iter().enumerate() {
        for (column_index, component) in row.iter().enumerate() {
            let start = offset + row_index * 16 + column_index * 4;
            bytes[start..start + 4].copy_from_slice(&component.to_le_bytes());
        }
    }
    Ok(())
}

fn write_reflected_vec3(
    bytes: &mut [u8],
    variables: &BTreeMap<String, (usize, usize)>,
    name: &str,
    value: [f32; 3],
) -> Result<()> {
    let (offset, size) = variables
        .get(name)
        .copied()
        .ok_or_else(|| format!("XMAP08 reflected CB0 omits {name}"))?;
    if size < 12 || offset + 12 > bytes.len() {
        return Err(format!("XMAP08 reflected {name} has invalid vector extent").into());
    }
    for (index, component) in value.into_iter().enumerate() {
        bytes[offset + index * 4..offset + index * 4 + 4].copy_from_slice(&component.to_le_bytes());
    }
    Ok(())
}

fn normalize3(value: [f32; 3]) -> Result<[f32; 3]> {
    let length = (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt();
    if !length.is_finite() || length <= f32::EPSILON {
        return Err("XMAP08 camera look direction is degenerate".into());
    }
    Ok([value[0] / length, value[1] / length, value[2] / length])
}

fn camera_for_map_copy(
    camera: &ViewerMapCameraState,
    map_copy_offset: f32,
) -> Result<crate::ProductionMapCamera> {
    let mut eye = camera.current_eye();
    let mut look = camera.current_look_at();
    eye[0] -= map_copy_offset;
    look[0] -= map_copy_offset;
    ProductionMapCameraProjection::from_engine_constants(
        camera.viewport_width as f32 / camera.viewport_height as f32,
    )?
    .camera_from_eye_look_at(eye, look)
}

#[cfg(windows)]
fn constant_buffer_descriptor_sha256(
    program: &PreparedMapEffectProgram,
    slot: u32,
    byte_len: usize,
) -> Result<String> {
    let variables = reflected_cbuffer_variables(program, slot)?;
    let byte_len = u32::try_from(byte_len)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"hoi4.xmap08.d3d11-constant-buffer-desc.v1\0");
    bytes.extend_from_slice(program.effect_name.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&slot.to_le_bytes());
    bytes.extend_from_slice(&byte_len.to_le_bytes());
    bytes.extend_from_slice(&winapi::um::d3d11::D3D11_USAGE_DEFAULT.to_le_bytes());
    bytes.extend_from_slice(&winapi::um::d3d11::D3D11_BIND_CONSTANT_BUFFER.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // CPUAccessFlags
    bytes.extend_from_slice(&0u32.to_le_bytes()); // MiscFlags
    bytes.extend_from_slice(&0u32.to_le_bytes()); // StructureByteStride
    bytes.extend_from_slice(format!("{variables:?}").as_bytes());
    Ok(Sha256::hex(&bytes))
}

fn state_map(values: &[(String, String)]) -> BTreeMap<&str, &str> {
    values
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect()
}

fn require_state(
    values: &BTreeMap<&str, &str>,
    key: &str,
    expected: &str,
    effect: &str,
) -> Result<()> {
    if values.get(key).copied() != Some(expected) {
        return Err(format!(
            "XMAP08 {effect} state {key} must be {expected}, got {:?}",
            values.get(key)
        )
        .into());
    }
    Ok(())
}

fn require_state_any(
    values: &BTreeMap<&str, &str>,
    key: &str,
    expected: &[&str],
    effect: &str,
) -> Result<()> {
    if !values
        .get(key)
        .is_some_and(|value| expected.contains(value))
    {
        return Err(format!(
            "XMAP08 {effect} state {key} must be one of {expected:?}, got {:?}",
            values.get(key)
        )
        .into());
    }
    Ok(())
}

fn initial_receipt(
    frame: &PreparedRiversStraitsFrame,
    index_buffers: usize,
    static_resources: usize,
) -> Result<Xmap08D3dHandoffReceipt> {
    let index_buffers =
        u32::try_from(index_buffers).map_err(|_| "XMAP08 index-buffer count exceeds u32")?;
    let static_resources =
        u32::try_from(static_resources).map_err(|_| "XMAP08 static-resource count exceeds u32")?;
    let retained_river_draws = u32::try_from(frame.river_draws.len())
        .map_err(|_| "XMAP08 retained river draw count exceeds u32")?;
    let retained_strait_draws = u32::try_from(frame.strait_draws.len())
        .map_err(|_| "XMAP08 retained strait draw count exceeds u32")?;
    let mut receipt = Xmap08D3dHandoffReceipt {
        frame_sha256: frame.deterministic_sha256.clone(),
        uploaded_programs: 2,
        uploaded_vertex_buffers: u32::from(!frame.river_draws.is_empty())
            + u32::from(!frame.strait_vertices.is_empty()),
        uploaded_index_buffers: index_buffers
            .checked_add(u32::from(!frame.strait_indices.is_empty()))
            .ok_or("XMAP08 uploaded index-buffer count overflow")?,
        uploaded_static_resources: static_resources,
        retained_river_draws,
        retained_strait_draws,
        executed_river_draws: 0,
        executed_strait_draws: 0,
        river_ready_contract_sha256: None,
        strait_ready_contract_sha256: None,
        river_draw_receipts: Vec::new(),
        strait_draw_receipts: Vec::new(),
        river_component_closure: None,
        strait_component_closure: None,
        deterministic_sha256: String::new(),
    };
    reseal_receipt(&mut receipt)?;
    Ok(receipt)
}

fn reseal_receipt(receipt: &mut Xmap08D3dHandoffReceipt) -> Result<()> {
    receipt.deterministic_sha256 = receipt_sha256(receipt)?;
    Ok(())
}

fn receipt_sha256(receipt: &Xmap08D3dHandoffReceipt) -> Result<String> {
    let mut unsigned = receipt.clone();
    unsigned.deterministic_sha256.clear();
    Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_component_closure(
        component_id: &str,
        stage_name: &str,
        operation_id: &str,
        assigned_ordinal: u32,
    ) -> Xmap08ComponentClosure {
        let assignment =
            AssignedStage::seal("8".repeat(64), stage_name, operation_id, assigned_ordinal)
                .unwrap();
        Xmap08ComponentClosure {
            schema: XMAP08_COMPONENT_CLOSURE_SCHEMA.into(),
            component_id: component_id.into(),
            stage_name: stage_name.into(),
            operation_id: operation_id.into(),
            assigned_ordinal,
            topology_sha256: assignment.topology_sha256,
            assignment_sha256: assignment.assignment_sha256,
            hour_index: 7,
            execution_authority_sha256: "9".repeat(64),
            replay_frame_identity_sha256: "0".repeat(64),
            content_manifest_sha256: "1".repeat(64),
            foundation_content_operation_sha256: "2".repeat(64),
            viewer_state_sha256: "3".repeat(64),
            main_scene_target_sha256: "4".repeat(64),
            main_scene_resource_identity_sha256: "6".repeat(64),
            map_copy_receipt_set_sha256: "5".repeat(64),
            prepared_frame_sha256: "a".repeat(64),
            program_sha256: "6".repeat(64),
            disposition: CompositionComponentDisposition::AuthenticatedEmpty {
                enumerator_witness_sha256: "7".repeat(64),
            },
            closure_sha256: String::new(),
        }
        .seal()
        .unwrap()
    }

    #[test]
    fn receipt_seals_river_and_strait_ready_contracts_independently() {
        let mut receipt = Xmap08D3dHandoffReceipt {
            frame_sha256: "a".repeat(64),
            uploaded_programs: 2,
            uploaded_vertex_buffers: 2,
            uploaded_index_buffers: 2,
            uploaded_static_resources: 5,
            retained_river_draws: 1,
            retained_strait_draws: 1,
            executed_river_draws: 1,
            executed_strait_draws: 1,
            river_ready_contract_sha256: Some("b".repeat(64)),
            strait_ready_contract_sha256: Some("c".repeat(64)),
            river_draw_receipts: Vec::new(),
            strait_draw_receipts: Vec::new(),
            river_component_closure: Some(empty_component_closure(
                XMAP08_RIVER_COMPONENT_ID,
                XMAP08_RIVER_STAGE_NAME,
                XMAP08_RIVER_OPERATION_ID,
                8,
            )),
            strait_component_closure: Some(empty_component_closure(
                XMAP08_STRAIT_COMPONENT_ID,
                XMAP08_STRAIT_STAGE_NAME,
                XMAP08_STRAIT_OPERATION_ID,
                9,
            )),
            deterministic_sha256: String::new(),
        };
        reseal_receipt(&mut receipt).unwrap();
        let both = receipt.deterministic_sha256.clone();
        receipt.river_ready_contract_sha256 = None;
        reseal_receipt(&mut receipt).unwrap();
        let strait_only = receipt.deterministic_sha256.clone();
        receipt.river_ready_contract_sha256 = Some("b".repeat(64));
        receipt.strait_ready_contract_sha256 = None;
        reseal_receipt(&mut receipt).unwrap();
        let river_only = receipt.deterministic_sha256.clone();
        assert_ne!(both, strait_only);
        assert_ne!(both, river_only);
        assert_ne!(strait_only, river_only);
    }

    #[test]
    fn completion_receipt_accepts_exact_empty_passes_and_rejects_partial_draws() {
        let frame_sha256 = "a".repeat(64);
        let mut empty = Xmap08D3dHandoffReceipt {
            frame_sha256: frame_sha256.clone(),
            uploaded_programs: 2,
            uploaded_vertex_buffers: 0,
            uploaded_index_buffers: 0,
            uploaded_static_resources: 5,
            retained_river_draws: 0,
            retained_strait_draws: 0,
            executed_river_draws: 0,
            executed_strait_draws: 0,
            river_ready_contract_sha256: None,
            strait_ready_contract_sha256: None,
            river_draw_receipts: Vec::new(),
            strait_draw_receipts: Vec::new(),
            river_component_closure: Some(empty_component_closure(
                XMAP08_RIVER_COMPONENT_ID,
                XMAP08_RIVER_STAGE_NAME,
                XMAP08_RIVER_OPERATION_ID,
                8,
            )),
            strait_component_closure: Some(empty_component_closure(
                XMAP08_STRAIT_COMPONENT_ID,
                XMAP08_STRAIT_STAGE_NAME,
                XMAP08_STRAIT_OPERATION_ID,
                9,
            )),
            deterministic_sha256: String::new(),
        };
        reseal_receipt(&mut empty).unwrap();
        empty
            .validate_complete_structure_for_frame(&frame_sha256)
            .unwrap();

        let mut partial = empty.clone();
        partial.retained_river_draws = 2;
        partial.executed_river_draws = 1;
        partial.river_ready_contract_sha256 = Some("b".repeat(64));
        reseal_receipt(&mut partial).unwrap();
        assert!(partial
            .validate_complete_structure_for_frame(&frame_sha256)
            .is_err());

        let mut non_empty_without_contract = empty;
        non_empty_without_contract.retained_strait_draws = 1;
        non_empty_without_contract.executed_strait_draws = 1;
        reseal_receipt(&mut non_empty_without_contract).unwrap();
        assert!(non_empty_without_contract
            .validate_complete_structure_for_frame(&frame_sha256)
            .is_err());
    }

    #[test]
    fn completion_receipt_is_bound_to_the_exact_execution_authority() {
        let topology = "8".repeat(64);
        let mut authority = Xmap08ExecutionFrameAuthority {
            hour_index: 7,
            replay_frame_identity_sha256: "0".repeat(64),
            political_frame_sha256: "a".repeat(64),
            content_manifest_sha256: "1".repeat(64),
            foundation_content_operation_sha256: "2".repeat(64),
            viewer_state_sha256: "3".repeat(64),
            main_scene_target_sha256: "4".repeat(64),
            main_scene_resource_identity_sha256: "6".repeat(64),
            map_copy_receipt_set_sha256: "5".repeat(64),
            river_assignment: AssignedStage::seal(
                &topology,
                XMAP08_RIVER_STAGE_NAME,
                XMAP08_RIVER_OPERATION_ID,
                8,
            )
            .unwrap(),
            strait_assignment: AssignedStage::seal(
                &topology,
                XMAP08_STRAIT_STAGE_NAME,
                XMAP08_STRAIT_OPERATION_ID,
                9,
            )
            .unwrap(),
            deterministic_sha256: String::new(),
        };
        authority.deterministic_sha256 = execution_frame_authority_sha256(&authority).unwrap();
        authority.validate().unwrap();

        let mut river = empty_component_closure(
            XMAP08_RIVER_COMPONENT_ID,
            XMAP08_RIVER_STAGE_NAME,
            XMAP08_RIVER_OPERATION_ID,
            8,
        );
        river.execution_authority_sha256 = authority.deterministic_sha256.clone();
        river = river.seal().unwrap();
        let mut strait = empty_component_closure(
            XMAP08_STRAIT_COMPONENT_ID,
            XMAP08_STRAIT_STAGE_NAME,
            XMAP08_STRAIT_OPERATION_ID,
            9,
        );
        strait.execution_authority_sha256 = authority.deterministic_sha256.clone();
        strait = strait.seal().unwrap();
        let frame_sha256 = "a".repeat(64);
        let mut receipt = Xmap08D3dHandoffReceipt {
            frame_sha256: frame_sha256.clone(),
            uploaded_programs: 2,
            uploaded_vertex_buffers: 0,
            uploaded_index_buffers: 0,
            uploaded_static_resources: 5,
            retained_river_draws: 0,
            retained_strait_draws: 0,
            executed_river_draws: 0,
            executed_strait_draws: 0,
            river_ready_contract_sha256: None,
            strait_ready_contract_sha256: None,
            river_draw_receipts: Vec::new(),
            strait_draw_receipts: Vec::new(),
            river_component_closure: Some(river),
            strait_component_closure: Some(strait),
            deterministic_sha256: String::new(),
        };
        reseal_receipt(&mut receipt).unwrap();
        receipt
            .validate_complete_for_authority(&frame_sha256, &authority)
            .unwrap();

        authority.political_frame_sha256 = "b".repeat(64);
        authority.deterministic_sha256 = execution_frame_authority_sha256(&authority).unwrap();
        authority.validate().unwrap();
        assert!(receipt
            .validate_complete_for_authority(&frame_sha256, &authority)
            .is_err());
    }

    #[test]
    fn component_closure_rejects_forged_assignment_and_cross_hour_provider_mix() {
        let mut forged = empty_component_closure(
            XMAP08_RIVER_COMPONENT_ID,
            XMAP08_RIVER_STAGE_NAME,
            XMAP08_RIVER_OPERATION_ID,
            8,
        );
        forged.assignment_sha256 = "f".repeat(64);
        forged.closure_sha256 = component_closure_sha256(&forged).unwrap();
        assert!(forged.validate().is_err());

        let mut wrong_stage = empty_component_closure(
            XMAP08_RIVER_COMPONENT_ID,
            XMAP08_RIVER_STAGE_NAME,
            XMAP08_RIVER_OPERATION_ID,
            8,
        );
        wrong_stage.stage_name = XMAP08_STRAIT_STAGE_NAME.into();
        wrong_stage.operation_id = XMAP08_STRAIT_OPERATION_ID.into();
        assert!(wrong_stage.seal().is_err());

        let frame_sha256 = "a".repeat(64);
        let river = empty_component_closure(
            XMAP08_RIVER_COMPONENT_ID,
            XMAP08_RIVER_STAGE_NAME,
            XMAP08_RIVER_OPERATION_ID,
            8,
        );
        let mut strait = empty_component_closure(
            XMAP08_STRAIT_COMPONENT_ID,
            XMAP08_STRAIT_STAGE_NAME,
            XMAP08_STRAIT_OPERATION_ID,
            9,
        );
        strait.replay_frame_identity_sha256 = "e".repeat(64);
        strait = strait.seal().unwrap();
        let mut receipt = Xmap08D3dHandoffReceipt {
            frame_sha256: frame_sha256.clone(),
            uploaded_programs: 2,
            uploaded_vertex_buffers: 0,
            uploaded_index_buffers: 0,
            uploaded_static_resources: 5,
            retained_river_draws: 0,
            retained_strait_draws: 0,
            executed_river_draws: 0,
            executed_strait_draws: 0,
            river_ready_contract_sha256: None,
            strait_ready_contract_sha256: None,
            river_draw_receipts: Vec::new(),
            strait_draw_receipts: Vec::new(),
            river_component_closure: Some(river),
            strait_component_closure: Some(strait),
            deterministic_sha256: String::new(),
        };
        reseal_receipt(&mut receipt).unwrap();
        assert!(receipt
            .validate_complete_structure_for_frame(&frame_sha256)
            .is_err());
    }

    #[test]
    fn river_flow_direction_matches_exact_group_parity() {
        for render_group in 0u8..=u8::MAX {
            assert_eq!(
                f32::from_bits(river_flow_direction_f32_bits(render_group)),
                if render_group & 1 == 0 { -1.0 } else { 1.0 }
            );
        }
    }

    #[cfg(windows)]
    struct TestRiverResource {
        owner: &'static str,
        symbol: &'static str,
        shader_register: u32,
        camera: Option<String>,
        replay: String,
    }

    #[cfg(windows)]
    unsafe impl NativeXmap08RiverShaderResource for TestRiverResource {
        fn owner_packet(&self) -> &str {
            self.owner
        }
        fn symbol(&self) -> &str {
            self.symbol
        }
        fn shader_register(&self) -> u32 {
            self.shader_register
        }
        fn camera_frame_sha256(&self) -> Option<&str> {
            self.camera.as_deref()
        }
        fn replay_frame_identity_sha256(&self) -> &str {
            &self.replay
        }
        fn resource_sha256(&self) -> &str {
            "2d9cc53746e6d06ecde21d7bfe29eeca1f05cb88e4d9c2a228e1164bf1f4a2f2"
        }
        fn descriptor_sha256(&self) -> &str {
            "b9b60e00fe126988bbb6a108c24e5e2d67ff44693cbdb7b06814c478fa8146cd"
        }
        fn raw_shader_resource_view(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
            std::ptr::null_mut()
        }
    }

    #[cfg(windows)]
    #[test]
    fn river_provider_identity_enforces_packet_specific_camera_scope() {
        let camera = "a".repeat(64);
        let replay = "b".repeat(64);
        let terrain = TestRiverResource {
            owner: "XMAP04",
            symbol: "HeightNormal",
            shader_register: 4,
            camera: None,
            replay: replay.clone(),
        };
        validate_external_river_resource_identity(
            &terrain,
            "XMAP04",
            "HeightNormal",
            4,
            &camera,
            &replay,
        )
        .unwrap();
        let mut water = TestRiverResource {
            owner: "XMAP07",
            symbol: "WaterColor",
            shader_register: 0,
            camera: None,
            replay: replay.clone(),
        };
        validate_external_river_resource_identity(
            &water,
            "XMAP07",
            "WaterColor",
            0,
            &camera,
            &replay,
        )
        .unwrap();
        water.camera = Some(camera.clone());
        assert!(validate_external_river_resource_identity(
            &water,
            "XMAP07",
            "WaterColor",
            0,
            &camera,
            &replay,
        )
        .is_err());

        let mut lighting = TestRiverResource {
            owner: "XMAP09",
            symbol: "LightIndexMap",
            shader_register: 10,
            camera: Some(camera.clone()),
            replay: replay.clone(),
        };
        validate_external_river_resource_identity(
            &lighting,
            "XMAP09",
            "LightIndexMap",
            10,
            &camera,
            &replay,
        )
        .unwrap();
        lighting.camera = Some("c".repeat(64));
        assert!(validate_external_river_resource_identity(
            &lighting,
            "XMAP09",
            "LightIndexMap",
            10,
            &camera,
            &replay,
        )
        .is_err());
        lighting.camera = Some(camera.clone());
        lighting.replay = "d".repeat(64);
        assert!(validate_external_river_resource_identity(
            &lighting,
            "XMAP09",
            "LightIndexMap",
            10,
            &camera,
            &replay,
        )
        .is_err());

        let screen_shadow = TestRiverResource {
            owner: "XMAP10",
            symbol: "ShadowMap",
            shader_register: 15,
            camera: Some(camera.clone()),
            replay: replay.clone(),
        };
        validate_external_river_resource_identity(
            &screen_shadow,
            "XMAP10",
            "ShadowMap",
            15,
            &camera,
            &replay,
        )
        .unwrap();
    }

    #[cfg(windows)]
    struct TestSceneCb0 {
        render_view_mask: u8,
        map_copy_offset_f32_bits: u32,
        camera: String,
        replay: String,
        map_copy_set: String,
    }

    #[cfg(windows)]
    unsafe impl crate::NativeXmap04UnderwaterSceneConstantBuffer for TestSceneCb0 {
        fn owner_packet(&self) -> &str {
            "XMAP10"
        }
        fn symbol(&self) -> &str {
            "SceneCB0"
        }
        fn shader_register(&self) -> u32 {
            0
        }
        fn byte_len(&self) -> u32 {
            416
        }
        fn viewport_width(&self) -> u32 {
            64
        }
        fn viewport_height(&self) -> u32 {
            64
        }
        fn frame_identity_sha256(&self) -> &str {
            &self.camera
        }
        fn replay_frame_identity_sha256(&self) -> &str {
            &self.replay
        }
        fn payload_sha256(&self) -> &str {
            "37132880363b1207d6fcb41b2a3fc940963cab8ae33b90c8d6ec93115f0448f7"
        }
        fn descriptor_sha256(&self) -> &str {
            "ffb07670ec3a24e330b237280b348c95f9756a0801a8bb112307760bb573fe1b"
        }
        fn raw_buffer(&self) -> *mut winapi::um::d3d11::ID3D11Buffer {
            std::ptr::null_mut()
        }
    }

    #[cfg(windows)]
    unsafe impl NativeXmap08SceneConstantBuffer for TestSceneCb0 {
        fn map_copy_receipt_set_sha256(&self) -> &str {
            &self.map_copy_set
        }
        fn render_view_mask(&self) -> u8 {
            self.render_view_mask
        }
        fn map_copy_offset_f32_bits(&self) -> u32 {
            self.map_copy_offset_f32_bits
        }
        fn composition_receipt_sha256(&self) -> &str {
            "4191665597c169d10eaefcc0801cbeb973b4098b39d49328aad065af1e07c9c1"
        }
        fn reflected_layout_sha256(&self) -> &str {
            "fb8e4134f935cfb3a54c411a4d9afbb8c18f0429ec9f07401367d646592415cd"
        }
        fn main_scene_resource_identity_sha256(&self) -> &str {
            "8c6abff1fda48f35de7cca3f88cdb7af4bca0f09972f3ecb95bd697afad75b33"
        }
        fn raw_main_scene_color_texture(&self) -> *mut winapi::um::d3d11::ID3D11Texture2D {
            std::ptr::null_mut()
        }
    }

    #[cfg(windows)]
    #[test]
    fn scene_cb0_provider_requires_exact_canonical_map_copy_rows_and_order() {
        let camera = "a".repeat(64);
        let replay = "b".repeat(64);
        let rows = [
            crate::Xmap04MapCopyReceiptRow {
                render_view_mask: 1,
                map_copy_offset_f32_bits: 0.0f32.to_bits(),
            },
            crate::Xmap04MapCopyReceiptRow {
                render_view_mask: 2,
                map_copy_offset_f32_bits: 64.0f32.to_bits(),
            },
        ];
        let map_copy_set = crate::xmap04_map_copy_receipt_set_sha256(64, &rows).unwrap();
        let left = TestSceneCb0 {
            render_view_mask: rows[0].render_view_mask,
            map_copy_offset_f32_bits: rows[0].map_copy_offset_f32_bits,
            camera: camera.clone(),
            replay: replay.clone(),
            map_copy_set: map_copy_set.clone(),
        };
        let right = TestSceneCb0 {
            render_view_mask: rows[1].render_view_mask,
            map_copy_offset_f32_bits: rows[1].map_copy_offset_f32_bits,
            camera: camera.clone(),
            replay: replay.clone(),
            map_copy_set: map_copy_set.clone(),
        };
        let canonical: [&dyn NativeXmap08SceneConstantBuffer; 2] = [&left, &right];
        validate_scene_cb0_provider_identities(
            64,
            &camera,
            &replay,
            &map_copy_set,
            416,
            "fb8e4134f935cfb3a54c411a4d9afbb8c18f0429ec9f07401367d646592415cd",
            64,
            64,
            "8c6abff1fda48f35de7cca3f88cdb7af4bca0f09972f3ecb95bd697afad75b33",
            &canonical,
        )
        .unwrap();

        let reversed: [&dyn NativeXmap08SceneConstantBuffer; 2] = [&right, &left];
        assert!(validate_scene_cb0_provider_identities(
            64,
            &camera,
            &replay,
            &map_copy_set,
            416,
            &"e".repeat(64),
            64,
            64,
            "8c6abff1fda48f35de7cca3f88cdb7af4bca0f09972f3ecb95bd697afad75b33",
            &canonical,
        )
        .is_err());
        assert!(validate_scene_cb0_provider_identities(
            64,
            &camera,
            &replay,
            &map_copy_set,
            416,
            "fb8e4134f935cfb3a54c411a4d9afbb8c18f0429ec9f07401367d646592415cd",
            64,
            64,
            "8c6abff1fda48f35de7cca3f88cdb7af4bca0f09972f3ecb95bd697afad75b33",
            &reversed,
        )
        .is_err());
        assert!(validate_scene_cb0_provider_identities(
            64,
            &camera,
            &"c".repeat(64),
            &map_copy_set,
            416,
            "fb8e4134f935cfb3a54c411a4d9afbb8c18f0429ec9f07401367d646592415cd",
            64,
            64,
            "8c6abff1fda48f35de7cca3f88cdb7af4bca0f09972f3ecb95bd697afad75b33",
            &canonical,
        )
        .is_err());
        assert!(validate_scene_cb0_provider_identities(
            64,
            &camera,
            &replay,
            &map_copy_set,
            416,
            "fb8e4134f935cfb3a54c411a4d9afbb8c18f0429ec9f07401367d646592415cd",
            64,
            64,
            &"d".repeat(64),
            &canonical,
        )
        .is_err());
    }
}
