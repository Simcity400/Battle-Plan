//! A31 exact national-focus presentation transport (compact domain 89).
//!
//! The wire owns only pointer-free identities. Process addresses are accepted
//! solely by the lane-local a729 observer and must be replaced with stable
//! focus-definition ids before a [`Snapshot`] reaches this module.

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 89;
pub const DOMAIN_NAME: &str = "national_focus_presentation_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"NFP1");
pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_VISIBLE_ITEMS: usize = 131_072;
pub const MAX_TEXT_BYTES: usize = 65_536;
pub const EXECUTABLE_SHA256: [u8; 32] = hex32(
    b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
);
pub const EXACT20_FUNCTION_MANIFEST_SHA256: [u8; 32] = hex32(
    b"303351ed65468aa7655b904998819c96ca1c325f906b5a3bf32c4f18decc7969",
);
pub const EXACT20_JOB_RECEIPT_SHA256: [u8; 32] = hex32(
    b"c6c3fb002bc05f1e092d7eb9b436aa720ca2cf19ad08f055747216dfa5871b4f",
);

/// Static authority is complete, but a real complete/transition fixture has
/// not authenticated the live observer. Linking this module cannot schedule it.
pub const PRODUCTION_SCHEDULING_ENABLED: bool = false;
pub const LIVE_FIXTURE_ACCEPTED: bool = false;

