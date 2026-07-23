//! Receipt-bound production envelope for A36 compact domain 88.
//!
//! `ABL1` remains the pointer-free selected-context row codec. `ABA2` binds an
//! admitted `ABL1` frame to the exact-a729 executable and exact20 evidence
//! receipt, and gives the compact fold an explicit unavailable representation.
//! Core rows are admitted from the exact refresh/state observer. Portable
//! final text remains optional and is absent when no exact text transaction
//! was observed.

use crate::{
    decode_payload as decode_abl1, encode_payload as encode_abl1, validate_frame as validate_abl1,
    AbilityError, AbilityFrame, ExactA729AbilityControllerSource, COMPACT_DOMAIN_SCHEDULED,
    EXACT_A729_AUTHORITY_BOUND, LIVE_TRANSITION_PROMOTION_BOUND,
};

pub const DOMAIN_ID: u16 = 88;
pub const DOMAIN_NAME: &str = "active_command_abilities_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"ABA2");
pub const SCHEMA_VERSION: u32 = 1;
pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const EXACT20_FUNCTION_MANIFEST_SHA256: [u8; 32] =
    hex32(b"12768dfa9a8b47ceda4feae9cdc45b76811f602ecd557008aa936e95adf2db45");
pub const ABILITY_REFRESH_DECOMPILE_SHA256: [u8; 32] =
    hex32(b"aa4eb90663e60c74b031d83b41c7a28a1867245611a05a6c9d4b1856ae0076f7");
pub const MAX_NESTED_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

pub const FIXED_HEADER_LEN: usize = 136;

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut out = [0; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (nibble(input[i * 2]) << 4) | nibble(input[i * 2 + 1]);
        i += 1;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityState {
    Unavailable,
    Exact20Receipt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionAbilityFrame {
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub status_flags: u32,
    pub authority: AuthorityState,
    pub snapshot: Option<AbilityFrame>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    ActivationDisabled,
    UnauthenticatedSource,
    Source(String),
    Ability(AbilityError),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}

fn contains_pointer_literal(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.windows(10).any(|window| {
        window[0] == b'0'
            && matches!(window[1], b'x' | b'X')
            && window[2..].iter().all(u8::is_ascii_hexdigit)
    })
}

fn valid_portable_identity(value: &str) -> bool {
    !value.is_empty()
        && !contains_pointer_literal(value)
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'.' | b'-' | b':' | b'/' | b'@')
        })
}

fn context_kind_code(kind: crate::SelectedContextKind) -> u32 {
    match kind {
        crate::SelectedContextKind::ArmyLeader => 1,
        crate::SelectedContextKind::FieldMarshal => 2,
        crate::SelectedContextKind::ArmyGroup => 3,
        crate::SelectedContextKind::Army => 4,
        crate::SelectedContextKind::Other(value) => value | 0x8000_0000,
    }
}

pub fn validate_frame(frame: &ProductionAbilityFrame) -> Result<(), CarrierError> {
    match frame.authority {
        AuthorityState::Unavailable
            if compact_contract::compact_status_unknown_flags_v1(frame.status_flags) == 0
                && compact_contract::compact_status_is_failure(frame.status_flags)
                && frame.snapshot.is_none() =>
        {
            Ok(())
        }
        AuthorityState::Exact20Receipt
            if frame.status_flags == 0 && frame.snapshot.is_some() =>
        {
            let snapshot = frame.snapshot.as_ref().expect("checked snapshot");
            if snapshot.elapsed_wall_ms != frame.elapsed_wall_ms
                || snapshot.frame_hour != frame.frame_hour
                || snapshot.status_flags != 0
            {
                return invalid("ABA2 outer/ABL1 frame identity mismatch");
            }
            validate_abl1(snapshot).map_err(CarrierError::Ability)?;
            if snapshot.country_tag.len() != 3
                || !snapshot
                    .country_tag
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
            {
                return invalid("ABA2 production country tag is not portable");
            }
            if !valid_portable_identity(&snapshot.selected_context_id) {
                return invalid("ABA2 selected-context identity is not portable");
            }
            let expected_record_id = format!(
                "a729:a36:{}:{}:{}:{}",
                snapshot.frame_hour,
                snapshot.country_tag,
                context_kind_code(snapshot.selected_context_kind),
                snapshot.selected_context_id,
            );
            if snapshot.source_record_id != expected_record_id {
                return invalid("ABA2 source-record identity disagrees with frame context");
            }
            for row in &snapshot.rows {
                if row.target_kind.is_none() || row.target_id.is_none() {
                    return invalid("ABA2 production row lacks portable target identity");
                }
                if !valid_portable_identity(&row.ability_id)
                    || !valid_portable_identity(&row.icon)
                    || !row
                        .target_kind
                        .as_deref()
                        .is_some_and(valid_portable_identity)
                    || !row
                        .target_id
                        .as_deref()
                        .is_some_and(valid_portable_identity)
                    || row
                        .affected_division_ids
                        .iter()
                        .any(|identity| !valid_portable_identity(identity))
                {
                    return invalid("ABA2 production row contains a nonportable identity");
                }
                // The authenticated refresh resolves CAbility+0x2A8 for an
                // active row. 0x140FF44E0 and 0x140FF5B20 construct that key as
                // the final ordinary icon plus `_animated`; admitting an
                // ordinary icon here would contradict the exact UI law before
                // the replay definition join can reject it.
                if row.active && !row.icon.ends_with("_animated") {
                    return invalid("ABA2 active row lacks the exact animated icon key");
                }
            }
            Ok(())
        }
        _ => invalid("ABA2 authority/availability mismatch"),
    }
}

