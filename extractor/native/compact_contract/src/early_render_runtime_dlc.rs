use crate::campaign_identity::Sha256;
use crate::early_render_control::EARLY_STAGED_HOOK_GENERATION;
use std::collections::BTreeSet;

const MAGIC: &[u8; 4] = b"EDLC";
const VERSION: u16 = 1;
const FIXED_LEN: usize = 176;
pub const MAX_RUNTIME_DLC_COUNT: usize = 256;
pub const MAX_RUNTIME_DLC_NAME_BYTES: usize = 4096;
pub const A729_DLC_MANAGER_GETTER_RVA: u64 = 0x0206_0f10;
pub const A729_ACTIVE_DLC_ENUMERATOR_RVA: u64 = 0x0206_3660;
pub const A729_STRING_VECTOR_TEARDOWN_RVA: u64 = 0x0012_07d0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EarlyRenderRuntimeDlcRosterV1 {
    pub launch_manifest_sha256: [u8; 32],
    pub effective_vfs_manifest_sha256: [u8; 32],
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
    pub raw_name_roster_sha256: [u8; 32],
    pub raw_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EarlyRenderRuntimeDlcCodecError {
    Length,
    Magic,
    Version,
    Reserved,
    InvalidField,
    Utf8,
}

pub fn canonical_runtime_dlc_roster_sha256(raw_names: &[String]) -> [u8; 32] {
    let mut preimage = Vec::new();
    preimage.extend_from_slice(b"hoi4-a729-runtime-dlc-roster-v1\0");
    preimage.extend_from_slice(&(raw_names.len() as u32).to_le_bytes());
    for (ordinal, name) in raw_names.iter().enumerate() {
        preimage.extend_from_slice(&(ordinal as u32).to_le_bytes());
        preimage.extend_from_slice(&(name.len() as u32).to_le_bytes());
        preimage.extend_from_slice(name.as_bytes());
    }
    Sha256::digest(&preimage)
}

fn nonzero_hash(value: &[u8; 32]) -> bool {
    value.iter().any(|byte| *byte != 0)
}

fn validate(value: &EarlyRenderRuntimeDlcRosterV1) -> Result<(), EarlyRenderRuntimeDlcCodecError> {
    if value.process_id == 0
        || value.process_start_nonce == 0
        || value.hook_epoch_nonce == 0
        || value.staged_hook_generation != EARLY_STAGED_HOOK_GENERATION
        || value.game_device_ptr == 0
        || value.manager_getter_rva != A729_DLC_MANAGER_GETTER_RVA
        || value.active_dlc_enumerator_rva != A729_ACTIVE_DLC_ENUMERATOR_RVA
        || value.string_vector_teardown_rva != A729_STRING_VECTOR_TEARDOWN_RVA
        || value.sample_count != 2
        || !value.double_stable
        || value.roster_count as usize != value.raw_names.len()
        || value.raw_names.len() > MAX_RUNTIME_DLC_COUNT
        || !nonzero_hash(&value.launch_manifest_sha256)
        || !nonzero_hash(&value.effective_vfs_manifest_sha256)
        || value.raw_name_roster_sha256 != canonical_runtime_dlc_roster_sha256(&value.raw_names)
    {
        return Err(EarlyRenderRuntimeDlcCodecError::InvalidField);
    }
    let mut unique = BTreeSet::new();
    for name in &value.raw_names {
        if name.is_empty()
            || name.len() > MAX_RUNTIME_DLC_NAME_BYTES
            || name.as_bytes().contains(&0)
            || !unique.insert(name.as_str())
        {
            return Err(EarlyRenderRuntimeDlcCodecError::InvalidField);
        }
    }
    Ok(())
}

pub fn encode_early_render_runtime_dlc_roster(
    value: &EarlyRenderRuntimeDlcRosterV1,
) -> Result<Vec<u8>, EarlyRenderRuntimeDlcCodecError> {
    validate(value)?;
    let name_bytes = value
        .raw_names
        .iter()
        .try_fold(0usize, |total, name| total.checked_add(4 + name.len()))
        .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?;
    let mut out = Vec::with_capacity(
        FIXED_LEN
            .checked_add(name_bytes)
            .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?,
    );
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&value.launch_manifest_sha256);
    out.extend_from_slice(&value.effective_vfs_manifest_sha256);
    out.extend_from_slice(&value.process_id.to_le_bytes());
    out.extend_from_slice(&value.process_start_nonce.to_le_bytes());
    out.extend_from_slice(&value.hook_epoch_nonce.to_le_bytes());
    out.extend_from_slice(&value.staged_hook_generation.to_le_bytes());
    out.extend_from_slice(&value.game_device_ptr.to_le_bytes());
    out.extend_from_slice(&value.manager_getter_rva.to_le_bytes());
    out.extend_from_slice(&value.active_dlc_enumerator_rva.to_le_bytes());
    out.extend_from_slice(&value.string_vector_teardown_rva.to_le_bytes());
    out.extend_from_slice(&value.sample_count.to_le_bytes());
    out.push(u8::from(value.double_stable));
    out.extend_from_slice(&[0; 3]);
    out.extend_from_slice(&value.roster_count.to_le_bytes());
    out.extend_from_slice(&value.raw_name_roster_sha256);
    for name in &value.raw_names {
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
    }
    Ok(out)
}

