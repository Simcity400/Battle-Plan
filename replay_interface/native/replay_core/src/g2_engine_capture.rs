//! Fail-closed admission for the recorder-owned post-process-end G2 observation.
//!
//! G2 is optional. Its presence is declared only by the selected game's
//! `index.json`; loose role entries cannot opt a recording into the contract.
//! When declared, both payloads must be independently authenticated container
//! entries and the witness must join the recorder's early-render authority.

use serde::Deserialize;
use serde_json::{Map, Value};
use snapshot::sha256::{to_hex, Sha256};
use snapshot::Store;

const PACKAGED_SCHEMA: &str = "hoi4-g2-packaged-engine-capture-v1";
const WITNESS_SCHEMA: &str = "hoi4-g2-engine-pixel-witness-v3";
const WITNESS_ROLE: &str = "g2_engine_witness";
const BMP_ROLE: &str = "g2_engine_bgra_bmp";
const WITNESS_PATH: &str = "g2/g2_engine_pixel_witness.json";
const BMP_PATH: &str = "g2/g2_engine_map_boundary.bmp";
const BMP_FILE: &str = "g2_engine_map_boundary.bmp";
const STATUS: &str = "AUTHENTICATED_POST_PROCESS_END_MAP_READBACK";
const POST_PROCESS_RENDER_ROOT_RVA: u32 = 0x00FF_3450;
const RENDER_BOUNDARY: &str = "post_authenticated_map_bgra8_no_selected_present_writers";
const EARLY_LAUNCH_SCHEMA: &str = "extractor.early-render-launch-manifest.v1";
const EARLY_RECEIPT_SCHEMA: &str = "extractor.early-render-capture-receipt.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackagedG2EngineCapture {
    pub selected_game_id: String,
    pub selected_index_path: String,
    pub selected_index_sha256: String,
    pub witness_path: String,
    pub witness_sha256: String,
    pub witness_bytes: u64,
    pub bmp_path: String,
    pub bmp_sha256: String,
    pub bmp_bytes: u64,
    pub capture_content_manifest_digest_sha256: String,
    pub replay_content_manifest_digest_sha256: String,
    pub witness: G2EnginePixelWitness,
    engine_bgra: Box<[u8]>,
}