const ITEM_FLAG_HIGHLIGHT: u16 = 1;
const ITEM_FLAG_HISTORICAL: u16 = 1 << 1;
const ITEM_FLAG_JOINT_COUNTRY: u16 = 1 << 2;
const ITEM_KNOWN_FLAGS: u16 =
    ITEM_FLAG_HIGHLIGHT | ITEM_FLAG_HISTORICAL | ITEM_FLAG_JOINT_COUNTRY;

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut output = [0; 32];
    let mut index = 0;
    while index < output.len() {
        output[index] = (nibble(input[index * 2]) << 4) | nibble(input[index * 2 + 1]);
        index += 1;
    }
    output
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityState {
    Unavailable,
    Exact20Receipt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusPresentationItem {
    pub focus_id: String,
    pub source_ordinal: u32,
    /// Final item+0x540 state: 0 unavailable, 1 complete, 2 available, 3 active.
    pub presentation_state_code: u8,
    /// Exact final integer passed by 0x141bb5df0, never a progress-days estimate.
    pub display_progress_percent: u8,
    pub highlight_glow_visible: bool,
    pub historical_marker_visible: bool,
    pub joint_viewing_country_tag: Option<String>,
}

impl FocusPresentationItem {
    pub fn available(&self) -> bool {
        self.presentation_state_code == 2
    }

    pub fn continuous_glow_visible(&self) -> bool {
        self.presentation_state_code == 3
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub source_sequence: u64,
    pub producer_complete: bool,
    pub country_tag: String,
    pub view_id: String,
    pub producer_record_identity: String,
    pub expected_visible_item_count: u32,
    pub visible_items: Vec<FocusPresentationItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub elapsed_wall_ms: u64,
    pub frame_hour: u64,
    pub status_flags: u32,
    pub authority: AuthorityState,
    pub snapshot: Option<Snapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    ActivationDisabled,
    UnauthenticatedSource,
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub trait ExactNationalFocusPresentationSource {
    fn is_exact_a729_focus_view_source(&self) -> bool;
    fn producer_manifest_sha256(&self) -> [u8; 32];
    fn collect_snapshot(&self, frame_hour: u64, country_tag: &str) -> Result<Snapshot, String>;
}

pub fn collect_and_validate<S: ExactNationalFocusPresentationSource>(
    source: &S,
    frame_hour: u64,
    country_tag: &str,
) -> Result<Snapshot, CarrierError> {
    if !source.is_exact_a729_focus_view_source()
        || source.producer_manifest_sha256() != EXACT20_FUNCTION_MANIFEST_SHA256
    {
        return Err(CarrierError::UnauthenticatedSource);
    }
    let snapshot = source
        .collect_snapshot(frame_hour, country_tag)
        .map_err(CarrierError::Source)?;
    if snapshot.country_tag != country_tag {
        return invalid("requested and returned country identities differ");
    }
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub fn collect_and_encode<S: ExactNationalFocusPresentationSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    country_tag: &str,
) -> Result<Vec<u8>, CarrierError> {
    if !PRODUCTION_SCHEDULING_ENABLED || !LIVE_FIXTURE_ACCEPTED {
        return Err(CarrierError::ActivationDisabled);
    }
    let snapshot = collect_and_validate(source, frame_hour, country_tag)?;
    encode_payload(&Frame {
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
    encode_payload(&Frame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority: AuthorityState::Unavailable,
        snapshot: None,
    })
}

pub fn validate_snapshot(snapshot: &Snapshot) -> Result<(), CarrierError> {
    if snapshot.source_sequence == 0
        || !snapshot.producer_complete
        || !valid_tag(&snapshot.country_tag)
        || !valid_text(&snapshot.view_id)
        || !valid_text(&snapshot.producer_record_identity)
        || snapshot.visible_items.len() > MAX_VISIBLE_ITEMS
        || snapshot.expected_visible_item_count as usize != snapshot.visible_items.len()
        || snapshot.view_id != format!("nationalfocusview:{}", snapshot.country_tag)
    {
        return invalid("NFP1 snapshot identity/completeness invalid");
    }
    let mut ids = BTreeSet::new();
    for (ordinal, item) in snapshot.visible_items.iter().enumerate() {
        if item.source_ordinal as usize != ordinal
            || !valid_focus_id(&item.focus_id)
            || !ids.insert(item.focus_id.as_str())
            || item.presentation_state_code > 3
            || item.display_progress_percent > 100
            || item
                .joint_viewing_country_tag
                .as_deref()
                .is_some_and(|tag| !valid_tag(tag))
        {
            return invalid("NFP1 item identity/order/domain invalid");
        }
        match item.presentation_state_code {
            0 | 2 if item.display_progress_percent != 0 => {
                return invalid("NFP1 not-started item carries progress")
            }
            1 if item.display_progress_percent != 100 => {
                return invalid("NFP1 completed item does not carry 100 percent")
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn validate_frame(frame: &Frame) -> Result<(), CarrierError> {
    match frame.authority {
        AuthorityState::Unavailable if frame.status_flags != 0 && frame.snapshot.is_none() => Ok(()),
        AuthorityState::Exact20Receipt if frame.status_flags == 0 => {
            let snapshot = frame
                .snapshot
                .as_ref()
                .ok_or_else(|| CarrierError::Invalid("NFP1 exact frame omitted snapshot".into()))?;
            validate_snapshot(snapshot)?;
            let expected_identity = format!(
                "a729:a31:{}:{}:{}",
                frame.frame_hour, snapshot.country_tag, snapshot.source_sequence
            );
            if snapshot.producer_record_identity != expected_identity {
                return invalid("NFP1 producer record identity disagrees with frame");
            }
            Ok(())
        }
        _ => invalid("NFP1 authority/status/snapshot mismatch"),
    }
}

pub fn encode_payload(frame: &Frame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut output = Vec::new();
    u32v(&mut output, MAGIC);
    u32v(&mut output, SCHEMA_VERSION);
    u32v(
        &mut output,
        match frame.authority {
            AuthorityState::Unavailable => 0,
            AuthorityState::Exact20Receipt => 1,
        },
    );
    u32v(&mut output, frame.status_flags);
    u64v(&mut output, frame.elapsed_wall_ms);
    u64v(&mut output, frame.frame_hour);
    output.extend_from_slice(&EXECUTABLE_SHA256);
    output.extend_from_slice(&EXACT20_FUNCTION_MANIFEST_SHA256);
    output.extend_from_slice(&EXACT20_JOB_RECEIPT_SHA256);
    output.push(u8::from(frame.snapshot.is_some()));
    output.extend_from_slice(&[0; 3]);
    if let Some(snapshot) = &frame.snapshot {
        u64v(&mut output, snapshot.source_sequence);
        textv(&mut output, &snapshot.country_tag);
        textv(&mut output, &snapshot.view_id);
        textv(&mut output, &snapshot.producer_record_identity);
        u32v(&mut output, snapshot.expected_visible_item_count);
        u32v(&mut output, snapshot.visible_items.len() as u32);
        for item in &snapshot.visible_items {
            u32v(&mut output, item.source_ordinal);
            output.push(item.presentation_state_code);
            output.push(item.display_progress_percent);
            let mut flags = 0;
            if item.highlight_glow_visible {
                flags |= ITEM_FLAG_HIGHLIGHT;
            }
            if item.historical_marker_visible {
                flags |= ITEM_FLAG_HISTORICAL;
            }
            if item.joint_viewing_country_tag.is_some() {
                flags |= ITEM_FLAG_JOINT_COUNTRY;
            }
            u16v(&mut output, flags);
            textv(&mut output, &item.focus_id);
            opt_textv(&mut output, item.joint_viewing_country_tag.as_deref());
        }
    }
    Ok(output)
}

pub fn decode_payload(payload: &[u8]) -> Result<Frame, CarrierError> {
    let mut reader = Reader { payload, at: 0 };
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("NFP1 magic/schema mismatch");
    }
    let authority = match reader.u32()? {
        0 => AuthorityState::Unavailable,
        1 => AuthorityState::Exact20Receipt,
        _ => return invalid("NFP1 authority enum unknown"),
    };
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    if reader.bytes(32)? != EXECUTABLE_SHA256
        || reader.bytes(32)? != EXACT20_FUNCTION_MANIFEST_SHA256
        || reader.bytes(32)? != EXACT20_JOB_RECEIPT_SHA256
    {
        return invalid("NFP1 executable/receipt identity mismatch");
    }
    let present = reader.u8()?;
    if present > 1 || reader.bytes(3)? != [0, 0, 0] {
        return invalid("NFP1 snapshot marker/reserved bytes invalid");
    }
    let snapshot = if present == 1 {
        let source_sequence = reader.u64()?;
        let country_tag = reader.text()?;
        let view_id = reader.text()?;
        let producer_record_identity = reader.text()?;
        let expected_visible_item_count = reader.u32()?;
        let count = reader.u32()? as usize;
        if count > MAX_VISIBLE_ITEMS {
            return invalid("NFP1 item count exceeds bound");
        }
        let mut visible_items = Vec::with_capacity(count);
        for _ in 0..count {
            let source_ordinal = reader.u32()?;
            let presentation_state_code = reader.u8()?;
            let display_progress_percent = reader.u8()?;
            let flags = reader.u16()?;
            if flags & !ITEM_KNOWN_FLAGS != 0 {
                return invalid("NFP1 item carries unknown flags");
            }
            let focus_id = reader.text()?;
            let joint_viewing_country_tag = reader.opt_text()?;
            if (flags & ITEM_FLAG_JOINT_COUNTRY != 0) != joint_viewing_country_tag.is_some() {
                return invalid("NFP1 joint-country flag/value mismatch");
            }
            visible_items.push(FocusPresentationItem {
                focus_id,
                source_ordinal,
                presentation_state_code,
                display_progress_percent,
                highlight_glow_visible: flags & ITEM_FLAG_HIGHLIGHT != 0,
                historical_marker_visible: flags & ITEM_FLAG_HISTORICAL != 0,
                joint_viewing_country_tag,
            });
        }
        Some(Snapshot {
            source_sequence,
            producer_complete: true,
            country_tag,
            view_id,
            producer_record_identity,
            expected_visible_item_count,
            visible_items,
        })
    } else {
        None
    };
    if reader.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = Frame {
        elapsed_wall_ms,
        frame_hour,
        status_flags,
        authority,
        snapshot,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn valid_tag(value: &str) -> bool {
    value.len() == 3
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && !value.as_bytes().contains(&0)
        && !value.chars().any(char::is_control)
}

/// Portable Clausewitz focus-definition identity. The engine definition
/// pointer is resolved before this boundary; host paths, pointer spellings,
/// and path separators are never valid substitutes for the authored token.
/// Dots, dashes, and `@` remain available to mods in addition to the usual
/// ASCII alphanumeric/underscore spelling.
pub fn valid_focus_id(value: &str) -> bool {
    const MAX_FOCUS_ID_BYTES: usize = 128;
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_FOCUS_ID_BYTES
        || !(bytes[0].is_ascii_alphanumeric() || bytes[0] == b'_')
        || value.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'@'))
        })
        || value.contains("..")
    {
        return false;
    }
    let pointer_digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"));
    !pointer_digits.is_some_and(|digits| {
        digits.len() >= 8 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}

fn u16v(output: &mut Vec<u8>, value: u16) { output.extend_from_slice(&value.to_le_bytes()); }
fn u32v(output: &mut Vec<u8>, value: u32) { output.extend_from_slice(&value.to_le_bytes()); }
fn u64v(output: &mut Vec<u8>, value: u64) { output.extend_from_slice(&value.to_le_bytes()); }
fn textv(output: &mut Vec<u8>, value: &str) { u32v(output, value.len() as u32); output.extend_from_slice(value.as_bytes()); }
fn opt_textv(output: &mut Vec<u8>, value: Option<&str>) { match value { Some(value) => { output.push(1); textv(output, value); }, None => output.push(0), } }

struct Reader<'a> { payload: &'a [u8], at: usize }
impl<'a> Reader<'a> {
    fn bytes(&mut self, count: usize) -> Result<&'a [u8], CarrierError> { let end = self.at.checked_add(count).ok_or(CarrierError::Truncated)?; let bytes = self.payload.get(self.at..end).ok_or(CarrierError::Truncated)?; self.at = end; Ok(bytes) }
    fn u8(&mut self) -> Result<u8, CarrierError> { Ok(self.bytes(1)?[0]) }
    fn u16(&mut self) -> Result<u16, CarrierError> { Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap())) }
    fn u32(&mut self) -> Result<u32, CarrierError> { Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> Result<u64, CarrierError> { Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap())) }
    fn text(&mut self) -> Result<String, CarrierError> { let count = self.u32()? as usize; if count > MAX_TEXT_BYTES { return invalid("NFP1 text exceeds bound"); } String::from_utf8(self.bytes(count)?.to_vec()).map_err(|_| CarrierError::Invalid("NFP1 text is not UTF-8".into())) }
    fn opt_text(&mut self) -> Result<Option<String>, CarrierError> { match self.u8()? { 0 => Ok(None), 1 => Ok(Some(self.text()?)), _ => invalid("NFP1 optional-text marker invalid"), } }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame() -> Frame { Frame { elapsed_wall_ms: 7, frame_hour: 42, status_flags: 0, authority: AuthorityState::Exact20Receipt, snapshot: Some(Snapshot { source_sequence: 9, producer_complete: true, country_tag: "GER".into(), view_id: "nationalfocusview:GER".into(), producer_record_identity: "a729:a31:42:GER:9".into(), expected_visible_item_count: 2, visible_items: vec![ FocusPresentationItem { focus_id: "ger_a".into(), source_ordinal: 0, presentation_state_code: 1, display_progress_percent: 100, highlight_glow_visible: false, historical_marker_visible: true, joint_viewing_country_tag: None }, FocusPresentationItem { focus_id: "ger_b".into(), source_ordinal: 1, presentation_state_code: 3, display_progress_percent: 37, highlight_glow_visible: true, historical_marker_visible: false, joint_viewing_country_tag: Some("ITA".into()) } ] }) } }
    #[test] fn roundtrip_preserves_pointer_free_item_identity_and_engine_order() { let expected = frame(); assert_eq!(decode_payload(&encode_payload(&expected).unwrap()).unwrap(), expected); }
    #[test] fn incomplete_or_inferred_state_rejects_whole_frame() { let mut value = frame(); value.snapshot.as_mut().unwrap().visible_items[1].source_ordinal = 7; assert!(encode_payload(&value).is_err()); let mut value = frame(); value.snapshot.as_mut().unwrap().visible_items[0].display_progress_percent = 99; assert!(encode_payload(&value).is_err()); }
    #[test] fn production_activation_remains_false_without_live_fixture() { struct Source; impl ExactNationalFocusPresentationSource for Source { fn is_exact_a729_focus_view_source(&self) -> bool { true } fn producer_manifest_sha256(&self) -> [u8; 32] { EXACT20_FUNCTION_MANIFEST_SHA256 } fn collect_snapshot(&self, _: u64, _: &str) -> Result<Snapshot, String> { Ok(frame().snapshot.unwrap()) } } assert_eq!(collect_and_encode(&Source, 42, 7, "GER"), Err(CarrierError::ActivationDisabled)); }
    #[test]
    fn source_authority_and_requested_country_are_mandatory() {
        struct Source { authenticated: bool, manifest: [u8; 32], country: &'static str }
        impl ExactNationalFocusPresentationSource for Source {
            fn is_exact_a729_focus_view_source(&self) -> bool { self.authenticated }
            fn producer_manifest_sha256(&self) -> [u8; 32] { self.manifest }
            fn collect_snapshot(&self, _: u64, _: &str) -> Result<Snapshot, String> {
                let mut snapshot = frame().snapshot.unwrap();
                snapshot.country_tag = self.country.into();
                snapshot.view_id = format!("nationalfocusview:{}", self.country);
                snapshot.producer_record_identity = format!("a729:a31:42:{}:9", self.country);
                Ok(snapshot)
            }
        }
        assert_eq!(collect_and_validate(&Source { authenticated: false, manifest: EXACT20_FUNCTION_MANIFEST_SHA256, country: "GER" }, 42, "GER"), Err(CarrierError::UnauthenticatedSource));
        assert_eq!(collect_and_validate(&Source { authenticated: true, manifest: [7; 32], country: "GER" }, 42, "GER"), Err(CarrierError::UnauthenticatedSource));
        assert!(collect_and_validate(&Source { authenticated: true, manifest: EXACT20_FUNCTION_MANIFEST_SHA256, country: "ITA" }, 42, "GER").is_err());
        assert!(collect_and_validate(&Source { authenticated: true, manifest: EXACT20_FUNCTION_MANIFEST_SHA256, country: "GER" }, 42, "GER").is_ok());
    }
    #[test]
    fn focus_identity_is_portable_and_mod_valid() {
        for accepted in ["GER_autobahn", "mod-focus.v2", "shared@branch", "_joint_focus"] {
            assert!(valid_focus_id(accepted), "{accepted}");
        }
        for rejected in [
            "0x000001abcdef1234",
            r"C:\\hoi4\\focus",
            "/opt/hoi4/focus",
            r"\\host\\share\\focus",
            "focus/child",
            "focus..child",
            "focus child",
        ] {
            assert!(!valid_focus_id(rejected), "{rejected}");
            let mut value = frame();
            value.snapshot.as_mut().unwrap().visible_items[0].focus_id = rejected.into();
            assert!(encode_payload(&value).is_err(), "{rejected}");
        }
    }
}
