use crate::{hex_sha256, PromotionError};
use portable_render_stream::{
    OccupationSecondaryUploadReceipt, PoliticalTextureFormat, PoliticalTextureKind,
    PoliticalTexturePacket, SelectedPresentReceipt, A729_EXECUTABLE_SHA256,
    OCCUPATION_SECONDARY_UPLOAD_RECEIPT_SCHEMA, SNOW_MUD_UPLOAD_INSTRUCTIONS_SHA256,
    SNOW_MUD_UPLOAD_TARGET_RVA,
};

pub const OCCUPATION_SECONDARY_TEXTURE_SCHEMA: &str =
    "hoi4.a729.occupation_secondary_upload.v1";
pub use portable_render_stream::{
    OCCUPATION_SECONDARY_ALPHA, OCCUPATION_SECONDARY_BYTES_PER_TEXEL,
    OCCUPATION_SECONDARY_BYTE_LENGTH, OCCUPATION_SECONDARY_COVERAGE_ALPHAS,
    OCCUPATION_SECONDARY_HEIGHT, OCCUPATION_SECONDARY_ROW_PITCH,
    OCCUPATION_SECONDARY_TARGET_BINDING, OCCUPATION_SECONDARY_WIDTH,
};

/// Exact final CPU raster worker proven in
/// `G3_OCCUPATION_PRODUCER_RE_20260720`. The live hook remains on the shared
/// upload helper; this identity records the semantic producer law and is not a
/// claim that the hook intercepted this function directly.
pub const OCCUPATION_SECONDARY_RASTER_WORKER_RVA: u32 = 0x019c_8a10;
pub const OCCUPATION_SECONDARY_RASTER_WORKER_DECOMPILE_SHA256: &str =
    "8915c1e7db348c9ed057dc2d1014ff472288c199f534ae7a9429625b895f996b";

/// Pointer-bearing candidate copied at the already-authenticated generic
/// texture upload helper immediately before UpdateSubresource. It remains
/// extractor-local until a selected-submit resource snapshot and reflected
/// `ProvinceSecondaryColorMap` consumer join both succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawOccupationSecondaryTextureObservation {
    pub schema: String,
    pub executable_sha256: String,
    pub raster_worker_rva: u32,
    pub raster_worker_decompile_sha256: String,
    pub upload_target_rva: u32,
    pub upload_instructions_sha256: String,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub upload_order: u32,
    pub producer_game_hour_raw: i32,
    pub producer_qpc_ticks: u64,
    pub producer_qpc_frequency_hz: u64,
    /// Process-local join key. It is discarded before portable serialization.
    pub resource_ptr: usize,
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub bytes_per_texel: u32,
    pub bytes: Vec<u8>,
    pub payload_sha256: String,
}

impl RawOccupationSecondaryTextureObservation {
    pub fn has_exact_nontransparent_alpha_law(&self) -> bool {
        let mut nontransparent = false;
        for texel in self.bytes.chunks_exact(OCCUPATION_SECONDARY_BYTES_PER_TEXEL as usize) {
            if !OCCUPATION_SECONDARY_COVERAGE_ALPHAS.contains(&texel[3]) {
                return false;
            }
            if texel[3] != 0 {
                nontransparent = true;
            }
        }
        nontransparent
            && self.bytes.len() == OCCUPATION_SECONDARY_BYTE_LENGTH
            && self
                .bytes
                .chunks_exact(OCCUPATION_SECONDARY_BYTES_PER_TEXEL as usize)
                .remainder()
                .is_empty()
    }
}