impl PackagedG2EngineCapture {
    /// Exact, top-down, tightly packed BGRA8 engine pixels authenticated by
    /// the selected replay-container index, role table, witness, and recorder
    /// authority. Consumers must still join these bytes to the selected replay
    /// scene before treating them as a parity oracle.
    pub fn engine_bgra(&self) -> &[u8] {
        &self.engine_bgra
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct G2PostProcessBoundaryAuthentication {
    pub producer_rva: u32,
    pub immediate_context_pointer: String,
    pub registered_backbuffer_view_pointer: String,
    pub registered_backbuffer_resource_pointer: String,
    pub registration_sequence: u64,
    pub recreate_generation: u64,
    pub exact_swapchain_backbuffer_registration: bool,
    pub post_process_original_returned: bool,
    pub readback_inserted_before_post_process_hook_return: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct G2GlobalPovAuthentication {
    pub observer_ui_root_rva: u32,
    pub observer_predicate_function_rva: u32,
    pub game_state_root_rva: usize,
    pub selected_controller_handle_offset: u32,
    pub selected_controller_handle: u32,
    pub handle_remap_table_offset: u32,
    pub controller_table_offset: u32,
    pub remapped_controller_index: u32,
    pub controller_pointer: String,
    pub controller_observer_count_offset: u32,
    pub controller_observer_count: i32,
    pub global_observer_mode: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct G2CloseZoomAuthentication {
    pub camera_owner_vtable_rva: u32,
    pub camera_root_pointer: String,
    pub current_eye_f32_bits: [u32; 3],
    pub current_look_at_f32_bits: [u32; 3],
    pub target_eye_f32_bits: [u32; 3],
    pub target_look_at_f32_bits: [u32; 3],
    pub required_height_f32_bits: u32,
    pub exact_retained_close_zoom_band: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct G2EnginePixelWitness {
    pub schema: String,
    pub status: String,
    pub observation_id: String,
    pub render_boundary: String,
    pub pixel_format: String,
    pub authority_exe_sha256: String,
    pub launch_epoch_sha256: String,
    pub capture_receipt_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub content_manifest_digest_sha256: String,
    pub settings_file_sha256: String,
    pub localisation_language: String,
    pub camera_payload_sha256: String,
    pub pov_country_or_global: String,
    pub selected_hour: i64,
    pub selected_present_sequence: u32,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub source_dxgi_format: u32,
    pub source_sample_count: u32,
    pub engine_bmp_file: String,
    pub engine_bmp_sha256: String,
    pub engine_bgra_sha256: String,
    pub pixel_byte_len: u64,
    pub capture_context_sha256: String,
    pub close_zoom_authentication: G2CloseZoomAuthentication,
    pub global_pov_authentication: G2GlobalPovAuthentication,
    pub post_process_boundary_authentication: G2PostProcessBoundaryAuthentication,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexDeclaration {
    schema: String,
    witness_path: String,
    witness_sha256: String,
    witness_bytes: u64,
    bmp_path: String,
    bmp_sha256: String,
    bmp_bytes: u64,
    capture_content_manifest_digest_sha256: String,
    replay_content_manifest_digest_sha256: String,
}

pub(crate) fn admit_from_store(
    store: &Store,
    game_id: &str,
    recorder_manifest: Option<&Map<String, Value>>,
) -> Result<Option<PackagedG2EngineCapture>, String> {
    let recorder_declares_g2 = recorder_manifest
        .and_then(|root| root.get("replay_packaging"))
        .and_then(Value::as_object)
        .and_then(|packaging| packaging.get("g2_engine_capture"))
        .is_some_and(|declaration| !declaration.is_null());
    let Some(container) = store.container() else {
        if recorder_declares_g2 {
            return Err(
                "recorder manifest declares G2 outside an authenticated .battleplan container"
                    .into(),
            );
        }
        return Ok(None);
    };
    let index_path = format!("payload/{game_id}/index.json");
    let index_bytes = container
        .read_entry_bytes(&index_path)
        .map_err(|error| format!("read authenticated G2 index {index_path:?}: {error}"))?;
    let index = snapshot::parse_json_strict(&index_bytes)
        .map_err(|error| format!("strict parse G2 index declaration: {error}"))?;
    let declaration = index.get("g2_engine_capture");
    let witness_roles = container.entries_by_role(WITNESS_ROLE);
    let bmp_roles = container.entries_by_role(BMP_ROLE);
    let Some(declaration) = declaration.filter(|value| !value.is_null()) else {
        if !witness_roles.is_empty() || !bmp_roles.is_empty() || recorder_declares_g2 {
            return Err(
                "container or recorder manifest has G2 packaging evidence but index.json does not declare G2"
                    .into(),
            );
        }
        return Ok(None);
    };
    let declaration: IndexDeclaration = serde_json::from_value(declaration.clone())
        .map_err(|error| format!("invalid exact G2 index declaration: {error}"))?;
    validate_declaration(&declaration)?;
    let witness_entry = match witness_roles.as_slice() {
        [entry] => entry,
        entries => {
            return Err(format!(
                "G2 index requires exactly one {WITNESS_ROLE:?} role; found {}",
                entries.len()
            ))
        }
    };
    let bmp_entry = match bmp_roles.as_slice() {
        [entry] => entry,
        entries => {
            return Err(format!(
                "G2 index requires exactly one {BMP_ROLE:?} role; found {}",
                entries.len()
            ))
        }
    };
    let expected_witness_path = format!("payload/{game_id}/{WITNESS_PATH}");
    let expected_bmp_path = format!("payload/{game_id}/{BMP_PATH}");
    validate_entry(
        &witness_entry.path,
        &witness_entry.sha256,
        witness_entry.bytes,
        &expected_witness_path,
        &declaration.witness_sha256,
        declaration.witness_bytes,
        WITNESS_ROLE,
    )?;
    validate_entry(
        &bmp_entry.path,
        &bmp_entry.sha256,
        bmp_entry.bytes,
        &expected_bmp_path,
        &declaration.bmp_sha256,
        declaration.bmp_bytes,
        BMP_ROLE,
    )?;
    let witness_bytes = container
        .read_entry_bytes(&expected_witness_path)
        .map_err(|error| format!("read authenticated G2 witness: {error}"))?;
    let bmp = container
        .read_entry_bytes(&expected_bmp_path)
        .map_err(|error| format!("read authenticated G2 BMP: {error}"))?;
    validate_bytes(
        &witness_bytes,
        declaration.witness_bytes,
        &declaration.witness_sha256,
        "G2 witness",
    )?;
    validate_bytes(
        &bmp,
        declaration.bmp_bytes,
        &declaration.bmp_sha256,
        "G2 BMP",
    )?;
    let witness = decode_witness(&witness_bytes)?;
    validate_witness_and_bmp(&witness, &bmp)?;
    validate_recorder_authority(
        &witness,
        &declaration,
        recorder_manifest.ok_or(
            "packaged G2 capture has no recorder_manifest early-render authority",
        )?,
    )?;
    Ok(Some(PackagedG2EngineCapture {
        selected_game_id: game_id.to_owned(),
        selected_index_path: index_path,
        selected_index_sha256: to_hex(&Sha256::digest(&index_bytes)),
        witness_path: declaration.witness_path,
        witness_sha256: declaration.witness_sha256,
        witness_bytes: declaration.witness_bytes,
        bmp_path: declaration.bmp_path,
        bmp_sha256: declaration.bmp_sha256,
        bmp_bytes: declaration.bmp_bytes,
        capture_content_manifest_digest_sha256: declaration
            .capture_content_manifest_digest_sha256,
        replay_content_manifest_digest_sha256: declaration.replay_content_manifest_digest_sha256,
        witness,
        engine_bgra: bmp[54..].into(),
    }))
}

fn validate_declaration(declaration: &IndexDeclaration) -> Result<(), String> {
    if declaration.schema != PACKAGED_SCHEMA
        || declaration.witness_path != WITNESS_PATH
        || declaration.bmp_path != BMP_PATH
        || !canonical_sha256(&declaration.witness_sha256)
        || !canonical_sha256(&declaration.bmp_sha256)
        || !canonical_sha256(&declaration.capture_content_manifest_digest_sha256)
        || !canonical_sha256(&declaration.replay_content_manifest_digest_sha256)
        || declaration.witness_bytes == 0
        || declaration.bmp_bytes < 54
    {
        return Err("G2 index declaration violates its exact path/hash/length contract".into());
    }
    Ok(())
}

fn decode_witness(bytes: &[u8]) -> Result<G2EnginePixelWitness, String> {
    let value = snapshot::parse_json_strict(bytes)
        .map_err(|error| format!("strict parse packaged G2 witness: {error}"))?;
    serde_json::from_value(value)
        .map_err(|error| format!("decode exact packaged G2 witness: {error}"))
}

fn validate_entry(
    entry_path: &str,
    entry_sha256: &str,
    entry_bytes: u64,
    path: &str,
    sha256: &str,
    bytes: u64,
    role: &str,
) -> Result<(), String> {
    if entry_path != path || entry_sha256 != sha256 || entry_bytes != bytes {
        return Err(format!(
            "G2 {role} role entry does not byte-match its canonical index identity"
        ));
    }
    Ok(())
}

fn validate_bytes(bytes: &[u8], expected_len: u64, expected_sha: &str, role: &str) -> Result<(), String> {
    if u64::try_from(bytes.len()).ok() != Some(expected_len)
        || to_hex(&Sha256::digest(bytes)) != expected_sha
    {
        return Err(format!("{role} authenticated bytes differ from index identity"));
    }
    Ok(())
}

fn validate_witness_and_bmp(witness: &G2EnginePixelWitness, bmp: &[u8]) -> Result<(), String> {
    let boundary = &witness.post_process_boundary_authentication;
    let pov = &witness.global_pov_authentication;
    let camera = &witness.close_zoom_authentication;
    if witness.schema != WITNESS_SCHEMA
        || witness.status != STATUS
        || witness.observation_id.trim().is_empty()
        || witness.observation_id.len() > 512
        || witness.render_boundary != RENDER_BOUNDARY
        || witness.pixel_format != "BGRA8_UNORM"
        || witness.pov_country_or_global != "global"
        || witness.selected_hour < 0
        || witness.selected_present_sequence == 0
        || witness.viewport_width == 0
        || witness.viewport_height == 0
        || !matches!(witness.source_dxgi_format, 28 | 29 | 87 | 91)
        || witness.source_sample_count != 1
        || witness.engine_bmp_file != BMP_FILE
        || witness.localisation_language.trim().is_empty()
        || boundary.producer_rva != POST_PROCESS_RENDER_ROOT_RVA
        || !valid_pointer(&boundary.immediate_context_pointer)
        || !valid_pointer(&boundary.registered_backbuffer_view_pointer)
        || !valid_pointer(&boundary.registered_backbuffer_resource_pointer)
        || boundary.registration_sequence == 0
        || boundary.recreate_generation == 0
        || !boundary.exact_swapchain_backbuffer_registration
        || !boundary.post_process_original_returned
        || !boundary.readback_inserted_before_post_process_hook_return
        || pov.observer_ui_root_rva != 0x01DD_9390
        || pov.observer_predicate_function_rva != 0x006F_0430
        || pov.game_state_root_rva != 0x0331_6E90
        || !matches!(pov.selected_controller_handle_offset, 0x520 | 0x524)
        || pov.handle_remap_table_offset != 0x340
        || pov.controller_table_offset != 0x310
        || !valid_pointer(&pov.controller_pointer)
        || pov.controller_observer_count_offset != 0x41c
        || pov.controller_observer_count >= 1
        || !pov.global_observer_mode
        || camera.camera_owner_vtable_rva != 0x0295_2510
        || !valid_pointer(&camera.camera_root_pointer)
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
        return Err("packaged G2 witness violates exact post-process boundary fields".into());
    }
    for digest in [
        &witness.authority_exe_sha256,
        &witness.launch_epoch_sha256,
        &witness.capture_receipt_sha256,
        &witness.effective_vfs_manifest_sha256,
        &witness.content_manifest_digest_sha256,
        &witness.settings_file_sha256,
        &witness.camera_payload_sha256,
        &witness.engine_bmp_sha256,
        &witness.engine_bgra_sha256,
        &witness.capture_context_sha256,
    ] {
        if !canonical_sha256(digest) {
            return Err("packaged G2 witness contains a noncanonical SHA-256".into());
        }
    }
    if bmp.len() < 54
        || &bmp[..2] != b"BM"
        || u32_at(bmp, 2).and_then(|value| usize::try_from(value).ok()) != Some(bmp.len())
        || bmp.get(6..10) != Some([0u8; 4].as_slice())
        || u32_at(bmp, 10) != Some(54)
        || u32_at(bmp, 14) != Some(40)
        || u16_at(bmp, 26) != Some(1)
        || u16_at(bmp, 28) != Some(32)
        || u32_at(bmp, 30) != Some(0)
        || bmp.get(38..54) != Some([0u8; 16].as_slice())
    {
        return Err("packaged G2 BMP is not exact uncompressed BGRA32".into());
    }
    let width = i32_at(bmp, 18).ok_or("packaged G2 BMP has no width")?;
    let signed_height = i32_at(bmp, 22).ok_or("packaged G2 BMP has no height")?;
    if width <= 0
        || signed_height >= 0
        || width as u32 != witness.viewport_width
        || signed_height.unsigned_abs() != witness.viewport_height
    {
        return Err("packaged G2 BMP dimensions differ from its witness".into());
    }
    let expected_pixels = u64::from(witness.viewport_width)
        .checked_mul(u64::from(witness.viewport_height))
        .and_then(|count| count.checked_mul(4))
        .ok_or("packaged G2 pixel dimensions overflow")?;
    let pixels = &bmp[54..];
    if u64::try_from(pixels.len()).ok() != Some(expected_pixels)
        || witness.pixel_byte_len != expected_pixels
        || u32_at(bmp, 34) != u32::try_from(expected_pixels).ok()
        || witness.engine_bmp_sha256 != to_hex(&Sha256::digest(bmp))
        || witness.engine_bgra_sha256 != to_hex(&Sha256::digest(pixels))
    {
        return Err("packaged G2 BMP bytes or digests differ from its witness".into());
    }
    Ok(())
}

fn validate_recorder_authority(
    witness: &G2EnginePixelWitness,
    declaration: &IndexDeclaration,
    root: &Map<String, Value>,
) -> Result<(), String> {
    let launch = object(root.get("early_render_launch_manifest"), "early launch manifest")?;
    let receipt = object(root.get("early_render_capture_receipt"), "early capture receipt")?;
    let content = object(root.get("portable_content_manifest"), "portable content manifest")?;
    let modset = object(root.get("modset"), "recorder modset")?;
    let replay_packaging = object(root.get("replay_packaging"), "replay_packaging")?;
    let packaged_declaration: IndexDeclaration = serde_json::from_value(
        replay_packaging
            .get("g2_engine_capture")
            .cloned()
            .ok_or("recorder replay_packaging has no G2 declaration")?,
    )
    .map_err(|error| format!("recorder replay_packaging G2 declaration is invalid: {error}"))?;
    if &packaged_declaration != declaration {
        return Err(
            "index G2 declaration differs from recorder_manifest replay_packaging seal".into(),
        );
    }
    if string(launch, "schema")? != EARLY_LAUNCH_SCHEMA
        || string(receipt, "schema")? != EARLY_RECEIPT_SCHEMA
    {
        return Err("packaged G2 recorder authority has an unsupported early-render schema".into());
    }
    let receipt_sha = string(root, "early_render_capture_receipt_sha256")?;
    for digest in [
        receipt_sha,
        string(launch, "executable_sha256")?,
        string(launch, "effective_vfs_manifest_sha256")?,
        string(receipt, "launch_manifest_sha256")?,
        string(receipt, "effective_vfs_manifest_sha256")?,
        string(receipt, "executable_sha256")?,
        string(receipt, "hook_set_sha256")?,
        string(content, "manifest_digest_sha256")?,
    ] {
        if !canonical_sha256(digest) {
            return Err("packaged G2 recorder authority contains a noncanonical SHA-256".into());
        }
    }
    let launch_exe = string(launch, "executable_sha256")?;
    let launch_vfs = string(launch, "effective_vfs_manifest_sha256")?;
    let launch_language = string(launch, "localisation_language")?;
    let settings = object(launch.get("required_settings"), "early launch required_settings")?;
    let settings_sha = string(settings, "settings_file_sha256")?;
    // v2 KEEPS THE LAUNCH SETTINGS IN THEIR OWN BINDING. A resolved content
    // chain no longer implies the settings were proven, so the language, the
    // display values and the digest they were parsed from all hang off
    // `settings_binding` together with the authority that proved them --
    // reading them off `modset` directly is reading the v1 vocabulary.
    let settings_binding = object(
        modset.get("settings_binding"),
        "recorder modset settings_binding",
    )?;
    let display = object(
        settings_binding.get("display_variant"),
        "recorder display_variant",
    )?;
    let recorder_settings_sha = string(settings_binding, "attach_settings_sha256")?;
    let recorder_language = string(settings_binding, "language")?;
    let content_digest = string(content, "manifest_digest_sha256")?;
    if witness.capture_receipt_sha256 != receipt_sha
        || witness.authority_exe_sha256 != launch_exe
        || witness.authority_exe_sha256 != string(receipt, "executable_sha256")?
        || witness.effective_vfs_manifest_sha256 != launch_vfs
        || witness.effective_vfs_manifest_sha256
            != string(receipt, "effective_vfs_manifest_sha256")?
        || witness.content_manifest_digest_sha256
            != declaration.capture_content_manifest_digest_sha256
        || content_digest != declaration.replay_content_manifest_digest_sha256
        || witness.settings_file_sha256 != settings_sha
        || witness.settings_file_sha256 != recorder_settings_sha
        || witness.localisation_language != launch_language
        || witness.localisation_language != recorder_language
    {
        return Err("packaged G2 witness does not join recorder early-render authority".into());
    }
    if let Some(value) = content.get("effective_vfs_manifest_sha256") {
        if value.as_str() != Some(witness.effective_vfs_manifest_sha256.as_str()) {
            return Err("packaged G2 witness differs from portable-content VFS authority".into());
        }
    }
    if let Some(value) = content.get("localisation_language") {
        if value.as_str() != Some(witness.localisation_language.as_str()) {
            return Err("packaged G2 witness differs from portable-content language authority".into());
        }
    }
    if let Some(required_settings) = content
        .get("required_settings")
        .and_then(Value::as_object)
    {
        if required_settings
            .get("settings_file_sha256")
            .and_then(Value::as_str)
            != Some(witness.settings_file_sha256.as_str())
        {
            return Err("packaged G2 witness differs from portable-content settings authority".into());
        }
    }
    let launch_epoch = portable_render_stream::compute_render_launch_epoch_sha256(
        u32_value(receipt, "process_id")?,
        u64_value(receipt, "process_start_nonce")?,
        u64_value(receipt, "hook_ready_qpc")?,
        u64_value(receipt, "first_game_d3d_creation_qpc")?,
        u64_value(receipt, "first_creation_sequence")?,
        u64_value(receipt, "hook_epoch_nonce")?,
        string(receipt, "hook_set_sha256")?,
        receipt_sha,
    );
    if witness.launch_epoch_sha256 != launch_epoch {
        return Err("packaged G2 launch epoch does not recompute from recorder receipt".into());
    }
    Ok(())
}

fn object<'a>(value: Option<&'a Value>, role: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .and_then(Value::as_object)
        .ok_or_else(|| format!("packaged G2 recorder authority has no {role} object"))
}

fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("packaged G2 recorder authority has no {key} string"))
}

fn u64_value(object: &Map<String, Value>, key: &str) -> Result<u64, String> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("packaged G2 recorder authority has no {key} u64"))
}

