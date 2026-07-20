//! Pointer-free d84 compact carrier for authenticated A38 scripted-GUI frames.
//!
//! The codec deliberately stores engine order, effective-VFS winner identity,
//! explicit unavailable reasons, and tombstones. It does not make d84
//! schedulable: registry promotion is a separate exact20/live-source gate.

use std::collections::{BTreeMap, BTreeSet};

use crate::scripted_gui_runtime_carrier::{
    collect_and_validate_scripted_gui_runtime, validate_scripted_gui_runtime_frame, CarrierError,
    portable_unavailable_diagnostic, EngineResult, ExactScriptedGuiRuntimeSource, ScriptedGuiDefinitionIdentity,
    ScriptedGuiRuntimeFrame, ScriptedGuiRuntimeRow,
};

pub const DOMAIN_ID: u16 = 84;
pub const DOMAIN_NAME: &str = "scripted_gui_runtime_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"SGR1");
pub const SCHEMA_VERSION: u32 = 1;
pub const AUTHORITY_AUTHENTICATED_FRAME_SOURCE: u32 = 1;
pub const MAX_TOTAL_RESULTS: usize = 1_000_000;
pub const MAX_TOTAL_TEXT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptedGuiRuntimeEnvelope {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    /// `None` is source-unavailable, never measured-empty.
    pub frame: Option<ScriptedGuiRuntimeFrame>,
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompactError {
    Source(CarrierError),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

pub fn collect_and_encode<S: ExactScriptedGuiRuntimeSource>(
    source: &S,
    frame_hour: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, CompactError> {
    let frame = collect_and_validate_scripted_gui_runtime(source, frame_hour)
        .map_err(CompactError::Source)?;
    encode_payload(&ScriptedGuiRuntimeEnvelope {
        elapsed_wall_ms,
        status_flags: 0,
        frame_hour,
        frame: Some(frame),
        unavailable_reason: None,
    })
}

pub fn encode_source_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
    reason: impl Into<String>,
) -> Result<Vec<u8>, CompactError> {
    let reason = portable_unavailable_diagnostic(&reason.into());
    encode_payload(&ScriptedGuiRuntimeEnvelope {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        frame: None,
        unavailable_reason: Some(reason),
    })
}

pub fn validate_envelope(value: &ScriptedGuiRuntimeEnvelope) -> Result<(), CompactError> {
    match (&value.frame, &value.unavailable_reason) {
        (Some(frame), None) => {
            if value.status_flags != 0 || frame.hour_index != value.frame_hour {
                return invalid("measured d84 frame has failure status or mismatched hour");
            }
            validate_scripted_gui_runtime_frame(frame).map_err(CompactError::Source)?;
            validate_compact_bounds(frame)
        }
        (None, Some(reason)) => {
            if value.status_flags == 0 || reason.trim().is_empty() {
                return invalid("unavailable d84 frame requires failure status and reason");
            }
            if portable_unavailable_diagnostic(reason) != *reason {
                return invalid("unavailable d84 reason contains pointer or host-path detail");
            }
            bounded_text(reason)?;
            Ok(())
        }
        _ => invalid("d84 availability and reason are inconsistent"),
    }
}

fn validate_compact_bounds(frame: &ScriptedGuiRuntimeFrame) -> Result<(), CompactError> {
    let mut result_count = 0usize;
    let mut text_bytes = frame.producer_record_identity.len();
    for row in &frame.rows {
        result_count = result_count
            .checked_add(2 + row.triggers.len() + row.properties.len() + row.effect_enabled.len())
            .ok_or_else(|| CompactError::Invalid("d84 result count overflow".into()))?;
        text_bytes = text_bytes
            .checked_add(row.country_tag.len())
            .and_then(|v| v.checked_add(row.gui_id.len()))
            .and_then(|v| v.checked_add(definition_text_bytes(&row.definition)))
            .ok_or_else(|| CompactError::Invalid("d84 text byte count overflow".into()))?;
        for (name, value) in &row.triggers {
            text_bytes = add_result_text(text_bytes, name.len(), value)?;
        }
        for ((element, property), value) in &row.properties {
            text_bytes = text_bytes
                .checked_add(element.len())
                .and_then(|v| v.checked_add(property.len()))
                .ok_or_else(|| CompactError::Invalid("d84 text byte count overflow".into()))?;
            text_bytes = add_result_text(text_bytes, 0, value)?;
        }
        for (name, value) in &row.effect_enabled {
            text_bytes = add_result_text(text_bytes, name.len(), value)?;
        }
    }
    for (tag, gui) in &frame.tombstones {
        text_bytes = text_bytes
            .checked_add(tag.len())
            .and_then(|v| v.checked_add(gui.len()))
            .ok_or_else(|| CompactError::Invalid("d84 text byte count overflow".into()))?;
    }
    if result_count > MAX_TOTAL_RESULTS || text_bytes > MAX_TOTAL_TEXT_BYTES {
        return invalid("d84 compact aggregate bound exceeded");
    }
    Ok(())
}

fn definition_text_bytes(value: &ScriptedGuiDefinitionIdentity) -> usize {
    value.logical_path.len()
        + value.layer_kind.len()
        + value.layer_id.len()
        + value.context_type.len()
        + value.window_name.len()
}

fn add_result_text<T>(
    current: usize,
    key_bytes: usize,
    value: &EngineResult<T>,
) -> Result<usize, CompactError>
where
    T: TextBytes,
{
    current
        .checked_add(key_bytes)
        .and_then(|v| match value {
            EngineResult::Value(value) => v.checked_add(value.text_bytes()),
            EngineResult::Unavailable(reason) => v.checked_add(reason.len()),
        })
        .ok_or_else(|| CompactError::Invalid("d84 text byte count overflow".into()))
}

trait TextBytes {
    fn text_bytes(&self) -> usize;
}
impl TextBytes for bool {
    fn text_bytes(&self) -> usize {
        0
    }
}
impl TextBytes for String {
    fn text_bytes(&self) -> usize {
        self.len()
    }
}

pub fn encode_payload(value: &ScriptedGuiRuntimeEnvelope) -> Result<Vec<u8>, CompactError> {
    validate_envelope(value)?;
    let mut out = Vec::new();
    u32v(&mut out, MAGIC);
    u32v(&mut out, SCHEMA_VERSION);
    u32v(&mut out, AUTHORITY_AUTHENTICATED_FRAME_SOURCE);
    u32v(&mut out, value.status_flags);
    u64v(&mut out, value.elapsed_wall_ms);
    u64v(&mut out, value.frame_hour);
    out.push(u8::from(value.frame.is_some()));
    out.extend_from_slice(&[0; 3]);
    if let Some(reason) = &value.unavailable_reason {
        string(&mut out, reason, false)?;
        return Ok(out);
    }
    let frame = value.frame.as_ref().expect("validated measured frame");
    string(&mut out, &frame.producer_record_identity, false)?;
    for hash in [
        frame.executable_sha256,
        frame.ui14_engine_vfs_sha256,
        frame.static_authority_receipt_sha256,
        frame.producer_manifest_sha256,
    ] {
        out.extend_from_slice(&hash);
    }
    u32v(&mut out, frame.rows.len() as u32);
    u32v(&mut out, frame.tombstones.len() as u32);
    for row in &frame.rows {
        encode_row(&mut out, row)?;
    }
    for (country_tag, gui_id) in &frame.tombstones {
        string(&mut out, country_tag, false)?;
        string(&mut out, gui_id, false)?;
    }
    Ok(out)
}

fn encode_row(out: &mut Vec<u8>, row: &ScriptedGuiRuntimeRow) -> Result<(), CompactError> {
    u32v(out, row.manager_order_index);
    string(out, &row.country_tag, false)?;
    string(out, &row.gui_id, false)?;
    let definition = &row.definition;
    string(out, &definition.logical_path, false)?;
    u32v(out, definition.winning_layer_ordinal);
    string(out, &definition.layer_kind, false)?;
    string(out, &definition.layer_id, false)?;
    out.extend_from_slice(&definition.source_sha256);
    out.extend_from_slice(&definition.payload_sha256);
    u64v(out, definition.payload_bytes);
    string(out, &definition.context_type, false)?;
    string(out, &definition.window_name, false)?;
    bool_result(out, &row.visible)?;
    bool_result(out, &row.enabled)?;
    u32v(out, row.triggers.len() as u32);
    for (name, result) in &row.triggers {
        string(out, name, false)?;
        bool_result(out, result)?;
    }
    u32v(out, row.properties.len() as u32);
    for ((element, property), result) in &row.properties {
        string(out, element, false)?;
        string(out, property, false)?;
        string_result(out, result)?;
    }
    u32v(out, row.effect_enabled.len() as u32);
    for (name, result) in &row.effect_enabled {
        string(out, name, false)?;
        bool_result(out, result)?;
    }
    Ok(())
}

fn bool_result(out: &mut Vec<u8>, value: &EngineResult<bool>) -> Result<(), CompactError> {
    match value {
        EngineResult::Value(value) => {
            out.push(1);
            out.push(u8::from(*value));
        }
        EngineResult::Unavailable(reason) => {
            out.push(0);
            string(out, reason, false)?;
        }
    }
    Ok(())
}

fn string_result(out: &mut Vec<u8>, value: &EngineResult<String>) -> Result<(), CompactError> {
    match value {
        EngineResult::Value(value) => {
            out.push(1);
            string(out, value, true)?;
        }
        EngineResult::Unavailable(reason) => {
            out.push(0);
            string(out, reason, false)?;
        }
    }
    Ok(())
}

pub fn decode_payload(payload: &[u8]) -> Result<ScriptedGuiRuntimeEnvelope, CompactError> {
    let mut reader = Reader { payload, at: 0 };
    if reader.u32()? != MAGIC
        || reader.u32()? != SCHEMA_VERSION
        || reader.u32()? != AUTHORITY_AUTHENTICATED_FRAME_SOURCE
    {
        return invalid("SGR1 magic/schema/authority mismatch");
    }
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    let has_frame = reader.bool()?;
    if reader.bytes(3)? != [0, 0, 0] {
        return invalid("SGR1 availability reserved bytes are nonzero");
    }
    let (frame, unavailable_reason) = if has_frame {
        let producer_record_identity = reader.string()?;
        let executable_sha256 = reader.hash()?;
        let ui14_engine_vfs_sha256 = reader.hash()?;
        let static_authority_receipt_sha256 = reader.hash()?;
        let producer_manifest_sha256 = reader.hash()?;
        let row_count = reader.count(crate::scripted_gui_runtime_carrier::MAX_GUI_ROWS)?;
        let tombstone_count = reader.count(crate::scripted_gui_runtime_carrier::MAX_GUI_ROWS)?;
        let mut rows = Vec::with_capacity(row_count);
        let mut expected_gui_keys = BTreeSet::new();
        for _ in 0..row_count {
            let row = decode_row(&mut reader)?;
            if !expected_gui_keys.insert((row.country_tag.clone(), row.gui_id.clone())) {
                return invalid("SGR1 duplicate country/gui row");
            }
            rows.push(row);
        }
        let mut tombstones = BTreeSet::new();
        for _ in 0..tombstone_count {
            if !tombstones.insert((reader.string()?, reader.string()?)) {
                return invalid("SGR1 duplicate tombstone");
            }
        }
        (
            Some(ScriptedGuiRuntimeFrame {
                hour_index: frame_hour,
                producer_record_identity,
                executable_sha256,
                ui14_engine_vfs_sha256,
                static_authority_receipt_sha256,
                producer_manifest_sha256,
                expected_gui_keys,
                rows,
                tombstones,
            }),
            None,
        )
    } else {
        (None, Some(reader.string()?))
    };
    if reader.at != payload.len() {
        return Err(CompactError::TrailingBytes);
    }
    let value = ScriptedGuiRuntimeEnvelope {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        frame,
        unavailable_reason,
    };
    validate_envelope(&value)?;
    Ok(value)
}

fn decode_row(reader: &mut Reader<'_>) -> Result<ScriptedGuiRuntimeRow, CompactError> {
    let manager_order_index = reader.u32()?;
    let country_tag = reader.string()?;
    let gui_id = reader.string()?;
    let definition = ScriptedGuiDefinitionIdentity {
        logical_path: reader.string()?,
        winning_layer_ordinal: reader.u32()?,
        layer_kind: reader.string()?,
        layer_id: reader.string()?,
        source_sha256: reader.hash()?,
        payload_sha256: reader.hash()?,
        payload_bytes: reader.u64()?,
        context_type: reader.string()?,
        window_name: reader.string()?,
    };
    let visible = reader.bool_result()?;
    let enabled = reader.bool_result()?;
    let trigger_count = reader.count(crate::scripted_gui_runtime_carrier::MAX_RESULTS_PER_MAP)?;
    let mut triggers = BTreeMap::new();
    for _ in 0..trigger_count {
        if triggers
            .insert(reader.string()?, reader.bool_result()?)
            .is_some()
        {
            return invalid("SGR1 duplicate trigger result");
        }
    }
    let property_count = reader.count(crate::scripted_gui_runtime_carrier::MAX_RESULTS_PER_MAP)?;
    let mut properties = BTreeMap::new();
    for _ in 0..property_count {
        if properties
            .insert(
                (reader.string()?, reader.string()?),
                reader.string_result()?,
            )
            .is_some()
        {
            return invalid("SGR1 duplicate property result");
        }
    }
    let effect_count = reader.count(crate::scripted_gui_runtime_carrier::MAX_RESULTS_PER_MAP)?;
    let mut effect_enabled = BTreeMap::new();
    for _ in 0..effect_count {
        if effect_enabled
            .insert(reader.string()?, reader.bool_result()?)
            .is_some()
        {
            return invalid("SGR1 duplicate effect result");
        }
    }
    Ok(ScriptedGuiRuntimeRow {
        manager_order_index,
        country_tag,
        gui_id,
        definition,
        visible,
        enabled,
        expected_trigger_names: triggers.keys().cloned().collect(),
        triggers,
        expected_property_keys: properties.keys().cloned().collect(),
        properties,
        expected_effect_names: effect_enabled.keys().cloned().collect(),
        effect_enabled,
    })
}

fn bounded_text(value: &str) -> Result<(), CompactError> {
    if value.len() > 65_536 || value.chars().any(|ch| ch == '\0') {
        invalid("SGR1 string is unbounded or contains NUL")
    } else {
        Ok(())
    }
}

fn string(out: &mut Vec<u8>, value: &str, allow_empty: bool) -> Result<(), CompactError> {
    bounded_text(value)?;
    if !allow_empty && value.is_empty() {
        return invalid("SGR1 required string is empty");
    }
    u32v(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CompactError> {
    Err(CompactError::Invalid(message.into()))
}
fn u32v(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u64v(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

struct Reader<'a> {
    payload: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> Result<&'a [u8], CompactError> {
        let end = self.at.checked_add(len).ok_or(CompactError::Truncated)?;
        let bytes = self
            .payload
            .get(self.at..end)
            .ok_or(CompactError::Truncated)?;
        self.at = end;
        Ok(bytes)
    }
    fn bool(&mut self) -> Result<bool, CompactError> {
        match self.bytes(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("SGR1 invalid bool byte"),
        }
    }
    fn u32(&mut self) -> Result<u32, CompactError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CompactError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn count(&mut self, max: usize) -> Result<usize, CompactError> {
        let count = self.u32()? as usize;
        if count > max {
            invalid("SGR1 collection count exceeds bound")
        } else {
            Ok(count)
        }
    }
    fn string(&mut self) -> Result<String, CompactError> {
        let len = self.count(65_536)?;
        String::from_utf8(self.bytes(len)?.to_vec())
            .map_err(|_| CompactError::Invalid("SGR1 string is not UTF-8".into()))
    }
    fn hash(&mut self) -> Result<[u8; 32], CompactError> {
        Ok(self.bytes(32)?.try_into().unwrap())
    }
    fn bool_result(&mut self) -> Result<EngineResult<bool>, CompactError> {
        match self.bytes(1)?[0] {
            0 => Ok(EngineResult::Unavailable(self.string()?)),
            1 => Ok(EngineResult::Value(self.bool()?)),
            _ => invalid("SGR1 invalid bool-result tag"),
        }
    }
    fn string_result(&mut self) -> Result<EngineResult<String>, CompactError> {
        match self.bytes(1)?[0] {
            0 => Ok(EngineResult::Unavailable(self.string()?)),
            1 => Ok(EngineResult::Value(self.string()?)),
            _ => invalid("SGR1 invalid string-result tag"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripted_gui_runtime_carrier::{
        EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256, EXACT_A729_EXECUTABLE_SHA256,
    };

    fn envelope() -> ScriptedGuiRuntimeEnvelope {
        let row = ScriptedGuiRuntimeRow {
            manager_order_index: 0,
            country_tag: "USA".into(),
            gui_id: "usa_congress".into(),
            definition: ScriptedGuiDefinitionIdentity {
                logical_path: "common/scripted_guis/USA_congress.txt".into(),
                winning_layer_ordinal: 2,
                layer_kind: "mod".into(),
                layer_id: "workshop:7".into(),
                source_sha256: [2; 32],
                payload_sha256: [3; 32],
                payload_bytes: 44,
                context_type: "country".into(),
                window_name: "congress_window".into(),
            },
            visible: EngineResult::Value(true),
            enabled: EngineResult::Unavailable("engine-only scope".into()),
            expected_trigger_names: BTreeSet::from(["senate_visible".into()]),
            triggers: BTreeMap::from([(
                "senate_visible".into(),
                EngineResult::Unavailable("engine-only trigger".into()),
            )]),
            expected_property_keys: BTreeSet::from([("icon".into(), "image".into())]),
            properties: BTreeMap::from([(
                ("icon".into(), "image".into()),
                EngineResult::Value("GFX_senate".into()),
            )]),
            expected_effect_names: BTreeSet::from(["invest".into()]),
            effect_enabled: BTreeMap::from([("invest".into(), EngineResult::Value(false))]),
        };
        ScriptedGuiRuntimeEnvelope {
            elapsed_wall_ms: 17,
            status_flags: 0,
            frame_hour: 99,
            frame: Some(ScriptedGuiRuntimeFrame {
                hour_index: 99,
                producer_record_identity: "a38:99:frame-1".into(),
                executable_sha256: EXACT_A729_EXECUTABLE_SHA256,
                ui14_engine_vfs_sha256: [4; 32],
                static_authority_receipt_sha256: EXACT20_A38_STATIC_AUTHORITY_RECEIPT_SHA256,
                producer_manifest_sha256: [6; 32],
                expected_gui_keys: BTreeSet::from([("USA".into(), "usa_congress".into())]),
                rows: vec![row],
                tombstones: BTreeSet::new(),
            }),
            unavailable_reason: None,
        }
    }

    #[test]
    fn roundtrip_preserves_order_definition_identity_and_unavailable_reasons() {
        let expected = envelope();
        let bytes = encode_payload(&expected).unwrap();
        assert_eq!(decode_payload(&bytes).unwrap(), expected);
    }

    #[test]
    fn unavailable_is_not_measured_empty_and_requires_failure_status() {
        assert!(encode_source_unavailable(1, 2, 0, "missing hook").is_err());
        let bytes = encode_source_unavailable(1, 2, 4, "missing hook").unwrap();
        let decoded = decode_payload(&bytes).unwrap();
        assert!(decoded.frame.is_none());
        assert_eq!(decoded.unavailable_reason.as_deref(), Some("missing hook"));

        let bytes = encode_source_unavailable(
            1,
            2,
            4,
            "failed at C:\\Users\\Example\\capture.bin 0x141234567",
        )
        .unwrap();
        let decoded = decode_payload(&bytes).unwrap();
        assert_eq!(
            decoded.unavailable_reason.as_deref(),
            Some("producer_diagnostic_redacted")
        );
    }
}
