//! A26/XUI05 authenticated final-tooltip carrier (production d72).
//!
//! RenderDoc rectangles and OCR text remain validation oracles only. Production
//! collection requires an authenticated a729 tooltip hook implementing the
//! source trait.

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 72;
pub const DOMAIN_NAME: &str = "resolved_tooltip_instances_live";
pub const SCHEMA_VERSION: u32 = 2;
pub const AUTHORITY_EXACT_A729_TOOLTIP_HOOK: u32 = 1;
pub const MAGIC: u32 = u32::from_le_bytes(*b"TTI2");
pub const HEADER_LEN: usize = 96;
pub const MAX_INSTANCES: usize = 4_096;
pub const MAX_STRING_BYTES: usize = 65_536;
/// The ordinary collector owns the exact-a729 hook lifecycle and emits a
/// typed-unavailable TTI2 record whenever installation or selected-Present
/// admission fails. A positive visibility fixture is regression evidence, not
/// a prerequisite for using the already-authenticated passive observer.
pub const PRODUCTION_SCHEDULING_ENABLED: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectI32 {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTooltipInstance {
    pub frame_hour: u64,
    pub pov_country_or_global: String,
    /// Exact authored `CGuiType+0x28` name of the hovered GUI object. This is
    /// not promoted into a fabricated source-row or view-path identity.
    pub hovered_element_authored_name: String,
    /// Exact cached authored name of the global CTextBox owner.
    pub tooltip_owner_authored_name: String,
    pub repeat_index: u32,
    pub screen_rect: RectI32,
    pub viewport_width_px: i32,
    pub viewport_height_px: i32,
    pub producer_record_identity: String,
    /// One copied final post-layout engine string. Newlines remain bytes inside
    /// this blob; they are not reclassified as authored semantic rows.
    pub final_resolved_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedPresentStamp {
    pub frame_hour: u64,
    pub selected_present_candidate_id: u64,
    pub selected_present_ordinal: u32,
    pub selected_present_qpc_ticks: u64,
    pub selected_present_receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipTraceFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub selected_present_candidate_id: u64,
    pub selected_present_ordinal: u32,
    pub selected_present_qpc_ticks: u64,
    pub selected_present_receipt_sha256: String,
    pub producer_manifest_sha256: [u8; 32],
    pub completeness_record_identity: String,
    pub expected_instance_count: u32,
    pub instances: Vec<ResolvedTooltipInstance>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TooltipTraceError {
    UnauthenticatedSource,
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub trait ExactTooltipInstanceSource {
    fn is_exact_a729_tooltip_source(&self) -> bool;
    fn producer_manifest_sha256(&self) -> [u8; 32];
    /// Seal one immutable source snapshot against an authenticated selected
    /// Present. Completeness, count, and instances must come from this one call.
    fn capture_selected_present(
        &self,
        selected_present: &SelectedPresentStamp,
        elapsed_wall_ms: u64,
        status_flags: u32,
    ) -> Result<TooltipTraceFrame, String>;
}

pub fn collect_and_encode<S: ExactTooltipInstanceSource>(
    source: &S,
    selected_present: &SelectedPresentStamp,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, TooltipTraceError> {
    if !source.is_exact_a729_tooltip_source() {
        return Err(TooltipTraceError::UnauthenticatedSource);
    }
    let frame = source
        .capture_selected_present(selected_present, elapsed_wall_ms, status_flags)
        .map_err(TooltipTraceError::Source)?;
    if frame.producer_manifest_sha256 != source.producer_manifest_sha256() {
        return Err(TooltipTraceError::UnauthenticatedSource);
    }
    if frame.frame_hour != selected_present.frame_hour
        || frame.selected_present_candidate_id
            != selected_present.selected_present_candidate_id
        || frame.selected_present_ordinal != selected_present.selected_present_ordinal
        || frame.selected_present_qpc_ticks != selected_present.selected_present_qpc_ticks
        || frame.selected_present_receipt_sha256
            != selected_present.selected_present_receipt_sha256
    {
        return Err(TooltipTraceError::UnauthenticatedSource);
    }
    encode_payload(&frame)
}

pub fn validate_frame(frame: &TooltipTraceFrame) -> Result<(), TooltipTraceError> {
    if frame.status_flags != 0 {
        if frame.selected_present_candidate_id != 0
            || frame.selected_present_ordinal != 0
            || frame.selected_present_qpc_ticks != 0
            || !frame.selected_present_receipt_sha256.is_empty()
            || frame.producer_manifest_sha256 != [0; 32]
            || !frame.completeness_record_identity.is_empty()
            || frame.expected_instance_count != 0
            || !frame.instances.is_empty()
        {
            return invalid("source-unavailable frame carries measured tooltip authority");
        }
        return Ok(());
    }
    if frame.producer_manifest_sha256 == [0; 32] {
        return invalid("producer manifest hash is zero");
    }
    validate_nonempty(&frame.completeness_record_identity, "completeness record")?;
    validate_sha256_hex(
        &frame.selected_present_receipt_sha256,
        "selected Present receipt",
    )?;
    if frame.selected_present_candidate_id == 0 || frame.selected_present_qpc_ticks == 0 {
        return invalid("selected Present identity is zero");
    }
    if frame.instances.len() > MAX_INSTANCES
        || frame.expected_instance_count as usize != frame.instances.len()
    {
        return invalid("instance completeness/count mismatch");
    }
    let mut keys = BTreeSet::new();
    for instance in &frame.instances {
        if instance.frame_hour != frame.frame_hour {
            return invalid("instance frame_hour differs from header");
        }
        if !valid_pov_country_or_global(&instance.pov_country_or_global) {
            return invalid("pov country/global identity is not portable");
        }
        validate_nonempty(
            &instance.hovered_element_authored_name,
            "hovered authored element name",
        )?;
        validate_nonempty(
            &instance.tooltip_owner_authored_name,
            "tooltip owner authored name",
        )?;
        validate_nonempty(&instance.producer_record_identity, "producer record")?;
        validate_nonempty(&instance.final_resolved_text, "final resolved text")?;
        if !keys.insert((
            instance.pov_country_or_global.as_str(),
            instance.hovered_element_authored_name.as_str(),
            instance.tooltip_owner_authored_name.as_str(),
            instance.repeat_index,
        )) {
            return invalid("duplicate tooltip instance key");
        }
        validate_rect(instance.screen_rect)?;
        if instance.viewport_width_px <= 0 || instance.viewport_height_px <= 0 {
            return invalid("viewport is nonpositive");
        }
        if !rects_intersect(
            instance.screen_rect,
            RectI32 {
                x: 0,
                y: 0,
                width: instance.viewport_width_px,
                height: instance.viewport_height_px,
            },
        ) {
            return invalid("tooltip rectangle misses viewport");
        }
    }
    Ok(())
}

pub fn encode_payload(frame: &TooltipTraceFrame) -> Result<Vec<u8>, TooltipTraceError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    put_u32(&mut out, MAGIC);
    put_u32(&mut out, SCHEMA_VERSION);
    put_u32(&mut out, AUTHORITY_EXACT_A729_TOOLTIP_HOOK);
    put_u32(&mut out, frame.status_flags);
    put_u64(&mut out, frame.elapsed_wall_ms);
    put_u64(&mut out, frame.frame_hour);
    put_u64(&mut out, frame.selected_present_candidate_id);
    put_u32(&mut out, frame.selected_present_ordinal);
    put_u64(&mut out, frame.selected_present_qpc_ticks);
    out.extend_from_slice(&frame.producer_manifest_sha256);
    put_u32(&mut out, frame.expected_instance_count);
    put_u32(&mut out, frame.instances.len() as u32);
    put_u32(&mut out, HEADER_LEN as u32);
    put_string(
        &mut out,
        &frame.selected_present_receipt_sha256,
        frame.status_flags != 0,
    )?;
    put_string(
        &mut out,
        &frame.completeness_record_identity,
        frame.status_flags != 0,
    )?;
    for instance in &frame.instances {
        put_u64(&mut out, instance.frame_hour);
        put_string(&mut out, &instance.pov_country_or_global, false)?;
        put_string(&mut out, &instance.hovered_element_authored_name, false)?;
        put_string(&mut out, &instance.tooltip_owner_authored_name, false)?;
        put_u32(&mut out, instance.repeat_index);
        put_rect(&mut out, instance.screen_rect);
        put_i32(&mut out, instance.viewport_width_px);
        put_i32(&mut out, instance.viewport_height_px);
        put_string(&mut out, &instance.producer_record_identity, false)?;
        put_string(&mut out, &instance.final_resolved_text, false)?;
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<TooltipTraceFrame, TooltipTraceError> {
    let mut reader = Reader::new(payload);
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("magic/schema mismatch");
    }
    if reader.u32()? != AUTHORITY_EXACT_A729_TOOLTIP_HOOK {
        return Err(TooltipTraceError::UnauthenticatedSource);
    }
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    let selected_present_candidate_id = reader.u64()?;
    let selected_present_ordinal = reader.u32()?;
    let selected_present_qpc_ticks = reader.u64()?;
    let producer_manifest_sha256 = reader.array32()?;
    let expected_instance_count = reader.u32()?;
    let instance_count = reader.u32()? as usize;
    if reader.u32()? as usize != HEADER_LEN || instance_count > MAX_INSTANCES {
        return invalid("header/count invalid");
    }
    let selected_present_receipt_sha256 = reader.string(true)?;
    let completeness_record_identity = reader.string(true)?;
    let mut instances = Vec::with_capacity(instance_count);
    for _ in 0..instance_count {
        let instance_frame_hour = reader.u64()?;
        let pov_country_or_global = reader.string(false)?;
        let hovered_element_authored_name = reader.string(false)?;
        let tooltip_owner_authored_name = reader.string(false)?;
        let repeat_index = reader.u32()?;
        let screen_rect = reader.rect()?;
        let viewport_width_px = reader.i32()?;
        let viewport_height_px = reader.i32()?;
        let producer_record_identity = reader.string(false)?;
        let final_resolved_text = reader.string(false)?;
        instances.push(ResolvedTooltipInstance {
            frame_hour: instance_frame_hour,
            pov_country_or_global,
            hovered_element_authored_name,
            tooltip_owner_authored_name,
            repeat_index,
            screen_rect,
            viewport_width_px,
            viewport_height_px,
            producer_record_identity,
            final_resolved_text,
        });
    }
    if !reader.done() {
        return Err(TooltipTraceError::TrailingBytes);
    }
    let frame = TooltipTraceFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        selected_present_candidate_id,
        selected_present_ordinal,
        selected_present_qpc_ticks,
        selected_present_receipt_sha256,
        producer_manifest_sha256,
        completeness_record_identity,
        expected_instance_count,
        instances,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

#[cfg(windows)]
pub mod a729_hook;

fn rects_intersect(a: RectI32, b: RectI32) -> bool {
    a.x < b.x.saturating_add(b.width)
        && b.x < a.x.saturating_add(a.width)
        && a.y < b.y.saturating_add(b.height)
        && b.y < a.y.saturating_add(a.height)
}

fn invalid<T>(message: &str) -> Result<T, TooltipTraceError> {
    Err(TooltipTraceError::Invalid(message.into()))
}

fn validate_nonempty(value: &str, field: &str) -> Result<(), TooltipTraceError> {
    if value.trim().is_empty() || value.len() > MAX_STRING_BYTES {
        invalid(&format!("{field} empty or oversized"))
    } else {
        Ok(())
    }
}

pub(crate) fn validate_sha256_hex(
    value: &str,
    field: &str,
) -> Result<(), TooltipTraceError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || value.bytes().all(|byte| byte == b'0')
    {
        return invalid(&format!("{field} is not a nonzero lowercase SHA-256"));
    }
    Ok(())
}

/// Canonical fail-closed representation for a scheduled observer failure.
/// This is not a measured-hidden tooltip and cannot be replayed as an empty
/// instance set because all selected-Present and producer authority is absent.
pub fn encode_source_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, TooltipTraceError> {
    if status_flags == 0 {
        return invalid("source-unavailable frame requires nonzero status");
    }
    encode_payload(&TooltipTraceFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        selected_present_candidate_id: 0,
        selected_present_ordinal: 0,
        selected_present_qpc_ticks: 0,
        selected_present_receipt_sha256: String::new(),
        producer_manifest_sha256: [0; 32],
        completeness_record_identity: String::new(),
        expected_instance_count: 0,
        instances: Vec::new(),
    })
}

pub fn valid_pov_country_or_global(value: &str) -> bool {
    value == "global"
        || (value.len() == 3
            && value
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()))
}

fn validate_rect(rect: RectI32) -> Result<(), TooltipTraceError> {
    if rect.width <= 0 || rect.height <= 0 {
        invalid("screen rectangle is nonpositive")
    } else {
        Ok(())
    }
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_rect(out: &mut Vec<u8>, rect: RectI32) {
    put_i32(out, rect.x);
    put_i32(out, rect.y);
    put_i32(out, rect.width);
    put_i32(out, rect.height);
}

fn put_string(out: &mut Vec<u8>, value: &str, allow_empty: bool) -> Result<(), TooltipTraceError> {
    if value.len() > MAX_STRING_BYTES || (!allow_empty && value.trim().is_empty()) {
        return invalid("wire string empty or oversized");
    }
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], TooltipTraceError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(TooltipTraceError::Truncated)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(TooltipTraceError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, TooltipTraceError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32, TooltipTraceError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, TooltipTraceError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn array32(&mut self) -> Result<[u8; 32], TooltipTraceError> {
        Ok(self.take(32)?.try_into().unwrap())
    }

    fn string(&mut self, allow_empty: bool) -> Result<String, TooltipTraceError> {
        let count = self.u32()? as usize;
        if count > MAX_STRING_BYTES || (!allow_empty && count == 0) {
            return invalid("wire string length invalid");
        }
        String::from_utf8(self.take(count)?.to_vec())
            .map_err(|_| TooltipTraceError::Invalid("wire string not UTF-8".into()))
    }

    fn rect(&mut self) -> Result<RectI32, TooltipTraceError> {
        Ok(RectI32 {
            x: self.i32()?,
            y: self.i32()?,
            width: self.i32()?,
            height: self.i32()?,
        })
    }

    fn done(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact_frame() -> TooltipTraceFrame {
        TooltipTraceFrame {
            elapsed_wall_ms: 7,
            status_flags: 0,
            frame_hour: 900,
            selected_present_candidate_id: 7,
            selected_present_ordinal: 0,
            selected_present_qpc_ticks: 123_456,
            selected_present_receipt_sha256: "a5".repeat(32),
            producer_manifest_sha256: [0xA5; 32],
            completeness_record_identity: "tooltip_hook.frames[900]".into(),
            expected_instance_count: 1,
            instances: vec![ResolvedTooltipInstance {
                frame_hour: 900,
                pov_country_or_global: "GER".into(),
                hovered_element_authored_name: "political_power".into(),
                tooltip_owner_authored_name: "ToolTip".into(),
                repeat_index: 0,
                screen_rect: RectI32 {
                    x: 1223,
                    y: 671,
                    width: 297,
                    height: 148,
                },
                viewport_width_px: 1920,
                viewport_height_px: 1080,
                producer_record_identity: "tooltip_hook.instances[0]".into(),
                final_resolved_text: "Political Power\nDaily gain: +2.00".into(),
            }],
        }
    }

    #[test]
    fn exact_frame_round_trips_every_field() {
        let frame = exact_frame();
        assert_eq!(
            decode_payload(&encode_payload(&frame).unwrap()).unwrap(),
            frame
        );
    }

    #[test]
    fn measured_empty_frame_is_distinct_and_round_trips() {
        let mut frame = exact_frame();
        frame.expected_instance_count = 0;
        frame.instances.clear();
        assert_eq!(
            decode_payload(&encode_payload(&frame).unwrap()).unwrap(),
            frame
        );
    }

    #[test]
    fn completeness_key_rect_and_text_fail_closed() {
        let mut frame = exact_frame();
        frame.expected_instance_count = 2;
        assert!(validate_frame(&frame).is_err());

        let mut frame = exact_frame();
        frame.instances.push(frame.instances[0].clone());
        frame.expected_instance_count = 2;
        assert!(validate_frame(&frame).is_err());

        let mut frame = exact_frame();
        frame.instances[0].screen_rect.x = 3000;
        assert!(validate_frame(&frame).is_err());

        let mut frame = exact_frame();
        frame.instances[0].final_resolved_text.clear();
        assert!(validate_frame(&frame).is_err());
    }

    #[test]
    fn manifest_and_selected_present_fail_closed() {
        let mut frame = exact_frame();
        frame.producer_manifest_sha256 = [0; 32];
        assert!(validate_frame(&frame).is_err());

        let mut frame = exact_frame();
        frame.selected_present_receipt_sha256 = "0".repeat(64);
        assert!(validate_frame(&frame).is_err());
    }

    #[test]
    fn exact_final_text_blob_preserves_newlines_without_semantic_rows() {
        let frame = exact_frame();
        assert_eq!(
            frame.instances[0].final_resolved_text,
            "Political Power\nDaily gain: +2.00"
        );
    }

    struct Fake(bool);

    impl ExactTooltipInstanceSource for Fake {
        fn is_exact_a729_tooltip_source(&self) -> bool {
            self.0
        }

        fn producer_manifest_sha256(&self) -> [u8; 32] {
            [0xA5; 32]
        }

        fn capture_selected_present(
            &self,
            _: &SelectedPresentStamp,
            _: u64,
            _: u32,
        ) -> Result<TooltipTraceFrame, String> {
            Ok(exact_frame())
        }
    }

    #[test]
    fn collector_seam_rejects_unauthenticated_source() {
        let selected = SelectedPresentStamp {
            frame_hour: 900,
            selected_present_candidate_id: 7,
            selected_present_ordinal: 0,
            selected_present_qpc_ticks: 123_456,
            selected_present_receipt_sha256: "a5".repeat(32),
        };
        assert_eq!(
            collect_and_encode(&Fake(false), &selected, 0, 0),
            Err(TooltipTraceError::UnauthenticatedSource)
        );
        assert!(collect_and_encode(&Fake(true), &selected, 0, 0).is_ok());
        let mut torn = selected;
        torn.selected_present_candidate_id = 8;
        assert_eq!(
            collect_and_encode(&Fake(true), &torn, 0, 0),
            Err(TooltipTraceError::UnauthenticatedSource)
        );
    }
}
