use crate::{hex_sha256, PromotionError};
use map_passive_carriers_live_lane::xmap07::{
    cross_check_reflected_fade, validate_snow_mud_fade_sample,
};
use portable_render_stream::{
    SelectedPresentReceipt, SnowMudFadeReceipt, SnowMudTexturePacket, A729_EXECUTABLE_SHA256,
    SNOW_MUD_PRODUCER_FUNCTION_RVA,
    SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256, SNOW_MUD_TEXTURE_BYTES_PER_TEXEL,
    SNOW_MUD_TEXTURE_BYTE_LENGTH, SNOW_MUD_TEXTURE_HEIGHT, SNOW_MUD_TEXTURE_ROW_PITCH,
    SNOW_MUD_TEXTURE_SCHEMA, SNOW_MUD_TEXTURE_WIDTH, SNOW_MUD_UPLOAD_CALLSITE_RVA,
    SNOW_MUD_UPLOAD_INSTRUCTIONS_SHA256, SNOW_MUD_UPLOAD_TARGET_RVA,
};

/// Pointer-free bytes copied at the a729 upload callee before the original
/// UpdateSubresource dispatch. The owner hook authenticates that this call is
/// upload helper are never admitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawSnowMudTextureObservation {
    pub executable_sha256: String,
    pub producer_function_rva: u32,
    pub upload_callsite_rva: u32,
    pub upload_target_rva: u32,
    pub producer_instructions_sha256: String,
    pub upload_instructions_sha256: String,
    pub process_start_nonce: u64,
    pub hook_epoch_nonce: u64,
    pub producer_ordinal: u64,
    pub producer_game_hour_raw: i32,
    pub producer_qpc_ticks: u64,
    pub producer_qpc_frequency_hz: u64,
    /// Process-local join key only. It identifies selected-submit SRV
    /// consumers whose exact draw-time snapshot must then equal these H bytes;
    /// the pointer is never serialized into H.
    pub resource_ptr: usize,
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub bytes_per_texel: u32,
    pub format: String,
    pub bytes: Vec<u8>,
    pub payload_sha256: String,
    /// The XMAP07 fade word, read twice and bracketed by a five-word chain
    /// witness on each side, COPIED inside the same displayed-hour/QPC bracket
    /// as `bytes`.
    ///
    /// This is the source the XMAP07 evidence requires, and it is copied here,
    /// beside the texture, rather than read at promotion time, because the
    /// global owner must use the same displayed-hour cutoff as the texture — a later read would be a
    /// different cutoff. Nothing is validated at copy time; the witness is what
    /// lets `promote_snow_mud_texture_observation` prove, after the bracket,
    /// that the anchor discovery resolved was still the anchor.
    pub passive_fade_sample: map_passive_carriers_live_lane::xmap07::SnowMudFadeSample,
    /// The anchor discovery resolved once, carried so that promotion can
    /// validate the witness **without touching live memory**.
    pub passive_anchor: map_passive_carriers_live_lane::anchor::MapRenderAnchor,
    /// The module base the anchor was resolved against, for the vptr identity.
    pub passive_module_base: u64,
    /// The anchor-cache generation this observation was copied against.
    ///
    /// Observations are queued and promoted later, so "the anchor moved" is a
    /// statement about the anchor THIS observation used, not about whatever is
    /// cached by the time it is promoted. Carrying the generation is what lets
    /// the invalidation advance exactly once per anchor, by whoever actually
    /// observed it, instead of a second delayed frame knocking out the correct
    /// replacement that was published in the meantime.
    pub passive_anchor_generation: u64,
}