pub fn collect_and_encode<S: ExactA729AbilityControllerSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, CarrierError> {
    if !EXACT_A729_AUTHORITY_BOUND
        || !LIVE_TRANSITION_PROMOTION_BOUND
        || !COMPACT_DOMAIN_SCHEDULED
    {
        return Err(CarrierError::ActivationDisabled);
    }
    if !source.is_exact_a729_ability_controller() {
        return Err(CarrierError::UnauthenticatedSource);
    }
    let mut snapshot = source
        .collect_frame(frame_hour)
        .map_err(CarrierError::Source)?;
    if snapshot.frame_hour != frame_hour {
        return invalid("ABA2 source returned the wrong frame hour");
    }
    snapshot.elapsed_wall_ms = elapsed_wall_ms;
    snapshot.status_flags = 0;
    encode_payload(&ProductionAbilityFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags: 0,
        authority: AuthorityState::Exact20Receipt,
        snapshot: Some(snapshot),
    })
}

pub fn encode_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    encode_payload(&ProductionAbilityFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority: AuthorityState::Unavailable,
        snapshot: None,
    })
}

pub fn encode_payload(frame: &ProductionAbilityFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let nested = frame
        .snapshot
        .as_ref()
        .map(encode_abl1)
        .transpose()
        .map_err(CarrierError::Ability)?
        .unwrap_or_default();
    if nested.len() > MAX_NESTED_PAYLOAD_BYTES {
        return invalid("ABA2 nested ABL1 payload exceeds bound");
    }
    let nested_len = u32::try_from(nested.len())
        .map_err(|_| CarrierError::Invalid("ABA2 nested length overflow".into()))?;
    let mut out = Vec::with_capacity(FIXED_HEADER_LEN + nested.len());
    put_u32(&mut out, MAGIC);
    put_u32(&mut out, SCHEMA_VERSION);
    put_u32(
        &mut out,
        match frame.authority {
            AuthorityState::Unavailable => 0,
            AuthorityState::Exact20Receipt => 1,
        },
    );
    put_u32(&mut out, frame.status_flags);
    put_u64(&mut out, frame.elapsed_wall_ms);
    put_u64(&mut out, frame.frame_hour);
    out.extend_from_slice(&EXECUTABLE_SHA256);
    out.extend_from_slice(&EXACT20_FUNCTION_MANIFEST_SHA256);
    out.extend_from_slice(&ABILITY_REFRESH_DECOMPILE_SHA256);
    put_u32(&mut out, nested_len);
    put_u32(&mut out, 0);
    out.extend_from_slice(&nested);
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<ProductionAbilityFrame, CarrierError> {
    let mut reader = Reader::new(payload);
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("ABA2 magic/schema mismatch");
    }
    let authority = match reader.u32()? {
        0 => AuthorityState::Unavailable,
        1 => AuthorityState::Exact20Receipt,
        _ => return invalid("ABA2 authority is unknown"),
    };
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    if reader.take(32)? != EXECUTABLE_SHA256.as_slice()
        || reader.take(32)? != EXACT20_FUNCTION_MANIFEST_SHA256.as_slice()
        || reader.take(32)? != ABILITY_REFRESH_DECOMPILE_SHA256.as_slice()
    {
        return invalid("ABA2 receipt identity mismatch");
    }
    let nested_len = reader.u32()? as usize;
    if nested_len > MAX_NESTED_PAYLOAD_BYTES || reader.u32()? != 0 {
        return invalid("ABA2 nested length/reserved field invalid");
    }
    let snapshot = if nested_len == 0 {
        None
    } else {
        Some(decode_abl1(reader.take(nested_len)?).map_err(CarrierError::Ability)?)
    };
    if !reader.done() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = ProductionAbilityFrame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority,
        snapshot,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

struct Reader<'a> {
    payload: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(payload: &'a [u8]) -> Self {
        Self { payload, offset: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], CarrierError> {
        let end = self.offset.checked_add(len).ok_or(CarrierError::Truncated)?;
        let bytes = self
            .payload
            .get(self.offset..end)
            .ok_or(CarrierError::Truncated)?;
        self.offset = end;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("fixed width")))
    }

    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("fixed width")))
    }

    fn done(&self) -> bool {
        self.offset == self.payload.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AbilityRow, SelectedContextKind, TimerValue};

    fn snapshot() -> AbilityFrame {
        AbilityFrame {
            elapsed_wall_ms: 7,
            status_flags: 0,
            frame_hour: 900,
            country_tag: "GER".into(),
            selected_context_kind: SelectedContextKind::ArmyLeader,
            selected_context_id: "leader:42".into(),
            source_record_id: "a729:a36:900:GER:1:leader:42".into(),
            complete: true,
            rows: vec![AbilityRow {
                ability_id: "force_attack".into(),
                available: false,
                active: true,
                activation_hour: Some(897),
                cooldown_total: TimerValue::Hours(504),
                cooldown_remaining: TimerValue::NotStarted,
                cost_command_power_fixed: Some(2_200_000),
                target_kind: Some("army_leader".into()),
                target_id: Some("leader:42".into()),
                duration_total: TimerValue::Hours(168),
                duration_remaining: TimerValue::Hours(165),
                display_order: 0,
                icon: "GFX_ability_force_attack_animated".into(),
                tooltip: Some("Force Attack".into()),
                disabled_reason: Some("Already active".into()),
                affected_division_ids: vec!["division:9".into()],
            }],
        }
    }

    fn frame() -> ProductionAbilityFrame {
        ProductionAbilityFrame {
            elapsed_wall_ms: 7,
            frame_hour: 900,
            status_flags: 0,
            authority: AuthorityState::Exact20Receipt,
            snapshot: Some(snapshot()),
        }
    }

    #[test]
    fn exact_envelope_round_trips_receipt_bound_abl1() {
        let expected = frame();
        let encoded = encode_payload(&expected).expect("encode ABA2");
        assert!(encoded.len() >= FIXED_HEADER_LEN);
        assert_eq!(decode_payload(&encoded).expect("decode ABA2"), expected);
    }

    #[test]
    fn unavailable_is_explicit_and_never_an_empty_measurement() {
        let encoded = encode_unavailable(901, 8, 1 << 9).expect("encode unavailable");
        let decoded = decode_payload(&encoded).expect("decode unavailable");
        assert_eq!(decoded.authority, AuthorityState::Unavailable);
        assert_eq!(decoded.snapshot, None);
        assert_eq!(decoded.status_flags, 1 << 9);
    }

    #[test]
    fn unavailable_rejects_unknown_or_informational_only_status() {
        assert!(encode_unavailable(901, 8, 1 << 31).is_err());
        assert!(encode_unavailable(
            901,
            8,
            compact_contract::COMPACT_STATUS_DELTA_DATA,
        )
        .is_err());
    }

    #[test]
    fn receipt_or_outer_inner_identity_tampering_rejects() {
        let mut receipt = encode_payload(&frame()).expect("encode ABA2");
        receipt[32] ^= 0x80;
        assert!(decode_payload(&receipt).is_err());

        let mut mismatched = frame();
        mismatched.snapshot.as_mut().expect("snapshot").frame_hour += 1;
        assert!(encode_payload(&mismatched).is_err());
    }

    #[test]
    fn runtime_pointer_shaped_identity_or_forged_record_id_rejects() {
        let mut pointer = frame();
        pointer
            .snapshot
            .as_mut()
            .expect("snapshot")
            .rows[0]
            .target_id = Some("leader:0x141D072F0".into());
        assert!(encode_payload(&pointer).is_err());

        let mut forged = frame();
        forged
            .snapshot
            .as_mut()
            .expect("snapshot")
            .source_record_id = "a729:a36:900:GER:1:leader:99".into();
        assert!(encode_payload(&forged).is_err());
    }

    #[test]
    fn active_row_with_ordinary_definition_icon_rejects() {
        let mut ordinary_icon = frame();
        ordinary_icon
            .snapshot
            .as_mut()
            .expect("snapshot")
            .rows[0]
            .icon = "GFX_ability_force_attack".into();
        assert!(encode_payload(&ordinary_icon).is_err());
    }

    #[test]
    fn signed_effective_cost_round_trips_without_clamping() {
        let mut signed = frame();
        signed
            .snapshot
            .as_mut()
            .expect("snapshot")
            .rows[0]
            .cost_command_power_fixed = Some(-2_200_000);
        let encoded = encode_payload(&signed).expect("encode signed effective cost");
        assert_eq!(decode_payload(&encoded).expect("decode signed effective cost"), signed);
    }

    struct Fake;
    impl crate::sealed::Sealed for Fake {}
    impl ExactA729AbilityControllerSource for Fake {
        fn is_exact_a729_ability_controller(&self) -> bool {
            true
        }

        fn collect_frame(&self, _frame_hour: u64) -> Result<AbilityFrame, String> {
            Ok(snapshot())
        }
    }

    #[test]
    fn production_collection_encodes_authenticated_promoted_source() {
        let encoded = collect_and_encode(&Fake, 900, 7)
            .expect("encode authenticated promoted A36 source");
        let decoded = decode_payload(&encoded).expect("decode promoted ABA2 frame");
        assert_eq!(decoded.authority, AuthorityState::Exact20Receipt);
        assert_eq!(decoded.snapshot, Some(snapshot()));
    }
}
