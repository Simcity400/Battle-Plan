
use crate::production_features::xmap10::{
    PreparedXmap10Frame, Xmap10CompiledProgramIdentity, Xmap10LutBlendPassKind, Xmap10LutInput,
    Xmap10NativeCompositorReceiptRef, Xmap10NativeProgramReceiptRef, Xmap10PassKind,
    Xmap10ProgramRole, Xmap10TargetPlan, Xmap10TargetRole, Xmap10TemporalAdaptationSource,
    XMAP10_NATIVE_COMPOSITOR_RECEIPT_SCHEMA, XMAP10_NATIVE_PROGRAM_RECEIPT_SCHEMA,
};
use crate::{GameFileResolver, Result, Xmap10PreParticleSceneReceipt};
use portable_render_stream::{AssignedStage, AssignedStageSchedule};
use serde::{Deserialize, Serialize};
use snapshot::sha256::Sha256;

pub const XMAP10_REDUCTION_PLAN_SCHEMA: &str = "hoi4.xmap10.luminance-reduction-plan.a729.v1";
pub const XMAP10_TEMPORAL_POLICY_SCHEMA: &str = "hoi4.xmap10.temporal-native-policy.a729.v1";
pub const XMAP10_POSTFX_FRAME_RECEIPT_SCHEMA: &str = "hoi4.xmap10.postfx-frame-receipt.a729.v1";
pub const XMAP10_TARGET_BOUND_PASS_RECEIPT_SCHEMA: &str =
    "hoi4.xmap10.target-bound-pass-receipt.a729.v1";
pub const XMAP10_LUMINANCE_DOWNSAMPLE_CHAIN_RECEIPT_SCHEMA: &str =
    "hoi4.xmap10.luminance-downsample-chain-receipt.a729.v1";
pub const XMAP10_LUMINANCE_DOWNSAMPLE_SUBPASS_RECEIPT_SCHEMA: &str =
    "hoi4.xmap10.luminance-downsample-subpass-receipt.a729.v1";
pub const XMAP10_LUT_EXECUTION_RECEIPT_SCHEMA: &str = "hoi4.xmap10.lut-execution-receipt.a729.v1";

