//! Exact collector-side READY/FIRST/COMMIT proof adapter for H4CT.

use crate::duplex_client::EarlyRenderControlBinding;
use compact_contract::early_render_control::{
    EarlyCommitCaptureReceiptV1, EarlyFirstGameD3dV1, EarlyG2CaptureArmV1, EarlyHookReadyV1,
    EARLY_HOOK_COMPLETENESS_MASK,
};
use compact_contract::early_render_runtime_dlc::EarlyRenderRuntimeDlcRosterV1;
use production_render_submit_live_lane::a729_hook;

fn decode_sha256(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 {
        return Err("early-render SHA-256 is not 64 lowercase hex characters".into());
    }
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
            .ok_or("early-render SHA-256 is not canonical lowercase hex")?;
    }
    Ok(out)
}

fn encode_sha256(value: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in value {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn ready_proof(binding: &EarlyRenderControlBinding) -> Result<EarlyHookReadyV1, String> {
    let ready = a729_hook::early_capture_readiness()?;
    let executable_sha256 = decode_sha256(&ready.process.executable_sha256)?;
    if ready.process.process_id != binding.process_id
        || ready.process.process_start_nonce != binding.process_start_nonce
        || executable_sha256 != binding.executable_sha256
        || ready.d3d_device_created_qpc.is_some()
        || ready.game_device_count != 0
        || ready.first_creation_sequence.is_some()
        || ready.creation_hook_count != a729_hook::BOOTSTRAP_HOOK_COUNT as u32
        || ready.excluded_probe_device_ptr != 0
    {
        return Err("early-render READY was observed after game D3D creation or for a foreign process epoch".into());
    }
    Ok(EarlyHookReadyV1 {
        launch_manifest_sha256: binding.launch_manifest_sha256,
        effective_vfs_manifest_sha256: binding.effective_vfs_manifest_sha256,
        effective_vfs_index_sha256: binding.effective_vfs_index_sha256,
        effective_vfs_index_row_count: binding.effective_vfs_index_row_count,
        effective_vfs_index_chunk_count: binding.effective_vfs_index_chunk_count,
        content_manifest_bytes_sha256: binding.content_manifest_bytes_sha256,
        content_manifest_byte_count: binding.content_manifest_byte_count,
        content_manifest_chunk_count: binding.content_manifest_chunk_count,
        process_id: ready.process.process_id,
        process_start_nonce: ready.process.process_start_nonce,
        executable_sha256,
        collector_sha256: binding.collector_sha256,
        hook_epoch_nonce: ready.hook_epoch_nonce,
        hook_set_sha256: decode_sha256(&ready.hook_set_receipt_sha256)?,
        hook_ready_sequence: ready.hook_ready_sequence,
        hook_ready_qpc: ready.observer_installed_qpc,
        game_d3d_creation_count_at_hook_ready: 0,
        preexisting_game_d3d_object_count: 0,
        completeness_bits: EARLY_HOOK_COMPLETENESS_MASK,
    })
}

pub fn first_game_d3d_proof(
    binding: &EarlyRenderControlBinding,
    hook_epoch_nonce: u64,
) -> Result<Option<EarlyFirstGameD3dV1>, String> {
    let ready = a729_hook::early_capture_readiness()?;
    if ready.process.process_id != binding.process_id
        || ready.process.process_start_nonce != binding.process_start_nonce
        || ready.hook_epoch_nonce != hook_epoch_nonce
        || ready.game_device_count > 1
    {
        return Err("early-render FIRST belongs to a foreign/repeated game D3D epoch".into());
    }
    let Some(creation_qpc) = ready.d3d_device_created_qpc else {
        return Ok(None);
    };
    if ready.game_device_count != 1
        || ready.first_creation_sequence.is_none()
        || a729_hook::staged_hook_count_for_group_mask(ready.staged_hook_group_mask)
            != Some(ready.staged_hook_count)
        || ready.creation_hook_count
            != a729_hook::BOOTSTRAP_HOOK_COUNT as u32 + ready.staged_hook_count
        || ready.staged_hook_generation != 1
        || (!ready.diagnostic_stage_probe
            && (ready.staged_hook_group_mask != a729_hook::PRODUCTION_STAGED_HOOK_GROUP_MASK
                || ready.staged_hook_count != a729_hook::STAGED_HOOK_COUNT as u32))
        || (ready.diagnostic_stage_probe
            && ready.staged_hook_group_mask == a729_hook::PRODUCTION_STAGED_HOOK_GROUP_MASK)
        || ready.staged_hook_set_sha256.len() != 64
        || ready.game_device_ptr == 0
        || ready.immediate_context_ptr == 0
    {
        return Ok(None);
    }
    let first_creation_sequence = ready
        .first_creation_sequence
        .ok_or("early-render FIRST has no creation sequence")?;
    Ok(Some(EarlyFirstGameD3dV1 {
        launch_manifest_sha256: binding.launch_manifest_sha256,
        process_id: binding.process_id,
        process_start_nonce: binding.process_start_nonce,
        hook_epoch_nonce,
        game_device_ordinal: 1,
        first_creation_sequence,
        creation_qpc,
        staged_hook_generation: ready.staged_hook_generation,
        staged_hook_count: ready.staged_hook_count,
        diagnostic_stage_probe: ready.diagnostic_stage_probe,
        staged_hook_group_mask: ready.staged_hook_group_mask,
        staged_hook_set_sha256: decode_sha256(&ready.staged_hook_set_sha256)?,
        game_device_ptr: u64::try_from(ready.game_device_ptr)
            .map_err(|_| "game device pointer does not fit the FIRST wire")?,
        immediate_context_ptr: u64::try_from(ready.immediate_context_ptr)
            .map_err(|_| "immediate context pointer does not fit the FIRST wire")?,
        // The staged lifecycle creates no collector NULL probe at all, which
        // is stronger than identifying and filtering one after creation.
        collector_null_probe_excluded: true,
    }))
}

pub fn runtime_dlc_roster_proof(
    binding: &EarlyRenderControlBinding,
    hook_epoch_nonce: u64,
    staged_hook_generation: u64,
    game_device_ptr: u64,
) -> Result<Option<EarlyRenderRuntimeDlcRosterV1>, String> {
    let ready = a729_hook::early_capture_readiness()?;
    if ready.process.process_id != binding.process_id
        || ready.process.process_start_nonce != binding.process_start_nonce
        || ready.hook_epoch_nonce != hook_epoch_nonce
    {
        return Err("runtime DLC roster belongs to a foreign process or hook epoch".into());
    }
    if ready.first_creation_sequence.is_none() {
        return Ok(None);
    }
    if ready.staged_hook_generation != staged_hook_generation
        || ready.game_device_ptr as u64 != game_device_ptr
    {
        return Err("runtime DLC roster request differs from accepted FIRST identity".into());
    }
    let observed = a729_hook::runtime_dlc_roster_observation()?;
    if observed.process_id != binding.process_id
        || observed.process_start_nonce != binding.process_start_nonce
        || observed.hook_epoch_nonce != hook_epoch_nonce
        || observed.staged_hook_generation != staged_hook_generation
        || observed.game_device_ptr as u64 != game_device_ptr
    {
        return Err("runtime DLC roster observation differs from accepted FIRST identity".into());
    }
    Ok(Some(EarlyRenderRuntimeDlcRosterV1 {
        launch_manifest_sha256: binding.launch_manifest_sha256,
        effective_vfs_manifest_sha256: binding.effective_vfs_manifest_sha256,
        process_id: observed.process_id,
        process_start_nonce: observed.process_start_nonce,
        hook_epoch_nonce: observed.hook_epoch_nonce,
        staged_hook_generation: observed.staged_hook_generation,
        game_device_ptr: observed.game_device_ptr as u64,
        manager_getter_rva: observed.manager_getter_rva as u64,
        active_dlc_enumerator_rva: observed.active_dlc_enumerator_rva as u64,
        string_vector_teardown_rva: observed.string_vector_teardown_rva as u64,
        sample_count: observed.sample_count,
        double_stable: observed.double_stable,
        roster_count: observed.roster_count,
        raw_name_roster_sha256: decode_sha256(&observed.raw_name_roster_sha256)?,
        raw_names: observed.raw_names,
    }))
}

pub fn commit_capture_receipt(
    binding: &EarlyRenderControlBinding,
    commit: &EarlyCommitCaptureReceiptV1,
) -> Result<(), String> {
    if commit.process_id != binding.process_id
        || commit.process_start_nonce != binding.process_start_nonce
    {
        return Err("early-render COMMIT names a foreign process epoch".into());
    }
    a729_hook::commit_capture_receipt(
        commit.process_id,
        commit.process_start_nonce,
        commit.hook_epoch_nonce,
        &encode_sha256(&commit.receipt_sha256),
    )
}

pub fn arm_g2_capture(
    binding: &EarlyRenderControlBinding,
    commit: &EarlyCommitCaptureReceiptV1,
    arm: EarlyG2CaptureArmV1,
) -> Result<(), String> {
    if arm.process_id != binding.process_id
        || arm.process_start_nonce != binding.process_start_nonce
        || arm.hook_epoch_nonce != commit.hook_epoch_nonce
        || arm.launch_manifest_sha256 != binding.launch_manifest_sha256
        || arm.effective_vfs_manifest_sha256 != binding.effective_vfs_manifest_sha256
        || arm.authority_exe_sha256 != binding.executable_sha256
        || arm.capture_receipt_sha256 != commit.receipt_sha256
    {
        return Err("G2 arm differs from the authenticated launch/COMMIT binding".into());
    }
    a729_hook::arm_g2_capture(arm)
}
