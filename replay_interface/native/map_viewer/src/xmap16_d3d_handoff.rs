
use crate::{PreparedXmap16Frame, Result};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;

pub const XMAP16_RAIL_STAGE_NAME: &str = "xmap16.rail.default_ribbon.execute";
pub const XMAP16_RAIL_OPERATION_ID: &str = "rail.default-ribbon.draw";
pub const XMAP16_RAIL_COMPONENT_ID: &str = "rail.default_ribbon";
pub const XMAP16_RAIL_TARGET_BOUND_DRAW_SCHEMA: &str = "hoi4.xmap16.rail-target-bound-draw.v1";
pub const XMAP16_RAIL_COMPONENT_CLOSURE_SCHEMA: &str = "hoi4.xmap16.rail-component-closure.v1";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap16RailPipelineStateContract {
    pub raster_fill_mode: u32,
    pub raster_cull_mode: u32,
    pub raster_front_counter_clockwise: bool,
    pub raster_depth_bias: i32,
    pub raster_depth_bias_clamp_bits: u32,
    pub raster_slope_scaled_depth_bias_bits: u32,
    pub raster_depth_clip: bool,
    pub raster_scissor: bool,
    pub raster_multisample: bool,
    pub raster_antialiased_line: bool,
    pub blend_alpha_to_coverage: bool,
    pub blend_independent: bool,
    pub blend_enabled: bool,
    pub blend_source: u32,
    pub blend_destination: u32,
    pub blend_operation: u32,
    pub blend_source_alpha: u32,
    pub blend_destination_alpha: u32,
    pub blend_alpha_operation: u32,
    pub blend_write_mask: u8,
    pub depth_enabled: bool,
    pub depth_write_mask: u32,
    pub depth_comparison: u32,
    pub stencil_enabled: bool,
    pub stencil_read_mask: u8,
    pub stencil_write_mask: u8,
    pub stencil_fail: u32,
    pub stencil_depth_fail: u32,
    pub stencil_pass: u32,
    pub stencil_comparison: u32,
}

pub const XMAP16_RAIL_PIPELINE_STATE: Xmap16RailPipelineStateContract =
    Xmap16RailPipelineStateContract {
        raster_fill_mode: 3,
        raster_cull_mode: 3,
        raster_front_counter_clockwise: false,
        raster_depth_bias: 0,
        raster_depth_bias_clamp_bits: 0.0f32.to_bits(),
        raster_slope_scaled_depth_bias_bits: 0.0f32.to_bits(),
        raster_depth_clip: true,
        raster_scissor: false,
        raster_multisample: false,
        raster_antialiased_line: false,
        blend_alpha_to_coverage: false,
        blend_independent: false,
        blend_enabled: true,
        blend_source: 5,
        blend_destination: 6,
        blend_operation: 1,
        blend_source_alpha: 5,
        blend_destination_alpha: 6,
        blend_alpha_operation: 1,
        blend_write_mask: 0x07,
        depth_enabled: false,
        depth_write_mask: 1,
        depth_comparison: 2,
        stencil_enabled: false,
        stencil_read_mask: 0xff,
        stencil_write_mask: 0xff,
        stencil_fail: 1,
        stencil_depth_fail: 1,
        stencil_pass: 1,
        stencil_comparison: 8,
    };

