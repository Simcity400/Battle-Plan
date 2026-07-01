#![forbid(unsafe_code)]

use std::collections::BTreeMap;

pub const DOMAIN_ID: u16 = 57;
pub const DOMAIN_NAME: &str = "session_multiplayer_metadata_live";

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const PAYLOAD_MAGIC: [u8; 4] = *b"S57M";
pub const PAYLOAD_VERSION: u16 = 1;
pub const PAYLOAD_HEADER_LEN: usize = 64;
pub const PLAYER_COUNTRY_ROW_LEN: usize = 40;
pub const SESSION_EVENT_ROW_LEN: usize = 64;

pub const STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_DELTA_DATA: u32 = 1 << 10;
pub const STATUS_KNOWN_MASK: u32 = STATUS_WORKER_PANIC
    | STATUS_BRIDGE_UNAVAILABLE
    | STATUS_SOURCE_UNAVAILABLE
    | STATUS_PARTIAL_DATA
    | STATUS_DELTA_DATA;

pub const SOURCE_FLAG_DLL_NATIVE: u32 = 1;
pub const SOURCE_FLAG_SESSION_ROOT_VALIDATED: u32 = 1 << 1;
pub const SOURCE_FLAG_COMMAND_HOOK_VALIDATED: u32 = 1 << 2;
pub const SOURCE_FLAG_MESSAGE_HOOK_VALIDATED: u32 = 1 << 3;
pub const SOURCE_FLAG_NOTIFICATION_HOOK_VALIDATED: u32 = 1 << 4;
pub const SOURCE_FLAG_PROOF_FIXTURE: u32 = 1 << 31;
pub const SOURCE_FLAG_KNOWN_MASK: u32 = SOURCE_FLAG_DLL_NATIVE
    | SOURCE_FLAG_SESSION_ROOT_VALIDATED
    | SOURCE_FLAG_COMMAND_HOOK_VALIDATED
    | SOURCE_FLAG_MESSAGE_HOOK_VALIDATED
    | SOURCE_FLAG_NOTIFICATION_HOOK_VALIDATED
    | SOURCE_FLAG_PROOF_FIXTURE;

pub const PLAYER_FLAG_HOST: u32 = 1;
pub const PLAYER_FLAG_HUMAN: u32 = 1 << 1;
pub const PLAYER_FLAG_CONNECTED: u32 = 1 << 2;
pub const PLAYER_FLAG_OBSERVER: u32 = 1 << 3;
pub const PLAYER_FLAG_KNOWN_MASK: u32 =
    PLAYER_FLAG_HOST | PLAYER_FLAG_HUMAN | PLAYER_FLAG_CONNECTED | PLAYER_FLAG_OBSERVER;

pub const EVENT_FLAG_DIRECT_CAPTURE: u32 = 1;
pub const EVENT_FLAG_UI_VISIBLE: u32 = 1 << 1;
pub const EVENT_FLAG_HOST_COMMAND: u32 = 1 << 2;
pub const EVENT_FLAG_TEXT_PRESENT: u32 = 1 << 3;
pub const EVENT_FLAG_KNOWN_MASK: u32 = EVENT_FLAG_DIRECT_CAPTURE
    | EVENT_FLAG_UI_VISIBLE
    | EVENT_FLAG_HOST_COMMAND
    | EVENT_FLAG_TEXT_PRESENT;

pub const MAX_TAG_DB_ID: u32 = 4096;
pub const MAX_STRING_BLOB_BYTES: usize = 256 * 1024;
pub const MAX_PLAYER_ROWS: u32 = 4096;
pub const MAX_EVENT_ROWS: u32 = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u16)]
pub enum SessionEventKind {
    Hotjoin = 1,
    TagSwitch = 2,
    GameSpeed = 3,
    Pause = 4,
    Chat = 5,
    SystemMessage = 6,
    Alert = 7,
    Notification = 8,
}