fn u32_value(object: &Map<String, Value>, key: &str) -> Result<u32, String> {
    u64_value(object, key)
        .and_then(|value| u32::try_from(value).map_err(|_| format!("G2 {key} exceeds u32")))
}

fn canonical_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_pointer(value: &str) -> bool {
    let Some(digits) = value.strip_prefix("0x") else {
        return false;
    };
    !digits.is_empty()
        && digits.bytes().any(|byte| byte != b'0')
        && digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn i32_at(bytes: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use snapshot::pack::{pack, PackEntry, PackIdentity};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    fn sha(bytes: &[u8]) -> String {
        to_hex(&Sha256::digest(bytes))
    }

    fn bmp() -> Vec<u8> {
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
        bmp
    }

    fn portable_content_manifest() -> portable_content_manifest::ContentManifest {
        let mut manifest = portable_content_manifest::ContentManifest {
            schema: portable_content_manifest::SCHEMA.into(),
            product_modset_sha256: "10".repeat(32),
            source_game: portable_content_manifest::SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256: "22".repeat(32),
                build_profile: "hoi4-a729".into(),
            },
            source_profile: portable_content_manifest::SourceProfileIdentity {
                profile_id: "g2-fixture-profile".into(),
                playset_id: "g2-fixture-playset".into(),
                selection_sha256: "00".repeat(32),
            },
            effective_vfs_manifest_sha256: "33".repeat(32),
            ordered_layers: Vec::new(),
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::from([
                ("physical_width".into(), "1920".into()),
                ("physical_height".into(), "1080".into()),
                ("min_gui_width".into(), "1280".into()),
                ("min_gui_height".into(), "720".into()),
                ("gui_scale_f32_bits".into(), 1.0f32.to_bits().to_string()),
                ("refresh_rate_hz".into(), "60".into()),
                ("max_refresh_rate_hz".into(), "60".into()),
                ("fullscreen".into(), "false".into()),
                ("borderless".into(), "true".into()),
                ("display_mode".into(), "borderless_fullscreen".into()),
                ("settings_file_sha256".into(), "44".repeat(32)),
                // The authority that proved these settings travels in the
                // same map, so a replay cannot reproduce them without knowing
                // how well they were proven.
                ("settings_authority".into(), "pre_launch_bracketed".into()),
            ]),
            required_content: Vec::new(),
            manifest_digest_sha256: None,
        };
        manifest.source_profile.selection_sha256 =
            manifest.computed_selection_sha256().unwrap();
        manifest.with_computed_digest().unwrap()
    }

    fn authority(receipt_sha: &str) -> Map<String, Value> {
        let hook_set = "11".repeat(32);
        let content_manifest = portable_content_manifest();
        let launch_epoch = portable_render_stream::compute_render_launch_epoch_sha256(
            4,
            5,
            6,
            8,
            7,
            9,
            &hook_set,
            receipt_sha,
        );
        json!({
            "early_render_launch_manifest": {
                "schema": EARLY_LAUNCH_SCHEMA,
                "executable_sha256": "22".repeat(32),
                "effective_vfs_manifest_sha256": "33".repeat(32),
                "localisation_language": "l_english",
                "required_settings": {"settings_file_sha256": "44".repeat(32)}
            },
            "early_render_capture_receipt": {
                "schema": EARLY_RECEIPT_SCHEMA,
                "launch_manifest_sha256": "55".repeat(32),
                "effective_vfs_manifest_sha256": "33".repeat(32),
                "executable_sha256": "22".repeat(32),
                "process_id": 4,
                "process_start_nonce": 5,
                "hook_ready_qpc": 6,
                "first_creation_sequence": 7,
                "first_game_d3d_creation_qpc": 8,
                "hook_epoch_nonce": 9,
                "hook_set_sha256": hook_set
            },
            "early_render_capture_receipt_sha256": receipt_sha,
            "portable_content_manifest": serde_json::to_value(&content_manifest).unwrap(),
            "modset": {
                "resolved": true,
                "errors": [],
                "game": {"exe_sha256": content_manifest.source_game.executable_sha256.clone()},
                "dlc": {"enabled": []},
                "dlc_display_names": {},
                "settings_binding": {
                    "authority": "pre_launch_bracketed",
                    "launch_proven": true,
                    "errors": [],
                    "language": "l_english",
                    "display_variant": {
                        "physical_width": 1920,
                        "physical_height": 1080,
                        "min_gui_width": 1280,
                        "min_gui_height": 720,
                        "gui_scale_f32_bits": 1.0f32.to_bits(),
                        "refresh_rate_hz": 60,
                        "max_refresh_rate_hz": 60,
                        "fullscreen": false,
                        "borderless": true,
                        "display_mode": "borderless_fullscreen",
                    },
                    "attach_settings_sha256": "44".repeat(32),
                    "seal_settings_sha256": null,
                    "endpoint_semantics_equal": null
                },
            },
            "test_launch_epoch": launch_epoch
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn witness(authority: &Map<String, Value>, bmp: &[u8]) -> Vec<u8> {
        let pixels = &bmp[54..];
        serde_json::to_vec(&json!({
            "schema": WITNESS_SCHEMA,
            "status": STATUS,
            "observation_id": "g2-test",
            "render_boundary": RENDER_BOUNDARY,
            "pixel_format": "BGRA8_UNORM",
            "authority_exe_sha256": "22".repeat(32),
            "launch_epoch_sha256": authority["test_launch_epoch"],
            "capture_receipt_sha256": authority["early_render_capture_receipt_sha256"],
            "effective_vfs_manifest_sha256": "33".repeat(32),
            "content_manifest_digest_sha256": "aa".repeat(32),
            "settings_file_sha256": "44".repeat(32),
            "localisation_language": "l_english",
            "camera_payload_sha256": "77".repeat(32),
            "pov_country_or_global": "global",
            "selected_hour": 12,
            "selected_present_sequence": 1,
            "viewport_width": 1,
            "viewport_height": 1,
            "source_dxgi_format": 87,
            "source_sample_count": 1,
            "engine_bmp_file": BMP_FILE,
            "engine_bmp_sha256": sha(bmp),
            "engine_bgra_sha256": sha(pixels),
            "pixel_byte_len": pixels.len(),
            "capture_context_sha256": "88".repeat(32),
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
                "producer_rva": POST_PROCESS_RENDER_ROOT_RVA,
                "immediate_context_pointer": "0x1000",
                "registered_backbuffer_view_pointer": "0x2000",
                "registered_backbuffer_resource_pointer": "0x3000",
                "registration_sequence": 4,
                "recreate_generation": 1,
                "exact_swapchain_backbuffer_registration": true,
                "post_process_original_returned": true,
                "readback_inserted_before_post_process_hook_return": true
            }
        }))
        .unwrap()
    }

    fn temp_file(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "replay_g2_{tag}_{}_{}.battleplan",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn package(extra_witness_role: bool) -> (PathBuf, Map<String, Value>) {
        let receipt_sha = "99".repeat(32);
        let mut authority = authority(&receipt_sha);
        let content_manifest: portable_content_manifest::ContentManifest =
            serde_json::from_value(authority["portable_content_manifest"].clone()).unwrap();
        let replay_content_manifest_digest_sha256 = content_manifest
            .manifest_digest_sha256
            .clone()
            .expect("finalized portable content manifest");
        let bmp = bmp();
        let witness = witness(&authority, &bmp);
        let declaration = json!({
            "schema": PACKAGED_SCHEMA,
            "witness_path": WITNESS_PATH,
            "witness_sha256": sha(&witness),
            "witness_bytes": witness.len(),
            "bmp_path": BMP_PATH,
            "bmp_sha256": sha(&bmp),
            "bmp_bytes": bmp.len(),
            "capture_content_manifest_digest_sha256": "aa".repeat(32),
            "replay_content_manifest_digest_sha256": replay_content_manifest_digest_sha256
        });
        authority.insert(
            "replay_packaging".into(),
            json!({
                "g2_engine_capture": declaration.clone(),
                "static_artifacts": []
            }),
        );
        let index = serde_json::to_vec(&json!({
            "game_id": "g",
            "schema_version": "1",
            "snapshots": [{"hour_index": 12, "file": "000000000012.snapshot.json"}],
            "g2_engine_capture": declaration,
            "static_artifacts": [
                {"name": "templates_equipment_catalog", "present": false},
                {"name": "localisation_join", "present": false},
                {"name": "decisions_static_definitions", "present": false},
                {"name": "idea_tech_loc_xref_static", "present": false},
                {"name": "ship_definitions_static", "present": false},
                {"name": "gfx_icon_asset_manifest_static", "present": false},
                {"name": "technology_sharing_static", "present": false},
                {"name": "focus_defs_static", "present": false}
            ]
        }))
        .unwrap();
        let mut entries = vec![
            PackEntry {
                path: "payload/g/index.json".into(),
                bytes: index,
                role: "index",
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/g/{WITNESS_PATH}"),
                bytes: witness,
                role: WITNESS_ROLE,
                hour_index: None,
            },
            PackEntry {
                path: format!("payload/g/{BMP_PATH}"),
                bytes: bmp,
                role: BMP_ROLE,
                hour_index: None,
            },
            PackEntry {
                path: "payload/g/timeline.json".into(),
                bytes: serde_json::to_vec(&json!({
                    "format": "hoi4_compact_stream_timeline_v1",
                    "stream_format": "compact",
                    "stream": "collector_raw/collector_stream.bin",
                    "session_manifest": "recorder/native_session_manifest.json",
                    "raw_stream_role": "collector_compact_stream",
                    "session_manifest_role": "recorder_manifest"
                }))
                .unwrap(),
                role: "timeline",
                hour_index: None,
            },
            PackEntry {
                path: "payload/g/collector_raw/collector_stream.bin".into(),
                bytes: b"H4CRfixture-stream".to_vec(),
                role: snapshot::COMPACT_STREAM_ROLE,
                hour_index: None,
            },
            PackEntry {
                path: "payload/g/recorder/native_session_manifest.json".into(),
                bytes: serde_json::to_vec(&authority).unwrap(),
                role: "recorder_manifest",
                hour_index: None,
            },
            PackEntry {
                path: "payload/g/000000000012.snapshot.json".into(),
                bytes: br#"{"game_id":"g","schema_version":"1","hour_index":12}"#.to_vec(),
                role: "snapshot",
                hour_index: Some(12),
            },
        ];
        if extra_witness_role {
            entries.push(PackEntry {
                path: "payload/g/g2/duplicate.json".into(),
                bytes: b"{}".to_vec(),
                role: WITNESS_ROLE,
                hour_index: None,
            });
        }
        let packed = pack(
            "g",
            &entries,
            &PackIdentity {
                schema_version: Some("1".into()),
                content_manifest: Some(content_manifest),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        let path = temp_file(if extra_witness_role { "duplicate" } else { "valid" });
        fs::write(&path, packed).unwrap();
        (path, authority)
    }

    #[test]
    fn exact_container_roles_index_bytes_bmp_and_recorder_authority_are_admitted() {
        let (path, authority) = package(false);
        let store = portable_test_fs_retry::with_read_guard(&path, || {
            Store::open_replay(&path, &snapshot::ExpectedIdentity::default())
        })
        .expect("hold the package open across the scanner's window")
        .unwrap();
        let capture = admit_from_store(&store, "g", Some(&authority))
            .unwrap()
            .expect("declared G2 capture");
        assert_eq!(capture.witness.selected_hour, 12);
        assert_eq!(capture.bmp_path, BMP_PATH);
        assert_eq!(capture.selected_game_id, "g");
        assert_eq!(capture.selected_index_path, "payload/g/index.json");
        assert_eq!(capture.engine_bgra(), &[1, 2, 3, 4]);
        let selected_index_bytes = store
            .container()
            .unwrap()
            .read_entry_bytes(&capture.selected_index_path)
            .unwrap();
        assert_eq!(
            capture.selected_index_sha256,
            to_hex(&Sha256::digest(&selected_index_bytes))
        );
        drop(store);
        portable_test_fs_retry::remove_file(path).unwrap();
    }

    #[test]
    fn replay_source_exposes_only_the_fully_admitted_packaged_capture() {
        let (path, _) = package(false);
        let source = portable_test_fs_retry::with_read_guard(&path, || {
            crate::source::open_recording(&path)
        })
        .expect("hold the package open across the scanner's window")
        .unwrap();
        let capture = source.g2_engine_capture().expect("admitted G2 package");
        assert_eq!(capture.witness.viewport_width, 1);
        assert_eq!(capture.selected_game_id, "g");
        assert_eq!(capture.selected_index_path, "payload/g/index.json");
        assert!(canonical_sha256(&capture.selected_index_sha256));
        assert_eq!(capture.capture_content_manifest_digest_sha256, "aa".repeat(32));
        assert_eq!(
            capture.replay_content_manifest_digest_sha256,
            portable_content_manifest()
                .manifest_digest_sha256
                .expect("finalized portable content manifest")
        );
        drop(source);
        portable_test_fs_retry::remove_file(path).unwrap();
    }

    #[test]
    fn duplicate_g2_role_is_rejected_even_when_the_index_selects_one_path() {
        let (path, authority) = package(true);
        let store = portable_test_fs_retry::with_read_guard(&path, || {
            Store::open_replay(&path, &snapshot::ExpectedIdentity::default())
        })
        .expect("hold the package open across the scanner's window")
        .unwrap();
        let error = admit_from_store(&store, "g", Some(&authority)).unwrap_err();
        assert!(error.contains("exactly one"), "{error}");
        drop(store);
        portable_test_fs_retry::remove_file(path).unwrap();
    }

    #[test]
    fn index_declaration_must_equal_recorder_packaging_seal() {
        let (path, mut authority) = package(false);
        authority["replay_packaging"]["g2_engine_capture"]["bmp_bytes"] = json!(59);
        let store = portable_test_fs_retry::with_read_guard(&path, || {
            Store::open_replay(&path, &snapshot::ExpectedIdentity::default())
        })
        .expect("hold the package open across the scanner's window")
        .unwrap();
        let error = admit_from_store(&store, "g", Some(&authority)).unwrap_err();
        assert!(error.contains("differs"), "{error}");
        drop(store);
        portable_test_fs_retry::remove_file(path).unwrap();
    }

    #[test]
    fn exact_index_declaration_rejects_unknown_fields() {
        let value = json!({
            "schema": PACKAGED_SCHEMA,
            "witness_path": WITNESS_PATH,
            "witness_sha256": "11".repeat(32),
            "witness_bytes": 1,
            "bmp_path": BMP_PATH,
            "bmp_sha256": "22".repeat(32),
            "bmp_bytes": 54,
            "capture_content_manifest_digest_sha256": "33".repeat(32),
            "replay_content_manifest_digest_sha256": "44".repeat(32),
            "unexpected": true
        });
        assert!(serde_json::from_value::<IndexDeclaration>(value).is_err());
    }

    #[test]
    fn recorder_and_index_null_g2_markers_match_absent_roles() {
        let index = br#"{"game_id":"g","schema_version":"1","snapshots":[],"g2_engine_capture":null}"#.to_vec();
        let entries = [PackEntry {
            path: "payload/g/index.json".into(),
            bytes: index,
            role: "index",
            hour_index: None,
        }];
        let path = temp_file("absent");
        fs::write(
            &path,
            pack("g", &entries, &PackIdentity::default(), false).unwrap(),
        )
        .unwrap();
        let store = portable_test_fs_retry::with_read_guard(&path, || {
            Store::open_replay(&path, &snapshot::ExpectedIdentity::default())
        })
        .expect("hold the package open across the scanner's window")
        .unwrap();
        let manifest = json!({"replay_packaging": {"g2_engine_capture": null}});
        assert!(admit_from_store(&store, "g", manifest.as_object()).unwrap().is_none());
        drop(store);
        portable_test_fs_retry::remove_file(path).unwrap();
    }

    #[test]
    fn witness_duplicate_members_are_rejected_before_typed_projection() {
        assert!(decode_witness(
            br#"{"schema":"hoi4-g2-engine-pixel-witness-v3","schema":"forged"}"#
        )
        .is_err());
    }

    #[test]
    fn bmp_pixel_tamper_is_rejected_against_the_strict_witness_digest() {
        let authority = authority(&"99".repeat(32));
        let mut bmp = bmp();
        let decoded = decode_witness(&witness(&authority, &bmp)).unwrap();
        bmp[54] ^= 0xff;
        assert!(validate_witness_and_bmp(&decoded, &bmp).is_err());
    }

    #[test]
    fn zero_pointer_with_leading_zeroes_is_rejected() {
        assert!(!valid_pointer("0x0"));
        assert!(!valid_pointer("0x00"));
        assert!(valid_pointer("0x000A"));
    }
}