const DXGI_FORMAT_R16G16B16A16_FLOAT_U32: u32 = 10;
const DXGI_FORMAT_R8G8B8A8_UNORM_U32: u32 = 28;
const DXGI_FORMAT_B8G8R8A8_UNORM_U32: u32 = 87;
pub const XMAP10_LUMINANCE_REDUCTION_EVIDENCE_SHA256: &str =
    "f2d8ae114eb89fe6ef80ea27c535d9b3b7e3e4361dc50e4f83c0b3ba3c79dca8";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10ReductionStep {
    pub ordinal: u8,
    pub source_width: u32,
    pub source_height: u32,
    pub destination_width: u32,
    pub destination_height: u32,
    pub source_is_bright_alpha: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10LuminanceReductionPlan {
    pub schema: String,
    pub steps: Vec<Xmap10ReductionStep>,
    pub evidence_receipt_sha256: String,
    pub deterministic_sha256: String,
}

impl Xmap10LuminanceReductionPlan {
    pub fn from_target_plan(targets: &Xmap10TargetPlan) -> Result<Self> {
        targets.validate()?;
        let main = targets
            .ordered_targets
            .iter()
            .find(|target| target.role == Xmap10TargetRole::MainScene)
            .ok_or("XMAP10 target plan omitted MainScene")?;
        let steps = targets
            .ordered_passes
            .iter()
            .filter_map(|pass| match pass.pass {
                Xmap10PassKind::LuminanceDownsample(ordinal) => Some((ordinal, pass)),
                _ => None,
            })
            .map(|(ordinal, pass)| {
                let source = pass
                    .read_extents
                    .first()
                    .ok_or("XMAP10 luminance pass omitted its source extent")?;
                let destination = pass
                    .viewport
                    .ok_or("XMAP10 luminance pass omitted its destination viewport")?;
                Ok(Xmap10ReductionStep {
                    ordinal: u8::try_from(ordinal)
                        .map_err(|_| "XMAP10 luminance ordinal exceeds u8")?,
                    source_width: source.width,
                    source_height: source.height,
                    destination_width: destination.width,
                    destination_height: destination.height,
                    source_is_bright_alpha: ordinal == 0,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Self::new(
            steps,
            XMAP10_LUMINANCE_REDUCTION_EVIDENCE_SHA256.into(),
            main.width,
            main.height,
        )
    }

    pub fn new(
        steps: Vec<Xmap10ReductionStep>,
        evidence_receipt_sha256: String,
        main_scene_width: u32,
        main_scene_height: u32,
    ) -> Result<Self> {
        let mut plan = Self {
            schema: XMAP10_REDUCTION_PLAN_SCHEMA.into(),
            steps,
            evidence_receipt_sha256,
            deterministic_sha256: String::new(),
        };
        plan.deterministic_sha256 = reduction_plan_digest(&plan);
        plan.validate_for_main_scene(main_scene_width, main_scene_height)?;
        Ok(plan)
    }

    pub fn validate_for_main_scene(&self, width: u32, height: u32) -> Result<()> {
        if self.schema != XMAP10_REDUCTION_PLAN_SCHEMA
            || self.steps.is_empty()
            || self.evidence_receipt_sha256 != XMAP10_LUMINANCE_REDUCTION_EVIDENCE_SHA256
            || self.deterministic_sha256 != reduction_plan_digest(self)
            || width < 4
            || height < 4
        {
            return Err("XMAP10 luminance reduction plan identity is invalid".into());
        }
        let first = self
            .steps
            .first()
            .ok_or("XMAP10 luminance reduction plan is empty")?;
        if first.ordinal != 0
            || !first.source_is_bright_alpha
            || (first.source_width, first.source_height) != (width / 2, height / 2)
            || (first.destination_width, first.destination_height)
                != (((width / 2) + 1) / 2, ((height / 2) + 1) / 2)
        {
            return Err(
                "XMAP10 first luminance reduction is not the bright-alpha quarter-scene pass"
                    .into(),
            );
        }
        for (index, step) in self.steps.iter().enumerate() {
            if usize::from(step.ordinal) != index
                || step.source_width == 0
                || step.source_height == 0
                || step.destination_width == 0
                || step.destination_height == 0
                || (index != 0 && step.source_is_bright_alpha)
                || step.destination_width != (step.source_width + 1) / 2
                || step.destination_height != (step.source_height + 1) / 2
            {
                return Err("XMAP10 luminance reduction step is malformed".into());
            }
            if index > 0 {
                let prior = &self.steps[index - 1];
                if (step.source_width, step.source_height)
                    != (prior.destination_width, prior.destination_height)
                {
                    return Err(
                        "XMAP10 luminance reduction viewports do not form a contiguous chain"
                            .into(),
                    );
                }
            }
            if step.destination_width > ((width / 2) + 1) / 2
                || step.destination_height > ((height / 2) + 1) / 2
            {
                return Err("XMAP10 luminance reduction exceeds its R16 ping-pong backing".into());
            }
        }
        let final_step = self.steps.last().expect("non-empty checked above");
        if !(final_step.destination_width > 3 || final_step.destination_height > 3)
            || (final_step.destination_width + 1) / 2 > 3
            || (final_step.destination_height + 1) / 2 > 3
        {
            return Err(
                "XMAP10 luminance reduction does not stop at the exact-a729 gather boundary".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xmap10TemporalPolicy {
    Continue { prior_frame_sequence: u64 },
    EngineResourceReset {
        reason: crate::TemporalLuminanceResetReason,
        main_scene_resource_identity_sha256: String,
        allocation_generation: u64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xmap10TemporalExecutionInput {
    pub frame_sequence: u64,
    pub delta_seconds: f32,
    pub scalar_source_receipt_sha256: String,
    pub policy: Xmap10TemporalPolicy,
}

impl Xmap10TemporalExecutionInput {
    fn validate(&self, frame: &PreparedXmap10Frame) -> Result<()> {
        if self.frame_sequence != frame.grade.frame_sequence
            || self.frame_sequence != frame.temporal_adaptation.frame_sequence
            || !self.delta_seconds.is_finite()
            || self.delta_seconds < 0.0
            || !is_sha256(&self.scalar_source_receipt_sha256)
        {
            return Err("XMAP10 temporal execution input is invalid".into());
        }
        match &self.policy {
            Xmap10TemporalPolicy::Continue {
                prior_frame_sequence,
            } => {
                let Xmap10TemporalAdaptationSource::Continue {
                    prior_frame_sequence: prepared_prior,
                    ..
                } = &frame.temporal_adaptation.source
                else {
                    return Err("XMAP10 temporal continuation uses a reset preparation".into());
                };
                if prior_frame_sequence != prepared_prior {
                    return Err(
                        "XMAP10 temporal continuation does not match the prepared prior frame"
                            .into(),
                    );
                }
            }
            Xmap10TemporalPolicy::EngineResourceReset {
                reason,
                main_scene_resource_identity_sha256,
                allocation_generation,
            } => {
                let Xmap10TemporalAdaptationSource::EngineResourceReset {
                    reason: prepared_reason,
                    main_scene_resource_identity_sha256: prepared_main_scene,
                    allocation_generation: prepared_generation,
                } = &frame.temporal_adaptation.source
                else {
                    return Err("XMAP10 engine reset uses a continuation preparation".into());
                };
                if self.delta_seconds != 0.0
                    || reason != prepared_reason
                    || main_scene_resource_identity_sha256 != prepared_main_scene
                    || allocation_generation != prepared_generation
                {
                    return Err("XMAP10 engine-reset policy is unauthenticated".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10AttachedFinalTarget {
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub sample_count: u32,
    pub descriptor_sha256: String,
    pub target_identity_sha256: String,
    pub resource_identity_sha256: String,
    pub allocation_generation: u64,
}

impl Xmap10AttachedFinalTarget {
    fn validate(&self, frame: &PreparedXmap10Frame) -> Result<()> {
        let expected = frame
            .targets
            .ordered_targets
            .iter()
            .find(|target| target.role == Xmap10TargetRole::Swapchain)
            .ok_or("XMAP10 preparation omitted Swapchain target")?;
        if self.width != expected.width
            || self.height != expected.height
            || self.dxgi_format != expected.dxgi_format
            || self.sample_count != expected.sample_count
            || self.sample_count != 1
            || !matches!(
                self.dxgi_format,
                DXGI_FORMAT_R8G8B8A8_UNORM_U32 | DXGI_FORMAT_B8G8R8A8_UNORM_U32
            )
            || self.descriptor_sha256 != expected.descriptor_sha256
            || !is_sha256(&self.target_identity_sha256)
            || !is_sha256(&self.resource_identity_sha256)
            || self.allocation_generation == 0
        {
            return Err(
                "XMAP10 attached final target differs from the prepared single-sample BGRA/RGBA8 target"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10TargetBoundPassReceipt {
    pub schema: String,
    pub hour_index: i64,
    pub assigned_stage: AssignedStage,
    pub ordered_input_target_identity_sha256s: Vec<String>,
    pub output_target_identity_sha256: String,
    pub program_identity_sha256: String,
    pub ordered_resource_identity_sha256s: Vec<String>,
    pub ordered_constant_buffer_receipt_sha256s: Vec<String>,
    pub draw_receipt_sha256: String,
    pub packet_pass_receipt_sha256: String,
}

impl Xmap10TargetBoundPassReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        hour_index: i64,
        assigned_stage: AssignedStage,
        ordered_input_target_identity_sha256s: Vec<String>,
        output_target_identity_sha256: String,
        program_identity_sha256: String,
        ordered_resource_identity_sha256s: Vec<String>,
        ordered_constant_buffer_receipt_sha256s: Vec<String>,
        draw_receipt_sha256: String,
    ) -> Result<Self> {
        let mut receipt = Self {
            schema: XMAP10_TARGET_BOUND_PASS_RECEIPT_SCHEMA.into(),
            hour_index,
            assigned_stage,
            ordered_input_target_identity_sha256s,
            output_target_identity_sha256,
            program_identity_sha256,
            ordered_resource_identity_sha256s,
            ordered_constant_buffer_receipt_sha256s,
            draw_receipt_sha256,
            packet_pass_receipt_sha256: String::new(),
        };
        receipt.validate_body()?;
        receipt.packet_pass_receipt_sha256 = canonical_packet_sha256(&receipt)?;
        receipt.validate()?;
        Ok(receipt)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        if !is_sha256(&self.packet_pass_receipt_sha256) {
            return Err("XMAP10 pass receipt digest is invalid".into());
        }
        let mut unsigned = self.clone();
        unsigned.packet_pass_receipt_sha256.clear();
        if self.packet_pass_receipt_sha256 != canonical_packet_sha256(&unsigned)? {
            return Err("XMAP10 pass receipt digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        if self.schema != XMAP10_TARGET_BOUND_PASS_RECEIPT_SCHEMA || self.hour_index < 0 {
            return Err("XMAP10 target-bound pass receipt identity is invalid".into());
        }
        self.assigned_stage
            .validate()
            .map_err(|error| format!("XMAP10 assigned pass: {error}"))?;
        if !is_xmap10_postfx_assignment(&self.assigned_stage) {
            return Err("XMAP10 pass receipt names an unowned scheduled operation".into());
        }
        if self.ordered_input_target_identity_sha256s.is_empty()
            || self.ordered_resource_identity_sha256s.is_empty()
            || self.ordered_constant_buffer_receipt_sha256s.is_empty()
            || !is_sha256(&self.output_target_identity_sha256)
            || !is_sha256(&self.program_identity_sha256)
            || !is_sha256(&self.draw_receipt_sha256)
            || self
                .ordered_input_target_identity_sha256s
                .iter()
                .chain(&self.ordered_resource_identity_sha256s)
                .chain(&self.ordered_constant_buffer_receipt_sha256s)
                .any(|value| !is_sha256(value))
        {
            return Err(
                "XMAP10 target-bound pass omits an actual target/program/resource/cbuffer/draw identity"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10LuminanceDownsampleSubpassReceipt {
    pub schema: String,
    pub ordinal: u8,
    pub input_width: u32,
    pub input_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub actual_pass_receipt_sha256: String,
    pub packet_subpass_receipt_sha256: String,
}

impl Xmap10LuminanceDownsampleSubpassReceipt {
    pub fn seal(
        ordinal: u8,
        input_width: u32,
        input_height: u32,
        output_width: u32,
        output_height: u32,
        actual_pass_receipt_sha256: String,
    ) -> Result<Self> {
        let mut receipt = Self {
            schema: XMAP10_LUMINANCE_DOWNSAMPLE_SUBPASS_RECEIPT_SCHEMA.into(),
            ordinal,
            input_width,
            input_height,
            output_width,
            output_height,
            actual_pass_receipt_sha256,
            packet_subpass_receipt_sha256: String::new(),
        };
        receipt.validate_body()?;
        receipt.packet_subpass_receipt_sha256 = canonical_packet_sha256(&receipt)?;
        receipt.validate()?;
        Ok(receipt)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        let mut unsigned = self.clone();
        unsigned.packet_subpass_receipt_sha256.clear();
        if !is_sha256(&self.packet_subpass_receipt_sha256)
            || self.packet_subpass_receipt_sha256 != canonical_packet_sha256(&unsigned)?
        {
            return Err("XMAP10 luminance subpass receipt digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        if self.schema != XMAP10_LUMINANCE_DOWNSAMPLE_SUBPASS_RECEIPT_SCHEMA
            || self.input_width == 0
            || self.input_height == 0
            || self.output_width != (self.input_width + 1) / 2
            || self.output_height != (self.input_height + 1) / 2
            || !is_sha256(&self.actual_pass_receipt_sha256)
        {
            return Err("XMAP10 luminance subpass dimensions or draw identity are invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10LuminanceDownsampleChainReceipt {
    pub schema: String,
    pub assigned_stage: AssignedStage,
    pub initial_width: u32,
    pub initial_height: u32,
    pub ordered_subpasses: Vec<Xmap10LuminanceDownsampleSubpassReceipt>,
    pub terminal_next_width: u32,
    pub terminal_next_height: u32,
    pub packet_chain_receipt_sha256: String,
}

impl Xmap10LuminanceDownsampleChainReceipt {
    pub fn seal(
        assigned_stage: AssignedStage,
        initial_width: u32,
        initial_height: u32,
        ordered_subpasses: Vec<Xmap10LuminanceDownsampleSubpassReceipt>,
    ) -> Result<Self> {
        let last = ordered_subpasses
            .last()
            .ok_or("XMAP10 luminance chain has no subpasses")?;
        let mut receipt = Self {
            schema: XMAP10_LUMINANCE_DOWNSAMPLE_CHAIN_RECEIPT_SCHEMA.into(),
            assigned_stage,
            initial_width,
            initial_height,
            terminal_next_width: (last.output_width + 1) / 2,
            terminal_next_height: (last.output_height + 1) / 2,
            ordered_subpasses,
            packet_chain_receipt_sha256: String::new(),
        };
        receipt.validate_body()?;
        receipt.packet_chain_receipt_sha256 = canonical_packet_sha256(&receipt)?;
        receipt.validate()?;
        Ok(receipt)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body()?;
        let mut unsigned = self.clone();
        unsigned.packet_chain_receipt_sha256.clear();
        if !is_sha256(&self.packet_chain_receipt_sha256)
            || self.packet_chain_receipt_sha256 != canonical_packet_sha256(&unsigned)?
        {
            return Err("XMAP10 luminance chain receipt digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self) -> Result<()> {
        if self.schema != XMAP10_LUMINANCE_DOWNSAMPLE_CHAIN_RECEIPT_SCHEMA
            || self.assigned_stage.stage_name != "postfx"
            || self.assigned_stage.operation_name != "postfx.luminance_downsample_chain"
            || self.initial_width == 0
            || self.initial_height == 0
            || self.ordered_subpasses.is_empty()
        {
            return Err("XMAP10 luminance chain identity is invalid".into());
        }
        self.assigned_stage
            .validate()
            .map_err(|error| format!("XMAP10 luminance assignment: {error}"))?;
        let mut expected_width = self.initial_width;
        let mut expected_height = self.initial_height;
        for (index, subpass) in self.ordered_subpasses.iter().enumerate() {
            subpass.validate()?;
            if usize::from(subpass.ordinal) != index
                || (subpass.input_width, subpass.input_height) != (expected_width, expected_height)
            {
                return Err("XMAP10 luminance subpass chain is not contiguous".into());
            }
            expected_width = subpass.output_width;
            expected_height = subpass.output_height;
        }
        if self.terminal_next_width != (expected_width + 1) / 2
            || self.terminal_next_height != (expected_height + 1) / 2
            || self.terminal_next_width > 3
            || self.terminal_next_height > 3
            || (expected_width <= 3 && expected_height <= 3)
        {
            return Err("XMAP10 luminance chain does not stop at the exact gather boundary".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10PostFxFrameReceipt {
    pub schema: String,
    /// Which foundation envelope this frame drew. See
    /// `Xmap10PostFxExecutionInput::foundation_envelope_sha256`.
    pub foundation_envelope_sha256: String,
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub main_scene_resource_identity_sha256: String,
    pub main_scene_allocation_generation: u64,
    pub map_copy_receipt_set_sha256: String,
    pub upstream_map_scene_receipt_sha256: String,
    pub final_target: Xmap10AttachedFinalTarget,
    pub topology_sha256: String,
    pub ordered_assigned_operations: Vec<AssignedStage>,
    pub ordered_actual_pass_receipt_sha256s: Vec<String>,
    pub lut_execution_receipt_sha256: String,
    pub restore_scene_receipt_sha256: String,
    pub packet_frame_receipt_sha256: String,
}

impl Xmap10PostFxFrameReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn seal(
        foundation_envelope_sha256: String,
        hour_index: i64,
        content_manifest_sha256: String,
        foundation_content_operation_sha256: String,
        viewer_state_sha256: String,
        main_scene_target_sha256: String,
        main_scene_resource_identity_sha256: String,
        main_scene_allocation_generation: u64,
        map_copy_receipt_set_sha256: String,
        upstream_map_scene_receipt_sha256: String,
        final_target: Xmap10AttachedFinalTarget,
        lut_execution_receipt_sha256: String,
        ordered_passes: &[Xmap10TargetBoundPassReceipt],
    ) -> Result<Self> {
        let first = ordered_passes
            .first()
            .ok_or("XMAP10 post-FX frame has no actual pass receipts")?;
        let ordered_assigned_operations = ordered_passes
            .iter()
            .map(|pass| pass.assigned_stage.clone())
            .collect();
        let ordered_actual_pass_receipt_sha256s = ordered_passes
            .iter()
            .map(|pass| pass.packet_pass_receipt_sha256.clone())
            .collect::<Vec<_>>();
        let restore_scene_receipt_sha256 = ordered_actual_pass_receipt_sha256s
            .last()
            .cloned()
            .ok_or("XMAP10 post-FX frame has no RestoreScene receipt")?;
        let mut receipt = Self {
            schema: XMAP10_POSTFX_FRAME_RECEIPT_SCHEMA.into(),
            foundation_envelope_sha256,
            hour_index,
            content_manifest_sha256,
            foundation_content_operation_sha256,
            viewer_state_sha256,
            main_scene_target_sha256,
            main_scene_resource_identity_sha256,
            main_scene_allocation_generation,
            map_copy_receipt_set_sha256,
            upstream_map_scene_receipt_sha256,
            final_target,
            topology_sha256: first.assigned_stage.topology_sha256.clone(),
            ordered_assigned_operations,
            ordered_actual_pass_receipt_sha256s,
            lut_execution_receipt_sha256,
            restore_scene_receipt_sha256,
            packet_frame_receipt_sha256: String::new(),
        };
        receipt.validate_body(Some(ordered_passes))?;
        receipt.packet_frame_receipt_sha256 = canonical_packet_sha256(&receipt)?;
        receipt.validate()?;
        Ok(receipt)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_body(None)?;
        let mut unsigned = self.clone();
        unsigned.packet_frame_receipt_sha256.clear();
        if !is_sha256(&self.packet_frame_receipt_sha256)
            || self.packet_frame_receipt_sha256 != canonical_packet_sha256(&unsigned)?
        {
            return Err("XMAP10 post-FX frame receipt digest mismatch".into());
        }
        Ok(())
    }

    fn validate_body(&self, actual_passes: Option<&[Xmap10TargetBoundPassReceipt]>) -> Result<()> {
        if self.schema != XMAP10_POSTFX_FRAME_RECEIPT_SCHEMA || self.hour_index < 0 {
            return Err("XMAP10 post-FX frame identity is invalid".into());
        }
        for value in [
            &self.foundation_envelope_sha256,
            &self.content_manifest_sha256,
            &self.foundation_content_operation_sha256,
            &self.viewer_state_sha256,
            &self.main_scene_target_sha256,
            &self.main_scene_resource_identity_sha256,
            &self.map_copy_receipt_set_sha256,
            &self.upstream_map_scene_receipt_sha256,
            &self.topology_sha256,
            &self.lut_execution_receipt_sha256,
            &self.restore_scene_receipt_sha256,
        ] {
            if !is_sha256(value) {
                return Err("XMAP10 post-FX frame omits a shared authenticated identity".into());
            }
        }
        if self.main_scene_allocation_generation == 0
            || self.final_target.width == 0
            || self.final_target.height == 0
            || self.final_target.sample_count != 1
            || !matches!(
                self.final_target.dxgi_format,
                DXGI_FORMAT_R8G8B8A8_UNORM_U32 | DXGI_FORMAT_B8G8R8A8_UNORM_U32
            )
            || !is_sha256(&self.final_target.descriptor_sha256)
            || !is_sha256(&self.final_target.target_identity_sha256)
        {
            return Err("XMAP10 post-FX frame final target is invalid".into());
        }
        if self.ordered_assigned_operations.len() != XMAP10_POSTFX_OPERATION_NAMES.len()
            || self.ordered_actual_pass_receipt_sha256s.len() != XMAP10_POSTFX_OPERATION_NAMES.len()
        {
            return Err("XMAP10 post-FX frame does not contain the six aggregate passes".into());
        }
        let mut prior_ordinal = 0;
        for (index, (assignment, receipt_sha256)) in self
            .ordered_assigned_operations
            .iter()
            .zip(&self.ordered_actual_pass_receipt_sha256s)
            .enumerate()
        {
            assignment
                .validate()
                .map_err(|error| format!("XMAP10 frame assignment: {error}"))?;
            let (expected_stage, expected_operation) = XMAP10_POSTFX_OPERATION_NAMES[index];
            if assignment.stage_name != expected_stage
                || assignment.operation_name != expected_operation
                || assignment.topology_sha256 != self.topology_sha256
                || assignment.assigned_ordinal <= prior_ordinal
                || !is_sha256(receipt_sha256)
            {
                return Err(
                    "XMAP10 aggregate pass order differs from the assigned schedule".into(),
                );
            }
            prior_ordinal = assignment.assigned_ordinal;
        }
        if self.restore_scene_receipt_sha256
            != *self
                .ordered_actual_pass_receipt_sha256s
                .last()
                .expect("six pass receipts checked above")
        {
            return Err("XMAP10 RestoreScene receipt is not the aggregate tail".into());
        }
        if let Some(actual_passes) = actual_passes {
            if actual_passes.len() != XMAP10_POSTFX_OPERATION_NAMES.len() {
                return Err("XMAP10 actual pass receipt count differs from aggregate order".into());
            }
            for ((assignment, receipt_sha256), actual) in self
                .ordered_assigned_operations
                .iter()
                .zip(&self.ordered_actual_pass_receipt_sha256s)
                .zip(actual_passes)
            {
                actual.validate()?;
                if actual.hour_index != self.hour_index
                    || &actual.assigned_stage != assignment
                    || &actual.packet_pass_receipt_sha256 != receipt_sha256
                {
                    return Err("XMAP10 frame receipt differs from an actual pass receipt".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10PostFxExecutionInput {
    /// The foundation envelope this draw belongs to.
    ///
    /// The four values below identify a replay hour, its content and the viewer
    /// state; two DIFFERENT envelopes can agree on all four, so on their own
    /// they let a receipt produced for one frame be accepted into another
    /// frame's bundle (Codex review, round 14). The envelope digest is what
    /// says which frame.
    pub foundation_envelope_sha256: String,
    pub content_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub viewer_state_sha256: String,
    pub main_scene_target_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub upstream_map_scene_receipt_sha256: String,
    pub assigned_stage_schedule: AssignedStageSchedule,
}

impl Xmap10PostFxExecutionInput {
    fn validate(
        &self,
        frame: &PreparedXmap10Frame,
        pre_particle_scene: &Xmap10PreParticleSceneReceipt,
    ) -> Result<Vec<AssignedStage>> {
        frame.validate()?;
        pre_particle_scene.validate()?;
        self.assigned_stage_schedule
            .validate()
            .map_err(|error| format!("XMAP10 assigned-stage schedule: {error}"))?;
        if self.content_manifest_sha256 != frame.content.content_manifest_sha256
            || self.content_manifest_sha256 != pre_particle_scene.content_manifest_sha256
            || self.main_scene_target_sha256 != pre_particle_scene.main_scene_target_sha256
            || frame.hour_index != pre_particle_scene.hour_index
        {
            return Err(
                "XMAP10 post-FX input differs from its prepared/pre-particle frame identity".into(),
            );
        }
        for value in [
            &self.foundation_envelope_sha256,
            &self.content_manifest_sha256,
            &self.foundation_content_operation_sha256,
            &self.viewer_state_sha256,
            &self.main_scene_target_sha256,
            &self.map_copy_receipt_set_sha256,
            &self.upstream_map_scene_receipt_sha256,
        ] {
            if !is_sha256(value) {
                return Err("XMAP10 post-FX input omits a shared authenticated identity".into());
            }
        }
        XMAP10_POSTFX_OPERATION_NAMES
            .iter()
            .map(|(stage, operation)| {
                self.assigned_stage_schedule
                    .require(stage, operation)
                    .map(Clone::clone)
                    .map_err(|error| format!("XMAP10 required assignment: {error}").into())
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10CompletedPostFxFrame {
    pub postfx_frame: Xmap10PostFxFrameReceipt,
    pub target_bound_passes: Vec<Xmap10TargetBoundPassReceipt>,
    pub luminance_downsample_chain: Xmap10LuminanceDownsampleChainReceipt,
    pub lut_execution: Xmap10LutExecutionReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10LutPassExecutionReceipt {
    pub ordinal: u8,
    pub output_target_index: u8,
    pub ordered_input_resource_identity_sha256s: Vec<String>,
    pub output_resource_identity_sha256: String,
    pub program_identity_sha256: String,
    pub execution_receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10LutExecutionReceipt {
    pub schema: String,
    pub frame_sequence: u64,
    pub lut_program_sha256: String,
    pub ordered_lut_source_resource_identity_sha256s: Vec<String>,
    pub ordered_color_cube_resource_identity_sha256s: Vec<String>,
    pub ordered_passes: Vec<Xmap10LutPassExecutionReceipt>,
    pub final_color_cube_content_identity_sha256: String,
    pub deterministic_sha256: String,
}

impl Xmap10LutExecutionReceipt {
    fn validate_against(&self, frame: &PreparedXmap10Frame) -> Result<()> {
        self.validate()?;
        if self.schema != XMAP10_LUT_EXECUTION_RECEIPT_SCHEMA
            || self.frame_sequence != frame.grade.frame_sequence
            || self.lut_program_sha256 != frame.grade.lut_program.deterministic_sha256
            || self.ordered_color_cube_resource_identity_sha256s.len() != 3
            || self.ordered_passes.len() != frame.grade.lut_program.ordered_passes.len()
        {
            return Err("XMAP10 LUT execution does not name the prepared pass plan".into());
        }
        for ((receipt, planned), index) in self
            .ordered_passes
            .iter()
            .zip(&frame.grade.lut_program.ordered_passes)
            .zip(0usize..)
        {
            let expected_inputs = match &planned.kind {
                Xmap10LutBlendPassKind::Copy { source } => vec![source],
                Xmap10LutBlendPassKind::Blend1 { base, blend, .. } => vec![base, blend],
                Xmap10LutBlendPassKind::Blend2 {
                    base_day,
                    base_night,
                    blend_day,
                    blend_night,
                    ..
                } => vec![base_day, base_night, blend_day, blend_night],
            }
            .into_iter()
            .map(|input| match input {
                Xmap10LutInput::Source(source) => self
                    .ordered_lut_source_resource_identity_sha256s
                    .get(usize::from(*source))
                    .cloned()
                    .ok_or_else(|| "XMAP10 LUT source receipt index is unavailable".into()),
                Xmap10LutInput::Intermediate(ordinal) => self
                    .ordered_passes
                    .get(usize::from(*ordinal))
                    .filter(|prior| prior.ordinal < receipt.ordinal)
                    .map(|prior| prior.output_resource_identity_sha256.clone())
                    .ok_or_else(|| "XMAP10 LUT receipt references a non-prior intermediate".into()),
            })
            .collect::<Result<Vec<_>>>()?;
            if usize::from(receipt.ordinal) != index
                || receipt.ordinal != planned.ordinal
                || receipt.output_target_index != planned.output_target_index
                || receipt.ordered_input_resource_identity_sha256s != expected_inputs
                || receipt.ordered_input_resource_identity_sha256s.is_empty()
                || receipt
                    .ordered_input_resource_identity_sha256s
                    .iter()
                    .chain([
                        &receipt.output_resource_identity_sha256,
                        &receipt.program_identity_sha256,
                        &receipt.execution_receipt_sha256,
                    ])
                    .any(|value| !is_sha256(value))
                || receipt.output_resource_identity_sha256
                    != self.ordered_color_cube_resource_identity_sha256s
                        [usize::from(receipt.output_target_index)]
            {
                return Err("XMAP10 LUT execution pass identity is invalid".into());
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP10_LUT_EXECUTION_RECEIPT_SCHEMA
            || self.frame_sequence == 0
            || !is_sha256(&self.lut_program_sha256)
            || self.ordered_color_cube_resource_identity_sha256s.len() != 3
            || self.ordered_passes.is_empty()
        {
            return Err("XMAP10 LUT execution receipt identity is invalid".into());
        }
        if self
            .ordered_lut_source_resource_identity_sha256s
            .iter()
            .chain(&self.ordered_color_cube_resource_identity_sha256s)
            .any(|value| !is_sha256(value))
        {
            return Err("XMAP10 LUT execution omits a physical LUT resource identity".into());
        }
        for (index, receipt) in self.ordered_passes.iter().enumerate() {
            if usize::from(receipt.ordinal) != index
                || usize::from(receipt.output_target_index)
                    >= self.ordered_color_cube_resource_identity_sha256s.len()
                || receipt.ordered_input_resource_identity_sha256s.is_empty()
                || receipt
                    .ordered_input_resource_identity_sha256s
                    .iter()
                    .chain([
                        &receipt.output_resource_identity_sha256,
                        &receipt.program_identity_sha256,
                        &receipt.execution_receipt_sha256,
                    ])
                    .any(|value| !is_sha256(value))
                || receipt.execution_receipt_sha256 != lut_pass_execution_digest(receipt)?
            {
                return Err("XMAP10 LUT execution pass receipt is invalid".into());
            }
        }
        if self
            .ordered_color_cube_resource_identity_sha256s
            .iter()
            .any(|value| !is_sha256(value))
            || !is_sha256(&self.final_color_cube_content_identity_sha256)
        {
            return Err("XMAP10 LUT execution omits a physical ColorCube identity".into());
        }
        let final_pass = self
            .ordered_passes
            .last()
            .ok_or("XMAP10 LUT execution has no final pass")?;
        let expected_final = xmap10_identity_set(
            "postfx.color_cube.final-content",
            &[
                final_pass.output_resource_identity_sha256.clone(),
                self.lut_program_sha256.clone(),
                xmap10_identity_set(
                    "postfx.lut.ordered-pass-executions",
                    &self
                        .ordered_passes
                        .iter()
                        .map(|pass| pass.execution_receipt_sha256.clone())
                        .collect::<Vec<_>>(),
                ),
            ],
        );
        if self.final_color_cube_content_identity_sha256 != expected_final {
            return Err(
                "XMAP10 final ColorCube content identity is not the executed LUT chain".into(),
            );
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if !is_sha256(&self.deterministic_sha256)
            || self.deterministic_sha256 != canonical_packet_sha256(&unsigned)?
        {
            return Err("XMAP10 LUT execution receipt digest mismatch".into());
        }
        Ok(())
    }
}

fn lut_pass_execution_digest(pass: &Xmap10LutPassExecutionReceipt) -> Result<String> {
    canonical_packet_sha256(&(
        "postfx.lut.pass-execution.v1",
        pass.ordinal,
        pass.output_target_index,
        &pass.ordered_input_resource_identity_sha256s,
        &pass.output_resource_identity_sha256,
        &pass.program_identity_sha256,
    ))
}

fn xmap10_identity_set(label: &str, ordered_identities: &[String]) -> String {
    Sha256::hex(
        &serde_json::to_vec(&(
            "hoi4.xmap10.native-identity-set.v1",
            label,
            ordered_identities,
        ))
        .expect("XMAP10 identity-set serialization cannot fail"),
    )
}

pub const XMAP10_SCREEN_SHADOW_RECEIPT_SCHEMA: &str = "hoi4.xmap10.screen-shadow-execution.v3";
pub const XMAP10_SCREEN_CLEAR_BITS: [u32; 4] = [0x3f800000, 0, 0, 0x3f800000];

#[cfg(test)]
mod retained_history_tests {
    use super::*;
    fn frame(sequence: u64) -> Xmap10ShadowHistoryFrame {
        Xmap10ShadowHistoryFrame { epoch: 1, sequence, camera_sha256: "a".repeat(64),
            replay_sha256: "b".repeat(64), content_sha256: "c".repeat(64) }
    }
    #[test]
    fn retained_history_requires_actual_complete_predecessor_and_exact_epoch() {
        let mut history = Xmap10ShadowHistory::new(1, "d".repeat(64)).unwrap();
        assert_eq!(history.read(&frame(1)).unwrap_err(), Xmap10RetainedShadowRefusal::FirstAllocationUndefined);
        history.begin(&frame(1)).unwrap();
        assert_eq!(history.read(&frame(2)).unwrap_err(), Xmap10RetainedShadowRefusal::WriteInProgress);
        history.complete(&frame(1), "e".repeat(64)).unwrap();
        let mut next = frame(2);
        next.camera_sha256 = "f".repeat(64);
        let read = history.read(&next).unwrap();
        assert_eq!(read.allocation_generation, 1);
        assert_eq!(read.producer.camera_sha256, "a".repeat(64));
        assert_eq!(read.consumer.camera_sha256, "f".repeat(64));
        assert_eq!(read.age_in_render_frames, 1);
        assert_eq!(history.read(&frame(3)).unwrap_err(), Xmap10RetainedShadowRefusal::HistorySequenceMismatch);
        next.epoch = 2;
        assert_eq!(history.read(&next).unwrap_err(), Xmap10RetainedShadowRefusal::HistoryReset);
        next.epoch = 1;
        next.content_sha256 = "0".repeat(64);
        assert_eq!(history.read(&next).unwrap_err(), Xmap10RetainedShadowRefusal::ContentMismatch);
    }
    #[test]
    fn failed_or_partial_write_never_reuses_prior_complete_contents() {
        let mut history = Xmap10ShadowHistory::new(1, "d".repeat(64)).unwrap();
        history.begin(&frame(1)).unwrap();
        history.complete(&frame(1), "e".repeat(64)).unwrap();
        history.begin(&frame(2)).unwrap();
        history.fail(Xmap10RetainedShadowRefusal::UpstreamCasterUnavailable);
        assert_eq!(history.read(&frame(3)).unwrap_err(), Xmap10RetainedShadowRefusal::UpstreamCasterUnavailable);
        assert!(history.complete(&frame(2), "f".repeat(64)).is_err());
        history.begin(&frame(3)).unwrap();
        history.complete(&frame(3), "f".repeat(64)).unwrap();
        assert_eq!(history.read(&frame(4)).unwrap().completed_write_sha256, "f".repeat(64));
        assert_eq!(Xmap10ShadowHistory::new(2, "d".repeat(64)).unwrap().read(&frame(4)).unwrap_err(),
            Xmap10RetainedShadowRefusal::RecreatedUndefined);
    }
}


#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10ShadowHistoryFrame {
    pub epoch: u64,
    pub sequence: u64,
    pub camera_sha256: String,
    pub replay_sha256: String,
    pub content_sha256: String,
}

impl Xmap10ShadowHistoryFrame {
    pub fn validate(&self) -> Result<()> {
        if [&self.camera_sha256, &self.replay_sha256, &self.content_sha256]
            .iter().any(|hash| !is_sha256(hash)) {
            return Err("XMAP10 shadow history frame has invalid identities".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Xmap10RetainedShadowRefusal {
    FirstAllocationUndefined,
    RecreatedUndefined,
    HistoryReset,
    HistorySequenceMismatch,
    AllocationMismatch,
    DeviceMismatch,
    ContentMismatch,
    WriteInProgress,
    FailedHistory,
    UpstreamCasterUnavailable,
    ScreenTreeProducerUnavailable,
}

impl std::fmt::Display for Xmap10RetainedShadowRefusal {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "map-arrow-light-depth-retained-shadow-map-unavailable: {self:?}")
    }
}
impl std::error::Error for Xmap10RetainedShadowRefusal {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10RetainedShadowReadReceipt {
    pub allocation_generation: u64,
    pub allocation_descriptor_sha256: String,
    pub producer: Xmap10ShadowHistoryFrame,
    pub consumer: Xmap10ShadowHistoryFrame,
    pub completed_write_sha256: String,
    pub age_in_render_frames: u64,
}

impl Xmap10RetainedShadowReadReceipt {
    pub fn validate(&self) -> Result<()> {
        self.producer.validate()?;
        self.consumer.validate()?;
        if self.allocation_generation == 0
            || !is_sha256(&self.allocation_descriptor_sha256)
            || !is_sha256(&self.completed_write_sha256)
            || self.producer.epoch != self.consumer.epoch
            || self.producer.content_sha256 != self.consumer.content_sha256
            || self.producer.sequence.checked_add(1) != Some(self.consumer.sequence)
            || self.age_in_render_frames != 1 {
            return Err("XMAP10 retained shadow read has a stale allocation/history join".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Xmap10RetainedShadowState {
    Undefined(Xmap10RetainedShadowRefusal),
    Writing(Xmap10ShadowHistoryFrame),
    Complete { frame: Xmap10ShadowHistoryFrame, write_sha256: String },
    Failed(Xmap10RetainedShadowRefusal),
}

#[derive(Clone, Debug)]
struct Xmap10ShadowHistory {
    allocation_generation: u64,
    descriptor_sha256: String,
    state: std::rc::Rc<std::cell::RefCell<Xmap10RetainedShadowState>>,
}

impl Xmap10ShadowHistory {
    fn new(allocation_generation: u64, descriptor_sha256: String) -> Result<Self> {
        if allocation_generation == 0 || !is_sha256(&descriptor_sha256) {
            return Err("XMAP10 shadow allocation identity is invalid".into());
        }
        Ok(Self { allocation_generation, descriptor_sha256,
            state: std::rc::Rc::new(std::cell::RefCell::new(Xmap10RetainedShadowState::Undefined(if allocation_generation == 1 {
                Xmap10RetainedShadowRefusal::FirstAllocationUndefined
            } else { Xmap10RetainedShadowRefusal::RecreatedUndefined }))) })
    }

    fn read(&self, consumer: &Xmap10ShadowHistoryFrame) -> std::result::Result<Xmap10RetainedShadowReadReceipt, Xmap10RetainedShadowRefusal> {
        let state = self.state.borrow();
        let (producer, write) = match &*state {
            Xmap10RetainedShadowState::Undefined(reason) | Xmap10RetainedShadowState::Failed(reason) => return Err(reason.clone()),
            Xmap10RetainedShadowState::Writing(_) => return Err(Xmap10RetainedShadowRefusal::WriteInProgress),
            Xmap10RetainedShadowState::Complete { frame, write_sha256 } => (frame, write_sha256),
        };
        if producer.epoch != consumer.epoch { return Err(Xmap10RetainedShadowRefusal::HistoryReset); }
        if producer.content_sha256 != consumer.content_sha256 { return Err(Xmap10RetainedShadowRefusal::ContentMismatch); }
        if producer.sequence.checked_add(1) != Some(consumer.sequence) { return Err(Xmap10RetainedShadowRefusal::HistorySequenceMismatch); }
        let read = Xmap10RetainedShadowReadReceipt { allocation_generation: self.allocation_generation,
            allocation_descriptor_sha256: self.descriptor_sha256.clone(), producer: producer.clone(),
            consumer: consumer.clone(), completed_write_sha256: write.clone(), age_in_render_frames: 1 };
        read.validate().map_err(|_| Xmap10RetainedShadowRefusal::FailedHistory)?;
        Ok(read)
    }

    fn begin(&mut self, frame: &Xmap10ShadowHistoryFrame) -> Result<()> {
        frame.validate()?;
        if matches!(*self.state.borrow(), Xmap10RetainedShadowState::Writing(_)) {
            return Err(Xmap10RetainedShadowRefusal::WriteInProgress.into());
        }
        *self.state.borrow_mut() = Xmap10RetainedShadowState::Writing(frame.clone());
        Ok(())
    }

    fn complete(&mut self, frame: &Xmap10ShadowHistoryFrame, write_sha256: String) -> Result<()> {
        if *self.state.borrow() != Xmap10RetainedShadowState::Writing(frame.clone()) || !is_sha256(&write_sha256) {
            self.fail(Xmap10RetainedShadowRefusal::FailedHistory);
            return Err("XMAP10 screen write cannot publish an unmatched completion".into());
        }
        *self.state.borrow_mut() = Xmap10RetainedShadowState::Complete { frame: frame.clone(), write_sha256 };
        Ok(())
    }

    fn fail(&mut self, reason: Xmap10RetainedShadowRefusal) {
        *self.state.borrow_mut() = Xmap10RetainedShadowState::Failed(reason);
    }
}


struct Xmap10ShadowWriteGuard {
    state: std::rc::Rc<std::cell::RefCell<Xmap10RetainedShadowState>>,
    finished: bool,
}

impl Xmap10ShadowWriteGuard {
    fn new(history: &Xmap10ShadowHistory) -> Self { Self { state: history.state.clone(), finished: false } }
    fn finish(mut self) { self.finished = true; }
}

impl Drop for Xmap10ShadowWriteGuard {
    fn drop(&mut self) {
        if !self.finished {
            *self.state.borrow_mut() = Xmap10RetainedShadowState::Failed(Xmap10RetainedShadowRefusal::FailedHistory);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10NativeShaderResourceBinding {
    pub symbol: String,
    pub owner_packet: String,
    pub producer_camera_frame_sha256: String,
    pub producer_replay_frame_identity_sha256: String,
    pub consumer_camera_frame_sha256: String,
    pub consumer_replay_frame_identity_sha256: String,
    pub stable_resource_id: String,
    pub resource_sha256: String,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap10ScreenShadowExecutionReceipt {
    pub schema: String,
    pub active: bool,
    pub cleared_color_f32_bits: [u32; 4],
    pub tree_partitions: Vec<crate::xmap11_d3d_handoff::Xmap11UnlitPartitionReceipt>,
    pub ordered_partition_matrix_sha256s: Vec<String>,
    pub terrain_frame_sha256: String,
    pub map_copy_receipt_set_sha256: String,
    pub xmap09_light_depth_execution_sha256: String,
    pub xmap09_shadow_matrix_payload_sha256: String,
    pub screen_chain_plan_sha256: String,
    pub content_closure_sha256: String,
    pub ordered_scene_cb0_composition_sha256s: Vec<String>,
    pub terrain_unlit_vertex_bytecode_sha256: String,
    pub terrain_unlit_pixel_bytecode_sha256: String,
    pub blur_horizontal_vertex_bytecode_sha256: String,
    pub blur_horizontal_pixel_bytecode_sha256: String,
    pub blur_vertical_vertex_bytecode_sha256: String,
    pub blur_vertical_pixel_bytecode_sha256: String,
    pub draw_count: u32,
    pub binding: Xmap10NativeShaderResourceBinding,
    pub deterministic_sha256: String,
}

impl Xmap10ScreenShadowExecutionReceipt {
    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP10_SCREEN_SHADOW_RECEIPT_SCHEMA
            || self.binding.symbol != "ShadowMap"
            || self.binding.owner_packet != "XMAP10"
        {
            return Err("XMAP10 screen-shadow receipt identity is invalid".into());
        }
        if self.cleared_color_f32_bits != XMAP10_SCREEN_CLEAR_BITS
            || (self.active && (self.ordered_scene_cb0_composition_sha256s.is_empty()
                || self.tree_partitions.len() != self.ordered_partition_matrix_sha256s.len()
                || self.tree_partitions.len() != self.ordered_scene_cb0_composition_sha256s.len()))
            || (!self.active && (self.draw_count != 0 || !self.tree_partitions.is_empty()
                || !self.ordered_partition_matrix_sha256s.is_empty() || !self.ordered_scene_cb0_composition_sha256s.is_empty()))
            || self.binding.producer_camera_frame_sha256 != self.binding.consumer_camera_frame_sha256
            || self.binding.producer_replay_frame_identity_sha256 != self.binding.consumer_replay_frame_identity_sha256 {
            return Err("XMAP10 screen-shadow receipt violates current clear/producer law".into());
        }
        let mut previous_partition = 0;
        let mut batches = std::collections::BTreeSet::new();
        for (tree, matrix) in self.tree_partitions.iter().zip(&self.ordered_partition_matrix_sha256s) {
            tree.validate()?;
            if tree.partition <= previous_partition || (previous_partition == 0 && tree.partition != 1)
                || tree.matrix_payload_sha256 != *matrix
                || tree.ordered_batch_indices.iter().any(|index| !batches.insert(*index)) {
                return Err("XMAP10 tree/matrix partition receipt census mismatch".into());
            }
            previous_partition = tree.partition;
        }
        for hash in [
            &self.terrain_frame_sha256,
            &self.map_copy_receipt_set_sha256,
            &self.xmap09_light_depth_execution_sha256,
            &self.xmap09_shadow_matrix_payload_sha256,
            &self.screen_chain_plan_sha256,
            &self.terrain_unlit_vertex_bytecode_sha256,
            &self.terrain_unlit_pixel_bytecode_sha256,
            &self.blur_horizontal_vertex_bytecode_sha256,
            &self.blur_horizontal_pixel_bytecode_sha256,
            &self.blur_vertical_vertex_bytecode_sha256,
            &self.blur_vertical_pixel_bytecode_sha256,
            &self.binding.producer_camera_frame_sha256,
            &self.binding.producer_replay_frame_identity_sha256,
            &self.binding.consumer_camera_frame_sha256,
            &self.binding.consumer_replay_frame_identity_sha256,
            &self.content_closure_sha256,
            &self.binding.resource_sha256,
            &self.binding.descriptor_sha256,
        ]
        .into_iter()
        .chain(&self.ordered_scene_cb0_composition_sha256s)
        {
            if !is_sha256(hash) {
                return Err("XMAP10 screen-shadow receipt contains an invalid SHA-256".into());
            }
        }
        if self.binding.stable_resource_id.trim().is_empty() {
            return Err("XMAP10 screen-shadow binding has no stable resource ID".into());
        }
        let mut unsigned = self.clone();
        unsigned.deterministic_sha256.clear();
        if !is_sha256(&self.deterministic_sha256)
            || self.deterministic_sha256 != canonical_packet_sha256(&unsigned)?
        {
            return Err("XMAP10 screen-shadow receipt digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UnlitTextureProviderRefusal {
    program: &'static str,
    shader_register: u32,
    symbol: String,
}

impl std::fmt::Display for UnlitTextureProviderRefusal {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "unlit-reflected-texture-provider-unavailable: {} t{} ({})", self.program, self.shader_register, self.symbol)
    }
}
impl std::error::Error for UnlitTextureProviderRefusal {}

pub(crate) fn require_unlit_texture_providers(
    program: &'static str,
    resources: &[crate::shader_compile::DxbcResource],
) -> Result<()> {
    for resource in resources.iter().filter(|resource| resource.resource_type == 2) {
        let supplied = match program {
            "terrainunlit" => matches!(resource.bind_point, 0 | 1 | 2 | 7 | 12),
            "treeunlit" => matches!(resource.bind_point, 0..=4 | 6 | 8..=15),
            _ => false,
        };
        if !supplied {
            return Err(UnlitTextureProviderRefusal {
                program, shader_register: resource.bind_point, symbol: resource.name.clone(),
            }.into());
        }
    }
    Ok(())
}

pub fn xmap10_terrain_unlit_cb1_bytes(
    xmap04_draw_cb1: &[u8],
    shadow_matrix: &crate::Xmap09ShadowMatrixBinding,
) -> Result<Vec<u8>> {
    if xmap04_draw_cb1.len() != crate::TERRAIN_CHUNK_CB1_BYTES
        || xmap04_draw_cb1[..64].iter().any(|byte| *byte != 0)
        || shadow_matrix.symbol != "ShadowMapTextureMatrix"
        || shadow_matrix.owner_packet != "XMAP09"
        || !is_sha256(&shadow_matrix.input_payload_sha256)
        || shadow_matrix.descriptor_sha256
            != Sha256::hex(b"XMAP09\0ShadowMapTextureMatrix\0float4x4\064")
        || shadow_matrix
            .shadow_map_texture_matrix_f32_bits
            .iter()
            .any(|bits| !f32::from_bits(*bits).is_finite())
    {
        return Err("XMAP10 terrainunlit CB1 inputs violate the XMAP04/XMAP09 join".into());
    }
    let matrix_bytes = shadow_matrix
        .shadow_map_texture_matrix_f32_bits
        .iter()
        .flat_map(|bits| bits.to_le_bytes())
        .collect::<Vec<_>>();
    if shadow_matrix.payload_sha256 != Sha256::hex(&matrix_bytes) {
        return Err("XMAP10 terrainunlit shadow-matrix payload digest mismatch".into());
    }
    let mut bytes = xmap04_draw_cb1.to_vec();
    bytes[..64].copy_from_slice(&matrix_bytes);
    Ok(bytes)
}

impl Xmap10CompletedPostFxFrame {
    pub fn validate(&self) -> Result<()> {
        self.luminance_downsample_chain.validate()?;
        self.lut_execution.validate()?;
        self.postfx_frame
            .validate_body(Some(&self.target_bound_passes))?;
        self.postfx_frame.validate()?;
        if self.postfx_frame.lut_execution_receipt_sha256 != self.lut_execution.deterministic_sha256
        {
            return Err("XMAP10 completed frame does not bind the LUT execution receipt".into());
        }
        let luminance_pass = self
            .target_bound_passes
            .get(1)
            .ok_or("XMAP10 completed frame omits the luminance chain pass")?;
        if luminance_pass.assigned_stage != self.luminance_downsample_chain.assigned_stage
            || luminance_pass.draw_receipt_sha256
                != self.luminance_downsample_chain.packet_chain_receipt_sha256
        {
            return Err(
                "XMAP10 completed frame does not bind the aggregate luminance chain receipt".into(),
            );
        }
        let restore = self
            .target_bound_passes
            .last()
            .ok_or("XMAP10 completed frame omits RestoreScene")?;
        if restore.ordered_input_target_identity_sha256s.get(2)
            != Some(&self.lut_execution.final_color_cube_content_identity_sha256)
            || restore.ordered_resource_identity_sha256s.get(2)
                != Some(&self.lut_execution.final_color_cube_content_identity_sha256)
        {
            return Err(
                "XMAP10 RestoreScene does not consume the authenticated final LUT content".into(),
            );
        }
        Ok(())
    }
}

const XMAP10_POSTFX_OPERATION_NAMES: [(&str, &str); 6] = [
    ("postfx", "postfx.bright_pass"),
    ("postfx", "postfx.luminance_downsample_chain"),
    ("postfx", "postfx.luminance_gather"),
    ("postfx", "postfx.bloom.horizontal"),
    ("postfx", "postfx.bloom.vertical"),
    ("postfx", "postfx.restore_scene"),
];

pub fn xmap10_postfx_assigned_stage_topology(
) -> Result<portable_render_stream::AssignedStageTopology> {
    let nodes = XMAP10_POSTFX_OPERATION_NAMES
        .iter()
        .map(|(stage, operation)| {
            portable_render_stream::AssignedStageIdentity::new(*stage, *operation)
        })
        .collect::<Vec<_>>();
    let edges = nodes
        .windows(2)
        .map(|pair| {
            portable_render_stream::AssignedStageEdge::new(pair[0].clone(), pair[1].clone())
        })
        .collect();
    portable_render_stream::AssignedStageTopology::seal(nodes, edges).map_err(Into::into)
}

fn is_xmap10_postfx_assignment(assignment: &AssignedStage) -> bool {
    XMAP10_POSTFX_OPERATION_NAMES
        .iter()
        .any(|(stage, operation)| {
            assignment.stage_name == *stage && assignment.operation_name == *operation
        })
}

fn reduction_plan_digest(plan: &Xmap10LuminanceReductionPlan) -> String {
    let mut bytes = Vec::new();
    push_string(&mut bytes, &plan.schema);
    push_string(&mut bytes, &plan.evidence_receipt_sha256);
    for step in &plan.steps {
        bytes.push(step.ordinal);
        bytes.extend_from_slice(&step.source_width.to_le_bytes());
        bytes.extend_from_slice(&step.source_height.to_le_bytes());
        bytes.extend_from_slice(&step.destination_width.to_le_bytes());
        bytes.extend_from_slice(&step.destination_height.to_le_bytes());
        bytes.push(u8::from(step.source_is_bright_alpha));
    }
    Sha256::hex(&bytes)
}

fn push_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn canonical_packet_sha256<T: Serialize>(value: &T) -> Result<String> {
    Ok(Sha256::hex(&serde_json::to_vec(value)?))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::bloom::BloomBlurKernel;
    use crate::color_lut::{parse_color_correction_tga, ColorCorrectionLut};
    use crate::d3d11_device::{native_d3d11_check_void_upload_device, D3dPtr};
    use crate::shader_compile::{
        build_stage_hlsl_with_defines, d3d_compile_stage, effect_file_from_resolver,
        parse_dxbc_reflection, DxbcReflection, ShaderStage, ENGINE_COMPILE_FLAGS,
    };
    use crate::{
        NativeProductionMainSceneTarget, NativeXmap04TerrainHandoff,
        NativeXmap09ShaderResourceView, NativeXmap10MapCopySceneCb0View, PreparedTerrainFrame,
        ProductionMainSceneTargetDesc, ShadowScreenChainPlan, ShadowScreenPassKind,
    };
    use std::collections::BTreeMap;
    use std::{mem, ptr};
    use winapi::shared::dxgiformat::{
        DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_R16_FLOAT, DXGI_FORMAT_R32G32_SINT,
        DXGI_FORMAT_R8G8B8A8_UNORM,
    };
    use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
    use winapi::shared::guiddef::GUID;
    use winapi::um::d3d11::*;
    use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP;
    use winapi::um::unknwnbase::IUnknown;
    use winapi::Interface;

    const LUMINANCE_SHADER: &str = "gfx/FX/downsample_luminance.shader";
    const LUT_SHADER: &str = "gfx/FX/lut_blender.shader";
    const REPLAY_PRESENTATION_RESOURCE_IDENTITY_GUID: GUID = GUID {
        Data1: 0x7d3f2c91,
        Data2: 0x8b2e,
        Data3: 0x4f6d,
        Data4: [0xa1, 0x73, 0x24, 0x68, 0x91, 0xcf, 0x52, 0x0b],
    };

    struct NativeTarget {
        _texture: D3dPtr<ID3D11Texture2D>,
        rtv: D3dPtr<ID3D11RenderTargetView>,
        srv: D3dPtr<ID3D11ShaderResourceView>,
        width: u32,
        height: u32,
        resource_identity_sha256: String,
    }

    struct NativeSampledTexture {
        _texture: D3dPtr<ID3D11Texture2D>,
        srv: D3dPtr<ID3D11ShaderResourceView>,
        resource_identity_sha256: String,
    }

    struct NativeProgram {
        vertex: D3dPtr<ID3D11VertexShader>,
        pixel: D3dPtr<ID3D11PixelShader>,
        vertex_bytecode: Vec<u8>,
        pixel_bytecode: Vec<u8>,
        reflection_sha256: String,
    }

    struct NativeShadowBlurProgram {
        program: NativeProgram,
        blend: D3dPtr<ID3D11BlendState>,
        sampler: Option<D3dPtr<ID3D11SamplerState>>,
    }

    impl NativeShadowBlurProgram {
        unsafe fn bind(&self, context: *mut ID3D11DeviceContext, cb: *mut ID3D11Buffer) {
            (*context).VSSetShader(self.program.vertex.as_raw(), ptr::null(), 0);
            (*context).PSSetShader(self.program.pixel.as_raw(), ptr::null(), 0);
            (*context).GSSetShader(ptr::null_mut(), ptr::null(), 0);
            (*context).HSSetShader(ptr::null_mut(), ptr::null(), 0);
            (*context).DSSetShader(ptr::null_mut(), ptr::null(), 0);
            (*context).SOSetTargets(0, ptr::null(), ptr::null());
            (*context).VSSetConstantBuffers(0, 1, &cb);
            (*context).PSSetConstantBuffers(0, 1, &cb);
            (*context).OMSetBlendState(self.blend.as_raw(), &[0.0; 4], u32::MAX);
            let sampler = self.sampler.as_ref().map_or(ptr::null_mut(), D3dPtr::as_raw);
            (*context).PSSetSamplers(0, 1, &sampler);
        }
    }

    struct NativeLutProgram {
        program: NativeProgram,
        effect_name: String,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct PostEffectBaseCb {
        values: [f32; 20],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct LuminanceDownsampleCb {
        values: [f32; 12],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct LuminanceGatherCb {
        values: [f32; 12],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct BloomCb {
        values: [f32; 12],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct LutBlendCb {
        values: [f32; 4],
    }

    pub struct NativeXmap10PostFxExecutor {
        device: D3dPtr<ID3D11Device>,
        context: D3dPtr<ID3D11DeviceContext>,
        prepared_frame_sha256: String,
        content_closure_sha256: String,
        target_plan_sha256: String,
        main_scene_resource_identity_sha256: String,
        main_scene_allocation_generation: u64,
        main_scene_texture: D3dPtr<ID3D11Texture2D>,
        programs: Vec<(Xmap10ProgramRole, NativeProgram)>,
        scalar_downsample: NativeProgram,
        lut_blend1: NativeLutProgram,
        lut_blend2: NativeLutProgram,
        program_receipt: Xmap10NativeProgramReceiptRef,
        main_scene_srv: D3dPtr<ID3D11ShaderResourceView>,
        bright: NativeTarget,
        luminance_ping: [NativeTarget; 2],
        temporal: [NativeTarget; 2],
        temporal_read: usize,
        bloom_horizontal: NativeTarget,
        restore_bloom: NativeTarget,
        lut_source_keys: Vec<portable_content_manifest::UsedContentKey>,
        lut_sources: Vec<NativeSampledTexture>,
        color_cubes: [NativeTarget; 3],
        final_color_cube_index: usize,
        input_layout: D3dPtr<ID3D11InputLayout>,
        fullscreen_vertices: D3dPtr<ID3D11Buffer>,
        post_effect_cb: D3dPtr<ID3D11Buffer>,
        luminance_cb: D3dPtr<ID3D11Buffer>,
        luminance_gather_cb: D3dPtr<ID3D11Buffer>,
        bloom_cb: D3dPtr<ID3D11Buffer>,
        lut_cb: D3dPtr<ID3D11Buffer>,
        linear_clamp: D3dPtr<ID3D11SamplerState>,
        point_clamp: D3dPtr<ID3D11SamplerState>,
        opaque_blend: D3dPtr<ID3D11BlendState>,
        rgb_only_blend: D3dPtr<ID3D11BlendState>,
        depth_disabled: D3dPtr<ID3D11DepthStencilState>,
        rasterizer: D3dPtr<ID3D11RasterizerState>,
        main_width: u32,
        main_height: u32,
        last_frame_sequence: Option<u64>,
    }

    pub struct NativeXmap10ScreenTargets {
        final_target: NativeTarget,
        intermediate_target: NativeTarget,
        depth: D3dPtr<ID3D11DepthStencilView>,
        device_identity: usize,
        history: Xmap10ShadowHistory,
    }

    pub struct NativeXmap10RetainedShadowView<'a> {
        raw_srv: *mut ID3D11ShaderResourceView,
        receipt: Xmap10RetainedShadowReadReceipt,
        device_identity: usize,
        _owner: std::marker::PhantomData<&'a NativeXmap10ScreenTargets>,
    }

    impl NativeXmap10RetainedShadowView<'_> {
        pub fn receipt(&self) -> &Xmap10RetainedShadowReadReceipt { &self.receipt }
        pub(crate) fn as_raw(&self) -> *mut ID3D11ShaderResourceView { self.raw_srv }
        pub(crate) fn device_identity(&self) -> usize { self.device_identity }
    }

    impl NativeXmap10ScreenTargets {
        unsafe fn create(device: &D3dPtr<ID3D11Device>, plan: &ShadowScreenChainPlan, generation: u64) -> Result<Self> {
            validate_screen_shadow_plan(plan)?;
            let target = &plan.producer_and_final_target;
            let descriptor_sha256 = Sha256::hex(&serde_json::to_vec(&(
                target.width, target.height, target.dxgi_format, target.mip_levels,
                target.bind_flags, 1_u32, 45_u32,
            ))?);
            let allocation_sha256 = Sha256::hex(&serde_json::to_vec(&(&descriptor_sha256, generation))?);
            let history = Xmap10ShadowHistory::new(generation, descriptor_sha256)?;
            let final_target = create_target(device, &allocation_sha256, target.width, target.height,
                target.dxgi_format, "retained-shadow-producer")?;
            let intermediate_target = create_target(device, &allocation_sha256, target.width, target.height,
                target.dxgi_format, "retained-shadow-intermediate")?;
            let depth_desc = D3D11_TEXTURE2D_DESC { Width: target.width, Height: target.height,
                MipLevels: 1, ArraySize: 1, Format: 45, SampleDesc: winapi::shared::dxgitype::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_DEPTH_STENCIL, CPUAccessFlags: 0, MiscFlags: 0 };
            let mut texture = ptr::null_mut();
            hr((*device.as_raw()).CreateTexture2D(&depth_desc, ptr::null(), &mut texture), "CreateTexture2D(screen shadow depth)")?;
            let texture = D3dPtr::from_raw(texture, "screen shadow depth texture")?;
            let mut depth = ptr::null_mut();
            hr((*device.as_raw()).CreateDepthStencilView(texture.as_raw() as *mut _, ptr::null(), &mut depth), "CreateDepthStencilView(screen shadow)")?;
            Ok(Self { final_target, intermediate_target, depth: D3dPtr::from_raw(depth, "screen shadow depth")?,
                device_identity: device.as_raw() as usize, history })
        }

        pub fn retained_before_frame(&self, frame: &Xmap10ShadowHistoryFrame)
            -> std::result::Result<NativeXmap10RetainedShadowView<'_>, Xmap10RetainedShadowRefusal> {
            let receipt = self.history.read(frame)?;
            Ok(NativeXmap10RetainedShadowView { raw_srv: self.final_target.srv.as_raw(), receipt,
                device_identity: self.device_identity, _owner: std::marker::PhantomData })
        }

        unsafe fn begin_write(&mut self, context: *mut ID3D11DeviceContext, frame: &Xmap10ShadowHistoryFrame) -> Result<()> {
            self.history.begin(frame)?;
            let nulls = [ptr::null_mut(); 16];
            (*context).PSSetShaderResources(0, 16, nulls.as_ptr());
            (*context).VSSetShaderResources(0, 16, nulls.as_ptr());
            let rtv = self.final_target.rtv.as_raw();
            (*context).OMSetRenderTargets(1, &rtv, self.depth.as_raw());
            (*context).ClearRenderTargetView(rtv, &XMAP10_SCREEN_CLEAR_BITS.map(f32::from_bits));
            (*context).ClearDepthStencilView(self.depth.as_raw(), D3D11_CLEAR_DEPTH, 1.0, 0);
            Ok(())
        }
    }

    #[cfg(test)]
    mod retained_target_tests {
        use super::*;
        #[test]
        fn native_screen_clear_writes_engine_color_and_failed_guard_invalidates_history() {
            use winapi::um::d3dcommon::{D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0};
            unsafe {
                let mut device = ptr::null_mut();
                let mut context = ptr::null_mut();
                hr(D3D11CreateDevice(ptr::null_mut(), D3D_DRIVER_TYPE_WARP, ptr::null_mut(), 0,
                    &D3D_FEATURE_LEVEL_11_0, 1, D3D11_SDK_VERSION, &mut device, ptr::null_mut(), &mut context), "screen clear WARP").unwrap();
                let device = D3dPtr::from_raw(device, "screen clear device").unwrap();
                let context = D3dPtr::from_raw(context, "screen clear context").unwrap();
                let final_desc = crate::ShadowScreenTarget { stable_resource_id: "screen".into(), width: 8, height: 8,
                    dxgi_format: 87, mip_levels: 1, bind_flags: 0x28 };
                let mut intermediate = final_desc.clone(); intermediate.stable_resource_id = "intermediate".into();
                let plan = ShadowScreenChainPlan {
                    light_depth_target: crate::L03LightDepthTarget { stable_resource_id: "depth".into(), width: 2048, height: 2048, dxgi_format: 44, mip_levels: 1 },
                    producer_and_final_target: final_desc, blur_intermediate_target: intermediate,
                    passes: [(ShadowScreenPassKind::TerrainUnlitProducer, "depth", "screen", None),
                        (ShadowScreenPassKind::HorizontalBlur, "screen", "intermediate", Some([0; 8])),
                        (ShadowScreenPassKind::VerticalBlur, "intermediate", "screen", Some([0; 8]))].into_iter()
                        .map(|(kind, source, target, cb0_f32_bits)| crate::ShadowScreenPassPlan { kind, source_resource_id: source.into(), target_resource_id: target.into(), cb0_f32_bits }).collect(),
                };
                let mut targets = NativeXmap10ScreenTargets::create(&device, &plan, 1).unwrap();
                let frame = Xmap10ShadowHistoryFrame { epoch: 1, sequence: 1, camera_sha256: "a".repeat(64),
                    replay_sha256: "b".repeat(64), content_sha256: "c".repeat(64) };
                assert!(matches!(targets.retained_before_frame(&frame), Err(Xmap10RetainedShadowRefusal::FirstAllocationUndefined)));
                (*context.as_raw()).ClearRenderTargetView(targets.final_target.rtv.as_raw(), &[0.0, 1.0, 1.0, 0.0]);
                targets.begin_write(context.as_raw(), &frame).unwrap();
                let guard = Xmap10ShadowWriteGuard::new(&targets.history);
                let mut desc: D3D11_TEXTURE2D_DESC = mem::zeroed();
                (*targets.final_target._texture.as_raw()).GetDesc(&mut desc);
                desc.Usage = D3D11_USAGE_STAGING; desc.BindFlags = 0; desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ; desc.MiscFlags = 0;
                let mut staging = ptr::null_mut();
                hr((*device.as_raw()).CreateTexture2D(&desc, ptr::null(), &mut staging), "screen clear staging").unwrap();
                let staging = D3dPtr::from_raw(staging, "screen clear staging").unwrap();
                (*context.as_raw()).OMSetRenderTargets(0, ptr::null(), ptr::null_mut());
                (*context.as_raw()).CopyResource(staging.as_raw().cast(), targets.final_target._texture.as_raw().cast());
                let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
                hr((*context.as_raw()).Map(staging.as_raw().cast(), 0, D3D11_MAP_READ, 0, &mut mapped), "screen clear readback").unwrap();
                let pixel = std::slice::from_raw_parts(mapped.pData.cast::<u8>(), 4).to_vec();
                (*context.as_raw()).Unmap(staging.as_raw().cast(), 0);
                assert_eq!(pixel, [0, 0, 255, 255]);
                drop(guard);
                let mut next = frame.clone(); next.sequence += 1;
                assert_eq!(targets.history.read(&next).unwrap_err(), Xmap10RetainedShadowRefusal::FailedHistory);
                targets.begin_write(context.as_raw(), &next).unwrap();
                let guard = Xmap10ShadowWriteGuard::new(&targets.history);
                targets.history.complete(&next, "d".repeat(64)).unwrap(); guard.finish();
                next.sequence += 1;
                assert_eq!(targets.retained_before_frame(&next).unwrap().receipt.completed_write_sha256, "d".repeat(64));
            }
        }
    }

    pub(crate) struct NativeXmap10ScreenWriteLease<'a> {
        pub(crate) device: *mut ID3D11Device,
        pub(crate) context: *mut ID3D11DeviceContext,
        pub(crate) color_rtv: *mut ID3D11RenderTargetView,
        pub(crate) depth_dsv: *mut ID3D11DepthStencilView,
        pub(crate) width: u32,
        pub(crate) height: u32,
        pub(crate) frame: &'a Xmap10ShadowHistoryFrame,
        pub(crate) viewer_sha256: &'a str,
        pub(crate) map_copy_sha256: &'a str,
        _owner: std::marker::PhantomData<&'a NativeXmap10ScreenTargets>,
    }

    pub struct NativeXmap10ScreenShadowHandoff {
        device: D3dPtr<ID3D11Device>,
        terrain_unlit: NativeProgram,
        terrain_input_layout: D3dPtr<ID3D11InputLayout>,
        blur_horizontal: NativeShadowBlurProgram,
        blur_vertical: NativeShadowBlurProgram,
        blur_input_layout: D3dPtr<ID3D11InputLayout>,
        blur_vertices: D3dPtr<ID3D11Buffer>,
        blur_cb0: D3dPtr<ID3D11Buffer>,
        terrain_cb1: D3dPtr<ID3D11Buffer>,
        terrain_samplers: BTreeMap<u32, D3dPtr<ID3D11SamplerState>>,
        targets: NativeXmap10ScreenTargets,
        depth_disabled: D3dPtr<ID3D11DepthStencilState>,
        terrain_rasterizer: D3dPtr<ID3D11RasterizerState>,
        fullscreen_rasterizer: D3dPtr<ID3D11RasterizerState>,
        plan: ShadowScreenChainPlan,
        plan_sha256: String,
        content_closure_sha256: String,
        terrain_program_sha256: String,
        draw_fow_cutoff: f32,
        texture_quality: u8,
        admission_sha256: String,
        static_resources: Vec<(D3dPtr<ID3D11Texture2D>, D3dPtr<ID3D11ShaderResourceView>, crate::TreeGpuResourceIdentity)>,
        terrain_blend: D3dPtr<ID3D11BlendState>,
        terrain_depth: D3dPtr<ID3D11DepthStencilState>,
        receipt: Option<Xmap10ScreenShadowExecutionReceipt>,
    }

    pub struct NativeXmap10ShaderResourceView<'a> {
        binding: Xmap10NativeShaderResourceBinding,
        raw_srv: *mut ID3D11ShaderResourceView,
        device_identity: usize,
        shader_register: Option<u32>,
        _owner: std::marker::PhantomData<&'a NativeXmap10ScreenShadowHandoff>,
    }

    impl NativeXmap10ShaderResourceView<'_> {
        pub fn binding(&self) -> &Xmap10NativeShaderResourceBinding {
            &self.binding
        }

        pub fn frame_identity_sha256(&self) -> &str {
            &self.binding.consumer_camera_frame_sha256
        }

        pub fn replay_frame_identity_sha256(&self) -> &str {
            &self.binding.consumer_replay_frame_identity_sha256
        }

        pub fn resource_sha256(&self) -> &str {
            &self.binding.resource_sha256
        }

        pub fn descriptor_sha256(&self) -> &str {
            &self.binding.descriptor_sha256
        }

        pub fn as_raw(&self) -> *mut ID3D11ShaderResourceView {
            self.raw_srv
        }

        pub fn device_identity(&self) -> usize {
            self.device_identity
        }

        pub fn at_shader_register(mut self, shader_register: u32) -> Self {
            self.shader_register = Some(shader_register);
            self
        }
    }

    unsafe impl crate::NativeXmap04UnderwaterShaderResource for NativeXmap10ShaderResourceView<'_> {
        fn owner_packet(&self) -> &str {
            &self.binding.owner_packet
        }
        fn symbol(&self) -> &str {
            &self.binding.symbol
        }
        fn shader_register(&self) -> u32 {
            self.shader_register.unwrap_or(u32::MAX)
        }
        fn frame_identity_sha256(&self) -> &str {
            &self.binding.consumer_camera_frame_sha256
        }
        fn replay_frame_identity_sha256(&self) -> &str {
            &self.binding.consumer_replay_frame_identity_sha256
        }
        fn resource_sha256(&self) -> &str {
            &self.binding.resource_sha256
        }
        fn descriptor_sha256(&self) -> &str {
            &self.binding.descriptor_sha256
        }
        fn raw_shader_resource_view(&self) -> *mut ID3D11ShaderResourceView {
            self.raw_srv
        }
    }

    impl NativeXmap10PostFxExecutor {
        pub unsafe fn create_on_main_scene(
            device: *mut ID3D11Device,
            context: *mut ID3D11DeviceContext,
            resolver: &GameFileResolver,
            frame: &PreparedXmap10Frame,
            main_scene: &NativeProductionMainSceneTarget,
        ) -> Result<Self> {
            if main_scene.device_identity() != device as usize {
                return Err("XMAP10 post-FX MainScene belongs to another D3D11 device".into());
            }
            Self::create_from_main_scene_parts(
                device,
                context,
                main_scene.desc(),
                main_scene.color_texture(),
                main_scene.resource_identity_sha256().to_owned(),
                main_scene.allocation_generation(),
                resolver,
                frame,
            )
        }

        #[allow(clippy::too_many_arguments)]
        unsafe fn create_from_main_scene_parts(
            device_raw: *mut ID3D11Device,
            context_raw: *mut ID3D11DeviceContext,
            main_desc: &ProductionMainSceneTargetDesc,
            main_scene_texture: *mut ID3D11Texture2D,
            main_scene_resource_identity_sha256: String,
            main_scene_allocation_generation: u64,
            resolver: &GameFileResolver,
            frame: &PreparedXmap10Frame,
        ) -> Result<Self> {
            frame.validate()?;
            if !resolver.has_content_admission() {
                return Err("XMAP10 native execution requires an admission-bound VFS".into());
            }
            for key in &frame.content.ordered_used_content {
                resolver.read_admitted_used_content(key)?;
            }
            validate_target_plan_for_desc(frame, main_desc)?;
            if !is_sha256(&main_scene_resource_identity_sha256)
                || main_scene_allocation_generation == 0
            {
                return Err(
                    "XMAP10 post-FX MainScene has no authenticated resource identity".into(),
                );
            }

            if device_raw.is_null() || context_raw.is_null() || main_scene_texture.is_null() {
                return Err("XMAP10 post-FX exposes null D3D11 interfaces".into());
            }
            (*(device_raw as *mut IUnknown)).AddRef();
            (*(context_raw as *mut IUnknown)).AddRef();
            let device = D3dPtr::from_raw(device_raw, "XMAP10 app device")?;
            let context = D3dPtr::from_raw(context_raw, "XMAP10 app context")?;
            validate_context_for_device(&device, context_raw)?;
            validate_texture_for_device(&device, main_scene_texture)?;
            (*(main_scene_texture as *mut IUnknown)).AddRef();
            let retained_main_scene_texture =
                D3dPtr::from_raw(main_scene_texture, "XMAP10 MainScene color texture")?;

            let mut programs = Vec::with_capacity(frame.active_effects.len());
            let mut identities = Vec::with_capacity(frame.active_effects.len());
            for effect in &frame.active_effects {
                let program = compile_program(
                    &device,
                    resolver,
                    &effect.shader_root,
                    &effect.effect_name,
                    effect.role,
                    &effect.ordered_defines,
                )?;
                identities.push(Xmap10CompiledProgramIdentity {
                    role: effect.role,
                    active_effect_sha256: effect.deterministic_sha256.clone(),
                    vertex_bytecode_sha256: Sha256::hex(&program.vertex_bytecode),
                    pixel_bytecode_sha256: Sha256::hex(&program.pixel_bytecode),
                    reflection_sha256: program.reflection_sha256.clone(),
                });
                programs.push((effect.role, program));
            }

            let luminance_effect = frame
                .active_effects
                .iter()
                .find(|effect| effect.role == Xmap10ProgramRole::LuminanceDownsample)
                .ok_or("XMAP10 preparation omitted luminance downsample program")?;
            if !luminance_effect
                .ordered_defines
                .iter()
                .any(|define| define.eq_ignore_ascii_case("LUMINANCE_SAMPLE_ALPHA"))
            {
                return Err("XMAP10 first luminance program omits LUMINANCE_SAMPLE_ALPHA".into());
            }
            let scalar_defines = luminance_effect
                .ordered_defines
                .iter()
                .filter(|define| !define.eq_ignore_ascii_case("LUMINANCE_SAMPLE_ALPHA"))
                .cloned()
                .collect::<Vec<_>>();
            let scalar_downsample = compile_program(
                &device,
                resolver,
                LUMINANCE_SHADER,
                "LuminanceDownsample",
                Xmap10ProgramRole::LuminanceDownsample,
                &scalar_defines,
            )?;

            frame.grade.lut_program.validate()?;
            let lut_blend1 = NativeLutProgram {
                program: compile_lut_program(&device, resolver, "LutBlend1", 2)?,
                effect_name: "LutBlend1".into(),
            };
            let lut_blend2 = NativeLutProgram {
                program: compile_lut_program(&device, resolver, "LutBlend2", 4)?,
                effect_name: "LutBlend2".into(),
            };

            let mut program_receipt = Xmap10NativeProgramReceiptRef {
                schema: XMAP10_NATIVE_PROGRAM_RECEIPT_SCHEMA.into(),
                prepared_frame_sha256: frame.deterministic_sha256.clone(),
                ordered_programs: identities,
                receipt_sha256: String::new(),
            };
            program_receipt.receipt_sha256 = native_program_receipt_digest(
                &program_receipt,
                &scalar_downsample,
                [&lut_blend1, &lut_blend2],
            );

            let main_scene_srv =
                create_srv_for_texture(&device, main_scene_texture, "XMAP10 MainScene SRV")?;
            let half_width = main_desc.color_width / 2;
            let half_height = main_desc.color_height / 2;
            let quarter_width = (half_width + 1) / 2;
            let quarter_height = (half_height + 1) / 2;
            let bright = create_target(
                &device,
                &frame.deterministic_sha256,
                half_width,
                half_height,
                DXGI_FORMAT_R16G16B16A16_FLOAT as u32,
                "XMAP10 BrightPass",
            )?;
            let luminance_ping = [
                create_target(
                    &device,
                    &frame.deterministic_sha256,
                    quarter_width,
                    quarter_height,
                    DXGI_FORMAT_R16_FLOAT as u32,
                    "XMAP10 LuminancePing0",
                )?,
                create_target(
                    &device,
                    &frame.deterministic_sha256,
                    quarter_width,
                    quarter_height,
                    DXGI_FORMAT_R16_FLOAT as u32,
                    "XMAP10 LuminancePing1",
                )?,
            ];
            let temporal = [
                create_target(
                    &device,
                    &frame.deterministic_sha256,
                    1,
                    1,
                    DXGI_FORMAT_R16_FLOAT as u32,
                    "XMAP10 AdaptedLuminance0",
                )?,
                create_target(
                    &device,
                    &frame.deterministic_sha256,
                    1,
                    1,
                    DXGI_FORMAT_R16_FLOAT as u32,
                    "XMAP10 AdaptedLuminance1",
                )?,
            ];
            let bloom_horizontal = create_target(
                &device,
                &frame.deterministic_sha256,
                half_width,
                half_height,
                DXGI_FORMAT_R16G16B16A16_FLOAT as u32,
                "XMAP10 BloomHorizontal",
            )?;
            let restore_bloom = create_target(
                &device,
                &frame.deterministic_sha256,
                half_width,
                half_height,
                DXGI_FORMAT_R16G16B16A16_FLOAT as u32,
                "XMAP10 RestoreBloom",
            )?;

            let mut lut_sources =
                Vec::with_capacity(frame.grade.lut_program.ordered_lut_sources.len());
            for lut in &frame.grade.lut_program.ordered_lut_sources {
                let bytes = resolver.read_admitted_used_content(lut)?;
                let decoded =
                    parse_color_correction_tga(&bytes, &lut.normalized_game_relative_path)?;
                lut_sources.push(upload_lut(&device, &decoded, &lut.payload_sha256)?);
            }
            let color_cubes = [
                create_target(
                    &device,
                    &frame.deterministic_sha256,
                    1024,
                    32,
                    DXGI_FORMAT_R8G8B8A8_UNORM as u32,
                    "XMAP10 ColorCube0",
                )?,
                create_target(
                    &device,
                    &frame.deterministic_sha256,
                    1024,
                    32,
                    DXGI_FORMAT_R8G8B8A8_UNORM as u32,
                    "XMAP10 ColorCube1",
                )?,
                create_target(
                    &device,
                    &frame.deterministic_sha256,
                    1024,
                    32,
                    DXGI_FORMAT_R8G8B8A8_UNORM as u32,
                    "XMAP10 ColorCube2",
                )?,
            ];
            let final_color_cube_index = usize::from(
                frame.grade.lut_program.ordered_passes
                    [usize::from(frame.grade.lut_program.final_pass_ordinal)]
                .output_target_index,
            );
            let first_program = programs
                .first()
                .ok_or("XMAP10 compiled no active programs")?;
            let input_layout =
                create_fullscreen_input_layout(&device, &first_program.1.vertex_bytecode)?;
            let fullscreen_vertices = create_fullscreen_vertices(&device)?;
            let post_effect_cb = create_constant_buffer(
                &device,
                mem::size_of::<PostEffectBaseCb>() as u32,
                "XMAP10 PostEffectBase",
            )?;
            let luminance_cb = create_constant_buffer(
                &device,
                mem::size_of::<LuminanceDownsampleCb>() as u32,
                "XMAP10 luminance vertex/downsample cb2",
            )?;
            let luminance_gather_cb = create_constant_buffer(
                &device,
                mem::size_of::<LuminanceGatherCb>() as u32,
                "XMAP10 luminance gather pixel cb2",
            )?;
            let bloom_cb = create_constant_buffer(
                &device,
                mem::size_of::<BloomCb>() as u32,
                "XMAP10 bloom cb2",
            )?;
            let lut_cb = create_constant_buffer(
                &device,
                mem::size_of::<LutBlendCb>() as u32,
                "XMAP10 LUT cb0",
            )?;
            let linear_clamp = create_sampler(&device, false)?;
            let point_clamp = create_sampler(&device, true)?;
            let opaque_blend = create_blend(&device, false)?;
            let rgb_only_blend = create_blend(&device, true)?;
            let depth_disabled = create_depth_disabled(&device)?;
            let rasterizer = create_rasterizer(&device)?;

            let executor = Self {
                device,
                context,
                prepared_frame_sha256: frame.deterministic_sha256.clone(),
                content_closure_sha256: frame.content.deterministic_sha256.clone(),
                target_plan_sha256: frame.targets.deterministic_sha256.clone(),
                main_scene_resource_identity_sha256,
                main_scene_allocation_generation,
                main_scene_texture: retained_main_scene_texture,
                programs,
                scalar_downsample,
                lut_blend1,
                lut_blend2,
                program_receipt,
                main_scene_srv,
                bright,
                luminance_ping,
                temporal,
                temporal_read: 0,
                bloom_horizontal,
                restore_bloom,
                lut_source_keys: frame.grade.lut_program.ordered_lut_sources.clone(),
                lut_sources,
                color_cubes,
                final_color_cube_index,
                input_layout,
                fullscreen_vertices,
                post_effect_cb,
                luminance_cb,
                luminance_gather_cb,
                bloom_cb,
                lut_cb,
                linear_clamp,
                point_clamp,
                opaque_blend,
                rgb_only_blend,
                depth_disabled,
                rasterizer,
                main_width: main_desc.color_width,
                main_height: main_desc.color_height,
                last_frame_sequence: None,
            };
            executor.bind_common_pipeline();
            Ok(executor)
        }

        pub fn program_receipt(&self) -> &Xmap10NativeProgramReceiptRef {
            &self.program_receipt
        }

        pub fn main_scene_resource_identity_sha256(&self) -> &str {
            &self.main_scene_resource_identity_sha256
        }

        pub unsafe fn prepare_frame(
            &mut self,
            resolver: &GameFileResolver,
            frame: &PreparedXmap10Frame,
        ) -> Result<()> {
            frame.validate()?;
            if !resolver.has_content_admission()
                || frame.targets.deterministic_sha256 != self.target_plan_sha256
                || frame.active_effects.len() != self.program_receipt.ordered_programs.len()
                || frame
                    .active_effects
                    .iter()
                    .zip(&self.program_receipt.ordered_programs)
                    .any(|(effect, program)| {
                        effect.role != program.role
                            || effect.deterministic_sha256 != program.active_effect_sha256
                    })
            {
                return Err(
                    "XMAP10 retained compositor cannot accept a changed shader/target variant"
                        .into(),
                );
            }
            for key in &frame.content.ordered_used_content {
                resolver.read_admitted_used_content(key)?;
            }
            if self.lut_source_keys != frame.grade.lut_program.ordered_lut_sources {
                let mut refreshed =
                    Vec::with_capacity(frame.grade.lut_program.ordered_lut_sources.len());
                for lut in &frame.grade.lut_program.ordered_lut_sources {
                    let bytes = resolver.read_admitted_used_content(lut)?;
                    let decoded =
                        parse_color_correction_tga(&bytes, &lut.normalized_game_relative_path)?;
                    refreshed.push(upload_lut(&self.device, &decoded, &lut.payload_sha256)?);
                }
                self.lut_source_keys = frame.grade.lut_program.ordered_lut_sources.clone();
                self.lut_sources = refreshed;
            }
            self.prepared_frame_sha256 = frame.deterministic_sha256.clone();
            self.content_closure_sha256 = frame.content.deterministic_sha256.clone();
            self.program_receipt.prepared_frame_sha256 = frame.deterministic_sha256.clone();
            self.program_receipt.receipt_sha256 = native_program_receipt_digest(
                &self.program_receipt,
                &self.scalar_downsample,
                [&self.lut_blend1, &self.lut_blend2],
            );
            Ok(())
        }

        #[allow(clippy::too_many_arguments)]
        pub unsafe fn execute(
            &mut self,
            frame: &PreparedXmap10Frame,
            pre_particle_scene: &Xmap10PreParticleSceneReceipt,
            postfx_input: &Xmap10PostFxExecutionInput,
            scene: &NativeXmap10MapCopySceneCb0View<'_>,
            final_rtv: *mut ID3D11RenderTargetView,
            final_target: &Xmap10AttachedFinalTarget,
            reduction: &Xmap10LuminanceReductionPlan,
            temporal: &Xmap10TemporalExecutionInput,
        ) -> Result<Xmap10CompletedPostFxFrame> {
            let assignments = self.validate_execution(
                frame,
                pre_particle_scene,
                postfx_input,
                scene,
                final_rtv,
                final_target,
                reduction,
                temporal,
            )?;

            let kernel = BloomBlurKernel::from_gaussian(
                f32::from_bits(frame.grade.bloom_width_f32_bits),
                f32::from_bits(frame.grade.bloom_scale_f32_bits),
            )?;
            let compositor_receipt = self.compositor_receipt(frame, reduction);
            let _ready = frame
                .admit_native_execution(self.program_receipt.clone(), compositor_receipt.clone())?;

            self.apply_temporal_policy(temporal)?;
            self.bind_common_pipeline();

            let lut_execution = self.execute_lut_program(frame)?;
            lut_execution.validate_against(frame)?;

            let post_cb = post_effect_constants(frame, self.main_width, self.main_height);
            update_constant_buffer(&self.context, &self.device, &self.post_effect_cb, &post_cb)?;
            let scene_native_binding_receipt = native_identity_set(
                "postfx.scene_cb0",
                &[
                    scene.payload_sha256().into(),
                    scene.composition_receipt_sha256().into(),
                    scene.descriptor_sha256().into(),
                    scene.reflected_layout_sha256().into(),
                ],
            );
            let post_cb_receipt =
                native_constant_buffer_receipt("postfx.post_effect_base.cb1", &post_cb.values);
            let common_constant_buffers = vec![scene_native_binding_receipt, post_cb_receipt];

            let scene_buffers = [scene.raw_buffer()];
            let post_buffers = [self.post_effect_cb.as_raw()];
            (*self.context.as_raw()).VSSetConstantBuffers(0, 1, scene_buffers.as_ptr());
            (*self.context.as_raw()).PSSetConstantBuffers(0, 1, scene_buffers.as_ptr());
            (*self.context.as_raw()).VSSetConstantBuffers(1, 1, post_buffers.as_ptr());
            (*self.context.as_raw()).PSSetConstantBuffers(1, 1, post_buffers.as_ptr());

            self.draw_pass(
                Xmap10ProgramRole::BrightPass,
                self.bright.rtv.as_raw(),
                self.bright.width,
                self.bright.height,
                &[self.main_scene_srv.as_raw()],
                &[self.linear_clamp.as_raw()],
                self.opaque_blend.as_raw(),
            )?;
            let bright_program_identity =
                native_program_identity(self.program(Xmap10ProgramRole::BrightPass)?);
            let bright_inputs = vec![self.main_scene_resource_identity_sha256.clone()];
            let bright_output = self.bright.resource_identity_sha256.clone();
            let bright_resources = vec![
                self.main_scene_resource_identity_sha256.clone(),
                bright_output.clone(),
            ];
            let bright_draw = native_draw_receipt(
                &assignments[0],
                "postfx.bright_pass",
                &bright_program_identity,
                &bright_inputs,
                &bright_output,
                &bright_resources,
                &common_constant_buffers,
                self.bright.width,
                self.bright.height,
            );
            let bright_pass = Xmap10TargetBoundPassReceipt::seal(
                frame.hour_index,
                assignments[0].clone(),
                bright_inputs,
                bright_output,
                bright_program_identity,
                bright_resources,
                common_constant_buffers.clone(),
                bright_draw,
            )?;

            let (luminance_chain, luminance_constant_buffers) = self.execute_luminance_reduction(
                reduction,
                &assignments[1],
                &common_constant_buffers,
            )?;
            let luminance_program_identity = native_identity_set(
                "postfx.luminance_downsample_chain.programs",
                &[
                    native_program_identity(self.program(Xmap10ProgramRole::LuminanceDownsample)?),
                    native_program_identity(&self.scalar_downsample),
                ],
            );
            let luminance_inputs = vec![self.bright.resource_identity_sha256.clone()];
            let luminance_output = native_identity_set(
                "postfx.luminance_downsample_chain.outputs",
                &[
                    self.luminance_ping[0].resource_identity_sha256.clone(),
                    self.luminance_ping[1].resource_identity_sha256.clone(),
                ],
            );
            let luminance_resources = vec![
                self.bright.resource_identity_sha256.clone(),
                self.luminance_ping[0].resource_identity_sha256.clone(),
                self.luminance_ping[1].resource_identity_sha256.clone(),
            ];
            let luminance_pass = Xmap10TargetBoundPassReceipt::seal(
                frame.hour_index,
                assignments[1].clone(),
                luminance_inputs,
                luminance_output,
                luminance_program_identity,
                luminance_resources,
                luminance_constant_buffers,
                luminance_chain.packet_chain_receipt_sha256.clone(),
            )?;

            let temporal_write = 1 - self.temporal_read;
            let gather_source_index = (reduction.steps.len() - 1) & 1;
            let gather_inputs = vec![
                self.luminance_ping[gather_source_index]
                    .resource_identity_sha256
                    .clone(),
                self.temporal[self.temporal_read]
                    .resource_identity_sha256
                    .clone(),
            ];
            let gather_output = self.temporal[temporal_write]
                .resource_identity_sha256
                .clone();
            let mut gather_resources = gather_inputs.clone();
            gather_resources.push(gather_output.clone());
            let (gather_draw, gather_constant_buffers) = self.execute_luminance_gather(
                frame,
                reduction,
                temporal,
                temporal_write,
                &assignments[2],
                &common_constant_buffers,
            )?;
            let gather_pass = Xmap10TargetBoundPassReceipt::seal(
                frame.hour_index,
                assignments[2].clone(),
                gather_inputs,
                gather_output,
                native_program_identity(self.program(Xmap10ProgramRole::LuminanceGather)?),
                gather_resources,
                gather_constant_buffers,
                gather_draw,
            )?;

            let bloom_receipts = self.execute_bloom(
                &kernel,
                [&assignments[3], &assignments[4]],
                &common_constant_buffers,
            )?;
            if bloom_receipts.len() != 2 {
                return Err("XMAP10 bloom execution omitted an aggregate pass".into());
            }
            let bloom_horizontal_pass = Xmap10TargetBoundPassReceipt::seal(
                frame.hour_index,
                assignments[3].clone(),
                vec![self.bright.resource_identity_sha256.clone()],
                self.bloom_horizontal.resource_identity_sha256.clone(),
                native_program_identity(self.program(Xmap10ProgramRole::BloomHorizontal)?),
                vec![
                    self.bright.resource_identity_sha256.clone(),
                    self.bloom_horizontal.resource_identity_sha256.clone(),
                ],
                bloom_receipts[0].1.clone(),
                bloom_receipts[0].0.clone(),
            )?;
            let bloom_vertical_pass = Xmap10TargetBoundPassReceipt::seal(
                frame.hour_index,
                assignments[4].clone(),
                vec![self.bloom_horizontal.resource_identity_sha256.clone()],
                self.restore_bloom.resource_identity_sha256.clone(),
                native_program_identity(self.program(Xmap10ProgramRole::BloomVertical)?),
                vec![
                    self.bloom_horizontal.resource_identity_sha256.clone(),
                    self.restore_bloom.resource_identity_sha256.clone(),
                ],
                bloom_receipts[1].1.clone(),
                bloom_receipts[1].0.clone(),
            )?;

            self.draw_pass(
                Xmap10ProgramRole::RestoreScene,
                final_rtv,
                final_target.width,
                final_target.height,
                &[
                    self.main_scene_srv.as_raw(),
                    self.restore_bloom.srv.as_raw(),
                    self.color_cubes[self.final_color_cube_index].srv.as_raw(),
                    self.temporal[temporal_write].srv.as_raw(),
                ],
                &[
                    self.linear_clamp.as_raw(),
                    self.linear_clamp.as_raw(),
                    self.linear_clamp.as_raw(),
                    self.linear_clamp.as_raw(),
                ],
                self.rgb_only_blend.as_raw(),
            )?;
            let restore_inputs = vec![
                self.main_scene_resource_identity_sha256.clone(),
                self.restore_bloom.resource_identity_sha256.clone(),
                lut_execution
                    .final_color_cube_content_identity_sha256
                    .clone(),
                self.temporal[temporal_write]
                    .resource_identity_sha256
                    .clone(),
            ];
            let mut restore_resources = restore_inputs.clone();
            restore_resources.push(final_target.target_identity_sha256.clone());
            let restore_program_identity =
                native_program_identity(self.program(Xmap10ProgramRole::RestoreScene)?);
            let restore_draw = native_draw_receipt(
                &assignments[5],
                "postfx.restore_scene",
                &restore_program_identity,
                &restore_inputs,
                &final_target.target_identity_sha256,
                &restore_resources,
                &common_constant_buffers,
                final_target.width,
                final_target.height,
            );
            let restore_pass = Xmap10TargetBoundPassReceipt::seal(
                frame.hour_index,
                assignments[5].clone(),
                restore_inputs,
                final_target.target_identity_sha256.clone(),
                restore_program_identity,
                restore_resources,
                common_constant_buffers,
                restore_draw,
            )?;

            self.temporal_read = temporal_write;
            self.last_frame_sequence = Some(temporal.frame_sequence);

            let target_bound_passes = vec![
                bright_pass,
                luminance_pass,
                gather_pass,
                bloom_horizontal_pass,
                bloom_vertical_pass,
                restore_pass,
            ];
            let postfx_frame = Xmap10PostFxFrameReceipt::seal(
                postfx_input.foundation_envelope_sha256.clone(),
                frame.hour_index,
                postfx_input.content_manifest_sha256.clone(),
                postfx_input.foundation_content_operation_sha256.clone(),
                postfx_input.viewer_state_sha256.clone(),
                postfx_input.main_scene_target_sha256.clone(),
                self.main_scene_resource_identity_sha256.clone(),
                self.main_scene_allocation_generation,
                postfx_input.map_copy_receipt_set_sha256.clone(),
                postfx_input.upstream_map_scene_receipt_sha256.clone(),
                final_target.clone(),
                lut_execution.deterministic_sha256.clone(),
                &target_bound_passes,
            )?;
            let completed = Xmap10CompletedPostFxFrame {
                postfx_frame,
                target_bound_passes,
                luminance_downsample_chain: luminance_chain,
                lut_execution,
            };
            completed.validate()?;
            Ok(completed)
        }

        #[allow(clippy::too_many_arguments)]
        unsafe fn validate_execution(
            &self,
            frame: &PreparedXmap10Frame,
            pre_particle_scene: &Xmap10PreParticleSceneReceipt,
            postfx_input: &Xmap10PostFxExecutionInput,
            scene: &NativeXmap10MapCopySceneCb0View<'_>,
            final_rtv: *mut ID3D11RenderTargetView,
            final_target: &Xmap10AttachedFinalTarget,
            reduction: &Xmap10LuminanceReductionPlan,
            temporal: &Xmap10TemporalExecutionInput,
        ) -> Result<Vec<AssignedStage>> {
            frame.validate()?;
            if frame.deterministic_sha256 != self.prepared_frame_sha256
                || frame.content.deterministic_sha256 != self.content_closure_sha256
                || frame.targets.deterministic_sha256 != self.target_plan_sha256
                || scene.raw_buffer().is_null()
                || final_rtv.is_null()
            {
                return Err(
                    "XMAP10 executor/frame identity changed or a native binding is null".into(),
                );
            }
            if scene.owner_packet() != "XMAP10"
                || scene.symbol() != "SceneCB0"
                || scene.shader_register() != 0
                || scene.byte_len() != 416
                || scene.payload_sha256() != frame.scene_cb0.scene_cb0_sha256
                || scene.composition_receipt_sha256() != frame.scene_cb0.receipt_sha256
                || scene.device_identity() != self.device.as_raw() as usize
                || scene.render_view_mask() != 1
                || scene.main_scene_resource_identity_sha256()
                    != self.main_scene_resource_identity_sha256
                || scene.main_scene_allocation_generation() != self.main_scene_allocation_generation
                || scene.raw_main_scene_color_texture() != self.main_scene_texture.as_raw()
            {
                return Err(
                    "XMAP10 native SceneCB0/MainScene view is stale or cross-allocation".into(),
                );
            }
            final_target.validate(frame)?;
            reduction.validate_for_main_scene(self.main_width, self.main_height)?;
            temporal.validate(frame)?;
            if let Xmap10TemporalPolicy::EngineResourceReset {
                main_scene_resource_identity_sha256,
                allocation_generation,
                ..
            } = &temporal.policy
            {
                if main_scene_resource_identity_sha256 != &self.main_scene_resource_identity_sha256
                    || *allocation_generation != self.main_scene_allocation_generation
                {
                    return Err(
                        "XMAP10 temporal reset does not name this MainScene allocation".into(),
                    );
                }
            }
            let assignments = postfx_input.validate(frame, pre_particle_scene)?;
            if pre_particle_scene.scene_cb0.sha256 != scene.payload_sha256()
                || pre_particle_scene.scene_cb0.byte_count != scene.byte_len()
            {
                return Err(
                    "XMAP10 native SceneCB0 differs from the pre-particle operation receipt".into(),
                );
            }
            validate_buffer_desc(scene.raw_buffer(), 416, D3D11_BIND_CONSTANT_BUFFER)?;
            validate_final_rtv(self.device.as_raw(), final_rtv, final_target)?;
            Ok(assignments)
        }

        fn apply_temporal_policy(&mut self, input: &Xmap10TemporalExecutionInput) -> Result<()> {
            match &input.policy {
                Xmap10TemporalPolicy::Continue {
                    prior_frame_sequence,
                } => {
                    if self.last_frame_sequence != Some(*prior_frame_sequence) {
                        return Err(
                            "XMAP10 temporal continuation has no matching retained R16 state"
                                .into(),
                        );
                    }
                }
                Xmap10TemporalPolicy::EngineResourceReset { .. } => unsafe {
                    self.temporal_read = 0;
                    let zero = [0.0_f32; 4];
                    for target in &self.temporal {
                        (*self.context.as_raw()).ClearRenderTargetView(target.rtv.as_raw(), &zero);
                    }
                    self.last_frame_sequence = None;
                },
            }
            Ok(())
        }

        unsafe fn bind_common_pipeline(&self) {
            let stride = mem::size_of::<[i32; 2]>() as u32;
            let offset = 0u32;
            let vertex = [self.fullscreen_vertices.as_raw()];
            (*self.context.as_raw()).IASetInputLayout(self.input_layout.as_raw());
            (*self.context.as_raw()).IASetVertexBuffers(0, 1, vertex.as_ptr(), &stride, &offset);
            (*self.context.as_raw()).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            (*self.context.as_raw()).OMSetDepthStencilState(self.depth_disabled.as_raw(), 0);
            (*self.context.as_raw()).RSSetState(self.rasterizer.as_raw());
        }

        unsafe fn execute_lut_program(
            &self,
            frame: &PreparedXmap10Frame,
        ) -> Result<Xmap10LutExecutionReceipt> {
            frame.grade.lut_program.validate()?;
            let mut ordered_passes =
                Vec::with_capacity(frame.grade.lut_program.ordered_passes.len());
            for pass in &frame.grade.lut_program.ordered_passes {
                let target = &self.color_cubes[usize::from(pass.output_target_index)];
                let inputs = match &pass.kind {
                    Xmap10LutBlendPassKind::Copy { source } => vec![source],
                    Xmap10LutBlendPassKind::Blend1 { base, blend, .. } => vec![base, blend],
                    Xmap10LutBlendPassKind::Blend2 {
                        base_day,
                        base_night,
                        blend_day,
                        blend_night,
                        ..
                    } => vec![base_day, base_night, blend_day, blend_night],
                };
                let ordered_input_resource_identity_sha256s = inputs
                    .iter()
                    .map(|input| self.lut_input_resource_identity(&frame.grade.lut_program, input))
                    .collect::<Result<Vec<_>>>()?;
                let program_identity_sha256;
                match &pass.kind {
                    Xmap10LutBlendPassKind::Copy { source } => {
                        program_identity_sha256 = native_identity_set(
                            "postfx.lut.copy-resource",
                            &[frame.grade.lut_program.deterministic_sha256.clone()],
                        );
                        let source_texture =
                            self.lut_input_texture(&frame.grade.lut_program, source)?;
                        (*self.context.as_raw()).CopyResource(
                            target._texture.as_raw() as *mut _,
                            source_texture as *mut _,
                        );
                        native_d3d11_check_void_upload_device(
                            &self.device,
                            "XMAP10 LUT endpoint copy",
                        )?;
                    }
                    Xmap10LutBlendPassKind::Blend1 {
                        base,
                        blend,
                        factor_f32_bits,
                    } => {
                        program_identity_sha256 = native_program_identity(&self.lut_blend1.program);
                        self.execute_lut_draw(
                            &self.lut_blend1.program,
                            target,
                            &[
                                self.lut_input_srv(&frame.grade.lut_program, base)?,
                                self.lut_input_srv(&frame.grade.lut_program, blend)?,
                            ],
                            0.0,
                            f32::from_bits(*factor_f32_bits),
                        )?;
                    }
                    Xmap10LutBlendPassKind::Blend2 {
                        base_day,
                        base_night,
                        blend_day,
                        blend_night,
                        day_night_factor_f32_bits,
                        cross_factor_f32_bits,
                    } => {
                        program_identity_sha256 = native_program_identity(&self.lut_blend2.program);
                        self.execute_lut_draw(
                            &self.lut_blend2.program,
                            target,
                            &[
                                self.lut_input_srv(&frame.grade.lut_program, base_day)?,
                                self.lut_input_srv(&frame.grade.lut_program, base_night)?,
                                self.lut_input_srv(&frame.grade.lut_program, blend_day)?,
                                self.lut_input_srv(&frame.grade.lut_program, blend_night)?,
                            ],
                            f32::from_bits(*day_night_factor_f32_bits),
                            f32::from_bits(*cross_factor_f32_bits),
                        )?;
                    }
                }
                let output_resource_identity_sha256 = target.resource_identity_sha256.clone();
                let mut receipt = Xmap10LutPassExecutionReceipt {
                    ordinal: pass.ordinal,
                    output_target_index: pass.output_target_index,
                    ordered_input_resource_identity_sha256s,
                    output_resource_identity_sha256,
                    program_identity_sha256,
                    execution_receipt_sha256: String::new(),
                };
                receipt.execution_receipt_sha256 = lut_pass_execution_digest(&receipt)?;
                ordered_passes.push(receipt);
            }
            let final_color_cube_content_identity_sha256 = native_identity_set(
                "postfx.color_cube.final-content",
                &[
                    self.color_cubes[self.final_color_cube_index]
                        .resource_identity_sha256
                        .clone(),
                    frame.grade.lut_program.deterministic_sha256.clone(),
                    native_identity_set(
                        "postfx.lut.ordered-pass-executions",
                        &ordered_passes
                            .iter()
                            .map(|pass| pass.execution_receipt_sha256.clone())
                            .collect::<Vec<_>>(),
                    ),
                ],
            );
            let mut receipt = Xmap10LutExecutionReceipt {
                schema: XMAP10_LUT_EXECUTION_RECEIPT_SCHEMA.into(),
                frame_sequence: frame.grade.frame_sequence,
                lut_program_sha256: frame.grade.lut_program.deterministic_sha256.clone(),
                ordered_lut_source_resource_identity_sha256s: self
                    .lut_sources
                    .iter()
                    .map(|source| source.resource_identity_sha256.clone())
                    .collect(),
                ordered_color_cube_resource_identity_sha256s: self
                    .color_cubes
                    .iter()
                    .map(|target| target.resource_identity_sha256.clone())
                    .collect(),
                ordered_passes,
                final_color_cube_content_identity_sha256,
                deterministic_sha256: String::new(),
            };
            receipt.deterministic_sha256 = canonical_packet_sha256(&receipt)?;
            receipt.validate_against(frame)?;
            Ok(receipt)
        }

        fn lut_input_resource_identity(
            &self,
            program: &crate::production_features::xmap10::Xmap10LutBlendProgram,
            input: &Xmap10LutInput,
        ) -> Result<String> {
            match input {
                Xmap10LutInput::Source(index) => self
                    .lut_sources
                    .get(usize::from(*index))
                    .map(|source| source.resource_identity_sha256.clone())
                    .ok_or_else(|| "XMAP10 LUT source identity is unavailable".into()),
                Xmap10LutInput::Intermediate(ordinal) => {
                    let pass = program
                        .ordered_passes
                        .get(usize::from(*ordinal))
                        .ok_or("XMAP10 LUT intermediate identity is unavailable")?;
                    Ok(self.color_cubes[usize::from(pass.output_target_index)]
                        .resource_identity_sha256
                        .clone())
                }
            }
        }

        unsafe fn execute_lut_draw(
            &self,
            program: &NativeProgram,
            target: &NativeTarget,
            srvs: &[*mut ID3D11ShaderResourceView],
            factor1: f32,
            factor2: f32,
        ) -> Result<()> {
            let constants = LutBlendCb {
                values: [
                    1.0 / target.width as f32,
                    1.0 / target.height as f32,
                    factor1,
                    factor2,
                ],
            };
            update_constant_buffer(&self.context, &self.device, &self.lut_cb, &constants)?;
            let buffers = [self.lut_cb.as_raw()];
            (*self.context.as_raw()).VSSetConstantBuffers(0, 1, buffers.as_ptr());
            (*self.context.as_raw()).PSSetConstantBuffers(0, 1, buffers.as_ptr());
            let samplers = vec![self.point_clamp.as_raw(); srvs.len()];
            self.draw_native_program(
                program,
                target.rtv.as_raw(),
                target.width,
                target.height,
                srvs,
                &samplers,
                self.rgb_only_blend.as_raw(),
            )
        }

        fn lut_input_srv(
            &self,
            program: &crate::production_features::xmap10::Xmap10LutBlendProgram,
            input: &Xmap10LutInput,
        ) -> Result<*mut ID3D11ShaderResourceView> {
            match input {
                Xmap10LutInput::Source(index) => self
                    .lut_sources
                    .get(usize::from(*index))
                    .map(|source| source.srv.as_raw())
                    .ok_or_else(|| "XMAP10 LUT source index is unavailable".into()),
                Xmap10LutInput::Intermediate(ordinal) => {
                    let pass = program
                        .ordered_passes
                        .get(usize::from(*ordinal))
                        .ok_or("XMAP10 LUT intermediate pass is unavailable")?;
                    Ok(self.color_cubes[usize::from(pass.output_target_index)]
                        .srv
                        .as_raw())
                }
            }
        }

        fn lut_input_texture(
            &self,
            program: &crate::production_features::xmap10::Xmap10LutBlendProgram,
            input: &Xmap10LutInput,
        ) -> Result<*mut ID3D11Texture2D> {
            match input {
                Xmap10LutInput::Source(index) => self
                    .lut_sources
                    .get(usize::from(*index))
                    .map(|source| source._texture.as_raw())
                    .ok_or_else(|| "XMAP10 LUT source texture is unavailable".into()),
                Xmap10LutInput::Intermediate(ordinal) => {
                    let pass = program
                        .ordered_passes
                        .get(usize::from(*ordinal))
                        .ok_or("XMAP10 LUT intermediate texture is unavailable")?;
                    Ok(self.color_cubes[usize::from(pass.output_target_index)]
                        ._texture
                        .as_raw())
                }
            }
        }

        unsafe fn execute_luminance_reduction(
            &self,
            reduction: &Xmap10LuminanceReductionPlan,
            assigned_stage: &AssignedStage,
            common_constant_buffers: &[String],
        ) -> Result<(Xmap10LuminanceDownsampleChainReceipt, Vec<String>)> {
            let alpha_program = self.program(Xmap10ProgramRole::LuminanceDownsample)?;
            let mut source = self.bright.srv.as_raw();
            let mut source_identity = self.bright.resource_identity_sha256.clone();
            let mut subpasses = Vec::with_capacity(reduction.steps.len());
            let mut aggregate_constant_buffers = common_constant_buffers.to_vec();
            for (index, step) in reduction.steps.iter().enumerate() {
                let destination_index = index & 1;
                let constants = LuminanceDownsampleCb {
                    values: [
                        1.0 / step.destination_width as f32,
                        1.0 / step.destination_height as f32,
                        0.0,
                        0.0,
                        if index == 0 {
                            1.0
                        } else {
                            step.source_width as f32 / self.luminance_ping[0].width as f32
                        },
                        if index == 0 {
                            1.0
                        } else {
                            step.source_height as f32 / self.luminance_ping[0].height as f32
                        },
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                    ],
                };
                update_constant_buffer(
                    &self.context,
                    &self.device,
                    &self.luminance_cb,
                    &constants,
                )?;
                let buffers = [self.luminance_cb.as_raw()];
                (*self.context.as_raw()).VSSetConstantBuffers(2, 1, buffers.as_ptr());
                (*self.context.as_raw()).PSSetConstantBuffers(2, 1, buffers.as_ptr());
                let program = if index == 0 {
                    alpha_program
                } else {
                    &self.scalar_downsample
                };
                let program_identity_sha256 = native_program_identity(program);
                let cbuffer_receipt_sha256 = native_constant_buffer_receipt(
                    &format!("postfx.luminance_downsample_chain.{index}.cb2"),
                    &constants.values,
                );
                let mut ordered_constant_buffers = common_constant_buffers.to_vec();
                ordered_constant_buffers.push(cbuffer_receipt_sha256.clone());
                let output_identity = self.luminance_ping[destination_index]
                    .resource_identity_sha256
                    .clone();
                self.draw_native_program(
                    program,
                    self.luminance_ping[destination_index].rtv.as_raw(),
                    step.destination_width,
                    step.destination_height,
                    &[source],
                    &[self.linear_clamp.as_raw()],
                    self.opaque_blend.as_raw(),
                )?;
                let ordered_inputs = vec![source_identity.clone()];
                let ordered_resources = vec![source_identity.clone(), output_identity.clone()];
                let draw_receipt_sha256 = native_draw_receipt(
                    assigned_stage,
                    &format!("postfx.luminance_downsample_chain.{index}"),
                    &program_identity_sha256,
                    &ordered_inputs,
                    &output_identity,
                    &ordered_resources,
                    &ordered_constant_buffers,
                    step.destination_width,
                    step.destination_height,
                );
                subpasses.push(Xmap10LuminanceDownsampleSubpassReceipt::seal(
                    u8::try_from(index)?,
                    step.source_width,
                    step.source_height,
                    step.destination_width,
                    step.destination_height,
                    draw_receipt_sha256,
                )?);
                aggregate_constant_buffers.push(cbuffer_receipt_sha256);
                source = self.luminance_ping[destination_index].srv.as_raw();
                source_identity = output_identity;
            }
            let first = reduction
                .steps
                .first()
                .ok_or("XMAP10 luminance reduction has no first step")?;
            Ok((
                Xmap10LuminanceDownsampleChainReceipt::seal(
                    assigned_stage.clone(),
                    first.source_width,
                    first.source_height,
                    subpasses,
                )?,
                aggregate_constant_buffers,
            ))
        }

        unsafe fn execute_luminance_gather(
            &self,
            frame: &PreparedXmap10Frame,
            reduction: &Xmap10LuminanceReductionPlan,
            temporal: &Xmap10TemporalExecutionInput,
            temporal_write: usize,
            assigned_stage: &AssignedStage,
            common_constant_buffers: &[String],
        ) -> Result<(String, Vec<String>)> {
            let last = reduction
                .steps
                .last()
                .ok_or("XMAP10 luminance gather has no reduction input")?;
            let source_index = (reduction.steps.len() - 1) & 1;
            let vertex_constants = LuminanceDownsampleCb {
                values: [
                    1.0,
                    1.0,
                    0.0,
                    0.0,
                    last.destination_width as f32 / self.luminance_ping[source_index].width as f32,
                    last.destination_height as f32
                        / self.luminance_ping[source_index].height as f32,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                ],
            };
            let pixel_constants = LuminanceGatherCb {
                values: [
                    1.0 / last.destination_width as f32,
                    1.0 / last.destination_height as f32,
                    match &temporal.policy {
                        Xmap10TemporalPolicy::EngineResourceReset { .. } => 100.0,
                        Xmap10TemporalPolicy::Continue { .. } => {
                            temporal.delta_seconds
                                * f32::from_bits(frame.grade.hdr_adjustment_speed_f32_bits)
                        }
                    },
                    0.0,
                    last.destination_width as f32,
                    last.destination_height as f32,
                    1.0 / self.luminance_ping[source_index].width as f32,
                    1.0 / self.luminance_ping[source_index].height as f32,
                    f32::from_bits(frame.grade.min_hdr_adjustment_f32_bits),
                    f32::from_bits(frame.grade.max_hdr_adjustment_f32_bits),
                    0.0,
                    0.0,
                ],
            };
            update_constant_buffer(
                &self.context,
                &self.device,
                &self.luminance_cb,
                &vertex_constants,
            )?;
            update_constant_buffer(
                &self.context,
                &self.device,
                &self.luminance_gather_cb,
                &pixel_constants,
            )?;
            let vertex_buffers = [self.luminance_cb.as_raw()];
            let pixel_buffers = [self.luminance_gather_cb.as_raw()];
            (*self.context.as_raw()).VSSetConstantBuffers(2, 1, vertex_buffers.as_ptr());
            (*self.context.as_raw()).PSSetConstantBuffers(2, 1, pixel_buffers.as_ptr());
            let vertex_cbuffer_receipt_sha256 = native_constant_buffer_receipt(
                "postfx.luminance_gather.vs_cb2",
                &vertex_constants.values,
            );
            let pixel_cbuffer_receipt_sha256 = native_constant_buffer_receipt(
                "postfx.luminance_gather.ps_cb2",
                &pixel_constants.values,
            );
            let mut ordered_constant_buffers = common_constant_buffers.to_vec();
            ordered_constant_buffers.push(vertex_cbuffer_receipt_sha256);
            ordered_constant_buffers.push(pixel_cbuffer_receipt_sha256);
            self.draw_pass(
                Xmap10ProgramRole::LuminanceGather,
                self.temporal[temporal_write].rtv.as_raw(),
                1,
                1,
                &[
                    self.luminance_ping[source_index].srv.as_raw(),
                    self.temporal[self.temporal_read].srv.as_raw(),
                ],
                &[self.linear_clamp.as_raw(), self.point_clamp.as_raw()],
                self.opaque_blend.as_raw(),
            )?;
            let input_identities = vec![
                self.luminance_ping[source_index]
                    .resource_identity_sha256
                    .clone(),
                self.temporal[self.temporal_read]
                    .resource_identity_sha256
                    .clone(),
            ];
            let output_identity = self.temporal[temporal_write]
                .resource_identity_sha256
                .clone();
            let mut resource_identities = input_identities.clone();
            resource_identities.push(output_identity.clone());
            let draw_receipt_sha256 = native_draw_receipt(
                assigned_stage,
                "postfx.luminance_gather",
                &native_program_identity(self.program(Xmap10ProgramRole::LuminanceGather)?),
                &input_identities,
                &output_identity,
                &resource_identities,
                &ordered_constant_buffers,
                1,
                1,
            );
            Ok((draw_receipt_sha256, ordered_constant_buffers))
        }

        unsafe fn execute_bloom(
            &self,
            kernel: &BloomBlurKernel,
            assigned_stages: [&AssignedStage; 2],
            common_constant_buffers: &[String],
        ) -> Result<Vec<(String, Vec<String>)>> {
            let mut receipts = Vec::with_capacity(2);
            for (index, (role, axis, source, source_identity, target)) in [
                (
                    Xmap10ProgramRole::BloomHorizontal,
                    1.0,
                    self.bright.srv.as_raw(),
                    self.bright.resource_identity_sha256.as_str(),
                    &self.bloom_horizontal,
                ),
                (
                    Xmap10ProgramRole::BloomVertical,
                    0.0,
                    self.bloom_horizontal.srv.as_raw(),
                    self.bloom_horizontal.resource_identity_sha256.as_str(),
                    &self.restore_bloom,
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let constants = BloomCb {
                    values: [
                        1.0 / target.width as f32,
                        1.0 / target.height as f32,
                        axis,
                        kernel.weight0,
                        kernel.weights[0],
                        kernel.weights[1],
                        kernel.weights[2],
                        kernel.weights[3],
                        kernel.offsets[0],
                        kernel.offsets[1],
                        kernel.offsets[2],
                        kernel.offsets[3],
                    ],
                };
                update_constant_buffer(&self.context, &self.device, &self.bloom_cb, &constants)?;
                let buffers = [self.bloom_cb.as_raw()];
                (*self.context.as_raw()).VSSetConstantBuffers(2, 1, buffers.as_ptr());
                (*self.context.as_raw()).PSSetConstantBuffers(2, 1, buffers.as_ptr());
                let cbuffer_receipt_sha256 = native_constant_buffer_receipt(
                    if index == 0 {
                        "postfx.bloom.horizontal.cb2"
                    } else {
                        "postfx.bloom.vertical.cb2"
                    },
                    &constants.values,
                );
                let mut ordered_constant_buffers = common_constant_buffers.to_vec();
                ordered_constant_buffers.push(cbuffer_receipt_sha256);
                self.draw_pass(
                    role,
                    target.rtv.as_raw(),
                    target.width,
                    target.height,
                    &[source],
                    &[self.linear_clamp.as_raw()],
                    self.opaque_blend.as_raw(),
                )?;
                let input_identities = vec![source_identity.to_owned()];
                let output_identity = target.resource_identity_sha256.clone();
                let resource_identities = vec![source_identity.to_owned(), output_identity.clone()];
                receipts.push((
                    native_draw_receipt(
                        assigned_stages[index],
                        if index == 0 {
                            "postfx.bloom.horizontal"
                        } else {
                            "postfx.bloom.vertical"
                        },
                        &native_program_identity(self.program(role)?),
                        &input_identities,
                        &output_identity,
                        &resource_identities,
                        &ordered_constant_buffers,
                        target.width,
                        target.height,
                    ),
                    ordered_constant_buffers,
                ));
            }
            Ok(receipts)
        }

        unsafe fn draw_pass(
            &self,
            role: Xmap10ProgramRole,
            rtv: *mut ID3D11RenderTargetView,
            width: u32,
            height: u32,
            srvs: &[*mut ID3D11ShaderResourceView],
            samplers: &[*mut ID3D11SamplerState],
            blend: *mut ID3D11BlendState,
        ) -> Result<()> {
            let program = self.program(role)?;
            self.draw_native_program(program, rtv, width, height, srvs, samplers, blend)
        }

        unsafe fn draw_native_program(
            &self,
            program: &NativeProgram,
            rtv: *mut ID3D11RenderTargetView,
            width: u32,
            height: u32,
            srvs: &[*mut ID3D11ShaderResourceView],
            samplers: &[*mut ID3D11SamplerState],
            blend: *mut ID3D11BlendState,
        ) -> Result<()> {
            if rtv.is_null() || width == 0 || height == 0 || srvs.iter().any(|srv| srv.is_null()) {
                return Err("XMAP10 draw received a null target/resource or empty viewport".into());
            }
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: width as f32,
                Height: height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            (*self.context.as_raw()).OMSetRenderTargets(1, &rtv, ptr::null_mut());
            (*self.context.as_raw()).RSSetViewports(1, &viewport);
            (*self.context.as_raw()).RSSetScissorRects(0, ptr::null());
            (*self.context.as_raw()).OMSetBlendState(blend, &[0.0; 4], u32::MAX);
            (*self.context.as_raw()).VSSetShader(program.vertex.as_raw(), ptr::null_mut(), 0);
            (*self.context.as_raw()).PSSetShader(program.pixel.as_raw(), ptr::null_mut(), 0);
            (*self.context.as_raw()).PSSetShaderResources(0, srvs.len() as u32, srvs.as_ptr());
            (*self.context.as_raw()).PSSetSamplers(0, samplers.len() as u32, samplers.as_ptr());
            (*self.context.as_raw()).Draw(4, 0);
            let nulls = vec![ptr::null_mut(); srvs.len()];
            (*self.context.as_raw()).PSSetShaderResources(0, nulls.len() as u32, nulls.as_ptr());
            native_d3d11_check_void_upload_device(&self.device, "XMAP10 postfx draw")
        }

        fn program(&self, role: Xmap10ProgramRole) -> Result<&NativeProgram> {
            self.programs
                .iter()
                .find(|(candidate, _)| *candidate == role)
                .map(|(_, program)| program)
                .ok_or_else(|| format!("XMAP10 compiled program set omitted {role:?}").into())
        }

        fn compositor_receipt(
            &self,
            frame: &PreparedXmap10Frame,
            reduction: &Xmap10LuminanceReductionPlan,
        ) -> Xmap10NativeCompositorReceiptRef {
            let mut bytes = Vec::new();
            push_string(&mut bytes, &frame.deterministic_sha256);
            push_string(&mut bytes, &frame.targets.deterministic_sha256);
            push_string(&mut bytes, &reduction.deterministic_sha256);
            for pass in native_pass_names(
                frame.grade.lut_program.ordered_passes.len(),
                reduction.steps.len(),
            ) {
                push_string(&mut bytes, &pass);
            }
            Xmap10NativeCompositorReceiptRef {
                schema: XMAP10_NATIVE_COMPOSITOR_RECEIPT_SCHEMA.into(),
                prepared_frame_sha256: frame.deterministic_sha256.clone(),
                target_plan_sha256: frame.targets.deterministic_sha256.clone(),
                ordered_pass_count: native_pass_names(
                    frame.grade.lut_program.ordered_passes.len(),
                    reduction.steps.len(),
                )
                .len() as u16,
                receipt_sha256: Sha256::hex(&bytes),
            }
        }
    }

    impl NativeXmap10ScreenShadowHandoff {
        pub fn retained_before_frame(&self, frame: &Xmap10ShadowHistoryFrame)
            -> std::result::Result<NativeXmap10RetainedShadowView<'_>, Xmap10RetainedShadowRefusal> {
            self.targets.retained_before_frame(frame)
        }

        pub unsafe fn create_on_device(
            device: *mut ID3D11Device,
            resolver: &GameFileResolver,
            terrain: &PreparedTerrainFrame,
            plan: ShadowScreenChainPlan,
            allocation_generation: u64,
            admission: &replay_core::ValidatedContentAdmission,
            texture_quality: u8,
        ) -> Result<Self> {
            terrain.validate()?;
            if device.is_null() || !resolver.has_content_admission() {
                return Err(
                    "XMAP10 screen shadow requires a live device and admission-bound VFS".into(),
                );
            }
            validate_screen_shadow_plan(&plan)?;
            (*(device as *mut IUnknown)).AddRef();
            let device = D3dPtr::from_raw(device, "XMAP10 screen-shadow device")?;
            let plan_sha256 = Sha256::hex(&serde_json::to_vec(&plan)?);
            let (terrain_unlit, terrain_input_layout) =
                compile_terrain_unlit(&device, resolver, terrain)?;
            let blur_horizontal = compile_shadow_blur(&device, resolver, "ShadowBlurHorizontal")?;
            let blur_vertical = compile_shadow_blur(&device, resolver, "ShadowBlurVertical")?;
            let blur_input_layout =
                create_shadow_blur_input_layout(&device, &blur_horizontal.program.vertex_bytecode)?;
            let blur_vertices = create_shadow_blur_vertices(&device)?;
            let blur_cb0 = create_constant_buffer(&device, 32, "XMAP10 shadow blur cb0")?;
            let terrain_cb1 = create_constant_buffer(
                &device,
                u32::try_from(crate::TERRAIN_CHUNK_CB1_BYTES)?,
                "XMAP10 terrainunlit cb1",
            )?;

            let mut terrain_samplers = BTreeMap::new();
            for slot in parse_dxbc_reflection(&terrain_unlit.pixel_bytecode)?.resources.iter().filter(|r| r.resource_type == 3).map(|r| r.bind_point) {
                let sampler = terrain
                    .program
                    .samplers
                    .iter()
                    .find(|sampler| sampler.shader_register == slot)
                    .ok_or_else(|| {
                        format!("XMAP10 terrainunlit requires admitted sampler s{slot}")
                    })?;
                terrain_samplers.insert(
                    slot,
                    crate::xmap04_d3d_handoff::create_underwater_sampler(device.as_raw(), sampler)?,
                );
            }
            if texture_quality > 2 { return Err("XMAP10 fog texture quality is outside 0..=2".into()); }
            let mut static_resources = Vec::new();
            for (slot, semantic, path) in [
                (2, "FOWNoise_Texture", format!("map/terrain/fow_noise_{texture_quality}.dds")),
                (3, "FOWHeight_Texture", "map/terrain/fow_rgb_waterspec_a.dds".into()),
                (9, "EnvironmentMap", "map/terrain/reflection.dds".into()),
            ] {
                let content = admission.require_used_content(&path)?;
                let (texture, view, descriptor_sha256) = if slot == 9 {
                    crate::xmap12_d3d_handoff::create_admitted_environment_cube_on_device(device.as_raw(), resolver, admission, content)?
                } else {
                    crate::xmap12_d3d_handoff::create_admitted_material_dds_on_device(device.as_raw(), resolver, admission, content)?
                };
                static_resources.push((texture, view, crate::TreeGpuResourceIdentity { semantic: semantic.into(), slot,
                    content_payload_sha256: content.payload_sha256.clone(), descriptor_sha256 }));
            }
            let files = crate::shader_compile::effect_translation_unit_files(resolver, "gfx/FX/pdxmap.shader")?;
            let root = &files.last().ok_or("terrainunlit translation unit is empty")?.1;
            if root.effect_count("terrainunlit") != 1 { return Err("terrainunlit effect selection is ambiguous".into()); }
            let effect = root.effect("terrainunlit").ok_or("terrainunlit effect is absent")?;
            if effect.depth_stencil_state.is_some() || effect.rasterizer_state.is_some()
                || files.iter().any(|(_, file)| !file.depth_stencil_states.is_empty() || !file.rasterizer_states.is_empty()) {
                return Err("XMAP10 terrainunlit declares depth/raster state outside the verified default producer".into());
            }
            let blend_name = effect.blend_state.as_deref().unwrap_or("BlendState");
            let blend_rows = files.iter().rev().find_map(|(_, file)| file.blend_states.get(blend_name))
                .map(Vec::as_slice).unwrap_or(&[]);
            let terrain_blend = crate::xmap12_d3d_handoff::create_blend_state(device.as_raw(), blend_rows)?;
            let terrain_depth = create_terrain_unlit_depth(&device)?;
            let targets = NativeXmap10ScreenTargets::create(&device, &plan, allocation_generation)?;
            let terrain_rasterizer = create_terrain_rasterizer(device.as_raw())?;
            let depth_disabled = create_depth_disabled(&device)?;
            let fullscreen_rasterizer = create_rasterizer(&device)?;
            Ok(Self {
                device,
                terrain_unlit,
                terrain_input_layout,
                blur_horizontal,
                blur_vertical,
                blur_input_layout,
                blur_vertices,
                blur_cb0,
                terrain_cb1,
                terrain_samplers,
                targets,
                depth_disabled,
                terrain_rasterizer,
                fullscreen_rasterizer,
                plan,
                plan_sha256,
                content_closure_sha256: Sha256::hex(&serde_json::to_vec(&(admission.manifest_digest_sha256(), &terrain.program.deterministic_sha256, texture_quality))?),
                admission_sha256: admission.manifest_digest_sha256().to_owned(), texture_quality,
                static_resources, terrain_blend, terrain_depth,
                terrain_program_sha256: terrain.program.deterministic_sha256.clone(),
                draw_fow_cutoff: crate::xmap09_lighting_law::EffectiveDefines::from_resolver(resolver)?.scalar("NGraphics", "DRAW_FOW_CUTOFF")?,
                receipt: None,
            })
        }

        pub fn allocation_matches(&self, device: *mut ID3D11Device, terrain: &PreparedTerrainFrame,
            plan: &ShadowScreenChainPlan, admission_sha256: &str, texture_quality: u8) -> bool {
            let a = &self.plan.producer_and_final_target;
            let b = &plan.producer_and_final_target;
            self.admission_sha256 == admission_sha256 && self.texture_quality == texture_quality
                && self.device.as_raw() == device && self.terrain_program_sha256 == terrain.program.deterministic_sha256
                && (a.width, a.height, a.dxgi_format, a.mip_levels, a.bind_flags)
                    == (b.width, b.height, b.dxgi_format, b.mip_levels, b.bind_flags)
        }

        pub fn update_plan(&mut self, terrain: &PreparedTerrainFrame, plan: ShadowScreenChainPlan) -> Result<()> {
            validate_screen_shadow_plan(&plan)?;
            if !self.allocation_matches(self.device.as_raw(), terrain, &plan, &self.admission_sha256, self.texture_quality) {
                return Err("XMAP10 screen-shadow plan requires allocation recreation".into());
            }
            self.plan_sha256 = Sha256::hex(&serde_json::to_vec(&plan)?);
            self.plan = plan;
            Ok(())
        }

        pub fn content_identity_sha256(&self) -> &str { &self.content_closure_sha256 }

        pub fn producer_active(&self, light_active: bool, camera_height: f32) -> Result<bool> {
            if !camera_height.is_finite() || !self.draw_fow_cutoff.is_finite() {
                return Err("XMAP10 screen-shadow gate has a non-finite camera/define".into());
            }
            Ok(light_active || camera_height < self.draw_fow_cutoff)
        }

        pub unsafe fn execute(
            &mut self, context: *mut ID3D11DeviceContext,
            terrain: &PreparedTerrainFrame, terrain_handoff: &NativeXmap04TerrainHandoff,
            scene_cb0s: &[&NativeXmap10MapCopySceneCb0View<'_>],
            lighting: &crate::PreparedXmap09LightingFrame,
            light_depth: Option<&NativeXmap09ShaderResourceView<'_>>,
            light_depth_execution: &crate::Xmap09LightDepthExecutionReceipt,
            history_frame: &Xmap10ShadowHistoryFrame, viewer_sha256: &str, camera_height: f32,
            trees: Option<(&crate::PreparedTreeFrame, &crate::NativeXmap11ResourceOwner)>,
            shared: &[crate::NativeTreeShaderResource], point_lights: *mut ID3D11Buffer,
        ) -> Result<&Xmap10ScreenShadowExecutionReceipt> {
            use crate::NativeXmap04UnderwaterSceneConstantBuffer;
            use winapi::shared::dxgiformat::DXGI_FORMAT_R16_UINT;
            use winapi::um::d3dcommon::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
            validate_context_for_device(&self.device, context)?;
            self.receipt = None;
            self.targets.begin_write(context, history_frame)?;
            let write_guard = Xmap10ShadowWriteGuard::new(&self.targets.history);
            terrain.validate()?;
            lighting.validate()?;
            light_depth_execution.validate()?;
            if terrain.program.deterministic_sha256 != self.terrain_program_sha256
                || terrain_handoff.receipt().terrain_frame_sha256 != terrain.deterministic_sha256
                || history_frame.camera_sha256 != terrain.camera_frame_sha256
                || history_frame.replay_sha256 != terrain.replay_frame_identity_sha256
                || history_frame.content_sha256 != self.content_closure_sha256
                || lighting.input_payload_sha256 != light_depth_execution.input_payload_sha256 {
                return Err("XMAP10 screen-shadow frame/content custody mismatch".into());
            }
            let mut screen_resources = self.static_resources.iter().map(|(_, view, identity)| crate::NativeTreeShaderResource {
                identity: identity.clone(), view: view.as_raw(),
            }).collect::<Vec<_>>();
            screen_resources.extend(shared.iter().map(|row| crate::NativeTreeShaderResource { identity: row.identity.clone(), view: row.view }));
            let shared = screen_resources.as_slice();
            let rows = terrain.ordered_map_copy_receipts()?;
            let map_copy_receipt_set_sha256 = terrain.map_copy_receipt_set_sha256()?;
            if scene_cb0s.len() != rows.len() || scene_cb0s.iter().zip(&rows).any(|(scene, row)|
                scene.device_identity() != self.device.as_raw() as usize || scene.raw_buffer().is_null()
                || scene.owner_packet() != "XMAP10" || scene.symbol() != "SceneCB0"
                || scene.shader_register() != 0 || scene.byte_len() != 416
                || scene.frame_identity_sha256() != terrain.camera_frame_sha256
                || scene.replay_frame_identity_sha256() != terrain.replay_frame_identity_sha256
                || scene.map_copy_receipt_set_sha256() != map_copy_receipt_set_sha256
                || scene.render_view_mask() != row.render_view_mask
                || scene.map_copy_offset_f32_bits() != row.map_copy_offset_f32_bits) {
                return Err("XMAP10 screen-shadow partition SceneCB0 census mismatch".into());
            }
            let producer_active = self.producer_active(light_depth_execution.active, camera_height)?;
            let mut tree_partitions = Vec::new();
            let mut ordered_partition_matrix_sha256s = Vec::new();
            let width = self.plan.producer_and_final_target.width;
            let height = self.plan.producer_and_final_target.height;
            let mut draw_count = 0u32;
            if producer_active {
                let light_depth = light_depth.ok_or(Xmap10RetainedShadowRefusal::UpstreamCasterUnavailable)?;
                if light_depth.binding != light_depth_execution.binding.clone().ok_or(Xmap10RetainedShadowRefusal::UpstreamCasterUnavailable)?
                    || light_depth.binding.camera_frame_sha256 != history_frame.camera_sha256
                    || light_depth.binding.replay_frame_identity_sha256 != history_frame.replay_sha256 {
                    return Err("XMAP10 screen-shadow light-depth identity mismatch".into());
                }
                let (tree_frame, tree_owner) = trees.ok_or(Xmap10RetainedShadowRefusal::ScreenTreeProducerUnavailable)?;
                let mut terrain_views = [ptr::null_mut(); 16];
                for (target_slot, source_slot) in [(0, 2u8), (1, 3), (2, 4), (7, 6)] {
                    let candidates = shared.iter().filter(|r| r.identity.slot == source_slot).collect::<Vec<_>>();
                    if candidates.len() != 1 || candidates[0].view.is_null() {
                        return Err(format!("XMAP10 terrainunlit missing exact fog/province provider t{target_slot}").into());
                    }
                    terrain_views[target_slot] = candidates[0].view;
                }
                terrain_views[12] = light_depth.as_raw();
                let enabled_mask = rows.iter().fold(0, |mask, row| mask | row.render_view_mask);
                for scene in scene_cb0s {
                    let matrix = lighting.shadow_map_texture_matrix_for_partition(enabled_mask, scene.render_view_mask())?;
                    let lease = NativeXmap10ScreenWriteLease { device: self.device.as_raw(), context,
                        color_rtv: self.targets.final_target.rtv.as_raw(), depth_dsv: self.targets.depth.as_raw(),
                        width, height, frame: history_frame, viewer_sha256,
                        map_copy_sha256: &map_copy_receipt_set_sha256, _owner: std::marker::PhantomData };
                    let tree_receipt = tree_owner.draw_unlit_partition(tree_frame, &lease, scene, &matrix, shared, point_lights)?;
                    draw_count = draw_count.checked_add(u32::try_from(tree_receipt.ordered_batch_indices.len())?)
                        .ok_or("XMAP10 screen draw count overflow")?;
                    tree_partitions.push(tree_receipt);
                    ordered_partition_matrix_sha256s.push(matrix.payload_sha256.clone());
            (*context).RSSetScissorRects(0, ptr::null());
            (*context).RSSetState(self.terrain_rasterizer.as_raw());
            (*context).OMSetBlendState(self.terrain_blend.as_raw(), &[0.0; 4], u32::MAX);
            (*context).OMSetDepthStencilState(self.terrain_depth.as_raw(), 0);
            (*context).IASetInputLayout(self.terrain_input_layout.as_raw());
            (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            (*context).IASetIndexBuffer(
                terrain_handoff.xmap10_shared_index(),
                DXGI_FORMAT_R16_UINT,
                0,
            );
            (*context).VSSetShader(self.terrain_unlit.vertex.as_raw(), ptr::null_mut(), 0);
            (*context).PSSetShader(self.terrain_unlit.pixel.as_raw(), ptr::null_mut(), 0);
                    (*context).PSSetShaderResources(0, 16, terrain_views.as_ptr());
                    for (slot, sampler) in &self.terrain_samplers { (*context).PSSetSamplers(*slot, 1, &sampler.as_raw()); }
                    let slot0 = terrain_handoff.xmap10_shared_slot0();
                    let mut strides = [crate::TERRAIN_CHUNK_SLOT0_STRIDE, crate::TERRAIN_CHUNK_SLOT1_STRIDE];
                    let mut offsets = [0u32; 2];
                    for (draw_index, draw) in terrain.draws.iter().enumerate().filter(|(_, draw)| draw.render_view_mask == scene.render_view_mask()) {
                let slot1 = terrain_handoff.xmap10_draw_slot1(draw_index)?;
                let buffers = [slot0, slot1];
                (*context).IASetVertexBuffers(
                    0,
                    2,
                    buffers.as_ptr(),
                    strides.as_mut_ptr(),
                    offsets.as_mut_ptr(),
                );
                let scene_raw = scene.raw_buffer();
                (*context).VSSetConstantBuffers(0, 1, &scene_raw);
                (*context).PSSetConstantBuffers(0, 1, &scene_raw);
                let joined_cb1 = xmap10_terrain_unlit_cb1_bytes(&draw.cb1, &matrix)?;
                (*context).UpdateSubresource(
                    self.terrain_cb1.as_raw() as *mut _,
                    0,
                    ptr::null(),
                    joined_cb1.as_ptr() as *const _,
                    0,
                    0,
                );
                let terrain_cb1 = self.terrain_cb1.as_raw();
                (*context).VSSetConstantBuffers(1, 1, &terrain_cb1);
                (*context).PSSetConstantBuffers(1, 1, &terrain_cb1);
                (*context).DrawIndexed(crate::TERRAIN_CHUNK_INDEX_COUNT, 0, 0);
                        draw_count = draw_count.checked_add(1).ok_or("XMAP10 screen draw count overflow")?;
                    }
                }
                let actual_batches = tree_partitions.iter().flat_map(|r| r.ordered_batch_indices.iter().copied()).collect::<Vec<_>>();
                if actual_batches != tree_frame.ordered_main_draw_batch_indices {
                    return Err("XMAP10 screen-shadow tree census is incomplete or reordered".into());
                }
            }
            let nulls = [ptr::null_mut(); 16];
            (*context).PSSetShaderResources(0, 16, nulls.as_ptr());
            if light_depth_execution.active {
                self.draw_shadow_blur(context, &self.blur_horizontal, &self.targets.final_target,
                    &self.targets.intermediate_target, self.plan.passes[1].cb0_f32_bits.ok_or("XMAP10 horizontal blur constants missing")?)?;
                self.draw_shadow_blur(context, &self.blur_vertical, &self.targets.intermediate_target,
                    &self.targets.final_target, self.plan.passes[2].cb0_f32_bits.ok_or("XMAP10 vertical blur constants missing")?)?;
            }
            (*context).OMSetRenderTargets(0, ptr::null(), ptr::null_mut());
            native_d3d11_check_void_upload_device(&self.device, "XMAP10 complete tree/terrain screen write")?;
            let primary_matrix = lighting.shadow_map_texture_matrix()?;
            let binding = Xmap10NativeShaderResourceBinding {
                symbol: "ShadowMap".into(), owner_packet: "XMAP10".into(),
                producer_camera_frame_sha256: history_frame.camera_sha256.clone(),
                producer_replay_frame_identity_sha256: history_frame.replay_sha256.clone(),
                consumer_camera_frame_sha256: history_frame.camera_sha256.clone(),
                consumer_replay_frame_identity_sha256: history_frame.replay_sha256.clone(),
                stable_resource_id: format!("xmap10/screen-shadow/{}/{}", self.targets.history.allocation_generation, self.targets.history.descriptor_sha256),
                resource_sha256: Sha256::hex(&serde_json::to_vec(&(history_frame, &self.plan_sha256, &tree_partitions,
                    &ordered_partition_matrix_sha256s, &light_depth_execution.deterministic_sha256, XMAP10_SCREEN_CLEAR_BITS, draw_count))?),
                descriptor_sha256: self.targets.history.descriptor_sha256.clone(),
            };
            let mut receipt = Xmap10ScreenShadowExecutionReceipt {
                schema: XMAP10_SCREEN_SHADOW_RECEIPT_SCHEMA.into(), active: producer_active,
                cleared_color_f32_bits: XMAP10_SCREEN_CLEAR_BITS, tree_partitions, ordered_partition_matrix_sha256s,
                terrain_frame_sha256: terrain.deterministic_sha256.clone(), map_copy_receipt_set_sha256,
                xmap09_light_depth_execution_sha256: light_depth_execution.deterministic_sha256.clone(),
                xmap09_shadow_matrix_payload_sha256: primary_matrix.payload_sha256,
                screen_chain_plan_sha256: self.plan_sha256.clone(), content_closure_sha256: self.content_closure_sha256.clone(),
                ordered_scene_cb0_composition_sha256s: if producer_active { scene_cb0s.iter().map(|s| s.composition_receipt_sha256().to_owned()).collect() } else { Vec::new() },
                terrain_unlit_vertex_bytecode_sha256: Sha256::hex(&self.terrain_unlit.vertex_bytecode),
                terrain_unlit_pixel_bytecode_sha256: Sha256::hex(&self.terrain_unlit.pixel_bytecode),
                blur_horizontal_vertex_bytecode_sha256: Sha256::hex(&self.blur_horizontal.program.vertex_bytecode),
                blur_horizontal_pixel_bytecode_sha256: Sha256::hex(&self.blur_horizontal.program.pixel_bytecode),
                blur_vertical_vertex_bytecode_sha256: Sha256::hex(&self.blur_vertical.program.vertex_bytecode),
                blur_vertical_pixel_bytecode_sha256: Sha256::hex(&self.blur_vertical.program.pixel_bytecode),
                draw_count, binding, deterministic_sha256: String::new(),
            };
            receipt.deterministic_sha256 = canonical_packet_sha256(&receipt)?;
            receipt.validate()?;
            self.targets.history.complete(history_frame, receipt.deterministic_sha256.clone())?;
            write_guard.finish();
            self.receipt = Some(receipt);
            Ok(self.receipt.as_ref().expect("screen write receipt stored"))
        }

        unsafe fn draw_shadow_blur(
            &self,
            context: *mut ID3D11DeviceContext,
            program: &NativeShadowBlurProgram,
            source: &NativeTarget,
            target: &NativeTarget,
            cb0_f32_bits: [u32; 8],
        ) -> Result<()> {
            let constants = cb0_f32_bits.map(f32::from_bits);
            (*context).UpdateSubresource(
                self.blur_cb0.as_raw() as *mut _,
                0,
                ptr::null(),
                constants.as_ptr() as *const _,
                0,
                0,
            );
            native_d3d11_check_void_upload_device(
                &self.device,
                "UpdateSubresource(XMAP10 shadow blur cb0)",
            )?;
            let rtv = target.rtv.as_raw();
            let viewport = D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: target.width as f32,
                Height: target.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            };
            (*context).OMSetRenderTargets(1, &rtv, ptr::null_mut());
            (*context).RSSetViewports(1, &viewport);
            (*context).RSSetState(self.fullscreen_rasterizer.as_raw());
            (*context).OMSetDepthStencilState(self.depth_disabled.as_raw(), 0);
            (*context).IASetInputLayout(self.blur_input_layout.as_raw());
            (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            let vertices = [self.blur_vertices.as_raw()];
            let strides = [24u32];
            let offsets = [0u32];
            (*context).IASetVertexBuffers(
                0,
                1,
                vertices.as_ptr(),
                strides.as_ptr() as *mut _,
                offsets.as_ptr() as *mut _,
            );
            program.bind(context, self.blur_cb0.as_raw());
            let srv = source.srv.as_raw();
            (*context).PSSetShaderResources(0, 1, &srv);
            (*context).Draw(4, 0);
            let null_srv = ptr::null_mut();
            (*context).PSSetShaderResources(0, 1, &null_srv);
            Ok(())
        }

        pub fn receipt(&self) -> Option<&Xmap10ScreenShadowExecutionReceipt> {
            self.receipt.as_ref()
        }

        pub fn shadow_map_view(&self) -> Result<NativeXmap10ShaderResourceView<'_>> {
            let receipt = self
                .receipt
                .as_ref()
                .ok_or("XMAP10 screen shadow has not executed")?;
            receipt.validate()?;
            Ok(NativeXmap10ShaderResourceView {
                binding: receipt.binding.clone(),
                raw_srv: self.targets.final_target.srv.as_raw(),
                device_identity: self.device.as_raw() as usize,
                shader_register: None,
                _owner: std::marker::PhantomData,
            })
        }
    }

    fn validate_screen_shadow_plan(plan: &ShadowScreenChainPlan) -> Result<()> {
        if plan.producer_and_final_target.width == 0
            || plan.producer_and_final_target.height == 0
            || plan.producer_and_final_target.width != plan.blur_intermediate_target.width
            || plan.producer_and_final_target.height != plan.blur_intermediate_target.height
            || plan.producer_and_final_target.dxgi_format != 87
            || plan.blur_intermediate_target.dxgi_format != 87
            || plan.producer_and_final_target.mip_levels != 1
            || plan.blur_intermediate_target.mip_levels != 1
            || plan.producer_and_final_target.bind_flags
                != (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE)
            || plan.blur_intermediate_target.bind_flags
                != (D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE)
            || plan.producer_and_final_target.stable_resource_id
                == plan.blur_intermediate_target.stable_resource_id
            || plan.passes.len() != 3
            || plan.passes[0].kind != ShadowScreenPassKind::TerrainUnlitProducer
            || plan.passes[1].kind != ShadowScreenPassKind::HorizontalBlur
            || plan.passes[2].kind != ShadowScreenPassKind::VerticalBlur
            || plan.passes[0].source_resource_id != plan.light_depth_target.stable_resource_id
            || plan.passes[0].target_resource_id
                != plan.producer_and_final_target.stable_resource_id
            || plan.passes[1].source_resource_id
                != plan.producer_and_final_target.stable_resource_id
            || plan.passes[1].target_resource_id != plan.blur_intermediate_target.stable_resource_id
            || plan.passes[2].source_resource_id != plan.blur_intermediate_target.stable_resource_id
            || plan.passes[2].target_resource_id
                != plan.producer_and_final_target.stable_resource_id
            || plan.passes[0].cb0_f32_bits.is_some()
            || plan.passes[1].cb0_f32_bits.is_none()
            || plan.passes[2].cb0_f32_bits.is_none()
        {
            return Err("XMAP10 screen-shadow plan violates the exact S1 ping-pong law".into());
        }
        Ok(())
    }

    unsafe fn validate_context_for_device(
        device: &D3dPtr<ID3D11Device>,
        context: *mut ID3D11DeviceContext,
    ) -> Result<()> {
        if context.is_null() {
            return Err("XMAP10 screen-shadow context is null".into());
        }
        let mut context_device = ptr::null_mut();
        (*context).GetDevice(&mut context_device);
        let context_device =
            D3dPtr::from_raw(context_device, "XMAP10 screen-shadow context device")?;
        if context_device.as_raw() != device.as_raw() {
            return Err("XMAP10 screen-shadow context belongs to another device".into());
        }
        Ok(())
    }

    unsafe fn validate_texture_for_device(
        device: &D3dPtr<ID3D11Device>,
        texture: *mut ID3D11Texture2D,
    ) -> Result<()> {
        if texture.is_null() {
            return Err("XMAP10 post-FX MainScene texture is null".into());
        }
        let mut texture_device = ptr::null_mut();
        (*texture).GetDevice(&mut texture_device);
        let texture_device = D3dPtr::from_raw(texture_device, "XMAP10 post-FX MainScene device")?;
        if texture_device.as_raw() != device.as_raw() {
            return Err("XMAP10 post-FX MainScene texture belongs to another device".into());
        }
        Ok(())
    }

    unsafe fn create_terrain_unlit_depth(device: &D3dPtr<ID3D11Device>) -> Result<D3dPtr<ID3D11DepthStencilState>> {
        let mut desc: D3D11_DEPTH_STENCIL_DESC = mem::zeroed();
        desc.DepthEnable = 1;
        desc.DepthWriteMask = D3D11_DEPTH_WRITE_MASK_ALL;
        desc.DepthFunc = D3D11_COMPARISON_LESS_EQUAL;
        desc.FrontFace.StencilFailOp = D3D11_STENCIL_OP_KEEP;
        desc.FrontFace.StencilDepthFailOp = D3D11_STENCIL_OP_KEEP;
        desc.FrontFace.StencilPassOp = D3D11_STENCIL_OP_KEEP;
        desc.FrontFace.StencilFunc = D3D11_COMPARISON_ALWAYS;
        desc.BackFace = desc.FrontFace;
        let mut raw = ptr::null_mut();
        hr((*device.as_raw()).CreateDepthStencilState(&desc, &mut raw), "CreateDepthStencilState(terrainunlit)")?;
        D3dPtr::from_raw(raw, "terrainunlit depth")
    }

    unsafe fn compile_terrain_unlit(
        device: &D3dPtr<ID3D11Device>,
        resolver: &GameFileResolver,
        terrain: &PreparedTerrainFrame,
    ) -> Result<(NativeProgram, D3dPtr<ID3D11InputLayout>)> {
        let defines =
            crate::engine_map_shader_defines(crate::derive_a729_engine_map_shader_constants(
                terrain.map_width,
                terrain.map_height,
                false,
                0,
            )?);
        let vertex_hlsl = build_stage_hlsl_with_defines(
            resolver,
            "gfx/FX/pdxmap.shader",
            "terrainunlit",
            ShaderStage::Vertex,
            &defines,
        )?;
        let pixel_hlsl = build_stage_hlsl_with_defines(
            resolver,
            "gfx/FX/pdxmap.shader",
            "terrainunlit",
            ShaderStage::Pixel,
            &defines,
        )?;
        let vertex_bytecode =
            d3d_compile_stage(&vertex_hlsl, ShaderStage::Vertex, ENGINE_COMPILE_FLAGS)?;
        let pixel_bytecode =
            d3d_compile_stage(&pixel_hlsl, ShaderStage::Pixel, ENGINE_COMPILE_FLAGS)?;
        let vertex_reflection = parse_dxbc_reflection(&vertex_bytecode)?;
        let pixel_reflection = parse_dxbc_reflection(&pixel_bytecode)?;
        require_cbuffer(&vertex_reflection, 0, 416)?;
        require_cbuffer(&vertex_reflection, 1, crate::TERRAIN_CHUNK_CB1_BYTES as u32)?;
        require_cbuffer(&pixel_reflection, 0, 416)?;
        for slot in [0u32, 1, 2, 12] {
            require_texture(&pixel_reflection, slot)?;
            require_sampler(&pixel_reflection, slot)?;
        }
        require_unlit_texture_providers("terrainunlit", &pixel_reflection.resources)?;
        for (reflection, vertex) in [(&vertex_reflection, true), (&pixel_reflection, false)] {
            for resource in &reflection.resources {
                if resource.bind_count != 1 || match resource.resource_type {
                    0 => resource.bind_point > 1,
                    2 => vertex || resource.bind_point > 15 || (matches!(resource.bind_point, 0 | 1 | 2 | 7 | 12) && resource.dimension != 4),
                    3 => vertex || resource.bind_point > 15,
                    _ => true,
                } { return Err("XMAP10 terrainunlit reflection requires an unproven producer input".into()); }
                if resource.resource_type == 0 {
                    let size = if resource.bind_point == 0 { 416 } else { crate::TERRAIN_CHUNK_CB1_BYTES as u32 };
                    require_cbuffer(reflection, resource.bind_point, size)?;
                }
            }
        }
        let input_layout = create_terrain_unlit_input_layout(device, &vertex_bytecode)?;
        let program = create_program(
            device,
            vertex_bytecode,
            pixel_bytecode,
            vertex_reflection,
            pixel_reflection,
            "terrainunlit",
        )?;
        Ok((program, input_layout))
    }

    #[cfg(test)]
    mod shadow_blur_binding_tests {
        use super::*;

        #[test]
        fn authored_blur_binds_both_constant_stages_selected_blend_and_sampler() {
            use std::fs;
            use winapi::um::d3dcommon::{D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0};
            let root = crate::test_support::temp_test_dir("shadow_blur_bindings");
            let fx = root.join("gfx/FX");
            fs::create_dir_all(&fx).unwrap();
            fs::write(fx.join("shadowblur.shader"), r#"
VertexStruct Input { float4 p : POSITION; float2 uv : TEXCOORD; }
VertexStruct Output { float4 p : SV_POSITION; float2 uv : TEXCOORD; }
ConstantBuffer(0, 0) { float4 color; float4 scale; };
VertexShader = { MainCode VS [[ Output main(Input i) { Output o; o.p = float4(i.p.xy * scale.x, i.p.zw); o.uv = i.uv; return o; } ]] }
PixelShader = {
    Samplers = { Image = { Index = 0 MagFilter = "Point" MinFilter = "Point" MipFilter = "Point" AddressU = "Clamp" AddressV = "Clamp" } }
    MainCode PS [[ float4 main(Output i) : SV_Target { return Image_Texture.Sample(Image_Sampler, i.uv) * color; } ]]
}
BlendState BlendState { BlendEnable = no WriteMask = "ALL" }
BlendState Red { BlendEnable = no WriteMask = "RED" }
BlendState Green { BlendEnable = no WriteMask = "GREEN" }
Effect ShadowBlurHorizontal { VertexShader = "VS" PixelShader = "PS" BlendState = "Red" }
Effect ShadowBlurVertical { VertexShader = "VS" PixelShader = "PS" BlendState = "Green" }
"#).unwrap();
            fs::write(fx.join("defines_hlsl_dx11.fxh"), r#"
struct STextureSampler2D { Texture2D tex; SamplerState state; };
STextureSampler2D CreateSampler2D(Texture2D t, SamplerState s) { STextureSampler2D result = {t, s}; return result; }
"#).unwrap();
            let source = GameFileResolver::vanilla(&root).unwrap();
            let paths = ["gfx/fx/shadowblur.shader".into(), crate::shader_compile::DX11_PLATFORM_HEADER.to_ascii_lowercase()].into_iter().collect();
            let (resolver, _) = crate::test_support::admitted_vanilla_resolver(&source, &paths, "shadow-blur-binding");
            unsafe {
                let mut device = ptr::null_mut(); let mut context = ptr::null_mut();
                hr(D3D11CreateDevice(ptr::null_mut(), D3D_DRIVER_TYPE_WARP, ptr::null_mut(), 0,
                    &D3D_FEATURE_LEVEL_11_0, 1, D3D11_SDK_VERSION, &mut device, ptr::null_mut(), &mut context), "blur WARP").unwrap();
                let device = D3dPtr::from_raw(device, "blur device").unwrap();
                let context_owner = D3dPtr::from_raw(context, "blur context").unwrap();
                let input = create_target(&device, &"a".repeat(64), 8, 8, 28, "blur input").unwrap();
                let target = create_target(&device, &"b".repeat(64), 8, 8, 28, "blur target").unwrap();
                (*context).ClearRenderTargetView(input.rtv.as_raw(), &[0.25, 0.5, 0.75, 1.0]);
                let cb = create_constant_buffer(&device, 32, "blur test constants").unwrap();
                let values = [1.0f32; 8];
                (*context).UpdateSubresource(cb.as_raw().cast(), 0, ptr::null(), values.as_ptr().cast(), 0, 0);
                let vertices = create_shadow_blur_vertices(&device).unwrap();
                let raster = create_rasterizer(&device).unwrap();
                let depth = create_depth_disabled(&device).unwrap();
                let mut desc: D3D11_TEXTURE2D_DESC = mem::zeroed();
                (*target._texture.as_raw()).GetDesc(&mut desc);
                desc.Usage = D3D11_USAGE_STAGING; desc.BindFlags = 0; desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
                let mut staging = ptr::null_mut();
                hr((*device.as_raw()).CreateTexture2D(&desc, ptr::null(), &mut staging), "blur staging").unwrap();
                let staging = D3dPtr::from_raw(staging, "blur staging").unwrap();
                for (effect, expected) in [("ShadowBlurHorizontal", [64u8, 0, 255, 64]), ("ShadowBlurVertical", [255u8, 128, 255, 64])] {
                    let program = compile_shadow_blur(&device, &resolver, effect).unwrap();
                    let mut sampler: D3D11_SAMPLER_DESC = mem::zeroed();
                    (*program.sampler.as_ref().unwrap().as_raw()).GetDesc(&mut sampler);
                    assert_eq!(sampler.Filter, D3D11_FILTER_MIN_MAG_MIP_POINT);
                    assert_eq!(sampler.AddressU, D3D11_TEXTURE_ADDRESS_CLAMP);
                    let layout = create_shadow_blur_input_layout(&device, &program.program.vertex_bytecode).unwrap();
                    (*context).ClearRenderTargetView(target.rtv.as_raw(), &[1.0, 0.0, 1.0, 0.25]);
                    (*context).OMSetRenderTargets(1, &target.rtv.as_raw(), ptr::null_mut());
                    (*context).RSSetViewports(1, &D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: 8.0, Height: 8.0, MinDepth: 0.0, MaxDepth: 1.0 });
                    (*context).RSSetState(raster.as_raw());
                    (*context).OMSetDepthStencilState(depth.as_raw(), 0);
                    (*context).IASetInputLayout(layout.as_raw());
                    (*context).IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
                    (*context).IASetVertexBuffers(0, 1, &vertices.as_raw(), &24, &0);
                    (*context).VSSetConstantBuffers(0, 1, &ptr::null_mut());
                    (*context).PSSetConstantBuffers(0, 1, &ptr::null_mut());
                    program.bind(context, cb.as_raw());
                    (*context).PSSetShaderResources(0, 1, &input.srv.as_raw());
                    (*context).Draw(4, 0);
                    (*context).PSSetShaderResources(0, 1, &ptr::null_mut());
                    (*context).OMSetRenderTargets(0, ptr::null(), ptr::null_mut());
                    (*context).CopyResource(staging.as_raw().cast(), target._texture.as_raw().cast());
                    let mut mapped: D3D11_MAPPED_SUBRESOURCE = mem::zeroed();
                    hr((*context).Map(staging.as_raw().cast(), 0, D3D11_MAP_READ, 0, &mut mapped), "blur readback").unwrap();
                    for row in 0..8 {
                        let bytes = std::slice::from_raw_parts((mapped.pData as *const u8).add(row * mapped.RowPitch as usize), 32);
                        assert!(bytes.chunks_exact(4).all(|pixel| pixel == expected), "{effect} binding mismatch");
                    }
                    (*context).Unmap(staging.as_raw().cast(), 0);
                }
                drop(context_owner);
            }
            drop(resolver); drop(source); fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn authored_constant_blur_creates_without_sampler_declaration() {
            use std::fs;
            use winapi::um::d3dcommon::{D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_11_0};
            let root = crate::test_support::temp_test_dir("shadow_blur_without_sampler");
            let fx = root.join("gfx/FX");
            fs::create_dir_all(&fx).unwrap();
            fs::write(fx.join("shadowblur.shader"), r#"
VertexStruct Input { float4 p : POSITION; }
VertexShader = { MainCode VS [[ float4 main(Input i) : SV_POSITION { return i.p; } ]] }
PixelShader = { MainCode PS [[ float4 main() : SV_Target { return float4(0.25, 0.5, 0.75, 1); } ]] }
Effect ShadowBlurHorizontal { VertexShader = "VS" PixelShader = "PS" }
"#).unwrap();
            fs::write(fx.join("defines_hlsl_dx11.fxh"), "").unwrap();
            let source = GameFileResolver::vanilla(&root).unwrap();
            let paths = ["gfx/fx/shadowblur.shader".into(), crate::shader_compile::DX11_PLATFORM_HEADER.to_ascii_lowercase()].into_iter().collect();
            let (resolver, _) = crate::test_support::admitted_vanilla_resolver(&source, &paths, "constant-blur-binding");
            unsafe {
                let mut device = ptr::null_mut(); let mut context = ptr::null_mut();
                hr(D3D11CreateDevice(ptr::null_mut(), D3D_DRIVER_TYPE_WARP, ptr::null_mut(), 0,
                    &D3D_FEATURE_LEVEL_11_0, 1, D3D11_SDK_VERSION, &mut device, ptr::null_mut(), &mut context), "constant blur WARP").unwrap();
                let device = D3dPtr::from_raw(device, "constant blur device").unwrap();
                let context_owner = D3dPtr::from_raw(context, "constant blur context").unwrap();
                let program = compile_shadow_blur(&device, &resolver, "ShadowBlurHorizontal").unwrap();
                assert!(program.sampler.is_none());
                program.bind(context, ptr::null_mut());
                let mut sampler = ptr::null_mut();
                (*context).PSGetSamplers(0, 1, &mut sampler);
                assert!(sampler.is_null());
                drop(context_owner);
            }
            drop(resolver); drop(source); fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn shadow_blur_reflection_refuses_unbound_inputs_and_accepts_eliminated_inputs() {
            let vertex = "float4 main(float4 p : POSITION) : SV_POSITION { return p; }";
            let pixel = "cbuffer C : register(b0) { float4 color; }; float4 main() : SV_Target { return color; }";
            let reflect = |source: &str, stage| parse_dxbc_reflection(&d3d_compile_stage(source, stage, ENGINE_COMPILE_FLAGS).unwrap()).unwrap();
            let vs = reflect(vertex, ShaderStage::Vertex);
            let ps = reflect(pixel, ShaderStage::Pixel);
            require_shadow_blur_providers(&vs, &ps).unwrap();
            for bad in [pixel.replace("b0", "b1"), pixel.replace("float4 color;", "float4 padding[2]; float4 color;"),
                "Texture2D tex : register(t1); float4 main() : SV_Target { return tex.Load(int3(0, 0, 0)); }".into()] {
                assert!(require_shadow_blur_providers(&vs, &reflect(&bad, ShaderStage::Pixel)).unwrap_err().to_string().contains("shadow-blur-reflected-provider-unavailable"));
            }
        }
    }

    unsafe fn compile_shadow_blur(
        device: &D3dPtr<ID3D11Device>,
        resolver: &GameFileResolver,
        effect_name: &str,
    ) -> Result<NativeShadowBlurProgram> {
        let vertex_hlsl = build_stage_hlsl_with_defines(
            resolver,
            "gfx/FX/shadowblur.shader",
            effect_name,
            ShaderStage::Vertex,
            &[],
        )?;
        let pixel_hlsl = build_stage_hlsl_with_defines(
            resolver,
            "gfx/FX/shadowblur.shader",
            effect_name,
            ShaderStage::Pixel,
            &[],
        )?;
        let vertex_bytecode =
            d3d_compile_stage(&vertex_hlsl, ShaderStage::Vertex, ENGINE_COMPILE_FLAGS)?;
        let pixel_bytecode =
            d3d_compile_stage(&pixel_hlsl, ShaderStage::Pixel, ENGINE_COMPILE_FLAGS)?;
        let vertex_reflection = parse_dxbc_reflection(&vertex_bytecode)?;
        let pixel_reflection = parse_dxbc_reflection(&pixel_bytecode)?;
        require_shadow_blur_providers(&vertex_reflection, &pixel_reflection)?;
        let files = crate::shader_compile::effect_translation_unit_files(resolver, "gfx/FX/shadowblur.shader")?;
        let root = &files.last().ok_or("shadow blur translation unit is empty")?.1;
        if root.effect_count(effect_name) != 1 { return Err("shadow blur effect selection is ambiguous".into()); }
        let effect = root.effect(effect_name).ok_or("shadow blur effect is absent")?;
        let blend_name = effect.blend_state.as_deref().unwrap_or("BlendState");
        let blend_rows = files.iter().rev().find_map(|(_, file)| file.blend_states.get(blend_name))
            .map(Vec::as_slice).unwrap_or(&[]);
        let blend = crate::xmap12_d3d_handoff::create_blend_state(device.as_raw(), blend_rows)?;
        let sampler = if pixel_reflection.resources.iter().any(|resource| resource.resource_type == 3) {
            Some(crate::xmap12_d3d_handoff::create_effect_sampler_on_device(
                device.as_raw(), resolver, "gfx/FX/shadowblur.shader", 0,
            )?.0)
        } else { None };
        let program = create_program(
            device,
            vertex_bytecode,
            pixel_bytecode,
            vertex_reflection,
            pixel_reflection,
            effect_name,
        )?;
        Ok(NativeShadowBlurProgram { program, blend, sampler })
    }

    fn require_shadow_blur_providers(vertex: &DxbcReflection, pixel: &DxbcReflection) -> Result<()> {
        for (stage, reflection) in [(ShaderStage::Vertex, vertex), (ShaderStage::Pixel, pixel)] {
            for resource in &reflection.resources {
                let supplied = resource.bind_count == 1 && resource.bind_point == 0
                    && match resource.resource_type {
                        0 => reflection.cbuffers.iter().any(|cb| cb.name == resource.name && cb.size <= 32),
                        2 => stage == ShaderStage::Pixel && resource.dimension == 4,
                        3 => stage == ShaderStage::Pixel,
                        _ => false,
                    };
                if !supplied {
                    return Err(format!("shadow-blur-reflected-provider-unavailable: {stage:?} {} type {} register {} count {} dimension {}",
                        resource.name, resource.resource_type, resource.bind_point, resource.bind_count, resource.dimension).into());
                }
            }
        }
        Ok(())
    }

    unsafe fn create_terrain_unlit_input_layout(
        device: &D3dPtr<ID3D11Device>,
        vertex_bytecode: &[u8],
    ) -> Result<D3dPtr<ID3D11InputLayout>> {
        let position = b"POSITION\0";
        let texcoord = b"TEXCOORD\0";
        let elements = [
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: position.as_ptr() as *const i8,
                SemanticIndex: 0,
                Format: crate::TERRAIN_CHUNK_SLOT0_POSITION_FORMAT as _,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: texcoord.as_ptr() as *const i8,
                SemanticIndex: 0,
                Format: crate::TERRAIN_CHUNK_SLOT1_TEXCOORD_FORMAT as _,
                InputSlot: 1,
                AlignedByteOffset: 0,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
        ];
        let mut layout = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateInputLayout(
                elements.as_ptr(),
                elements.len() as u32,
                vertex_bytecode.as_ptr() as *const _,
                vertex_bytecode.len(),
                &mut layout,
            ),
            "CreateInputLayout(XMAP10 terrainunlit)",
        )?;
        D3dPtr::from_raw(layout, "XMAP10 terrainunlit input layout")
    }

    unsafe fn create_shadow_blur_input_layout(
        device: &D3dPtr<ID3D11Device>,
        vertex_bytecode: &[u8],
    ) -> Result<D3dPtr<ID3D11InputLayout>> {
        let position = b"POSITION\0";
        let texcoord = b"TEXCOORD\0";
        let elements = [
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: position.as_ptr() as *const i8,
                SemanticIndex: 0,
                Format: winapi::shared::dxgiformat::DXGI_FORMAT_R32G32B32A32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 0,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D11_INPUT_ELEMENT_DESC {
                SemanticName: texcoord.as_ptr() as *const i8,
                SemanticIndex: 0,
                Format: winapi::shared::dxgiformat::DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: 16,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
        ];
        let mut layout = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateInputLayout(
                elements.as_ptr(),
                elements.len() as u32,
                vertex_bytecode.as_ptr() as *const _,
                vertex_bytecode.len(),
                &mut layout,
            ),
            "CreateInputLayout(XMAP10 shadow blur)",
        )?;
        D3dPtr::from_raw(layout, "XMAP10 shadow blur input layout")
    }

    unsafe fn create_shadow_blur_vertices(
        device: &D3dPtr<ID3D11Device>,
    ) -> Result<D3dPtr<ID3D11Buffer>> {
        let vertices = [
            [-1.0f32, 1.0, 0.0, 1.0, 0.0, 0.0],
            [-1.0, -1.0, 0.0, 1.0, 0.0, 1.0],
            [1.0, 1.0, 0.0, 1.0, 1.0, 0.0],
            [1.0, -1.0, 0.0, 1.0, 1.0, 1.0],
        ];
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: mem::size_of_val(&vertices) as u32,
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_VERTEX_BUFFER,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let initial = D3D11_SUBRESOURCE_DATA {
            pSysMem: vertices.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut buffer = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateBuffer(&desc, &initial, &mut buffer),
            "CreateBuffer(XMAP10 shadow blur vertices)",
        )?;
        D3dPtr::from_raw(buffer, "XMAP10 shadow blur vertices")
    }

    unsafe fn create_terrain_rasterizer(
        device: *mut ID3D11Device,
    ) -> Result<D3dPtr<ID3D11RasterizerState>> {
        let desc = D3D11_RASTERIZER_DESC {
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
        let mut state = ptr::null_mut();
        hr(
            (*device).CreateRasterizerState(&desc, &mut state),
            "CreateRasterizerState(XMAP10 terrainunlit)",
        )?;
        D3dPtr::from_raw(state, "XMAP10 terrainunlit rasterizer")
    }

    fn validate_target_plan_for_desc(
        frame: &PreparedXmap10Frame,
        desc: &ProductionMainSceneTargetDesc,
    ) -> Result<()> {
        if desc.color_format != DXGI_FORMAT_R16G16B16A16_FLOAT_U32 || desc.color_samples != 1 {
            return Err("XMAP10 requires a single-sample R16G16B16A16 MainScene".into());
        }
        let main = frame
            .targets
            .ordered_targets
            .iter()
            .find(|target| target.role == Xmap10TargetRole::MainScene)
            .ok_or("XMAP10 target plan omitted MainScene")?;
        if (main.width, main.height, main.dxgi_format, main.sample_count)
            != (
                desc.color_width,
                desc.color_height,
                desc.color_format,
                desc.color_samples,
            )
        {
            return Err("XMAP10 prepared MainScene differs from production backend".into());
        }
        let passes = &frame.targets.ordered_passes;
        if !matches!(
            passes.first().map(|pass| pass.pass),
            Some(Xmap10PassKind::BrightPass)
        ) || !matches!(
            passes.last().map(|pass| pass.pass),
            Some(Xmap10PassKind::SwapLuminanceHistory)
        ) {
            return Err(
                "XMAP10 target plan does not bracket MainScene through presentation".into(),
            );
        }
        Ok(())
    }

    unsafe fn compile_program(
        device: &D3dPtr<ID3D11Device>,
        resolver: &GameFileResolver,
        shader_root: &str,
        effect_name: &str,
        role: Xmap10ProgramRole,
        ordered_defines: &[String],
    ) -> Result<NativeProgram> {
        let effect_file = effect_file_from_resolver(resolver, shader_root)?;
        let effect = effect_file
            .effect(effect_name)
            .ok_or_else(|| format!("XMAP10 effect {shader_root}:{effect_name} is absent"))?;
        if role == Xmap10ProgramRole::RestoreScene {
            let state_name = effect
                .blend_state
                .as_deref()
                .ok_or("XMAP10 RestoreScene effective effect omits its blend state")?;
            let state = effect_file
                .blend_states
                .get(state_name)
                .ok_or("XMAP10 RestoreScene effective blend state is unresolved")?;
            let value = |key: &str| {
                state
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(key))
                    .map(|(_, value)| value.trim_matches('"'))
            };
            if value("BlendEnable") != Some("no") || value("WriteMask") != Some("RED|GREEN|BLUE") {
                return Err(
                    "XMAP10 RestoreScene effective blend state is not RGB-only overwrite".into(),
                );
            }
        }
        let effect_defines = effect
            .defines
            .iter()
            .map(|define| define.to_ascii_uppercase())
            .collect::<Vec<_>>();
        let engine_defines = ordered_defines
            .iter()
            .filter(|define| !effect_defines.contains(&define.to_ascii_uppercase()))
            .map(|define| define_pair(define))
            .collect::<Result<Vec<_>>>()?;
        let vertex_hlsl = build_stage_hlsl_with_defines(
            resolver,
            shader_root,
            effect_name,
            ShaderStage::Vertex,
            &engine_defines,
        )?;
        let mut pixel_hlsl = build_stage_hlsl_with_defines(
            resolver,
            shader_root,
            effect_name,
            ShaderStage::Pixel,
            &engine_defines,
        )?;
        if role == Xmap10ProgramRole::LuminanceGather {
            pixel_hlsl = select_luminance_gather_cbuffer(pixel_hlsl, &effect_file)?;
        }
        let vertex_bytecode =
            d3d_compile_stage(&vertex_hlsl, ShaderStage::Vertex, ENGINE_COMPILE_FLAGS)?;
        let pixel_bytecode =
            d3d_compile_stage(&pixel_hlsl, ShaderStage::Pixel, ENGINE_COMPILE_FLAGS)?;
        let vertex_reflection = parse_dxbc_reflection(&vertex_bytecode)?;
        let pixel_reflection = parse_dxbc_reflection(&pixel_bytecode)?;
        validate_program_reflection(role, &vertex_reflection, &pixel_reflection)?;
        create_program(
            device,
            vertex_bytecode,
            pixel_bytecode,
            vertex_reflection,
            pixel_reflection,
            effect_name,
        )
    }

    unsafe fn compile_lut_program(
        device: &D3dPtr<ID3D11Device>,
        resolver: &GameFileResolver,
        effect_name: &str,
        resource_count: u32,
    ) -> Result<NativeProgram> {
        let vertex_hlsl = build_stage_hlsl_with_defines(
            resolver,
            LUT_SHADER,
            effect_name,
            ShaderStage::Vertex,
            &[],
        )?;
        let pixel_hlsl = build_stage_hlsl_with_defines(
            resolver,
            LUT_SHADER,
            effect_name,
            ShaderStage::Pixel,
            &[],
        )?;
        let vertex_bytecode =
            d3d_compile_stage(&vertex_hlsl, ShaderStage::Vertex, ENGINE_COMPILE_FLAGS)?;
        let pixel_bytecode =
            d3d_compile_stage(&pixel_hlsl, ShaderStage::Pixel, ENGINE_COMPILE_FLAGS)?;
        let vertex_reflection = parse_dxbc_reflection(&vertex_bytecode)?;
        let pixel_reflection = parse_dxbc_reflection(&pixel_bytecode)?;
        require_cbuffer(&vertex_reflection, 0, 16)?;
        for slot in 0..resource_count {
            require_texture(&pixel_reflection, slot)?;
            require_sampler(&pixel_reflection, slot)?;
        }
        create_program(
            device,
            vertex_bytecode,
            pixel_bytecode,
            vertex_reflection,
            pixel_reflection,
            effect_name,
        )
    }

    fn select_luminance_gather_cbuffer(
        mut hlsl: String,
        file: &crate::shader_compile::EffectFile,
    ) -> Result<String> {
        let body = file
            .elements
            .iter()
            .filter_map(|(_, element)| match element {
                crate::shader_compile::EffectElement::ConstantBuffer { slot: 2, body, .. }
                    if body.contains("InvSize_TauDeltaTime")
                        && body.contains("GatherSize_PixelSize") =>
                {
                    Some(body)
                }
                _ => None,
            })
            .next()
            .ok_or("XMAP10 effective luminance shader omitted the gather cb2 declaration")?;
        let marker = "cbuffer dx11_cb2 : register(b2)";
        let start = hlsl
            .find(marker)
            .ok_or("XMAP10 reconstructed luminance HLSL omitted cb2")?;
        let brace = hlsl[start..]
            .find('{')
            .map(|offset| start + offset)
            .ok_or("XMAP10 reconstructed luminance cb2 has no body")?;
        let close = matching_brace(&hlsl, brace)?;
        hlsl.replace_range(
            brace..=close,
            &format!("{{\n{}\n}}", normalize_effect_body(body)),
        );
        Ok(hlsl)
    }

    fn matching_brace(text: &str, open: usize) -> Result<usize> {
        let mut depth = 0u32;
        for (offset, byte) in text.as_bytes()[open..].iter().enumerate() {
            match *byte {
                b'{' => depth += 1,
                b'}' => {
                    depth = depth.checked_sub(1).ok_or("XMAP10 HLSL brace underflow")?;
                    if depth == 0 {
                        return Ok(open + offset);
                    }
                }
                _ => {}
            }
        }
        Err("XMAP10 HLSL cbuffer body is unterminated".into())
    }

    fn normalize_effect_body(body: &str) -> String {
        body.lines()
            .map(|line| {
                let trimmed = line.trim_start();
                if let Some(rest) = trimmed.strip_prefix('@') {
                    format!("{}#{}", &line[..line.len() - trimmed.len()], rest)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn define_pair(define: &str) -> Result<(String, String)> {
        let trimmed = define.trim();
        if trimmed.is_empty() {
            return Err("XMAP10 shader define is empty".into());
        }
        let (name, value) = trimmed
            .split_once('=')
            .map(|(name, value)| (name.trim(), value.trim()))
            .unwrap_or((trimmed, ""));
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(format!("XMAP10 shader define name is invalid: {trimmed:?}").into());
        }
        Ok((name.to_string(), value.to_string()))
    }

    fn validate_program_reflection(
        role: Xmap10ProgramRole,
        vertex: &DxbcReflection,
        pixel: &DxbcReflection,
    ) -> Result<()> {
        if vertex.target >> 16 != 0xfffe
            || pixel.target >> 16 != 0xffff
            || !vertex
                .inputs
                .iter()
                .any(|input| input.semantic_name.eq_ignore_ascii_case("POSITION"))
        {
            return Err(format!("XMAP10 {role:?} shader stage/signature mismatch").into());
        }
        match role {
            Xmap10ProgramRole::BrightPass => {
                require_texture(pixel, 0)?;
                require_sampler(pixel, 0)?;
                require_cbuffer(pixel, 1, 80)?;
            }
            Xmap10ProgramRole::LuminanceDownsample => {
                require_texture(pixel, 0)?;
                require_sampler(pixel, 0)?;
                require_cbuffer(vertex, 2, 48)?;
            }
            Xmap10ProgramRole::LuminanceGather => {
                for slot in 0..2 {
                    require_texture(pixel, slot)?;
                    require_sampler(pixel, slot)?;
                }
                require_cbuffer(vertex, 2, 48)?;
                require_cbuffer(pixel, 2, 48)?;
            }
            Xmap10ProgramRole::BloomHorizontal | Xmap10ProgramRole::BloomVertical => {
                require_texture(pixel, 0)?;
                require_sampler(pixel, 0)?;
                require_cbuffer(vertex, 2, 48)?;
            }
            Xmap10ProgramRole::RestoreScene => {
                for slot in 0..4 {
                    require_texture(pixel, slot)?;
                    require_sampler(pixel, slot)?;
                }
                require_cbuffer(pixel, 1, 80)?;
            }
        }
        Ok(())
    }

    fn require_texture(reflection: &DxbcReflection, slot: u32) -> Result<()> {
        if reflection
            .resources
            .iter()
            .any(|resource| resource.resource_type == 2 && resource.bind_point == slot)
        {
            Ok(())
        } else {
            Err(format!("XMAP10 reflection omitted texture t{slot}").into())
        }
    }

    fn require_sampler(reflection: &DxbcReflection, slot: u32) -> Result<()> {
        if reflection
            .resources
            .iter()
            .any(|resource| resource.resource_type == 3 && resource.bind_point == slot)
        {
            Ok(())
        } else {
            Err(format!("XMAP10 reflection omitted sampler s{slot}").into())
        }
    }

    fn require_cbuffer(reflection: &DxbcReflection, slot: u32, size: u32) -> Result<()> {
        let name = format!("dx11_cb{slot}");
        if reflection
            .cbuffers
            .iter()
            .any(|cbuffer| cbuffer.name == name && cbuffer.size == size)
        {
            Ok(())
        } else {
            Err(format!("XMAP10 reflection omitted {name} at {size} bytes").into())
        }
    }

    unsafe fn create_program(
        device: &D3dPtr<ID3D11Device>,
        vertex_bytecode: Vec<u8>,
        pixel_bytecode: Vec<u8>,
        vertex_reflection: DxbcReflection,
        pixel_reflection: DxbcReflection,
        label: &str,
    ) -> Result<NativeProgram> {
        let mut vertex = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateVertexShader(
                vertex_bytecode.as_ptr() as *const _,
                vertex_bytecode.len(),
                ptr::null_mut(),
                &mut vertex,
            ),
            &format!("CreateVertexShader(XMAP10 {label})"),
        )?;
        let mut pixel = ptr::null_mut();
        hr(
            (*device.as_raw()).CreatePixelShader(
                pixel_bytecode.as_ptr() as *const _,
                pixel_bytecode.len(),
                ptr::null_mut(),
                &mut pixel,
            ),
            &format!("CreatePixelShader(XMAP10 {label})"),
        )?;
        Ok(NativeProgram {
            vertex: D3dPtr::from_raw(vertex, "XMAP10 vertex shader")?,
            pixel: D3dPtr::from_raw(pixel, "XMAP10 pixel shader")?,
            reflection_sha256: reflection_digest(&vertex_reflection, &pixel_reflection),
            vertex_bytecode,
            pixel_bytecode,
        })
    }

    fn reflection_digest(vertex: &DxbcReflection, pixel: &DxbcReflection) -> String {
        let mut bytes = Vec::new();
        for reflection in [vertex, pixel] {
            bytes.extend_from_slice(&reflection.target.to_le_bytes());
            push_string(&mut bytes, &reflection.creator);
            for resource in &reflection.resources {
                push_string(&mut bytes, &resource.name);
                bytes.extend_from_slice(&resource.resource_type.to_le_bytes());
                bytes.extend_from_slice(&resource.bind_point.to_le_bytes());
                bytes.extend_from_slice(&resource.bind_count.to_le_bytes());
            }
            for cbuffer in &reflection.cbuffers {
                push_string(&mut bytes, &cbuffer.name);
                bytes.extend_from_slice(&cbuffer.size.to_le_bytes());
                for var in &cbuffer.vars {
                    push_string(&mut bytes, &var.name);
                    bytes.extend_from_slice(&var.offset.to_le_bytes());
                    bytes.extend_from_slice(&var.size.to_le_bytes());
                    bytes.push(u8::from(var.used));
                }
            }
            for parameter in reflection.inputs.iter().chain(&reflection.outputs) {
                push_string(&mut bytes, &parameter.semantic_name);
                bytes.extend_from_slice(&parameter.semantic_index.to_le_bytes());
                bytes.extend_from_slice(&parameter.register.to_le_bytes());
                bytes.extend_from_slice(&parameter.component_type.to_le_bytes());
                bytes.push(parameter.mask);
                bytes.push(parameter.read_write_mask);
            }
        }
        Sha256::hex(&bytes)
    }

    fn native_program_receipt_digest(
        receipt: &Xmap10NativeProgramReceiptRef,
        scalar: &NativeProgram,
        luts: [&NativeLutProgram; 2],
    ) -> String {
        let mut bytes = Vec::new();
        push_string(&mut bytes, &receipt.schema);
        push_string(&mut bytes, &receipt.prepared_frame_sha256);
        for program in &receipt.ordered_programs {
            bytes.push(program_role_byte(program.role));
            push_string(&mut bytes, &program.active_effect_sha256);
            push_string(&mut bytes, &program.vertex_bytecode_sha256);
            push_string(&mut bytes, &program.pixel_bytecode_sha256);
            push_string(&mut bytes, &program.reflection_sha256);
        }
        push_string(&mut bytes, &Sha256::hex(&scalar.vertex_bytecode));
        push_string(&mut bytes, &Sha256::hex(&scalar.pixel_bytecode));
        push_string(&mut bytes, &scalar.reflection_sha256);
        for lut in luts {
            push_string(&mut bytes, &lut.effect_name);
            push_string(&mut bytes, &Sha256::hex(&lut.program.vertex_bytecode));
            push_string(&mut bytes, &Sha256::hex(&lut.program.pixel_bytecode));
            push_string(&mut bytes, &lut.program.reflection_sha256);
        }
        Sha256::hex(&bytes)
    }

    fn program_role_byte(role: Xmap10ProgramRole) -> u8 {
        match role {
            Xmap10ProgramRole::BrightPass => 0,
            Xmap10ProgramRole::LuminanceDownsample => 1,
            Xmap10ProgramRole::LuminanceGather => 2,
            Xmap10ProgramRole::BloomHorizontal => 3,
            Xmap10ProgramRole::BloomVertical => 4,
            Xmap10ProgramRole::RestoreScene => 5,
        }
    }

    fn native_program_identity(program: &NativeProgram) -> String {
        Sha256::hex(
            &serde_json::to_vec(&(
                "hoi4.xmap10.native-program.v1",
                Sha256::hex(&program.vertex_bytecode),
                Sha256::hex(&program.pixel_bytecode),
                &program.reflection_sha256,
            ))
            .expect("native program identity serialization cannot fail"),
        )
    }

    fn native_identity_set(label: &str, ordered_identities: &[String]) -> String {
        Sha256::hex(
            &serde_json::to_vec(&(
                "hoi4.xmap10.native-identity-set.v1",
                label,
                ordered_identities,
            ))
            .expect("native identity-set serialization cannot fail"),
        )
    }

    fn native_constant_buffer_receipt(label: &str, values: &[f32]) -> String {
        let bits = values
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>();
        Sha256::hex(
            &serde_json::to_vec(&("hoi4.xmap10.native-cbuffer.v1", label, bits))
                .expect("native cbuffer identity serialization cannot fail"),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn native_draw_receipt(
        assigned_stage: &AssignedStage,
        detail: &str,
        program_identity_sha256: &str,
        ordered_input_target_identity_sha256s: &[String],
        output_target_identity_sha256: &str,
        ordered_resource_identity_sha256s: &[String],
        ordered_constant_buffer_receipt_sha256s: &[String],
        width: u32,
        height: u32,
    ) -> String {
        Sha256::hex(
            &serde_json::to_vec(&(
                "hoi4.xmap10.native-draw.v1",
                assigned_stage,
                detail,
                program_identity_sha256,
                ordered_input_target_identity_sha256s,
                output_target_identity_sha256,
                ordered_resource_identity_sha256s,
                ordered_constant_buffer_receipt_sha256s,
                width,
                height,
                4_u32,
                0_u32,
            ))
            .expect("native draw receipt serialization cannot fail"),
        )
    }

    unsafe fn create_target(
        device: &D3dPtr<ID3D11Device>,
        prepared_frame_sha256: &str,
        width: u32,
        height: u32,
        format: u32,
        label: &str,
    ) -> Result<NativeTarget> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format as _,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateTexture2D(&desc, ptr::null(), &mut texture),
            &format!("CreateTexture2D({label})"),
        )?;
        let texture = D3dPtr::from_raw(texture, label)?;
        let mut rtv = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateRenderTargetView(
                texture.as_raw() as *mut _,
                ptr::null(),
                &mut rtv,
            ),
            &format!("CreateRenderTargetView({label})"),
        )?;
        let mut srv = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateShaderResourceView(
                texture.as_raw() as *mut _,
                ptr::null(),
                &mut srv,
            ),
            &format!("CreateShaderResourceView({label})"),
        )?;
        Ok(NativeTarget {
            _texture: texture,
            rtv: D3dPtr::from_raw(rtv, "XMAP10 target RTV")?,
            srv: D3dPtr::from_raw(srv, "XMAP10 target SRV")?,
            width,
            height,
            resource_identity_sha256: Sha256::hex(&serde_json::to_vec(&(
                "hoi4.xmap10.native-target.v1",
                prepared_frame_sha256,
                label,
                width,
                height,
                format,
                1_u32,
                D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE,
            ))?),
        })
    }

    unsafe fn create_srv_for_texture(
        device: &D3dPtr<ID3D11Device>,
        texture: *mut ID3D11Texture2D,
        label: &str,
    ) -> Result<D3dPtr<ID3D11ShaderResourceView>> {
        if texture.is_null() {
            return Err(format!("{label} source is null").into());
        }
        let mut view = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateShaderResourceView(texture as *mut _, ptr::null(), &mut view),
            &format!("CreateShaderResourceView({label})"),
        )?;
        D3dPtr::from_raw(view, label)
    }

    unsafe fn upload_lut(
        device: &D3dPtr<ID3D11Device>,
        lut: &ColorCorrectionLut,
        payload_sha256: &str,
    ) -> Result<NativeSampledTexture> {
        if lut.width != 1024 || lut.height != 32 || lut.slice_size != 32 {
            return Err(
                "XMAP10 native ColorCube endpoint is not the exact 1024x32 cube32 layout".into(),
            );
        }
        let mut rgba = Vec::with_capacity(lut.width * lut.height * 4);
        for texel in &lut.texels {
            for channel in texel {
                rgba.push((channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            rgba.push(255);
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: lut.width as u32,
            Height: lut.height as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let initial = D3D11_SUBRESOURCE_DATA {
            pSysMem: rgba.as_ptr() as *const _,
            SysMemPitch: (lut.width * 4) as u32,
            SysMemSlicePitch: rgba.len() as u32,
        };
        let mut texture = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateTexture2D(&desc, &initial, &mut texture),
            &format!("CreateTexture2D(XMAP10 LUT {payload_sha256})"),
        )?;
        let texture = D3dPtr::from_raw(texture, "XMAP10 LUT texture")?;
        let mut srv = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateShaderResourceView(
                texture.as_raw() as *mut _,
                ptr::null(),
                &mut srv,
            ),
            "CreateShaderResourceView(XMAP10 LUT)",
        )?;
        Ok(NativeSampledTexture {
            _texture: texture,
            srv: D3dPtr::from_raw(srv, "XMAP10 LUT SRV")?,
            resource_identity_sha256: native_identity_set(
                "postfx.lut.source-resource",
                &[payload_sha256.to_owned(), Sha256::hex(&rgba)],
            ),
        })
    }

    unsafe fn create_fullscreen_input_layout(
        device: &D3dPtr<ID3D11Device>,
        vertex_bytecode: &[u8],
    ) -> Result<D3dPtr<ID3D11InputLayout>> {
        let semantic = b"POSITION\0";
        let element = D3D11_INPUT_ELEMENT_DESC {
            SemanticName: semantic.as_ptr() as *const i8,
            SemanticIndex: 0,
            Format: DXGI_FORMAT_R32G32_SINT,
            InputSlot: 0,
            AlignedByteOffset: 0,
            InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        };
        let mut layout = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateInputLayout(
                &element,
                1,
                vertex_bytecode.as_ptr() as *const _,
                vertex_bytecode.len(),
                &mut layout,
            ),
            "CreateInputLayout(XMAP10 fullscreen)",
        )?;
        D3dPtr::from_raw(layout, "XMAP10 fullscreen input layout")
    }

    unsafe fn create_fullscreen_vertices(
        device: &D3dPtr<ID3D11Device>,
    ) -> Result<D3dPtr<ID3D11Buffer>> {
        let vertices = [[-1i32, -1i32], [-1, 1], [1, -1], [1, 1]];
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: mem::size_of_val(&vertices) as u32,
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_VERTEX_BUFFER,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let initial = D3D11_SUBRESOURCE_DATA {
            pSysMem: vertices.as_ptr() as *const _,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut buffer = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateBuffer(&desc, &initial, &mut buffer),
            "CreateBuffer(XMAP10 fullscreen vertices)",
        )?;
        D3dPtr::from_raw(buffer, "XMAP10 fullscreen vertices")
    }

    unsafe fn create_constant_buffer(
        device: &D3dPtr<ID3D11Device>,
        byte_width: u32,
        label: &str,
    ) -> Result<D3dPtr<ID3D11Buffer>> {
        if byte_width == 0 || byte_width % 16 != 0 {
            return Err(format!("{label} byte width is not 16-byte aligned").into());
        }
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: byte_width,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER,
            CPUAccessFlags: 0,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut buffer = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateBuffer(&desc, ptr::null(), &mut buffer),
            &format!("CreateBuffer({label})"),
        )?;
        D3dPtr::from_raw(buffer, label)
    }

    unsafe fn update_constant_buffer<T: Copy>(
        context: &D3dPtr<ID3D11DeviceContext>,
        device: &D3dPtr<ID3D11Device>,
        buffer: &D3dPtr<ID3D11Buffer>,
        value: &T,
    ) -> Result<()> {
        (*context.as_raw()).UpdateSubresource(
            buffer.as_raw() as *mut _,
            0,
            ptr::null(),
            value as *const T as *const _,
            0,
            0,
        );
        native_d3d11_check_void_upload_device(device, "UpdateSubresource(XMAP10 constant buffer)")
    }

    unsafe fn create_sampler(
        device: &D3dPtr<ID3D11Device>,
        point: bool,
    ) -> Result<D3dPtr<ID3D11SamplerState>> {
        let desc = D3D11_SAMPLER_DESC {
            Filter: if point {
                D3D11_FILTER_MIN_MAG_MIP_POINT
            } else {
                D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT
            },
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            MipLODBias: 0.0,
            MaxAnisotropy: 1,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            BorderColor: [0.0; 4],
            MinLOD: 0.0,
            MaxLOD: f32::MAX,
        };
        let mut sampler = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateSamplerState(&desc, &mut sampler),
            "CreateSamplerState(XMAP10)",
        )?;
        D3dPtr::from_raw(sampler, "XMAP10 sampler")
    }

    unsafe fn create_blend(
        device: &D3dPtr<ID3D11Device>,
        rgb_only: bool,
    ) -> Result<D3dPtr<ID3D11BlendState>> {
        let write_mask = if rgb_only {
            (D3D11_COLOR_WRITE_ENABLE_RED
                | D3D11_COLOR_WRITE_ENABLE_GREEN
                | D3D11_COLOR_WRITE_ENABLE_BLUE) as u8
        } else {
            D3D11_COLOR_WRITE_ENABLE_ALL as u8
        };
        let target = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: 0,
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_ZERO,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_ZERO,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: write_mask,
        };
        let desc = D3D11_BLEND_DESC {
            AlphaToCoverageEnable: 0,
            IndependentBlendEnable: 0,
            RenderTarget: [target; 8],
        };
        let mut state = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateBlendState(&desc, &mut state),
            "CreateBlendState(XMAP10)",
        )?;
        D3dPtr::from_raw(state, "XMAP10 blend state")
    }

    unsafe fn create_depth_disabled(
        device: &D3dPtr<ID3D11Device>,
    ) -> Result<D3dPtr<ID3D11DepthStencilState>> {
        let desc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: 0,
            DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ZERO,
            DepthFunc: D3D11_COMPARISON_ALWAYS,
            StencilEnable: 0,
            StencilReadMask: 0,
            StencilWriteMask: 0,
            FrontFace: mem::zeroed(),
            BackFace: mem::zeroed(),
        };
        let mut state = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateDepthStencilState(&desc, &mut state),
            "CreateDepthStencilState(XMAP10)",
        )?;
        D3dPtr::from_raw(state, "XMAP10 depth-disabled state")
    }

    unsafe fn create_rasterizer(
        device: &D3dPtr<ID3D11Device>,
    ) -> Result<D3dPtr<ID3D11RasterizerState>> {
        let desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            FrontCounterClockwise: 0,
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: 1,
            ScissorEnable: 0,
            MultisampleEnable: 0,
            AntialiasedLineEnable: 0,
        };
        let mut state = ptr::null_mut();
        hr(
            (*device.as_raw()).CreateRasterizerState(&desc, &mut state),
            "CreateRasterizerState(XMAP10)",
        )?;
        D3dPtr::from_raw(state, "XMAP10 rasterizer state")
    }

    fn post_effect_constants(
        frame: &PreparedXmap10Frame,
        width: u32,
        height: u32,
    ) -> PostEffectBaseCb {
        let hsv = frame.grade.hsv_f32_bits.map(f32::from_bits);
        let balance = frame.grade.color_balance_f32_bits.map(f32::from_bits);
        PostEffectBaseCb {
            values: [
                hsv[0],
                hsv[1],
                hsv[2],
                hsv[3],
                balance[0],
                balance[1],
                balance[2],
                f32::from_bits(frame.grade.emissive_bloom_strength_f32_bits),
                2.0 / width as f32,
                2.0 / height as f32,
                1.0 / width as f32,
                1.0 / height as f32,
                1.0,
                1.0,
                f32::from_bits(frame.grade.bright_threshold_f32_bits),
                f32::from_bits(frame.grade.middle_grey_f32_bits),
                f32::from_bits(frame.grade.luminance_white_squared_f32_bits),
                0.0,
                0.0,
                0.0,
            ],
        }
    }

    unsafe fn validate_buffer_desc(
        buffer: *mut ID3D11Buffer,
        byte_width: u32,
        bind_flags: u32,
    ) -> Result<()> {
        let mut desc: D3D11_BUFFER_DESC = mem::zeroed();
        (*buffer).GetDesc(&mut desc);
        if desc.ByteWidth != byte_width || desc.BindFlags & bind_flags != bind_flags {
            Err("XMAP10 SceneCB0 native descriptor mismatch".into())
        } else {
            Ok(())
        }
    }

    pub unsafe fn final_rtv_resource_identity_sha256(
        expected_device: *mut ID3D11Device,
        rtv: *mut ID3D11RenderTargetView,
        allocation_generation: u64,
    ) -> Result<String> {
        if expected_device.is_null() || rtv.is_null() || allocation_generation == 0 {
            return Err("XMAP10 final RTV identity input is invalid".into());
        }
        let mut actual_device = ptr::null_mut();
        (*rtv).GetDevice(&mut actual_device);
        let actual_device = D3dPtr::from_raw(actual_device, "XMAP10 final RTV device")?;
        if actual_device.as_raw() != expected_device {
            return Err("XMAP10 final RTV belongs to a different D3D11 device".into());
        }
        let mut resource = ptr::null_mut();
        (*rtv).GetResource(&mut resource);
        let resource = D3dPtr::from_raw(resource, "XMAP10 final RTV resource")?;
        let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
        hr(
            (*resource.as_raw()).QueryInterface(
                &ID3D11Texture2D::uuidof(),
                &mut texture as *mut *mut ID3D11Texture2D as *mut *mut _,
            ),
            "QueryInterface(XMAP10 final Texture2D)",
        )?;
        let texture = D3dPtr::from_raw(texture, "XMAP10 final Texture2D")?;
        let mut tag = [0u8; 72];
        let mut tag_bytes = u32::try_from(tag.len())?;
        hr(
            (*texture.as_raw()).GetPrivateData(
                &REPLAY_PRESENTATION_RESOURCE_IDENTITY_GUID,
                &mut tag_bytes,
                tag.as_mut_ptr().cast(),
            ),
            "GetPrivateData(XMAP10 final resource identity)",
        )?;
        if usize::try_from(tag_bytes)? != tag.len()
            || u64::from_le_bytes(tag[..8].try_into()?) != allocation_generation
        {
            return Err("XMAP10 final RTV allocation tag/generation mismatch".into());
        }
        let identity = std::str::from_utf8(&tag[8..])?;
        if !is_sha256(identity) {
            return Err("XMAP10 final RTV allocation identity tag is invalid".into());
        }
        Ok(identity.to_owned())
    }

    unsafe fn validate_final_rtv(
        expected_device: *mut ID3D11Device,
        rtv: *mut ID3D11RenderTargetView,
        expected: &Xmap10AttachedFinalTarget,
    ) -> Result<()> {
        let actual_identity = final_rtv_resource_identity_sha256(
            expected_device,
            rtv,
            expected.allocation_generation,
        )?;
        let mut resource = ptr::null_mut();
        (*rtv).GetResource(&mut resource);
        let resource = D3dPtr::from_raw(resource, "XMAP10 final RTV validation resource")?;
        let mut texture: *mut ID3D11Texture2D = ptr::null_mut();
        hr(
            (*resource.as_raw()).QueryInterface(
                &ID3D11Texture2D::uuidof(),
                &mut texture as *mut *mut ID3D11Texture2D as *mut *mut _,
            ),
            "QueryInterface(XMAP10 final validation Texture2D)",
        )?;
        let texture = D3dPtr::from_raw(texture, "XMAP10 final validation Texture2D")?;
        let mut desc: D3D11_TEXTURE2D_DESC = mem::zeroed();
        (*texture.as_raw()).GetDesc(&mut desc);
        if desc.Width != expected.width
            || desc.Height != expected.height
            || desc.Format as u32 != expected.dxgi_format
            || desc.SampleDesc.Count != expected.sample_count
            || actual_identity != expected.resource_identity_sha256
        {
            return Err("XMAP10 final RTV resource descriptor mismatch".into());
        }
        Ok(())
    }

    fn native_pass_names(lut_pass_count: usize, reduction_pass_count: usize) -> Vec<String> {
        let mut passes = (0..lut_pass_count)
            .map(|ordinal| format!("postfx.lut.{ordinal}"))
            .collect::<Vec<_>>();
        passes.push("postfx.bright_pass".into());
        for ordinal in 0..reduction_pass_count {
            passes.push(format!("postfx.luminance_downsample.{ordinal}"));
        }
        passes.extend([
            "postfx.luminance_gather".into(),
            "postfx.bloom.horizontal".into(),
            "postfx.bloom.vertical".into(),
            "postfx.restore_scene".into(),
        ]);
        passes
    }

    fn hr(value: i32, action: &str) -> Result<()> {
        if value < 0 {
            Err(format!("{action} failed with HRESULT 0x{:08x}", value as u32).into())
        } else {
            Ok(())
        }
    }

    pub use final_rtv_resource_identity_sha256 as exported_final_rtv_resource_identity_sha256;
    pub use NativeXmap10PostFxExecutor as ExportedNativeXmap10PostFxExecutor;
    pub use NativeXmap10ScreenShadowHandoff as ExportedNativeXmap10ScreenShadowHandoff;
    pub use NativeXmap10ShaderResourceView as ExportedNativeXmap10ShaderResourceView;
}

#[cfg(windows)]
pub use native::exported_final_rtv_resource_identity_sha256 as xmap10_final_rtv_resource_identity_sha256;
#[cfg(windows)]
pub use native::ExportedNativeXmap10PostFxExecutor as NativeXmap10PostFxExecutor;
#[cfg(windows)]
pub use native::{NativeXmap10ScreenTargets, NativeXmap10RetainedShadowView};
#[cfg(windows)]
pub(crate) use native::NativeXmap10ScreenWriteLease;
#[cfg(windows)]
pub use native::ExportedNativeXmap10ScreenShadowHandoff as NativeXmap10ScreenShadowHandoff;
#[cfg(windows)]
pub use native::ExportedNativeXmap10ShaderResourceView as NativeXmap10ShaderResourceView;

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(label: &str) -> String {
        Sha256::hex(label.as_bytes())
    }

    #[test]
    fn reduction_plan_is_derived_from_the_dynamic_target_graph() {
        let hd = Xmap10TargetPlan::new(1920, 1080).unwrap();
        let hd_plan = Xmap10LuminanceReductionPlan::from_target_plan(&hd).unwrap();
        assert_eq!(hd_plan.steps.len(), 8);
        assert_eq!(
            (
                hd_plan.steps[0].source_width,
                hd_plan.steps[0].source_height,
                hd_plan.steps[0].destination_width,
                hd_plan.steps[0].destination_height,
            ),
            (960, 540, 480, 270)
        );
        assert_eq!(
            (
                hd_plan.steps.last().unwrap().destination_width,
                hd_plan.steps.last().unwrap().destination_height,
            ),
            (4, 3)
        );

        let uhd = Xmap10TargetPlan::new(3840, 2160).unwrap();
        let uhd_plan = Xmap10LuminanceReductionPlan::from_target_plan(&uhd).unwrap();
        assert_eq!(uhd_plan.steps.len(), 9);
        assert_eq!(
            (
                uhd_plan.steps.last().unwrap().destination_width,
                uhd_plan.steps.last().unwrap().destination_height,
            ),
            (4, 3)
        );

        let odd = Xmap10TargetPlan::new(1365, 767).unwrap();
        let odd_plan = Xmap10LuminanceReductionPlan::from_target_plan(&odd).unwrap();
        assert_eq!(
            (
                odd_plan.steps[0].source_width,
                odd_plan.steps[0].source_height,
                odd_plan.steps[0].destination_width,
                odd_plan.steps[0].destination_height,
            ),
            (682, 383, 341, 192)
        );
    }

    #[test]
    fn screen_shadow_receipt_binds_every_producer_and_rejects_tampering() {
        let mut receipt = Xmap10ScreenShadowExecutionReceipt {
            schema: XMAP10_SCREEN_SHADOW_RECEIPT_SCHEMA.into(),
            active: true,
            cleared_color_f32_bits: XMAP10_SCREEN_CLEAR_BITS,
            tree_partitions: [1, 4].into_iter().map(|partition| {
                let mut tree = crate::xmap11_d3d_handoff::Xmap11UnlitPartitionReceipt {
                    partition, prepared_frame_sha256: sha("trees"), program_sha256: sha("tree-program"),
                    matrix_payload_sha256: sha(&format!("matrix-{partition}")), cb1_sha256: sha("tree-cb1"),
                    ordered_batch_indices: vec![partition as u32], resource_sha256: sha("tree-resources"), deterministic_sha256: String::new(),
                };
                tree.deterministic_sha256 = canonical_packet_sha256(&tree).unwrap();
                tree
            }).collect(),
            ordered_partition_matrix_sha256s: vec![sha("matrix-1"), sha("matrix-4")],
            terrain_frame_sha256: sha("terrain"),
            map_copy_receipt_set_sha256: sha("copies"),
            xmap09_light_depth_execution_sha256: sha("light-depth"),
            xmap09_shadow_matrix_payload_sha256: sha("shadow-matrix"),
            screen_chain_plan_sha256: sha("plan"),
            content_closure_sha256: sha("content-closure"),
            ordered_scene_cb0_composition_sha256s: vec![sha("scene-main"), sha("scene-east")],
            terrain_unlit_vertex_bytecode_sha256: sha("terrain-vs"),
            terrain_unlit_pixel_bytecode_sha256: sha("terrain-ps"),
            blur_horizontal_vertex_bytecode_sha256: sha("blur-h-vs"),
            blur_horizontal_pixel_bytecode_sha256: sha("blur-h"),
            blur_vertical_vertex_bytecode_sha256: sha("blur-v-vs"),
            blur_vertical_pixel_bytecode_sha256: sha("blur-v"),
            draw_count: 9,
            binding: Xmap10NativeShaderResourceBinding {
                symbol: "ShadowMap".into(),
                owner_packet: "XMAP10".into(),
                producer_camera_frame_sha256: sha("camera"),
                producer_replay_frame_identity_sha256: sha("replay"),
                consumer_camera_frame_sha256: sha("camera"),
                consumer_replay_frame_identity_sha256: sha("replay"),
                stable_resource_id: "shadow-final".into(),
                resource_sha256: sha("resource"),
                descriptor_sha256: sha("descriptor"),
            },
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = canonical_packet_sha256(&receipt).unwrap();
        receipt.validate().unwrap();
        receipt.draw_count += 1;
        assert!(receipt.validate().is_err());

        receipt.draw_count -= 1;
        receipt.active = false;
        receipt.ordered_scene_cb0_composition_sha256s.clear();
        receipt.tree_partitions.clear();
        receipt.ordered_partition_matrix_sha256s.clear();
        receipt.deterministic_sha256.clear();
        receipt.deterministic_sha256 = canonical_packet_sha256(&receipt).unwrap();
        assert!(
            receipt.validate().is_err(),
            "inactive receipt must record zero draws"
        );
        receipt.draw_count = 0;

        receipt.deterministic_sha256.clear();
        receipt.deterministic_sha256 = canonical_packet_sha256(&receipt).unwrap();
        receipt.validate().unwrap();
        receipt.binding.consumer_camera_frame_sha256 = sha("next-camera");
        receipt.deterministic_sha256.clear();
        receipt.deterministic_sha256 = canonical_packet_sha256(&receipt).unwrap();
        assert!(receipt.validate().is_err(), "inactive clear is a current write, never a retained prior frame");
        assert_eq!(receipt.binding.producer_camera_frame_sha256, sha("camera"));
        assert_eq!(
            receipt.binding.producer_replay_frame_identity_sha256,
            sha("replay")
        );
    }

    #[test]
    fn unlit_reflected_textures_require_exact_producer_slot_coverage() {
        use crate::shader_compile::DxbcResource;
        for (program, supplied) in [
            ("terrainunlit", vec![0, 1, 2, 7, 12]),
            ("treeunlit", vec![0, 1, 2, 3, 4, 6, 8, 9, 10, 11, 12, 13, 14, 15]),
        ] {
            let texture = |slot| DxbcResource { name: format!("AuthoredTexture{slot}"), resource_type: 2,
                dimension: 4, bind_point: slot, bind_count: 1 };
            let exact = supplied.iter().copied().map(texture).collect::<Vec<_>>();
            require_unlit_texture_providers(program, &exact).unwrap();
            for slot in (0..=16).filter(|slot| !supplied.contains(slot)) {
                let mut with_missing_provider = exact.clone();
                with_missing_provider.push(texture(slot));
                let error = require_unlit_texture_providers(program, &with_missing_provider).unwrap_err();
                let refusal = error.downcast_ref::<UnlitTextureProviderRefusal>().unwrap();
                assert_eq!(refusal.program, program);
                assert_eq!(refusal.shader_register, slot);
                assert_eq!(refusal.symbol, format!("AuthoredTexture{slot}"));
                assert!(error.to_string().contains("unlit-reflected-texture-provider-unavailable"));
            }
            let sampler = DxbcResource { name: "UnusedSampler".into(), resource_type: 3,
                dimension: 0, bind_point: 5, bind_count: 1 };
            require_unlit_texture_providers(program, &[sampler]).unwrap();
        }
    }

    #[test]
    fn terrain_unlit_cb1_joins_xmap09_matrix_with_xmap04_draw_rows() {
        let mut xmap04 = vec![0u8; crate::TERRAIN_CHUNK_CB1_BYTES];
        for (index, byte) in xmap04[64..].iter_mut().enumerate() {
            *byte = u8::try_from(index + 1).unwrap();
        }
        let bits = std::array::from_fn(|index| (index as f32 + 0.25).to_bits());
        let matrix_bytes = bits
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let mut matrix = crate::Xmap09ShadowMatrixBinding {
            symbol: "ShadowMapTextureMatrix".into(),
            owner_packet: "XMAP09".into(),
            input_payload_sha256: sha("xmap09-input"),
            shadow_map_texture_matrix_f32_bits: bits,
            payload_sha256: Sha256::hex(&matrix_bytes),
            descriptor_sha256: Sha256::hex(b"XMAP09\0ShadowMapTextureMatrix\0float4x4\064"),
        };
        let joined = xmap10_terrain_unlit_cb1_bytes(&xmap04, &matrix).unwrap();
        assert_eq!(&joined[..64], matrix_bytes);
        assert_eq!(&joined[64..], &xmap04[64..]);

        matrix.shadow_map_texture_matrix_f32_bits[3] = f32::NAN.to_bits();
        assert!(xmap10_terrain_unlit_cb1_bytes(&xmap04, &matrix).is_err());
        matrix.shadow_map_texture_matrix_f32_bits = bits;
        matrix.payload_sha256 = Sha256::hex(&matrix_bytes);
        xmap04[0] = 1;
        assert!(xmap10_terrain_unlit_cb1_bytes(&xmap04, &matrix).is_err());
    }

    #[test]
    fn reduction_plan_rejects_tampered_chain_dimensions() {
        let targets = Xmap10TargetPlan::new(1920, 1080).unwrap();
        let mut plan = Xmap10LuminanceReductionPlan::from_target_plan(&targets).unwrap();
        plan.steps[3].destination_width += 1;
        plan.deterministic_sha256 = reduction_plan_digest(&plan);
        assert!(plan.validate_for_main_scene(1920, 1080).is_err());
    }

    #[test]
    fn lut_execution_recomputes_pass_chain_and_final_content() {
        let source = Sha256::hex(b"lut-source");
        let cubes = vec![
            Sha256::hex(b"cube-0"),
            Sha256::hex(b"cube-1"),
            Sha256::hex(b"cube-2"),
        ];
        let mut pass = Xmap10LutPassExecutionReceipt {
            ordinal: 0,
            output_target_index: 0,
            ordered_input_resource_identity_sha256s: vec![source.clone()],
            output_resource_identity_sha256: cubes[0].clone(),
            program_identity_sha256: Sha256::hex(b"lut-program-native"),
            execution_receipt_sha256: String::new(),
        };
        pass.execution_receipt_sha256 = lut_pass_execution_digest(&pass).unwrap();
        let lut_program = Sha256::hex(b"lut-program-plan");
        let final_content = xmap10_identity_set(
            "postfx.color_cube.final-content",
            &[
                cubes[0].clone(),
                lut_program.clone(),
                xmap10_identity_set(
                    "postfx.lut.ordered-pass-executions",
                    &[pass.execution_receipt_sha256.clone()],
                ),
            ],
        );
        let mut receipt = Xmap10LutExecutionReceipt {
            schema: XMAP10_LUT_EXECUTION_RECEIPT_SCHEMA.into(),
            frame_sequence: 1,
            lut_program_sha256: lut_program,
            ordered_lut_source_resource_identity_sha256s: vec![source],
            ordered_color_cube_resource_identity_sha256s: cubes,
            ordered_passes: vec![pass],
            final_color_cube_content_identity_sha256: final_content,
            deterministic_sha256: String::new(),
        };
        receipt.deterministic_sha256 = canonical_packet_sha256(&receipt).unwrap();
        receipt.validate().unwrap();

        let mut tampered_pass = receipt.clone();
        tampered_pass.ordered_passes[0].ordered_input_resource_identity_sha256s[0] =
            Sha256::hex(b"substituted-source");
        tampered_pass.deterministic_sha256.clear();
        tampered_pass.deterministic_sha256 = canonical_packet_sha256(&tampered_pass).unwrap();
        assert!(tampered_pass.validate().is_err());

        let mut tampered_final = receipt;
        tampered_final.final_color_cube_content_identity_sha256 = Sha256::hex(b"forged-final");
        tampered_final.deterministic_sha256.clear();
        tampered_final.deterministic_sha256 = canonical_packet_sha256(&tampered_final).unwrap();
        assert!(tampered_final.validate().is_err());
    }
}