/// Promote only after the collector has independently proven the exact
/// resource bytes and the reflected terrain-t6/water-t3 consumer topology.
pub fn promote_occupation_secondary_texture_observation(
    raw: &RawOccupationSecondaryTextureObservation,
    selected_present: &SelectedPresentReceipt,
    selected_present_qpc_frequency_hz: u64,
    expected_process_start_nonce: u64,
    expected_hook_epoch_nonce: u64,
    terrain_t6_consumer_count: u32,
    water_t3_consumer_count: u32,
) -> Result<PoliticalTexturePacket, PromotionError> {
    if raw.schema != OCCUPATION_SECONDARY_TEXTURE_SCHEMA
        || raw.executable_sha256 != A729_EXECUTABLE_SHA256
        || raw.raster_worker_rva != OCCUPATION_SECONDARY_RASTER_WORKER_RVA
        || raw.raster_worker_decompile_sha256
            != OCCUPATION_SECONDARY_RASTER_WORKER_DECOMPILE_SHA256
        || raw.upload_target_rva != SNOW_MUD_UPLOAD_TARGET_RVA
        || raw.upload_instructions_sha256 != SNOW_MUD_UPLOAD_INSTRUCTIONS_SHA256
    {
        return Err(PromotionError::Invalid(
            "occupation secondary upload has the wrong a729/static identity".into(),
        ));
    }
    if expected_process_start_nonce == 0
        || expected_hook_epoch_nonce == 0
        || raw.process_start_nonce != expected_process_start_nonce
        || raw.hook_epoch_nonce != expected_hook_epoch_nonce
        || raw.upload_order == 0
        || raw.resource_ptr == 0
        || raw.producer_game_hour_raw > selected_present.selected_game_hour_raw
        || raw.producer_qpc_ticks == 0
        || raw.producer_qpc_ticks > selected_present.selected_qpc_ticks
        || raw.producer_qpc_frequency_hz == 0
        || raw.producer_qpc_frequency_hz != selected_present_qpc_frequency_hz
        || selected_present.candidate_count == 0
        || selected_present.selected_ordinal.checked_add(1)
            != Some(selected_present.candidate_count)
        || selected_present.selected_qpc_ticks == 0
        || selected_present.selected_qpc_ticks > selected_present.selection_cutoff_qpc_ticks
    {
        return Err(PromotionError::Invalid(
            "occupation secondary upload has a foreign or inverted selected-Present join".into(),
        ));
    }
    if raw.width != OCCUPATION_SECONDARY_WIDTH
        || raw.height != OCCUPATION_SECONDARY_HEIGHT
        || raw.row_pitch != OCCUPATION_SECONDARY_ROW_PITCH
        || raw.bytes_per_texel != OCCUPATION_SECONDARY_BYTES_PER_TEXEL
        || raw.bytes.len() != OCCUPATION_SECONDARY_BYTE_LENGTH
        || raw.payload_sha256 != hex_sha256(&raw.bytes)
        || !raw.has_exact_nontransparent_alpha_law()
    {
        return Err(PromotionError::Invalid(
            "occupation secondary upload descriptor, payload, or alpha law is not exact".into(),
        ));
    }
    if terrain_t6_consumer_count == 0 || water_t3_consumer_count == 0 {
        return Err(PromotionError::Invalid(
            "occupation secondary upload lacks reflected terrain-t6/water-t3 consumers".into(),
        ));
    }

    let mut occupation_observation = OccupationSecondaryUploadReceipt {
        schema: OCCUPATION_SECONDARY_UPLOAD_RECEIPT_SCHEMA.into(),
        executable_sha256: raw.executable_sha256.clone(),
        raster_worker_rva: raw.raster_worker_rva,
        raster_worker_decompile_sha256: raw.raster_worker_decompile_sha256.clone(),
        upload_target_rva: raw.upload_target_rva,
        upload_instructions_sha256: raw.upload_instructions_sha256.clone(),
        process_start_nonce: raw.process_start_nonce,
        hook_epoch_nonce: raw.hook_epoch_nonce,
        upload_order: raw.upload_order,
        producer_game_hour_raw: raw.producer_game_hour_raw,
        producer_qpc_ticks: raw.producer_qpc_ticks,
        producer_qpc_frequency_hz: raw.producer_qpc_frequency_hz,
        selected_present_sha256: selected_present
            .computed_sha256()
            .map_err(PromotionError::Invalid)?,
        payload_sha256: raw.payload_sha256.clone(),
        receipt_sha256: String::new(),
    };
    occupation_observation.receipt_sha256 = occupation_observation
        .computed_receipt_sha256()
        .map_err(PromotionError::Invalid)?;
    let packet = PoliticalTexturePacket {
        kind: PoliticalTextureKind::Secondary,
        target_binding_identity: OCCUPATION_SECONDARY_TARGET_BINDING.into(),
        upload_order: raw.upload_order,
        width: raw.width,
        height: raw.height,
        row_pitch: raw.row_pitch,
        format: PoliticalTextureFormat::B8g8r8a8UnormSrgb,
        bytes: raw.bytes.clone(),
        sha256: raw.payload_sha256.clone(),
        occupation_observation: Some(occupation_observation),
    };
    packet
        .occupation_observation
        .as_ref()
        .expect("occupation observation was just constructed")
        .validate_against(&packet, selected_present)
        .map_err(PromotionError::Invalid)?;
    Ok(packet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(bytes: Vec<u8>) -> RawOccupationSecondaryTextureObservation {
        RawOccupationSecondaryTextureObservation {
            schema: OCCUPATION_SECONDARY_TEXTURE_SCHEMA.into(),
            executable_sha256: A729_EXECUTABLE_SHA256.into(),
            raster_worker_rva: OCCUPATION_SECONDARY_RASTER_WORKER_RVA,
            raster_worker_decompile_sha256:
                OCCUPATION_SECONDARY_RASTER_WORKER_DECOMPILE_SHA256.into(),
            upload_target_rva: SNOW_MUD_UPLOAD_TARGET_RVA,
            upload_instructions_sha256: SNOW_MUD_UPLOAD_INSTRUCTIONS_SHA256.into(),
            process_start_nonce: 1,
            hook_epoch_nonce: 1,
            upload_order: 1,
            producer_game_hour_raw: 1,
            producer_qpc_ticks: 1,
            producer_qpc_frequency_hz: 1,
            resource_ptr: 1,
            width: OCCUPATION_SECONDARY_WIDTH,
            height: OCCUPATION_SECONDARY_HEIGHT,
            row_pitch: OCCUPATION_SECONDARY_ROW_PITCH,
            bytes_per_texel: OCCUPATION_SECONDARY_BYTES_PER_TEXEL,
            payload_sha256: hex_sha256(&bytes),
            bytes,
        }
    }

    #[test]
    fn exact_two_by_two_coverage_alpha_levels_are_admitted() {
        let mut bytes = vec![0; OCCUPATION_SECONDARY_BYTE_LENGTH];
        for (index, alpha) in OCCUPATION_SECONDARY_COVERAGE_ALPHAS
            .iter()
            .copied()
            .enumerate()
        {
            bytes[index * OCCUPATION_SECONDARY_BYTES_PER_TEXEL as usize + 3] = alpha;
        }
        let mut raw = observation(bytes);
        assert!(raw.has_exact_nontransparent_alpha_law());

        raw.bytes[3] = 75;
        assert!(!raw.has_exact_nontransparent_alpha_law());
    }
}
