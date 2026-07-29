//! Early-launch authority for exact d77/d80 rendering capture.
//!
//! Exact D3D creation receipts cannot be recovered after the game has already
//! created immutable shader/state objects.  This module therefore creates the
//! selected game process suspended, injects the already-validated collector,
//! waits for a process-epoch-bound complete hook receipt, and resumes the
//! primary thread exactly once.  Paths remain recording-local; the hashed
//! launch manifest contains only portable content identities.

use crate::portable_content_manifest::PortableLaunchBinding;
use portable_content_manifest::{ContentLayerKind, LayerStorageKind, RecordedContentLayer};
use serde::{Deserialize, Serialize};
use snapshot::sha256::{to_hex, Sha256};
use std::collections::BTreeMap;
use std::ffi::{c_void, OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

pub const EARLY_LAUNCH_MANIFEST_SCHEMA: &str = "extractor.early-render-launch-manifest.v1";
pub const EARLY_HOOK_READY_SCHEMA: &str = "extractor.early-render-hook-ready.v1";
pub const EARLY_CAPTURE_RECEIPT_SCHEMA: &str = "extractor.early-render-capture-receipt.v1";
pub const EARLY_DIAGNOSTIC_LIVENESS_SCHEMA: &str = "extractor.early-render-diagnostic-liveness.v1";
pub const EARLY_DIAGNOSTIC_STAGE_MASK_FULL: u64 = 0x3ff;
pub const EARLY_DIAGNOSTIC_STAGE_MASK_COM: u64 = 0x01;
pub const EARLY_DIAGNOSTIC_COM_HOOK_MASK_SHIFT: u32 =
    compact_contract::early_render_control::EARLY_DIAGNOSTIC_COM_SELECTOR_SHIFT;
pub const EARLY_DIAGNOSTIC_COM_HOOK_MASK_FULL: u64 =
    compact_contract::early_render_control::EARLY_DIAGNOSTIC_COM_SELECTOR_VALUE_MASK;
pub const G2_CAPTURE_INTENT_SCHEMA: &str = "hoi4-g2-recorder-capture-intent-v1";
pub const G2_ENGINE_WITNESS_SCHEMA: &str = "hoi4-g2-engine-pixel-witness-v3";
pub const G2_ENGINE_BMP_FILE: &str = "g2_engine_map_boundary.bmp";
pub const G2_ENGINE_WITNESS_FILE: &str = "g2_engine_pixel_witness.json";
const G2_ENGINE_STATUS: &str = "AUTHENTICATED_POST_PROCESS_END_MAP_READBACK";
const G2_RENDER_BOUNDARY: &str = "post_authenticated_map_bgra8_no_selected_present_writers";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct G2CaptureIntentFile {
    pub schema: String,
    pub observation_id: String,
    pub settings_file_sha256: String,
    pub localisation_language: String,
    pub camera_payload_sha256: String,
    pub pov_country_or_global: String,
    pub expected_selected_hour: i64,
    pub capture_context_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct G2CaptureIntent {
    pub observation_id: String,
    pub settings_file_sha256: String,
    pub localisation_language: String,
    pub camera_payload_sha256: String,
    pub pov_country_or_global: String,
    pub expected_selected_hour: i64,
    pub capture_context_path: PathBuf,
    pub capture_context_sha256: String,
}

impl G2CaptureIntent {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path)
            .map_err(|error| format!("read G2 recorder capture intent failed: {error}"))?;
        let parsed_value = snapshot::parse_json_strict(&bytes)
            .map_err(|error| format!("strict parse G2 recorder capture intent failed: {error}"))?;
        let parsed: G2CaptureIntentFile = serde_json::from_value(parsed_value)
            .map_err(|error| format!("decode G2 recorder capture intent failed: {error}"))?;
        if parsed.schema != G2_CAPTURE_INTENT_SCHEMA
            || parsed.observation_id.trim().is_empty()
            || parsed.observation_id.len() > 512
            || parsed.localisation_language.trim().is_empty()
            || parsed.localisation_language.len() > 512
            || parsed.pov_country_or_global != "global"
            || parsed.expected_selected_hour < 0
        {
            return Err("G2 recorder capture intent has invalid identity fields".into());
        }
        require_sha256(&parsed.settings_file_sha256, "G2 settings file")?;
        require_sha256(&parsed.camera_payload_sha256, "G2 camera payload")?;
        let capture_context_path =
            canonical_file(&parsed.capture_context_path, "G2 capture context")?;
        let first = fs::read(&capture_context_path)
            .map_err(|error| format!("read G2 capture context failed: {error}"))?;
        let second = fs::read(&capture_context_path)
            .map_err(|error| format!("re-read G2 capture context failed: {error}"))?;
        if first != second {
            return Err("G2 capture context changed during admission".into());
        }
        let context = snapshot::parse_json_strict(&first)
            .map_err(|error| format!("strict parse G2 capture context failed: {error}"))?;
        if context.get("schema").and_then(serde_json::Value::as_str)
            != Some("hoi4-rendering-law-capture-context-v1")
            || context.get("packets") != Some(&serde_json::json!(["G2"]))
            || context.pointer("/inventory/active_hoi4_pids") != Some(&serde_json::json!([]))
        {
            return Err("G2 capture context is not an isolated pre-launch G2 snapshot".into());
        }
        let capture_context_sha256 = to_hex(&Sha256::digest(&first));
        Ok(Self {
            observation_id: parsed.observation_id,
            settings_file_sha256: parsed.settings_file_sha256,
            localisation_language: parsed.localisation_language,
            camera_payload_sha256: parsed.camera_payload_sha256,
            pov_country_or_global: parsed.pov_country_or_global,
            expected_selected_hour: parsed.expected_selected_hour,
            capture_context_path,
            capture_context_sha256,
        })
    }

    pub fn validate_unchanged(&self) -> Result<(), String> {
        require_file_sha256(
            &self.capture_context_path,
            &self.capture_context_sha256,
            "G2 capture context",
        )
    }

    pub fn validate_context_authority(
        &self,
        executable_sha256: &str,
        settings_file_sha256: &str,
        localisation_language: &str,
    ) -> Result<(), String> {
        self.validate_unchanged()?;
        let bytes = fs::read(&self.capture_context_path)
            .map_err(|error| format!("read G2 capture context authority failed: {error}"))?;
        let context = snapshot::parse_json_strict(&bytes).map_err(|error| {
            format!("strict parse G2 capture context authority failed: {error}")
        })?;
        let context_language = context
            .pointer("/inventory/settings/language")
            .and_then(serde_json::Value::as_str)
            .map(|value| {
                value
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .or_else(|| {
                        value
                            .strip_prefix('\'')
                            .and_then(|value| value.strip_suffix('\''))
                    })
                    .unwrap_or(value)
            });
        if context
            .pointer("/inventory/paths/executable/sha256")
            .and_then(serde_json::Value::as_str)
            != Some(executable_sha256)
            || context
                .pointer("/inventory/paths/settings/sha256")
                .and_then(serde_json::Value::as_str)
                != Some(settings_file_sha256)
            || context_language != Some(localisation_language)
        {
            return Err(
                "G2 capture context differs from launch executable/settings/language".into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct G2PostProcessBoundaryWitness {
    producer_rva: u32,
    immediate_context_pointer: String,
    registered_backbuffer_view_pointer: String,
    registered_backbuffer_resource_pointer: String,
    registration_sequence: u64,
    recreate_generation: u64,
    exact_swapchain_backbuffer_registration: bool,
    post_process_original_returned: bool,
    readback_inserted_before_post_process_hook_return: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct G2GlobalPovWitness {
    observer_ui_root_rva: u32,
    observer_predicate_function_rva: u32,
    game_state_root_rva: usize,
    selected_controller_handle_offset: u32,
    selected_controller_handle: u32,
    handle_remap_table_offset: u32,
    controller_table_offset: u32,
    remapped_controller_index: u32,
    controller_pointer: String,
    controller_observer_count_offset: u32,
    controller_observer_count: i32,
    global_observer_mode: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct G2CloseZoomWitness {
    camera_owner_vtable_rva: u32,
    camera_root_pointer: String,
    current_eye_f32_bits: [u32; 3],
    current_look_at_f32_bits: [u32; 3],
    target_eye_f32_bits: [u32; 3],
    target_look_at_f32_bits: [u32; 3],
    required_height_f32_bits: u32,
    exact_retained_close_zoom_band: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct G2EngineWitness {
    schema: String,
    status: String,
    observation_id: String,
    render_boundary: String,
    pixel_format: String,
    authority_exe_sha256: String,
    launch_epoch_sha256: String,
    capture_receipt_sha256: String,
    effective_vfs_manifest_sha256: String,
    content_manifest_digest_sha256: String,
    settings_file_sha256: String,
    localisation_language: String,
    camera_payload_sha256: String,
    pov_country_or_global: String,
    selected_hour: i64,
    selected_present_sequence: u32,
    viewport_width: u32,
    viewport_height: u32,
    source_dxgi_format: u32,
    source_sample_count: u32,
    engine_bmp_file: String,
    engine_bmp_sha256: String,
    engine_bgra_sha256: String,
    pixel_byte_len: usize,
    capture_context_sha256: String,
    close_zoom_authentication: G2CloseZoomWitness,
    global_pov_authentication: G2GlobalPovWitness,
    post_process_boundary_authentication: G2PostProcessBoundaryWitness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct G2CaptureArtifactSeal {
    pub observation_id: String,
    pub capture_context_sha256: String,
    pub witness_sha256: String,
    pub witness_bytes: u64,
    pub bmp_sha256: String,
    pub bmp_bytes: u64,
    pub capture_content_manifest_digest_sha256: String,
}

fn valid_g2_pointer(value: &str) -> bool {
    let Some(digits) = value.strip_prefix("0x") else {
        return false;
    };
    !digits.is_empty()
        && digits.bytes().any(|byte| byte != b'0')
        && digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
}

/// Fail-closed recorder-side admission for the collector-authored raw G2
/// observation. A requested witness is part of the production capture result;
/// absence, additional files, identity drift, or malformed BMP bytes aborts
/// finalization rather than being treated as an optional sidecar.
pub fn validate_g2_capture_artifacts(
    intent: &G2CaptureIntent,
    output_directory: &Path,
    manifest: &EarlyRenderLaunchManifest,
    receipt: &EarlyRenderCaptureReceipt,
    content_manifest_digest_sha256: &str,
) -> Result<(), String> {
    intent.validate_unchanged()?;
    manifest.validate()?;
    receipt.validate()?;
    require_sha256(content_manifest_digest_sha256, "G2 content manifest digest")?;
    intent.validate_context_authority(
        &manifest.executable_sha256,
        &intent.settings_file_sha256,
        &intent.localisation_language,
    )?;
    let mut names = fs::read_dir(output_directory)
        .map_err(|error| format!("read G2 output directory failed: {error}"))?
        .map(|entry| {
            let entry = entry.map_err(|error| format!("read G2 output entry failed: {error}"))?;
            if !entry
                .file_type()
                .map_err(|error| format!("read G2 output entry type failed: {error}"))?
                .is_file()
            {
                return Err("G2 output contains a non-file entry".to_string());
            }
            entry
                .file_name()
                .into_string()
                .map_err(|_| "G2 output has a non-Unicode file name".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    if names
        != vec![
            G2_ENGINE_BMP_FILE.to_string(),
            G2_ENGINE_WITNESS_FILE.to_string(),
        ]
    {
        return Err("G2 output does not contain exactly the BMP and raw witness".into());
    }
    let bmp = fs::read(output_directory.join(G2_ENGINE_BMP_FILE))
        .map_err(|error| format!("read G2 BMP failed: {error}"))?;
    let witness_bytes = fs::read(output_directory.join(G2_ENGINE_WITNESS_FILE))
        .map_err(|error| format!("read G2 witness failed: {error}"))?;
    let witness_value = snapshot::parse_json_strict(&witness_bytes)
        .map_err(|error| format!("strict parse G2 witness failed: {error}"))?;
    let witness: G2EngineWitness = serde_json::from_value(witness_value)
        .map_err(|error| format!("decode strict G2 witness failed: {error}"))?;
    let receipt_sha256 = receipt.sha256()?;
    let launch_epoch_sha256 = portable_render_stream::compute_render_launch_epoch_sha256(
        receipt.process_id,
        receipt.process_start_nonce,
        receipt.hook_ready_qpc,
        receipt.first_game_d3d_creation_qpc,
        receipt.first_creation_sequence,
        receipt.hook_epoch_nonce,
        &receipt.hook_set_sha256,
        &receipt_sha256,
    );
    if witness.schema != G2_ENGINE_WITNESS_SCHEMA
        || witness.status != G2_ENGINE_STATUS
        || witness.observation_id != intent.observation_id
        || witness.render_boundary != G2_RENDER_BOUNDARY
        || witness.pixel_format != "BGRA8_UNORM"
        || witness.authority_exe_sha256 != manifest.executable_sha256
        || witness.launch_epoch_sha256 != launch_epoch_sha256
        || witness.capture_receipt_sha256 != receipt_sha256
        || witness.effective_vfs_manifest_sha256 != manifest.effective_vfs_manifest_sha256
        || witness.content_manifest_digest_sha256 != content_manifest_digest_sha256
        || witness.settings_file_sha256 != intent.settings_file_sha256
        || witness.localisation_language != intent.localisation_language
        || witness.camera_payload_sha256 != intent.camera_payload_sha256
        || witness.pov_country_or_global != intent.pov_country_or_global
        || witness.selected_hour != intent.expected_selected_hour
        || witness.selected_present_sequence == 0
        || witness.viewport_width == 0
        || witness.viewport_height == 0
        || !matches!(witness.source_dxgi_format, 28 | 29 | 87 | 91)
        || witness.source_sample_count != 1
        || witness.engine_bmp_file != G2_ENGINE_BMP_FILE
        || witness.capture_context_sha256 != intent.capture_context_sha256
    {
        return Err("G2 witness differs from recorder-owned launch/capture authority".into());
    }
    let boundary = &witness.post_process_boundary_authentication;
    if boundary.producer_rva != production_render_submit_live_lane::POST_PROCESS_RENDER_ROOT_RVA
        || !valid_g2_pointer(&boundary.immediate_context_pointer)
        || !valid_g2_pointer(&boundary.registered_backbuffer_view_pointer)
        || !valid_g2_pointer(&boundary.registered_backbuffer_resource_pointer)
        || boundary.registration_sequence == 0
        || boundary.recreate_generation == 0
        || !boundary.exact_swapchain_backbuffer_registration
        || !boundary.post_process_original_returned
        || !boundary.readback_inserted_before_post_process_hook_return
    {
        return Err("G2 witness has invalid exact post-process boundary authentication".into());
    }
    let pov = &witness.global_pov_authentication;
    if pov.observer_ui_root_rva != 0x01DD_9390
        || pov.observer_predicate_function_rva != 0x006F_0430
        || pov.game_state_root_rva != 0x0331_6E90
        || !matches!(pov.selected_controller_handle_offset, 0x520 | 0x524)
        || pov.handle_remap_table_offset != 0x340
        || pov.controller_table_offset != 0x310
        || !valid_g2_pointer(&pov.controller_pointer)
        || pov.controller_observer_count_offset != 0x41c
        || pov.controller_observer_count >= 1
        || !pov.global_observer_mode
    {
        return Err("G2 witness has no exact engine-attested global observer POV".into());
    }
    let camera = &witness.close_zoom_authentication;
    if camera.camera_owner_vtable_rva != 0x0295_2510
        || !valid_g2_pointer(&camera.camera_root_pointer)
        || camera.required_height_f32_bits != 0x4248_29C8
        || camera.current_eye_f32_bits[1] != camera.required_height_f32_bits
        || camera.target_eye_f32_bits[1] != camera.required_height_f32_bits
        || [
            camera.current_eye_f32_bits,
            camera.current_look_at_f32_bits,
            camera.target_eye_f32_bits,
            camera.target_look_at_f32_bits,
        ]
        .into_iter()
        .flatten()
        .any(|bits| !f32::from_bits(bits).is_finite())
        || !camera.exact_retained_close_zoom_band
    {
        return Err("G2 witness has no exact engine-attested retained close-zoom camera".into());
    }
    if bmp.len() < 54
        || &bmp[..2] != b"BM"
        || u32::from_le_bytes(bmp[2..6].try_into().unwrap()) as usize != bmp.len()
        || &bmp[6..10] != [0u8; 4].as_slice()
        || u32::from_le_bytes(bmp[10..14].try_into().unwrap()) != 54
        || u32::from_le_bytes(bmp[14..18].try_into().unwrap()) != 40
        || u16::from_le_bytes(bmp[26..28].try_into().unwrap()) != 1
        || u16::from_le_bytes(bmp[28..30].try_into().unwrap()) != 32
        || u32::from_le_bytes(bmp[30..34].try_into().unwrap()) != 0
        || &bmp[38..54] != [0u8; 16].as_slice()
    {
        return Err("G2 BMP header is not exact top-down BGRA32".into());
    }
    let width = i32::from_le_bytes(bmp[18..22].try_into().unwrap());
    let signed_height = i32::from_le_bytes(bmp[22..26].try_into().unwrap());
    if width <= 0
        || signed_height >= 0
        || width as u32 != witness.viewport_width
        || signed_height.unsigned_abs() != witness.viewport_height
    {
        return Err("G2 BMP dimensions differ from the witness".into());
    }
    let pixels = &bmp[54..];
    let expected_pixels = usize::try_from(witness.viewport_width)
        .ok()
        .and_then(|width| width.checked_mul(witness.viewport_height as usize))
        .and_then(|count| count.checked_mul(4))
        .ok_or("G2 witness pixel dimensions overflowed")?;
    if pixels.len() != expected_pixels
        || witness.pixel_byte_len != expected_pixels
        || u32::try_from(expected_pixels).ok()
            != Some(u32::from_le_bytes(bmp[34..38].try_into().unwrap()))
        || witness.engine_bmp_sha256 != to_hex(&Sha256::digest(&bmp))
        || witness.engine_bgra_sha256 != to_hex(&Sha256::digest(pixels))
    {
        return Err("G2 BMP bytes/digests differ from the witness".into());
    }
    Ok(())
}

/// Seal the exact already-admitted files for replay packaging. The second read
/// closes the validation-to-pack mutation window; the packer later requires
/// these hashes and lengths again before it copies either payload.
pub fn seal_g2_capture_artifacts(
    intent: &G2CaptureIntent,
    output_directory: &Path,
    manifest: &EarlyRenderLaunchManifest,
    receipt: &EarlyRenderCaptureReceipt,
    content_manifest_digest_sha256: &str,
) -> Result<G2CaptureArtifactSeal, String> {
    validate_g2_capture_artifacts(
        intent,
        output_directory,
        manifest,
        receipt,
        content_manifest_digest_sha256,
    )?;
    let witness = fs::read(output_directory.join(G2_ENGINE_WITNESS_FILE))
        .map_err(|error| format!("re-read sealed G2 witness failed: {error}"))?;
    let bmp = fs::read(output_directory.join(G2_ENGINE_BMP_FILE))
        .map_err(|error| format!("re-read sealed G2 BMP failed: {error}"))?;
    // Re-run the admission after the reads so a concurrent replacement cannot
    // make the returned seal describe bytes other than the admitted pair.
    validate_g2_capture_artifacts(
        intent,
        output_directory,
        manifest,
        receipt,
        content_manifest_digest_sha256,
    )?;
    let witness_after = fs::read(output_directory.join(G2_ENGINE_WITNESS_FILE))
        .map_err(|error| format!("final read of sealed G2 witness failed: {error}"))?;
    let bmp_after = fs::read(output_directory.join(G2_ENGINE_BMP_FILE))
        .map_err(|error| format!("final read of sealed G2 BMP failed: {error}"))?;
    if witness != witness_after || bmp != bmp_after {
        return Err("G2 artifacts changed while their replay seal was created".into());
    }
    Ok(G2CaptureArtifactSeal {
        observation_id: intent.observation_id.clone(),
        capture_context_sha256: intent.capture_context_sha256.clone(),
        witness_sha256: to_hex(&Sha256::digest(&witness)),
        witness_bytes: u64::try_from(witness.len()).map_err(|_| "G2 witness is too large")?,
        bmp_sha256: to_hex(&Sha256::digest(&bmp)),
        bmp_bytes: u64::try_from(bmp.len()).map_err(|_| "G2 BMP is too large")?,
        capture_content_manifest_digest_sha256: content_manifest_digest_sha256.to_owned(),
    })
}

pub fn staged_hook_count_for_group_mask(mask: u64) -> Result<u32, String> {
    compact_contract::early_render_control::staged_hook_count_for_group_mask(mask).ok_or_else(
        || "diagnostic staged-hook mask has invalid group or packed COM-selector bits".into(),
    )
}

pub fn validate_diagnostic_com_hook_mask(mask: u64) -> Result<(), String> {
    if mask == 0
        || mask & !EARLY_DIAGNOSTIC_COM_HOOK_MASK_FULL != 0
        || mask == EARLY_DIAGNOSTIC_COM_HOOK_MASK_FULL
    {
        return Err(
            "diagnostic COM-hook mask must be a nonzero strict 19-bit subset of 0x7ffff".into(),
        );
    }
    Ok(())
}

pub fn pack_diagnostic_hook_masks(group_mask: u64, com_hook_mask: u64) -> Result<u64, String> {
    if group_mask == EARLY_DIAGNOSTIC_STAGE_MASK_FULL
        || group_mask & 0x01 == 0
        || group_mask & !EARLY_DIAGNOSTIC_STAGE_MASK_FULL != 0
        || compact_contract::early_render_control::staged_hook_count_for_group_mask(group_mask)
            .is_none()
    {
        return Err(
            "diagnostic staged-hook mask must be a strict 0x3ff subset containing COM (0x01)"
                .into(),
        );
    }
    validate_diagnostic_com_hook_mask(com_hook_mask)?;
    let packed = group_mask | (com_hook_mask << EARLY_DIAGNOSTIC_COM_HOOK_MASK_SHIFT);
    staged_hook_count_for_group_mask(packed)?;
    Ok(packed)
}

pub fn diagnostic_stage_group_mask(mask: u64) -> u64 {
    mask & EARLY_DIAGNOSTIC_STAGE_MASK_FULL
}

pub fn diagnostic_com_hook_mask(mask: u64) -> u64 {
    (mask >> EARLY_DIAGNOSTIC_COM_HOOK_MASK_SHIFT) & EARLY_DIAGNOSTIC_COM_HOOK_MASK_FULL
}

fn valid_windows_environment_key(key: &str) -> bool {
    match key.strip_prefix('=') {
        Some(hidden_drive_key) => matches!(
            hidden_drive_key.as_bytes(),
            [drive, b':'] if drive.is_ascii_alphabetic()
        ),
        None => !key.is_empty() && !key.contains('='),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestedProfileSelectionReceipt {
    pub schema: String,
    pub dlc_load_sha256: String,
    pub disabled_dlc_descriptor_ids: Vec<String>,
    pub requested_enabled_dlc_ids: Vec<String>,
}

impl From<crate::runtime_dlc_roster::RequestedProfileSelection>
    for RequestedProfileSelectionReceipt
{
    fn from(value: crate::runtime_dlc_roster::RequestedProfileSelection) -> Self {
        Self {
            schema: value.schema.into(),
            dlc_load_sha256: value.dlc_load_sha256,
            disabled_dlc_descriptor_ids: value.disabled_dlc_descriptor_ids,
            requested_enabled_dlc_ids: value.requested_enabled_dlc_ids.into_iter().collect(),
        }
    }
}

impl RequestedProfileSelectionReceipt {
    fn validate(&self) -> Result<(), String> {
        if self.schema != crate::runtime_dlc_roster::REQUESTED_PROFILE_SELECTION_SCHEMA
            || self
                .disabled_dlc_descriptor_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self
                .requested_enabled_dlc_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self
                .requested_enabled_dlc_ids
                .iter()
                .any(|id| id.trim().is_empty())
            || self.disabled_dlc_descriptor_ids.iter().any(|id| {
                let parts = id.split('/').collect::<Vec<_>>();
                parts.len() != 3
                    || parts[0] != "dlc"
                    || parts[1].is_empty()
                    || parts[2].is_empty()
                    || !parts[2].ends_with(".dlc")
            })
        {
            return Err("requested profile DLC selection is malformed or noncanonical".into());
        }
        require_sha256(&self.dlc_load_sha256, "requested dlc_load.json")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EarlyLaunchLayerKind {
    Base,
    Mod,
    Dlc,
    IntegratedDlc,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlyLaunchLayer {
    pub kind: EarlyLaunchLayerKind,
    pub layer_id: String,
    pub descriptor_sha256: Option<String>,
    /// Exact archive/tree payload digest. Directory layers use a deterministic
    /// tree digest supplied by finalized VFS admission; descriptor hashes are
    /// never substituted for payload identity.
    pub content_sha256: String,
    pub storage_kind: LayerStorageKind,
}

/// Path-redacted launch identity persisted with the recording.  Local paths
/// are deliberately absent so relocating an identical installation/profile
/// does not change this identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlyRenderLaunchManifest {
    pub schema: String,
    pub executable_sha256: String,
    pub collector_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub ordered_content_layers: Vec<EarlyLaunchLayer>,
    pub requested_profile_selection: RequestedProfileSelectionReceipt,
    pub effective_runtime_dlc_candidate_ids: Vec<String>,
    pub localisation_language: String,
    pub required_settings: BTreeMap<String, String>,
    /// User-provided engine arguments only.  The recording-local absolute
    /// `-userdir` binding is injected by this module and is never serialized.
    pub semantic_arguments: Vec<String>,
}

impl EarlyRenderLaunchManifest {
    pub fn from_bound_launch(
        launch: &PortableLaunchBinding,
        ordered_layers: &[RecordedContentLayer],
        directory_layer_content_sha256: &BTreeMap<String, String>,
        requested_profile_selection: RequestedProfileSelectionReceipt,
        collector_sha256: String,
        effective_vfs_manifest_sha256: String,
        semantic_arguments: Vec<String>,
    ) -> Result<Self, String> {
        if ordered_layers.is_empty() {
            return Err("early launch has no finalized effective-VFS layer order".into());
        }
        let mods = launch
            .ordered_mods
            .iter()
            .map(|row| (row.layer_id.as_str(), row))
            .collect::<BTreeMap<_, _>>();
        let mut ordered_content_layers = Vec::with_capacity(ordered_layers.len());
        for (expected_ordinal, layer) in ordered_layers.iter().enumerate() {
            if layer.ordinal as usize != expected_ordinal {
                return Err("finalized effective-VFS layer ordinals are not contiguous".into());
            }
            let (kind, descriptor_sha256, bound_content_sha256) = match layer.layer_kind {
                ContentLayerKind::Base => (EarlyLaunchLayerKind::Base, None, None),
                ContentLayerKind::IntegratedDlc => {
                    let bound = launch
                        .integrated_dlc_layers
                        .values()
                        .find(|row| row.layer_id == layer.layer_id)
                        .ok_or("finalized integrated-DLC layer is absent from launch binding")?;
                    (
                        EarlyLaunchLayerKind::IntegratedDlc,
                        Some(bound.descriptor_sha256.clone()),
                        bound.source_container_sha256.clone(),
                    )
                }
                ContentLayerKind::Dlc => {
                    let bound = launch
                        .enabled_dlc_layers
                        .values()
                        .find(|row| row.layer_id == layer.layer_id)
                        .ok_or("finalized DLC layer is absent from selected launch DLC")?;
                    (
                        EarlyLaunchLayerKind::Dlc,
                        Some(bound.descriptor_sha256.clone()),
                        bound.source_container_sha256.clone(),
                    )
                }
                ContentLayerKind::Mod => {
                    let bound = mods
                        .get(layer.layer_id.as_str())
                        .ok_or("finalized mod layer is absent from ordered launch mods")?;
                    (
                        EarlyLaunchLayerKind::Mod,
                        Some(bound.descriptor_sha256.clone()),
                        Some(bound.content_sha256.clone()),
                    )
                }
            };
            if layer.descriptor_sha256 != descriptor_sha256
                || layer.source_container_sha256
                    != bound_content_sha256
                        .clone()
                        .filter(|_| layer.storage_kind == LayerStorageKind::ZipArchive)
            {
                return Err(
                    "finalized layer descriptor/container differs from launch binding".into(),
                );
            }
            let content_sha256 = match (layer.storage_kind.clone(), bound_content_sha256) {
                (LayerStorageKind::ZipArchive, Some(value)) => value,
                (LayerStorageKind::ZipArchive, None) => {
                    return Err("archive launch layer has no source-container digest".into())
                }
                (LayerStorageKind::Directory, Some(value))
                    if matches!(kind, EarlyLaunchLayerKind::Mod) =>
                {
                    value
                }
                (LayerStorageKind::Directory, _) => directory_layer_content_sha256
                    .get(&layer.layer_id)
                    .cloned()
                    .ok_or("directory launch layer has no deterministic content-tree digest")?,
            };
            ordered_content_layers.push(EarlyLaunchLayer {
                kind,
                layer_id: layer.layer_id.clone(),
                descriptor_sha256,
                content_sha256,
                storage_kind: layer.storage_kind.clone(),
            });
        }
        let observed_mods = ordered_content_layers
            .iter()
            .filter(|row| matches!(row.kind, EarlyLaunchLayerKind::Mod))
            .map(|row| row.layer_id.as_str())
            .collect::<Vec<_>>();
        let expected_mods = launch
            .ordered_mods
            .iter()
            .map(|row| row.layer_id.as_str())
            .collect::<Vec<_>>();
        if observed_mods != expected_mods {
            return Err("finalized mod layer order differs from launcher order".into());
        }
        let observed_integrated = ordered_content_layers
            .iter()
            .filter(|row| matches!(row.kind, EarlyLaunchLayerKind::IntegratedDlc))
            .map(|row| row.layer_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let expected_integrated = launch
            .integrated_dlc_layers
            .values()
            .map(|row| row.layer_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if observed_integrated != expected_integrated {
            return Err("finalized integrated-DLC layers differ from launch binding".into());
        }
        let observed_dlc = ordered_content_layers
            .iter()
            .filter(|row| matches!(row.kind, EarlyLaunchLayerKind::Dlc))
            .map(|row| row.layer_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let expected_dlc = launch
            .enabled_dlc_layers
            .values()
            .map(|row| row.layer_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if observed_dlc != expected_dlc {
            return Err("finalized selected-DLC layers differ from launch binding".into());
        }
        let manifest = Self {
            schema: EARLY_LAUNCH_MANIFEST_SCHEMA.into(),
            executable_sha256: launch.executable_sha256.clone(),
            collector_sha256,
            effective_vfs_manifest_sha256,
            ordered_content_layers,
            requested_profile_selection,
            effective_runtime_dlc_candidate_ids: launch.enabled_dlc_ids.iter().cloned().collect(),
            localisation_language: launch.localisation_language.clone(),
            required_settings: launch.required_settings.clone(),
            semantic_arguments,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != EARLY_LAUNCH_MANIFEST_SCHEMA
            || self.localisation_language.trim().is_empty()
            || self.required_settings.is_empty()
        {
            return Err("early render launch manifest header is incomplete".into());
        }
        for (label, value) in [
            ("executable", &self.executable_sha256),
            ("collector", &self.collector_sha256),
            ("effective VFS", &self.effective_vfs_manifest_sha256),
        ] {
            require_sha256(value, label)?;
        }
        if self.semantic_arguments.iter().any(|argument| {
            argument.trim().is_empty()
                || argument.contains('\0')
                || argument.to_ascii_lowercase().starts_with("-userdir")
        }) {
            return Err(
                "semantic launch arguments are blank, contain NUL, or override -userdir".into(),
            );
        }
        validate_launcher_effective_arguments(&self.semantic_arguments)?;
        self.requested_profile_selection.validate()?;
        let mut prior_phase = 0u8;
        let mut layer_ids = std::collections::BTreeSet::new();
        let mut base_count = 0usize;
        for layer in &self.ordered_content_layers {
            if layer.layer_id.trim().is_empty() || !layer_ids.insert(layer.layer_id.as_str()) {
                return Err("early launch content layer has a blank id".into());
            }
            if let Some(descriptor) = &layer.descriptor_sha256 {
                require_sha256(descriptor, "content-layer descriptor")?;
            } else if !matches!(layer.kind, EarlyLaunchLayerKind::Base) {
                return Err("non-base launch layer has no descriptor digest".into());
            }
            require_sha256(&layer.content_sha256, "content-layer payload")?;
            let phase = match layer.kind {
                EarlyLaunchLayerKind::Base => {
                    base_count += 1;
                    0
                }
                EarlyLaunchLayerKind::IntegratedDlc => 1,
                EarlyLaunchLayerKind::Dlc => 2,
                EarlyLaunchLayerKind::Mod => 3,
            };
            if phase < prior_phase {
                return Err("launch layers are not in integrated-DLC, DLC, mod VFS order".into());
            }
            prior_phase = phase;
        }
        if base_count != 1
            || !matches!(
                self.ordered_content_layers.first().map(|row| &row.kind),
                Some(EarlyLaunchLayerKind::Base)
            )
        {
            return Err("launch layer order does not begin with exactly one base layer".into());
        }
        if self
            .effective_runtime_dlc_candidate_ids
            .iter()
            .any(|id| id.trim().is_empty())
            || self
                .effective_runtime_dlc_candidate_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(
                "effective-runtime DLC candidate ids are blank, repeated, or noncanonical".into(),
            );
        }
        if self
            .requested_profile_selection
            .requested_enabled_dlc_ids
            .iter()
            .any(|id| {
                self.effective_runtime_dlc_candidate_ids
                    .binary_search(id)
                    .is_err()
            })
        {
            return Err(
                "requested DLC selection is outside the effective-runtime candidate set".into(),
            );
        }
        Ok(())
    }

    /// Rebuild the complete portable launch identity from the independently
    /// admitted launch/VFS facts and require byte-for-byte equality. This is
    /// the pre-injection gate used by the normal recorder CLI; a user-supplied
    /// manifest is never accepted merely because its fields are well formed.
    pub fn validate_against_finalized_vfs(
        &self,
        launch: &PortableLaunchBinding,
        ordered_layers: &[RecordedContentLayer],
        directory_layer_content_sha256: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        let expected = Self::from_bound_launch(
            launch,
            ordered_layers,
            directory_layer_content_sha256,
            self.requested_profile_selection.clone(),
            self.collector_sha256.clone(),
            self.effective_vfs_manifest_sha256.clone(),
            self.semantic_arguments.clone(),
        )?;
        if &expected != self {
            return Err(
                "early launch manifest differs from independently finalized launch/VFS facts"
                    .into(),
            );
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| format!("serialize early launch manifest failed: {error}"))?;
        Ok(to_hex(&Sha256::digest(&bytes)))
    }
}

#[derive(Clone, Debug)]
pub struct EarlyRenderLaunchSpec {
    pub executable_path: PathBuf,
    pub user_dir: PathBuf,
    pub collector_path: PathBuf,
    pub manifest: EarlyRenderLaunchManifest,
    pub hook_ready_timeout_ms: u32,
    /// Preserve the historical minimized capture presentation by default.
    /// Headless control is PID-bound and does not depend on this choice.
    pub minimize_after_first_d3d: bool,
    /// Diagnostic-only subset of the authenticated staged hook set. `None`
    /// preserves the immutable production hook set and child environment.
    pub diagnostic_staged_hook_mask: Option<u64>,
    /// Optional recorder-owned G2 observation. The existing collector remains
    /// the sole D3D hook owner; no auxiliary capture DLL is injected.
    pub g2_capture_intent: Option<G2CaptureIntent>,
    pub g2_output_directory: Option<PathBuf>,
    pub g2_content_manifest_digest_sha256: Option<String>,
}

impl EarlyRenderLaunchSpec {
    pub fn validate_local_files(&self) -> Result<(), String> {
        self.manifest.validate()?;
        if self.hook_ready_timeout_ms == 0
            || self.hook_ready_timeout_ms > crate::MAX_EARLY_HOOK_TIMEOUT_MS
        {
            return Err(format!(
                "early hook-ready timeout is outside 1..={} ms",
                crate::MAX_EARLY_HOOK_TIMEOUT_MS
            ));
        }
        if let Some(mask) = self.diagnostic_staged_hook_mask {
            staged_hook_count_for_group_mask(mask)?;
        }
        match (
            &self.g2_capture_intent,
            &self.g2_output_directory,
            &self.g2_content_manifest_digest_sha256,
        ) {
            (None, None, None) => {}
            (Some(intent), Some(output), Some(content_digest)) => {
                if self.diagnostic_staged_hook_mask.is_some() {
                    return Err("G2 capture is unavailable in diagnostic no-COMMIT launch".into());
                }
                intent.validate_unchanged()?;
                require_sha256(content_digest, "G2 content manifest digest")?;
                if !output.is_absolute() {
                    return Err("G2 output directory is not absolute".into());
                }
                if output.exists() {
                    return Err("G2 output directory already exists; refusing overwrite".into());
                }
            }
            _ => return Err("G2 capture intent/output/content binding is incomplete".into()),
        }
        let executable = canonical_file(&self.executable_path, "game executable")?;
        let collector = canonical_file(&self.collector_path, "collector DLL")?;
        let user_dir = fs::canonicalize(&self.user_dir)
            .map_err(|error| format!("canonicalize selected user directory failed: {error}"))?;
        if !user_dir.is_dir() || !user_dir.join("dlc_load.json").is_file() {
            return Err("selected user directory is not a directory with dlc_load.json".into());
        }
        require_file_sha256(
            &executable,
            &self.manifest.executable_sha256,
            "game executable",
        )?;
        require_file_sha256(&collector, &self.manifest.collector_sha256, "collector DLL")?;
        Ok(())
    }
}

/// Local-only inputs needed to create the game process suspended. The
/// portable launch manifest is intentionally constructed later, in memory,
/// from the suspended process's exact modset and admitted effective VFS.
#[derive(Clone, Debug)]
pub struct EarlyRenderProcessCreateSpec {
    pub executable_path: PathBuf,
    pub user_dir: PathBuf,
    pub semantic_arguments: Vec<String>,
    pub diagnostic_staged_hook_mask: Option<u64>,
}

impl EarlyRenderProcessCreateSpec {
    pub fn validate_local_files(&self) -> Result<(), String> {
        canonical_file(&self.executable_path, "game executable")?;
        let user_dir = fs::canonicalize(&self.user_dir)
            .map_err(|error| format!("canonicalize selected user directory failed: {error}"))?;
        if !user_dir.is_dir() || !user_dir.join("dlc_load.json").is_file() {
            return Err("selected user directory is not a directory with dlc_load.json".into());
        }
        if self.semantic_arguments.iter().any(|argument| {
            argument.trim().is_empty()
                || argument.contains('\0')
                || argument.to_ascii_lowercase().starts_with("-userdir")
        }) {
            return Err(
                "semantic launch arguments are blank, contain NUL, or override -userdir".into(),
            );
        }
        validate_launcher_effective_arguments(&self.semantic_arguments)?;
        if let Some(mask) = self.diagnostic_staged_hook_mask {
            staged_hook_count_for_group_mask(mask)?;
        }
        Ok(())
    }
}

fn validate_launcher_effective_arguments(arguments: &[String]) -> Result<(), String> {
    let continue_index = arguments
        .iter()
        .position(|argument| argument.eq_ignore_ascii_case("--continuelastsave"));
    let gdpr_index = arguments
        .iter()
        .position(|argument| argument.eq_ignore_ascii_case("--gdpr-compliant"));
    if let Some(continue_index) = continue_index {
        let gdpr_index = gdpr_index.ok_or(
            "launcher-effective --continuelastsave is missing the authoritative --gdpr-compliant argument",
        )?;
        if gdpr_index < continue_index {
            return Err(
                "launcher-effective arguments differ from authoritative continue-save order".into(),
            );
        }
    }
    if arguments
        .iter()
        .any(|argument| argument.eq_ignore_ascii_case("-gdpr-compliant"))
    {
        return Err(
            "launcher settings spelling is not launcher-effective argv; expected --gdpr-compliant"
                .into(),
        );
    }
    for (index, argument) in arguments.iter().enumerate() {
        if arguments[..index]
            .iter()
            .any(|prior| prior.eq_ignore_ascii_case(argument))
        {
            return Err("launcher-effective arguments contain a duplicate entry".into());
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlyHookCompleteness {
    pub all_shader_create: bool,
    pub input_layout_create: bool,
    pub blend_state_create: bool,
    pub depth_stencil_state_create: bool,
    pub rasterizer_state_create: bool,
    pub sampler_state_create: bool,
    pub buffer_create_update: bool,
    pub texture_create_update: bool,
    pub resource_view_create: bool,
    pub draw_state_readback: bool,
}

impl EarlyHookCompleteness {
    fn is_complete(&self) -> bool {
        self.all_shader_create
            && self.input_layout_create
            && self.blend_state_create
            && self.depth_stencil_state_create
            && self.rasterizer_state_create
            && self.sampler_state_create
            && self.buffer_create_update
            && self.texture_create_update
            && self.resource_view_create
            && self.draw_state_readback
    }
}

/// Collector-authored receipt returned while the primary game thread is still
/// suspended.  The launcher independently binds and seals it before resume.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlyHookReadyProof {
    pub schema: String,
    pub launch_manifest_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub effective_vfs_index_sha256: String,
    pub effective_vfs_index_row_count: u32,
    pub effective_vfs_index_chunk_count: u32,
    pub content_manifest_bytes_sha256: String,
    pub content_manifest_byte_count: u64,
    pub content_manifest_chunk_count: u32,
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub executable_sha256: String,
    pub collector_sha256: String,
    pub hook_epoch_nonce: u64,
    pub hook_set_sha256: String,
    pub hook_ready_sequence: u64,
    pub hook_ready_qpc: u64,
    pub game_d3d_creation_count_at_hook_ready: u64,
    pub preexisting_game_d3d_object_count: u64,
    pub completeness: EarlyHookCompleteness,
}

impl EarlyHookReadyProof {
    pub fn validate_for(
        &self,
        manifest: &EarlyRenderLaunchManifest,
        process_id: u32,
        process_start_nonce: u64,
    ) -> Result<(), String> {
        if self.schema != EARLY_HOOK_READY_SCHEMA
            || self.process_id != process_id
            || self.process_start_nonce != process_start_nonce
            || self.executable_sha256 != manifest.executable_sha256
            || self.collector_sha256 != manifest.collector_sha256
            || self.effective_vfs_manifest_sha256 != manifest.effective_vfs_manifest_sha256
            || self.effective_vfs_index_row_count == 0
            || self.effective_vfs_index_chunk_count == 0
            || self.effective_vfs_index_chunk_count > self.effective_vfs_index_row_count
            || self.content_manifest_byte_count == 0
            || self.content_manifest_chunk_count == 0
            || self.launch_manifest_sha256 != manifest.sha256()?
            || self.hook_epoch_nonce == 0
            || self.hook_ready_sequence == 0
            || self.hook_ready_qpc == 0
            || self.game_d3d_creation_count_at_hook_ready != 0
            || self.preexisting_game_d3d_object_count != 0
            || !self.completeness.is_complete()
        {
            return Err(
                "early hook-ready proof is late, incomplete, or authority-mismatched".into(),
            );
        }
        require_sha256(&self.hook_set_sha256, "hook set")?;
        require_sha256(&self.effective_vfs_index_sha256, "effective VFS index")?;
        require_sha256(
            &self.content_manifest_bytes_sha256,
            "early content manifest bytes",
        )?;
        Ok(())
    }
}

/// First game-owned D3D device creation after resume.  Collector-internal NULL
/// probe devices are never counted as game devices and cannot satisfy this
/// proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlyFirstGameD3dProof {
    pub schema: String,
    pub launch_manifest_sha256: String,
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub game_device_ordinal: u64,
    pub first_creation_sequence: u64,
    pub creation_qpc: u64,
    pub staged_hook_generation: u64,
    pub staged_hook_count: u32,
    pub diagnostic_stage_probe: bool,
    pub staged_hook_group_mask: u64,
    pub staged_hook_set_sha256: String,
    pub game_device_ptr: u64,
    pub immediate_context_ptr: u64,
    pub collector_null_probe_excluded: bool,
}

/// Fresh collector-side readiness sample requested only after the recorder has
/// observed the full diagnostic hold. The proof is bound to the authenticated
/// H4CT session, the exact FIRST identity, and both transcript digests. It is
/// non-promotable and can never substitute for a production COMMIT receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlyDiagnosticLivenessProof {
    pub schema: String,
    pub session_id: String,
    pub launch_manifest_sha256: String,
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub game_device_ordinal: u64,
    pub first_creation_sequence: u64,
    pub creation_qpc: u64,
    pub staged_hook_generation: u64,
    pub staged_hook_count: u32,
    pub staged_hook_group_mask: u64,
    pub staged_hook_set_sha256: String,
    pub game_device_ptr: u64,
    pub immediate_context_ptr: u64,
    pub required_hold_ms: u64,
    pub request_nonce: String,
    pub request_sha256: String,
    pub sampled_qpc: u64,
    pub qpc_frequency_hz: u64,
    pub observed_hold_ms: u64,
    pub response_sha256: String,
    pub end_disposition: u16,
}

impl EarlyDiagnosticLivenessProof {
    pub fn validate_for(
        &self,
        receipt: &EarlyRenderCaptureReceipt,
        required_hold_ms: u64,
    ) -> Result<(), String> {
        use compact_contract::early_render_control::{
            EARLY_DIAGNOSTIC_END_RECORDER_CANCEL, EARLY_DIAGNOSTIC_MIN_HOLD_MS,
        };
        if receipt.diagnostic_staged_hook_mask.is_none()
            || self.schema != EARLY_DIAGNOSTIC_LIVENESS_SCHEMA
            || self.launch_manifest_sha256 != receipt.launch_manifest_sha256
            || self.process_id != receipt.process_id
            || self.process_start_nonce != receipt.process_start_nonce
            || self.hook_epoch_nonce != receipt.hook_epoch_nonce
            || self.game_device_ordinal != 1
            || self.first_creation_sequence != receipt.first_creation_sequence
            || self.creation_qpc != receipt.first_game_d3d_creation_qpc
            || self.staged_hook_generation != receipt.staged_hook_generation
            || self.staged_hook_count != receipt.staged_hook_count
            || self.staged_hook_group_mask != receipt.diagnostic_staged_hook_mask.unwrap()
            || self.staged_hook_set_sha256 != receipt.staged_hook_set_sha256
            || self.game_device_ptr != receipt.game_device_ptr
            || self.immediate_context_ptr != receipt.immediate_context_ptr
            || required_hold_ms < EARLY_DIAGNOSTIC_MIN_HOLD_MS
            || self.required_hold_ms != required_hold_ms
            || self.sampled_qpc <= self.creation_qpc
            || self.qpc_frequency_hz == 0
            || self.observed_hold_ms < required_hold_ms
            || self.end_disposition != EARLY_DIAGNOSTIC_END_RECORDER_CANCEL
        {
            return Err(
                "diagnostic liveness response is not bound to the exact held FIRST identity".into(),
            );
        }
        if self.session_id.len() != 32
            || self.request_nonce.len() != 32
            || [&self.session_id, &self.request_nonce]
                .into_iter()
                .any(|value| {
                    value
                        .bytes()
                        .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
                })
        {
            return Err("diagnostic liveness session/nonce is not canonical hex".into());
        }
        for (label, value) in [
            ("diagnostic launch manifest", &self.launch_manifest_sha256),
            ("diagnostic staged hook set", &self.staged_hook_set_sha256),
            ("diagnostic request", &self.request_sha256),
            ("diagnostic response", &self.response_sha256),
        ] {
            require_sha256(value, label)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlyRuntimeDlcRosterProof {
    pub schema: String,
    pub launch_manifest_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub staged_hook_generation: u64,
    pub game_device_ptr: u64,
    pub manager_getter_rva: u64,
    pub active_dlc_enumerator_rva: u64,
    pub string_vector_teardown_rva: u64,
    pub sample_count: u32,
    pub double_stable: bool,
    pub roster_count: u32,
    pub raw_name_roster_sha256: String,
    pub raw_names: Vec<String>,
    pub effective_runtime_dlc_ids: Vec<String>,
    pub display_name_to_layer_id: BTreeMap<String, String>,
}

impl EarlyRuntimeDlcRosterProof {
    fn validate_for(
        &self,
        ready: &EarlyHookReadyProof,
        first: &EarlyFirstGameD3dProof,
    ) -> Result<(), String> {
        use compact_contract::early_render_runtime_dlc::{
            canonical_runtime_dlc_roster_sha256, A729_ACTIVE_DLC_ENUMERATOR_RVA,
            A729_DLC_MANAGER_GETTER_RVA, A729_STRING_VECTOR_TEARDOWN_RVA,
        };
        if self.schema != crate::runtime_dlc_roster::EFFECTIVE_RUNTIME_DLC_SCHEMA
            || self.launch_manifest_sha256 != ready.launch_manifest_sha256
            || self.effective_vfs_manifest_sha256 != ready.effective_vfs_manifest_sha256
            || self.process_id != ready.process_id
            || self.process_start_nonce != ready.process_start_nonce
            || self.hook_epoch_nonce != ready.hook_epoch_nonce
            || self.staged_hook_generation != first.staged_hook_generation
            || self.game_device_ptr != first.game_device_ptr
            || self.manager_getter_rva != A729_DLC_MANAGER_GETTER_RVA
            || self.active_dlc_enumerator_rva != A729_ACTIVE_DLC_ENUMERATOR_RVA
            || self.string_vector_teardown_rva != A729_STRING_VECTOR_TEARDOWN_RVA
            || self.sample_count != 2
            || !self.double_stable
            || self.roster_count as usize != self.raw_names.len()
            || self
                .effective_runtime_dlc_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self.effective_runtime_dlc_ids.len() != self.display_name_to_layer_id.len()
        {
            return Err(
                "runtime DLC roster is not bound to the post-device hook/process epoch".into(),
            );
        }
        require_sha256(&self.raw_name_roster_sha256, "runtime DLC roster")?;
        if self.raw_name_roster_sha256
            != to_hex(&canonical_runtime_dlc_roster_sha256(&self.raw_names))
        {
            return Err("runtime DLC roster canonical digest mismatch".into());
        }
        Ok(())
    }
}

impl EarlyFirstGameD3dProof {
    fn validate_for(
        &self,
        ready: &EarlyHookReadyProof,
        resume_qpc: u64,
        diagnostic_staged_hook_mask: Option<u64>,
    ) -> Result<(), String> {
        let expected_staged_hook_count = diagnostic_staged_hook_mask
            .map(staged_hook_count_for_group_mask)
            .transpose()?
            .unwrap_or(compact_contract::early_render_control::EARLY_STAGED_HOOK_COUNT);
        let expected_group_mask = diagnostic_staged_hook_mask.unwrap_or(
            compact_contract::early_render_control::EARLY_STAGED_HOOK_PRODUCTION_GROUP_MASK,
        );
        let expected_diagnostic = diagnostic_staged_hook_mask.is_some();
        if self.schema != EARLY_HOOK_READY_SCHEMA
            || self.launch_manifest_sha256 != ready.launch_manifest_sha256
            || self.process_id != ready.process_id
            || self.process_start_nonce != ready.process_start_nonce
            || self.hook_epoch_nonce != ready.hook_epoch_nonce
            || self.game_device_ordinal != 1
            || self.first_creation_sequence == 0
            || self.staged_hook_generation
                != compact_contract::early_render_control::EARLY_STAGED_HOOK_GENERATION
            || self.staged_hook_count != expected_staged_hook_count
            || self.diagnostic_stage_probe != expected_diagnostic
            || self.staged_hook_group_mask != expected_group_mask
            || self.game_device_ptr == 0
            || self.immediate_context_ptr == 0
            || !self.collector_null_probe_excluded
            || self.creation_qpc <= ready.hook_ready_qpc
            || self.creation_qpc < resume_qpc
        {
            return Err(
                "first game D3D creation is not bound after resume to the ready hook epoch".into(),
            );
        }
        require_sha256(&self.staged_hook_set_sha256, "staged hook set")?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlyRenderCaptureReceipt {
    pub schema: String,
    pub launch_manifest_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub effective_vfs_index_sha256: String,
    pub effective_vfs_index_row_count: u32,
    pub effective_vfs_index_chunk_count: u32,
    pub content_manifest_bytes_sha256: String,
    pub content_manifest_byte_count: u64,
    pub content_manifest_chunk_count: u32,
    pub process_id: u32,
    pub process_start_nonce: u64,
    pub executable_sha256: String,
    pub collector_sha256: String,
    pub created_suspended: bool,
    pub hook_epoch_nonce: u64,
    pub hook_set_sha256: String,
    pub hook_ready_sequence: u64,
    pub resume_sequence: u64,
    pub hook_ready_qpc: u64,
    pub resume_qpc: u64,
    pub first_creation_sequence: u64,
    pub first_game_d3d_creation_qpc: u64,
    pub staged_hook_generation: u64,
    pub staged_hook_count: u32,
    pub staged_hook_set_sha256: String,
    /// Present only on diagnostic staged-hook subset probes. Production v1
    /// receipt bytes remain unchanged because `None` is not serialized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_staged_hook_mask: Option<u64>,
    pub game_device_ptr: u64,
    pub immediate_context_ptr: u64,
    pub effective_runtime_dlc: EffectiveRuntimeDlcReceipt,
    pub game_d3d_creation_count_at_hook_ready: u64,
    pub preexisting_game_d3d_object_count: u64,
    pub completeness: EarlyHookCompleteness,
    pub transcript_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveRuntimeDlcReceipt {
    pub schema: String,
    pub source: String,
    pub manager_getter_rva: u64,
    pub active_dlc_enumerator_rva: u64,
    pub string_vector_teardown_rva: u64,
    pub sample_count: u32,
    pub double_stable: bool,
    pub active_display_names_engine_order: Vec<String>,
    pub enabled_dlc_ids: Vec<String>,
    pub display_name_to_layer_id: BTreeMap<String, String>,
    pub engine_order_sha256: String,
}

impl EffectiveRuntimeDlcReceipt {
    fn validate(&self) -> Result<(), String> {
        use compact_contract::early_render_runtime_dlc::{
            canonical_runtime_dlc_roster_sha256, A729_ACTIVE_DLC_ENUMERATOR_RVA,
            A729_DLC_MANAGER_GETTER_RVA, A729_STRING_VECTOR_TEARDOWN_RVA,
        };
        if self.schema != crate::runtime_dlc_roster::EFFECTIVE_RUNTIME_DLC_SCHEMA
            || self.source != "engine_active_dlc_manager_v1"
            || self.manager_getter_rva != A729_DLC_MANAGER_GETTER_RVA
            || self.active_dlc_enumerator_rva != A729_ACTIVE_DLC_ENUMERATOR_RVA
            || self.string_vector_teardown_rva != A729_STRING_VECTOR_TEARDOWN_RVA
            || self.sample_count != 2
            || !self.double_stable
            || self
                .enabled_dlc_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self.enabled_dlc_ids.len() != self.display_name_to_layer_id.len()
            || self.active_display_names_engine_order.len() != self.enabled_dlc_ids.len()
        {
            return Err("effective runtime DLC receipt is incomplete or noncanonical".into());
        }
        let enabled = self
            .enabled_dlc_ids
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        let mapped = self
            .display_name_to_layer_id
            .values()
            .collect::<std::collections::BTreeSet<_>>();
        if enabled != mapped
            || self
                .active_display_names_engine_order
                .iter()
                .any(|name| !self.display_name_to_layer_id.contains_key(name))
        {
            return Err("effective runtime DLC receipt name/layer sets disagree".into());
        }
        require_sha256(
            &self.engine_order_sha256,
            "effective runtime DLC engine order",
        )?;
        if self.engine_order_sha256
            != to_hex(&canonical_runtime_dlc_roster_sha256(
                &self.active_display_names_engine_order,
            ))
        {
            return Err("effective runtime DLC receipt digest mismatch".into());
        }
        Ok(())
    }
}

impl EarlyRenderCaptureReceipt {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != EARLY_CAPTURE_RECEIPT_SCHEMA
            || !self.created_suspended
            || self.process_id == 0
            || self.process_start_nonce == 0
            || self.hook_epoch_nonce == 0
            || self.effective_vfs_index_row_count == 0
            || self.effective_vfs_index_chunk_count == 0
            || self.effective_vfs_index_chunk_count > self.effective_vfs_index_row_count
            || self.content_manifest_byte_count == 0
            || self.content_manifest_chunk_count == 0
            || self.hook_ready_sequence == 0
            || self.resume_sequence <= self.hook_ready_sequence
            || self.hook_ready_qpc == 0
            || self.resume_qpc <= self.hook_ready_qpc
            || self.first_creation_sequence == 0
            || self.first_game_d3d_creation_qpc < self.resume_qpc
            || self.staged_hook_generation
                != compact_contract::early_render_control::EARLY_STAGED_HOOK_GENERATION
            || match self.diagnostic_staged_hook_mask {
                Some(mask) => staged_hook_count_for_group_mask(mask)
                    .map(|expected| self.staged_hook_count != expected)
                    .unwrap_or(true),
                None => {
                    self.staged_hook_count
                        != compact_contract::early_render_control::EARLY_STAGED_HOOK_COUNT
                }
            }
            || self.game_device_ptr == 0
            || self.immediate_context_ptr == 0
            || self.game_d3d_creation_count_at_hook_ready != 0
            || self.preexisting_game_d3d_object_count != 0
            || !self.completeness.is_complete()
        {
            return Err("early render capture receipt is late, incomplete, or malformed".into());
        }
        self.effective_runtime_dlc.validate()?;
        for (label, value) in [
            ("launch manifest", &self.launch_manifest_sha256),
            ("effective VFS", &self.effective_vfs_manifest_sha256),
            ("effective VFS index", &self.effective_vfs_index_sha256),
            (
                "early content manifest bytes",
                &self.content_manifest_bytes_sha256,
            ),
            ("executable", &self.executable_sha256),
            ("collector", &self.collector_sha256),
            ("hook set", &self.hook_set_sha256),
            ("staged hook set", &self.staged_hook_set_sha256),
            ("transcript", &self.transcript_sha256),
        ] {
            require_sha256(value, label)?;
        }
        if self.transcript_sha256 != self.expected_transcript_sha256()? {
            return Err("early render capture transcript digest mismatch".into());
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<String, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| format!("serialize early capture receipt failed: {error}"))?;
        Ok(to_hex(&Sha256::digest(&bytes)))
    }

    fn expected_transcript_sha256(&self) -> Result<String, String> {
        #[derive(Serialize)]
        struct Transcript<'a> {
            events: [&'static str; 8],
            launch_manifest_sha256: &'a str,
            effective_vfs_index_sha256: &'a str,
            effective_vfs_index_row_count: u32,
            effective_vfs_index_chunk_count: u32,
            content_manifest_bytes_sha256: &'a str,
            content_manifest_byte_count: u64,
            content_manifest_chunk_count: u32,
            process_id: u32,
            process_start_nonce: u64,
            hook_epoch_nonce: u64,
            hook_set_sha256: &'a str,
            hook_ready_sequence: u64,
            resume_sequence: u64,
            hook_ready_qpc: u64,
            resume_qpc: u64,
            first_creation_sequence: u64,
            first_game_d3d_creation_qpc: u64,
            staged_hook_generation: u64,
            staged_hook_count: u32,
            staged_hook_set_sha256: &'a str,
            game_device_ptr: u64,
            immediate_context_ptr: u64,
            effective_runtime_dlc: &'a EffectiveRuntimeDlcReceipt,
        }
        let transcript = Transcript {
            events: [
                "created_suspended",
                "injection_committed",
                "collector_epoch_bound",
                "hooks_ready",
                "primary_thread_resumed",
                "first_game_d3d_creation",
                "staged_hook_set_ready",
                "runtime_dlc_roster_validated",
            ],
            launch_manifest_sha256: &self.launch_manifest_sha256,
            effective_vfs_index_sha256: &self.effective_vfs_index_sha256,
            effective_vfs_index_row_count: self.effective_vfs_index_row_count,
            effective_vfs_index_chunk_count: self.effective_vfs_index_chunk_count,
            content_manifest_bytes_sha256: &self.content_manifest_bytes_sha256,
            content_manifest_byte_count: self.content_manifest_byte_count,
            content_manifest_chunk_count: self.content_manifest_chunk_count,
            process_id: self.process_id,
            process_start_nonce: self.process_start_nonce,
            hook_epoch_nonce: self.hook_epoch_nonce,
            hook_set_sha256: &self.hook_set_sha256,
            hook_ready_sequence: self.hook_ready_sequence,
            resume_sequence: self.resume_sequence,
            hook_ready_qpc: self.hook_ready_qpc,
            resume_qpc: self.resume_qpc,
            first_creation_sequence: self.first_creation_sequence,
            first_game_d3d_creation_qpc: self.first_game_d3d_creation_qpc,
            staged_hook_generation: self.staged_hook_generation,
            staged_hook_count: self.staged_hook_count,
            staged_hook_set_sha256: &self.staged_hook_set_sha256,
            game_device_ptr: self.game_device_ptr,
            immediate_context_ptr: self.immediate_context_ptr,
            effective_runtime_dlc: &self.effective_runtime_dlc,
        };
        let bytes = serde_json::to_vec(&transcript)
            .map_err(|error| format!("serialize early launch transcript failed: {error}"))?;
        Ok(to_hex(&Sha256::digest(&bytes)))
    }
}

pub trait EarlyHookReadyChannel {
    /// The listener must already exist before collector injection.  This call
    /// returns only after the collector has bound its process epoch and installed
    /// the complete creation/readback hook set while the primary thread remains
    /// suspended.
    fn await_ready(
        &mut self,
        process_id: u32,
        process_start_nonce: u64,
        timeout_ms: u32,
    ) -> Result<EarlyHookReadyProof, String>;

    /// Waits for the first game-owned D3D device creation after resume under
    /// the same hook epoch.  The collector's validation probe is excluded.
    fn await_first_game_d3d_creation(
        &mut self,
        process_id: u32,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
        timeout_ms: u32,
    ) -> Result<EarlyFirstGameD3dProof, String>;

    /// Receives the collector's twice-stable engine-manager DLC roster after
    /// FIRST. The host must join these raw names to authenticated descriptor
    /// and VFS identities before it can construct or send COMMIT.
    fn await_runtime_dlc_roster(
        &mut self,
        process_id: u32,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
        staged_hook_generation: u64,
        game_device_ptr: u64,
        timeout_ms: u32,
    ) -> Result<EarlyRuntimeDlcRosterProof, String>;

    /// After the recorder's own monotonic clock has held the diagnostic target
    /// for at least the contract minimum, request a fresh collector-side sample
    /// of the still-installed staged hook set. This path is diagnostic-only.
    fn request_diagnostic_liveness(
        &mut self,
        receipt: &EarlyRenderCaptureReceipt,
        required_hold_ms: u64,
    ) -> Result<EarlyDiagnosticLivenessProof, String>;

    /// Send the authenticated recorder-cancel terminal bound to the accepted
    /// response digest. This is not COMMIT and admits no capture stream.
    fn send_diagnostic_end(
        &mut self,
        receipt: &EarlyRenderCaptureReceipt,
        proof: &EarlyDiagnosticLivenessProof,
    ) -> Result<(), String>;

    /// Arms the sole existing collector D3D hook owner for one exact G2
    /// witness. This message is sent immediately before COMMIT and is bound to
    /// the same receipt digest; an auxiliary hook DLL is never involved.
    fn arm_g2_capture(
        &mut self,
        arm: &compact_contract::early_render_control::EarlyG2CaptureArmV1,
    ) -> Result<(), String>;

    /// Commits the final host-sealed receipt digest back to the collector.
    /// Buffered d77/d80 rows may be released only after this succeeds, and
    /// every row must carry this exact digest in its launch epoch.
    fn commit_capture_receipt(
        &mut self,
        process_id: u32,
        process_start_nonce: u64,
        hook_epoch_nonce: u64,
        receipt_sha256: &str,
    ) -> Result<(), String>;
}

fn require_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} SHA-256 is not canonical lowercase hex"));
    }
    Ok(())
}

fn decode_sha256(value: &str, label: &str) -> Result<[u8; 32], String> {
    require_sha256(value, label)?;
    let mut out = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        out[index] = digit(pair[0])
            .zip(digit(pair[1]))
            .map(|(high, low)| (high << 4) | low)
            .ok_or_else(|| format!("{label} SHA-256 is not canonical lowercase hex"))?;
    }
    Ok(out)
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, String> {
    let canonical =
        fs::canonicalize(path).map_err(|error| format!("canonicalize {label} failed: {error}"))?;
    if !canonical.is_file() {
        return Err(format!("{label} is not a file"));
    }
    Ok(canonical)
}

fn require_file_sha256(path: &Path, expected: &str, label: &str) -> Result<(), String> {
    require_sha256(expected, label)?;
    let first = fs::read(path).map_err(|error| format!("read {label} failed: {error}"))?;
    let second = fs::read(path).map_err(|error| format!("re-read {label} failed: {error}"))?;
    if first != second {
        return Err(format!("{label} changed while binding"));
    }
    let actual = to_hex(&Sha256::digest(&first));
    if actual != expected {
        return Err(format!("{label} SHA-256 mismatch"));
    }
    Ok(())
}

#[cfg(all(windows, feature = "evidence-client-driving"))]
mod windows {
    use super::*;
    use std::marker::PhantomData;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::AsRawHandle;
    use std::ptr;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    type Bool = i32;
    type Dword = u32;
    type Handle = *mut c_void;
    type Hmodule = *mut c_void;
    type Hwnd = *mut c_void;

    const CREATE_SUSPENDED: Dword = 0x0000_0004;
    const CREATE_UNICODE_ENVIRONMENT: Dword = 0x0000_0400;
    const EXTENDED_STARTUPINFO_PRESENT: Dword = 0x0008_0000;
    const SW_SHOWMINNOACTIVE: i32 = 7;
    const MEM_COMMIT: Dword = 0x0000_1000;
    const MEM_RESERVE: Dword = 0x0000_2000;
    const MEM_RELEASE: Dword = 0x0000_8000;
    const PAGE_READWRITE: Dword = 0x04;
    const WAIT_OBJECT_0: Dword = 0;
    const INFINITE: Dword = 0xffff_ffff;
    const TH32CS_SNAPMODULE: Dword = 0x0000_0008;
    const TH32CS_SNAPMODULE32: Dword = 0x0000_0010;
    const INVALID_HANDLE_VALUE: isize = -1;
    const ERROR_NO_MORE_FILES: Dword = 18;
    const ERROR_PARTIAL_COPY: Dword = 299;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: Dword = 0x0000_2000;
    const PROC_THREAD_ATTRIBUTE_JOB_LIST: usize = 0x0002_000d;
    const DUPLICATE_SAME_ACCESS: Dword = 0x0000_0002;

    #[repr(C)]
    struct StartupInfoW {
        cb: Dword,
        reserved: *mut u16,
        desktop: *mut u16,
        title: *mut u16,
        x: Dword,
        y: Dword,
        x_size: Dword,
        y_size: Dword,
        x_count_chars: Dword,
        y_count_chars: Dword,
        fill_attribute: Dword,
        flags: Dword,
        show_window: u16,
        reserved2_bytes: u16,
        reserved2: *mut u8,
        std_input: Handle,
        std_output: Handle,
        std_error: Handle,
    }

    #[repr(C)]
    struct StartupInfoExW {
        startup_info: StartupInfoW,
        attribute_list: *mut c_void,
    }

    #[repr(C)]
    struct ProcessInformation {
        process: Handle,
        thread: Handle,
        process_id: Dword,
        thread_id: Dword,
    }

    #[repr(C)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: Dword,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: Dword,
        affinity: usize,
        priority_class: Dword,
        scheduling_class: Dword,
    }

    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[repr(C)]
    struct ModuleEntry32W {
        size: Dword,
        module_id: Dword,
        process_id: Dword,
        global_usage: Dword,
        process_usage: Dword,
        base_address: *mut u8,
        base_size: Dword,
        module: Hmodule,
        module_name: [u16; 256],
        executable_path: [u16; 260],
    }

    #[repr(C)]
    struct FileTime {
        low: Dword,
        high: Dword,
    }

    #[repr(C)]
    struct ByHandleFileInformation {
        file_attributes: Dword,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: Dword,
        file_size_high: Dword,
        file_size_low: Dword,
        number_of_links: Dword,
        file_index_high: Dword,
        file_index_low: Dword,
    }

    #[link(name = "kernel32")]
    #[allow(clashing_extern_declarations)]
    extern "system" {
        fn CreateProcessW(
            application_name: *const u16,
            command_line: *mut u16,
            process_attributes: *mut c_void,
            thread_attributes: *mut c_void,
            inherit_handles: Bool,
            creation_flags: Dword,
            environment: *mut c_void,
            current_directory: *const u16,
            startup_info: *mut StartupInfoW,
            process_information: *mut ProcessInformation,
        ) -> Bool;
        fn InitializeProcThreadAttributeList(
            attribute_list: *mut c_void,
            attribute_count: Dword,
            flags: Dword,
            size: *mut usize,
        ) -> Bool;
        fn UpdateProcThreadAttribute(
            attribute_list: *mut c_void,
            flags: Dword,
            attribute: usize,
            value: *const c_void,
            size: usize,
            previous_value: *mut c_void,
            return_size: *mut usize,
        ) -> Bool;
        fn DeleteProcThreadAttributeList(attribute_list: *mut c_void);
        fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(
            job: Handle,
            information_class: i32,
            information: *const c_void,
            information_length: Dword,
        ) -> Bool;
        fn IsProcessInJob(process: Handle, job: Handle, result: *mut Bool) -> Bool;
        fn DuplicateHandle(
            source_process: Handle,
            source_handle: Handle,
            target_process: Handle,
            target_handle: *mut Handle,
            desired_access: Dword,
            inherit_handle: Bool,
            options: Dword,
        ) -> Bool;
        fn GetCurrentProcess() -> Handle;
        fn GetProcessId(process: Handle) -> Dword;
        fn VirtualAllocEx(
            process: Handle,
            address: *mut c_void,
            size: usize,
            allocation_type: Dword,
            protection: Dword,
        ) -> *mut c_void;
        fn VirtualFreeEx(
            process: Handle,
            address: *mut c_void,
            size: usize,
            free_type: Dword,
        ) -> Bool;
        fn WriteProcessMemory(
            process: Handle,
            address: *mut c_void,
            buffer: *const c_void,
            size: usize,
            written: *mut usize,
        ) -> Bool;
        fn CreateRemoteThread(
            process: Handle,
            attributes: *mut c_void,
            stack_size: usize,
            start: *const c_void,
            parameter: *mut c_void,
            flags: Dword,
            thread_id: *mut Dword,
        ) -> Handle;
        fn WaitForSingleObject(handle: Handle, milliseconds: Dword) -> Dword;
        fn GetExitCodeThread(thread: Handle, exit_code: *mut Dword) -> Bool;
        fn GetProcessTimes(
            process: Handle,
            creation: *mut u64,
            exit: *mut u64,
            kernel: *mut u64,
            user: *mut u64,
        ) -> Bool;
        fn QueryFullProcessImageNameW(
            process: Handle,
            flags: Dword,
            executable_name: *mut u16,
            size: *mut Dword,
        ) -> Bool;
        fn GetFileInformationByHandle(
            file: Handle,
            information: *mut ByHandleFileInformation,
        ) -> Bool;
        fn ResumeThread(thread: Handle) -> Dword;
        fn QueryPerformanceCounter(value: *mut i64) -> Bool;
        fn TerminateProcess(process: Handle, exit_code: Dword) -> Bool;
        fn CloseHandle(handle: Handle) -> Bool;
        fn GetModuleHandleW(name: *const u16) -> Hmodule;
        fn GetProcAddress(module: Hmodule, name: *const u8) -> *const c_void;
        fn CreateToolhelp32Snapshot(flags: Dword, process_id: Dword) -> Handle;
        fn Module32FirstW(snapshot: Handle, entry: *mut ModuleEntry32W) -> Bool;
        fn Module32NextW(snapshot: Handle, entry: *mut ModuleEntry32W) -> Bool;
        fn GetLastError() -> Dword;
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: extern "system" fn(Hwnd, isize) -> Bool, lparam: isize) -> Bool;
        fn GetWindowThreadProcessId(window: Hwnd, process_id: *mut Dword) -> Dword;
        fn ShowWindow(window: Hwnd, command: i32) -> Bool;
    }

    struct MinimizeOwnedWindow {
        process_id: Dword,
    }

    extern "system" fn minimize_owned_window(window: Hwnd, lparam: isize) -> Bool {
        let context = unsafe { &mut *(lparam as *mut MinimizeOwnedWindow) };
        let mut process_id = 0;
        unsafe {
            GetWindowThreadProcessId(window, &mut process_id);
        }
        if process_id == context.process_id {
            unsafe {
                ShowWindow(window, SW_SHOWMINNOACTIVE);
            }
        }
        1
    }

    /// Allows the game to create its renderer through the normal shown-window path,
    /// then keeps every recorder-owned game window minimized without activation.
    struct MinimizedPresentationGuard {
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl MinimizedPresentationGuard {
        fn start(process_id: Dword) -> Result<Self, String> {
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = Arc::clone(&stop);
            let worker = std::thread::Builder::new()
                .name(format!("hoi4-minimized-presentation-{process_id}"))
                .spawn(move || {
                    while !worker_stop.load(Ordering::Acquire) {
                        let mut context = MinimizeOwnedWindow { process_id };
                        unsafe {
                            EnumWindows(
                                minimize_owned_window,
                                (&mut context as *mut MinimizeOwnedWindow) as isize,
                            );
                        }
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                })
                .map_err(|error| format!("start minimized presentation guard failed: {error}"))?;
            Ok(Self {
                stop,
                worker: Some(worker),
            })
        }
    }

    impl Drop for MinimizedPresentationGuard {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    pub struct EarlyRenderProcess {
        process: Handle,
        primary_thread: Handle,
        kill_on_close_job: Handle,
        pub process_id: u32,
        pub receipt: EarlyRenderCaptureReceipt,
        diagnostic_no_commit: bool,
        termination_observed: bool,
        _presentation_guard: Option<MinimizedPresentationGuard>,
    }

    unsafe impl Send for EarlyRenderProcess {}

    impl Drop for EarlyRenderProcess {
        fn drop(&mut self) {
            if self.diagnostic_no_commit && !self.termination_observed {
                self.termination_observed = terminate_exact_owned_process(
                    self.process,
                    self.receipt.process_start_nonce,
                    0xE080_00D2,
                    10_000,
                )
                .is_ok();
            }
            unsafe {
                // Closing this child-only job is the final fail-safe. The job
                // contains only the exact suspended process created by this
                // transaction, so wrapper/host abort cannot orphan it or touch
                // any pre-existing the game identity.
                CloseHandle(self.kill_on_close_job);
                WaitForSingleObject(self.process, 10_000);
                CloseHandle(self.primary_thread);
                CloseHandle(self.process);
            }
        }
    }

    impl EarlyRenderProcess {
        pub fn wait(&self, timeout_ms: u32) -> Result<bool, String> {
            match unsafe { WaitForSingleObject(self.process, timeout_ms) } {
                WAIT_OBJECT_0 => Ok(true),
                258 => Ok(false),
                other => Err(format!("wait for early-launched game returned {other}")),
            }
        }

        /// Terminates only an owned diagnostic no-COMMIT target. Production
        /// launches cannot call this path accidentally.
        pub fn terminate_diagnostic(&mut self) -> Result<(), String> {
            if !self.diagnostic_no_commit {
                return Err("refusing to terminate a non-diagnostic early-render process".into());
            }
            terminate_exact_owned_process(
                self.process,
                self.receipt.process_start_nonce,
                0xE080_00D1,
                10_000,
            )?;
            self.termination_observed = true;
            Ok(())
        }
    }

    /// A game process whose primary thread is still held by the single
    /// `CREATE_SUSPENDED` count. The PID and creation nonce are available so
    /// the recorder can bind and expose H4CR/H4CT listeners before collector
    /// injection. Dropping this value before activation terminates the process;
    /// a suspended orphan can never escape a failed launch transaction.
    pub struct PendingEarlyRenderProcess {
        process: Option<Handle>,
        primary_thread: Option<Handle>,
        kill_on_close_job: Option<Handle>,
        process_id: u32,
        process_start_nonce: u64,
        creation_job_generation: u64,
        creation_job_count: u32,
        creation_job_authority_claimed: AtomicBool,
        executable: PathBuf,
        user_dir: PathBuf,
        semantic_arguments: Vec<String>,
    }

    unsafe impl Send for PendingEarlyRenderProcess {}

    /// Unforgeable authority for the one job placed in the process creation
    /// attribute list. Its private fields and borrow from the pending launch
    /// prevent ordinary attach paths from manufacturing a job-membership
    /// bypass. The capability is useful only while the exact suspended process
    /// and its creation job are both retained by `PendingEarlyRenderProcess`.
    pub struct RecorderOwnedCreationJob<'a> {
        process: Handle,
        job: Handle,
        process_id: u32,
        process_start_nonce: u64,
        creation_job_generation: u64,
        creation_job_count: u32,
        _pending: PhantomData<&'a PendingEarlyRenderProcess>,
    }

    /// A duplicated reference to the exact recorder-owned creation job. This
    /// is the injection lease for an early launch; dropping the pending launch
    /// or moving it into `EarlyRenderProcess` cannot create a lease gap.
    #[derive(Debug)]
    pub struct RecorderOwnedCreationJobLease {
        job: Handle,
        process_id: u32,
        process_start_nonce: u64,
        creation_job_generation: u64,
    }

    unsafe impl Send for RecorderOwnedCreationJobLease {}

    impl RecorderOwnedCreationJobLease {
        pub fn process_id(&self) -> u32 {
            self.process_id
        }

        pub fn process_start_nonce(&self) -> u64 {
            self.process_start_nonce
        }

        pub fn creation_job_generation(&self) -> u64 {
            self.creation_job_generation
        }
    }

    impl Drop for RecorderOwnedCreationJobLease {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.job) };
        }
    }

    impl RecorderOwnedCreationJob<'_> {
        /// Revalidate the private creation authority against the independently
        /// retained target handle, then duplicate only that exact job handle.
        /// A foreign/closed job, PID reuse, nonce drift, or a creation list that
        /// was not exactly one recorder-owned job fails closed.
        pub fn retain_exact_job(
            &self,
            retained_process: *mut c_void,
            expected_pid: u32,
            expected_creation_filetime: u64,
        ) -> Result<RecorderOwnedCreationJobLease, String> {
            if retained_process.is_null()
                || self.process.is_null()
                || self.job.is_null()
                || self.creation_job_generation == 0
                || self.creation_job_count != 1
            {
                return Err(
                    "recorder-owned creation job authority is incomplete or ambiguous".into(),
                );
            }
            if self.process_id != expected_pid
                || self.process_start_nonce != expected_creation_filetime
            {
                return Err("recorder-owned creation job PID/creation nonce mismatch".into());
            }
            let retained_process = retained_process as Handle;
            if unsafe { GetProcessId(self.process) } != expected_pid
                || unsafe { GetProcessId(retained_process) } != expected_pid
            {
                return Err("recorder-owned creation job retained process PID mismatch".into());
            }
            if process_creation_nonce(self.process)? != expected_creation_filetime
                || process_creation_nonce(retained_process)? != expected_creation_filetime
            {
                return Err("recorder-owned creation job retained process nonce mismatch".into());
            }
            require_specific_job_membership(self.process, self.job)?;
            require_specific_job_membership(retained_process, self.job)?;

            let current = unsafe { GetCurrentProcess() };
            let mut duplicate = ptr::null_mut();
            if unsafe {
                DuplicateHandle(
                    current,
                    self.job,
                    current,
                    &mut duplicate,
                    0,
                    0,
                    DUPLICATE_SAME_ACCESS,
                )
            } == 0
            {
                return Err(format!(
                    "duplicate recorder-owned creation job failed: {}",
                    last_error()
                ));
            }

            let revalidated = (|| {
                if unsafe { GetProcessId(retained_process) } != expected_pid
                    || process_creation_nonce(retained_process)? != expected_creation_filetime
                {
                    return Err(
                        "recorder-owned creation job process changed during lease retention".into(),
                    );
                }
                require_specific_job_membership(retained_process, duplicate)
            })();
            if let Err(error) = revalidated {
                unsafe { CloseHandle(duplicate) };
                return Err(error);
            }
            Ok(RecorderOwnedCreationJobLease {
                job: duplicate,
                process_id: expected_pid,
                process_start_nonce: expected_creation_filetime,
                creation_job_generation: self.creation_job_generation,
            })
        }
    }

    fn require_specific_job_membership(process: Handle, job: Handle) -> Result<(), String> {
        let mut member = 0;
        if unsafe { IsProcessInJob(process, job, &mut member) } == 0 {
            return Err(format!(
                "validate recorder-owned creation job membership failed: {}",
                last_error()
            ));
        }
        if member == 0 {
            return Err("target is not a member of the exact recorder-owned creation job".into());
        }
        Ok(())
    }

    impl Drop for PendingEarlyRenderProcess {
        fn drop(&mut self) {
            if let Some(process) = self.process.take() {
                let _ = terminate_exact_owned_process(
                    process,
                    self.process_start_nonce,
                    0xE080_0001,
                    5_000,
                );
                if let Some(job) = self.kill_on_close_job.take() {
                    unsafe { CloseHandle(job) };
                    unsafe { WaitForSingleObject(process, 5_000) };
                }
                if let Some(thread) = self.primary_thread.take() {
                    unsafe { CloseHandle(thread) };
                }
                unsafe { CloseHandle(process) };
            }
            if let Some(job) = self.kill_on_close_job.take() {
                unsafe { CloseHandle(job) };
            }
        }
    }

    impl PendingEarlyRenderProcess {
        pub fn process_id(&self) -> u32 {
            self.process_id
        }

        pub fn process_start_nonce(&self) -> u64 {
            self.process_start_nonce
        }

        pub fn recorder_owned_creation_job(&self) -> Result<RecorderOwnedCreationJob<'_>, String> {
            self.creation_job_authority_claimed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| {
                    "recorder-owned creation job authority was already claimed; concurrent attach is ambiguous"
                        .to_string()
                })?;
            let process = self
                .process
                .ok_or("pending early process lost process handle")?;
            let job = self
                .kill_on_close_job
                .ok_or("pending early process lost creation job handle")?;
            if self.creation_job_generation == 0 || self.creation_job_count != 1 {
                return Err("pending early process has ambiguous creation job provenance".into());
            }
            if unsafe { GetProcessId(process) } != self.process_id
                || process_creation_nonce(process)? != self.process_start_nonce
            {
                return Err("pending early process identity changed before job authority".into());
            }
            require_specific_job_membership(process, job)?;
            Ok(RecorderOwnedCreationJob {
                process,
                job,
                process_id: self.process_id,
                process_start_nonce: self.process_start_nonce,
                creation_job_generation: self.creation_job_generation,
                creation_job_count: self.creation_job_count,
                _pending: PhantomData,
            })
        }

        /// Resolve the launcher/VFS binding while the process is provably still
        /// suspended. This path uses the exact executable and `-userdir`
        /// selected by `create_suspended_early_render_process`; it does not
        /// pretend a game-held log exists before the first game instruction.
        pub fn resolve_modset(&self) -> crate::modset::ModsetReport {
            crate::modset::resolve_suspended_launch_modset(
                self.process_id,
                &self.executable,
                &self.user_dir,
            )
        }

        /// Before the suspended primary thread has executed the user-mode loader,
        /// Toolhelp module enumeration may deterministically return
        /// ERROR_PARTIAL_COPY. Bind the newly-created process through its retained
        /// process handle, exact image path, file identity, and double-read SHA
        /// instead; loaded-module/base validation is mandatory after resume and
        /// before H4CT COMMIT.
        pub fn validate_suspended_image_authority(
            &self,
            expected_sha256: &str,
        ) -> Result<(), String> {
            let process = self
                .process
                .ok_or("pending early process lost process handle")?;
            let observed = process_image_path(process)?;
            let observed = canonical_file(&observed, "suspended process image")?;
            if observed != self.executable {
                return Err(format!(
                    "retained suspended process image {} differs from selected {}",
                    observed.display(),
                    self.executable.display()
                ));
            }
            let selected_file = fs::File::open(&self.executable)
                .map_err(|error| format!("open selected executable failed: {error}"))?;
            let observed_file = fs::File::open(&observed)
                .map_err(|error| format!("open suspended process image failed: {error}"))?;
            if file_identity(&selected_file)? != file_identity(&observed_file)? {
                return Err("retained suspended process image file identity changed".into());
            }
            require_file_sha256(&observed, expected_sha256, "suspended process image")?;
            if process_creation_nonce(process)? != self.process_start_nonce {
                return Err("retained suspended process creation nonce changed".into());
            }
            Ok(())
        }

        /// Inject only after the caller has created its PID-bound listener.
        /// READY is verified while suspended; exactly one suspend count is then
        /// released; FIRST and COMMIT complete the authenticated transaction.
        pub fn activate_with(
            self,
            spec: &EarlyRenderLaunchSpec,
            ready: &mut impl EarlyHookReadyChannel,
        ) -> Result<EarlyRenderProcess, String> {
            self.activate_with_post_resume_validation(spec, ready, || Ok(()))
        }

        pub fn activate_with_post_resume_validation(
            mut self,
            spec: &EarlyRenderLaunchSpec,
            ready: &mut impl EarlyHookReadyChannel,
            post_resume_validation: impl FnOnce() -> Result<(), String>,
        ) -> Result<EarlyRenderProcess, String> {
            spec.validate_local_files()?;
            if canonical_file(&spec.executable_path, "game executable")? != self.executable
                || fs::canonicalize(&spec.user_dir).map_err(|error| {
                    format!("canonicalize selected user directory failed: {error}")
                })? != self.user_dir
                || spec.manifest.semantic_arguments != self.semantic_arguments
            {
                return Err("capture activation differs from suspended process creation".into());
            }
            let process = self
                .process
                .ok_or("pending early process lost process handle")?;
            let primary_thread = self
                .primary_thread
                .ok_or("pending early process lost primary-thread handle")?;
            let collector = canonical_file(&spec.collector_path, "collector DLL")?;
            let loader_authority = inject_load_library(process, &collector)?;
            let hook = ready.await_ready(
                self.process_id,
                self.process_start_nonce,
                spec.hook_ready_timeout_ms,
            )?;
            hook.validate_for(&spec.manifest, self.process_id, self.process_start_nonce)?;
            let resume_sequence = hook
                .hook_ready_sequence
                .checked_add(1)
                .ok_or("early launch resume sequence overflow")?;
            let prior_suspend_count = unsafe { ResumeThread(primary_thread) };
            if prior_suspend_count != 1 {
                return Err(format!(
                    "ResumeThread expected prior suspend count 1, observed {prior_suspend_count}"
                ));
            }
            let resume_qpc = query_performance_counter()?;
            let first_device = ready.await_first_game_d3d_creation(
                self.process_id,
                self.process_start_nonce,
                hook.hook_epoch_nonce,
                spec.hook_ready_timeout_ms,
            )?;
            first_device.validate_for(&hook, resume_qpc, spec.diagnostic_staged_hook_mask)?;
            let presentation_guard = spec
                .minimize_after_first_d3d
                .then(|| MinimizedPresentationGuard::start(self.process_id))
                .transpose()?;
            loader_authority.validate_post_resume(self.process_id)?;
            post_resume_validation()?;
            let runtime_dlc = ready.await_runtime_dlc_roster(
                self.process_id,
                self.process_start_nonce,
                hook.hook_epoch_nonce,
                first_device.staged_hook_generation,
                first_device.game_device_ptr,
                spec.hook_ready_timeout_ms,
            )?;
            runtime_dlc.validate_for(&hook, &first_device)?;
            let mut receipt = EarlyRenderCaptureReceipt {
                schema: EARLY_CAPTURE_RECEIPT_SCHEMA.into(),
                launch_manifest_sha256: spec.manifest.sha256()?,
                effective_vfs_manifest_sha256: spec.manifest.effective_vfs_manifest_sha256.clone(),
                effective_vfs_index_sha256: hook.effective_vfs_index_sha256,
                effective_vfs_index_row_count: hook.effective_vfs_index_row_count,
                effective_vfs_index_chunk_count: hook.effective_vfs_index_chunk_count,
                content_manifest_bytes_sha256: hook.content_manifest_bytes_sha256,
                content_manifest_byte_count: hook.content_manifest_byte_count,
                content_manifest_chunk_count: hook.content_manifest_chunk_count,
                process_id: self.process_id,
                process_start_nonce: self.process_start_nonce,
                executable_sha256: spec.manifest.executable_sha256.clone(),
                collector_sha256: spec.manifest.collector_sha256.clone(),
                created_suspended: true,
                hook_epoch_nonce: hook.hook_epoch_nonce,
                hook_set_sha256: hook.hook_set_sha256,
                hook_ready_sequence: hook.hook_ready_sequence,
                resume_sequence,
                hook_ready_qpc: hook.hook_ready_qpc,
                resume_qpc,
                first_creation_sequence: first_device.first_creation_sequence,
                first_game_d3d_creation_qpc: first_device.creation_qpc,
                staged_hook_generation: first_device.staged_hook_generation,
                staged_hook_count: first_device.staged_hook_count,
                staged_hook_set_sha256: first_device.staged_hook_set_sha256,
                diagnostic_staged_hook_mask: spec.diagnostic_staged_hook_mask,
                game_device_ptr: first_device.game_device_ptr,
                immediate_context_ptr: first_device.immediate_context_ptr,
                effective_runtime_dlc: EffectiveRuntimeDlcReceipt {
                    schema: crate::runtime_dlc_roster::EFFECTIVE_RUNTIME_DLC_SCHEMA.into(),
                    source: "engine_active_dlc_manager_v1".into(),
                    manager_getter_rva: runtime_dlc.manager_getter_rva,
                    active_dlc_enumerator_rva: runtime_dlc.active_dlc_enumerator_rva,
                    string_vector_teardown_rva: runtime_dlc.string_vector_teardown_rva,
                    sample_count: runtime_dlc.sample_count,
                    double_stable: runtime_dlc.double_stable,
                    active_display_names_engine_order: runtime_dlc.raw_names,
                    enabled_dlc_ids: runtime_dlc.effective_runtime_dlc_ids,
                    display_name_to_layer_id: runtime_dlc.display_name_to_layer_id,
                    engine_order_sha256: runtime_dlc.raw_name_roster_sha256,
                },
                game_d3d_creation_count_at_hook_ready: hook.game_d3d_creation_count_at_hook_ready,
                preexisting_game_d3d_object_count: hook.preexisting_game_d3d_object_count,
                completeness: hook.completeness,
                transcript_sha256: String::new(),
            };
            receipt.transcript_sha256 = receipt.expected_transcript_sha256()?;
            receipt.validate()?;
            if spec.diagnostic_staged_hook_mask.is_none() {
                let receipt_sha256 = receipt.sha256()?;
                if let (Some(intent), Some(output_directory), Some(content_digest)) = (
                    spec.g2_capture_intent.as_ref(),
                    spec.g2_output_directory.as_ref(),
                    spec.g2_content_manifest_digest_sha256.as_ref(),
                ) {
                    intent.validate_unchanged()?;
                    let launch_epoch_sha256 =
                        portable_render_stream::compute_render_launch_epoch_sha256(
                            receipt.process_id,
                            receipt.process_start_nonce,
                            receipt.hook_ready_qpc,
                            receipt.first_game_d3d_creation_qpc,
                            receipt.first_creation_sequence,
                            receipt.hook_epoch_nonce,
                            &receipt.hook_set_sha256,
                            &receipt_sha256,
                        );
                    let arm = compact_contract::early_render_control::EarlyG2CaptureArmV1 {
                        process_id: receipt.process_id,
                        process_start_nonce: receipt.process_start_nonce,
                        hook_epoch_nonce: receipt.hook_epoch_nonce,
                        launch_manifest_sha256: decode_sha256(
                            &receipt.launch_manifest_sha256,
                            "G2 launch manifest",
                        )?,
                        launch_epoch_sha256: decode_sha256(
                            &launch_epoch_sha256,
                            "G2 launch epoch",
                        )?,
                        capture_receipt_sha256: decode_sha256(
                            &receipt_sha256,
                            "G2 capture receipt",
                        )?,
                        authority_exe_sha256: decode_sha256(
                            &receipt.executable_sha256,
                            "G2 authority executable",
                        )?,
                        effective_vfs_manifest_sha256: decode_sha256(
                            &receipt.effective_vfs_manifest_sha256,
                            "G2 effective VFS manifest",
                        )?,
                        content_manifest_digest_sha256: decode_sha256(
                            content_digest,
                            "G2 content manifest digest",
                        )?,
                        settings_file_sha256: decode_sha256(
                            &intent.settings_file_sha256,
                            "G2 settings file",
                        )?,
                        camera_payload_sha256: decode_sha256(
                            &intent.camera_payload_sha256,
                            "G2 camera payload",
                        )?,
                        capture_context_sha256: decode_sha256(
                            &intent.capture_context_sha256,
                            "G2 capture context",
                        )?,
                        expected_selected_hour: intent.expected_selected_hour,
                        observation_id: intent.observation_id.clone(),
                        localisation_language: intent.localisation_language.clone(),
                        pov_country_or_global: intent.pov_country_or_global.clone(),
                        capture_context_path: intent
                            .capture_context_path
                            .to_str()
                            .ok_or("G2 capture context path is not Unicode")?
                            .to_owned(),
                        output_directory: output_directory
                            .to_str()
                            .ok_or("G2 output directory is not Unicode")?
                            .to_owned(),
                    };
                    ready.arm_g2_capture(&arm)?;
                }
                ready.commit_capture_receipt(
                    self.process_id,
                    self.process_start_nonce,
                    hook.hook_epoch_nonce,
                    &receipt_sha256,
                )?;
            }

            let kill_on_close_job = self
                .kill_on_close_job
                .take()
                .ok_or("pending early process lost kill-on-close job")?;
            // Production transfers after COMMIT; diagnostic transfers after
            // FIRST + runtime-roster validation while deliberately withholding
            // COMMIT. On every earlier failure the pending value remains armed.
            self.process = None;
            self.primary_thread = None;
            Ok(EarlyRenderProcess {
                process,
                primary_thread,
                process_id: self.process_id,
                receipt,
                diagnostic_no_commit: spec.diagnostic_staged_hook_mask.is_some(),
                kill_on_close_job,
                termination_observed: false,
                _presentation_guard: presentation_guard,
            })
        }
    }

    pub fn create_suspended_early_render_process(
        spec: &EarlyRenderLaunchSpec,
    ) -> Result<PendingEarlyRenderProcess, String> {
        spec.validate_local_files()?;
        create_suspended_game_process(&EarlyRenderProcessCreateSpec {
            executable_path: spec.executable_path.clone(),
            user_dir: spec.user_dir.clone(),
            semantic_arguments: spec.manifest.semantic_arguments.clone(),
            diagnostic_staged_hook_mask: spec.diagnostic_staged_hook_mask,
        })
    }

    fn configure_normal_startup(startup: &mut StartupInfoExW) {
        startup.startup_info.flags = 0;
        startup.startup_info.show_window = 0;
        startup.startup_info.x = 0;
        startup.startup_info.y = 0;
    }

    pub fn create_suspended_game_process(
        spec: &EarlyRenderProcessCreateSpec,
    ) -> Result<PendingEarlyRenderProcess, String> {
        spec.validate_local_files()?;
        let executable = canonical_file(&spec.executable_path, "game executable")?;
        let user_dir = fs::canonicalize(&spec.user_dir)
            .map_err(|error| format!("canonicalize selected user directory failed: {error}"))?;
        let working_directory = executable
            .parent()
            .ok_or("game executable has no parent directory")?;
        let command = build_launcher_effective_command_line(
            &executable,
            &user_dir,
            &spec.semantic_arguments,
        )?;
        let mut command_wide = wide_nul(&command);
        let executable_wide = wide_nul(executable.as_os_str());
        let working_wide = wide_nul(working_directory.as_os_str());
        let mut diagnostic_environment = spec
            .diagnostic_staged_hook_mask
            .map(build_diagnostic_environment_block)
            .transpose()?;
        let kill_on_close_job = create_kill_on_close_job()?;
        let job_attribute_list = match create_job_attribute_list(kill_on_close_job) {
            Ok(value) => value,
            Err(error) => {
                unsafe { CloseHandle(kill_on_close_job) };
                return Err(error);
            }
        };
        let mut startup: StartupInfoExW = unsafe { zeroed() };
        startup.startup_info.cb = size_of::<StartupInfoExW>() as u32;
        // Production capture is recorder-controlled and must not steal the
        // Use the game's normal shown-window startup so its renderer initializes.
        // After the authenticated first D3D creation, a PID-bound guard keeps
        // every game window minimized without activation.
        configure_normal_startup(&mut startup);
        startup.attribute_list = job_attribute_list.as_ptr();
        let mut process: ProcessInformation = unsafe { zeroed() };
        let create_result = unsafe {
            CreateProcessW(
                executable_wide.as_ptr(),
                command_wide.as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                CREATE_SUSPENDED
                    | EXTENDED_STARTUPINFO_PRESENT
                    | if diagnostic_environment.is_some() {
                        CREATE_UNICODE_ENVIRONMENT
                    } else {
                        0
                    },
                diagnostic_environment
                    .as_mut()
                    .map_or(ptr::null_mut(), |environment| {
                        environment.as_mut_ptr().cast()
                    }),
                working_wide.as_ptr(),
                (&mut startup as *mut StartupInfoExW).cast(),
                &mut process,
            )
        };
        let create_error = (create_result == 0).then(last_error);
        // Drop deletes the initialized attribute list before releasing either
        // its aligned storage or the retained job-handle array.
        drop(job_attribute_list);
        if create_result == 0 {
            unsafe { CloseHandle(kill_on_close_job) };
            return Err(format!(
                "CreateProcessW(CREATE_SUSPENDED) failed: {}",
                create_error.unwrap()
            ));
        }
        let process_start_nonce = match process_creation_nonce(process.process) {
            Ok(value) => value,
            Err(error) => {
                unsafe {
                    TerminateProcess(process.process, 0xE080_0001);
                    WaitForSingleObject(process.process, 5_000);
                    CloseHandle(process.thread);
                    CloseHandle(process.process);
                    CloseHandle(kill_on_close_job);
                }
                return Err(error);
            }
        };
        let creation_job_generation = match query_performance_counter() {
            Ok(value) if value != 0 => value,
            Ok(_) => {
                unsafe {
                    TerminateProcess(process.process, 0xE080_0001);
                    WaitForSingleObject(process.process, 5_000);
                    CloseHandle(process.thread);
                    CloseHandle(process.process);
                    CloseHandle(kill_on_close_job);
                }
                return Err("creation job generation nonce was zero".into());
            }
            Err(error) => {
                unsafe {
                    TerminateProcess(process.process, 0xE080_0001);
                    WaitForSingleObject(process.process, 5_000);
                    CloseHandle(process.thread);
                    CloseHandle(process.process);
                    CloseHandle(kill_on_close_job);
                }
                return Err(error);
            }
        };
        Ok(PendingEarlyRenderProcess {
            process: Some(process.process),
            primary_thread: Some(process.thread),
            kill_on_close_job: Some(kill_on_close_job),
            process_id: process.process_id,
            process_start_nonce,
            creation_job_generation,
            creation_job_count: 1,
            creation_job_authority_claimed: AtomicBool::new(false),
            executable,
            user_dir,
            semantic_arguments: spec.semantic_arguments.clone(),
        })
    }

    fn create_kill_on_close_job() -> Result<Handle, String> {
        let job = unsafe { CreateJobObjectW(ptr::null_mut(), ptr::null()) };
        if job.is_null() {
            return Err(format!("CreateJobObjectW failed: {}", last_error()));
        }
        let mut information: JobObjectExtendedLimitInformation = unsafe { zeroed() };
        information.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                (&information as *const JobObjectExtendedLimitInformation).cast(),
                size_of::<JobObjectExtendedLimitInformation>() as Dword,
            )
        } == 0
        {
            let error = last_error();
            unsafe { CloseHandle(job) };
            return Err(format!("configure kill-on-close job failed: {error}"));
        }
        Ok(job)
    }

    struct JobAttributeList {
        storage: Vec<usize>,
        job_list_storage: Box<[Handle; 1]>,
        list: *mut c_void,
    }

    impl JobAttributeList {
        fn as_ptr(&self) -> *mut c_void {
            self.list
        }
    }

    impl Drop for JobAttributeList {
        fn drop(&mut self) {
            unsafe { DeleteProcThreadAttributeList(self.list) };
            // Both backing allocations remain fields until after this Drop
            // implementation returns. Read them so their lifetime invariant is
            // explicit to reviewers and future refactors.
            let _ = (self.storage.len(), self.job_list_storage.len());
        }
    }

    fn create_job_attribute_list(job: Handle) -> Result<JobAttributeList, String> {
        let mut required_bytes = 0usize;
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut required_bytes) };
        if required_bytes == 0 {
            return Err(format!(
                "size process job attribute list failed: {}",
                last_error()
            ));
        }
        let words = required_bytes
            .checked_add(size_of::<usize>() - 1)
            .ok_or("process job attribute list size overflow")?
            / size_of::<usize>();
        let mut storage = vec![0usize; words];
        let list = storage.as_mut_ptr().cast::<c_void>();
        if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut required_bytes) } == 0 {
            return Err(format!(
                "initialize process job attribute list failed: {}",
                last_error()
            ));
        }
        // PROC_THREAD_ATTRIBUTE_JOB_LIST retains this pointer until
        // CreateProcessW consumes the attribute list; it is not an inline copy.
        // Own the backing array across this function boundary so release
        // optimization cannot invalidate a pointer to the local parameter.
        let job_list_storage = Box::new([job]);
        let result = JobAttributeList {
            storage,
            job_list_storage,
            list,
        };
        if unsafe {
            UpdateProcThreadAttribute(
                result.list,
                0,
                PROC_THREAD_ATTRIBUTE_JOB_LIST,
                result.job_list_storage.as_ptr().cast(),
                size_of::<Handle>(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        } == 0
        {
            let error = last_error();
            return Err(format!(
                "bind kill-on-close job to process creation failed: {error}"
            ));
        }
        Ok(result)
    }

    fn terminate_exact_owned_process(
        process: Handle,
        expected_start_nonce: u64,
        exit_code: Dword,
        wait_ms: Dword,
    ) -> Result<(), String> {
        let observed_start_nonce = process_creation_nonce(process)?;
        if observed_start_nonce != expected_start_nonce {
            return Err(format!(
                "owned process start nonce changed: expected {expected_start_nonce}, observed {observed_start_nonce}"
            ));
        }
        match unsafe { WaitForSingleObject(process, 0) } {
            WAIT_OBJECT_0 => return Ok(()),
            258 => {}
            other => return Err(format!("owned process liveness wait returned {other}")),
        }
        if unsafe { TerminateProcess(process, exit_code) } == 0 {
            return Err(format!(
                "terminate exact owned process failed: {}",
                last_error()
            ));
        }
        match unsafe { WaitForSingleObject(process, wait_ms) } {
            WAIT_OBJECT_0 => Ok(()),
            other => Err(format!(
                "exact owned process did not terminate within {wait_ms} ms (wait={other})"
            )),
        }
    }

    fn build_diagnostic_environment_block(mask: u64) -> Result<Vec<u16>, String> {
        staged_hook_count_for_group_mask(mask)?;
        let diagnostic_key = "BATTLE_PLAN_XN15L_DIAGNOSTIC_STAGE_MASK";
        let mut entries = std::env::vars_os()
            .filter(|(key, _)| !key.to_string_lossy().eq_ignore_ascii_case(diagnostic_key))
            .collect::<Vec<_>>();
        entries.push((diagnostic_key.into(), format!("0x{mask:02x}").into()));
        entries.sort_by(|left, right| {
            left.0
                .to_string_lossy()
                .to_lowercase()
                .cmp(&right.0.to_string_lossy().to_lowercase())
        });
        let mut block = Vec::new();
        for (key, value) in entries {
            let key = key.to_string_lossy();
            let value = value.to_string_lossy();
            if key.contains('\0') || value.contains('\0') || !valid_windows_environment_key(&key) {
                return Err("parent environment contains an invalid key/value".into());
            }
            block.extend(format!("{key}={value}").encode_utf16());
            block.push(0);
        }
        block.push(0);
        Ok(block)
    }

    pub fn launch_suspended_early_render_capture(
        spec: &EarlyRenderLaunchSpec,
        ready: &mut impl EarlyHookReadyChannel,
    ) -> Result<EarlyRenderProcess, String> {
        create_suspended_early_render_process(spec)?.activate_with(spec, ready)
    }

    struct SuspendedLoaderAuthority {
        module_name: String,
        export_rva: usize,
        injected_start_address: usize,
    }

    impl SuspendedLoaderAuthority {
        fn validate_post_resume(&self, process_id: u32) -> Result<(), String> {
            let remote_base = target_module_base(process_id, &self.module_name)?;
            let expected = remote_base
                .checked_add(self.export_rva)
                .ok_or("post-resume LoadLibraryW address overflow")?;
            if expected != self.injected_start_address {
                return Err(format!(
                    "post-resume loader base/RVA validation differs from injected start: 0x{expected:x} != 0x{:x}",
                    self.injected_start_address
                ));
            }
            Ok(())
        }
    }

    fn inject_load_library(
        process: Handle,
        collector: &Path,
    ) -> Result<SuspendedLoaderAuthority, String> {
        let collector_wide = wide_nul(collector.as_os_str());
        let bytes = unsafe {
            std::slice::from_raw_parts(
                collector_wide.as_ptr().cast::<u8>(),
                collector_wide.len() * size_of::<u16>(),
            )
        };
        let remote = unsafe {
            VirtualAllocEx(
                process,
                ptr::null_mut(),
                bytes.len(),
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if remote.is_null() {
            return Err(format!(
                "VirtualAllocEx for collector path failed: {}",
                last_error()
            ));
        }
        let result = (|| {
            let mut written = 0usize;
            if unsafe {
                WriteProcessMemory(
                    process,
                    remote,
                    bytes.as_ptr().cast(),
                    bytes.len(),
                    &mut written,
                )
            } == 0
                || written != bytes.len()
            {
                return Err(format!(
                    "WriteProcessMemory for collector path failed: {}",
                    last_error()
                ));
            }
            let kernel_name = wide_nul(OsStr::new("kernel32.dll"));
            let kernel = unsafe { GetModuleHandleW(kernel_name.as_ptr()) };
            let load_library = unsafe { GetProcAddress(kernel, b"LoadLibraryW\0".as_ptr()) };
            if kernel.is_null() || load_library.is_null() {
                return Err("resolve local kernel32!LoadLibraryW failed".into());
            }
            let local_module_name = "kernel32.dll".to_string();
            let local_module_base = kernel as usize;
            let export_rva = (load_library as usize)
                .checked_sub(local_module_base)
                .ok_or("LoadLibraryW address precedes its backing module base")?;
            // System DLL section mappings use one boot-scoped address across
            // same-architecture processes. The suspended target's loader list
            // is not yet enumerable; use the exact local address now, then
            // prove target module base + export RVA equality after resume and
            // before H4CT COMMIT/frame admission.
            let remote_load_library = load_library as usize;
            let remote_thread = unsafe {
                CreateRemoteThread(
                    process,
                    ptr::null_mut(),
                    0,
                    remote_load_library as *const c_void,
                    remote,
                    0,
                    ptr::null_mut(),
                )
            };
            if remote_thread.is_null() {
                return Err(format!(
                    "CreateRemoteThread(LoadLibraryW) failed: {}",
                    last_error()
                ));
            }
            let wait = unsafe { WaitForSingleObject(remote_thread, INFINITE) };
            let mut exit_code = 0u32;
            let exit_ok = unsafe { GetExitCodeThread(remote_thread, &mut exit_code) } != 0;
            unsafe { CloseHandle(remote_thread) };
            if wait != WAIT_OBJECT_0 || !exit_ok || exit_code == 0 {
                return Err("remote LoadLibraryW did not return a collector module".into());
            }
            Ok(SuspendedLoaderAuthority {
                module_name: local_module_name,
                export_rva,
                injected_start_address: remote_load_library,
            })
        })();
        unsafe { VirtualFreeEx(process, remote, 0, MEM_RELEASE) };
        result
    }

    fn target_module_base(process_id: u32, wanted: &str) -> Result<usize, String> {
        target_modules(process_id)?
            .into_iter()
            .find(|(name, _, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, base, _)| base)
            .ok_or_else(|| {
                format!("target process {process_id} has no {wanted} module before resume")
            })
    }

    fn target_modules(process_id: u32) -> Result<Vec<(String, usize, usize)>, String> {
        let mut snapshot = ptr::null_mut();
        let mut snapshot_error = 0;
        for attempt in 0..8 {
            snapshot = unsafe {
                CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, process_id)
            };
            if snapshot as isize != INVALID_HANDLE_VALUE {
                break;
            }
            snapshot_error = last_error();
            if snapshot_error != ERROR_PARTIAL_COPY || attempt == 7 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if snapshot as isize == INVALID_HANDLE_VALUE {
            return Err(format!(
                "CreateToolhelp32Snapshot({process_id}) failed: {snapshot_error}",
            ));
        }
        let result = (|| {
            let mut entry: ModuleEntry32W = unsafe { zeroed() };
            entry.size = size_of::<ModuleEntry32W>() as u32;
            if unsafe { Module32FirstW(snapshot, &mut entry) } == 0 {
                return Err(format!(
                    "Module32FirstW({process_id}) failed: {}",
                    last_error()
                ));
            }
            let mut modules = Vec::new();
            loop {
                let end = entry
                    .module_name
                    .iter()
                    .position(|value| *value == 0)
                    .unwrap_or(entry.module_name.len());
                let name = String::from_utf16_lossy(&entry.module_name[..end]);
                modules.push((name, entry.base_address as usize, entry.base_size as usize));
                if unsafe { Module32NextW(snapshot, &mut entry) } == 0 {
                    let error = last_error();
                    if error != ERROR_NO_MORE_FILES {
                        return Err(format!(
                            "Module32NextW({process_id}) stopped with error {error}"
                        ));
                    }
                    break;
                }
            }
            Ok(modules)
        })();
        unsafe { CloseHandle(snapshot) };
        result
    }

    fn process_creation_nonce(process: Handle) -> Result<u64, String> {
        let mut creation = 0u64;
        let mut exit = 0u64;
        let mut kernel = 0u64;
        let mut user = 0u64;
        if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
            == 0
            || creation == 0
        {
            return Err(format!(
                "GetProcessTimes for suspended game failed: {}",
                last_error()
            ));
        }
        Ok(creation)
    }

    fn process_image_path(process: Handle) -> Result<PathBuf, String> {
        let mut buffer = vec![0u16; 32_768];
        let mut length = buffer.len() as Dword;
        if unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) } == 0
        {
            return Err(format!(
                "QueryFullProcessImageNameW for suspended process failed: {}",
                last_error()
            ));
        }
        if length == 0 || length as usize > buffer.len() {
            return Err("suspended process image path length is invalid".into());
        }
        buffer.truncate(length as usize);
        Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer)))
    }

    fn file_identity(file: &fs::File) -> Result<(u32, u64, u64), String> {
        let mut information: ByHandleFileInformation = unsafe { zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as Handle, &mut information) }
            == 0
        {
            return Err(format!(
                "GetFileInformationByHandle for suspended image failed: {}",
                last_error()
            ));
        }
        Ok((
            information.volume_serial_number,
            (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low),
            (u64::from(information.file_size_high) << 32) | u64::from(information.file_size_low),
        ))
    }

    fn query_performance_counter() -> Result<u64, String> {
        let mut value = 0i64;
        if unsafe { QueryPerformanceCounter(&mut value) } == 0 || value <= 0 {
            return Err(format!("QueryPerformanceCounter failed: {}", last_error()));
        }
        Ok(value as u64)
    }

    pub(super) fn launcher_argv_path(value: &Path) -> Result<OsString, String> {
        const VERBATIM: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
        const VERBATIM_UNC: &[u16] = &[
            b'\\' as u16,
            b'\\' as u16,
            b'?' as u16,
            b'\\' as u16,
            b'U' as u16,
            b'N' as u16,
            b'C' as u16,
            b'\\' as u16,
        ];
        let wide = value.as_os_str().encode_wide().collect::<Vec<_>>();
        let normalized = if wide.starts_with(VERBATIM_UNC) {
            let mut out = vec![b'\\' as u16, b'\\' as u16];
            out.extend_from_slice(&wide[VERBATIM_UNC.len()..]);
            out
        } else if wide.starts_with(VERBATIM) {
            wide[VERBATIM.len()..].to_vec()
        } else {
            wide
        };
        if normalized.is_empty()
            || normalized.starts_with(&[b'\\' as u16, b'\\' as u16, b'.' as u16, b'\\' as u16])
        {
            return Err("launcher argv path is empty or names a device namespace".into());
        }
        Ok(OsString::from_wide(&normalized))
    }

    pub(super) fn build_launcher_effective_command_line(
        executable: &Path,
        user_dir: &Path,
        semantic_arguments: &[String],
    ) -> Result<OsString, String> {
        validate_launcher_effective_arguments(semantic_arguments)?;
        let executable = launcher_argv_path(executable)?;
        let user_dir = launcher_argv_path(user_dir)?;
        let mut userdir_argument = OsString::from("-userdir=");
        userdir_argument.push(user_dir);
        let arguments = std::iter::once(executable)
            .chain(semantic_arguments.iter().map(OsString::from))
            .chain(std::iter::once(userdir_argument));
        let mut command = OsString::new();
        for (index, argument) in arguments.enumerate() {
            if index != 0 {
                command.push(" ");
            }
            command.push(quote_windows_argument(&argument));
        }
        Ok(command)
    }

    fn quote_windows_argument(value: &OsStr) -> OsString {
        let value = value.encode_wide().collect::<Vec<_>>();
        if !value.is_empty()
            && !value
                .iter()
                .any(|unit| matches!(*unit, 0x09..=0x0d | 0x20) || *unit == b'"' as u16)
        {
            return OsString::from_wide(&value);
        }
        let mut out = vec![b'"' as u16];
        let mut slashes = 0usize;
        for unit in value {
            if unit == b'\\' as u16 {
                slashes += 1;
            } else if unit == b'"' as u16 {
                out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
                out.push(b'"' as u16);
                slashes = 0;
            } else {
                out.extend(std::iter::repeat_n(b'\\' as u16, slashes));
                slashes = 0;
                out.push(unit);
            }
        }
        out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        out.push(b'"' as u16);
        OsString::from_wide(&out)
    }

    fn wide_nul(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn last_error() -> u32 {
        unsafe { GetLastError() }
    }

    #[cfg(test)]
    mod owned_job_tests {
        use super::*;
        use std::mem::ManuallyDrop;

        #[test]
        fn recorder_owned_startup_uses_normal_window_creation() {
            let mut startup: StartupInfoExW = unsafe { zeroed() };
            configure_normal_startup(&mut startup);
            assert_eq!(startup.startup_info.flags, 0);
            assert_eq!(startup.startup_info.show_window, 0);
            assert_eq!(startup.startup_info.x, 0);
            assert_eq!(startup.startup_info.y, 0);
        }

        struct TestProcess {
            process: Handle,
            thread: Handle,
            start_nonce: u64,
        }

        impl Drop for TestProcess {
            fn drop(&mut self) {
                let _ = terminate_exact_owned_process(
                    self.process,
                    self.start_nonce,
                    0xE080_00F1,
                    5_000,
                );
                unsafe {
                    CloseHandle(self.thread);
                    CloseHandle(self.process);
                }
            }
        }

        fn spawn_sleeping_cmd() -> TestProcess {
            let executable = std::env::var_os("ComSpec")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"));
            let executable_wide = wide_nul(executable.as_os_str());
            let mut command = wide_nul(OsStr::new(r#"cmd.exe /d /c "ping -n 30 127.0.0.1 >nul""#));
            let mut startup: StartupInfoW = unsafe { zeroed() };
            startup.cb = size_of::<StartupInfoW>() as Dword;
            let mut process: ProcessInformation = unsafe { zeroed() };
            assert_ne!(
                unsafe {
                    CreateProcessW(
                        executable_wide.as_ptr(),
                        command.as_mut_ptr(),
                        ptr::null_mut(),
                        ptr::null_mut(),
                        0,
                        CREATE_SUSPENDED,
                        ptr::null_mut(),
                        ptr::null(),
                        &mut startup,
                        &mut process,
                    )
                },
                0,
                "test CreateProcessW failed: {}",
                last_error()
            );
            TestProcess {
                process: process.process,
                thread: process.thread,
                start_nonce: process_creation_nonce(process.process).unwrap(),
            }
        }

        fn spawn_sleeping_cmd_in_job(job: Handle) -> TestProcess {
            let executable = std::env::var_os("ComSpec")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"));
            let executable_wide = wide_nul(executable.as_os_str());
            let mut command = wide_nul(OsStr::new(r#"cmd.exe /d /c "ping -n 30 127.0.0.1 >nul""#));
            let job_attribute_list = create_job_attribute_list(job).unwrap();
            clobber_stack_after_attribute_builder_returns();
            let mut startup: StartupInfoExW = unsafe { zeroed() };
            startup.startup_info.cb = size_of::<StartupInfoExW>() as Dword;
            startup.attribute_list = job_attribute_list.as_ptr();
            let mut process: ProcessInformation = unsafe { zeroed() };
            let created = unsafe {
                CreateProcessW(
                    executable_wide.as_ptr(),
                    command.as_mut_ptr(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                    CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
                    ptr::null_mut(),
                    ptr::null(),
                    (&mut startup as *mut StartupInfoExW).cast(),
                    &mut process,
                )
            };
            let create_error = (created == 0).then(last_error);
            drop(job_attribute_list);
            assert_ne!(
                created,
                0,
                "test job-bound CreateProcessW failed: {}",
                create_error.unwrap_or_default()
            );
            TestProcess {
                process: process.process,
                thread: process.thread,
                start_nonce: process_creation_nonce(process.process).unwrap(),
            }
        }

        #[inline(never)]
        fn clobber_stack_after_attribute_builder_returns() {
            let mut words = [0usize; 512];
            for (index, word) in words.iter_mut().enumerate() {
                *word = 0xBAD0_0000usize.wrapping_add(index);
            }
            std::hint::black_box(&mut words);
        }

        #[test]
        fn job_attribute_list_owns_handle_array_across_stack_clobber() {
            let job = create_kill_on_close_job().unwrap();
            let owned = spawn_sleeping_cmd_in_job(job);
            assert_eq!(unsafe { ResumeThread(owned.thread) }, 1);
            unsafe { CloseHandle(job) };
            assert_eq!(
                unsafe { WaitForSingleObject(owned.process, 5_000) },
                WAIT_OBJECT_0
            );
        }

        #[test]
        fn kill_on_close_job_terminates_only_exact_assigned_identity() {
            let protected = spawn_sleeping_cmd();
            let job = create_kill_on_close_job().unwrap();
            let owned = spawn_sleeping_cmd_in_job(job);
            assert_ne!(protected.start_nonce, owned.start_nonce);
            assert_eq!(unsafe { ResumeThread(protected.thread) }, 1);
            assert_eq!(unsafe { ResumeThread(owned.thread) }, 1);
            unsafe { CloseHandle(job) };
            assert_eq!(
                unsafe { WaitForSingleObject(owned.process, 5_000) },
                WAIT_OBJECT_0
            );
            assert_eq!(unsafe { WaitForSingleObject(protected.process, 0) }, 258);
            assert_eq!(
                process_creation_nonce(protected.process).unwrap(),
                protected.start_nonce
            );
        }

        #[test]
        fn identity_mismatch_refuses_direct_kill_but_job_fail_safe_reaps_owned_only() {
            let protected = spawn_sleeping_cmd();
            let job = create_kill_on_close_job().unwrap();
            let owned = spawn_sleeping_cmd_in_job(job);
            assert_eq!(unsafe { ResumeThread(protected.thread) }, 1);
            assert_eq!(unsafe { ResumeThread(owned.thread) }, 1);
            assert!(terminate_exact_owned_process(
                owned.process,
                owned.start_nonce.wrapping_add(1),
                0xE080_00F2,
                5_000,
            )
            .is_err());
            assert_eq!(unsafe { WaitForSingleObject(owned.process, 0) }, 258);
            unsafe { CloseHandle(job) };
            assert_eq!(
                unsafe { WaitForSingleObject(owned.process, 5_000) },
                WAIT_OBJECT_0
            );
            assert_eq!(unsafe { WaitForSingleObject(protected.process, 0) }, 258);
            assert_eq!(
                process_creation_nonce(protected.process).unwrap(),
                protected.start_nonce
            );
        }

        fn proof_for<'a>(
            process: &TestProcess,
            job: Handle,
            creation_job_count: u32,
        ) -> RecorderOwnedCreationJob<'a> {
            RecorderOwnedCreationJob {
                process: process.process,
                job,
                process_id: unsafe { GetProcessId(process.process) },
                process_start_nonce: process.start_nonce,
                creation_job_generation: 0xA729_0001,
                creation_job_count,
                _pending: PhantomData,
            }
        }

        #[test]
        fn exact_recorder_owned_creation_job_is_retained_without_second_assignment() {
            let job = create_kill_on_close_job().unwrap();
            let owned = spawn_sleeping_cmd_in_job(job);
            let pid = unsafe { GetProcessId(owned.process) };
            let proof = proof_for(&owned, job, 1);
            let lease = proof
                .retain_exact_job(owned.process.cast(), pid, owned.start_nonce)
                .unwrap();
            drop(proof);
            unsafe { CloseHandle(job) };
            assert_eq!(
                unsafe { WaitForSingleObject(owned.process, 0) },
                258,
                "duplicated exact creation-job lease must prevent a handle-gap kill"
            );
            drop(lease);
            assert_eq!(
                unsafe { WaitForSingleObject(owned.process, 5_000) },
                WAIT_OBJECT_0
            );
        }

        #[test]
        fn recorder_owned_creation_job_rejects_wrong_pid_nonce_and_pid_reuse_shape() {
            let job = create_kill_on_close_job().unwrap();
            let owned = spawn_sleeping_cmd_in_job(job);
            let pid = unsafe { GetProcessId(owned.process) };
            let proof = proof_for(&owned, job, 1);
            assert!(proof
                .retain_exact_job(owned.process.cast(), pid.wrapping_add(1), owned.start_nonce)
                .is_err());
            assert!(proof
                .retain_exact_job(owned.process.cast(), pid, owned.start_nonce.wrapping_add(1))
                .is_err());
            unsafe { CloseHandle(job) };
        }

        #[test]
        fn recorder_owned_creation_job_rejects_foreign_and_closed_job_handles() {
            let owned_job = create_kill_on_close_job().unwrap();
            let foreign_job = create_kill_on_close_job().unwrap();
            let owned = spawn_sleeping_cmd_in_job(owned_job);
            let pid = unsafe { GetProcessId(owned.process) };
            let foreign = proof_for(&owned, foreign_job, 1);
            assert!(foreign
                .retain_exact_job(owned.process.cast(), pid, owned.start_nonce)
                .is_err());
            unsafe { CloseHandle(foreign_job) };
            let closed = proof_for(&owned, foreign_job, 1);
            assert!(closed
                .retain_exact_job(owned.process.cast(), pid, owned.start_nonce)
                .is_err());
            unsafe { CloseHandle(owned_job) };
        }

        #[test]
        fn nested_or_multi_job_creation_provenance_is_ambiguous_and_rejected() {
            let job = create_kill_on_close_job().unwrap();
            let owned = spawn_sleeping_cmd_in_job(job);
            let pid = unsafe { GetProcessId(owned.process) };
            let nested_shape = proof_for(&owned, job, 2);
            assert!(nested_shape
                .retain_exact_job(owned.process.cast(), pid, owned.start_nonce)
                .is_err());
            unsafe { CloseHandle(job) };
        }

        #[test]
        fn pending_creation_job_authority_is_single_claim_for_concurrent_attach() {
            let job = create_kill_on_close_job().unwrap();
            let owned = ManuallyDrop::new(spawn_sleeping_cmd_in_job(job));
            let pending = PendingEarlyRenderProcess {
                process: Some(owned.process),
                primary_thread: Some(owned.thread),
                kill_on_close_job: Some(job),
                process_id: unsafe { GetProcessId(owned.process) },
                process_start_nonce: owned.start_nonce,
                creation_job_generation: 0xA729_0002,
                creation_job_count: 1,
                creation_job_authority_claimed: AtomicBool::new(false),
                executable: PathBuf::from(r"C:\Windows\System32\cmd.exe"),
                user_dir: std::env::temp_dir(),
                semantic_arguments: Vec::new(),
            };
            let first = pending.recorder_owned_creation_job().unwrap();
            assert!(pending.recorder_owned_creation_job().is_err());
            drop(first);
            assert!(pending.recorder_owned_creation_job().is_err());
            drop(pending);
        }
    }
}

#[cfg(all(windows, feature = "evidence-client-driving"))]
pub use windows::{
    create_suspended_early_render_process, create_suspended_game_process,
    launch_suspended_early_render_capture, EarlyRenderProcess, PendingEarlyRenderProcess,
    RecorderOwnedCreationJob, RecorderOwnedCreationJobLease,
};

#[cfg(all(windows, not(feature = "evidence-client-driving")))]
mod observer_only {
    use super::*;

    const DISABLED: &str =
        "client-driving is physically excluded from the production observer binary; build the Evidence tier with feature evidence-client-driving";

    pub struct EarlyRenderProcess {
        pub process_id: u32,
        pub receipt: EarlyRenderCaptureReceipt,
    }

    impl EarlyRenderProcess {
        pub fn wait(&self, _timeout_ms: u32) -> Result<bool, String> {
            Err(DISABLED.into())
        }

        pub fn terminate_diagnostic(&mut self) -> Result<(), String> {
            Err(DISABLED.into())
        }
    }

    pub struct PendingEarlyRenderProcess {
        process_id: u32,
        process_start_nonce: u64,
    }

    impl PendingEarlyRenderProcess {
        pub fn process_id(&self) -> u32 {
            self.process_id
        }

        pub fn process_start_nonce(&self) -> u64 {
            self.process_start_nonce
        }

        pub fn resolve_modset(&self) -> crate::modset::ModsetReport {
            crate::modset::resolve_modset(self.process_id, None)
        }

        pub fn validate_suspended_image_authority(
            &self,
            _expected_sha256: &str,
        ) -> Result<(), String> {
            Err(DISABLED.into())
        }

        pub fn activate_with_post_resume_validation(
            self,
            _spec: &EarlyRenderLaunchSpec,
            _ready: &mut impl EarlyHookReadyChannel,
            _post_resume_validation: impl FnOnce() -> Result<(), String>,
        ) -> Result<EarlyRenderProcess, String> {
            Err(DISABLED.into())
        }
    }

    pub fn create_suspended_early_render_process(
        _spec: &EarlyRenderLaunchSpec,
    ) -> Result<PendingEarlyRenderProcess, String> {
        Err(DISABLED.into())
    }

    pub fn create_suspended_game_process(
        _spec: &EarlyRenderProcessCreateSpec,
    ) -> Result<PendingEarlyRenderProcess, String> {
        Err(DISABLED.into())
    }

    pub fn launch_suspended_early_render_capture(
        _spec: &EarlyRenderLaunchSpec,
        _ready: &mut impl EarlyHookReadyChannel,
    ) -> Result<EarlyRenderProcess, String> {
        Err(DISABLED.into())
    }
}

#[cfg(all(windows, not(feature = "evidence-client-driving")))]
pub use observer_only::{
    create_suspended_early_render_process, create_suspended_game_process,
    launch_suspended_early_render_capture, EarlyRenderProcess, PendingEarlyRenderProcess,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portable_content_manifest::{BoundDlcLayer, BoundMod};
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn sha(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn hash(bytes: &[u8]) -> String {
        to_hex(&Sha256::digest(bytes))
    }

    fn requested_profile_selection() -> RequestedProfileSelectionReceipt {
        RequestedProfileSelectionReceipt {
            schema: crate::runtime_dlc_roster::REQUESTED_PROFILE_SELECTION_SCHEMA.into(),
            dlc_load_sha256: sha(12),
            disabled_dlc_descriptor_ids: Vec::new(),
            requested_enabled_dlc_ids: vec!["dlc001".into()],
        }
    }

    fn launch(executable_sha256: String) -> PortableLaunchBinding {
        PortableLaunchBinding {
            executable_sha256,
            ordered_mods: vec![BoundMod {
                layer_id: "mod/override.mod".into(),
                descriptor_sha256: sha(2),
                content_sha256: sha(3),
                storage_kind: LayerStorageKind::Directory,
            }],
            enabled_dlc_ids: BTreeSet::from(["dlc001".into()]),
            enabled_dlc_layers: BTreeMap::from([(
                "dlc001".into(),
                BoundDlcLayer {
                    layer_id: "dlc/dlc001.dlc".into(),
                    descriptor_sha256: sha(4),
                    source_container_sha256: Some(sha(5)),
                    storage_kind: LayerStorageKind::ZipArchive,
                },
            )]),
            integrated_dlc_layers: BTreeMap::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::from([
                ("settings_file_sha256".into(), sha(6)),
                ("display_mode".into(), "borderless".into()),
            ]),
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "recorder_early_launch_{label}_{}_{}",
            std::process::id(),
            nonce
        ))
    }

    fn ordered_layers() -> Vec<RecordedContentLayer> {
        vec![
            RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            },
            RecordedContentLayer {
                ordinal: 1,
                layer_kind: ContentLayerKind::Dlc,
                layer_id: "dlc/dlc001.dlc".into(),
                display_name: "DLC 001".into(),
                descriptor_sha256: Some(sha(4)),
                source_container_sha256: Some(sha(5)),
                storage_kind: LayerStorageKind::ZipArchive,
                replace_paths: Vec::new(),
                required_for_playback: true,
            },
            RecordedContentLayer {
                ordinal: 2,
                layer_kind: ContentLayerKind::Mod,
                layer_id: "mod/override.mod".into(),
                display_name: "override".into(),
                descriptor_sha256: Some(sha(2)),
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: vec!["gfx".into()],
                required_for_playback: true,
            },
        ]
    }

    fn receipt(manifest: &EarlyRenderLaunchManifest) -> EarlyRenderCaptureReceipt {
        let mut receipt = EarlyRenderCaptureReceipt {
            schema: EARLY_CAPTURE_RECEIPT_SCHEMA.into(),
            launch_manifest_sha256: manifest.sha256().unwrap(),
            effective_vfs_manifest_sha256: manifest.effective_vfs_manifest_sha256.clone(),
            effective_vfs_index_sha256: sha(9),
            effective_vfs_index_row_count: 3,
            effective_vfs_index_chunk_count: 2,
            content_manifest_bytes_sha256: sha(10),
            content_manifest_byte_count: 512,
            content_manifest_chunk_count: 1,
            process_id: 42,
            process_start_nonce: 43,
            executable_sha256: manifest.executable_sha256.clone(),
            collector_sha256: manifest.collector_sha256.clone(),
            created_suspended: true,
            hook_epoch_nonce: 44,
            hook_set_sha256: sha(7),
            hook_ready_sequence: 1,
            resume_sequence: 2,
            hook_ready_qpc: 100,
            resume_qpc: 101,
            first_creation_sequence: 7,
            first_game_d3d_creation_qpc: 102,
            staged_hook_generation:
                compact_contract::early_render_control::EARLY_STAGED_HOOK_GENERATION,
            staged_hook_count: compact_contract::early_render_control::EARLY_STAGED_HOOK_COUNT,
            staged_hook_set_sha256: sha(11),
            diagnostic_staged_hook_mask: None,
            game_device_ptr: 0x1000,
            immediate_context_ptr: 0x2000,
            effective_runtime_dlc: {
                let names = vec!["DLC 001".to_string()];
                EffectiveRuntimeDlcReceipt {
                    schema: crate::runtime_dlc_roster::EFFECTIVE_RUNTIME_DLC_SCHEMA.into(),
                    source: "engine_active_dlc_manager_v1".into(),
                    manager_getter_rva: compact_contract::early_render_runtime_dlc::A729_DLC_MANAGER_GETTER_RVA,
                    active_dlc_enumerator_rva: compact_contract::early_render_runtime_dlc::A729_ACTIVE_DLC_ENUMERATOR_RVA,
                    string_vector_teardown_rva: compact_contract::early_render_runtime_dlc::A729_STRING_VECTOR_TEARDOWN_RVA,
                    sample_count: 2,
                    double_stable: true,
                    active_display_names_engine_order: names.clone(),
                    enabled_dlc_ids: vec!["dlc001".into()],
                    display_name_to_layer_id: BTreeMap::from([(
                        "DLC 001".into(),
                        "dlc001".into(),
                    )]),
                    engine_order_sha256: to_hex(
                        &compact_contract::early_render_runtime_dlc::canonical_runtime_dlc_roster_sha256(&names),
                    ),
                }
            },
            game_d3d_creation_count_at_hook_ready: 0,
            preexisting_game_d3d_object_count: 0,
            completeness: EarlyHookCompleteness {
                all_shader_create: true,
                input_layout_create: true,
                blend_state_create: true,
                depth_stencil_state_create: true,
                rasterizer_state_create: true,
                sampler_state_create: true,
                buffer_create_update: true,
                texture_create_update: true,
                resource_view_create: true,
                draw_state_readback: true,
            },
            transcript_sha256: String::new(),
        };
        receipt.transcript_sha256 = receipt.expected_transcript_sha256().unwrap();
        receipt
    }

    #[test]
    fn diagnostic_stage_masks_have_exact_authenticated_hook_counts() {
        assert_eq!(staged_hook_count_for_group_mask(0x01).unwrap(), 19);
        assert_eq!(staged_hook_count_for_group_mask(0x03).unwrap(), 21);
        assert_eq!(staged_hook_count_for_group_mask(0x1ff).unwrap(), 68);
        assert_eq!(staged_hook_count_for_group_mask(0x3ff).unwrap(), 71);
        assert_eq!(staged_hook_count_for_group_mask(0).unwrap(), 0);
        assert!(staged_hook_count_for_group_mask(0x02).is_err());
        assert_eq!(staged_hook_count_for_group_mask(0x101).unwrap(), 21);
        assert_eq!(staged_hook_count_for_group_mask(0x201).unwrap(), 22);
    }

    #[test]
    fn diagnostic_com_hook_selector_is_packed_above_the_canonical_group_mask() {
        let one_com_hook = pack_diagnostic_hook_masks(0x01, 0x00001).unwrap();
        assert_eq!(one_com_hook, (1u64 << 32) | 0x01);
        assert_eq!(diagnostic_stage_group_mask(one_com_hook), 0x01);
        assert_eq!(diagnostic_com_hook_mask(one_com_hook), 0x00001);
        assert_eq!(staged_hook_count_for_group_mask(one_com_hook).unwrap(), 1);

        let three_com_hooks = pack_diagnostic_hook_masks(0x03, 0x40003).unwrap();
        assert_eq!(diagnostic_stage_group_mask(three_com_hooks), 0x03);
        assert_eq!(diagnostic_com_hook_mask(three_com_hooks), 0x40003);
        assert_eq!(
            staged_hook_count_for_group_mask(three_com_hooks).unwrap(),
            5
        );

        assert!(pack_diagnostic_hook_masks(0x01, 0).is_err());
        assert!(pack_diagnostic_hook_masks(0x01, 0x7ffff).is_err());
        assert!(pack_diagnostic_hook_masks(0x01, 0x80000).is_err());
        assert!(pack_diagnostic_hook_masks(0x3ff, 0x00001).is_err());
        assert!(pack_diagnostic_hook_masks(0, 0x00001).is_err());
        assert!(staged_hook_count_for_group_mask((1u64 << 51) | 0x01).is_err());
    }

    #[test]
    fn windows_hidden_drive_environment_keys_are_preserved() {
        assert!(valid_windows_environment_key("PATH"));
        assert!(valid_windows_environment_key("=C:"));
        assert!(!valid_windows_environment_key(""));
        assert!(!valid_windows_environment_key("BAD=KEY"));
        assert!(!valid_windows_environment_key("=hidden"));
        assert!(!valid_windows_environment_key("=1:"));
        assert!(!valid_windows_environment_key("=C:=BAD"));
    }

    #[test]
    fn diagnostic_receipt_accepts_com_only_without_changing_production_json() {
        let manifest = EarlyRenderLaunchManifest::from_bound_launch(
            &launch(sha(1)),
            &ordered_layers(),
            &BTreeMap::from([("base".into(), sha(10))]),
            requested_profile_selection(),
            sha(2),
            sha(9),
            Vec::new(),
        )
        .unwrap();
        let production = receipt(&manifest);
        let production_json = serde_json::to_value(&production).unwrap();
        assert!(production_json.get("diagnostic_staged_hook_mask").is_none());

        let mut diagnostic = production;
        diagnostic.diagnostic_staged_hook_mask = Some(0x01);
        diagnostic.staged_hook_count = 19;
        diagnostic.transcript_sha256 = diagnostic.expected_transcript_sha256().unwrap();
        diagnostic.validate().unwrap();
        assert_eq!(
            serde_json::to_value(&diagnostic).unwrap()["diagnostic_staged_hook_mask"],
            1
        );
    }

    #[test]
    fn g2_pointer_rejects_zero_with_leading_zeroes() {
        assert!(!valid_g2_pointer("0x0"));
        assert!(!valid_g2_pointer("0x00"));
        assert!(valid_g2_pointer("0x000A"));
    }

    #[test]
    fn recorder_g2_artifact_admission_recomputes_authority_and_bmp_bytes() {
        let root = temp_root("g2_artifacts");
        let output = root.join("g2_engine");
        fs::create_dir_all(&output).unwrap();
        let context_path = root.join("capture_context.json");
        let context = serde_json::json!({
            "schema": "hoi4-rendering-law-capture-context-v1",
            "packets": ["G2"],
            "inventory": {
                "active_hoi4_pids": [],
                "paths": {
                    "executable": {"sha256": sha(1)},
                    "settings": {"sha256": sha(6)}
                },
                "settings": {"language": "l_english"}
            }
        });
        let context_bytes = serde_json::to_vec(&context).unwrap();
        fs::write(&context_path, &context_bytes).unwrap();
        let intent = G2CaptureIntent {
            observation_id: "g2-test".into(),
            settings_file_sha256: sha(6),
            localisation_language: "l_english".into(),
            camera_payload_sha256: sha(13),
            pov_country_or_global: "global".into(),
            expected_selected_hour: 123,
            capture_context_path: context_path,
            capture_context_sha256: hash(&context_bytes),
        };
        let manifest = EarlyRenderLaunchManifest::from_bound_launch(
            &launch(sha(1)),
            &ordered_layers(),
            &BTreeMap::from([("base".into(), sha(10))]),
            requested_profile_selection(),
            sha(2),
            sha(9),
            Vec::new(),
        )
        .unwrap();
        let receipt = receipt(&manifest);
        let receipt_sha256 = receipt.sha256().unwrap();
        let launch_epoch_sha256 = portable_render_stream::compute_render_launch_epoch_sha256(
            receipt.process_id,
            receipt.process_start_nonce,
            receipt.hook_ready_qpc,
            receipt.first_game_d3d_creation_qpc,
            receipt.first_creation_sequence,
            receipt.hook_epoch_nonce,
            &receipt.hook_set_sha256,
            &receipt_sha256,
        );
        let pixels = [1u8, 2, 3, 4];
        let mut bmp = Vec::new();
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&58u32.to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&1i32.to_le_bytes());
        bmp.extend_from_slice(&(-1i32).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&4u32.to_le_bytes());
        bmp.extend_from_slice(&[0; 16]);
        bmp.extend_from_slice(&pixels);
        fs::write(output.join(G2_ENGINE_BMP_FILE), &bmp).unwrap();
        let content_digest = sha(14);
        let witness = serde_json::json!({
            "schema": G2_ENGINE_WITNESS_SCHEMA,
            "status": G2_ENGINE_STATUS,
            "observation_id": intent.observation_id,
            "render_boundary": G2_RENDER_BOUNDARY,
            "pixel_format": "BGRA8_UNORM",
            "authority_exe_sha256": manifest.executable_sha256,
            "launch_epoch_sha256": launch_epoch_sha256,
            "capture_receipt_sha256": receipt_sha256,
            "effective_vfs_manifest_sha256": manifest.effective_vfs_manifest_sha256,
            "content_manifest_digest_sha256": content_digest,
            "settings_file_sha256": intent.settings_file_sha256,
            "localisation_language": intent.localisation_language,
            "camera_payload_sha256": intent.camera_payload_sha256,
            "pov_country_or_global": intent.pov_country_or_global,
            "selected_hour": intent.expected_selected_hour,
            "selected_present_sequence": 1,
            "viewport_width": 1,
            "viewport_height": 1,
            "source_dxgi_format": 87,
            "source_sample_count": 1,
            "engine_bmp_file": G2_ENGINE_BMP_FILE,
            "engine_bmp_sha256": hash(&bmp),
            "engine_bgra_sha256": hash(&pixels),
            "pixel_byte_len": pixels.len(),
            "capture_context_sha256": intent.capture_context_sha256,
            "close_zoom_authentication": {
                "camera_owner_vtable_rva": 0x0295_2510,
                "camera_root_pointer": "0x5000",
                "current_eye_f32_bits": [1.0f32.to_bits(), 0x4248_29C8, 2.0f32.to_bits()],
                "current_look_at_f32_bits": [1.0f32.to_bits(), 3.0f32.to_bits(), 2.0f32.to_bits()],
                "target_eye_f32_bits": [1.0f32.to_bits(), 0x4248_29C8, 2.0f32.to_bits()],
                "target_look_at_f32_bits": [1.0f32.to_bits(), 3.0f32.to_bits(), 2.0f32.to_bits()],
                "required_height_f32_bits": 0x4248_29C8,
                "exact_retained_close_zoom_band": true
            },
            "global_pov_authentication": {
                "observer_ui_root_rva": 0x01DD_9390,
                "observer_predicate_function_rva": 0x006F_0430,
                "game_state_root_rva": 0x0331_6E90,
                "selected_controller_handle_offset": 0x520,
                "selected_controller_handle": 1,
                "handle_remap_table_offset": 0x340,
                "controller_table_offset": 0x310,
                "remapped_controller_index": 1,
                "controller_pointer": "0x4000",
                "controller_observer_count_offset": 0x41c,
                "controller_observer_count": 0,
                "global_observer_mode": true
            },
            "post_process_boundary_authentication": {
                "producer_rva": production_render_submit_live_lane::POST_PROCESS_RENDER_ROOT_RVA,
                "immediate_context_pointer": "0x1000",
                "registered_backbuffer_view_pointer": "0x2000",
                "registered_backbuffer_resource_pointer": "0x3000",
                "registration_sequence": 4,
                "recreate_generation": 1,
                "exact_swapchain_backbuffer_registration": true,
                "post_process_original_returned": true,
                "readback_inserted_before_post_process_hook_return": true
            }
        });
        fs::write(
            output.join(G2_ENGINE_WITNESS_FILE),
            serde_json::to_vec_pretty(&witness).unwrap(),
        )
        .unwrap();
        validate_g2_capture_artifacts(&intent, &output, &manifest, &receipt, &content_digest)
            .unwrap();

        let mut drift = witness;
        drift["unexpected"] = serde_json::json!(true);
        fs::write(
            output.join(G2_ENGINE_WITNESS_FILE),
            serde_json::to_vec_pretty(&drift).unwrap(),
        )
        .unwrap();
        assert!(validate_g2_capture_artifacts(
            &intent,
            &output,
            &manifest,
            &receipt,
            &content_digest,
        )
        .is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn relocated_launch_with_mod_override_and_selected_dlc_has_one_portable_identity() {
        let first = temp_root("first");
        let second = temp_root("second");
        let game_bytes = b"relocatable-game";
        let collector_bytes = b"relocatable-collector";
        for root in [&first, &second] {
            fs::create_dir_all(root.join("profile")).unwrap();
            fs::write(root.join("profile/dlc_load.json"), b"{}").unwrap();
            fs::write(root.join("hoi4.exe"), game_bytes).unwrap();
            fs::write(root.join("collector.dll"), collector_bytes).unwrap();
        }
        let launch = launch(hash(game_bytes));
        let manifest = EarlyRenderLaunchManifest::from_bound_launch(
            &launch,
            &ordered_layers(),
            &BTreeMap::from([("base".into(), sha(10))]),
            requested_profile_selection(),
            hash(collector_bytes),
            sha(9),
            vec!["-debug_smooth".into()],
        )
        .unwrap();
        for root in [&first, &second] {
            EarlyRenderLaunchSpec {
                executable_path: root.join("hoi4.exe"),
                user_dir: root.join("profile"),
                collector_path: root.join("collector.dll"),
                manifest: manifest.clone(),
                hook_ready_timeout_ms: 30_000,
                minimize_after_first_d3d: true,
                diagnostic_staged_hook_mask: None,
                g2_capture_intent: None,
                g2_output_directory: None,
                g2_content_manifest_digest_sha256: None,
            }
            .validate_local_files()
            .unwrap();
        }
        assert_eq!(
            manifest
                .ordered_content_layers
                .iter()
                .map(|row| row.layer_id.as_str())
                .collect::<Vec<_>>(),
            vec!["base", "dlc/dlc001.dlc", "mod/override.mod"]
        );
        assert_eq!(manifest.effective_runtime_dlc_candidate_ids, vec!["dlc001"]);
        assert_eq!(
            manifest.sha256().unwrap(),
            manifest.clone().sha256().unwrap()
        );
        let receipt = receipt(&manifest);
        receipt.validate().unwrap();
        let mut relocated = receipt.clone();
        assert_eq!(receipt.sha256().unwrap(), relocated.sha256().unwrap());
        relocated.hook_epoch_nonce += 1;
        assert!(relocated.validate().is_err());
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(second).unwrap();
    }

    #[test]
    fn late_incomplete_or_userdir_overriding_launch_fails_closed() {
        let launch = launch(sha(1));
        let manifest = EarlyRenderLaunchManifest::from_bound_launch(
            &launch,
            &ordered_layers(),
            &BTreeMap::from([("base".into(), sha(10))]),
            requested_profile_selection(),
            sha(8),
            sha(9),
            vec!["-userdir=C:/wrong".into()],
        );
        assert!(manifest.is_err());

        let manifest = EarlyRenderLaunchManifest::from_bound_launch(
            &launch,
            &ordered_layers(),
            &BTreeMap::from([("base".into(), sha(10))]),
            requested_profile_selection(),
            sha(8),
            sha(9),
            Vec::new(),
        )
        .unwrap();
        let mut value = receipt(&manifest);
        value.created_suspended = false;
        assert!(value.validate().is_err());
        let mut value = receipt(&manifest);
        value.preexisting_game_d3d_object_count = 1;
        assert!(value.validate().is_err());
        let mut value = receipt(&manifest);
        value.completeness.texture_create_update = false;
        assert!(value.validate().is_err());
        let mut value = receipt(&manifest);
        value.first_creation_sequence = 0;
        assert!(value.validate().is_err());
    }

    #[test]
    fn continue_save_requires_launcher_effective_gdpr_argument_and_order() {
        let missing = vec!["--continuelastsave".to_string()];
        assert!(validate_launcher_effective_arguments(&missing).is_err());
        let raw_launcher_setting = vec![
            "--continuelastsave".to_string(),
            "-gdpr-compliant".to_string(),
        ];
        assert!(validate_launcher_effective_arguments(&raw_launcher_setting).is_err());
        let reversed = vec![
            "--gdpr-compliant".to_string(),
            "--continuelastsave".to_string(),
        ];
        assert!(validate_launcher_effective_arguments(&reversed).is_err());
        let exact = vec![
            "--continuelastsave".to_string(),
            "--gdpr-compliant".to_string(),
        ];
        validate_launcher_effective_arguments(&exact).unwrap();
    }

    #[test]
    fn launcher_effective_arguments_reject_case_insensitive_duplicates() {
        let duplicate = vec!["--debug_mode".to_string(), "--DEBUG_MODE".to_string()];
        assert!(validate_launcher_effective_arguments(&duplicate).is_err());
    }

    #[cfg(all(windows, feature = "evidence-client-driving"))]
    #[test]
    fn launcher_argv_paths_drop_only_verbatim_prefixes() {
        assert_eq!(
            windows::launcher_argv_path(Path::new(
                r"\\?\X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe"
            ))
            .unwrap(),
            OsString::from(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe")
        );
        assert_eq!(
            windows::launcher_argv_path(Path::new(r"\\?\UNC\server\share\hoi4.exe")).unwrap(),
            OsString::from(r"\\server\share\hoi4.exe")
        );
        assert!(windows::launcher_argv_path(Path::new(r"\\.\PhysicalDrive0")).is_err());
    }

    #[cfg(all(windows, feature = "evidence-client-driving"))]
    #[test]
    fn launcher_command_line_matches_authoritative_continue_shape() {
        let command = windows::build_launcher_effective_command_line(
            Path::new(r"\\?\X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe"),
            Path::new(
                r"\\?\C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV",
            ),
            &[
                "--continuelastsave".to_string(),
                "--gdpr-compliant".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(
            command,
            OsString::from(
                r#""X:\SteamLibrary\steamapps\common\Hearts of Iron IV\hoi4.exe" --continuelastsave --gdpr-compliant "-userdir=C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV""#
            )
        );
        assert!(!command.to_string_lossy().contains(r"\\?\"));
    }
}
