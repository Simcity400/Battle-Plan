//! Fail-closed, pointer-free receipt for one generic GUI paint scope.
//!
//! This module deliberately does not classify unit counters. It authenticates
//! the generic outer GUI traversal, one already byte-gated final-draw wrapper,
//! authored GUI identity supplied by the owner resolver, and every D3D payload
//! hash in one selected Present. A caller may submit the resulting receipt to
//! D80/UI assembly only after `materialize_gui_paint_receipt` succeeds.

use portable_content_manifest::UsedContentKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::{
    ENTITY_FINAL_DRAW_RVA, INDEXED_INSTANCED_FINAL_DRAW_RVA, INSTANCED_FINAL_DRAW_RVA,
    PARTICLE_FINAL_DRAW_RVA,
};

pub const GUI_RENDER_2D_TREE_RVA: u32 = 0x0222_84b0;
pub const GUI_RENDER_2D_TREE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41,
    0x57,
];
pub const GUI_RENDER_2D_TREE_INSTRUCTIONS_SHA256: &str =
    "e85d8e23b8f5f869287654827c4bb0d60cd691916ed2d540e305e895b6e4280e";
pub const GUI_RENDER_2D_TREE_EVIDENCE_MANIFEST_SHA256: &str =
    "0b1f8e644087b46b5dfbd69252b2160d73ab56b500905a7923fc6a097ab0722d";
/// Frozen exact-10 replacement authority for the retained CAP6 executable
/// findings. These receipts authenticate static RE provenance only; they are
/// deliberately insufficient to activate live owner/path-to-draw capture.
pub const GUI_PAINT_EXACT10_REEXPORT_RECEIPT_SHA256: &str =
    "00108a3ce3a5419d3ce9c161af810f9540e711adbb703d7fb44e05ad95f1e4f4";
pub const GUI_PAINT_EXACT10_JOB_BINDINGS_SHA256: &str =
    "5b2051cfa0cd6ecbf4daca84a15465ebf9da55e6a3f5661555635dc064d640f1";
pub const GUI_PAINT_EXACT10_POOL_STATUS_SHA256: &str =
    "df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395";
pub const GUI_PAINT_EXACT10_IDENTITY_MANIFEST_SHA256: &str =
    "cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe";
pub const GUI_PAINT_EXACT10_CAP6_ARTIFACTS_SHA256: [&str; 11] = [
    "4e8ee62102fc74a428692bd7f56924f0374a053fdae89315ea4fad8b8c9528dc",
    "5e655a3894773c2e528d749f1d571bdd142b1bef662f982f0280cd3ec45a457c",
    "f8e67f4e71f06ac9dbad672106ae4a444ae315b46232f2413bffcfc403808bdb",
    "b010c88323e06c3fd382233b38ccff990ef12a651651e3dbeff214beb3c07d93",
    "04ec81f1ae2b5f123ac4f9f9e381689df08426e349031bc449a8a3583e0d6770",
    "990aefa465b125acd5ac9d894c62a38456bcd31ab531eba3b10673bc50255c4a",
    "6a25f5c1ac32637271d58bbb6de6dcf7bfbcf43896de605426a41a78153df2c3",
    "7e23cf58268a4f51dd21850218a3da09586d2235f488f12f5ca3e2b1eff8b9f3",
    "0fd6c56d702f010e46503a9d566e9a174c04505c7f6a30ce46ae48b82836275c",
    "54d753943aabdabd6a7ccf6222bc7b14a71c60679ad916d866329487c17af887",
    "74ba4709ad0c169547f4b07f8abad42d79e7a4488175af3ecebfd9b8731491bb",
];
pub const GUI_PAINT_A729_EXECUTABLE_SHA256: &str =
    portable_render_stream::A729_EXECUTABLE_SHA256;
/// The schema/materializer is complete, but no retained or live observation
/// yet authenticates a GUI owner/path scope to these final-draw wrappers.
/// Production must remain zero-draw until the live producer flips this only
/// after recording and revalidating such a receipt.
pub const GUI_PAINT_LIVE_RECEIPT_PROVEN: bool = false;
/// Filled only after an authority-gated owner-control observation exists.
/// `None` keeps activation mechanically impossible in this build.
pub const GUI_PAINT_AUTHORITY_MANIFEST_SHA256: Option<&str> = None;
/// Filled only after a controlled same-Present owner/path-to-draw receipt has
/// been observed. It may not be substituted by a texture/ordinal classifier.
pub const GUI_PAINT_CONTROL_OBSERVATION_SHA256: Option<&str> = None;

const MAX_ACTIVE_GUI_PAINT_SCOPES: usize = 64;
const MAX_PENDING_GUI_PAINT_PRESENTS: usize = 8;
const MAX_PENDING_GUI_PAINT_RECEIPTS: usize = 16_384;
const MAX_COMPLETED_GUI_PAINT_FRONTIERS: usize = 8;

const DRAW_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x8b, 0xf8, 0x8b, 0xda,
    0x85,
];
const DRAW_INDEXED_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x8b, 0xf9, 0x8b, 0xda,
    0x85,
];
const INSTANCED_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x30, 0x41, 0x8b, 0xf8, 0x8b, 0xda,
    0x85,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiPaintFinalDrawHook {
    Draw,
    DrawIndexed,
    DrawInstanced,
    DrawIndexedInstanced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiPaintShaderStage {
    Vertex,
    Pixel,
    Geometry,
    Hull,
    Domain,
    Compute,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintShaderBindingFingerprint {
    pub stage: GuiPaintShaderStage,
    pub bytecode_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintResourceBindingFingerprint {
    pub stage: GuiPaintShaderStage,
    pub slot: u8,
    pub resource_payload_sha256: String,
    pub view_descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintConstantBufferBindingFingerprint {
    pub stage: GuiPaintShaderStage,
    pub slot: u8,
    pub byte_offset: u32,
    pub byte_count: u32,
    pub payload_sha256: String,
}

impl GuiPaintFinalDrawHook {
    fn expected_rva(self) -> u32 {
        match self {
            Self::Draw => PARTICLE_FINAL_DRAW_RVA,
            Self::DrawIndexed => ENTITY_FINAL_DRAW_RVA,
            Self::DrawInstanced => INSTANCED_FINAL_DRAW_RVA,
            Self::DrawIndexedInstanced => INDEXED_INSTANCED_FINAL_DRAW_RVA,
        }
    }

    fn expected_sentinel(self) -> [u8; 16] {
        match self {
            Self::Draw => DRAW_SENTINEL,
            Self::DrawIndexed => DRAW_INDEXED_SENTINEL,
            Self::DrawInstanced | Self::DrawIndexedInstanced => INSTANCED_SENTINEL,
        }
    }
}

/// The exact hook bytes observed before a scope is accepted. The outer hook is
/// a traversal boundary, not a counter discriminator; the final-draw hook only
/// authenticates lowering. The pointer-bearing owner join below is still
/// mandatory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawGuiPaintHookReceipt {
    pub executable_sha256: String,
    pub hook_set_receipt_sha256: String,
    pub render_tree_rva: u32,
    pub render_tree_sentinel: [u8; 16],
    pub render_tree_instructions_sha256: String,
    pub render_tree_evidence_manifest_sha256: String,
    pub final_draw_kind: GuiPaintFinalDrawHook,
    pub final_draw_rva: u32,
    pub final_draw_sentinel: [u8; 16],
}

/// Extractor-local identity observation. Pointers are used only to prove that
/// the owner resolver, runtime root, and graphical traversal refer to the same
/// live scope; none is copied into `GuiPaintReceipt`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawGuiPaintOwnerObservation {
    pub owner_observation_sequence: u64,
    pub gui_owner_ptr: usize,
    pub runtime_root_ptr: usize,
    pub graphical_root_ptr: usize,
    pub graphical_node_ptr: usize,
    pub gui_owner_vtable_rva: u32,
    pub runtime_root_vtable_rva: u32,
    pub graphical_node_vtable_rva: u32,
    pub engine_instance_token: u64,
    pub authored_surface_identity: String,
    pub runtime_root: String,
    pub selected_gui_definition: UsedContentKey,
    pub child_path: Vec<u32>,
    pub element_type: String,
    pub name: String,
    pub paint_ordinal: u32,
}

/// Pointer-free fingerprint of one exact D3D lowering inside the GUI scope.
/// Aggregate hashes cover exact ordered bindings, including valid empty lists;
/// the member lists make shader/resource/CB provenance independently auditable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintDrawFingerprint {
    pub global_render_sequence: u64,
    pub ui_ordinal: u32,
    pub d3d_capture_sequence: u64,
    pub final_draw_kind: GuiPaintFinalDrawHook,
    pub ordered_shader_bindings: Vec<GuiPaintShaderBindingFingerprint>,
    pub ordered_resource_bindings: Vec<GuiPaintResourceBindingFingerprint>,
    pub ordered_constant_buffer_bindings: Vec<GuiPaintConstantBufferBindingFingerprint>,
    pub shader_bindings_sha256: String,
    pub resource_bindings_sha256: String,
    pub constant_buffer_bindings_sha256: String,
    pub geometry_sha256: String,
    pub pipeline_sha256: String,
    pub output_target_sha256: String,
    pub draw_payload_sha256: String,
}

/// One raw paint-scope observation. Entry/exit sequences come from the outer
/// GUI hook; draw sequences come from the byte-gated final-draw hook. Exact
/// adjacency is required, so an interleaved or partially observed scope fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawGuiPaintObservation {
    pub hooks: RawGuiPaintHookReceipt,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub selected_submit_index: u64,
    pub selected_frame_sequence: u64,
    pub selected_present_global_order: u32,
    pub selected_present_receipt_sha256: String,
    pub scope_entry_global_sequence: u64,
    pub scope_exit_global_sequence: u64,
    pub scope_depth_at_entry: u32,
    pub nested_scope_count: u32,
    pub intervening_non_draw_event_count: u32,
    pub expected_draw_count: u32,
    pub expected_first_ui_ordinal: u32,
    pub expected_last_ui_ordinal: u32,
    pub complete_ui_operation_count: u32,
    pub owner: RawGuiPaintOwnerObservation,
    pub draws: Vec<GuiPaintDrawFingerprint>,
}

/// Hook-time observation buffered under the concrete Present candidate before
/// the hour-level selector can know which candidate is authoritative. The
/// selected-Present digest is deliberately absent and is injected only by the
/// deferred materializer after the a729 selection receipt recomputes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawGuiPaintCandidateObservation {
    pub hooks: RawGuiPaintHookReceipt,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub candidate_submit_index: u64,
    pub candidate_frame_sequence: u64,
    pub candidate_present_global_order: u32,
    pub scope_entry_global_sequence: u64,
    pub scope_exit_global_sequence: u64,
    pub scope_depth_at_entry: u32,
    pub nested_scope_count: u32,
    pub intervening_non_draw_event_count: u32,
    pub expected_draw_count: u32,
    pub expected_first_ui_ordinal: u32,
    pub expected_last_ui_ordinal: u32,
    pub complete_ui_operation_count: u32,
    pub owner: RawGuiPaintOwnerObservation,
    pub draws: Vec<GuiPaintDrawFingerprint>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintCommandIdentity {
    pub authored_surface_identity: String,
    pub runtime_root: String,
    pub selected_gui_definition: UsedContentKey,
    pub engine_instance_token: u64,
    pub gui_owner_vtable_rva: u32,
    pub runtime_root_vtable_rva: u32,
    pub graphical_node_vtable_rva: u32,
    pub child_path: Vec<u32>,
    pub element_type: String,
    pub name: String,
    pub paint_ordinal: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintDrawOrdinalSpan {
    pub first_ui_ordinal: u32,
    pub last_ui_ordinal: u32,
    pub draw_count: u32,
}

/// Portable admission receipt produced before D80/UI carrier emission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintReceipt {
    pub executable_sha256: String,
    pub exact10_reexport_receipt_sha256: String,
    pub exact10_job_bindings_sha256: String,
    pub exact10_pool_status_sha256: String,
    pub exact10_identity_manifest_sha256: String,
    pub hook_set_receipt_sha256: String,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub selected_submit_index: u64,
    pub selected_frame_sequence: u64,
    pub selected_present_global_order: u32,
    pub selected_present_receipt_sha256: String,
    pub identity: GuiPaintCommandIdentity,
    pub draw_ordinal_span: GuiPaintDrawOrdinalSpan,
    pub complete_ui_operation_count: u32,
    pub ordered_draws: Vec<GuiPaintDrawFingerprint>,
    pub ordered_draws_sha256: String,
    pub receipt_sha256: String,
}

