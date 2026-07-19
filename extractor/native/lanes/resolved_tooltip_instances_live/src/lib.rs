//! A26/XUI05 exact resolved-tooltip instance carrier (schedulable d72).
//!
//! RenderDoc rectangles and OCR text remain validation oracles only. Production
//! collection requires an authenticated a729 tooltip hook implementing the
//! source trait.

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 72;
pub const DOMAIN_NAME: &str = "resolved_tooltip_instances_live";
pub const SCHEMA_VERSION: u32 = 1;
pub const AUTHORITY_EXACT_A729_TOOLTIP_HOOK: u32 = 1;
pub const MAGIC: u32 = u32::from_le_bytes(*b"TTI1");
pub const HEADER_LEN: usize = 76;
pub const MAX_INSTANCES: usize = 4_096;
pub const MAX_ROWS_PER_INSTANCE: usize = 4_096;
pub const MAX_STRING_BYTES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectI32 {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooltipLineKind {
    Header,
    Description,
    Value,
    Breakdown,
    Scripted,
    Spacer,
    /// Exact engine-resolved fragment whose authoring class is no longer
    /// observable at the final setter boundary.
    Resolved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowAvailability {
    Resolved,
    NotApplicable,
    DataBlocked { field: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTooltipRow {
    pub source_ordinal: u32,
    pub line_kind: TooltipLineKind,
    pub source_identity: String,
    pub evaluation_context: String,
    pub availability: RowAvailability,
    pub resolved_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTooltipInstance {
    pub frame_hour: u64,
    pub pov_country_or_global: String,
    pub view_id: String,
    pub element_path: String,
    pub repeat_index: u32,
    pub screen_rect: RectI32,
    pub viewport_width_px: i32,
    pub viewport_height_px: i32,
    pub producer_record_identity: String,
    pub rows: Vec<ResolvedTooltipRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipTraceFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
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
    fn completeness_record_identity(&self, frame_hour: u64) -> Result<String, String>;
    fn expected_instance_count(&self, frame_hour: u64) -> Result<u32, String>;
    fn collect_frame(&self, frame_hour: u64) -> Result<Vec<ResolvedTooltipInstance>, String>;
}

pub fn collect_and_encode<S: ExactTooltipInstanceSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, TooltipTraceError> {
    if !source.is_exact_a729_tooltip_source() {
        return Err(TooltipTraceError::UnauthenticatedSource);
    }
    encode_payload(&TooltipTraceFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        producer_manifest_sha256: source.producer_manifest_sha256(),
        completeness_record_identity: source
            .completeness_record_identity(frame_hour)
            .map_err(TooltipTraceError::Source)?,
        expected_instance_count: source
            .expected_instance_count(frame_hour)
            .map_err(TooltipTraceError::Source)?,
        instances: source
            .collect_frame(frame_hour)
            .map_err(TooltipTraceError::Source)?,
    })
}

pub fn validate_frame(frame: &TooltipTraceFrame) -> Result<(), TooltipTraceError> {
    if frame.producer_manifest_sha256 == [0; 32] {
        return invalid("producer manifest hash is zero");
    }
    validate_nonempty(&frame.completeness_record_identity, "completeness record")?;
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
        validate_nonempty(&instance.pov_country_or_global, "pov country/global")?;
        validate_nonempty(&instance.view_id, "view id")?;
        validate_nonempty(&instance.element_path, "element path")?;
        validate_nonempty(&instance.producer_record_identity, "producer record")?;
        if !keys.insert((
            instance.pov_country_or_global.as_str(),
            instance.view_id.as_str(),
            instance.element_path.as_str(),
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
        if instance.rows.len() > MAX_ROWS_PER_INSTANCE {
            return invalid("tooltip row count exceeds bound");
        }
        let mut prior = None;
        for row in &instance.rows {
            if prior.is_some_and(|value| row.source_ordinal <= value) {
                return invalid("row ordinals are not strictly increasing");
            }
            prior = Some(row.source_ordinal);
            validate_nonempty(&row.source_identity, "row source identity")?;
            validate_nonempty(&row.evaluation_context, "row evaluation context")?;
            match &row.availability {
                RowAvailability::Resolved => {
                    validate_nonempty(&row.resolved_text, "resolved text")?;
                }
                RowAvailability::NotApplicable => {
                    if !row.resolved_text.is_empty() {
                        return invalid("not-applicable row carries text");
                    }
                }
                RowAvailability::DataBlocked { field } => {
                    validate_nonempty(field, "blocked field")?;
                    if !row.resolved_text.is_empty() {
                        return invalid("data-blocked row carries text");
                    }
                }
            }
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
    out.extend_from_slice(&frame.producer_manifest_sha256);
    put_u32(&mut out, frame.expected_instance_count);
    put_u32(&mut out, frame.instances.len() as u32);
    put_u32(&mut out, HEADER_LEN as u32);
    put_string(&mut out, &frame.completeness_record_identity, false)?;
    for instance in &frame.instances {
        put_u64(&mut out, instance.frame_hour);
        put_string(&mut out, &instance.pov_country_or_global, false)?;
        put_string(&mut out, &instance.view_id, false)?;
        put_string(&mut out, &instance.element_path, false)?;
        put_u32(&mut out, instance.repeat_index);
        put_rect(&mut out, instance.screen_rect);
        put_i32(&mut out, instance.viewport_width_px);
        put_i32(&mut out, instance.viewport_height_px);
        put_string(&mut out, &instance.producer_record_identity, false)?;
        put_u32(&mut out, instance.rows.len() as u32);
        for row in &instance.rows {
            put_u32(&mut out, row.source_ordinal);
            put_u32(&mut out, line_kind_code(row.line_kind));
            put_string(&mut out, &row.source_identity, false)?;
            put_string(&mut out, &row.evaluation_context, false)?;
            match &row.availability {
                RowAvailability::Resolved => {
                    put_u32(&mut out, 0);
                    put_optional_string(&mut out, None)?;
                }
                RowAvailability::NotApplicable => {
                    put_u32(&mut out, 1);
                    put_optional_string(&mut out, None)?;
                }
                RowAvailability::DataBlocked { field } => {
                    put_u32(&mut out, 2);
                    put_optional_string(&mut out, Some(field))?;
                }
            }
            put_string(&mut out, &row.resolved_text, true)?;
        }
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
    let producer_manifest_sha256 = reader.array32()?;
    let expected_instance_count = reader.u32()?;
    let instance_count = reader.u32()? as usize;
    if reader.u32()? as usize != HEADER_LEN || instance_count > MAX_INSTANCES {
        return invalid("header/count invalid");
    }
    let completeness_record_identity = reader.string(false)?;
    let mut instances = Vec::with_capacity(instance_count);
    for _ in 0..instance_count {
        let instance_frame_hour = reader.u64()?;
        let pov_country_or_global = reader.string(false)?;
        let view_id = reader.string(false)?;
        let element_path = reader.string(false)?;
        let repeat_index = reader.u32()?;
        let screen_rect = reader.rect()?;
        let viewport_width_px = reader.i32()?;
        let viewport_height_px = reader.i32()?;
        let producer_record_identity = reader.string(false)?;
        let row_count = reader.u32()? as usize;
        if row_count > MAX_ROWS_PER_INSTANCE {
            return invalid("row count exceeds bound");
        }
        let mut rows = Vec::with_capacity(row_count);
        for _ in 0..row_count {
            let source_ordinal = reader.u32()?;
            let line_kind = match reader.u32()? {
                0 => TooltipLineKind::Header,
                1 => TooltipLineKind::Description,
                2 => TooltipLineKind::Value,
                3 => TooltipLineKind::Breakdown,
                4 => TooltipLineKind::Scripted,
                5 => TooltipLineKind::Spacer,
                6 => TooltipLineKind::Resolved,
                _ => return invalid("line kind invalid"),
            };
            let source_identity = reader.string(false)?;
            let evaluation_context = reader.string(false)?;
            let availability_code = reader.u32()?;
            let blocked_field = reader.optional_string()?;
            let availability = match (availability_code, blocked_field) {
                (0, None) => RowAvailability::Resolved,
                (1, None) => RowAvailability::NotApplicable,
                (2, Some(field)) => RowAvailability::DataBlocked { field },
                _ => return invalid("availability encoding invalid"),
            };
            rows.push(ResolvedTooltipRow {
                source_ordinal,
                line_kind,
                source_identity,
                evaluation_context,
                availability,
                resolved_text: reader.string(true)?,
            });
        }
        instances.push(ResolvedTooltipInstance {
            frame_hour: instance_frame_hour,
            pov_country_or_global,
            view_id,
            element_path,
            repeat_index,
            screen_rect,
            viewport_width_px,
            viewport_height_px,
            producer_record_identity,
            rows,
        });
    }
    if !reader.done() {
        return Err(TooltipTraceError::TrailingBytes);
    }
    let frame = TooltipTraceFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        producer_manifest_sha256,
        completeness_record_identity,
        expected_instance_count,
        instances,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn line_kind_code(kind: TooltipLineKind) -> u32 {
    match kind {
        TooltipLineKind::Header => 0,
        TooltipLineKind::Description => 1,
        TooltipLineKind::Value => 2,
        TooltipLineKind::Breakdown => 3,
        TooltipLineKind::Scripted => 4,
        TooltipLineKind::Spacer => 5,
        TooltipLineKind::Resolved => 6,
    }
}

/// Preserve every final resolved fragment without guessing whether it was an
/// authored header, value, breakdown, or scripted-localisation leaf. Newline
/// boundaries are the only admitted row boundaries.
pub fn rows_from_final_resolved_text(
    final_text: &str,
    source_identity: &str,
    evaluation_context: &str,
) -> Result<Vec<ResolvedTooltipRow>, TooltipTraceError> {
    validate_nonempty(source_identity, "row source identity")?;
    validate_nonempty(evaluation_context, "row evaluation context")?;
    if final_text.is_empty() {
        return Ok(Vec::new());
    }
    final_text
        .split('\n')
        .enumerate()
        .map(|(index, fragment)| {
            let source_ordinal = u32::try_from(index)
                .map_err(|_| TooltipTraceError::Invalid("tooltip row ordinal overflow".into()))?;
            Ok(ResolvedTooltipRow {
                source_ordinal,
                line_kind: if fragment.is_empty() {
                    TooltipLineKind::Spacer
                } else {
                    TooltipLineKind::Resolved
                },
                source_identity: source_identity.to_string(),
                evaluation_context: evaluation_context.to_string(),
                availability: if fragment.is_empty() {
                    RowAvailability::NotApplicable
                } else {
                    RowAvailability::Resolved
                },
                resolved_text: fragment.to_string(),
            })
        })
        .collect()
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

fn put_optional_string(out: &mut Vec<u8>, value: Option<&str>) -> Result<(), TooltipTraceError> {
    match value {
        None => put_u32(out, u32::MAX),
        Some(value) => {
            validate_nonempty(value, "optional string")?;
            put_u32(out, value.len() as u32);
            out.extend_from_slice(value.as_bytes());
        }
    }
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

    fn optional_string(&mut self) -> Result<Option<String>, TooltipTraceError> {
        let count = self.u32()?;
        if count == u32::MAX {
            return Ok(None);
        }
        let count = count as usize;
        if count == 0 || count > MAX_STRING_BYTES {
            return invalid("optional string length invalid");
        }
        Ok(Some(
            String::from_utf8(self.take(count)?.to_vec())
                .map_err(|_| TooltipTraceError::Invalid("wire string not UTF-8".into()))?,
        ))
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
            producer_manifest_sha256: [0xA5; 32],
            completeness_record_identity: "tooltip_hook.frames[900]".into(),
            expected_instance_count: 1,
            instances: vec![ResolvedTooltipInstance {
                frame_hour: 900,
                pov_country_or_global: "GER".into(),
                view_id: "topbar".into(),
                element_path: "topbar/political_power".into(),
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
                rows: vec![
                    ResolvedTooltipRow {
                        source_ordinal: 10,
                        line_kind: TooltipLineKind::Header,
                        source_identity: "header".into(),
                        evaluation_context: "country=GER".into(),
                        availability: RowAvailability::Resolved,
                        resolved_text: "Political Power".into(),
                    },
                    ResolvedTooltipRow {
                        source_ordinal: 20,
                        line_kind: TooltipLineKind::Breakdown,
                        source_identity: "modifier[0]".into(),
                        evaluation_context: "country=GER".into(),
                        availability: RowAvailability::NotApplicable,
                        resolved_text: String::new(),
                    },
                ],
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
    fn completeness_key_rect_and_row_order_fail_closed() {
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
        frame.instances[0].rows.swap(0, 1);
        assert!(validate_frame(&frame).is_err());
    }

    #[test]
    fn availability_and_manifest_fail_closed() {
        let mut frame = exact_frame();
        frame.producer_manifest_sha256 = [0; 32];
        assert!(validate_frame(&frame).is_err());

        let mut frame = exact_frame();
        frame.instances[0].rows[0].availability = RowAvailability::NotApplicable;
        assert!(validate_frame(&frame).is_err());

        let mut frame = exact_frame();
        frame.instances[0].rows[1].availability = RowAvailability::DataBlocked {
            field: String::new(),
        };
        assert!(validate_frame(&frame).is_err());
    }

    #[test]
    fn exact_final_text_rows_preserve_newlines_without_semantic_guessing() {
        let rows = rows_from_final_resolved_text(
            "Header\nValue +10\n\nTrailing\n",
            "hovered-cgui-type:test",
            "controller-rva:0x002A4080",
        )
        .unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].line_kind, TooltipLineKind::Resolved);
        assert_eq!(rows[1].resolved_text, "Value +10");
        assert_eq!(rows[2].line_kind, TooltipLineKind::Spacer);
        assert_eq!(rows[2].availability, RowAvailability::NotApplicable);
        assert_eq!(rows[4].line_kind, TooltipLineKind::Spacer);
        assert_eq!(
            rows.iter()
                .map(|row| row.source_ordinal)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4]
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

        fn completeness_record_identity(&self, _: u64) -> Result<String, String> {
            Ok("tooltip_hook.frames[900]".into())
        }

        fn expected_instance_count(&self, _: u64) -> Result<u32, String> {
            Ok(1)
        }

        fn collect_frame(&self, _: u64) -> Result<Vec<ResolvedTooltipInstance>, String> {
            Ok(exact_frame().instances)
        }
    }

    #[test]
    fn collector_seam_rejects_unauthenticated_source() {
        assert_eq!(
            collect_and_encode(&Fake(false), 900, 0, 0),
            Err(TooltipTraceError::UnauthenticatedSource)
        );
        assert!(collect_and_encode(&Fake(true), 900, 0, 0).is_ok());
    }
}