pub fn decode_early_render_runtime_dlc_roster(
    bytes: &[u8],
) -> Result<EarlyRenderRuntimeDlcRosterV1, EarlyRenderRuntimeDlcCodecError> {
    if bytes.len() < FIXED_LEN {
        return Err(EarlyRenderRuntimeDlcCodecError::Length);
    }
    if bytes.get(..4) != Some(MAGIC) {
        return Err(EarlyRenderRuntimeDlcCodecError::Magic);
    }
    let mut at = 4usize;
    if take_u16(bytes, &mut at)? != VERSION {
        return Err(EarlyRenderRuntimeDlcCodecError::Version);
    }
    if take_u16(bytes, &mut at)? != 0 {
        return Err(EarlyRenderRuntimeDlcCodecError::Reserved);
    }
    let launch_manifest_sha256 = take_hash(bytes, &mut at)?;
    let effective_vfs_manifest_sha256 = take_hash(bytes, &mut at)?;
    let process_id = take_u32(bytes, &mut at)?;
    let process_start_nonce = take_u64(bytes, &mut at)?;
    let hook_epoch_nonce = take_u64(bytes, &mut at)?;
    let staged_hook_generation = take_u64(bytes, &mut at)?;
    let game_device_ptr = take_u64(bytes, &mut at)?;
    let manager_getter_rva = take_u64(bytes, &mut at)?;
    let active_dlc_enumerator_rva = take_u64(bytes, &mut at)?;
    let string_vector_teardown_rva = take_u64(bytes, &mut at)?;
    let sample_count = take_u32(bytes, &mut at)?;
    let double_stable = match *bytes
        .get(at)
        .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?
    {
        0 => false,
        1 => true,
        _ => return Err(EarlyRenderRuntimeDlcCodecError::Reserved),
    };
    at += 1;
    if bytes.get(at..at + 3) != Some(&[0; 3]) {
        return Err(EarlyRenderRuntimeDlcCodecError::Reserved);
    }
    at += 3;
    let roster_count = take_u32(bytes, &mut at)?;
    let raw_name_roster_sha256 = take_hash(bytes, &mut at)?;
    if roster_count as usize > MAX_RUNTIME_DLC_COUNT {
        return Err(EarlyRenderRuntimeDlcCodecError::InvalidField);
    }
    let mut raw_names = Vec::with_capacity(roster_count as usize);
    for _ in 0..roster_count {
        let len = take_u32(bytes, &mut at)? as usize;
        if len > MAX_RUNTIME_DLC_NAME_BYTES {
            return Err(EarlyRenderRuntimeDlcCodecError::InvalidField);
        }
        let end = at
            .checked_add(len)
            .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?;
        let raw = bytes
            .get(at..end)
            .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?;
        raw_names.push(
            std::str::from_utf8(raw)
                .map_err(|_| EarlyRenderRuntimeDlcCodecError::Utf8)?
                .to_owned(),
        );
        at = end;
    }
    if at != bytes.len() {
        return Err(EarlyRenderRuntimeDlcCodecError::Length);
    }
    let value = EarlyRenderRuntimeDlcRosterV1 {
        launch_manifest_sha256,
        effective_vfs_manifest_sha256,
        process_id,
        process_start_nonce,
        hook_epoch_nonce,
        staged_hook_generation,
        game_device_ptr,
        manager_getter_rva,
        active_dlc_enumerator_rva,
        string_vector_teardown_rva,
        sample_count,
        double_stable,
        roster_count,
        raw_name_roster_sha256,
        raw_names,
    };
    validate(&value)?;
    if encode_early_render_runtime_dlc_roster(&value)? != bytes {
        return Err(EarlyRenderRuntimeDlcCodecError::InvalidField);
    }
    Ok(value)
}