/// Complete zero-through-N-minus-one UI frontier for one selected Present.
/// D80/UI assembly should accept this wrapper, never an unvalidated loose list
/// of command receipts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintReceiptFrontier {
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub selected_submit_index: u64,
    pub selected_frame_sequence: u64,
    pub selected_present_global_order: u32,
    pub selected_present_receipt_sha256: String,
    pub complete_ui_operation_count: u32,
    pub ordered_receipts: Vec<GuiPaintReceipt>,
    pub ordered_receipt_hashes_sha256: String,
    pub frontier_sha256: String,
}

/// Validate and erase every process pointer. No asset, ordinal interval, CB
/// value, or replay UID is interpreted as a widget role here.
pub fn materialize_gui_paint_receipt(
    raw: RawGuiPaintObservation,
) -> Result<GuiPaintReceipt, String> {
    validate_hook_receipt(&raw.hooks)?;
    validate_owner(&raw.owner)?;
    if raw.process_start_nonce == 0
        || raw.hook_epoch_nonce == 0
        || raw.selected_submit_index == 0
        || raw.selected_frame_sequence == 0
        || raw.selected_present_global_order == 0
    {
        return Err("GUI paint Present/epoch identity contains zero".into());
    }
    check_sha256(
        &raw.selected_present_receipt_sha256,
        "selected-Present receipt",
    )?;
    if raw.scope_entry_global_sequence == 0
        || raw.scope_entry_global_sequence >= raw.scope_exit_global_sequence
        || raw.scope_depth_at_entry != 1
        || raw.nested_scope_count != 0
        || raw.intervening_non_draw_event_count != 0
    {
        return Err("GUI paint scope is nested, interleaved, or has invalid entry/exit order".into());
    }
    let draw_count = u32::try_from(raw.draws.len())
        .map_err(|_| "GUI paint draw count exceeds u32".to_string())?;
    if draw_count == 0 || draw_count != raw.expected_draw_count {
        return Err("GUI paint scope has a missing or partial draw frontier".into());
    }
    if raw.expected_last_ui_ordinal < raw.expected_first_ui_ordinal
        || raw
            .expected_last_ui_ordinal
            .checked_sub(raw.expected_first_ui_ordinal)
            .and_then(|span| span.checked_add(1))
            != Some(draw_count)
        || raw.complete_ui_operation_count == 0
        || raw.expected_last_ui_ordinal >= raw.complete_ui_operation_count
    {
        return Err("GUI paint UI ordinal span/frontier is incomplete".into());
    }
    validate_draw_frontier(&raw)?;

    let identity = GuiPaintCommandIdentity {
        authored_surface_identity: raw.owner.authored_surface_identity,
        runtime_root: raw.owner.runtime_root,
        selected_gui_definition: raw.owner.selected_gui_definition,
        engine_instance_token: raw.owner.engine_instance_token,
        gui_owner_vtable_rva: raw.owner.gui_owner_vtable_rva,
        runtime_root_vtable_rva: raw.owner.runtime_root_vtable_rva,
        graphical_node_vtable_rva: raw.owner.graphical_node_vtable_rva,
        child_path: raw.owner.child_path,
        element_type: raw.owner.element_type,
        name: raw.owner.name,
        paint_ordinal: raw.owner.paint_ordinal,
    };
    let draw_ordinal_span = GuiPaintDrawOrdinalSpan {
        first_ui_ordinal: raw.expected_first_ui_ordinal,
        last_ui_ordinal: raw.expected_last_ui_ordinal,
        draw_count,
    };
    let ordered_draws_sha256 = sha256_json(&raw.draws, "ordered GUI paint draws")?;
    let mut receipt = GuiPaintReceipt {
        executable_sha256: raw.hooks.executable_sha256,
        exact10_reexport_receipt_sha256: GUI_PAINT_EXACT10_REEXPORT_RECEIPT_SHA256.into(),
        exact10_job_bindings_sha256: GUI_PAINT_EXACT10_JOB_BINDINGS_SHA256.into(),
        exact10_pool_status_sha256: GUI_PAINT_EXACT10_POOL_STATUS_SHA256.into(),
        exact10_identity_manifest_sha256: GUI_PAINT_EXACT10_IDENTITY_MANIFEST_SHA256.into(),
        hook_set_receipt_sha256: raw.hooks.hook_set_receipt_sha256,
        process_start_nonce: raw.process_start_nonce,
        hook_epoch_nonce: raw.hook_epoch_nonce,
        selected_submit_index: raw.selected_submit_index,
        selected_frame_sequence: raw.selected_frame_sequence,
        selected_present_global_order: raw.selected_present_global_order,
        selected_present_receipt_sha256: raw.selected_present_receipt_sha256,
        identity,
        draw_ordinal_span,
        complete_ui_operation_count: raw.complete_ui_operation_count,
        ordered_draws: raw.draws,
        ordered_draws_sha256,
        receipt_sha256: String::new(),
    };
    receipt.receipt_sha256 = receipt_body_sha256(&receipt)?;
    Ok(receipt)
}

impl GuiPaintReceipt {
    /// Revalidate a deserialized receipt immediately before UI carrier
    /// emission. This catches mutation, truncation, reordering, or a receipt
    /// copied across Present/epoch boundaries.
    pub fn validate_for_emission(&self) -> Result<(), String> {
        if self.executable_sha256 != GUI_PAINT_A729_EXECUTABLE_SHA256
            || self.exact10_reexport_receipt_sha256
                != GUI_PAINT_EXACT10_REEXPORT_RECEIPT_SHA256
            || self.exact10_job_bindings_sha256 != GUI_PAINT_EXACT10_JOB_BINDINGS_SHA256
            || self.exact10_pool_status_sha256 != GUI_PAINT_EXACT10_POOL_STATUS_SHA256
            || self.exact10_identity_manifest_sha256
                != GUI_PAINT_EXACT10_IDENTITY_MANIFEST_SHA256
            || self.process_start_nonce == 0
            || self.hook_epoch_nonce == 0
            || self.selected_submit_index == 0
            || self.selected_frame_sequence == 0
            || self.selected_present_global_order == 0
        {
            return Err(
                "GUI paint receipt binary/exact-10/Present/epoch authority is invalid".into(),
            );
        }
        check_sha256(&self.hook_set_receipt_sha256, "hook-set receipt")?;
        check_sha256(
            &self.selected_present_receipt_sha256,
            "selected-Present receipt",
        )?;
        validate_portable_identity(&self.identity)?;
        let count = u32::try_from(self.ordered_draws.len())
            .map_err(|_| "GUI paint receipt draw count exceeds u32".to_string())?;
        if count == 0
            || self.draw_ordinal_span.draw_count != count
            || self.draw_ordinal_span.last_ui_ordinal
                < self.draw_ordinal_span.first_ui_ordinal
            || self
                .draw_ordinal_span
                .last_ui_ordinal
                .checked_sub(self.draw_ordinal_span.first_ui_ordinal)
                .and_then(|span| span.checked_add(1))
                != Some(count)
            || self.draw_ordinal_span.last_ui_ordinal >= self.complete_ui_operation_count
        {
            return Err("GUI paint portable ordinal span/frontier is invalid".into());
        }
        validate_portable_draws(
            &self.ordered_draws,
            self.draw_ordinal_span.first_ui_ordinal,
        )?;
        if sha256_json(&self.ordered_draws, "ordered GUI paint draws")?
            != self.ordered_draws_sha256
        {
            return Err("GUI paint ordered-draw digest mismatch".into());
        }
        check_sha256(&self.receipt_sha256, "GUI paint receipt")?;
        if receipt_body_sha256(self)? != self.receipt_sha256 {
            return Err("GUI paint receipt digest mismatch".into());
        }
        Ok(())
    }
}

/// Assemble the only admissible input to UI carrier emission. Gaps,
/// duplicates, overlaps, cross-Present rows, and interleaved global draw order
/// reject the complete frontier.
pub fn materialize_gui_paint_frontier(
    ordered_receipts: Vec<GuiPaintReceipt>,
) -> Result<GuiPaintReceiptFrontier, String> {
    let first = ordered_receipts
        .first()
        .ok_or_else(|| "GUI paint selected-Present frontier is empty".to_string())?;
    first.validate_for_emission()?;
    let process_start_nonce = first.process_start_nonce;
    let hook_epoch_nonce = first.hook_epoch_nonce;
    let selected_submit_index = first.selected_submit_index;
    let selected_frame_sequence = first.selected_frame_sequence;
    let selected_present_global_order = first.selected_present_global_order;
    let selected_present_receipt_sha256 = first.selected_present_receipt_sha256.clone();
    let complete_ui_operation_count = first.complete_ui_operation_count;
    let mut expected_ui_ordinal = 0u32;
    let mut previous_global_render_sequence: Option<u64> = None;
    let mut receipt_hashes = Vec::with_capacity(ordered_receipts.len());
    for receipt in &ordered_receipts {
        receipt.validate_for_emission()?;
        if receipt.process_start_nonce != process_start_nonce
            || receipt.hook_epoch_nonce != hook_epoch_nonce
            || receipt.selected_submit_index != selected_submit_index
            || receipt.selected_frame_sequence != selected_frame_sequence
            || receipt.selected_present_global_order != selected_present_global_order
            || receipt.selected_present_receipt_sha256 != selected_present_receipt_sha256
            || receipt.complete_ui_operation_count != complete_ui_operation_count
        {
            return Err("GUI paint frontier crosses a Present or capture epoch".into());
        }
        if receipt.draw_ordinal_span.first_ui_ordinal != expected_ui_ordinal {
            return Err("GUI paint frontier has a gap, duplicate, or overlap".into());
        }
        let first_draw = receipt
            .ordered_draws
            .first()
            .ok_or_else(|| "GUI paint command receipt has no draw".to_string())?;
        if previous_global_render_sequence.is_some_and(|sequence| {
            sequence.checked_add(1) != Some(first_draw.global_render_sequence)
        }) {
            return Err("GUI paint frontier has an interleaved or missing global draw".into());
        }
        previous_global_render_sequence = receipt
            .ordered_draws
            .last()
            .map(|draw| draw.global_render_sequence);
        expected_ui_ordinal = receipt
            .draw_ordinal_span
            .last_ui_ordinal
            .checked_add(1)
            .ok_or_else(|| "GUI paint frontier ordinal overflowed".to_string())?;
        receipt_hashes.push(receipt.receipt_sha256.as_str());
    }
    if expected_ui_ordinal != complete_ui_operation_count {
        return Err("GUI paint frontier does not close zero through N-minus-one".into());
    }
    let ordered_receipt_hashes_sha256 =
        sha256_json(&receipt_hashes, "ordered GUI paint receipt hashes")?;
    let mut frontier = GuiPaintReceiptFrontier {
        process_start_nonce,
        hook_epoch_nonce,
        selected_submit_index,
        selected_frame_sequence,
        selected_present_global_order,
        selected_present_receipt_sha256,
        complete_ui_operation_count,
        ordered_receipts,
        ordered_receipt_hashes_sha256,
        frontier_sha256: String::new(),
    };
    frontier.frontier_sha256 = frontier_body_sha256(&frontier)?;
    Ok(frontier)
}