/// Seals one authenticated live observation into the current portable render
/// carrier. The outer schema version advances independently as new optional
/// exact families are added; this SnowMud payload keeps its own v3 identity.
/// The display envelope is supplied by the selected Present and all
/// epoch/hour/QPC joins are checked here before any process-local state is
/// discarded.
///
/// # The fade source
///
/// The recorder reads the global owner at the same displayed-hour cutoff as the
/// texture, never from a reflected draw submit. The previous path did the
/// opposite — `fade_receipt.fade_f32_bits` is produced by
/// `selected_present_snow_mud_fade_receipt`, which reads `SnowMudFade` out of
/// each selected-Present draw's REFLECTED constant buffer. A source census for
/// `0x5ce8` found that the passive read did not exist.
///
/// The observation now carries a passive fade sampled inside the texture's own
/// bracket (`RawSnowMudTextureObservation::passive_fade_sample`), and that is
/// the value sealed. The reflected receipt is demoted to a cross-check: it can
/// only agree or refuse, and there is no path on which its bits become the
/// packet's. A promotion with no passive anchor refuses, where before it would
/// have sealed the reflected value alone.
pub fn promote_snow_mud_texture_observation(
    raw: &RawSnowMudTextureObservation,
    selected_present: &SelectedPresentReceipt,
    selected_present_qpc_frequency_hz: u64,
    expected_process_start_nonce: u64,
    expected_hook_epoch_nonce: u64,
    fade_receipt: &SnowMudFadeReceipt,
) -> Result<SnowMudTexturePacket, PromotionError> {
    // POST-BRACKET validation of the frozen sample. No live memory is touched
    // here: the witness words and the discovered anchor both travelled on the
    // observation, and this compares them. A chain that moved between
    // discovery and the copy refuses this frame's SnowMud carrier by name.
    let passive_fade = &validate_snow_mud_fade_sample(
        &raw.passive_fade_sample,
        &raw.passive_anchor,
        raw.passive_module_base,
    )
    .map_err(|refusal| {
        // The witness mismatch is the PROVEN change signal for the anchor
        // cache: the chain the recorder discovered is no longer the chain, so
        // the cached anchor must be rediscovered before the next capture.
        // Without this, a map object permanently replaced mid-process would
        // refuse this hour and then every later one, violating `PROJECT.md` —
        // “Loss and coverage”. Rediscovery itself happens on
        // the next map Present, never here and never in a bracket.
        if refusal.reason == map_passive_carriers_live_lane::xmap07::ANCHOR_CHANGED_REFUSAL {
            crate::a729_hook::invalidate_map_render_anchor(raw.passive_anchor_generation);
        }
        PromotionError::Invalid(refusal.to_string())
    })?;
    let identity_matches = raw.executable_sha256 == A729_EXECUTABLE_SHA256
        && raw.producer_function_rva == SNOW_MUD_PRODUCER_FUNCTION_RVA
        && raw.upload_callsite_rva == SNOW_MUD_UPLOAD_CALLSITE_RVA
        && raw.upload_target_rva == SNOW_MUD_UPLOAD_TARGET_RVA
        && raw.producer_instructions_sha256 == SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256
        && raw.upload_instructions_sha256 == SNOW_MUD_UPLOAD_INSTRUCTIONS_SHA256;
    if !identity_matches {
        return Err(PromotionError::Invalid(
            "SnowMudData live observation has the wrong a729/static identity".into(),
        ));
    }
    if expected_process_start_nonce == 0
        || expected_hook_epoch_nonce == 0
        || raw.process_start_nonce != expected_process_start_nonce
        || raw.hook_epoch_nonce != expected_hook_epoch_nonce
        || raw.producer_ordinal == 0
        || raw.resource_ptr == 0
        || raw.producer_game_hour_raw > selected_present.selected_game_hour_raw
        || raw.producer_qpc_ticks == 0
        || raw.producer_qpc_ticks > selected_present.selected_qpc_ticks
        || raw.producer_qpc_frequency_hz == 0
        || raw.producer_qpc_frequency_hz != selected_present_qpc_frequency_hz
        || selected_present.candidate_count == 0
        || selected_present.selected_ordinal.checked_add(1)
            != Some(selected_present.candidate_count)
        || selected_present.candidate_set_sha256.len() != 64
        || !selected_present
            .candidate_set_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || selected_present.selected_qpc_ticks == 0
        || selected_present.selected_qpc_ticks > selected_present.selection_cutoff_qpc_ticks
    {
        return Err(PromotionError::Invalid(
            "SnowMudData live observation has a foreign or inverted epoch/hour/order join".into(),
        ));
    }
    if raw.width != SNOW_MUD_TEXTURE_WIDTH
        || raw.height != SNOW_MUD_TEXTURE_HEIGHT
        || raw.row_pitch != SNOW_MUD_TEXTURE_ROW_PITCH
        || raw.bytes_per_texel != SNOW_MUD_TEXTURE_BYTES_PER_TEXEL
        || raw.format != "DXGI_FORMAT_B8G8R8A8_UNORM"
        || raw.bytes.len() != SNOW_MUD_TEXTURE_BYTE_LENGTH
    {
        return Err(PromotionError::Invalid(
            "SnowMudData live observation descriptor/byte count is not exact".into(),
        ));
    }
    if raw.payload_sha256 != hex_sha256(&raw.bytes) {
        return Err(PromotionError::Invalid(
            "SnowMudData live observation payload SHA-256 mismatch".into(),
        ));
    }

    if fade_receipt.consumer_count == 0
        || fade_receipt.consumer_set_sha256.len() != 64
        || !fade_receipt
            .consumer_set_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !f32::from_bits(fade_receipt.fade_f32_bits).is_finite()
        || !(0.0..=1.0).contains(&f32::from_bits(fade_receipt.fade_f32_bits))
    {
        return Err(PromotionError::Invalid(
            "SnowMudData selected-Present fade receipt is incomplete".into(),
        ));
    }

    // The passive read at `map_render_state + 0x5ce8` is the source. The
    // reflected consumer set is downstream of that same word, so a
    // disagreement means either the recorder sampled a different displayed
    // hour than the frame it is sealing or a consumer was mis-bound; both are
    // named refusals, and neither substitutes the reflected value.
    if passive_fade.map_render_state == 0 {
        return Err(PromotionError::Invalid(
            "SnowMudData passive fade carries no map_render_state anchor".into(),
        ));
    }
    cross_check_reflected_fade(
        passive_fade,
        fade_receipt.fade_f32_bits,
        fade_receipt.consumer_count,
    )
    .map_err(|refusal| PromotionError::Invalid(refusal.to_string()))?;

    let mut packet = SnowMudTexturePacket {
        schema: SNOW_MUD_TEXTURE_SCHEMA.into(),
        executable_sha256: raw.executable_sha256.clone(),
        producer_function_rva: raw.producer_function_rva,
        upload_callsite_rva: raw.upload_callsite_rva,
        upload_target_rva: raw.upload_target_rva,
        producer_instructions_sha256: raw.producer_instructions_sha256.clone(),
        upload_instructions_sha256: raw.upload_instructions_sha256.clone(),
        process_start_nonce: raw.process_start_nonce,
        hook_epoch_nonce: raw.hook_epoch_nonce,
        producer_ordinal: raw.producer_ordinal,
        producer_game_hour_raw: raw.producer_game_hour_raw,
        display_game_hour_raw: selected_present.selected_game_hour_raw,
        producer_qpc_ticks: raw.producer_qpc_ticks,
        producer_qpc_frequency_hz: raw.producer_qpc_frequency_hz,
        width: raw.width,
        height: raw.height,
        row_pitch: raw.row_pitch,
        bytes_per_texel: raw.bytes_per_texel,
        format: raw.format.clone(),
        bytes: raw.bytes.clone(),
        payload_sha256: raw.payload_sha256.clone(),
        selected_present_sha256: selected_present
            .computed_sha256()
            .map_err(PromotionError::Invalid)?,
        // The PASSIVE word, not the reflected one. They are equal by the
        // cross-check above, so this is the same bits; it is written from the
        // passive sample so that a future divergence lands here rather than
        // silently re-adopting the forbidden source.
        snow_mud_fade_f32_bits: passive_fade.fade_f32_bits,
        snow_mud_fade_consumer_count: fade_receipt.consumer_count,
        snow_mud_fade_consumer_set_sha256: fade_receipt.consumer_set_sha256.clone(),
        observation_sha256: String::new(),
    };
    packet.observation_sha256 = packet
        .computed_observation_sha256()
        .map_err(PromotionError::Invalid)?;
    Ok(packet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw() -> RawSnowMudTextureObservation {
        let bytes = vec![7; SNOW_MUD_TEXTURE_BYTE_LENGTH];
        RawSnowMudTextureObservation {
            executable_sha256: A729_EXECUTABLE_SHA256.into(),
            producer_function_rva: SNOW_MUD_PRODUCER_FUNCTION_RVA,
            upload_callsite_rva: SNOW_MUD_UPLOAD_CALLSITE_RVA,
            upload_target_rva: SNOW_MUD_UPLOAD_TARGET_RVA,
            producer_instructions_sha256: SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256.into(),
            upload_instructions_sha256: SNOW_MUD_UPLOAD_INSTRUCTIONS_SHA256.into(),
            process_start_nonce: 11,
            hook_epoch_nonce: 12,
            producer_ordinal: 13,
            producer_game_hour_raw: 44_000_000,
            producer_qpc_ticks: 100,
            producer_qpc_frequency_hz: 10_000_000,
            resource_ptr: 0x1234,
            width: SNOW_MUD_TEXTURE_WIDTH,
            height: SNOW_MUD_TEXTURE_HEIGHT,
            row_pitch: SNOW_MUD_TEXTURE_ROW_PITCH,
            bytes_per_texel: SNOW_MUD_TEXTURE_BYTES_PER_TEXEL,
            format: "DXGI_FORMAT_B8G8R8A8_UNORM".into(),
            payload_sha256: hex_sha256(&bytes),
            bytes,
            passive_fade_sample: fade_sample(0.25f32.to_bits()),
            passive_anchor: anchor(),
            passive_module_base: MODULE_BASE,
            passive_anchor_generation: 0,
        }
    }

    const MODULE_BASE: u64 = 0x1_4000_0000;

    fn anchor() -> map_passive_carriers_live_lane::anchor::MapRenderAnchor {
        map_passive_carriers_live_lane::anchor::MapRenderAnchor {
            game_root: 0x2_0000_0000,
            delegate: 0x2_0001_0000,
            map_object: 0x2_0002_0000,
            map_render_state: 0x2_0003_0000,
        }
    }

    /// A witness pair that matches [`anchor`] on both sides of the fade, so
    /// validation passes unless a test deliberately perturbs it.
    fn fade_sample(bits: u32) -> map_passive_carriers_live_lane::xmap07::SnowMudFadeSample {
        let anchor = anchor();
        let witness = map_passive_carriers_live_lane::xmap07::ChainWitness {
            root_slot: anchor.game_root,
            root_vptr: MODULE_BASE + 0x0295_2510,
            delegate: anchor.delegate,
            map_object: anchor.map_object,
            map_render_state: anchor.map_render_state,
        };
        map_passive_carriers_live_lane::xmap07::SnowMudFadeSample {
            witness_before: witness,
            fade_f32_bits_first: bits,
            fade_f32_bits_second: bits,
            witness_after: witness,
        }
    }

    fn selected_present() -> SelectedPresentReceipt {
        SelectedPresentReceipt::sealed(
            1,
            "a".repeat(64),
            21,
            0,
            44_000_001,
            101,
            102,
        )
    }

    fn fade_receipt() -> SnowMudFadeReceipt {
        SnowMudFadeReceipt {
            fade_f32_bits: 0.25f32.to_bits(),
            consumer_count: 2,
            consumer_set_sha256: "b".repeat(64),
        }
    }

    #[test]
    fn exact_observation_is_sealed() {
        let packet = promote_snow_mud_texture_observation(
            &raw(),
            &selected_present(),
            10_000_000,
            11,
            12,
            &fade_receipt(),
        )
        .unwrap();
        assert_eq!(packet.observation_sha256, packet.computed_observation_sha256().unwrap());
        assert_eq!(
            packet.snow_mud_fade_f32_bits,
            raw().passive_fade_sample.fade_f32_bits_first,
            "the sealed fade is the passive read at map_render_state+0x5ce8"
        );
    }

    /// kills: dropping the cross-check so a reflected/passive disagreement
    /// seals silently, and sealing a fade with no passive anchor at all.
    ///
    /// It does **not** kill "write `fade_receipt.fade_f32_bits` into the packet
    /// instead of the passive word": the cross-check above forces the two to be
    /// equal before the packet is built, so both assignments produce identical
    /// bytes and no test can separate them. The passive read being the source
    /// is enforced by there being nowhere else for the value to come from once
    /// the anchor requirement holds, not by this assertion.
    #[test]
    fn the_reflected_fade_can_only_cross_check_and_never_becomes_the_source() {
        let mut disagreeing = fade_receipt();
        disagreeing.fade_f32_bits = 0.5f32.to_bits();
        assert!(promote_snow_mud_texture_observation(
            &raw(),
            &selected_present(),
            10_000_000,
            11,
            12,
            &disagreeing,
        )
        .is_err());

        // A witness that no longer matches the discovered anchor: the chain
        // moved between discovery and the bracket copy.
        let mut unanchored = raw();
        unanchored.passive_fade_sample.witness_after.delegate ^= 0x1000;
        assert!(promote_snow_mud_texture_observation(
            &unanchored,
            &selected_present(),
            10_000_000,
            11,
            12,
            &fade_receipt(),
        )
        .is_err());
    }

    /// kills: dropping the anchor invalidation on the witness mismatch. Without
    /// it a map object replaced permanently mid-process refuses this hour and
    /// then every later one, because the cached anchor is never replaced — the
    /// recording paying for one bad hour.
    ///
    /// Also kills: invalidating on ANY promotion refusal. Rediscovery is
    /// allowed only on a proven change signal, so a merely torn fade word must
    /// leave the anchor alone.
    ///
    /// (The validation runner is cargo-nextest, which runs each test in its own
    /// process, so the generation counter here is this test's alone.)
    #[test]
    fn only_the_witness_mismatch_invalidates_the_cached_anchor() {
        let before = crate::a729_hook::map_render_anchor_generation();
        let mut unanchored = raw();
        unanchored.passive_anchor_generation = before;
        unanchored.passive_fade_sample.witness_before.map_render_state ^= 0x1000;
        assert!(promote_snow_mud_texture_observation(
            &unanchored,
            &selected_present(),
            10_000_000,
            11,
            12,
            &fade_receipt(),
        )
        .is_err());
        let after_signal = crate::a729_hook::map_render_anchor_generation();
        assert!(
            after_signal > before,
            "the chain-change signal must invalidate the cached anchor"
        );

        // A SECOND delayed frame carrying the SAME stale anchor. Observations
        // are queued, so this is ordinary, and it must not invalidate the
        // replacement the Present hook has published in the meantime: this
        // observation never saw that anchor. kills: the unconditional
        // `fetch_add`, under which two delayed frames could knock out one good
        // anchor after another.
        assert!(promote_snow_mud_texture_observation(
            &unanchored,
            &selected_present(),
            10_000_000,
            11,
            12,
            &fade_receipt(),
        )
        .is_err());
        assert_eq!(
            crate::a729_hook::map_render_anchor_generation(),
            after_signal,
            "an observation of an already-invalidated anchor must not invalidate its replacement"
        );

        // A torn fade word is not a chain change: same anchor, unusable value.
        let mut torn = raw();
        torn.passive_anchor_generation = after_signal;
        torn.passive_fade_sample.fade_f32_bits_first = 2.0f32.to_bits();
        torn.passive_fade_sample.fade_f32_bits_second = 2.0f32.to_bits();
        assert!(promote_snow_mud_texture_observation(
            &torn,
            &selected_present(),
            10_000_000,
            11,
            12,
            &fade_receipt(),
        )
        .is_err());
        assert_eq!(
            crate::a729_hook::map_render_anchor_generation(),
            after_signal,
            "a refusal that is not the chain-change signal must not force rediscovery"
        );
    }

    #[test]
    fn identity_payload_epoch_and_order_mismatches_fail_closed() {
        let mut cases = Vec::new();
        let mut wrong_identity = raw();
        wrong_identity.upload_target_rva ^= 1;
        cases.push(wrong_identity);
        let mut wrong_payload = raw();
        wrong_payload.bytes[0] ^= 1;
        cases.push(wrong_payload);
        let mut wrong_size = raw();
        wrong_size.bytes.pop();
        wrong_size.payload_sha256 = hex_sha256(&wrong_size.bytes);
        cases.push(wrong_size);
        let mut future = raw();
        future.producer_qpc_ticks = 102;
        cases.push(future);
        for value in cases {
            assert!(promote_snow_mud_texture_observation(
                &value,
                &selected_present(),
                10_000_000,
                11,
                12,
                &fade_receipt(),
            )
            .is_err());
        }
        assert!(promote_snow_mud_texture_observation(
            &raw(),
            &selected_present(),
            10_000_000,
            99,
            12,
            &fade_receipt(),
        )
        .is_err());
    }
}