impl SessionEventKind {
    pub fn from_raw(raw: u16) -> Option<Self> {
        Some(match raw {
            1 => Self::Hotjoin,
            2 => Self::TagSwitch,
            3 => Self::GameSpeed,
            4 => Self::Pause,
            5 => Self::Chat,
            6 => Self::SystemMessage,
            7 => Self::Alert,
            8 => Self::Notification,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Hotjoin => "hotjoin",
            Self::TagSwitch => "tag_switch",
            Self::GameSpeed => "game_speed",
            Self::Pause => "pause",
            Self::Chat => "chat",
            Self::SystemMessage => "system_message",
            Self::Alert => "alert",
            Self::Notification => "notification",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerCountryRow {
    pub player_key_hash: u64,
    pub player_name: String,
    pub country_tag: String,
    pub tag_db_id: u32,
    pub session_slot: u32,
    pub flags: u32,
    pub valid_from_hour_raw: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEventRow {
    pub kind: SessionEventKind,
    pub flags: u32,
    pub actor_key_hash: u64,
    pub actor_name: String,
    pub text: String,
    pub tag_db_id: u32,
    pub value_i32: i32,
    pub related_value_i32: i32,
    pub command_sequence: u64,
    pub event_hour_raw: i32,
    pub event_source_id: u32,
    pub event_state: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionMetadataFrame {
    pub game_hour_raw: i32,
    pub status_flags: u32,
    pub source_flags: u32,
    pub schema_flags: u32,
    pub root_evidence_mask: u32,
    pub player_country_rows: Vec<PlayerCountryRow>,
    pub event_rows: Vec<SessionEventRow>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PayloadSummary {
    pub game_hour_raw: i32,
    pub status_flags: u32,
    pub source_flags: u32,
    pub player_country_rows: u32,
    pub event_rows: u32,
    pub string_bytes: u32,
    pub min_event_hour_raw: Option<i32>,
    pub max_event_hour_raw: Option<i32>,
    pub event_kind_counts: BTreeMap<u16, u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamGate {
    pub ok: bool,
    pub record_count: u64,
    pub domain_data_records: u64,
    pub frame_end_records: u64,
    pub domain_perf_records: u64,
    pub first_hour_raw: Option<i32>,
    pub last_hour_raw: Option<i32>,
    pub skipped_hours: u64,
    pub zero_skipped_hours: bool,
    pub partial_data_seen: bool,
    pub source_unavailable_seen: bool,
    pub worker_panic_seen: bool,
    pub bridge_unavailable_seen: bool,
    pub payload_summary: PayloadSummary,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PayloadGate {
    pub ok: bool,
    pub summary: PayloadSummary,
    pub errors: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StringRef {
    off: u32,
    len: u16,
}

#[derive(Default)]
struct StringTable {
    bytes: Vec<u8>,
}

impl StringTable {
    fn push(&mut self, value: &str) -> StringRef {
        let bytes = value.as_bytes();
        let len = bytes.len().min(u16::MAX as usize);
        let off = self.bytes.len().min(u32::MAX as usize) as u32;
        self.bytes.extend_from_slice(&bytes[..len]);
        StringRef {
            off,
            len: len as u16,
        }
    }
}

pub fn encode_payload(frame: &SessionMetadataFrame) -> Vec<u8> {
    let mut strings = StringTable::default();
    let player_refs = frame
        .player_country_rows
        .iter()
        .map(|row| {
            (
                strings.push(&row.player_name),
                strings.push(&row.country_tag),
            )
        })
        .collect::<Vec<_>>();
    let event_refs = frame
        .event_rows
        .iter()
        .map(|row| (strings.push(&row.actor_name), strings.push(&row.text)))
        .collect::<Vec<_>>();

    let player_count = frame.player_country_rows.len().min(u32::MAX as usize) as u32;
    let event_count = frame.event_rows.len().min(u32::MAX as usize) as u32;
    let string_bytes = strings.bytes.len().min(u32::MAX as usize) as u32;
    let min_event_hour = frame.event_rows.iter().map(|row| row.event_hour_raw).min();
    let max_event_hour = frame.event_rows.iter().map(|row| row.event_hour_raw).max();

    let mut out = Vec::with_capacity(
        PAYLOAD_HEADER_LEN
            + frame.player_country_rows.len() * PLAYER_COUNTRY_ROW_LEN
            + frame.event_rows.len() * SESSION_EVENT_ROW_LEN
            + strings.bytes.len(),
    );
    out.extend_from_slice(&PAYLOAD_MAGIC);
    out.extend_from_slice(&PAYLOAD_VERSION.to_le_bytes());
    out.extend_from_slice(&(PAYLOAD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&DOMAIN_ID.to_le_bytes());
    out.extend_from_slice(&(PLAYER_COUNTRY_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&(SESSION_EVENT_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&frame.game_hour_raw.to_le_bytes());
    out.extend_from_slice(&frame.status_flags.to_le_bytes());
    out.extend_from_slice(&frame.source_flags.to_le_bytes());
    out.extend_from_slice(&player_count.to_le_bytes());
    out.extend_from_slice(&event_count.to_le_bytes());
    out.extend_from_slice(&string_bytes.to_le_bytes());
    out.extend_from_slice(&frame.schema_flags.to_le_bytes());
    out.extend_from_slice(&frame.root_evidence_mask.to_le_bytes());
    out.extend_from_slice(&min_event_hour.unwrap_or(frame.game_hour_raw).to_le_bytes());
    out.extend_from_slice(&max_event_hour.unwrap_or(frame.game_hour_raw).to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    debug_assert_eq!(out.len(), PAYLOAD_HEADER_LEN);

    for (row, (player_name, country_tag)) in frame.player_country_rows.iter().zip(player_refs) {
        out.extend_from_slice(&row.player_key_hash.to_le_bytes());
        out.extend_from_slice(&player_name.off.to_le_bytes());
        out.extend_from_slice(&player_name.len.to_le_bytes());
        out.extend_from_slice(&country_tag.len.to_le_bytes());
        out.extend_from_slice(&country_tag.off.to_le_bytes());
        out.extend_from_slice(&row.tag_db_id.to_le_bytes());
        out.extend_from_slice(&row.session_slot.to_le_bytes());
        out.extend_from_slice(&row.flags.to_le_bytes());
        out.extend_from_slice(&row.valid_from_hour_raw.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }

    for (row, (actor_name, text)) in frame.event_rows.iter().zip(event_refs) {
        out.extend_from_slice(&(row.kind as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&row.flags.to_le_bytes());
        out.extend_from_slice(&row.actor_key_hash.to_le_bytes());
        out.extend_from_slice(&actor_name.off.to_le_bytes());
        out.extend_from_slice(&actor_name.len.to_le_bytes());
        out.extend_from_slice(&text.len.to_le_bytes());
        out.extend_from_slice(&text.off.to_le_bytes());
        out.extend_from_slice(&row.tag_db_id.to_le_bytes());
        out.extend_from_slice(&row.value_i32.to_le_bytes());
        out.extend_from_slice(&row.related_value_i32.to_le_bytes());
        out.extend_from_slice(&row.command_sequence.to_le_bytes());
        out.extend_from_slice(&row.event_hour_raw.to_le_bytes());
        out.extend_from_slice(&row.event_source_id.to_le_bytes());
        out.extend_from_slice(&row.event_state.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&strings.bytes);
    out
}

pub fn gate_payload(payload: &[u8]) -> PayloadGate {
    let mut errors = Vec::new();
    let mut summary = PayloadSummary::default();
    if payload.len() < PAYLOAD_HEADER_LEN {
        return PayloadGate {
            ok: false,
            summary,
            errors: vec!["payload shorter than header".to_string()],
        };
    }
    if &payload[0..4] != PAYLOAD_MAGIC {
        errors.push("invalid payload magic".to_string());
    }
    let version = read_u16(payload, 4);
    let header_len = read_u16(payload, 6) as usize;
    let domain_id = read_u16(payload, 8);
    let player_row_len = read_u16(payload, 10) as usize;
    let event_row_len = read_u16(payload, 12) as usize;
    if version != PAYLOAD_VERSION {
        errors.push(format!("payload version {version} != {PAYLOAD_VERSION}"));
    }
    if header_len != PAYLOAD_HEADER_LEN {
        errors.push(format!(
            "payload header_len {header_len} != {PAYLOAD_HEADER_LEN}"
        ));
    }
    if domain_id != DOMAIN_ID {
        errors.push(format!("payload domain_id {domain_id} != {DOMAIN_ID}"));
    }
    if player_row_len != PLAYER_COUNTRY_ROW_LEN {
        errors.push("unexpected player-country row length".to_string());
    }
    if event_row_len != SESSION_EVENT_ROW_LEN {
        errors.push("unexpected session event row length".to_string());
    }
    summary.game_hour_raw = read_i32(payload, 16);
    summary.status_flags = read_u32(payload, 20);
    summary.source_flags = read_u32(payload, 24);
    summary.player_country_rows = read_u32(payload, 28);
    summary.event_rows = read_u32(payload, 32);
    summary.string_bytes = read_u32(payload, 36);
    let schema_flags = read_u32(payload, 40);
    let root_evidence_mask = read_u32(payload, 44);
    summary.min_event_hour_raw = Some(read_i32(payload, 48));
    summary.max_event_hour_raw = Some(read_i32(payload, 52));

    if summary.status_flags & !STATUS_KNOWN_MASK != 0 {
        errors.push(format!(
            "unknown status flags 0x{:X}",
            summary.status_flags & !STATUS_KNOWN_MASK
        ));
    }
    if summary.source_flags & !SOURCE_FLAG_KNOWN_MASK != 0 {
        errors.push(format!(
            "unknown source flags 0x{:X}",
            summary.source_flags & !SOURCE_FLAG_KNOWN_MASK
        ));
    }
    if summary.player_country_rows > MAX_PLAYER_ROWS {
        errors.push("too many player-country rows".to_string());
    }
    if summary.event_rows > MAX_EVENT_ROWS {
        errors.push("too many session event rows".to_string());
    }
    if summary.string_bytes as usize > MAX_STRING_BLOB_BYTES {
        errors.push("string blob too large".to_string());
    }
    if schema_flags != 0 {
        errors.push(format!("unknown schema_flags 0x{schema_flags:X}"));
    }
    if root_evidence_mask == 0
        && (summary.source_flags & SOURCE_FLAG_PROOF_FIXTURE == 0)
        && (summary.player_country_rows > 0 || summary.event_rows > 0)
    {
        errors.push("non-empty production payload lacks root evidence".to_string());
    }

    let player_bytes = (summary.player_country_rows as usize).saturating_mul(player_row_len);
    let event_bytes = (summary.event_rows as usize).saturating_mul(event_row_len);
    let strings_start = header_len
        .saturating_add(player_bytes)
        .saturating_add(event_bytes);
    let expected_len = strings_start.saturating_add(summary.string_bytes as usize);
    if expected_len != payload.len() {
        errors.push(format!(
            "payload length {} != expected {expected_len}",
            payload.len()
        ));
        return PayloadGate {
            ok: false,
            summary,
            errors,
        };
    }
    let strings = &payload[strings_start..];
    let mut off = header_len;
    for row_index in 0..summary.player_country_rows {
        let row = &payload[off..off + player_row_len];
        let player_name = StringRef {
            off: read_u32(row, 8),
            len: read_u16(row, 12),
        };
        let country_tag = StringRef {
            off: read_u32(row, 16),
            len: read_u16(row, 14),
        };
        let tag_db_id = read_u32(row, 20);
        let flags = read_u32(row, 28);
        validate_string_ref(strings, player_name, "player_name", row_index, &mut errors);
        let country =
            validate_string_ref(strings, country_tag, "country_tag", row_index, &mut errors);
        if country.as_deref().map(valid_country_tag).unwrap_or(false) == false {
            errors.push(format!("player row {row_index}: invalid country tag"));
        }
        if tag_db_id == 0 || tag_db_id > MAX_TAG_DB_ID {
            errors.push(format!(
                "player row {row_index}: invalid tag_db_id {tag_db_id}"
            ));
        }
        if flags & !PLAYER_FLAG_KNOWN_MASK != 0 {
            errors.push(format!(
                "player row {row_index}: unknown flags 0x{:X}",
                flags & !PLAYER_FLAG_KNOWN_MASK
            ));
        }
        off += player_row_len;
    }
    for row_index in 0..summary.event_rows {
        let row = &payload[off..off + event_row_len];
        let raw_kind = read_u16(row, 0);
        let kind = SessionEventKind::from_raw(raw_kind);
        let flags = read_u32(row, 4);
        let actor_name = StringRef {
            off: read_u32(row, 16),
            len: read_u16(row, 20),
        };
        let text = StringRef {
            off: read_u32(row, 24),
            len: read_u16(row, 22),
        };
        let tag_db_id = read_u32(row, 28);
        let value_i32 = read_i32(row, 32);
        validate_string_ref(strings, actor_name, "actor_name", row_index, &mut errors);
        let text_value = validate_string_ref(strings, text, "event_text", row_index, &mut errors);
        if kind.is_none() {
            errors.push(format!("event row {row_index}: unknown kind {raw_kind}"));
        }
        if flags & !EVENT_FLAG_KNOWN_MASK != 0 {
            errors.push(format!(
                "event row {row_index}: unknown flags 0x{:X}",
                flags & !EVENT_FLAG_KNOWN_MASK
            ));
        }
        if tag_db_id > MAX_TAG_DB_ID {
            errors.push(format!(
                "event row {row_index}: invalid tag_db_id {tag_db_id}"
            ));
        }
        if let Some(kind) = kind {
            *summary.event_kind_counts.entry(kind as u16).or_insert(0) += 1;
            match kind {
                SessionEventKind::GameSpeed if !(0..=5).contains(&value_i32) => {
                    errors.push(format!(
                        "event row {row_index}: game speed {value_i32} outside 0..5"
                    ));
                }
                SessionEventKind::Pause if value_i32 != 0 && value_i32 != 1 => {
                    errors.push(format!(
                        "event row {row_index}: pause value {value_i32} is not 0/1"
                    ));
                }
                SessionEventKind::Chat
                | SessionEventKind::SystemMessage
                | SessionEventKind::Alert
                | SessionEventKind::Notification
                    if text_value.as_deref().unwrap_or("").is_empty() =>
                {
                    errors.push(format!("event row {row_index}: {:?} lacks text", kind));
                }
                _ => {}
            }
        }
        off += event_row_len;
    }

    PayloadGate {
        ok: errors.is_empty(),
        summary,
        errors,
    }
}

pub fn append_compact_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
}

pub fn encode_domain57_data_record(sequence: u64, frame: &SessionMetadataFrame) -> Vec<u8> {
    let mut out = Vec::new();
    let payload = encode_payload(frame);
    append_compact_record(
        &mut out,
        sequence,
        frame.game_hour_raw,
        DOMAIN_ID,
        RECORD_TYPE_DOMAIN_DATA,
        &payload,
    );
    out
}

pub fn encode_domain57_stream(frames: &[SessionMetadataFrame]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut sequence = 1u64;
    for frame in frames {
        let payload = encode_payload(frame);
        append_compact_record(
            &mut out,
            sequence,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        sequence += 1;
        append_compact_record(
            &mut out,
            sequence,
            frame.game_hour_raw,
            1000,
            RECORD_TYPE_FRAME_END,
            &[],
        );
        sequence += 1;
    }
    out
}

pub fn gate_compact_stream(raw: &[u8]) -> StreamGate {
    let mut gate = StreamGate {
        ok: true,
        zero_skipped_hours: true,
        ..StreamGate::default()
    };
    let mut offset = 0usize;
    let mut previous_frame_hour = None;
    while offset < raw.len() {
        if raw.len() - offset < COMPACT_RECORD_HEADER_LEN {
            gate.errors
                .push(format!("offset {offset}: truncated compact record header"));
            break;
        }
        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            gate.errors
                .push(format!("offset {offset}: invalid compact record magic"));
            break;
        }
        let version = read_u16(header, 4);
        let header_len = read_u16(header, 6) as usize;
        let game_hour_raw = read_i32(header, 16);
        let domain_id = read_u16(header, 20);
        let record_type = read_u16(header, 22);
        let payload_len = read_u32(header, 24) as usize;
        if version != COMPACT_RECORD_VERSION || header_len != COMPACT_RECORD_HEADER_LEN {
            gate.errors
                .push(format!("offset {offset}: unsupported compact header"));
            break;
        }
        if read_u32(header, 28) != 0 {
            gate.errors.push(format!(
                "offset {offset}: compact reserved field is nonzero"
            ));
        }
        let record_len = header_len.saturating_add(payload_len);
        if record_len > raw.len().saturating_sub(offset) {
            gate.errors
                .push(format!("offset {offset}: truncated compact record payload"));
            break;
        }
        let payload = &raw[offset + header_len..offset + record_len];
        gate.record_count = gate.record_count.saturating_add(1);
        match record_type {
            RECORD_TYPE_DOMAIN_DATA if domain_id == DOMAIN_ID => {
                gate.domain_data_records = gate.domain_data_records.saturating_add(1);
                let payload_gate = gate_payload(payload);
                if !payload_gate.ok {
                    gate.errors.extend(
                        payload_gate
                            .errors
                            .iter()
                            .map(|err| format!("offset {offset}: {err}")),
                    );
                }
                merge_payload_summary(&mut gate.payload_summary, &payload_gate.summary);
                gate.partial_data_seen |=
                    payload_gate.summary.status_flags & STATUS_PARTIAL_DATA != 0;
                gate.source_unavailable_seen |=
                    payload_gate.summary.status_flags & STATUS_SOURCE_UNAVAILABLE != 0;
                gate.worker_panic_seen |=
                    payload_gate.summary.status_flags & STATUS_WORKER_PANIC != 0;
                gate.bridge_unavailable_seen |=
                    payload_gate.summary.status_flags & STATUS_BRIDGE_UNAVAILABLE != 0;
            }
            RECORD_TYPE_DOMAIN_PERF if domain_id == DOMAIN_ID => {
                gate.domain_perf_records = gate.domain_perf_records.saturating_add(1);
            }
            RECORD_TYPE_FRAME_END => {
                gate.frame_end_records = gate.frame_end_records.saturating_add(1);
                if let Some(previous) = previous_frame_hour {
                    let delta = game_hour_raw.saturating_sub(previous);
                    if delta > 1 {
                        gate.skipped_hours = gate
                            .skipped_hours
                            .saturating_add(delta.saturating_sub(1) as u64);
                    }
                }
                previous_frame_hour = Some(game_hour_raw);
                gate.first_hour_raw.get_or_insert(game_hour_raw);
                gate.last_hour_raw = Some(game_hour_raw);
            }
            RECORD_TYPE_DOMAIN_DATA | RECORD_TYPE_DOMAIN_PERF => {}
            other => gate
                .errors
                .push(format!("offset {offset}: unknown record type {other}")),
        }
        offset += record_len;
    }
    if offset != raw.len() {
        gate.errors
            .push("compact stream ended before all bytes were consumed".to_string());
    }
    if gate.domain_data_records == 0 {
        gate.errors.push("no domain57 data records".to_string());
    }
    gate.zero_skipped_hours = gate.skipped_hours == 0;
    gate.ok = gate.errors.is_empty();
    gate
}

pub mod collector_module {
    use super::*;

    pub const COLLECTOR_MODULE_NAME: &str = "session_multiplayer_metadata_live.collector";

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct SessionMetadataInputs {
        pub game_hour_raw: i32,
        pub source_flags: u32,
        pub root_evidence_mask: u32,
        pub player_country_rows: Vec<PlayerCountryRow>,
        pub event_rows: Vec<SessionEventRow>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct CollectorOutput {
        pub frame: SessionMetadataFrame,
        pub payload: Vec<u8>,
        pub gate: PayloadGate,
    }

    pub fn collect_domain57(inputs: SessionMetadataInputs) -> CollectorOutput {
        let mut status_flags = 0u32;
        let source_flags = inputs.source_flags | SOURCE_FLAG_DLL_NATIVE;
        if inputs.root_evidence_mask == 0
            && source_flags & SOURCE_FLAG_PROOF_FIXTURE == 0
            && (!inputs.player_country_rows.is_empty() || !inputs.event_rows.is_empty())
        {
            status_flags |= STATUS_SOURCE_UNAVAILABLE;
        }
        if source_flags & SOURCE_FLAG_SESSION_ROOT_VALIDATED == 0
            && !inputs.player_country_rows.is_empty()
        {
            status_flags |= STATUS_PARTIAL_DATA;
        }
        let has_command_rows = inputs.event_rows.iter().any(|row| {
            matches!(
                row.kind,
                SessionEventKind::GameSpeed | SessionEventKind::Pause | SessionEventKind::TagSwitch
            )
        });
        if source_flags & SOURCE_FLAG_COMMAND_HOOK_VALIDATED == 0 && has_command_rows {
            status_flags |= STATUS_PARTIAL_DATA;
        }
        let has_message_rows = inputs.event_rows.iter().any(|row| {
            matches!(
                row.kind,
                SessionEventKind::Chat | SessionEventKind::SystemMessage
            )
        });
        if source_flags & SOURCE_FLAG_MESSAGE_HOOK_VALIDATED == 0 && has_message_rows {
            status_flags |= STATUS_PARTIAL_DATA;
        }
        let has_notification_rows = inputs.event_rows.iter().any(|row| {
            matches!(
                row.kind,
                SessionEventKind::Alert | SessionEventKind::Notification
            )
        });
        if source_flags & SOURCE_FLAG_NOTIFICATION_HOOK_VALIDATED == 0 && has_notification_rows {
            status_flags |= STATUS_PARTIAL_DATA;
        }

        let frame = SessionMetadataFrame {
            game_hour_raw: inputs.game_hour_raw,
            status_flags,
            source_flags,
            schema_flags: 0,
            root_evidence_mask: inputs.root_evidence_mask,
            player_country_rows: inputs.player_country_rows,
            event_rows: inputs.event_rows,
        };
        let payload = encode_payload(&frame);
        let gate = gate_payload(&payload);
        CollectorOutput {
            frame,
            payload,
            gate,
        }
    }
}

fn merge_payload_summary(total: &mut PayloadSummary, next: &PayloadSummary) {
    total.game_hour_raw = next.game_hour_raw;
    total.status_flags |= next.status_flags;
    total.source_flags |= next.source_flags;
    total.player_country_rows = total
        .player_country_rows
        .saturating_add(next.player_country_rows);
    total.event_rows = total.event_rows.saturating_add(next.event_rows);
    total.string_bytes = total.string_bytes.saturating_add(next.string_bytes);
    total.min_event_hour_raw = match (total.min_event_hour_raw, next.min_event_hour_raw) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (None, Some(b)) => Some(b),
        (a, None) => a,
    };
    total.max_event_hour_raw = match (total.max_event_hour_raw, next.max_event_hour_raw) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (None, Some(b)) => Some(b),
        (a, None) => a,
    };
    for (kind, count) in &next.event_kind_counts {
        *total.event_kind_counts.entry(*kind).or_insert(0) += count;
    }
}

fn validate_string_ref(
    strings: &[u8],
    field: StringRef,
    field_name: &str,
    row_index: u32,
    errors: &mut Vec<String>,
) -> Option<String> {
    let start = field.off as usize;
    let end = start.saturating_add(field.len as usize);
    if end > strings.len() {
        errors.push(format!(
            "row {row_index}: {field_name} string ref out of bounds"
        ));
        return None;
    }
    let slice = &strings[start..end];
    if slice.contains(&0) {
        errors.push(format!("row {row_index}: {field_name} contains NUL"));
        return None;
    }
    match std::str::from_utf8(slice) {
        Ok(value) => Some(value.to_string()),
        Err(_) => {
            errors.push(format!("row {row_index}: {field_name} is not utf-8"));
            None
        }
    }
}

fn valid_country_tag(value: &str) -> bool {
    value.len() == 3
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte == b'-')
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("u16 range"))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("i32 range"))
}

#[cfg(test)]
mod tests {
    use super::collector_module::{collect_domain57, SessionMetadataInputs};
    use super::*;

    #[test]
    fn domain_id_is_final_assigned_57() {
        assert_eq!(DOMAIN_ID, 57);
        assert_eq!(DOMAIN_NAME, "session_multiplayer_metadata_live");
    }

    #[test]
    fn payload_round_trips_through_gate() {
        let frame = sample_frame();
        let payload = encode_payload(&frame);
        let gate = gate_payload(&payload);
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.summary.player_country_rows, 2);
        assert_eq!(gate.summary.event_rows, 7);
        assert_eq!(
            gate.summary
                .event_kind_counts
                .get(&(SessionEventKind::Chat as u16)),
            Some(&1)
        );
    }

    #[test]
    fn compact_stream_gate_counts_domain_records_and_hours() {
        let mut later = sample_frame();
        later.game_hour_raw += 1;
        for row in &mut later.event_rows {
            row.event_hour_raw += 1;
        }
        let stream = encode_domain57_stream(&[sample_frame(), later]);
        let gate = gate_compact_stream(&stream);
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.domain_data_records, 2);
        assert_eq!(gate.frame_end_records, 2);
        assert!(gate.zero_skipped_hours);
        assert_eq!(gate.payload_summary.player_country_rows, 4);
    }

    #[test]
    fn gate_rejects_bad_speed_and_pause_values() {
        let mut frame = sample_frame();
        frame.event_rows.push(SessionEventRow {
            kind: SessionEventKind::GameSpeed,
            value_i32: 8,
            ..sample_event(SessionEventKind::GameSpeed, 0)
        });
        frame.event_rows.push(SessionEventRow {
            kind: SessionEventKind::Pause,
            value_i32: 2,
            ..sample_event(SessionEventKind::Pause, 0)
        });
        let gate = gate_payload(&encode_payload(&frame));
        assert!(!gate.ok);
        assert!(gate.errors.iter().any(|err| err.contains("game speed")));
        assert!(gate.errors.iter().any(|err| err.contains("pause value")));
    }

    #[test]
    fn collector_adapter_badges_missing_live_roots() {
        let output = collect_domain57(SessionMetadataInputs {
            game_hour_raw: 43_800_024,
            source_flags: 0,
            root_evidence_mask: 0,
            player_country_rows: sample_frame().player_country_rows,
            event_rows: Vec::new(),
        });
        assert!(output.frame.status_flags & STATUS_SOURCE_UNAVAILABLE != 0);
        assert!(output.frame.status_flags & STATUS_PARTIAL_DATA != 0);
        assert!(!output.gate.ok);
    }

    pub fn sample_frame() -> SessionMetadataFrame {
        SessionMetadataFrame {
            game_hour_raw: 43_800_024,
            status_flags: 0,
            source_flags: SOURCE_FLAG_DLL_NATIVE
                | SOURCE_FLAG_SESSION_ROOT_VALIDATED
                | SOURCE_FLAG_COMMAND_HOOK_VALIDATED
                | SOURCE_FLAG_MESSAGE_HOOK_VALIDATED
                | SOURCE_FLAG_NOTIFICATION_HOOK_VALIDATED
                | SOURCE_FLAG_PROOF_FIXTURE,
            schema_flags: 0,
            root_evidence_mask: 0x1F,
            player_country_rows: vec![
                PlayerCountryRow {
                    player_key_hash: 0xA11C_E001,
                    player_name: "host".to_string(),
                    country_tag: "GER".to_string(),
                    tag_db_id: 1,
                    session_slot: 0,
                    flags: PLAYER_FLAG_HOST | PLAYER_FLAG_HUMAN | PLAYER_FLAG_CONNECTED,
                    valid_from_hour_raw: 43_800_000,
                },
                PlayerCountryRow {
                    player_key_hash: 0xB22D_E002,
                    player_name: "ally".to_string(),
                    country_tag: "ITA".to_string(),
                    tag_db_id: 4,
                    session_slot: 1,
                    flags: PLAYER_FLAG_HUMAN | PLAYER_FLAG_CONNECTED,
                    valid_from_hour_raw: 43_800_000,
                },
            ],
            event_rows: vec![
                sample_event(SessionEventKind::Hotjoin, 0),
                sample_event(SessionEventKind::TagSwitch, 0),
                SessionEventRow {
                    kind: SessionEventKind::GameSpeed,
                    value_i32: 5,
                    ..sample_event(SessionEventKind::GameSpeed, 0)
                },
                SessionEventRow {
                    kind: SessionEventKind::Pause,
                    value_i32: 1,
                    ..sample_event(SessionEventKind::Pause, 0)
                },
                sample_event(SessionEventKind::Chat, EVENT_FLAG_TEXT_PRESENT),
                sample_event(SessionEventKind::Alert, EVENT_FLAG_TEXT_PRESENT),
                sample_event(SessionEventKind::Notification, EVENT_FLAG_TEXT_PRESENT),
            ],
        }
    }

    fn sample_event(kind: SessionEventKind, extra_flags: u32) -> SessionEventRow {
        SessionEventRow {
            kind,
            flags: EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_UI_VISIBLE | extra_flags,
            actor_key_hash: 0xA11C_E001,
            actor_name: "host".to_string(),
            text: match kind {
                SessionEventKind::Chat => "front ready?".to_string(),
                SessionEventKind::SystemMessage => "player joined".to_string(),
                SessionEventKind::Alert => "low manpower".to_string(),
                SessionEventKind::Notification => "war goal available".to_string(),
                _ => String::new(),
            },
            tag_db_id: 1,
            value_i32: 0,
            related_value_i32: 4,
            command_sequence: kind as u64,
            event_hour_raw: 43_800_024,
            event_source_id: kind as u32,
            event_state: 0,
        }
    }
}