impl GuiPaintReceiptFrontier {
    pub fn validate_for_ui_emission(&self) -> Result<(), String> {
        let rebuilt = materialize_gui_paint_frontier(self.ordered_receipts.clone())?;
        if rebuilt.process_start_nonce != self.process_start_nonce
            || rebuilt.hook_epoch_nonce != self.hook_epoch_nonce
            || rebuilt.selected_submit_index != self.selected_submit_index
            || rebuilt.selected_frame_sequence != self.selected_frame_sequence
            || rebuilt.selected_present_global_order != self.selected_present_global_order
            || rebuilt.selected_present_receipt_sha256 != self.selected_present_receipt_sha256
            || rebuilt.complete_ui_operation_count != self.complete_ui_operation_count
            || rebuilt.ordered_receipt_hashes_sha256 != self.ordered_receipt_hashes_sha256
            || rebuilt.frontier_sha256 != self.frontier_sha256
        {
            return Err("GUI paint frontier scalar or digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuiPaintActivationAuthority {
    pub executable_sha256: String,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub exact10_reexport_receipt_sha256: String,
    pub exact10_job_bindings_sha256: String,
    pub exact10_pool_status_sha256: String,
    pub exact10_identity_manifest_sha256: String,
    pub authority_manifest_sha256: String,
    pub control_observation_sha256: String,
    pub hook_set_receipt_sha256: String,
}

/// Authority for the one-time control probe used to obtain the missing live
/// owner/path-to-draw observation.  This mode can produce a diagnostic receipt
/// but can never feed D80/UI or satisfy production activation by itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuiPaintControlProbeAuthority {
    pub executable_sha256: String,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub exact10_reexport_receipt_sha256: String,
    pub exact10_job_bindings_sha256: String,
    pub exact10_pool_status_sha256: String,
    pub exact10_identity_manifest_sha256: String,
    pub hook_set_receipt_sha256: String,
    /// Hash of the immutable harness request which names the selected GUI
    /// definition/root and capture boundary. It is evidence metadata only.
    pub probe_request_sha256: String,
    pub probe_nonce: u64,
    /// Private constructor seal binding this authority to the recomputed
    /// selected-Present receipt. External callers cannot mint this field.
    selected_runtime_binding_sha256: String,
}

impl GuiPaintControlProbeAuthority {
    /// Bind the control probe to the same selected Present and early-launch
    /// hook set already authenticated by the production render runtime.
    #[cfg(windows)]
    pub fn from_selected_render_frame(
        selected: &crate::a729_hook::RawSelectedRenderFrameCapture,
        probe_request_sha256: String,
        probe_nonce: u64,
    ) -> Result<Self, String> {
        crate::a729_hook::validate_selected_render_frame_receipt(selected)?;
        selected.selected.runtime.catalog.validate_launch_binding()?;
        let frame = &selected.selected.frame;
        let catalog = &selected.selected.runtime.catalog;
        if catalog.process.executable_sha256 != GUI_PAINT_A729_EXECUTABLE_SHA256
            || catalog.process.process_start_nonce != frame.process_start_nonce
            || catalog.launch_epoch.process_start_nonce != frame.process_start_nonce
            || catalog.launch_epoch.hook_epoch_nonce != frame.hook_epoch_nonce
            || catalog.submit_index != frame.submit_index
            || catalog.frame_sequence != frame.frame_sequence
        {
            return Err("GUI paint control probe crosses its selected render epoch".into());
        }
        check_sha256(&probe_request_sha256, "control-probe request")?;
        if probe_nonce == 0 {
            return Err("GUI paint control-probe nonce is zero".into());
        }
        let present = GuiPaintPresentKey {
            process_start_nonce: frame.process_start_nonce,
            hook_epoch_nonce: frame.hook_epoch_nonce,
            selected_submit_index: selected.receipt.selected_submit_index,
            selected_frame_sequence: selected.receipt.selected_frame_sequence,
            selected_present_global_order: selected.receipt.selected_global_order,
            selected_present_receipt_sha256: selected.receipt.selection_receipt_sha256.clone(),
        };
        let mut authority = Self {
            executable_sha256: catalog.process.executable_sha256.clone(),
            process_start_nonce: frame.process_start_nonce,
            hook_epoch_nonce: frame.hook_epoch_nonce,
            exact10_reexport_receipt_sha256: GUI_PAINT_EXACT10_REEXPORT_RECEIPT_SHA256.into(),
            exact10_job_bindings_sha256: GUI_PAINT_EXACT10_JOB_BINDINGS_SHA256.into(),
            exact10_pool_status_sha256: GUI_PAINT_EXACT10_POOL_STATUS_SHA256.into(),
            exact10_identity_manifest_sha256: GUI_PAINT_EXACT10_IDENTITY_MANIFEST_SHA256.into(),
            hook_set_receipt_sha256: catalog.launch_epoch.hook_set_receipt_sha256.clone(),
            probe_request_sha256,
            probe_nonce,
            selected_runtime_binding_sha256: String::new(),
        };
        authority.selected_runtime_binding_sha256 = control_probe_runtime_binding_sha256(
            &authority.executable_sha256,
            authority.process_start_nonce,
            authority.hook_epoch_nonce,
            &authority.hook_set_receipt_sha256,
            &authority.probe_request_sha256,
            authority.probe_nonce,
            &present,
        )?;
        Ok(authority)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuiPaintInstrumentationMode {
    #[default]
    Inactive,
    ControlProbe,
    Production,
}

/// Pointer-free output of control-probe mode. `promotion_allowed` is a frozen
/// false field, not a caller option. A later reviewed source change must bind
/// the accepted receipt hash into the production constants separately.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintControlObservationReceipt {
    pub schema: String,
    pub promotion_allowed: bool,
    pub executable_sha256: String,
    pub exact10_reexport_receipt_sha256: String,
    pub exact10_job_bindings_sha256: String,
    pub exact10_pool_status_sha256: String,
    pub exact10_identity_manifest_sha256: String,
    pub hook_set_receipt_sha256: String,
    pub probe_request_sha256: String,
    pub probe_nonce: u64,
    pub selected_runtime_binding_sha256: String,
    pub buffered_candidate_observation_count: u32,
    pub discarded_non_selected_candidate_observation_count: u32,
    pub frontier: GuiPaintReceiptFrontier,
    pub receipt_sha256: String,
}

impl GuiPaintControlObservationReceipt {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != "hoi4.a729.gui-paint-control-observation.v1"
            || self.promotion_allowed
            || self.executable_sha256 != GUI_PAINT_A729_EXECUTABLE_SHA256
            || self.exact10_reexport_receipt_sha256
                != GUI_PAINT_EXACT10_REEXPORT_RECEIPT_SHA256
            || self.exact10_job_bindings_sha256 != GUI_PAINT_EXACT10_JOB_BINDINGS_SHA256
            || self.exact10_pool_status_sha256 != GUI_PAINT_EXACT10_POOL_STATUS_SHA256
            || self.exact10_identity_manifest_sha256
                != GUI_PAINT_EXACT10_IDENTITY_MANIFEST_SHA256
            || self.probe_nonce == 0
            || self.buffered_candidate_observation_count == 0
        {
            return Err("GUI paint control observation lacks frozen diagnostic authority".into());
        }
        check_sha256(&self.hook_set_receipt_sha256, "hook-set receipt")?;
        check_sha256(&self.probe_request_sha256, "control-probe request")?;
        check_sha256(
            &self.selected_runtime_binding_sha256,
            "selected runtime binding",
        )?;
        self.frontier.validate_for_ui_emission()?;
        let selected_count = u32::try_from(self.frontier.ordered_receipts.len())
            .map_err(|_| "GUI paint selected observation count exceeds u32".to_string())?;
        if selected_count == 0
            || self
                .discarded_non_selected_candidate_observation_count
                .checked_add(selected_count)
                != Some(self.buffered_candidate_observation_count)
        {
            return Err("GUI paint control observation candidate accounting is incomplete".into());
        }
        if self.frontier.ordered_receipts.iter().any(|receipt| {
            receipt.executable_sha256 != self.executable_sha256
                || receipt.hook_set_receipt_sha256 != self.hook_set_receipt_sha256
                || receipt.process_start_nonce != self.frontier.process_start_nonce
                || receipt.hook_epoch_nonce != self.frontier.hook_epoch_nonce
        }) {
            return Err("GUI paint control observation crosses its hook/runtime authority".into());
        }
        let present = GuiPaintPresentKey {
            process_start_nonce: self.frontier.process_start_nonce,
            hook_epoch_nonce: self.frontier.hook_epoch_nonce,
            selected_submit_index: self.frontier.selected_submit_index,
            selected_frame_sequence: self.frontier.selected_frame_sequence,
            selected_present_global_order: self.frontier.selected_present_global_order,
            selected_present_receipt_sha256: self
                .frontier
                .selected_present_receipt_sha256
                .clone(),
        };
        if control_probe_runtime_binding_sha256(
            &self.executable_sha256,
            present.process_start_nonce,
            present.hook_epoch_nonce,
            &self.hook_set_receipt_sha256,
            &self.probe_request_sha256,
            self.probe_nonce,
            &present,
        )? != self.selected_runtime_binding_sha256
        {
            return Err("GUI paint control observation selected-runtime binding mismatch".into());
        }
        check_sha256(&self.receipt_sha256, "control observation receipt")?;
        if control_observation_body_sha256(self)? != self.receipt_sha256 {
            return Err("GUI paint control observation digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintPresentKey {
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub selected_submit_index: u64,
    pub selected_frame_sequence: u64,
    pub selected_present_global_order: u32,
    pub selected_present_receipt_sha256: String,
}

impl GuiPaintPresentKey {
    /// Construct the key only after recomputing the current a729 runtime's
    /// selected-Present receipt. This is the supported hook path; copying
    /// retained ordinals or pointers into the public wire shape is rejected by
    /// the recomputation step.
    #[cfg(windows)]
    pub fn from_selected_render_frame(
        selected: &crate::a729_hook::RawSelectedRenderFrameCapture,
    ) -> Result<Self, String> {
        crate::a729_hook::validate_selected_render_frame_receipt(selected)?;
        let frame = &selected.selected.frame;
        Ok(Self {
            process_start_nonce: frame.process_start_nonce,
            hook_epoch_nonce: frame.hook_epoch_nonce,
            selected_submit_index: selected.receipt.selected_submit_index,
            selected_frame_sequence: selected.receipt.selected_frame_sequence,
            selected_present_global_order: selected.receipt.selected_global_order,
            selected_present_receipt_sha256: selected.receipt.selection_receipt_sha256.clone(),
        })
    }
}

/// Materialize hook-time candidates only after the current runtime selects and
/// authenticates one Present. This is the chronological capture path: hooks
/// buffer candidate observations without guessing a future selection digest;
/// the non-selected candidates never receive a portable receipt.
#[cfg(windows)]
pub fn materialize_gui_paint_control_observation_from_selected(
    selected: &crate::a729_hook::RawSelectedRenderFrameCapture,
    probe_request_sha256: String,
    probe_nonce: u64,
    candidates: Vec<RawGuiPaintCandidateObservation>,
) -> Result<GuiPaintControlObservationReceipt, String> {
    let authority = GuiPaintControlProbeAuthority::from_selected_render_frame(
        selected,
        probe_request_sha256,
        probe_nonce,
    )?;
    let present = GuiPaintPresentKey::from_selected_render_frame(selected)?;
    if candidates.is_empty() {
        return Err("selected Present has no buffered GUI paint candidates".into());
    }
    let buffered_candidate_observation_count = u32::try_from(candidates.len())
        .map_err(|_| "GUI paint buffered candidate count exceeds u32".to_string())?;
    let mut receipts = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if candidate.process_start_nonce != present.process_start_nonce
            || candidate.hook_epoch_nonce != present.hook_epoch_nonce
        {
            return Err("GUI paint candidate crosses the selected process/hook epoch".into());
        }
        if candidate.candidate_submit_index != present.selected_submit_index
            || candidate.candidate_frame_sequence != present.selected_frame_sequence
            || candidate.candidate_present_global_order != present.selected_present_global_order
        {
            continue;
        }
        if candidate.hooks.executable_sha256 != authority.executable_sha256
            || candidate.hooks.hook_set_receipt_sha256 != authority.hook_set_receipt_sha256
        {
            return Err("GUI paint candidate crosses the selected hook-set authority".into());
        }
        receipts.push(materialize_gui_paint_receipt(RawGuiPaintObservation {
            hooks: candidate.hooks,
            process_start_nonce: candidate.process_start_nonce,
            hook_epoch_nonce: candidate.hook_epoch_nonce,
            selected_submit_index: candidate.candidate_submit_index,
            selected_frame_sequence: candidate.candidate_frame_sequence,
            selected_present_global_order: candidate.candidate_present_global_order,
            selected_present_receipt_sha256: present
                .selected_present_receipt_sha256
                .clone(),
            scope_entry_global_sequence: candidate.scope_entry_global_sequence,
            scope_exit_global_sequence: candidate.scope_exit_global_sequence,
            scope_depth_at_entry: candidate.scope_depth_at_entry,
            nested_scope_count: candidate.nested_scope_count,
            intervening_non_draw_event_count: candidate.intervening_non_draw_event_count,
            expected_draw_count: candidate.expected_draw_count,
            expected_first_ui_ordinal: candidate.expected_first_ui_ordinal,
            expected_last_ui_ordinal: candidate.expected_last_ui_ordinal,
            complete_ui_operation_count: candidate.complete_ui_operation_count,
            owner: candidate.owner,
            draws: candidate.draws,
        })?);
    }
    if receipts.is_empty() {
        return Err("selected Present has no matching buffered GUI paint observations".into());
    }
    let selected_observation_count = u32::try_from(receipts.len())
        .map_err(|_| "GUI paint selected observation count exceeds u32".to_string())?;
    let discarded_non_selected_candidate_observation_count =
        buffered_candidate_observation_count
            .checked_sub(selected_observation_count)
            .ok_or_else(|| "GUI paint candidate accounting underflowed".to_string())?;
    let frontier = materialize_gui_paint_frontier(receipts)?;
    seal_gui_paint_control_observation(
        &authority,
        frontier,
        buffered_candidate_observation_count,
        discarded_non_selected_candidate_observation_count,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuiPaintScopeStart {
    pub hooks: RawGuiPaintHookReceipt,
    pub present: GuiPaintPresentKey,
    pub scope_entry_global_sequence: u64,
    pub owner: RawGuiPaintOwnerObservation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintInstrumentationTelemetry {
    pub activation_attempt_count: u64,
    pub activation_rejection_count: u64,
    pub activation_state_drop_count: u64,
    pub inactive_begin_count: u64,
    pub inactive_draw_count: u64,
    pub inactive_non_draw_count: u64,
    pub inactive_end_count: u64,
    pub inactive_present_count: u64,
    pub inactive_drain_count: u64,
    pub accepted_scope_begin_count: u64,
    pub accepted_draw_count: u64,
    pub materialized_receipt_count: u64,
    pub emitted_frontier_count: u64,
    pub nested_scope_drop_count: u64,
    pub missing_scope_drop_count: u64,
    pub invalid_owner_or_hook_drop_count: u64,
    pub draw_mismatch_drop_count: u64,
    pub non_draw_interleave_drop_count: u64,
    pub receipt_materialization_drop_count: u64,
    pub cross_present_drop_count: u64,
    pub present_frontier_drop_count: u64,
    pub active_scope_overflow_drop_count: u64,
    pub pending_queue_overflow_drop_count: u64,
    pub completed_queue_overflow_drop_count: u64,
    pub teardown_active_scope_drop_count: u64,
    pub teardown_pending_receipt_drop_count: u64,
    pub teardown_completed_frontier_drop_count: u64,
    pub telemetry_counter_overflow_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuiPaintInstrumentationSnapshot {
    pub active: bool,
    pub mode: GuiPaintInstrumentationMode,
    pub product_emission_allowed: bool,
    pub active_scope_count: usize,
    pub pending_present_count: usize,
    pub pending_receipt_count: usize,
    pub completed_frontier_count: usize,
    pub telemetry: GuiPaintInstrumentationTelemetry,
}

#[derive(Clone, Debug)]
struct ActiveGuiPaintScope {
    start: GuiPaintScopeStart,
    draws: Vec<GuiPaintDrawFingerprint>,
    intervening_non_draw_event_count: u32,
}

#[derive(Default)]
struct GuiPaintInstrumentationState {
    activation_authority: Option<GuiPaintActivationAuthority>,
    control_probe_authority: Option<GuiPaintControlProbeAuthority>,
    active_scopes: BTreeMap<u64, ActiveGuiPaintScope>,
    pending_receipts: BTreeMap<GuiPaintPresentKey, Vec<GuiPaintReceipt>>,
    completed_frontiers: VecDeque<GuiPaintReceiptFrontier>,
    telemetry: GuiPaintInstrumentationTelemetry,
}

static GUI_PAINT_INSTRUMENTATION_ACTIVE: AtomicBool = AtomicBool::new(false);
static GUI_PAINT_INSTRUMENTATION_STATE: OnceLock<Mutex<GuiPaintInstrumentationState>> =
    OnceLock::new();

fn gui_paint_instrumentation_state() -> &'static Mutex<GuiPaintInstrumentationState> {
    GUI_PAINT_INSTRUMENTATION_STATE
        .get_or_init(|| Mutex::new(GuiPaintInstrumentationState::default()))
}

/// Activation is impossible while either frozen authority digest is absent or
/// while `GUI_PAINT_LIVE_RECEIPT_PROVEN` remains false. This is intentionally a
/// code-change gate: runtime callers cannot invent authority strings.
pub fn activate_gui_paint_instrumentation(
    authority: &GuiPaintActivationAuthority,
) -> Result<(), String> {
    let mut state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    let state = &mut *state;
    bump(
        &mut state.telemetry.activation_attempt_count,
        &mut state.telemetry.telemetry_counter_overflow_count,
    );
    let admitted = GUI_PAINT_LIVE_RECEIPT_PROVEN
        && GUI_PAINT_AUTHORITY_MANIFEST_SHA256
            .is_some_and(|expected| authority.authority_manifest_sha256 == expected)
        && GUI_PAINT_CONTROL_OBSERVATION_SHA256
            .is_some_and(|expected| authority.control_observation_sha256 == expected)
        && authority.executable_sha256 == GUI_PAINT_A729_EXECUTABLE_SHA256
        && authority.process_start_nonce != 0
        && authority.hook_epoch_nonce != 0
        && authority.exact10_reexport_receipt_sha256
            == GUI_PAINT_EXACT10_REEXPORT_RECEIPT_SHA256
        && authority.exact10_job_bindings_sha256 == GUI_PAINT_EXACT10_JOB_BINDINGS_SHA256
        && authority.exact10_pool_status_sha256 == GUI_PAINT_EXACT10_POOL_STATUS_SHA256
        && authority.exact10_identity_manifest_sha256
            == GUI_PAINT_EXACT10_IDENTITY_MANIFEST_SHA256
        && check_sha256(&authority.authority_manifest_sha256, "authority manifest").is_ok()
        && check_sha256(&authority.control_observation_sha256, "control observation").is_ok()
        && check_sha256(&authority.hook_set_receipt_sha256, "hook-set receipt").is_ok();
    if !admitted {
        bump(
            &mut state.telemetry.activation_rejection_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        let dropped = clear_gui_paint_rows(state);
        add_count(
            &mut state.telemetry.activation_state_drop_count,
            dropped,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        state.activation_authority = None;
        state.control_probe_authority = None;
        GUI_PAINT_INSTRUMENTATION_ACTIVE.store(false, Ordering::Release);
        return Err(
            "GUI paint live authority/control observations are not frozen; activation stays false"
                .into(),
        );
    }
    if GUI_PAINT_INSTRUMENTATION_ACTIVE.load(Ordering::Acquire)
        || state.activation_authority.is_some()
        || state.control_probe_authority.is_some()
        || !state.active_scopes.is_empty()
        || !state.pending_receipts.is_empty()
        || !state.completed_frontiers.is_empty()
    {
        bump(
            &mut state.telemetry.activation_rejection_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        let dropped = clear_gui_paint_rows(state);
        add_count(
            &mut state.telemetry.activation_state_drop_count,
            dropped,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        state.activation_authority = None;
        state.control_probe_authority = None;
        GUI_PAINT_INSTRUMENTATION_ACTIVE.store(false, Ordering::Release);
        return Err("GUI paint activation encountered non-pristine instrumentation state".into());
    }
    state.activation_authority = Some(authority.clone());
    state.control_probe_authority = None;
    GUI_PAINT_INSTRUMENTATION_ACTIVE.store(true, Ordering::Release);
    Ok(())
}

/// Arm the exact hook pair for a diagnostic control observation without
/// changing any production gate. This is the only mode intended before the
/// CAP6 human discriminator sweeps.
pub fn activate_gui_paint_control_probe(
    authority: &GuiPaintControlProbeAuthority,
) -> Result<(), String> {
    let mut state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    let state = &mut *state;
    bump(
        &mut state.telemetry.activation_attempt_count,
        &mut state.telemetry.telemetry_counter_overflow_count,
    );
    let admitted = authority.executable_sha256 == GUI_PAINT_A729_EXECUTABLE_SHA256
        && authority.process_start_nonce != 0
        && authority.hook_epoch_nonce != 0
        && authority.probe_nonce != 0
        && authority.exact10_reexport_receipt_sha256
            == GUI_PAINT_EXACT10_REEXPORT_RECEIPT_SHA256
        && authority.exact10_job_bindings_sha256 == GUI_PAINT_EXACT10_JOB_BINDINGS_SHA256
        && authority.exact10_pool_status_sha256 == GUI_PAINT_EXACT10_POOL_STATUS_SHA256
        && authority.exact10_identity_manifest_sha256
            == GUI_PAINT_EXACT10_IDENTITY_MANIFEST_SHA256
        && check_sha256(&authority.hook_set_receipt_sha256, "hook-set receipt").is_ok()
        && check_sha256(&authority.probe_request_sha256, "control-probe request").is_ok()
        && check_sha256(
            &authority.selected_runtime_binding_sha256,
            "selected runtime binding",
        )
        .is_ok();
    if !admitted
        || GUI_PAINT_INSTRUMENTATION_ACTIVE.load(Ordering::Acquire)
        || state.activation_authority.is_some()
        || state.control_probe_authority.is_some()
        || !state.active_scopes.is_empty()
        || !state.pending_receipts.is_empty()
        || !state.completed_frontiers.is_empty()
    {
        bump(
            &mut state.telemetry.activation_rejection_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        let dropped = clear_gui_paint_rows(state);
        add_count(
            &mut state.telemetry.activation_state_drop_count,
            dropped,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        state.activation_authority = None;
        state.control_probe_authority = None;
        GUI_PAINT_INSTRUMENTATION_ACTIVE.store(false, Ordering::Release);
        return Err(
            "GUI paint control-probe authority is incomplete or state is not pristine".into(),
        );
    }
    state.activation_authority = None;
    state.control_probe_authority = Some(authority.clone());
    GUI_PAINT_INSTRUMENTATION_ACTIVE.store(true, Ordering::Release);
    Ok(())
}

pub fn gui_paint_instrumentation_is_active() -> bool {
    GUI_PAINT_INSTRUMENTATION_ACTIVE.load(Ordering::Acquire)
}

pub fn gui_paint_instrumentation_mode() -> Result<GuiPaintInstrumentationMode, String> {
    let state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    Ok(instrumentation_mode_locked(&state))
}

pub fn gui_paint_product_emission_is_active() -> bool {
    if !gui_paint_instrumentation_is_active() {
        return false;
    }
    gui_paint_instrumentation_state()
        .lock()
        .is_ok_and(|state| {
            state.activation_authority.is_some() && state.control_probe_authority.is_none()
        })
}

/// Start one generic paint scope after owner resolution. `thread_token` is the
/// exact native thread id supplied by the hook; it is extractor-local and is
/// erased before receipt materialization.
pub fn begin_gui_paint_scope(
    thread_token: u64,
    start: GuiPaintScopeStart,
) -> Result<(), String> {
    if !gui_paint_instrumentation_is_active() {
        record_inactive(|telemetry| &mut telemetry.inactive_begin_count);
        return Err("GUI paint instrumentation activation latch is false".into());
    }
    if thread_token == 0
        || start.scope_entry_global_sequence == 0
        || validate_present_key(&start.present).is_err()
        || validate_hook_receipt(&start.hooks).is_err()
        || validate_owner(&start.owner).is_err()
        || start.owner.owner_observation_sequence < start.scope_entry_global_sequence
    {
        record_drop(|telemetry| &mut telemetry.invalid_owner_or_hook_drop_count);
        return Err("GUI paint scope owner/hook/Present authority is incomplete".into());
    }
    let mut state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    let state = &mut *state;
    let authority_matches = state.activation_authority.as_ref().is_some_and(|authority| {
        authority.executable_sha256 == start.hooks.executable_sha256
            && authority.hook_set_receipt_sha256 == start.hooks.hook_set_receipt_sha256
            && authority.process_start_nonce == start.present.process_start_nonce
            && authority.hook_epoch_nonce == start.present.hook_epoch_nonce
    }) || state.control_probe_authority.as_ref().is_some_and(|authority| {
        authority.executable_sha256 == start.hooks.executable_sha256
            && authority.hook_set_receipt_sha256 == start.hooks.hook_set_receipt_sha256
            && authority.process_start_nonce == start.present.process_start_nonce
            && authority.hook_epoch_nonce == start.present.hook_epoch_nonce
            && control_probe_runtime_binding_sha256(
                &authority.executable_sha256,
                authority.process_start_nonce,
                authority.hook_epoch_nonce,
                &authority.hook_set_receipt_sha256,
                &authority.probe_request_sha256,
                authority.probe_nonce,
                &start.present,
            )
            .is_ok_and(|binding| binding == authority.selected_runtime_binding_sha256)
    });
    if !authority_matches {
        bump(
            &mut state.telemetry.invalid_owner_or_hook_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint scope does not match the activated hook-set authority".into());
    }
    if state.active_scopes.contains_key(&thread_token) {
        state.active_scopes.remove(&thread_token);
        bump(
            &mut state.telemetry.nested_scope_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("nested GUI paint scope rejected both scopes".into());
    }
    if state.active_scopes.len() >= MAX_ACTIVE_GUI_PAINT_SCOPES {
        bump(
            &mut state.telemetry.active_scope_overflow_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint active-scope table is full".into());
    }
    state.active_scopes.insert(
        thread_token,
        ActiveGuiPaintScope {
            start,
            draws: Vec::new(),
            intervening_non_draw_event_count: 0,
        },
    );
    bump(
        &mut state.telemetry.accepted_scope_begin_count,
        &mut state.telemetry.telemetry_counter_overflow_count,
    );
    Ok(())
}

/// Record one exact lowering observed while the owner scope is active. Any
/// hook-kind/order mismatch drops the whole scope immediately.
pub fn record_gui_paint_draw(
    thread_token: u64,
    draw: GuiPaintDrawFingerprint,
) -> Result<(), String> {
    if !gui_paint_instrumentation_is_active() {
        record_inactive(|telemetry| &mut telemetry.inactive_draw_count);
        return Err("GUI paint instrumentation activation latch is false".into());
    }
    let mut state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    let state = &mut *state;
    let Some(scope) = state.active_scopes.get(&thread_token) else {
        bump(
            &mut state.telemetry.missing_scope_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint draw has no active owner scope".into());
    };
    let valid = draw.final_draw_kind == scope.start.hooks.final_draw_kind
        && draw.global_render_sequence > scope.start.scope_entry_global_sequence
        && scope.draws.last().is_none_or(|previous| {
            previous.global_render_sequence.checked_add(1) == Some(draw.global_render_sequence)
                && previous.ui_ordinal.checked_add(1) == Some(draw.ui_ordinal)
                && previous.d3d_capture_sequence < draw.d3d_capture_sequence
        });
    if !valid {
        state.active_scopes.remove(&thread_token);
        bump(
            &mut state.telemetry.draw_mismatch_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint draw kind/order mismatched its owner scope".into());
    }
    let Some(scope) = state.active_scopes.get_mut(&thread_token) else {
        bump(
            &mut state.telemetry.missing_scope_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint owner scope vanished before draw insertion".into());
    };
    scope.draws.push(draw);
    bump(
        &mut state.telemetry.accepted_draw_count,
        &mut state.telemetry.telemetry_counter_overflow_count,
    );
    Ok(())
}

/// Non-draw work inside a command makes its one-to-one lowering ambiguous.
/// The count is retained until end-scope so materialization records the exact
/// rejection rather than silently truncating the command.
pub fn note_gui_paint_non_draw_event(thread_token: u64) -> Result<(), String> {
    if !gui_paint_instrumentation_is_active() {
        record_inactive(|telemetry| &mut telemetry.inactive_non_draw_count);
        return Err("GUI paint instrumentation activation latch is false".into());
    }
    let mut state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    let state = &mut *state;
    let Some(scope) = state.active_scopes.get(&thread_token) else {
        bump(
            &mut state.telemetry.missing_scope_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint non-draw event has no active owner scope".into());
    };
    let Some(next) = scope.intervening_non_draw_event_count.checked_add(1) else {
        state.active_scopes.remove(&thread_token);
        bump(
            &mut state.telemetry.non_draw_interleave_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint non-draw event count overflowed; scope dropped".into());
    };
    let Some(scope) = state.active_scopes.get_mut(&thread_token) else {
        bump(
            &mut state.telemetry.missing_scope_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint owner scope vanished before non-draw insertion".into());
    };
    scope.intervening_non_draw_event_count = next;
    Ok(())
}

/// Close a generic paint scope after its last lowering and queue the portable
/// receipt under its exact selected Present.
pub fn end_gui_paint_scope(
    thread_token: u64,
    scope_exit_global_sequence: u64,
    complete_ui_operation_count: u32,
) -> Result<String, String> {
    if !gui_paint_instrumentation_is_active() {
        record_inactive(|telemetry| &mut telemetry.inactive_end_count);
        return Err("GUI paint instrumentation activation latch is false".into());
    }
    let mut state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    let state = &mut *state;
    let Some(scope) = state.active_scopes.remove(&thread_token) else {
        bump(
            &mut state.telemetry.missing_scope_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint scope end has no active owner scope".into());
    };
    if scope.intervening_non_draw_event_count != 0 {
        bump(
            &mut state.telemetry.non_draw_interleave_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint scope contains an interleaved non-draw event".into());
    }
    let first_ui_ordinal = scope.draws.first().map_or(0, |draw| draw.ui_ordinal);
    let last_ui_ordinal = scope.draws.last().map_or(0, |draw| draw.ui_ordinal);
    let expected_draw_count = u32::try_from(scope.draws.len())
        .map_err(|_| "GUI paint scope draw count exceeds u32".to_string())?;
    let present = scope.start.present.clone();
    let raw = RawGuiPaintObservation {
        hooks: scope.start.hooks,
        process_start_nonce: present.process_start_nonce,
        hook_epoch_nonce: present.hook_epoch_nonce,
        selected_submit_index: present.selected_submit_index,
        selected_frame_sequence: present.selected_frame_sequence,
        selected_present_global_order: present.selected_present_global_order,
        selected_present_receipt_sha256: present.selected_present_receipt_sha256.clone(),
        scope_entry_global_sequence: scope.start.scope_entry_global_sequence,
        scope_exit_global_sequence,
        scope_depth_at_entry: 1,
        nested_scope_count: 0,
        intervening_non_draw_event_count: scope.intervening_non_draw_event_count,
        expected_draw_count,
        expected_first_ui_ordinal: first_ui_ordinal,
        expected_last_ui_ordinal: last_ui_ordinal,
        complete_ui_operation_count,
        owner: scope.start.owner,
        draws: scope.draws,
    };
    let receipt = match materialize_gui_paint_receipt(raw) {
        Ok(receipt) => receipt,
        Err(error) => {
            bump(
                &mut state.telemetry.receipt_materialization_drop_count,
                &mut state.telemetry.telemetry_counter_overflow_count,
            );
            return Err(error);
        }
    };
    let pending_count = state.pending_receipts.values().map(Vec::len).sum::<usize>();
    let new_present = !state.pending_receipts.contains_key(&present);
    if pending_count >= MAX_PENDING_GUI_PAINT_RECEIPTS
        || (new_present && state.pending_receipts.len() >= MAX_PENDING_GUI_PAINT_PRESENTS)
    {
        bump(
            &mut state.telemetry.pending_queue_overflow_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint pending receipt queue is full".into());
    }
    let receipt_sha256 = receipt.receipt_sha256.clone();
    state.pending_receipts.entry(present).or_default().push(receipt);
    bump(
        &mut state.telemetry.materialized_receipt_count,
        &mut state.telemetry.telemetry_counter_overflow_count,
    );
    Ok(receipt_sha256)
}

/// PresentEnd is the only publication boundary. Active scopes and receipts for
/// any other Present are dropped, never carried forward by timing proximity.
pub fn publish_gui_paint_present_frontier(
    present: &GuiPaintPresentKey,
) -> Result<String, String> {
    if !gui_paint_instrumentation_is_active() {
        record_inactive(|telemetry| &mut telemetry.inactive_present_count);
        return Err("GUI paint instrumentation activation latch is false".into());
    }
    validate_present_key(present)?;
    let mut state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    let state = &mut *state;
    let active_drops = state.active_scopes.len() as u64;
    state.active_scopes.clear();
    add_count(
        &mut state.telemetry.cross_present_drop_count,
        active_drops,
        &mut state.telemetry.telemetry_counter_overflow_count,
    );
    let stale_keys = state
        .pending_receipts
        .keys()
        .filter(|key| *key != present)
        .cloned()
        .collect::<Vec<_>>();
    for key in stale_keys {
        let dropped = state
            .pending_receipts
            .remove(&key)
            .map_or(0, |rows| rows.len() as u64);
        add_count(
            &mut state.telemetry.cross_present_drop_count,
            dropped,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
    }
    let Some(receipts) = state.pending_receipts.remove(present) else {
        bump(
            &mut state.telemetry.present_frontier_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("selected Present has no complete GUI paint receipts".into());
    };
    let frontier = match materialize_gui_paint_frontier(receipts) {
        Ok(frontier) => frontier,
        Err(error) => {
            bump(
                &mut state.telemetry.present_frontier_drop_count,
                &mut state.telemetry.telemetry_counter_overflow_count,
            );
            return Err(error);
        }
    };
    if state.completed_frontiers.len() >= MAX_COMPLETED_GUI_PAINT_FRONTIERS {
        bump(
            &mut state.telemetry.completed_queue_overflow_drop_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint completed frontier queue is full".into());
    }
    let frontier_sha256 = frontier.frontier_sha256.clone();
    state.completed_frontiers.push_back(frontier);
    bump(
        &mut state.telemetry.emitted_frontier_count,
        &mut state.telemetry.telemetry_counter_overflow_count,
    );
    Ok(frontier_sha256)
}

pub fn drain_gui_paint_frontiers() -> Result<Vec<GuiPaintReceiptFrontier>, String> {
    if !gui_paint_product_emission_is_active() {
        record_inactive(|telemetry| &mut telemetry.inactive_drain_count);
        return Err("GUI paint production emission latch is false".into());
    }
    let mut state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    let state = &mut *state;
    if !gui_paint_instrumentation_is_active() || state.activation_authority.is_none() {
        bump(
            &mut state.telemetry.inactive_drain_count,
            &mut state.telemetry.telemetry_counter_overflow_count,
        );
        return Err("GUI paint instrumentation deactivated before frontier drain".into());
    }
    Ok(state.completed_frontiers.drain(..).collect())
}

/// Drain diagnostic observations from control-probe mode. The returned rows
/// carry an immutable `promotion_allowed=false` and cannot be consumed by the
/// production frontier drain above.
pub fn drain_gui_paint_control_observations(
) -> Result<Vec<GuiPaintControlObservationReceipt>, String> {
    if !gui_paint_instrumentation_is_active() {
        record_inactive(|telemetry| &mut telemetry.inactive_drain_count);
        return Err("GUI paint instrumentation activation latch is false".into());
    }
    let mut state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    let authority = state
        .control_probe_authority
        .clone()
        .ok_or_else(|| {
            "GUI paint control-probe drain is unavailable in production mode".to_string()
        })?;
    if state.activation_authority.is_some() {
        return Err("GUI paint control-probe and production authorities overlap".into());
    }
    let mut observations = Vec::with_capacity(state.completed_frontiers.len());
    for frontier in state.completed_frontiers.drain(..) {
        let buffered_candidate_observation_count = u32::try_from(frontier.ordered_receipts.len())
            .map_err(|_| "GUI paint buffered candidate count exceeds u32".to_string())?;
        observations.push(seal_gui_paint_control_observation(
            &authority,
            frontier,
            buffered_candidate_observation_count,
            0,
        )?);
    }
    Ok(observations)
}

fn seal_gui_paint_control_observation(
    authority: &GuiPaintControlProbeAuthority,
    frontier: GuiPaintReceiptFrontier,
    buffered_candidate_observation_count: u32,
    discarded_non_selected_candidate_observation_count: u32,
) -> Result<GuiPaintControlObservationReceipt, String> {
    let mut observation = GuiPaintControlObservationReceipt {
        schema: "hoi4.a729.gui-paint-control-observation.v1".into(),
        promotion_allowed: false,
        executable_sha256: authority.executable_sha256.clone(),
        exact10_reexport_receipt_sha256: authority.exact10_reexport_receipt_sha256.clone(),
        exact10_job_bindings_sha256: authority.exact10_job_bindings_sha256.clone(),
        exact10_pool_status_sha256: authority.exact10_pool_status_sha256.clone(),
        exact10_identity_manifest_sha256: authority.exact10_identity_manifest_sha256.clone(),
        hook_set_receipt_sha256: authority.hook_set_receipt_sha256.clone(),
        probe_request_sha256: authority.probe_request_sha256.clone(),
        probe_nonce: authority.probe_nonce,
        selected_runtime_binding_sha256: authority.selected_runtime_binding_sha256.clone(),
        buffered_candidate_observation_count,
        discarded_non_selected_candidate_observation_count,
        frontier,
        receipt_sha256: String::new(),
    };
    observation.receipt_sha256 = control_observation_body_sha256(&observation)?;
    observation.validate()?;
    Ok(observation)
}

/// Disable instrumentation and account for every row that cannot survive
/// module/process teardown. Returned telemetry is pointer-free.
pub fn teardown_gui_paint_instrumentation() -> Result<GuiPaintInstrumentationSnapshot, String> {
    GUI_PAINT_INSTRUMENTATION_ACTIVE.store(false, Ordering::Release);
    let mut state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    let state = &mut *state;
    let active = state.active_scopes.len() as u64;
    let pending = state.pending_receipts.values().map(Vec::len).sum::<usize>() as u64;
    let completed = state.completed_frontiers.len() as u64;
    state.activation_authority = None;
    state.control_probe_authority = None;
    state.active_scopes.clear();
    state.pending_receipts.clear();
    state.completed_frontiers.clear();
    add_count(
        &mut state.telemetry.teardown_active_scope_drop_count,
        active,
        &mut state.telemetry.telemetry_counter_overflow_count,
    );
    add_count(
        &mut state.telemetry.teardown_pending_receipt_drop_count,
        pending,
        &mut state.telemetry.telemetry_counter_overflow_count,
    );
    add_count(
        &mut state.telemetry.teardown_completed_frontier_drop_count,
        completed,
        &mut state.telemetry.telemetry_counter_overflow_count,
    );
    Ok(snapshot_locked(state))
}

pub fn gui_paint_instrumentation_snapshot() -> Result<GuiPaintInstrumentationSnapshot, String> {
    let state = gui_paint_instrumentation_state()
        .lock()
        .map_err(|_| "GUI paint instrumentation state is poisoned".to_string())?;
    Ok(snapshot_locked(&state))
}

fn snapshot_locked(state: &GuiPaintInstrumentationState) -> GuiPaintInstrumentationSnapshot {
    GuiPaintInstrumentationSnapshot {
        active: gui_paint_instrumentation_is_active(),
        mode: instrumentation_mode_locked(state),
        product_emission_allowed: state.activation_authority.is_some()
            && state.control_probe_authority.is_none()
            && gui_paint_instrumentation_is_active(),
        active_scope_count: state.active_scopes.len(),
        pending_present_count: state.pending_receipts.len(),
        pending_receipt_count: state.pending_receipts.values().map(Vec::len).sum(),
        completed_frontier_count: state.completed_frontiers.len(),
        telemetry: state.telemetry,
    }
}

fn instrumentation_mode_locked(
    state: &GuiPaintInstrumentationState,
) -> GuiPaintInstrumentationMode {
    if !gui_paint_instrumentation_is_active() {
        GuiPaintInstrumentationMode::Inactive
    } else if state.control_probe_authority.is_some() && state.activation_authority.is_none() {
        GuiPaintInstrumentationMode::ControlProbe
    } else if state.activation_authority.is_some() && state.control_probe_authority.is_none() {
        GuiPaintInstrumentationMode::Production
    } else {
        GuiPaintInstrumentationMode::Inactive
    }
}

fn clear_gui_paint_rows(state: &mut GuiPaintInstrumentationState) -> u64 {
    let active = state.active_scopes.len() as u64;
    let pending = state.pending_receipts.values().map(Vec::len).sum::<usize>() as u64;
    let completed = state.completed_frontiers.len() as u64;
    state.active_scopes.clear();
    state.pending_receipts.clear();
    state.completed_frontiers.clear();
    active.saturating_add(pending).saturating_add(completed)
}

fn validate_present_key(present: &GuiPaintPresentKey) -> Result<(), String> {
    if present.process_start_nonce == 0
        || present.hook_epoch_nonce == 0
        || present.selected_submit_index == 0
        || present.selected_frame_sequence == 0
        || present.selected_present_global_order == 0
    {
        return Err("GUI paint Present key contains zero authority".into());
    }
    check_sha256(
        &present.selected_present_receipt_sha256,
        "selected-Present receipt",
    )
}

fn record_inactive(field: fn(&mut GuiPaintInstrumentationTelemetry) -> &mut u64) {
    if let Ok(mut state) = gui_paint_instrumentation_state().lock() {
        bump_telemetry_field(&mut state.telemetry, field);
    }
}

fn record_drop(field: fn(&mut GuiPaintInstrumentationTelemetry) -> &mut u64) {
    if let Ok(mut state) = gui_paint_instrumentation_state().lock() {
        bump_telemetry_field(&mut state.telemetry, field);
    }
}

fn bump_telemetry_field(
    telemetry: &mut GuiPaintInstrumentationTelemetry,
    field: fn(&mut GuiPaintInstrumentationTelemetry) -> &mut u64,
) {
    let next = field(telemetry).checked_add(1);
    match next {
        Some(next) => *field(telemetry) = next,
        None => {
            *field(telemetry) = u64::MAX;
            telemetry.telemetry_counter_overflow_count = telemetry
                .telemetry_counter_overflow_count
                .saturating_add(1);
        }
    }
}

fn bump(counter: &mut u64, overflow: &mut u64) {
    add_count(counter, 1, overflow);
}

fn add_count(counter: &mut u64, value: u64, overflow: &mut u64) {
    match counter.checked_add(value) {
        Some(next) => *counter = next,
        None => {
            *counter = u64::MAX;
            *overflow = overflow.saturating_add(1);
        }
    }
}

fn validate_hook_receipt(hooks: &RawGuiPaintHookReceipt) -> Result<(), String> {
    if hooks.executable_sha256 != GUI_PAINT_A729_EXECUTABLE_SHA256
        || hooks.render_tree_rva != GUI_RENDER_2D_TREE_RVA
        || hooks.render_tree_sentinel != GUI_RENDER_2D_TREE_SENTINEL
        || hooks.render_tree_instructions_sha256 != GUI_RENDER_2D_TREE_INSTRUCTIONS_SHA256
        || hooks.render_tree_evidence_manifest_sha256
            != GUI_RENDER_2D_TREE_EVIDENCE_MANIFEST_SHA256
        || hooks.final_draw_rva != hooks.final_draw_kind.expected_rva()
        || hooks.final_draw_sentinel != hooks.final_draw_kind.expected_sentinel()
    {
        return Err("GUI paint hook authority is not the exact byte-gated a729 pair".into());
    }
    check_sha256(&hooks.hook_set_receipt_sha256, "hook-set receipt")
}

fn validate_owner(owner: &RawGuiPaintOwnerObservation) -> Result<(), String> {
    if owner.owner_observation_sequence == 0
        || owner.gui_owner_ptr == 0
        || owner.runtime_root_ptr == 0
        || owner.graphical_root_ptr == 0
        || owner.graphical_node_ptr == 0
        || owner.engine_instance_token == 0
        || !valid_module_rva(owner.gui_owner_vtable_rva)
        || !valid_module_rva(owner.runtime_root_vtable_rva)
        || !valid_module_rva(owner.graphical_node_vtable_rva)
    {
        return Err("GUI paint owner/root/node join is incomplete".into());
    }
    owner
        .selected_gui_definition
        .validate()
        .map_err(|error| format!("GUI paint selected definition is invalid: {error}"))?;
    validate_identity_strings(
        &owner.authored_surface_identity,
        &owner.runtime_root,
        &owner.element_type,
        &owner.name,
    )
}

fn validate_portable_identity(identity: &GuiPaintCommandIdentity) -> Result<(), String> {
    if identity.engine_instance_token == 0
        || !valid_module_rva(identity.gui_owner_vtable_rva)
        || !valid_module_rva(identity.runtime_root_vtable_rva)
        || !valid_module_rva(identity.graphical_node_vtable_rva)
    {
        return Err("GUI paint engine instance/type identity is invalid".into());
    }
    identity
        .selected_gui_definition
        .validate()
        .map_err(|error| format!("GUI paint selected definition is invalid: {error}"))?;
    validate_identity_strings(
        &identity.authored_surface_identity,
        &identity.runtime_root,
        &identity.element_type,
        &identity.name,
    )
}

fn validate_identity_strings(
    authored_surface_identity: &str,
    runtime_root: &str,
    element_type: &str,
    name: &str,
) -> Result<(), String> {
    if authored_surface_identity.trim().is_empty()
        || runtime_root.trim().is_empty()
        || element_type.trim().is_empty()
        || name.trim().is_empty()
    {
        return Err("GUI paint authored identity contains an empty field".into());
    }
    Ok(())
}

fn valid_module_rva(rva: u32) -> bool {
    rva != 0 && rva < 0x0400_0000
}

fn validate_draw_frontier(raw: &RawGuiPaintObservation) -> Result<(), String> {
    if raw
        .draws
        .iter()
        .any(|draw| draw.final_draw_kind != raw.hooks.final_draw_kind)
    {
        return Err("GUI paint draw was not observed by the authenticated lowering hook".into());
    }
    validate_portable_draws(&raw.draws, raw.expected_first_ui_ordinal)?;
    let first = raw
        .draws
        .first()
        .ok_or_else(|| "GUI paint draw frontier is empty".to_string())?;
    let last = raw
        .draws
        .last()
        .ok_or_else(|| "GUI paint draw frontier is empty".to_string())?;
    if first.ui_ordinal != raw.expected_first_ui_ordinal
        || last.ui_ordinal != raw.expected_last_ui_ordinal
        || raw.owner.owner_observation_sequence < raw.scope_entry_global_sequence
        || raw.owner.owner_observation_sequence >= first.global_render_sequence
        || raw.scope_entry_global_sequence >= first.global_render_sequence
        || last.global_render_sequence >= raw.scope_exit_global_sequence
    {
        return Err("GUI paint draw frontier escapes its authenticated scope".into());
    }
    Ok(())
}

fn validate_portable_draws(
    draws: &[GuiPaintDrawFingerprint],
    first_ui_ordinal: u32,
) -> Result<(), String> {
    let mut previous_global: Option<u64> = None;
    let mut previous_capture = None;
    for (index, draw) in draws.iter().enumerate() {
        let index = u32::try_from(index)
            .map_err(|_| "GUI paint draw index exceeds u32".to_string())?;
        if draw.ui_ordinal != first_ui_ordinal.checked_add(index).ok_or_else(|| {
            "GUI paint UI ordinal overflowed while validating the frontier".to_string()
        })? || draw.global_render_sequence == 0
            || draw.d3d_capture_sequence == 0
            || draw.ordered_shader_bindings.is_empty()
        {
            return Err("GUI paint draw order or shader frontier is incomplete".into());
        }
        if previous_global.is_some_and(|value| value.checked_add(1) != Some(draw.global_render_sequence))
            || previous_capture.is_some_and(|value| value >= draw.d3d_capture_sequence)
        {
            return Err("GUI paint draws are duplicated, interleaved, or out of order".into());
        }
        if !draw
            .ordered_shader_bindings
            .iter()
            .any(|binding| binding.stage == GuiPaintShaderStage::Vertex)
            || !draw
                .ordered_shader_bindings
                .iter()
                .any(|binding| binding.stage == GuiPaintShaderStage::Pixel)
        {
            return Err("GUI paint draw lacks its vertex or pixel shader".into());
        }
        for (binding_index, binding) in draw.ordered_shader_bindings.iter().enumerate() {
            if draw.ordered_shader_bindings[..binding_index]
                .iter()
                .any(|prior| prior.stage == binding.stage)
            {
                return Err("GUI paint shader stage is duplicated".into());
            }
            check_sha256(&binding.bytecode_sha256, "shader bytecode")?;
        }
        for (binding_index, binding) in draw.ordered_resource_bindings.iter().enumerate() {
            if draw.ordered_resource_bindings[..binding_index]
                .iter()
                .any(|prior| prior.stage == binding.stage && prior.slot == binding.slot)
            {
                return Err("GUI paint resource stage/slot is duplicated".into());
            }
            check_sha256(&binding.resource_payload_sha256, "resource payload")?;
            check_sha256(&binding.view_descriptor_sha256, "resource view descriptor")?;
        }
        for (binding_index, binding) in draw
            .ordered_constant_buffer_bindings
            .iter()
            .enumerate()
        {
            if draw.ordered_constant_buffer_bindings[..binding_index]
                .iter()
                .any(|prior| prior.stage == binding.stage && prior.slot == binding.slot)
            {
                return Err("GUI paint constant-buffer stage/slot is duplicated".into());
            }
            if binding.byte_count == 0 {
                return Err("GUI paint constant-buffer binding has zero bytes".into());
            }
            check_sha256(&binding.payload_sha256, "constant buffer")?;
        }
        if sha256_json(&draw.ordered_shader_bindings, "shader bindings")?
            != draw.shader_bindings_sha256
            || sha256_json(&draw.ordered_resource_bindings, "resource bindings")?
                != draw.resource_bindings_sha256
            || sha256_json(
                &draw.ordered_constant_buffer_bindings,
                "constant-buffer bindings",
            )? != draw.constant_buffer_bindings_sha256
        {
            return Err("GUI paint binding aggregate digest mismatch".into());
        }
        for (label, hash) in [
            ("shader bindings", &draw.shader_bindings_sha256),
            ("resource bindings", &draw.resource_bindings_sha256),
            ("constant-buffer bindings", &draw.constant_buffer_bindings_sha256),
            ("geometry", &draw.geometry_sha256),
            ("pipeline", &draw.pipeline_sha256),
            ("output target", &draw.output_target_sha256),
            ("draw payload", &draw.draw_payload_sha256),
        ] {
            check_sha256(hash, label)?;
        }
        previous_global = Some(draw.global_render_sequence);
        previous_capture = Some(draw.d3d_capture_sequence);
    }
    Ok(())
}

fn receipt_body_sha256(receipt: &GuiPaintReceipt) -> Result<String, String> {
    #[derive(Serialize)]
    struct Body<'a> {
        executable_sha256: &'a str,
        exact10_reexport_receipt_sha256: &'a str,
        exact10_job_bindings_sha256: &'a str,
        exact10_pool_status_sha256: &'a str,
        exact10_identity_manifest_sha256: &'a str,
        hook_set_receipt_sha256: &'a str,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
        selected_submit_index: u64,
        selected_frame_sequence: u64,
        selected_present_global_order: u32,
        selected_present_receipt_sha256: &'a str,
        identity: &'a GuiPaintCommandIdentity,
        draw_ordinal_span: GuiPaintDrawOrdinalSpan,
        complete_ui_operation_count: u32,
        ordered_draws_sha256: &'a str,
    }
    sha256_json(
        &Body {
            executable_sha256: &receipt.executable_sha256,
            exact10_reexport_receipt_sha256: &receipt.exact10_reexport_receipt_sha256,
            exact10_job_bindings_sha256: &receipt.exact10_job_bindings_sha256,
            exact10_pool_status_sha256: &receipt.exact10_pool_status_sha256,
            exact10_identity_manifest_sha256: &receipt.exact10_identity_manifest_sha256,
            hook_set_receipt_sha256: &receipt.hook_set_receipt_sha256,
            process_start_nonce: receipt.process_start_nonce,
            hook_epoch_nonce: receipt.hook_epoch_nonce,
            selected_submit_index: receipt.selected_submit_index,
            selected_frame_sequence: receipt.selected_frame_sequence,
            selected_present_global_order: receipt.selected_present_global_order,
            selected_present_receipt_sha256: &receipt.selected_present_receipt_sha256,
            identity: &receipt.identity,
            draw_ordinal_span: receipt.draw_ordinal_span,
            complete_ui_operation_count: receipt.complete_ui_operation_count,
            ordered_draws_sha256: &receipt.ordered_draws_sha256,
        },
        "GUI paint receipt",
    )
}

fn frontier_body_sha256(frontier: &GuiPaintReceiptFrontier) -> Result<String, String> {
    #[derive(Serialize)]
    struct Body<'a> {
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
        selected_submit_index: u64,
        selected_frame_sequence: u64,
        selected_present_global_order: u32,
        selected_present_receipt_sha256: &'a str,
        complete_ui_operation_count: u32,
        ordered_receipt_hashes_sha256: &'a str,
    }
    sha256_json(
        &Body {
            process_start_nonce: frontier.process_start_nonce,
            hook_epoch_nonce: frontier.hook_epoch_nonce,
            selected_submit_index: frontier.selected_submit_index,
            selected_frame_sequence: frontier.selected_frame_sequence,
            selected_present_global_order: frontier.selected_present_global_order,
            selected_present_receipt_sha256: &frontier.selected_present_receipt_sha256,
            complete_ui_operation_count: frontier.complete_ui_operation_count,
            ordered_receipt_hashes_sha256: &frontier.ordered_receipt_hashes_sha256,
        },
        "GUI paint frontier",
    )
}

fn control_probe_runtime_binding_sha256(
    executable_sha256: &str,
    process_start_nonce: u64,
    hook_epoch_nonce: u64,
    hook_set_receipt_sha256: &str,
    probe_request_sha256: &str,
    probe_nonce: u64,
    present: &GuiPaintPresentKey,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Body<'a> {
        executable_sha256: &'a str,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
        hook_set_receipt_sha256: &'a str,
        probe_request_sha256: &'a str,
        probe_nonce: u64,
        present: &'a GuiPaintPresentKey,
    }
    sha256_json(
        &Body {
            executable_sha256,
            process_start_nonce,
            hook_epoch_nonce,
            hook_set_receipt_sha256,
            probe_request_sha256,
            probe_nonce,
            present,
        },
        "GUI paint control-probe selected runtime binding",
    )
}

fn control_observation_body_sha256(
    observation: &GuiPaintControlObservationReceipt,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Body<'a> {
        schema: &'a str,
        promotion_allowed: bool,
        executable_sha256: &'a str,
        exact10_reexport_receipt_sha256: &'a str,
        exact10_job_bindings_sha256: &'a str,
        exact10_pool_status_sha256: &'a str,
        exact10_identity_manifest_sha256: &'a str,
        hook_set_receipt_sha256: &'a str,
        probe_request_sha256: &'a str,
        probe_nonce: u64,
        selected_runtime_binding_sha256: &'a str,
        buffered_candidate_observation_count: u32,
        discarded_non_selected_candidate_observation_count: u32,
        frontier_sha256: &'a str,
    }
    sha256_json(
        &Body {
            schema: &observation.schema,
            promotion_allowed: observation.promotion_allowed,
            executable_sha256: &observation.executable_sha256,
            exact10_reexport_receipt_sha256: &observation.exact10_reexport_receipt_sha256,
            exact10_job_bindings_sha256: &observation.exact10_job_bindings_sha256,
            exact10_pool_status_sha256: &observation.exact10_pool_status_sha256,
            exact10_identity_manifest_sha256: &observation.exact10_identity_manifest_sha256,
            hook_set_receipt_sha256: &observation.hook_set_receipt_sha256,
            probe_request_sha256: &observation.probe_request_sha256,
            probe_nonce: observation.probe_nonce,
            selected_runtime_binding_sha256: &observation.selected_runtime_binding_sha256,
            buffered_candidate_observation_count: observation
                .buffered_candidate_observation_count,
            discarded_non_selected_candidate_observation_count: observation
                .discarded_non_selected_candidate_observation_count,
            frontier_sha256: &observation.frontier.frontier_sha256,
        },
        "GUI paint control observation",
    )
}

fn sha256_json<T: Serialize + ?Sized>(value: &T, label: &str) -> Result<String, String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| format!("could not serialize {label}: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn check_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} SHA-256 is not canonical lowercase hex"));
    }
    Ok(())
}

#[cfg(test)]
mod instrumentation_tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn hash(byte: char) -> String {
        std::iter::repeat(byte).take(64).collect()
    }

    fn present(order: u32) -> GuiPaintPresentKey {
        GuiPaintPresentKey {
            process_start_nonce: 1,
            hook_epoch_nonce: 2,
            selected_submit_index: u64::from(order),
            selected_frame_sequence: u64::from(order),
            selected_present_global_order: order,
            selected_present_receipt_sha256: hash('a'),
        }
    }

    fn start(present: GuiPaintPresentKey) -> GuiPaintScopeStart {
        GuiPaintScopeStart {
            hooks: RawGuiPaintHookReceipt {
                executable_sha256: GUI_PAINT_A729_EXECUTABLE_SHA256.into(),
                hook_set_receipt_sha256: hash('b'),
                render_tree_rva: GUI_RENDER_2D_TREE_RVA,
                render_tree_sentinel: GUI_RENDER_2D_TREE_SENTINEL,
                render_tree_instructions_sha256:
                    GUI_RENDER_2D_TREE_INSTRUCTIONS_SHA256.into(),
                render_tree_evidence_manifest_sha256:
                    GUI_RENDER_2D_TREE_EVIDENCE_MANIFEST_SHA256.into(),
                final_draw_kind: GuiPaintFinalDrawHook::Draw,
                final_draw_rva: PARTICLE_FINAL_DRAW_RVA,
                final_draw_sentinel: DRAW_SENTINEL,
            },
            present,
            scope_entry_global_sequence: 10,
            owner: RawGuiPaintOwnerObservation {
                owner_observation_sequence: 11,
                gui_owner_ptr: 0x1000,
                runtime_root_ptr: 0x2000,
                graphical_root_ptr: 0x3000,
                graphical_node_ptr: 0x4000,
                gui_owner_vtable_rva: 0x0100_0000,
                runtime_root_vtable_rva: 0x0100_1000,
                graphical_node_vtable_rva: 0x0100_2000,
                engine_instance_token: 7,
                authored_surface_identity: "mapicons".into(),
                runtime_root: "mapicons_container".into(),
                selected_gui_definition: UsedContentKey {
                    effective_vfs_manifest_sha256: hash('c'),
                    normalized_game_relative_path: "interface/mapicons.gui".into(),
                    winning_layer_ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    source_sha256: hash('d'),
                    payload_sha256: hash('e'),
                    payload_bytes: 1,
                },
                child_path: vec![0, 1],
                element_type: "textBoxType".into(),
                name: "count_txt".into(),
                paint_ordinal: 0,
            },
        }
    }

    fn draw() -> GuiPaintDrawFingerprint {
        let ordered_shader_bindings = vec![
            GuiPaintShaderBindingFingerprint {
                stage: GuiPaintShaderStage::Vertex,
                bytecode_sha256: hash('1'),
            },
            GuiPaintShaderBindingFingerprint {
                stage: GuiPaintShaderStage::Pixel,
                bytecode_sha256: hash('2'),
            },
        ];
        let ordered_resource_bindings = Vec::new();
        let ordered_constant_buffer_bindings = Vec::new();
        GuiPaintDrawFingerprint {
            global_render_sequence: 12,
            ui_ordinal: 0,
            d3d_capture_sequence: 1,
            final_draw_kind: GuiPaintFinalDrawHook::Draw,
            shader_bindings_sha256: sha256_json(&ordered_shader_bindings, "test shaders")
                .unwrap(),
            resource_bindings_sha256: sha256_json(&ordered_resource_bindings, "test resources")
                .unwrap(),
            constant_buffer_bindings_sha256: sha256_json(
                &ordered_constant_buffer_bindings,
                "test constant buffers",
            )
            .unwrap(),
            ordered_shader_bindings,
            ordered_resource_bindings,
            ordered_constant_buffer_bindings,
            geometry_sha256: hash('3'),
            pipeline_sha256: hash('4'),
            output_target_sha256: hash('5'),
            draw_payload_sha256: hash('6'),
        }
    }

    fn lock_and_reset() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        teardown_gui_paint_instrumentation().unwrap();
        guard
    }

    fn force_test_activation() {
        let mut state = gui_paint_instrumentation_state().lock().unwrap();
        state.control_probe_authority = None;
        state.activation_authority = Some(GuiPaintActivationAuthority {
            executable_sha256: GUI_PAINT_A729_EXECUTABLE_SHA256.into(),
            process_start_nonce: 1,
            hook_epoch_nonce: 2,
            exact10_reexport_receipt_sha256: GUI_PAINT_EXACT10_REEXPORT_RECEIPT_SHA256.into(),
            exact10_job_bindings_sha256: GUI_PAINT_EXACT10_JOB_BINDINGS_SHA256.into(),
            exact10_pool_status_sha256: GUI_PAINT_EXACT10_POOL_STATUS_SHA256.into(),
            exact10_identity_manifest_sha256: GUI_PAINT_EXACT10_IDENTITY_MANIFEST_SHA256.into(),
            authority_manifest_sha256: hash('7'),
            control_observation_sha256: hash('8'),
            hook_set_receipt_sha256: hash('b'),
        });
        GUI_PAINT_INSTRUMENTATION_ACTIVE.store(true, Ordering::Release);
    }

    fn probe_authority() -> GuiPaintControlProbeAuthority {
        let mut authority = GuiPaintControlProbeAuthority {
            executable_sha256: GUI_PAINT_A729_EXECUTABLE_SHA256.into(),
            process_start_nonce: 1,
            hook_epoch_nonce: 2,
            exact10_reexport_receipt_sha256: GUI_PAINT_EXACT10_REEXPORT_RECEIPT_SHA256.into(),
            exact10_job_bindings_sha256: GUI_PAINT_EXACT10_JOB_BINDINGS_SHA256.into(),
            exact10_pool_status_sha256: GUI_PAINT_EXACT10_POOL_STATUS_SHA256.into(),
            exact10_identity_manifest_sha256: GUI_PAINT_EXACT10_IDENTITY_MANIFEST_SHA256.into(),
            hook_set_receipt_sha256: hash('b'),
            probe_request_sha256: hash('f'),
            probe_nonce: 3,
            selected_runtime_binding_sha256: String::new(),
        };
        authority.selected_runtime_binding_sha256 = control_probe_runtime_binding_sha256(
            &authority.executable_sha256,
            authority.process_start_nonce,
            authority.hook_epoch_nonce,
            &authority.hook_set_receipt_sha256,
            &authority.probe_request_sha256,
            authority.probe_nonce,
            &present(1),
        )
        .unwrap();
        authority
    }

    #[test]
    fn control_probe_is_diagnostic_only_and_seals_observation() {
        let _guard = lock_and_reset();
        activate_gui_paint_control_probe(&probe_authority()).unwrap();
        assert_eq!(
            gui_paint_instrumentation_mode().unwrap(),
            GuiPaintInstrumentationMode::ControlProbe
        );
        assert!(!gui_paint_product_emission_is_active());

        let key = present(1);
        begin_gui_paint_scope(44, start(key.clone())).unwrap();
        record_gui_paint_draw(44, draw()).unwrap();
        end_gui_paint_scope(44, 13, 1).unwrap();
        publish_gui_paint_present_frontier(&key).unwrap();

        assert!(drain_gui_paint_frontiers().is_err());
        let rows = drain_gui_paint_control_observations().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].promotion_allowed);
        rows[0].validate().unwrap();
        teardown_gui_paint_instrumentation().unwrap();
    }

    #[test]
    fn production_activation_latch_rejects_unfrozen_authority() {
        let _guard = lock_and_reset();
        let before = gui_paint_instrumentation_snapshot().unwrap().telemetry;
        let result = activate_gui_paint_instrumentation(&GuiPaintActivationAuthority {
            executable_sha256: GUI_PAINT_A729_EXECUTABLE_SHA256.into(),
            process_start_nonce: 1,
            hook_epoch_nonce: 2,
            exact10_reexport_receipt_sha256: GUI_PAINT_EXACT10_REEXPORT_RECEIPT_SHA256.into(),
            exact10_job_bindings_sha256: GUI_PAINT_EXACT10_JOB_BINDINGS_SHA256.into(),
            exact10_pool_status_sha256: GUI_PAINT_EXACT10_POOL_STATUS_SHA256.into(),
            exact10_identity_manifest_sha256: GUI_PAINT_EXACT10_IDENTITY_MANIFEST_SHA256.into(),
            authority_manifest_sha256: hash('7'),
            control_observation_sha256: hash('8'),
            hook_set_receipt_sha256: hash('9'),
        });
        assert!(result.is_err());
        assert!(!gui_paint_instrumentation_is_active());
        let after = gui_paint_instrumentation_snapshot().unwrap().telemetry;
        assert_eq!(after.activation_attempt_count, before.activation_attempt_count + 1);
        assert_eq!(
            after.activation_rejection_count,
            before.activation_rejection_count + 1
        );
    }

    #[test]
    fn inactive_hooks_emit_no_receipt_and_account_drops() {
        let _guard = lock_and_reset();
        let before = gui_paint_instrumentation_snapshot().unwrap().telemetry;
        assert!(begin_gui_paint_scope(1, start(present(1))).is_err());
        assert!(record_gui_paint_draw(1, draw()).is_err());
        assert!(note_gui_paint_non_draw_event(1).is_err());
        assert!(end_gui_paint_scope(1, 13, 1).is_err());
        assert!(publish_gui_paint_present_frontier(&present(1)).is_err());
        assert!(drain_gui_paint_frontiers().is_err());
        let after = gui_paint_instrumentation_snapshot().unwrap().telemetry;
        assert_eq!(after.inactive_begin_count, before.inactive_begin_count + 1);
        assert_eq!(after.inactive_draw_count, before.inactive_draw_count + 1);
        assert_eq!(
            after.inactive_non_draw_count,
            before.inactive_non_draw_count + 1
        );
        assert_eq!(after.inactive_end_count, before.inactive_end_count + 1);
        assert_eq!(after.inactive_present_count, before.inactive_present_count + 1);
        assert_eq!(after.inactive_drain_count, before.inactive_drain_count + 1);
    }

    #[test]
    fn test_only_active_path_materializes_and_publishes_exact_frontier() {
        let _guard = lock_and_reset();
        force_test_activation();
        let key = present(1);
        begin_gui_paint_scope(44, start(key.clone())).unwrap();
        record_gui_paint_draw(44, draw()).unwrap();
        let receipt_hash = end_gui_paint_scope(44, 13, 1).unwrap();
        assert_eq!(receipt_hash.len(), 64);
        let frontier_hash = publish_gui_paint_present_frontier(&key).unwrap();
        assert_eq!(frontier_hash.len(), 64);
        let frontiers = drain_gui_paint_frontiers().unwrap();
        assert_eq!(frontiers.len(), 1);
        frontiers[0].validate_for_ui_emission().unwrap();
        teardown_gui_paint_instrumentation().unwrap();
    }

    #[test]
    fn teardown_accounts_active_pending_and_completed_rows() {
        let _guard = lock_and_reset();
        force_test_activation();
        let first = present(1);
        begin_gui_paint_scope(1, start(first.clone())).unwrap();
        record_gui_paint_draw(1, draw()).unwrap();
        end_gui_paint_scope(1, 13, 1).unwrap();
        publish_gui_paint_present_frontier(&first).unwrap();
        let second = present(2);
        begin_gui_paint_scope(2, start(second.clone())).unwrap();
        record_gui_paint_draw(2, draw()).unwrap();
        end_gui_paint_scope(2, 13, 1).unwrap();
        begin_gui_paint_scope(3, start(second)).unwrap();
        let before = gui_paint_instrumentation_snapshot().unwrap().telemetry;
        let snapshot = teardown_gui_paint_instrumentation().unwrap();
        assert!(!snapshot.active);
        assert_eq!(snapshot.active_scope_count, 0);
        assert_eq!(
            snapshot.telemetry.teardown_active_scope_drop_count,
            before.teardown_active_scope_drop_count + 1
        );
        assert_eq!(
            snapshot.telemetry.teardown_pending_receipt_drop_count,
            before.teardown_pending_receipt_drop_count + 1
        );
        assert_eq!(
            snapshot.telemetry.teardown_completed_frontier_drop_count,
            before.teardown_completed_frontier_drop_count + 1
        );
    }
}