fn take_u16(bytes: &[u8], at: &mut usize) -> Result<u16, EarlyRenderRuntimeDlcCodecError> {
    let end = at
        .checked_add(2)
        .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?;
    let value = u16::from_le_bytes(
        bytes
            .get(*at..end)
            .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at = end;
    Ok(value)
}

fn take_u32(bytes: &[u8], at: &mut usize) -> Result<u32, EarlyRenderRuntimeDlcCodecError> {
    let end = at
        .checked_add(4)
        .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?;
    let value = u32::from_le_bytes(
        bytes
            .get(*at..end)
            .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at = end;
    Ok(value)
}

fn take_u64(bytes: &[u8], at: &mut usize) -> Result<u64, EarlyRenderRuntimeDlcCodecError> {
    let end = at
        .checked_add(8)
        .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?;
    let value = u64::from_le_bytes(
        bytes
            .get(*at..end)
            .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?
            .try_into()
            .unwrap(),
    );
    *at = end;
    Ok(value)
}

fn take_hash(bytes: &[u8], at: &mut usize) -> Result<[u8; 32], EarlyRenderRuntimeDlcCodecError> {
    let end = at
        .checked_add(32)
        .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?;
    let value = bytes
        .get(*at..end)
        .ok_or(EarlyRenderRuntimeDlcCodecError::Length)?
        .try_into()
        .unwrap();
    *at = end;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> EarlyRenderRuntimeDlcRosterV1 {
        let raw_names = vec!["Together for Victory".into(), "München Pack".into()];
        EarlyRenderRuntimeDlcRosterV1 {
            launch_manifest_sha256: [1; 32],
            effective_vfs_manifest_sha256: [2; 32],
            process_id: 7,
            process_start_nonce: 8,
            hook_epoch_nonce: 9,
            staged_hook_generation: EARLY_STAGED_HOOK_GENERATION,
            game_device_ptr: 0x1000,
            manager_getter_rva: A729_DLC_MANAGER_GETTER_RVA,
            active_dlc_enumerator_rva: A729_ACTIVE_DLC_ENUMERATOR_RVA,
            string_vector_teardown_rva: A729_STRING_VECTOR_TEARDOWN_RVA,
            sample_count: 2,
            double_stable: true,
            roster_count: raw_names.len() as u32,
            raw_name_roster_sha256: canonical_runtime_dlc_roster_sha256(&raw_names),
            raw_names,
        }
    }

    #[test]
    fn exact_round_trip_and_name_hash_tamper_rejection() {
        let value = fixture();
        let bytes = encode_early_render_runtime_dlc_roster(&value).unwrap();
        assert_eq!(
            decode_early_render_runtime_dlc_roster(&bytes).unwrap(),
            value
        );
        let mut tampered = bytes;
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(
            decode_early_render_runtime_dlc_roster(&tampered),
            Err(EarlyRenderRuntimeDlcCodecError::InvalidField)
        );
    }

    #[test]
    fn empty_roster_is_valid_but_duplicate_names_fail_closed() {
        let mut empty = fixture();
        empty.raw_names.clear();
        empty.roster_count = 0;
        empty.raw_name_roster_sha256 = canonical_runtime_dlc_roster_sha256(&empty.raw_names);
        let bytes = encode_early_render_runtime_dlc_roster(&empty).unwrap();
        assert_eq!(
            decode_early_render_runtime_dlc_roster(&bytes).unwrap(),
            empty
        );

        let mut duplicate = fixture();
        duplicate.raw_names.push(duplicate.raw_names[0].clone());
        duplicate.roster_count = duplicate.raw_names.len() as u32;
        duplicate.raw_name_roster_sha256 =
            canonical_runtime_dlc_roster_sha256(&duplicate.raw_names);
        assert_eq!(
            encode_early_render_runtime_dlc_roster(&duplicate),
            Err(EarlyRenderRuntimeDlcCodecError::InvalidField)
        );
    }
}