pub const XMAP16_EXTERNAL_PIXEL_RESOURCES: [(&str, &str, u32); 5] = [
    ("ProvinceSecondaryColorMap", "XMAP05", 6),
    ("SnowMudTexture", "XMAP07", 8),
    ("GradientBorderChannel1", "XMAP05", 12),
    ("GradientBorderChannel2", "XMAP05", 13),
    ("ShadowMap", "XMAP10", 15),
];
pub const XMAP16_RAIL_DIAGNOSTIC_MANIFEST_SHA256: &str =
    "2582c0049f53135b48d3ed4a1d48da36fbaa29f3bd9dc8457dc3603e6f1623af";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap16RailDiagnosticDisposition {
    pub disposition: String,
    pub manifest_sha256: String,
    pub default_gate_a: bool,
    pub default_gate_b: bool,
    pub witness_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap16RailSharedExecutionIdentity {
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub map_copy_receipt_set_sha256: String,
}

impl Xmap16RailSharedExecutionIdentity {
    pub fn validate(&self) -> Result<()> {
        for (label, value) in [
            ("content manifest", &self.content_manifest_sha256),
            (
                "foundation content operation",
                &self.foundation_content_operation_sha256,
            ),
            ("viewer state", &self.viewer_state_sha256),
            ("MainScene target", &self.main_scene_target_sha256),
            ("map-copy receipt set", &self.map_copy_receipt_set_sha256),
        ] {
            validate_sha256(value, label)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap16RailTargetIdentity {
    pub target_sha256: String,
    pub descriptor_sha256: String,
    pub color_width: u32,
    pub color_height: u32,
    pub color_format: u32,
    pub color_samples: u32,
    pub color_quality: u32,
    pub depth_format: u32,
    pub depth_samples: u32,
    pub depth_quality: u32,
    pub deterministic_sha256: String,
}

impl Xmap16RailTargetIdentity {
    pub fn from_desc(
        target_sha256: impl Into<String>,
        desc: &crate::ProductionMainSceneTargetDesc,
    ) -> Result<Self> {
        desc.validate()?;
        let mut identity = Self {
            target_sha256: target_sha256.into(),
            descriptor_sha256: Sha256::hex(&serde_json::to_vec(desc)?),
            color_width: desc.color_width,
            color_height: desc.color_height,
            color_format: desc.color_format,
            color_samples: desc.color_samples,
            color_quality: desc.color_quality,
            depth_format: desc.depth_format,
            depth_samples: desc.depth_samples,
            depth_quality: desc.depth_quality,
            deterministic_sha256: String::new(),
        };
        identity.deterministic_sha256 = target_identity_digest(&identity)?;
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<()> {
        validate_sha256(&self.target_sha256, "rail target")?;
        validate_sha256(&self.descriptor_sha256, "rail target descriptor")?;
        if self.color_width == 0
            || self.color_height == 0
            || self.color_samples == 0
            || self.depth_samples == 0
            || self.color_samples != self.depth_samples
            || self.color_quality != self.depth_quality
            || self.deterministic_sha256 != target_identity_digest(self)?
        {
            return Err("XMAP16 railroad target identity is invalid".into());
        }
        Ok(())
    }
}

fn target_identity_digest(identity: &Xmap16RailTargetIdentity) -> Result<String> {
    let mut unsigned = identity.clone();
    unsigned.deterministic_sha256.clear();
    Ok(Sha256::hex(&serde_json::to_vec(&unsigned)?))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap16RailExternalResourceIdentity {
    pub symbol: String,
    pub shader_register: u32,
    pub owner_packet: String,
    pub payload_sha256: String,
    pub descriptor_sha256: String,
}

impl Xmap16RailExternalResourceIdentity {
    pub fn validate(&self) -> Result<()> {
        validate_sha256(&self.payload_sha256, "rail external resource payload")?;
        validate_sha256(&self.descriptor_sha256, "rail external resource descriptor")?;
        if !XMAP16_EXTERNAL_PIXEL_RESOURCES.contains(&(
            self.symbol.as_str(),
            self.owner_packet.as_str(),
            self.shader_register,
        )) {
            return Err(format!(
                "XMAP16 received unexpected railroad resource {} t{} from {}",
                self.symbol, self.shader_register, self.owner_packet
            )
            .into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap16RailTargetBoundDrawReceipt {
    pub schema: String,
    pub operation_id: String,
    pub stage_name: String,
    pub topology_sha256: String,
    pub assigned_ordinal: u32,
    pub assignment_sha256: String,
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub prepared_frame_sha256: String,
    pub handoff_receipt_sha256: String,
    pub ordered_actual_draw_receipt_sha256s: Vec<String>,
    pub issued_draw_count: u32,
    pub issued_index_count: u32,
    pub diagnostic_disposition_sha256: String,
    pub packet_draw_receipt_sha256: String,
}

impl Xmap16RailTargetBoundDrawReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP16_RAIL_TARGET_BOUND_DRAW_SCHEMA
            || self.operation_id != XMAP16_RAIL_OPERATION_ID
            || self.stage_name != XMAP16_RAIL_STAGE_NAME
            || self.assigned_ordinal == 0
            || self.ordered_actual_draw_receipt_sha256s.is_empty()
            || self.issued_draw_count
                != u32::try_from(self.ordered_actual_draw_receipt_sha256s.len())?
            || self.issued_index_count != 0
        {
            return Err("XMAP16 target-bound railroad draw receipt body is invalid".into());
        }
        for (label, value) in [
            ("assigned topology", &self.topology_sha256),
            ("assigned operation", &self.assignment_sha256),
            ("content manifest", &self.content_manifest_sha256),
            (
                "foundation content operation",
                &self.foundation_content_operation_sha256,
            ),
            ("viewer state", &self.viewer_state_sha256),
            ("MainScene target", &self.main_scene_target_sha256),
            ("map-copy receipt set", &self.map_copy_receipt_set_sha256),
            ("prepared railroad frame", &self.prepared_frame_sha256),
            ("railroad handoff", &self.handoff_receipt_sha256),
            (
                "rail diagnostic disposition",
                &self.diagnostic_disposition_sha256,
            ),
        ] {
            validate_sha256(value, label)?;
        }
        let mut actual = std::collections::BTreeSet::new();
        for receipt in &self.ordered_actual_draw_receipt_sha256s {
            validate_sha256(receipt, "actual railroad draw")?;
            if !actual.insert(receipt.as_str()) {
                return Err("XMAP16 railroad receipt repeats an actual draw".into());
            }
        }
        validate_sha256(
            &self.packet_draw_receipt_sha256,
            "target-bound railroad draw packet",
        )?;
        let mut unsigned = self.clone();
        unsigned.packet_draw_receipt_sha256.clear();
        if self.packet_draw_receipt_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP16 target-bound railroad draw digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Xmap16RailComponentDisposition {
    Present {
        packet_draw_receipt_sha256: String,
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
pub struct Xmap16RailComponentClosure {
    pub schema: String,
    pub component_id: String,
    pub operation_id: String,
    pub stage_name: String,
    pub topology_sha256: String,
    pub assigned_ordinal: u32,
    pub assignment_sha256: String,
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub prepared_frame_sha256: String,
    pub handoff_receipt_sha256: String,
    pub disposition: Xmap16RailComponentDisposition,
    pub closure_sha256: String,
}

impl Xmap16RailComponentClosure {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP16_RAIL_COMPONENT_CLOSURE_SCHEMA
            || self.component_id != XMAP16_RAIL_COMPONENT_ID
            || self.operation_id != XMAP16_RAIL_OPERATION_ID
            || self.stage_name != XMAP16_RAIL_STAGE_NAME
            || self.assigned_ordinal == 0
        {
            return Err("XMAP16 railroad component closure body is invalid".into());
        }
        for (label, value) in [
            ("assigned topology", &self.topology_sha256),
            ("assigned operation", &self.assignment_sha256),
            ("content manifest", &self.content_manifest_sha256),
            (
                "foundation content operation",
                &self.foundation_content_operation_sha256,
            ),
            ("viewer state", &self.viewer_state_sha256),
            ("MainScene target", &self.main_scene_target_sha256),
            ("map-copy receipt set", &self.map_copy_receipt_set_sha256),
            ("prepared railroad frame", &self.prepared_frame_sha256),
            ("railroad handoff", &self.handoff_receipt_sha256),
        ] {
            validate_sha256(value, label)?;
        }
        match &self.disposition {
            Xmap16RailComponentDisposition::Present {
                packet_draw_receipt_sha256,
            } => validate_sha256(packet_draw_receipt_sha256, "rail Present packet")?,
            Xmap16RailComponentDisposition::AuthenticatedEmpty {
                enumerator_witness_sha256,
            } => validate_sha256(enumerator_witness_sha256, "empty rail enumerator")?,
            Xmap16RailComponentDisposition::NotApplicable {
                applicability_witness_sha256,
                reason,
            } => {
                validate_sha256(applicability_witness_sha256, "rail applicability")?;
                if reason != "railway_alpha_disabled" {
                    return Err("XMAP16 railroad NotApplicable reason is invalid".into());
                }
            }
        }
        validate_sha256(&self.closure_sha256, "rail component closure")?;
        let mut unsigned = self.clone();
        unsigned.closure_sha256.clear();
        if self.closure_sha256 != Sha256::hex(&serde_json::to_vec(&unsigned)?) {
            return Err("XMAP16 railroad component closure digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap16RailExecutionReceipt {
    pub packet: Option<Xmap16RailTargetBoundDrawReceipt>,
    pub closure: Xmap16RailComponentClosure,
}

impl Xmap16RailExecutionReceipt {
    pub fn validate(&self) -> Result<()> {
        self.closure.validate()?;
        match (&self.packet, &self.closure.disposition) {
            (
                Some(packet),
                Xmap16RailComponentDisposition::Present {
                    packet_draw_receipt_sha256,
                },
            ) if packet.validate().is_ok()
                && packet_draw_receipt_sha256 == &packet.packet_draw_receipt_sha256 => {}
            (None, Xmap16RailComponentDisposition::AuthenticatedEmpty { .. })
            | (None, Xmap16RailComponentDisposition::NotApplicable { .. }) => {}
            _ => return Err("XMAP16 railroad execution receipt is inconsistent".into()),
        }
        Ok(())
    }
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase SHA-256").into());
    }
    Ok(())
}

fn validate_assigned_stage(assignment: &portable_render_stream::AssignedStage) -> Result<()> {
    assignment
        .validate()
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    if assignment.stage_name != XMAP16_RAIL_STAGE_NAME
        || assignment.operation_name != XMAP16_RAIL_OPERATION_ID
    {
        return Err("XMAP16 railroad draw received the wrong assigned stage".into());
    }
    Ok(())
}

fn empty_enumerator_witness(frame: &PreparedXmap16Frame) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(b"hoi4.xmap16.rail-empty-enumerator.v1\0");
    identity.extend_from_slice(frame.deterministic_sha256.as_bytes());
    identity.extend_from_slice(&(frame.rail.ordered_route_records.len() as u64).to_le_bytes());
    identity.extend_from_slice(&(frame.rail.ribbon_program.vertices.len() as u64).to_le_bytes());
    identity
        .extend_from_slice(&(frame.rail.ribbon_program.ordered_draws.len() as u64).to_le_bytes());
    for record in &frame.rail.ordered_route_records {
        identity.extend_from_slice(&record.previous_province_id.to_le_bytes());
        identity.extend_from_slice(&record.center_province_id.to_le_bytes());
        identity.extend_from_slice(&record.next_province_id.to_le_bytes());
        identity.extend_from_slice(&record.effective_level.to_le_bytes());
    }
    Sha256::hex(&identity)
}

fn alpha_not_applicable_witness(
    frame: &PreparedXmap16Frame,
    handoff_receipt_sha256: &str,
) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(b"hoi4.xmap16.rail-alpha-applicability.v1\0");
    identity.extend_from_slice(&frame.rail.railway_alpha.to_bits().to_le_bytes());
    identity.push(frame.rail.railway_draw_enabled as u8);
    identity.extend_from_slice(frame.deterministic_sha256.as_bytes());
    identity.extend_from_slice(handoff_receipt_sha256.as_bytes());
    Sha256::hex(&identity)
}

fn seal_component_closure(
    assignment: &portable_render_stream::AssignedStage,
    frame: &PreparedXmap16Frame,
    shared: &Xmap16RailSharedExecutionIdentity,
    handoff_receipt_sha256: &str,
    disposition: Xmap16RailComponentDisposition,
) -> Result<Xmap16RailComponentClosure> {
    validate_assigned_stage(assignment)?;
    frame.validate()?;
    shared.validate()?;
    validate_sha256(handoff_receipt_sha256, "railroad handoff")?;
    let mut closure = Xmap16RailComponentClosure {
        schema: XMAP16_RAIL_COMPONENT_CLOSURE_SCHEMA.into(),
        component_id: XMAP16_RAIL_COMPONENT_ID.into(),
        operation_id: XMAP16_RAIL_OPERATION_ID.into(),
        stage_name: XMAP16_RAIL_STAGE_NAME.into(),
        topology_sha256: assignment.topology_sha256.clone(),
        assigned_ordinal: assignment.assigned_ordinal,
        assignment_sha256: assignment.assignment_sha256.clone(),
        hour_index: frame.hour_index,
        content_manifest_sha256: shared.content_manifest_sha256.clone(),
        foundation_content_operation_sha256: shared.foundation_content_operation_sha256.clone(),
        viewer_state_sha256: shared.viewer_state_sha256.clone(),
        main_scene_target_sha256: shared.main_scene_target_sha256.clone(),
        map_copy_receipt_set_sha256: shared.map_copy_receipt_set_sha256.clone(),
        prepared_frame_sha256: frame.deterministic_sha256.clone(),
        handoff_receipt_sha256: handoff_receipt_sha256.into(),
        disposition,
        closure_sha256: String::new(),
    };
    closure.closure_sha256 = Sha256::hex(&serde_json::to_vec(&closure)?);
    closure.validate()?;
    Ok(closure)
}

fn seal_present_packet(
    assignment: &portable_render_stream::AssignedStage,
    frame: &PreparedXmap16Frame,
    shared: &Xmap16RailSharedExecutionIdentity,
    handoff: &Xmap16RailHandoffReceipt,
    ordered_actual_draw_receipt_sha256s: Vec<String>,
) -> Result<Xmap16RailTargetBoundDrawReceipt> {
    validate_assigned_stage(assignment)?;
    frame.validate()?;
    shared.validate()?;
    handoff.validate()?;
    let mut receipt = Xmap16RailTargetBoundDrawReceipt {
        schema: XMAP16_RAIL_TARGET_BOUND_DRAW_SCHEMA.into(),
        operation_id: XMAP16_RAIL_OPERATION_ID.into(),
        stage_name: XMAP16_RAIL_STAGE_NAME.into(),
        topology_sha256: assignment.topology_sha256.clone(),
        assigned_ordinal: assignment.assigned_ordinal,
        assignment_sha256: assignment.assignment_sha256.clone(),
        hour_index: frame.hour_index,
        content_manifest_sha256: shared.content_manifest_sha256.clone(),
        foundation_content_operation_sha256: shared.foundation_content_operation_sha256.clone(),
        viewer_state_sha256: shared.viewer_state_sha256.clone(),
        main_scene_target_sha256: shared.main_scene_target_sha256.clone(),
        map_copy_receipt_set_sha256: shared.map_copy_receipt_set_sha256.clone(),
        prepared_frame_sha256: frame.deterministic_sha256.clone(),
        handoff_receipt_sha256: handoff.deterministic_sha256.clone(),
        issued_draw_count: u32::try_from(ordered_actual_draw_receipt_sha256s.len())?,
        ordered_actual_draw_receipt_sha256s,
        issued_index_count: 0,
        diagnostic_disposition_sha256: handoff.diagnostic_disposition.witness_sha256.clone(),
        packet_draw_receipt_sha256: String::new(),
    };
    receipt.packet_draw_receipt_sha256 = Sha256::hex(&serde_json::to_vec(&receipt)?);
    receipt.validate()?;
    Ok(receipt)
}

fn actual_draw_receipt_sha256(
    frame: &PreparedXmap16Frame,
    handoff: &Xmap16RailHandoffReceipt,
    assignment: &portable_render_stream::AssignedStage,
    target: &Xmap16RailTargetIdentity,
    scene_cb0_payload_sha256: &str,
    external_resources: &[Xmap16RailExternalResourceIdentity],
    draw: &crate::rail_files::ExactRailRibbonDraw,
    draw_ordinal: u32,
) -> Result<String> {
    let mut identity = Vec::new();
    identity.extend_from_slice(b"hoi4.xmap16.rail-actual-draw.v1\0");
    identity.extend_from_slice(frame.deterministic_sha256.as_bytes());
    identity.extend_from_slice(handoff.deterministic_sha256.as_bytes());
    identity.extend_from_slice(assignment.assignment_sha256.as_bytes());
    identity.extend_from_slice(target.deterministic_sha256.as_bytes());
    identity.extend_from_slice(scene_cb0_payload_sha256.as_bytes());
    identity.extend_from_slice(&draw_ordinal.to_le_bytes());
    identity.extend_from_slice(&draw.route_record_ordinal.to_le_bytes());
    identity.extend_from_slice(&draw.first_vertex.to_le_bytes());
    identity.extend_from_slice(&draw.vertex_count.to_le_bytes());
    identity.extend_from_slice(&draw.effective_level.to_le_bytes());
    for resource in external_resources {
        resource.validate()?;
        identity.extend_from_slice(resource.symbol.as_bytes());
        identity.extend_from_slice(&resource.shader_register.to_le_bytes());
        identity.extend_from_slice(resource.owner_packet.as_bytes());
        identity.extend_from_slice(resource.payload_sha256.as_bytes());
        identity.extend_from_slice(resource.descriptor_sha256.as_bytes());
    }
    Ok(Sha256::hex(&identity))
}

impl Xmap16RailDiagnosticDisposition {
    pub fn exact_a729_default_off() -> Self {
        let mut disposition = Self {
            disposition: "not_applicable_default_off_debug_lines".into(),
            manifest_sha256: XMAP16_RAIL_DIAGNOSTIC_MANIFEST_SHA256.into(),
            default_gate_a: false,
            default_gate_b: false,
            witness_sha256: String::new(),
        };
        disposition.witness_sha256 = rail_diagnostic_witness(&disposition);
        disposition
    }

    pub fn validate(&self) -> Result<()> {
        if self.disposition != "not_applicable_default_off_debug_lines"
            || self.manifest_sha256 != XMAP16_RAIL_DIAGNOSTIC_MANIFEST_SHA256
            || self.default_gate_a
            || self.default_gate_b
            || self.witness_sha256 != rail_diagnostic_witness(self)
        {
            return Err("XMAP16 rail diagnostic disposition is invalid".into());
        }
        Ok(())
    }
}

fn rail_diagnostic_witness(value: &Xmap16RailDiagnosticDisposition) -> String {
    Sha256::hex(
        format!(
            "hoi4.xmap16.rail-diagnostic-disposition.v1\0{}\0{}\0{}\0{}",
            value.disposition,
            value.manifest_sha256,
            value.default_gate_a as u8,
            value.default_gate_b as u8
        )
        .as_bytes(),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap16RailHandoffReceipt {
    pub frame_sha256: String,
    pub program_sha256: String,
    pub vertex_payload_sha256: String,
    pub vertex_count: u32,
    pub draw_count: u32,
    pub rail_texture_sha256: String,
    pub rail_texture_descriptor_sha256: String,
    pub external_resource_contract_sha256: String,
    pub pipeline_state_sha256: String,
    pub diagnostic_disposition: Xmap16RailDiagnosticDisposition,
    pub native_resources_ready: bool,
    pub deterministic_sha256: String,
}

impl Xmap16RailHandoffReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.frame_sha256.len() != 64
            || self.program_sha256.len() != 64
            || self.vertex_payload_sha256.len() != 64
            || self.rail_texture_sha256.len() != 64
            || validate_sha256(
                &self.rail_texture_descriptor_sha256,
                "railroad texture descriptor",
            )
            .is_err()
            || self.external_resource_contract_sha256 != external_resource_contract_sha256()
            || self.pipeline_state_sha256 != rail_pipeline_state_sha256()
            || self.diagnostic_disposition.validate().is_err()
            || !self.native_resources_ready
            || self.deterministic_sha256 != handoff_digest(self)
        {
            return Err("XMAP16 native railroad handoff receipt is invalid".into());
        }
        Ok(())
    }
}

fn ribbon_vertex_bytes(frame: &PreparedXmap16Frame) -> Result<Vec<u8>> {
    let expected = frame
        .rail
        .ribbon_program
        .vertices
        .len()
        .checked_mul(crate::A729_RAILROAD_VERTEX_STRIDE_BYTES as usize)
        .ok_or("XMAP16 ribbon byte count overflow")?;
    let mut bytes = Vec::with_capacity(expected);
    for vertex in &frame.rail.ribbon_program.vertices {
        for value in vertex.position.into_iter().chain(vertex.uv) {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    if bytes.len() != expected {
        return Err("XMAP16 ribbon encoding violated the exact 24-byte stride".into());
    }
    Ok(bytes)
}

fn external_resource_contract_sha256() -> String {
    let mut identity = Vec::new();
    for (symbol, owner, slot) in XMAP16_EXTERNAL_PIXEL_RESOURCES {
        identity.extend_from_slice(symbol.as_bytes());
        identity.extend_from_slice(owner.as_bytes());
        identity.extend_from_slice(&slot.to_le_bytes());
    }
    Sha256::hex(&identity)
}

fn rail_pipeline_state_sha256() -> String {
    Sha256::hex(
        &serde_json::to_vec(&XMAP16_RAIL_PIPELINE_STATE)
            .expect("fixed XMAP16 railroad pipeline state serializes"),
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Xmap16RailDdsMipDescriptor {
    width: u32,
    height: u32,
    source_offset: u64,
    row_pitch: u32,
    slice_pitch: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Xmap16RailDdsDescriptor {
    width: u32,
    height: u32,
    mip_levels: u32,
    dxgi_format: u32,
    payload_sha256: String,
    mips: Vec<Xmap16RailDdsMipDescriptor>,
}

fn rail_texture_descriptor_sha256(descriptor: &Xmap16RailDdsDescriptor) -> String {
    let mut identity = b"XMAP16_RAIL_DDS_DESCRIPTOR_V1\0".to_vec();
    identity.extend_from_slice(
        &serde_json::to_vec(descriptor).expect("validated railroad DDS descriptor serializes"),
    );
    Sha256::hex(&identity)
}

fn handoff_receipt(
    frame: &PreparedXmap16Frame,
    vertex_bytes: &[u8],
    rail_texture_descriptor_sha256: &str,
) -> Result<Xmap16RailHandoffReceipt> {
    validate_sha256(
        rail_texture_descriptor_sha256,
        "railroad texture descriptor",
    )?;
    let mut receipt = Xmap16RailHandoffReceipt {
        frame_sha256: frame.deterministic_sha256.clone(),
        program_sha256: frame.program.deterministic_sha256.clone(),
        vertex_payload_sha256: Sha256::hex(vertex_bytes),
        vertex_count: u32::try_from(frame.rail.ribbon_program.vertices.len())?,
        draw_count: u32::try_from(frame.rail.ribbon_program.ordered_draws.len())?,
        rail_texture_sha256: frame.rail.railroad_texture.sha256.clone(),
        rail_texture_descriptor_sha256: rail_texture_descriptor_sha256.to_string(),
        external_resource_contract_sha256: external_resource_contract_sha256(),
        pipeline_state_sha256: rail_pipeline_state_sha256(),
        diagnostic_disposition: Xmap16RailDiagnosticDisposition::exact_a729_default_off(),
        native_resources_ready: true,
        deterministic_sha256: String::new(),
    };
    receipt.deterministic_sha256 = handoff_digest(&receipt);
    receipt.validate()?;
    Ok(receipt)
}

fn handoff_digest(receipt: &Xmap16RailHandoffReceipt) -> String {
    let mut identity = Vec::new();
    identity.extend_from_slice(receipt.frame_sha256.as_bytes());
    identity.extend_from_slice(receipt.program_sha256.as_bytes());
    identity.extend_from_slice(receipt.vertex_payload_sha256.as_bytes());
    identity.extend_from_slice(&receipt.vertex_count.to_le_bytes());
    identity.extend_from_slice(&receipt.draw_count.to_le_bytes());
    identity.extend_from_slice(receipt.rail_texture_sha256.as_bytes());
    identity.extend_from_slice(receipt.rail_texture_descriptor_sha256.as_bytes());
    identity.extend_from_slice(receipt.external_resource_contract_sha256.as_bytes());
    identity.extend_from_slice(receipt.pipeline_state_sha256.as_bytes());
    identity.extend_from_slice(receipt.diagnostic_disposition.witness_sha256.as_bytes());
    identity.push(receipt.native_resources_ready as u8);
    Sha256::hex(&identity)
}

#[cfg(windows)]
#[derive(Clone, Debug)]
pub struct NativeXmap16RailExternalResourceView {
    pub identity: Xmap16RailExternalResourceIdentity,
    raw_srv: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
}

#[cfg(windows)]
impl NativeXmap16RailExternalResourceView {
    pub unsafe fn from_provider(
        identity: Xmap16RailExternalResourceIdentity,
        raw_srv: *mut winapi::um::d3d11::ID3D11ShaderResourceView,
    ) -> Result<Self> {
        identity.validate()?;
        if raw_srv.is_null() {
            return Err("XMAP16 received a null external railroad SRV".into());
        }
        Ok(Self { identity, raw_srv })
    }

    pub fn as_raw(&self) -> *mut winapi::um::d3d11::ID3D11ShaderResourceView {
        self.raw_srv
    }
}

#[cfg(windows)]
pub unsafe trait NativeXmap16MainSceneTargetView {
    fn desc(&self) -> &crate::ProductionMainSceneTargetDesc;
    fn color_rtv(&self) -> *mut winapi::um::d3d11::ID3D11RenderTargetView;
    fn depth_dsv(&self) -> *mut winapi::um::d3d11::ID3D11DepthStencilView;
}

#[cfg(windows)]
unsafe impl NativeXmap16MainSceneTargetView for crate::NativeProductionMainSceneTarget {
    fn desc(&self) -> &crate::ProductionMainSceneTargetDesc {
        self.desc()
    }

    fn color_rtv(&self) -> *mut winapi::um::d3d11::ID3D11RenderTargetView {
        self.color_rtv()
    }

    fn depth_dsv(&self) -> *mut winapi::um::d3d11::ID3D11DepthStencilView {
        self.depth_dsv()
    }
}

#[cfg(windows)]
unsafe impl NativeXmap16MainSceneTargetView for crate::NativeProductionD3dBackend {
    fn desc(&self) -> &crate::ProductionMainSceneTargetDesc {
        self.desc()
    }

    fn color_rtv(&self) -> *mut winapi::um::d3d11::ID3D11RenderTargetView {
        self.color_rtv()
    }

    fn depth_dsv(&self) -> *mut winapi::um::d3d11::ID3D11DepthStencilView {
        self.depth_dsv()
    }
}

#[cfg(windows)]
#[derive(Clone, Copy)]
pub struct NativeXmap16RailTargetView<'a> {
    pub target: &'a dyn NativeXmap16MainSceneTargetView,
    pub target_sha256: &'a str,
}

#[cfg(windows)]
impl NativeXmap16RailTargetView<'_> {
    pub fn identity(&self) -> Result<Xmap16RailTargetIdentity> {
        Xmap16RailTargetIdentity::from_desc(self.target_sha256, self.target.desc())
    }
}

#[cfg(windows)]
pub unsafe trait NativeXmap16RailSceneConstantBufferView {
    fn symbol(&self) -> &str;
    fn owner_packet(&self) -> &str;
    fn shader_register(&self) -> u32;
    fn byte_len(&self) -> u32;
    fn payload_sha256(&self) -> &str;
    fn descriptor_sha256(&self) -> &str;
    fn composition_receipt_sha256(&self) -> &str;
    fn reflected_layout_sha256(&self) -> &str;
    fn camera_view_sha256(&self) -> &str;
    fn snow_mud_input_sha256(&self) -> &str;
    fn date(&self) -> &str;
    fn hour_index(&self) -> i64;
    fn map_width(&self) -> u32;
    fn map_height(&self) -> u32;
    fn viewport_width(&self) -> u32;
    fn viewport_height(&self) -> u32;
    fn map_copy_offset_f32_bits(&self) -> u32;
    fn raw_buffer(&self) -> *mut winapi::um::d3d11::ID3D11Buffer;
}

#[cfg(windows)]
unsafe impl NativeXmap16RailSceneConstantBufferView for crate::NativeXmap10MapCopySceneCb0View<'_> {
    fn symbol(&self) -> &str {
        self.symbol()
    }

    fn owner_packet(&self) -> &str {
        self.owner_packet()
    }

    fn shader_register(&self) -> u32 {
        self.shader_register()
    }

    fn byte_len(&self) -> u32 {
        self.byte_len()
    }

    fn payload_sha256(&self) -> &str {
        self.payload_sha256()
    }

    fn descriptor_sha256(&self) -> &str {
        self.descriptor_sha256()
    }

    fn composition_receipt_sha256(&self) -> &str {
        self.composition_receipt_sha256()
    }

    fn reflected_layout_sha256(&self) -> &str {
        self.reflected_layout_sha256()
    }

    fn camera_view_sha256(&self) -> &str {
        self.camera_frame_sha256()
    }

    fn snow_mud_input_sha256(&self) -> &str {
        self.snow_mud_input_sha256()
    }

    fn date(&self) -> &str {
        self.date()
    }

    fn hour_index(&self) -> i64 {
        self.hour_index()
    }

    fn map_width(&self) -> u32 {
        self.map_width()
    }

    fn map_height(&self) -> u32 {
        self.map_height()
    }

    fn viewport_width(&self) -> u32 {
        self.viewport_width()
    }

    fn viewport_height(&self) -> u32 {
        self.viewport_height()
    }

    fn map_copy_offset_f32_bits(&self) -> u32 {
        self.map_copy_offset_f32_bits()
    }

    fn raw_buffer(&self) -> *mut winapi::um::d3d11::ID3D11Buffer {
        self.raw_buffer()
    }
}

#[cfg(windows)]
pub struct NativeXmap16RailDrawInputs<'a> {
    pub context: *mut winapi::um::d3d11::ID3D11DeviceContext,
    pub target: NativeXmap16RailTargetView<'a>,
    pub assignment: &'a portable_render_stream::AssignedStage,
    pub shared: &'a Xmap16RailSharedExecutionIdentity,
    pub foundation: &'a portable_render_stream::FoundationRenderEnvelope,
    pub viewer: &'a crate::ViewerOwnedMapState,
    pub scene_cb0: &'a dyn NativeXmap16RailSceneConstantBufferView,
    pub external_resources: &'a [NativeXmap16RailExternalResourceView],
}

#[cfg(windows)]
pub struct NativeXmap16RailHandoff {
    vertex_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11VertexShader>,
    pixel_shader: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11PixelShader>,
    input_layout: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11InputLayout>,
    vertex_buffer: Option<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>>,
    _rail_texture: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    rail_texture_view: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    samplers: Vec<(
        u32,
        crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>,
    )>,
    cb1: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>,
    rasterizer: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>,
    blend: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>,
    depth_stencil: crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>,
    receipt: Xmap16RailHandoffReceipt,
}

#[cfg(windows)]
impl NativeXmap16RailHandoff {
    pub unsafe fn create_on_device(
        device: *mut winapi::um::d3d11::ID3D11Device,
        resolver: &crate::GameFileResolver,
        frame: &PreparedXmap16Frame,
    ) -> Result<Self> {
        use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
        use std::ffi::CString;
        use std::ptr;
        use winapi::um::d3d11::{
            ID3D11InputLayout, ID3D11PixelShader, ID3D11VertexShader, D3D11_INPUT_ELEMENT_DESC,
        };

        frame.validate()?;
        if device.is_null() {
            return Err("XMAP16 railroad handoff received a null D3D11 device".into());
        }
        let mut raw_vs = ptr::null_mut::<ID3D11VertexShader>();
        native_d3d11_hr(
            (*device).CreateVertexShader(
                frame.program.vertex_bytecode.as_ptr() as *const _,
                frame.program.vertex_bytecode.len(),
                ptr::null_mut(),
                &mut raw_vs,
            ),
            "CreateVertexShader(XMAP16 railroad)",
        )?;
        let vertex_shader = D3dPtr::from_raw(raw_vs, "XMAP16 railroad vertex shader")?;
        let mut raw_ps = ptr::null_mut::<ID3D11PixelShader>();
        native_d3d11_hr(
            (*device).CreatePixelShader(
                frame.program.pixel_bytecode.as_ptr() as *const _,
                frame.program.pixel_bytecode.len(),
                ptr::null_mut(),
                &mut raw_ps,
            ),
            "CreatePixelShader(XMAP16 railroad)",
        )?;
        let pixel_shader = D3dPtr::from_raw(raw_ps, "XMAP16 railroad pixel shader")?;
        let position = CString::new("POSITION")?;
        let texcoord = CString::new("TEXCOORD")?;
        let elements = [
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: position.as_ptr(),
                SemanticIndex: 0,
                Format: winapi::shared::dxgiformat::DXGI_FORMAT_R32G32B32A32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: winapi::um::d3d11::D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: texcoord.as_ptr(),
                SemanticIndex: 0,
                Format: winapi::shared::dxgiformat::DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 16,
                InputSlotClass: winapi::um::d3d11::D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
        ];
        let mut raw_layout = ptr::null_mut::<ID3D11InputLayout>();
        native_d3d11_hr(
            (*device).CreateInputLayout(
                elements.as_ptr(),
                elements.len() as u32,
                frame.program.vertex_bytecode.as_ptr() as *const _,
                frame.program.vertex_bytecode.len(),
                &mut raw_layout,
            ),
            "CreateInputLayout(XMAP16 railroad)",
        )?;
        let input_layout = D3dPtr::from_raw(raw_layout, "XMAP16 railroad input layout")?;
        let vertex_bytes = ribbon_vertex_bytes(frame)?;
        let vertex_buffer = create_vertex_buffer(device, &vertex_bytes)?;
        let (rail_texture, rail_texture_view, rail_texture_descriptor_sha256) =
            create_rail_texture(device, resolver, frame)?;
        let samplers = create_samplers(device, frame)?;
        let cb1 = create_cb1(device, frame.rail.railway_alpha)?;
        let rasterizer = create_rail_rasterizer_state(device)?;
        let blend = create_rail_blend_state(device)?;
        let depth_stencil = create_rail_depth_stencil_state(device)?;
        let receipt = handoff_receipt(frame, &vertex_bytes, &rail_texture_descriptor_sha256)?;
        Ok(Self {
            vertex_shader,
            pixel_shader,
            input_layout,
            vertex_buffer,
            _rail_texture: rail_texture,
            rail_texture_view,
            samplers,
            cb1,
            rasterizer,
            blend,
            depth_stencil,
            receipt,
        })
    }

    pub unsafe fn update_frame(
        &mut self,
        device: *mut winapi::um::d3d11::ID3D11Device,
        frame: &PreparedXmap16Frame,
    ) -> Result<&Xmap16RailHandoffReceipt> {
        frame.validate()?;
        if frame.program.deterministic_sha256 != self.receipt.program_sha256
            || frame.rail.railroad_texture.sha256 != self.receipt.rail_texture_sha256
        {
            return Err(
                "XMAP16 railroad program/texture changed; create a fresh native handoff".into(),
            );
        }
        if frame.deterministic_sha256 == self.receipt.frame_sha256 {
            return Ok(&self.receipt);
        }
        let bytes = ribbon_vertex_bytes(frame)?;
        self.vertex_buffer = create_vertex_buffer(device, &bytes)?;
        self.cb1 = create_cb1(device, frame.rail.railway_alpha)?;
        self.receipt =
            handoff_receipt(frame, &bytes, &self.receipt.rail_texture_descriptor_sha256)?;
        Ok(&self.receipt)
    }

    pub fn receipt(&self) -> &Xmap16RailHandoffReceipt {
        &self.receipt
    }

    pub unsafe fn draw_target_bound(
        &self,
        frame: &PreparedXmap16Frame,
        inputs: NativeXmap16RailDrawInputs<'_>,
    ) -> Result<Xmap16RailExecutionReceipt> {
        use std::ptr;
        use winapi::shared::dxgiformat::DXGI_FORMAT_UNKNOWN;
        use winapi::um::d3d11::D3D11_VIEWPORT;
        use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP;

        let target_identity = self.validate_common_draw_inputs(frame, &inputs)?;
        if !frame.rail.railway_draw_enabled {
            let closure = seal_component_closure(
                inputs.assignment,
                frame,
                inputs.shared,
                &self.receipt.deterministic_sha256,
                Xmap16RailComponentDisposition::NotApplicable {
                    applicability_witness_sha256: alpha_not_applicable_witness(
                        frame,
                        &self.receipt.deterministic_sha256,
                    ),
                    reason: "railway_alpha_disabled".into(),
                },
            )?;
            let execution = Xmap16RailExecutionReceipt {
                packet: None,
                closure,
            };
            execution.validate()?;
            return Ok(execution);
        }
        if frame.rail.ribbon_program.ordered_draws.is_empty() {
            let closure = seal_component_closure(
                inputs.assignment,
                frame,
                inputs.shared,
                &self.receipt.deterministic_sha256,
                Xmap16RailComponentDisposition::AuthenticatedEmpty {
                    enumerator_witness_sha256: empty_enumerator_witness(frame),
                },
            )?;
            let execution = Xmap16RailExecutionReceipt {
                packet: None,
                closure,
            };
            execution.validate()?;
            return Ok(execution);
        }
        let vertex_buffer = self
            .vertex_buffer
            .as_ref()
            .ok_or("XMAP16 enabled railroad frame has no vertex buffer")?;
        let external_identities = validate_external_views(inputs.external_resources)?;
        let context = inputs.context;
        let target = inputs.target.target;
        let rtv = [target.color_rtv()];
        (*context).OMSetRenderTargets(1, rtv.as_ptr(), target.depth_dsv());
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: target_identity.color_width as f32,
            Height: target_identity.color_height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        (*context).RSSetViewports(1, &viewport);
        (*context).RSSetState(self.rasterizer.as_raw());
        (*context).OMSetBlendState(self.blend.as_raw(), &[0.0; 4], u32::MAX);
        (*context).OMSetDepthStencilState(self.depth_stencil.as_raw(), 0);
        (*context).VSSetShader(self.vertex_shader.as_raw(), ptr::null(), 0);
        (*context).PSSetShader(self.pixel_shader.as_raw(), ptr::null(), 0);
        (*context).GSSetShader(ptr::null_mut(), ptr::null(), 0);
        (*context).HSSetShader(ptr::null_mut(), ptr::null(), 0);
        (*context).DSSetShader(ptr::null_mut(), ptr::null(), 0);
        (*context).CSSetShader(ptr::null_mut(), ptr::null(), 0);
        (*context).IASetInputLayout(self.input_layout.as_raw());
        (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        let buffers = [vertex_buffer.as_raw()];
        let strides = [crate::A729_RAILROAD_VERTEX_STRIDE_BYTES];
        let offsets = [0];
        (*context).IASetVertexBuffers(0, 1, buffers.as_ptr(), strides.as_ptr(), offsets.as_ptr());
        (*context).IASetIndexBuffer(ptr::null_mut(), DXGI_FORMAT_UNKNOWN, 0);
        let scene_cb0 = [inputs.scene_cb0.raw_buffer()];
        (*context).VSSetConstantBuffers(0, 1, scene_cb0.as_ptr());
        (*context).PSSetConstantBuffers(0, 1, scene_cb0.as_ptr());
        let cb1 = [self.cb1.as_raw()];
        (*context).PSSetConstantBuffers(1, 1, cb1.as_ptr());
        let rail_texture = [self.rail_texture_view.as_raw()];
        (*context).PSSetShaderResources(0, 1, rail_texture.as_ptr());
        for resource in inputs.external_resources {
            let view = [resource.as_raw()];
            (*context).PSSetShaderResources(resource.identity.shader_register, 1, view.as_ptr());
        }
        for (slot, sampler) in &self.samplers {
            let raw = [sampler.as_raw()];
            (*context).PSSetSamplers(*slot, 1, raw.as_ptr());
        }

        let mut ordered_actual_draw_receipt_sha256s =
            Vec::with_capacity(frame.rail.ribbon_program.ordered_draws.len());
        for (draw_ordinal, draw) in frame.rail.ribbon_program.ordered_draws.iter().enumerate() {
            (*context).Draw(draw.vertex_count, draw.first_vertex);
            ordered_actual_draw_receipt_sha256s.push(actual_draw_receipt_sha256(
                frame,
                &self.receipt,
                inputs.assignment,
                &target_identity,
                inputs.scene_cb0.payload_sha256(),
                &external_identities,
                draw,
                u32::try_from(draw_ordinal)?,
            )?);
        }

        // Release borrowed packet resources from the immediate context before
        let null_srv = [ptr::null_mut()];
        (*context).PSSetShaderResources(0, 1, null_srv.as_ptr());
        for (_, _, slot) in XMAP16_EXTERNAL_PIXEL_RESOURCES {
            (*context).PSSetShaderResources(slot, 1, null_srv.as_ptr());
        }
        let null_cb = [ptr::null_mut()];
        (*context).VSSetConstantBuffers(0, 1, null_cb.as_ptr());
        (*context).PSSetConstantBuffers(0, 1, null_cb.as_ptr());
        (*context).PSSetConstantBuffers(1, 1, null_cb.as_ptr());
        (*context).VSSetShader(ptr::null_mut(), ptr::null(), 0);
        (*context).PSSetShader(ptr::null_mut(), ptr::null(), 0);

        let packet = seal_present_packet(
            inputs.assignment,
            frame,
            inputs.shared,
            &self.receipt,
            ordered_actual_draw_receipt_sha256s,
        )?;
        let closure = seal_component_closure(
            inputs.assignment,
            frame,
            inputs.shared,
            &self.receipt.deterministic_sha256,
            Xmap16RailComponentDisposition::Present {
                packet_draw_receipt_sha256: packet.packet_draw_receipt_sha256.clone(),
            },
        )?;
        let execution = Xmap16RailExecutionReceipt {
            packet: Some(packet),
            closure,
        };
        execution.validate()?;
        Ok(execution)
    }

    fn validate_common_draw_inputs(
        &self,
        frame: &PreparedXmap16Frame,
        inputs: &NativeXmap16RailDrawInputs<'_>,
    ) -> Result<Xmap16RailTargetIdentity> {
        frame.validate()?;
        self.receipt.validate()?;
        validate_assigned_stage(inputs.assignment)?;
        inputs.shared.validate()?;
        inputs
            .foundation
            .validate()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        inputs.viewer.validate()?;
        if inputs.context.is_null() {
            return Err("XMAP16 railroad draw received a null D3D11 context".into());
        }
        if frame.deterministic_sha256 != self.receipt.frame_sha256
            || frame.program.deterministic_sha256 != self.receipt.program_sha256
            || frame.rail.railroad_texture.sha256 != self.receipt.rail_texture_sha256
            || inputs.foundation.hour_index != frame.hour_index
            || inputs.foundation.content_manifest_sha256 != inputs.shared.content_manifest_sha256
            || inputs.foundation.foundation_content_operation_sha256
                != inputs.shared.foundation_content_operation_sha256
            || Sha256::hex(&serde_json::to_vec(inputs.viewer)?) != inputs.shared.viewer_state_sha256
        {
            return Err("XMAP16 railroad draw identities disagree".into());
        }
        let family = inputs
            .foundation
            .families
            .iter()
            .find(|row| row.family == portable_render_stream::MapVisualFamily::RailSupply)
            .ok_or("XMAP16 railroad draw has no RailSupply foundation family")?;
        if !matches!(
            &family.disposition,
            portable_render_stream::FamilyDisposition::Present { payload_sha256, .. }
                if payload_sha256 == &frame.deterministic_sha256
        ) {
            return Err("XMAP16 railroad frame is not the admitted foundation payload".into());
        }
        let target_identity = inputs.target.identity()?;
        if target_identity.target_sha256 != inputs.shared.main_scene_target_sha256
            || inputs.target.target.color_rtv().is_null()
            || inputs.target.target.depth_dsv().is_null()
            || target_identity.color_width != inputs.viewer.viewport_width
            || target_identity.color_height != inputs.viewer.viewport_height
        {
            return Err("XMAP16 railroad draw target differs from the shared MainScene".into());
        }
        let camera = inputs
            .viewer
            .camera
            .as_ref()
            .ok_or("XMAP16 railroad draw requires a viewer-owned camera")?;
        let scene = inputs.scene_cb0;
        let map_copy_offset = f32::from_bits(scene.map_copy_offset_f32_bits());
        if scene.symbol() != "SceneCB0"
            || scene.owner_packet() != "XMAP10"
            || scene.shader_register() != 0
            || scene.byte_len() != 416
            || scene.raw_buffer().is_null()
            || scene.date().trim().is_empty()
            || scene.hour_index() != frame.hour_index
            || scene.map_width() == 0
            || scene.map_height() == 0
            || scene.viewport_width() != inputs.viewer.viewport_width
            || scene.viewport_height() != inputs.viewer.viewport_height
            || scene.camera_view_sha256() != camera.deterministic_sha256
            || !map_copy_offset.is_finite()
        {
            return Err("XMAP16 railroad SceneCB0 does not match the target-bound frame".into());
        }
        for (label, value) in [
            ("SceneCB0 payload", scene.payload_sha256()),
            ("SceneCB0 descriptor", scene.descriptor_sha256()),
            ("SceneCB0 composition", scene.composition_receipt_sha256()),
            ("SceneCB0 reflected layout", scene.reflected_layout_sha256()),
            ("SceneCB0 camera", scene.camera_view_sha256()),
            ("SceneCB0 snow/mud input", scene.snow_mud_input_sha256()),
        ] {
            validate_sha256(value, label)?;
        }
        Ok(target_identity)
    }
}

#[cfg(windows)]
fn validate_external_views(
    resources: &[NativeXmap16RailExternalResourceView],
) -> Result<Vec<Xmap16RailExternalResourceIdentity>> {
    if resources.len() != XMAP16_EXTERNAL_PIXEL_RESOURCES.len() {
        return Err("XMAP16 railroad draw external resource count is wrong".into());
    }
    let mut result = Vec::with_capacity(resources.len());
    for (resource, expected) in resources.iter().zip(XMAP16_EXTERNAL_PIXEL_RESOURCES) {
        resource.identity.validate()?;
        if resource.as_raw().is_null()
            || (
                resource.identity.symbol.as_str(),
                resource.identity.owner_packet.as_str(),
                resource.identity.shader_register,
            ) != expected
        {
            return Err("XMAP16 railroad draw external resource order is wrong".into());
        }
        result.push(resource.identity.clone());
    }
    Ok(result)
}

#[cfg(windows)]
unsafe fn create_vertex_buffer(
    device: *mut winapi::um::d3d11::ID3D11Device,
    bytes: &[u8],
) -> Result<Option<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11Buffer, D3D11_BIND_VERTEX_BUFFER, D3D11_BUFFER_DESC, D3D11_SUBRESOURCE_DATA,
        D3D11_USAGE_DEFAULT,
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: u32::try_from(bytes.len())?,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_VERTEX_BUFFER,
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
        (*device).CreateBuffer(&desc, &initial, &mut raw),
        "CreateBuffer(XMAP16 railroad ribbon)",
    )?;
    Ok(Some(D3dPtr::from_raw(raw, "XMAP16 railroad ribbon")?))
}

#[cfg(windows)]
unsafe fn create_cb1(
    device: *mut winapi::um::d3d11::ID3D11Device,
    railway_alpha: f32,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Buffer>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11Buffer, D3D11_BIND_CONSTANT_BUFFER, D3D11_BUFFER_DESC, D3D11_SUBRESOURCE_DATA,
        D3D11_USAGE_DEFAULT,
    };
    let mut bytes = [0u8; 80];
    bytes[68..72].copy_from_slice(&railway_alpha.to_bits().to_le_bytes());
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: bytes.len() as u32,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER,
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
        (*device).CreateBuffer(&desc, &initial, &mut raw),
        "CreateBuffer(XMAP16 railroad CB1)",
    )?;
    D3dPtr::from_raw(raw, "XMAP16 railroad CB1")
}

#[cfg(windows)]
unsafe fn create_rail_texture(
    device: *mut winapi::um::d3d11::ID3D11Device,
    resolver: &crate::GameFileResolver,
    frame: &PreparedXmap16Frame,
) -> Result<(
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11Texture2D>,
    crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11ShaderResourceView>,
    String,
)> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::um::d3d11::{
        ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    let content = frame
        .ordered_used_content
        .iter()
        .find(|content| content.normalized_game_relative_path == frame.rail.railroad_texture.source)
        .ok_or("XMAP16 used-content closure omitted railroad.dds")?;
    let bytes = resolver.read_admitted_used_content(content)?;
    if Sha256::hex(&bytes) != frame.rail.railroad_texture.sha256
        || bytes.len() as u64 != frame.rail.railroad_texture.byte_len
    {
        return Err("XMAP16 railroad.dds differs from its admitted payload identity".into());
    }
    let descriptor = parse_rail_dds_descriptor(&bytes, &frame.rail.railroad_texture.sha256)?;
    let mut initial = Vec::with_capacity(descriptor.mips.len());
    for mip in &descriptor.mips {
        let offset = usize::try_from(mip.source_offset)?;
        let end = offset
            .checked_add(mip.slice_pitch as usize)
            .ok_or("XMAP16 railroad DDS range overflow")?;
        let payload = bytes
            .get(offset..end)
            .ok_or("railroad DDS mip payload is truncated")?;
        initial.push(D3D11_SUBRESOURCE_DATA {
            pSysMem: payload.as_ptr() as *const _,
            SysMemPitch: mip.row_pitch,
            SysMemSlicePitch: mip.slice_pitch,
        });
    }
    let desc = D3D11_TEXTURE2D_DESC {
        Width: descriptor.width,
        Height: descriptor.height,
        MipLevels: descriptor.mip_levels,
        ArraySize: 1,
        Format: descriptor.dxgi_format,
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
        (*device).CreateTexture2D(&desc, initial.as_ptr(), &mut raw_texture),
        "CreateTexture2D(XMAP16 railroad.dds)",
    )?;
    let texture = D3dPtr::from_raw(raw_texture, "XMAP16 railroad texture")?;
    let mut raw_view = ptr::null_mut::<ID3D11ShaderResourceView>();
    native_d3d11_hr(
        (*device).CreateShaderResourceView(texture.as_raw() as *mut _, ptr::null(), &mut raw_view),
        "CreateShaderResourceView(XMAP16 railroad.dds)",
    )?;
    Ok((
        texture,
        D3dPtr::from_raw(raw_view, "XMAP16 railroad texture SRV")?,
        rail_texture_descriptor_sha256(&descriptor),
    ))
}

fn read_dds_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or("XMAP16 railroad DDS header is truncated")?
            .try_into()?,
    ))
}

fn parse_rail_dds_descriptor(
    bytes: &[u8],
    admitted_payload_sha256: &str,
) -> Result<Xmap16RailDdsDescriptor> {
    if bytes.len() < 128 || bytes.get(..4) != Some(b"DDS ") {
        return Err("XMAP16 railroad texture is not a complete DDS".into());
    }
    if read_dds_u32(bytes, 4)? != 124 || read_dds_u32(bytes, 76)? != 32 {
        return Err("XMAP16 railroad DDS has noncanonical header sizes".into());
    }
    let width = read_dds_u32(bytes, 16)?;
    let height = read_dds_u32(bytes, 12)?;
    let mip_levels = read_dds_u32(bytes, 28)?.max(1);
    if width == 0
        || height == 0
        || width > 16_384
        || height > 16_384
        || read_dds_u32(bytes, 24)?.max(1) != 1
    {
        return Err("XMAP16 railroad DDS is not a nonempty Texture2D".into());
    }
    let maximum_mip_levels = u32::BITS - width.max(height).leading_zeros();
    if mip_levels > maximum_mip_levels {
        return Err(format!(
            "XMAP16 railroad DDS declares {mip_levels} mips but {width}x{height} admits at most {maximum_mip_levels}"
        )
        .into());
    }
    if read_dds_u32(bytes, 112)? & (0x0000_fe00 | 0x0020_0000) != 0 {
        return Err("XMAP16 railroad DDS is a cube or volume texture".into());
    }

    let pixel_flags = read_dds_u32(bytes, 80)?;
    let fourcc = bytes
        .get(84..88)
        .ok_or("XMAP16 railroad DDS pixel format is truncated")?;
    let (dxgi_format, block_bytes, bytes_per_pixel, payload_offset) = if pixel_flags & 0x4 != 0
        && fourcc == b"DX10"
    {
        if bytes.len() < 148 {
            return Err("XMAP16 railroad DX10 DDS header is truncated".into());
        }
        if read_dds_u32(bytes, 132)? != 3
            || read_dds_u32(bytes, 140)? != 1
            || read_dds_u32(bytes, 136)? & 0x4 != 0
        {
            return Err("XMAP16 railroad DDS is not one non-cube Texture2D".into());
        }
        let format = read_dds_u32(bytes, 128)?;
        let (block, pixel) = match format {
            71 | 72 => (8, 0),
            74 | 75 | 77 | 78 => (16, 0),
            28 | 29 | 87 | 91 => (0, 4),
            other => {
                return Err(format!(
                    "XMAP16 railroad DDS DXGI format {other} has no proven exact upload layout"
                )
                .into())
            }
        };
        (format, block, pixel, 148usize)
    } else if pixel_flags & 0x4 != 0 {
        match fourcc {
            b"DXT1" => (71, 8, 0, 128),
            b"DXT3" => (74, 16, 0, 128),
            b"DXT5" => (77, 16, 0, 128),
            _ => {
                return Err(format!(
                    "XMAP16 railroad DDS fourcc {:?} is unsupported",
                    String::from_utf8_lossy(fourcc)
                )
                .into())
            }
        }
    } else if pixel_flags & 0x40 != 0 {
        let bits = read_dds_u32(bytes, 88)?;
        let masks = (
            read_dds_u32(bytes, 92)?,
            read_dds_u32(bytes, 96)?,
            read_dds_u32(bytes, 100)?,
            read_dds_u32(bytes, 104)?,
        );
        let format = match (bits, masks) {
            (32, (0x0000_00ff, 0x0000_ff00, 0x00ff_0000, 0xff00_0000)) => 28,
            (32, (0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000)) => 87,
            _ => {
                return Err(format!(
                    "XMAP16 railroad DDS RGB layout {bits}/{masks:08x?} is unsupported"
                )
                .into())
            }
        };
        (format, 0, 4, 128usize)
    } else {
        return Err(
            format!("XMAP16 railroad DDS pixel flags {pixel_flags:#010x} are unsupported").into(),
        );
    };

    let actual_payload_sha256 = Sha256::hex(bytes);
    if actual_payload_sha256 != admitted_payload_sha256 {
        return Err("XMAP16 railroad DDS differs from its admitted payload identity".into());
    }
    let mut offset = payload_offset;
    let mut mips = Vec::with_capacity(mip_levels as usize);
    for mip in 0..mip_levels {
        let mip_width = (width >> mip).max(1);
        let mip_height = (height >> mip).max(1);
        let (row_pitch, slice_pitch) = if block_bytes != 0 {
            let row = mip_width
                .div_ceil(4)
                .checked_mul(block_bytes)
                .ok_or("XMAP16 railroad DDS row pitch overflow")?;
            let rows = mip_height.div_ceil(4);
            (
                row,
                row.checked_mul(rows)
                    .ok_or("XMAP16 railroad DDS slice pitch overflow")?,
            )
        } else {
            let row = mip_width
                .checked_mul(bytes_per_pixel)
                .ok_or("XMAP16 railroad DDS row pitch overflow")?;
            (
                row,
                row.checked_mul(mip_height)
                    .ok_or("XMAP16 railroad DDS slice pitch overflow")?,
            )
        };
        let end = offset
            .checked_add(slice_pitch as usize)
            .ok_or("XMAP16 railroad DDS payload range overflow")?;
        bytes
            .get(offset..end)
            .ok_or_else(|| format!("XMAP16 railroad DDS mip {mip} is truncated"))?;
        mips.push(Xmap16RailDdsMipDescriptor {
            width: mip_width,
            height: mip_height,
            source_offset: offset as u64,
            row_pitch,
            slice_pitch,
        });
        offset = end;
    }
    if offset != bytes.len() {
        return Err(format!(
            "XMAP16 railroad DDS has {} trailing bytes outside its mip chain",
            bytes.len() - offset
        )
        .into());
    }
    Ok(Xmap16RailDdsDescriptor {
        width,
        height,
        mip_levels,
        dxgi_format,
        payload_sha256: actual_payload_sha256,
        mips,
    })
}

#[cfg(windows)]
unsafe fn create_samplers(
    device: *mut winapi::um::d3d11::ID3D11Device,
    frame: &PreparedXmap16Frame,
) -> Result<
    Vec<(
        u32,
        crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11SamplerState>,
    )>,
> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::um::d3d11::{
        ID3D11SamplerState, D3D11_COMPARISON_NEVER, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        D3D11_SAMPLER_DESC, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE_ADDRESS_WRAP,
    };
    frame
        .program
        .samplers
        .iter()
        .map(|sampler| {
            if sampler.mag_filter != "Linear"
                || sampler.min_filter != "Linear"
                || sampler.mip_filter != "Linear"
            {
                return Err(format!(
                    "XMAP16 sampler {} requires an unsupported effective filter",
                    sampler.symbol
                )
                .into());
            }
            let address = |value: &str| -> Result<u32> {
                match value {
                    "Wrap" => Ok(D3D11_TEXTURE_ADDRESS_WRAP),
                    "Clamp" => Ok(D3D11_TEXTURE_ADDRESS_CLAMP),
                    _ => Err(format!("XMAP16 sampler address mode {value} is unsupported").into()),
                }
            };
            let desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: address(&sampler.address_u)?,
                AddressV: address(&sampler.address_v)?,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MipLODBias: 0.0,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                BorderColor: [0.0; 4],
                MinLOD: f32::NEG_INFINITY,
                MaxLOD: f32::INFINITY,
            };
            let mut raw = ptr::null_mut::<ID3D11SamplerState>();
            native_d3d11_hr(
                (*device).CreateSamplerState(&desc, &mut raw),
                &format!("CreateSamplerState(XMAP16 {})", sampler.symbol),
            )?;
            Ok((
                sampler.shader_register,
                D3dPtr::from_raw(raw, "XMAP16 railroad sampler")?,
            ))
        })
        .collect()
}

#[cfg(windows)]
unsafe fn create_rail_rasterizer_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11RasterizerState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::minwindef::{FALSE, TRUE};
    use winapi::um::d3d11::{ID3D11RasterizerState, D3D11_RASTERIZER_DESC};

    let state = XMAP16_RAIL_PIPELINE_STATE;
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: state.raster_fill_mode,
        CullMode: state.raster_cull_mode,
        FrontCounterClockwise: if state.raster_front_counter_clockwise {
            TRUE
        } else {
            FALSE
        },
        DepthBias: state.raster_depth_bias,
        DepthBiasClamp: f32::from_bits(state.raster_depth_bias_clamp_bits),
        SlopeScaledDepthBias: f32::from_bits(state.raster_slope_scaled_depth_bias_bits),
        DepthClipEnable: if state.raster_depth_clip { TRUE } else { FALSE },
        ScissorEnable: if state.raster_scissor { TRUE } else { FALSE },
        MultisampleEnable: if state.raster_multisample {
            TRUE
        } else {
            FALSE
        },
        AntialiasedLineEnable: if state.raster_antialiased_line {
            TRUE
        } else {
            FALSE
        },
    };
    let mut raw = ptr::null_mut::<ID3D11RasterizerState>();
    native_d3d11_hr(
        (*device).CreateRasterizerState(&desc, &mut raw),
        "CreateRasterizerState(XMAP16 railroad default)",
    )?;
    D3dPtr::from_raw(raw, "XMAP16 railroad rasterizer")
}

#[cfg(windows)]
unsafe fn create_rail_blend_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11BlendState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::{mem, ptr};
    use winapi::shared::minwindef::{FALSE, TRUE};
    use winapi::um::d3d11::{ID3D11BlendState, D3D11_BLEND_DESC};

    let state = XMAP16_RAIL_PIPELINE_STATE;
    let mut desc: D3D11_BLEND_DESC = mem::zeroed();
    desc.AlphaToCoverageEnable = if state.blend_alpha_to_coverage {
        TRUE
    } else {
        FALSE
    };
    desc.IndependentBlendEnable = if state.blend_independent { TRUE } else { FALSE };
    desc.RenderTarget[0].BlendEnable = if state.blend_enabled { TRUE } else { FALSE };
    desc.RenderTarget[0].SrcBlend = state.blend_source;
    desc.RenderTarget[0].DestBlend = state.blend_destination;
    desc.RenderTarget[0].BlendOp = state.blend_operation;
    desc.RenderTarget[0].SrcBlendAlpha = state.blend_source_alpha;
    desc.RenderTarget[0].DestBlendAlpha = state.blend_destination_alpha;
    desc.RenderTarget[0].BlendOpAlpha = state.blend_alpha_operation;
    desc.RenderTarget[0].RenderTargetWriteMask = state.blend_write_mask;
    let mut raw = ptr::null_mut::<ID3D11BlendState>();
    native_d3d11_hr(
        (*device).CreateBlendState(&desc, &mut raw),
        "CreateBlendState(XMAP16 railroad BlendState)",
    )?;
    D3dPtr::from_raw(raw, "XMAP16 railroad blend")
}

#[cfg(windows)]
unsafe fn create_rail_depth_stencil_state(
    device: *mut winapi::um::d3d11::ID3D11Device,
) -> Result<crate::d3d11_device::D3dPtr<winapi::um::d3d11::ID3D11DepthStencilState>> {
    use crate::d3d11_device::{native_d3d11_hr, D3dPtr};
    use std::ptr;
    use winapi::shared::minwindef::{FALSE, TRUE};
    use winapi::um::d3d11::{
        ID3D11DepthStencilState, D3D11_DEPTH_STENCILOP_DESC, D3D11_DEPTH_STENCIL_DESC,
    };

    let state = XMAP16_RAIL_PIPELINE_STATE;
    let face = D3D11_DEPTH_STENCILOP_DESC {
        StencilFailOp: state.stencil_fail,
        StencilDepthFailOp: state.stencil_depth_fail,
        StencilPassOp: state.stencil_pass,
        StencilFunc: state.stencil_comparison,
    };
    let desc = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: if state.depth_enabled { TRUE } else { FALSE },
        DepthWriteMask: state.depth_write_mask,
        DepthFunc: state.depth_comparison,
        StencilEnable: if state.stencil_enabled { TRUE } else { FALSE },
        StencilReadMask: state.stencil_read_mask,
        StencilWriteMask: state.stencil_write_mask,
        FrontFace: face,
        BackFace: face,
    };
    let mut raw = ptr::null_mut::<ID3D11DepthStencilState>();
    native_d3d11_hr(
        (*device).CreateDepthStencilState(&desc, &mut raw),
        "CreateDepthStencilState(XMAP16 railroad DepthStencilState)",
    )?;
    D3dPtr::from_raw(raw, "XMAP16 railroad depth-stencil")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: char) -> String {
        std::iter::repeat(byte).take(64).collect()
    }

    fn legacy_rail_dds(
        width: u32,
        height: u32,
        mip_levels: u32,
        fourcc: Option<[u8; 4]>,
    ) -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[28..32].copy_from_slice(&mip_levels.to_le_bytes());
        bytes[76..80].copy_from_slice(&32u32.to_le_bytes());
        let block_bytes = match fourcc {
            Some(code) => {
                bytes[80..84].copy_from_slice(&0x4u32.to_le_bytes());
                bytes[84..88].copy_from_slice(&code);
                match &code {
                    b"DXT1" => 8,
                    b"DXT3" | b"DXT5" => 16,
                    _ => 0,
                }
            }
            None => {
                bytes[80..84].copy_from_slice(&0x40u32.to_le_bytes());
                bytes[88..92].copy_from_slice(&32u32.to_le_bytes());
                bytes[92..96].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
                bytes[96..100].copy_from_slice(&0x0000_ff00u32.to_le_bytes());
                bytes[100..104].copy_from_slice(&0x0000_00ffu32.to_le_bytes());
                bytes[104..108].copy_from_slice(&0xff00_0000u32.to_le_bytes());
                0
            }
        };
        for mip in 0..mip_levels {
            let mip_width = (width >> mip).max(1);
            let mip_height = (height >> mip).max(1);
            let byte_len = if block_bytes == 0 {
                mip_width * mip_height * 4
            } else {
                mip_width.div_ceil(4) * mip_height.div_ceil(4) * block_bytes
            };
            bytes.resize(bytes.len() + byte_len as usize, mip as u8);
        }
        bytes
    }

    #[test]
    fn railroad_dds_descriptor_preserves_vanilla_layout_exactly() {
        let bytes = legacy_rail_dds(64, 320, 9, None);
        let descriptor = parse_rail_dds_descriptor(&bytes, &Sha256::hex(&bytes)).unwrap();
        assert_eq!(descriptor.width, 64);
        assert_eq!(descriptor.height, 320);
        assert_eq!(descriptor.mip_levels, 9);
        assert_eq!(descriptor.dxgi_format, 87);
        assert_eq!(descriptor.mips.len(), 9);
        assert_eq!(descriptor.mips[0].source_offset, 128);
        assert_eq!(descriptor.mips[0].row_pitch, 256);
        assert_eq!(descriptor.mips[0].slice_pitch, 81_920);
        assert_eq!(rail_texture_descriptor_sha256(&descriptor).len(), 64);
    }

    #[test]
    fn railroad_dds_descriptor_accepts_effective_mod_dimensions_and_compression() {
        let bytes = legacy_rail_dds(17, 9, 3, Some(*b"DXT5"));
        let descriptor = parse_rail_dds_descriptor(&bytes, &Sha256::hex(&bytes)).unwrap();
        assert_eq!(descriptor.width, 17);
        assert_eq!(descriptor.height, 9);
        assert_eq!(descriptor.mip_levels, 3);
        assert_eq!(descriptor.dxgi_format, 77);
        assert_eq!(
            descriptor
                .mips
                .iter()
                .map(|mip| (mip.width, mip.height, mip.row_pitch, mip.slice_pitch))
                .collect::<Vec<_>>(),
            vec![(17, 9, 80, 240), (8, 4, 32, 32), (4, 2, 16, 16)]
        );
    }

    #[test]
    fn railroad_dds_descriptor_fails_closed_on_unproven_or_unsealed_payloads() {
        let unsupported = legacy_rail_dds(16, 16, 1, Some(*b"ATI2"));
        assert!(
            parse_rail_dds_descriptor(&unsupported, &Sha256::hex(&unsupported))
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );

        let mut trailing = legacy_rail_dds(8, 8, 1, None);
        let admitted = Sha256::hex(&trailing);
        trailing.push(0);
        assert!(parse_rail_dds_descriptor(&trailing, &admitted).is_err());
    }

    #[test]
    fn reflected_external_resource_contract_is_frozen_and_unique() {
        assert_eq!(
            XMAP16_EXTERNAL_PIXEL_RESOURCES,
            [
                ("ProvinceSecondaryColorMap", "XMAP05", 6),
                ("SnowMudTexture", "XMAP07", 8),
                ("GradientBorderChannel1", "XMAP05", 12),
                ("GradientBorderChannel2", "XMAP05", 13),
                ("ShadowMap", "XMAP10", 15),
            ]
        );
        let slots = XMAP16_EXTERNAL_PIXEL_RESOURCES
            .iter()
            .map(|row| row.2)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(slots.len(), XMAP16_EXTERNAL_PIXEL_RESOURCES.len());
        assert_eq!(external_resource_contract_sha256().len(), 64);
    }

    #[test]
    fn railroad_pipeline_state_matches_exact_generic_and_named_states() {
        assert_eq!(
            XMAP16_RAIL_PIPELINE_STATE,
            Xmap16RailPipelineStateContract {
                raster_fill_mode: 3,
                raster_cull_mode: 3,
                raster_front_counter_clockwise: false,
                raster_depth_bias: 0,
                raster_depth_bias_clamp_bits: 0.0f32.to_bits(),
                raster_slope_scaled_depth_bias_bits: 0.0f32.to_bits(),
                raster_depth_clip: true,
                raster_scissor: false,
                raster_multisample: false,
                raster_antialiased_line: false,
                blend_alpha_to_coverage: false,
                blend_independent: false,
                blend_enabled: true,
                blend_source: 5,
                blend_destination: 6,
                blend_operation: 1,
                blend_source_alpha: 5,
                blend_destination_alpha: 6,
                blend_alpha_operation: 1,
                blend_write_mask: 0x07,
                depth_enabled: false,
                depth_write_mask: 1,
                depth_comparison: 2,
                stencil_enabled: false,
                stencil_read_mask: 0xff,
                stencil_write_mask: 0xff,
                stencil_fail: 1,
                stencil_depth_fail: 1,
                stencil_pass: 1,
                stencil_comparison: 8,
            }
        );
        assert_eq!(rail_pipeline_state_sha256().len(), 64);
    }

    #[test]
    fn target_identity_seals_the_exact_main_scene_descriptor() {
        let desc = crate::ProductionMainSceneTargetDesc {
            color_attachment_ordinal: 1,
            color_width: 1920,
            color_height: 1080,
            color_format: 10,
            color_samples: 1,
            color_quality: 0,
            depth_attachment_ordinal: 2,
            depth_width: 1920,
            depth_height: 1080,
            depth_format: 45,
            depth_samples: 1,
            depth_quality: 0,
        };
        let identity = Xmap16RailTargetIdentity::from_desc(hash('a'), &desc).unwrap();
        identity.validate().unwrap();
        assert_eq!(
            identity.descriptor_sha256,
            Sha256::hex(&serde_json::to_vec(&desc).unwrap())
        );
        let mut tampered = identity;
        tampered.color_width += 1;
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn target_bound_receipt_rejects_empty_duplicate_and_reordered_draws() {
        let mut receipt = Xmap16RailTargetBoundDrawReceipt {
            schema: XMAP16_RAIL_TARGET_BOUND_DRAW_SCHEMA.into(),
            operation_id: XMAP16_RAIL_OPERATION_ID.into(),
            stage_name: XMAP16_RAIL_STAGE_NAME.into(),
            topology_sha256: hash('a'),
            assigned_ordinal: 7,
            assignment_sha256: hash('b'),
            hour_index: 24,
            content_manifest_sha256: hash('c'),
            foundation_content_operation_sha256: hash('d'),
            viewer_state_sha256: hash('e'),
            main_scene_target_sha256: hash('f'),
            map_copy_receipt_set_sha256: hash('1'),
            prepared_frame_sha256: hash('2'),
            handoff_receipt_sha256: hash('3'),
            ordered_actual_draw_receipt_sha256s: vec![hash('4'), hash('5')],
            issued_draw_count: 2,
            issued_index_count: 0,
            diagnostic_disposition_sha256: hash('6'),
            packet_draw_receipt_sha256: String::new(),
        };
        receipt.packet_draw_receipt_sha256 = Sha256::hex(&serde_json::to_vec(&receipt).unwrap());
        receipt.validate().unwrap();

        let encoded = serde_json::to_string(&receipt).unwrap();
        let schema = encoded.find("\"schema\"").unwrap();
        let operation = encoded.find("\"operation_id\"").unwrap();
        let stage = encoded.find("\"stage_name\"").unwrap();
        let topology = encoded.find("\"topology_sha256\"").unwrap();
        assert!(schema < operation && operation < stage && stage < topology);

        let mut empty = receipt.clone();
        empty.ordered_actual_draw_receipt_sha256s.clear();
        empty.issued_draw_count = 0;
        empty.packet_draw_receipt_sha256 = Sha256::hex(&serde_json::to_vec(&empty).unwrap());
        assert!(empty.validate().is_err());

        let mut duplicate = receipt.clone();
        duplicate.ordered_actual_draw_receipt_sha256s[1] =
            duplicate.ordered_actual_draw_receipt_sha256s[0].clone();
        duplicate.packet_draw_receipt_sha256 =
            Sha256::hex(&serde_json::to_vec(&duplicate).unwrap());
        assert!(duplicate.validate().is_err());

        let mut reordered = receipt;
        reordered.ordered_actual_draw_receipt_sha256s.swap(0, 1);
        assert!(reordered.validate().is_err());
    }

    #[test]
    fn component_closure_never_treats_no_draw_as_present() {
        let mut closure = Xmap16RailComponentClosure {
            schema: XMAP16_RAIL_COMPONENT_CLOSURE_SCHEMA.into(),
            component_id: XMAP16_RAIL_COMPONENT_ID.into(),
            operation_id: XMAP16_RAIL_OPERATION_ID.into(),
            stage_name: XMAP16_RAIL_STAGE_NAME.into(),
            topology_sha256: hash('a'),
            assigned_ordinal: 7,
            assignment_sha256: hash('b'),
            hour_index: 24,
            content_manifest_sha256: hash('c'),
            foundation_content_operation_sha256: hash('d'),
            viewer_state_sha256: hash('e'),
            main_scene_target_sha256: hash('f'),
            map_copy_receipt_set_sha256: hash('1'),
            prepared_frame_sha256: hash('2'),
            handoff_receipt_sha256: hash('3'),
            disposition: Xmap16RailComponentDisposition::NotApplicable {
                applicability_witness_sha256: hash('4'),
                reason: "railway_alpha_disabled".into(),
            },
            closure_sha256: String::new(),
        };
        closure.closure_sha256 = Sha256::hex(&serde_json::to_vec(&closure).unwrap());
        closure.validate().unwrap();
        Xmap16RailExecutionReceipt {
            packet: None,
            closure: closure.clone(),
        }
        .validate()
        .unwrap();

        closure.disposition = Xmap16RailComponentDisposition::Present {
            packet_draw_receipt_sha256: hash('5'),
        };
        closure.closure_sha256.clear();
        closure.closure_sha256 = Sha256::hex(&serde_json::to_vec(&closure).unwrap());
        assert!(Xmap16RailExecutionReceipt {
            packet: None,
            closure
        }
        .validate()
        .is_err());
    }
}
