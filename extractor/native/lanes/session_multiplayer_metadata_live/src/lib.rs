#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(windows)]
pub mod x12_a729_hook;

use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, parse_compact_record_header,
    validate_compact_record_semantics, RecordFramingError,
};
use std::collections::BTreeMap;

pub const DOMAIN_ID: u16 = 57;
pub const DOMAIN_NAME: &str = "session_multiplayer_metadata_live";

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
pub const DOMAIN_FRAME: u16 = 1000;
pub const FRAME_END_PAYLOAD_LEN: usize = 32;

pub const PAYLOAD_MAGIC: [u8; 4] = *b"S57M";
pub const PAYLOAD_VERSION: u16 = 3;
pub const SCALAR_PAYLOAD_VERSION: u16 = 2;
pub const LEGACY_PAYLOAD_VERSION: u16 = 1;
pub const PAYLOAD_HEADER_LEN: usize = 64;
pub const PLAYER_COUNTRY_ROW_LEN: usize = 40;
pub const SESSION_EVENT_ROW_LEN: usize = 64;
pub const WORLD_TENSION_EXTENSION_MAGIC: [u8; 4] = *b"WT3X";
pub const WORLD_TENSION_EXTENSION_VERSION: u16 = 1;
pub const WORLD_TENSION_EXTENSION_HEADER_LEN: usize = 72;
pub const WORLD_TENSION_COUNTRY_ROW_LEN: usize = 56;
pub const WORLD_TENSION_LEDGER_ROW_LEN: usize = 40;

pub const STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_DELTA_DATA: u32 = 1 << 10;
pub const STATUS_KNOWN_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;

pub const SOURCE_FLAG_DLL_NATIVE: u32 = 1;
pub const SOURCE_FLAG_SESSION_ROOT_VALIDATED: u32 = 1 << 1;
pub const SOURCE_FLAG_COMMAND_HOOK_VALIDATED: u32 = 1 << 2;
pub const SOURCE_FLAG_MESSAGE_HOOK_VALIDATED: u32 = 1 << 3;
pub const SOURCE_FLAG_NOTIFICATION_HOOK_VALIDATED: u32 = 1 << 4;
pub const SOURCE_FLAG_SPEED_PAUSE_HOOK_VALIDATED: u32 = 1 << 5;
pub const SOURCE_FLAG_PROOF_FIXTURE: u32 = 1 << 31;
pub const SOURCE_FLAG_KNOWN_MASK: u32 = SOURCE_FLAG_DLL_NATIVE
    | SOURCE_FLAG_SESSION_ROOT_VALIDATED
    | SOURCE_FLAG_COMMAND_HOOK_VALIDATED
    | SOURCE_FLAG_MESSAGE_HOOK_VALIDATED
    | SOURCE_FLAG_NOTIFICATION_HOOK_VALIDATED
    | SOURCE_FLAG_SPEED_PAUSE_HOOK_VALIDATED
    | SOURCE_FLAG_PROOF_FIXTURE;

pub const SCHEMA_FLAG_WORLD_TENSION_FIXED5: u32 = 1;
pub const SCHEMA_FLAG_WORLD_TENSION_DETAILS: u32 = 1 << 1;
pub const SCHEMA_FLAG_KNOWN_MASK: u32 =
    SCHEMA_FLAG_WORLD_TENSION_FIXED5 | SCHEMA_FLAG_WORLD_TENSION_DETAILS;
pub const ROOT_EVIDENCE_WORLD_TENSION: u32 = 1 << 5;
pub const ROOT_EVIDENCE_WORLD_TENSION_DETAILS: u32 = 1 << 6;
pub const WORLD_TENSION_FIXED5_ABSENT: i64 = i64::MIN;

pub const WORLD_TENSION_COUNTRY_FLAG_POLICY_CAP_ENABLED: u32 = 1;
pub const WORLD_TENSION_COUNTRY_FLAG_KNOWN_MASK: u32 =
    WORLD_TENSION_COUNTRY_FLAG_POLICY_CAP_ENABLED;
pub const WORLD_TENSION_DETAILS_FLAG_INCREASED: u32 = 1;
pub const WORLD_TENSION_DETAILS_FLAG_KNOWN_MASK: u32 = WORLD_TENSION_DETAILS_FLAG_INCREASED;
pub const MAX_WORLD_TENSION_LEDGER_ROWS: u32 = 100_000;

pub fn player_key_hash(name: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in name.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x1000_0000_01B3);
    }
    hash
}

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
pub const EVENT_FLAG_TRANSITION_HOOK: u32 = 1 << 4;
pub const EVENT_FLAG_KNOWN_MASK: u32 = EVENT_FLAG_DIRECT_CAPTURE
    | EVENT_FLAG_UI_VISIBLE
    | EVENT_FLAG_HOST_COMMAND
    | EVENT_FLAG_TEXT_PRESENT
    | EVENT_FLAG_TRANSITION_HOOK;

pub const EVENT_SOURCE_X12_PRE_MUTATION_BASELINE: u32 = 0x1200;
pub const EVENT_SOURCE_X12_SPEED_SETTER: u32 = 0x1201;
pub const EVENT_SOURCE_X12_PAUSE_TOGGLE: u32 = 0x1202;
pub const EVENT_SOURCE_X12_PAUSE_FORCED: u32 = 0x1203;

pub const MAX_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;

pub const MAX_WORLD_TENSION_COUNTRY_COUNT: u32 = 65_536;

pub const MAX_WORLD_TENSION_COUNTRY_ROWS: u32 = i32::MAX as u32;
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
    pub world_tension_fixed5: Option<i64>,
    pub world_tension_details: Option<WorldTensionDetails>,
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
    pub world_tension_fixed5: Option<i64>,
    pub world_tension_country_rows: u32,
    pub world_tension_ledger_rows: u32,
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
    pub status_flags_or: u32,
    pub failure_status_flags_or: u32,
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

fn encode_world_tension_extension(details: &WorldTensionDetails) -> Vec<u8> {
    let mut strings = StringTable::default();
    let reason_refs = details
        .ledger_rows
        .iter()
        .map(|row| strings.push(&row.reason))
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    out.extend_from_slice(&WORLD_TENSION_EXTENSION_MAGIC);
    out.extend_from_slice(&WORLD_TENSION_EXTENSION_VERSION.to_le_bytes());
    out.extend_from_slice(&(WORLD_TENSION_EXTENSION_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&(WORLD_TENSION_COUNTRY_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&(WORLD_TENSION_LEDGER_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(
        &(details.country_rows.len().min(u32::MAX as usize) as u32).to_le_bytes(),
    );
    out.extend_from_slice(&(details.ledger_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&(strings.bytes.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(
        &(if details.increased {
            WORLD_TENSION_DETAILS_FLAG_INCREASED
        } else {
            0
        })
        .to_le_bytes(),
    );
    out.extend_from_slice(&details.current_fixed5.to_le_bytes());
    out.extend_from_slice(&details.prior_fixed5.to_le_bytes());
    out.extend_from_slice(&details.monthly_decrease_fixed5.to_le_bytes());
    out.extend_from_slice(&details.time_scale_fixed5.to_le_bytes());
    out.extend_from_slice(&details.start_date_raw.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    debug_assert_eq!(out.len(), WORLD_TENSION_EXTENSION_HEADER_LEN);
    for row in &details.country_rows {
        out.extend_from_slice(&row.tag_db_id.to_le_bytes());
        out.extend_from_slice(&row.flags.to_le_bytes());
        out.extend_from_slice(&row.generated_threat_fixed.to_le_bytes());
        out.extend_from_slice(&row.generate_wargoal_threshold_fixed5.to_le_bytes());
        out.extend_from_slice(&row.join_faction_threshold_fixed5.to_le_bytes());
        out.extend_from_slice(&row.lend_lease_threshold_fixed5.to_le_bytes());
        out.extend_from_slice(&row.send_volunteers_threshold_fixed5.to_le_bytes());
        out.extend_from_slice(&row.guarantee_threshold_fixed5.to_le_bytes());
    }
    for (row, reason) in details.ledger_rows.iter().zip(reason_refs) {
        out.extend_from_slice(&row.source_tag_db_id.to_le_bytes());
        out.extend_from_slice(&row.date_raw.to_le_bytes());
        out.extend_from_slice(&row.current_threat_fixed.to_le_bytes());
        out.extend_from_slice(&row.original_threat_fixed.to_le_bytes());
        out.extend_from_slice(&reason.off.to_le_bytes());
        out.extend_from_slice(&reason.len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&row.source_sequence.to_le_bytes());
    }
    out.extend_from_slice(&strings.bytes);
    out
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
    let schema_flags = frame.schema_flags
        | if frame.world_tension_fixed5.is_some() {
            SCHEMA_FLAG_WORLD_TENSION_FIXED5
        } else {
            0
        }
        | if frame.world_tension_details.is_some() {
            SCHEMA_FLAG_WORLD_TENSION_DETAILS
        } else {
            0
        };
    out.extend_from_slice(&schema_flags.to_le_bytes());
    let root_evidence_mask = frame.root_evidence_mask
        | if frame.world_tension_details.is_some() {
            ROOT_EVIDENCE_WORLD_TENSION_DETAILS
        } else {
            0
        };
    out.extend_from_slice(&root_evidence_mask.to_le_bytes());
    out.extend_from_slice(&min_event_hour.unwrap_or(frame.game_hour_raw).to_le_bytes());
    out.extend_from_slice(&max_event_hour.unwrap_or(frame.game_hour_raw).to_le_bytes());
    out.extend_from_slice(
        &frame
            .world_tension_fixed5
            .unwrap_or(WORLD_TENSION_FIXED5_ABSENT)
            .to_le_bytes(),
    );
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
    if let Some(details) = &frame.world_tension_details {
        out.extend_from_slice(&encode_world_tension_extension(details));
    }
    out
}

pub fn decode_world_tension_details(payload: &[u8]) -> Result<Option<WorldTensionDetails>, String> {
    if payload.len() < PAYLOAD_HEADER_LEN {
        return Err("payload shorter than header".into());
    }
    let version = read_u16(payload, 4);
    let schema_flags = read_u32(payload, 40);
    if schema_flags & SCHEMA_FLAG_WORLD_TENSION_DETAILS == 0 {
        return Ok(None);
    }
    if version != PAYLOAD_VERSION {
        return Err("world-tension details require payload v3".into());
    }
    let player_count = read_u32(payload, 28) as usize;
    let event_count = read_u32(payload, 32) as usize;
    let base_string_bytes = read_u32(payload, 36) as usize;
    let base_end = PAYLOAD_HEADER_LEN
        .checked_add(
            player_count
                .checked_mul(PLAYER_COUNTRY_ROW_LEN)
                .ok_or("player byte overflow")?,
        )
        .and_then(|v| v.checked_add(event_count.checked_mul(SESSION_EVENT_ROW_LEN)?))
        .and_then(|v| v.checked_add(base_string_bytes))
        .ok_or("base payload length overflow")?;
    let ext = payload
        .get(base_end..)
        .ok_or("world-tension extension offset out of bounds")?;
    if ext.len() < WORLD_TENSION_EXTENSION_HEADER_LEN {
        return Err("world-tension extension shorter than header".into());
    }
    if ext[0..4] != WORLD_TENSION_EXTENSION_MAGIC {
        return Err("invalid world-tension extension magic".into());
    }
    if read_u16(ext, 4) != WORLD_TENSION_EXTENSION_VERSION
        || read_u16(ext, 6) as usize != WORLD_TENSION_EXTENSION_HEADER_LEN
        || read_u16(ext, 8) as usize != WORLD_TENSION_COUNTRY_ROW_LEN
        || read_u16(ext, 10) as usize != WORLD_TENSION_LEDGER_ROW_LEN
    {
        return Err("unsupported world-tension extension layout".into());
    }
    let country_count = read_u32(ext, 12);
    let ledger_count = read_u32(ext, 16);
    let string_bytes = read_u32(ext, 20) as usize;
    let flags = read_u32(ext, 24);
    if country_count > MAX_WORLD_TENSION_COUNTRY_ROWS
        || ledger_count > MAX_WORLD_TENSION_LEDGER_ROWS
    {
        return Err("world-tension extension row count exceeds bound".into());
    }
    if string_bytes > MAX_STRING_BLOB_BYTES || flags & !WORLD_TENSION_DETAILS_FLAG_KNOWN_MASK != 0 {
        return Err("world-tension extension flags/string length invalid".into());
    }
    let country_bytes = (country_count as usize)
        .checked_mul(WORLD_TENSION_COUNTRY_ROW_LEN)
        .ok_or("world-tension country bytes overflow")?;
    let ledger_bytes = (ledger_count as usize)
        .checked_mul(WORLD_TENSION_LEDGER_ROW_LEN)
        .ok_or("world-tension ledger bytes overflow")?;
    let strings_start = WORLD_TENSION_EXTENSION_HEADER_LEN
        .checked_add(country_bytes)
        .and_then(|v| v.checked_add(ledger_bytes))
        .ok_or("world-tension extension row layout overflow")?;
    let expected_len = strings_start
        .checked_add(string_bytes)
        .ok_or("world-tension extension length overflow")?;
    if expected_len != ext.len() {
        return Err("world-tension extension length mismatch".into());
    }
    let current_fixed5 = read_i64(ext, 28);
    let prior_fixed5 = read_i64(ext, 36);
    let monthly_decrease_fixed5 = read_i64(ext, 44);
    let time_scale_fixed5 = read_i64(ext, 52);
    let start_date_raw = read_i32(ext, 60);
    if read_u32(ext, 64) != 0 || read_u32(ext, 68) != 0 {
        return Err("world-tension extension reserved bytes are non-zero".into());
    }
    if !(0..=100_000).contains(&current_fixed5) || !(0..=100_000).contains(&prior_fixed5) {
        return Err("world-tension extension scalar outside exact bounds".into());
    }
    let increased = flags & WORLD_TENSION_DETAILS_FLAG_INCREASED != 0;
    if increased != (prior_fixed5 < current_fixed5) {
        return Err("world-tension increase flag disagrees with prior/current values".into());
    }
    let mut country_rows = Vec::with_capacity(country_count as usize);
    let mut off = WORLD_TENSION_EXTENSION_HEADER_LEN;
    let mut previous_tag = 0u32;
    for _ in 0..country_count {
        let row = &ext[off..off + WORLD_TENSION_COUNTRY_ROW_LEN];
        let tag_db_id = read_u32(row, 0);
        let row_flags = read_u32(row, 4);
        let values = [
            read_i64(row, 8),
            read_i64(row, 16),
            read_i64(row, 24),
            read_i64(row, 32),
            read_i64(row, 40),
            read_i64(row, 48),
        ];
        if tag_db_id == 0 || tag_db_id > MAX_TAG_DB_ID || tag_db_id <= previous_tag {
            return Err("world-tension country ids are invalid or non-canonical".into());
        }
        if row_flags & !WORLD_TENSION_COUNTRY_FLAG_KNOWN_MASK != 0
            || !(0..=10_000_000).contains(&values[0])
            || values[1..].iter().any(|v| !(0..=100_000).contains(v))
        {
            return Err("world-tension country row outside exact bounds".into());
        }
        previous_tag = tag_db_id;
        country_rows.push(WorldTensionCountryRow {
            tag_db_id,
            flags: row_flags,
            generated_threat_fixed: values[0],
            generate_wargoal_threshold_fixed5: values[1],
            join_faction_threshold_fixed5: values[2],
            lend_lease_threshold_fixed5: values[3],
            send_volunteers_threshold_fixed5: values[4],
            guarantee_threshold_fixed5: values[5],
        });
        off += WORLD_TENSION_COUNTRY_ROW_LEN;
    }
    let strings = &ext[strings_start..];
    let mut ledger_rows = Vec::with_capacity(ledger_count as usize);
    for row_index in 0..ledger_count {
        let row = &ext[off..off + WORLD_TENSION_LEDGER_ROW_LEN];
        let source_tag_db_id = read_u32(row, 0);
        let current_threat_fixed = read_i64(row, 8);
        let original_threat_fixed = read_i64(row, 16);
        let reason_ref = StringRef {
            off: read_u32(row, 24),
            len: read_u16(row, 28),
        };
        let source_sequence = read_u64(row, 32);
        if read_u16(row, 30) != 0
            || source_tag_db_id == 0
            || source_tag_db_id > MAX_TAG_DB_ID
            || !(-10_000_000..=10_000_000).contains(&current_threat_fixed)
            || !(-10_000_000..=10_000_000).contains(&original_threat_fixed)
            || source_sequence != row_index as u64
        {
            return Err("world-tension ledger row outside exact bounds".into());
        }
        let start = reason_ref.off as usize;
        let end = start
            .checked_add(reason_ref.len as usize)
            .ok_or("reason overflow")?;
        let reason_bytes = strings.get(start..end).ok_or("reason out of bounds")?;
        if reason_bytes.contains(&0) {
            return Err("world-tension reason contains NUL".into());
        }
        let reason = std::str::from_utf8(reason_bytes)
            .map_err(|_| "world-tension reason is not utf-8")?
            .to_string();
        ledger_rows.push(WorldTensionLedgerRow {
            source_tag_db_id,
            date_raw: read_i32(row, 4),
            current_threat_fixed,
            original_threat_fixed,
            reason,
            source_sequence,
        });
        off += WORLD_TENSION_LEDGER_ROW_LEN;
    }
    Ok(Some(WorldTensionDetails {
        current_fixed5,
        prior_fixed5,
        increased,
        monthly_decrease_fixed5,
        time_scale_fixed5,
        start_date_raw,
        country_rows,
        ledger_rows,
    }))
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
    if version != PAYLOAD_VERSION
        && version != SCALAR_PAYLOAD_VERSION
        && version != LEGACY_PAYLOAD_VERSION
    {
        errors.push(format!(
            "payload version {version} is not current {PAYLOAD_VERSION}, scalar {SCALAR_PAYLOAD_VERSION}, or legacy {LEGACY_PAYLOAD_VERSION}"
        ));
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
    let world_tension_raw = read_i64(payload, 56);
    summary.world_tension_fixed5 = (version >= SCALAR_PAYLOAD_VERSION
        && world_tension_raw != WORLD_TENSION_FIXED5_ABSENT)
        .then_some(world_tension_raw);

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
    if schema_flags & !SCHEMA_FLAG_KNOWN_MASK != 0 {
        errors.push(format!(
            "unknown schema_flags 0x{:X}",
            schema_flags & !SCHEMA_FLAG_KNOWN_MASK
        ));
    }
    let tension_schema_present = schema_flags & SCHEMA_FLAG_WORLD_TENSION_FIXED5 != 0;
    let details_schema_present = schema_flags & SCHEMA_FLAG_WORLD_TENSION_DETAILS != 0;
    let tension_root_present = root_evidence_mask & ROOT_EVIDENCE_WORLD_TENSION != 0;
    let details_root_present = root_evidence_mask & ROOT_EVIDENCE_WORLD_TENSION_DETAILS != 0;
    if version == LEGACY_PAYLOAD_VERSION && (tension_schema_present || tension_root_present) {
        errors.push("legacy payload claims v2 world-tension evidence".to_string());
    }
    if version != PAYLOAD_VERSION && (details_schema_present || details_root_present) {
        errors.push("pre-v3 payload claims world-tension detail evidence".to_string());
    }
    if tension_root_present && summary.source_flags & SOURCE_FLAG_SESSION_ROOT_VALIDATED == 0 {
        errors.push("world-tension root lacks validated session-root source".to_string());
    }
    if tension_schema_present != summary.world_tension_fixed5.is_some()
        || tension_root_present != summary.world_tension_fixed5.is_some()
    {
        errors.push("world-tension value/schema/root evidence disagree".to_string());
    }
    if details_schema_present != details_root_present
        || details_schema_present && !tension_schema_present
    {
        errors.push("world-tension detail schema/root/scalar evidence disagree".to_string());
    }
    if summary.world_tension_fixed5.is_some_and(|value| {
        !(compact_contract::build_profile::DOMAIN57_WORLD_TENSION_FIXED5_MIN
            ..=compact_contract::build_profile::DOMAIN57_WORLD_TENSION_FIXED5_MAX)
            .contains(&value)
    }) {
        errors.push("world-tension fixed5 value outside 0..=100000".to_string());
    }
    if root_evidence_mask == 0
        && (summary.source_flags & SOURCE_FLAG_PROOF_FIXTURE == 0)
        && (summary.player_country_rows > 0 || summary.event_rows > 0)
    {
        errors.push("non-empty production payload lacks root evidence".to_string());
    }

    if header_len != PAYLOAD_HEADER_LEN
        || player_row_len != PLAYER_COUNTRY_ROW_LEN
        || event_row_len != SESSION_EVENT_ROW_LEN
        || summary.player_country_rows > MAX_PLAYER_ROWS
        || summary.event_rows > MAX_EVENT_ROWS
        || summary.string_bytes as usize > MAX_STRING_BLOB_BYTES
    {
        return PayloadGate {
            ok: false,
            summary,
            errors,
        };
    }

    let Some(player_bytes) = (summary.player_country_rows as usize).checked_mul(player_row_len)
    else {
        errors.push("player-country row byte length overflow".to_string());
        return PayloadGate {
            ok: false,
            summary,
            errors,
        };
    };
    let Some(event_bytes) = (summary.event_rows as usize).checked_mul(event_row_len) else {
        errors.push("session event row byte length overflow".to_string());
        return PayloadGate {
            ok: false,
            summary,
            errors,
        };
    };
    let Some(strings_start) = header_len
        .checked_add(player_bytes)
        .and_then(|offset| offset.checked_add(event_bytes))
    else {
        errors.push("payload row layout overflow".to_string());
        return PayloadGate {
            ok: false,
            summary,
            errors,
        };
    };
    let Some(base_expected_len) = strings_start.checked_add(summary.string_bytes as usize) else {
        errors.push("payload total length overflow".to_string());
        return PayloadGate {
            ok: false,
            summary,
            errors,
        };
    };
    if payload.len() < base_expected_len
        || (!details_schema_present && base_expected_len != payload.len())
    {
        errors.push(format!(
            "payload length {} is incompatible with base length {base_expected_len}",
            payload.len()
        ));
        return PayloadGate {
            ok: false,
            summary,
            errors,
        };
    }
    let strings = &payload[strings_start..base_expected_len];
    match decode_world_tension_details(payload) {
        Ok(Some(details)) => {
            summary.world_tension_country_rows =
                details.country_rows.len().min(u32::MAX as usize) as u32;
            summary.world_tension_ledger_rows =
                details.ledger_rows.len().min(u32::MAX as usize) as u32;
            if summary.world_tension_fixed5 != Some(details.current_fixed5) {
                errors
                    .push("world-tension detail current value disagrees with header scalar".into());
            }
        }
        Ok(None) if details_schema_present => {
            errors.push("world-tension detail schema is present but trailer is absent".into());
        }
        Ok(None) => {}
        Err(error) => errors.push(error),
    }
    let mut off = header_len;
    for row_index in 0..summary.player_country_rows {
        let Some(row_end) = off.checked_add(player_row_len) else {
            errors.push(format!("player row {row_index}: row offset overflow"));
            break;
        };
        let Some(row) = payload.get(off..row_end) else {
            errors.push(format!("player row {row_index}: row out of bounds"));
            break;
        };
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
        off = row_end;
    }
    let mut observed_min_event_hour_raw = None;
    let mut observed_max_event_hour_raw = None;
    let mut previous_x12_sequence = None;
    let mut saw_x12_baseline = false;
    for row_index in 0..summary.event_rows {
        let Some(row_end) = off.checked_add(event_row_len) else {
            errors.push(format!("event row {row_index}: row offset overflow"));
            break;
        };
        let Some(row) = payload.get(off..row_end) else {
            errors.push(format!("event row {row_index}: row out of bounds"));
            break;
        };
        let raw_kind = read_u16(row, 0);
        let kind = SessionEventKind::from_raw(raw_kind);
        let flags = read_u32(row, 4);
        let actor_key_hash = read_u64(row, 8);
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
        let related_value_i32 = read_i32(row, 36);
        let command_sequence = read_u64(row, 40);
        let event_hour_raw = read_i32(row, 48);
        let event_source_id = read_u32(row, 52);
        let event_state = read_u32(row, 56);
        observed_min_event_hour_raw = Some(
            observed_min_event_hour_raw
                .map_or(event_hour_raw, |current: i32| current.min(event_hour_raw)),
        );
        observed_max_event_hour_raw = Some(
            observed_max_event_hour_raw
                .map_or(event_hour_raw, |current: i32| current.max(event_hour_raw)),
        );
        let actor_name_value =
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
            if flags & EVENT_FLAG_TRANSITION_HOOK != 0 {
                let expected_source = match kind {
                    SessionEventKind::GameSpeed
                        if event_source_id == EVENT_SOURCE_X12_PRE_MUTATION_BASELINE =>
                    {
                        Some(EVENT_SOURCE_X12_PRE_MUTATION_BASELINE)
                    }
                    SessionEventKind::GameSpeed => Some(EVENT_SOURCE_X12_SPEED_SETTER),
                    SessionEventKind::Pause => match event_source_id {
                        EVENT_SOURCE_X12_PAUSE_TOGGLE | EVENT_SOURCE_X12_PAUSE_FORCED => {
                            Some(event_source_id)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let tick = f32::from_bits(event_state);
                let exact_shape = flags == (EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_TRANSITION_HOOK)
                    && summary.source_flags & SOURCE_FLAG_SPEED_PAUSE_HOOK_VALIDATED != 0
                    && actor_key_hash > 0
                    && actor_key_hash <= u32::MAX as u64
                    && actor_name_value.as_deref().unwrap_or("").is_empty()
                    && text_value.as_deref().unwrap_or("").is_empty()
                    && tag_db_id == 0
                    && if event_source_id == EVENT_SOURCE_X12_PRE_MUTATION_BASELINE {
                        command_sequence == 0
                            && !saw_x12_baseline
                            && previous_x12_sequence.is_none()
                    } else {
                        command_sequence > 0
                    }
                    && event_hour_raw <= summary.game_hour_raw
                    && expected_source == Some(event_source_id)
                    && tick.is_finite()
                    && (0.0..=1.0).contains(&tick)
                    && event_state != 0x8000_0000
                    && match kind {
                        SessionEventKind::GameSpeed => (0..=1).contains(&related_value_i32),
                        SessionEventKind::Pause => (0..=4).contains(&related_value_i32),
                        _ => false,
                    };
                if !exact_shape {
                    errors.push(format!(
                        "event row {row_index}: malformed X12 transition-hook row"
                    ));
                }
                if event_source_id == EVENT_SOURCE_X12_PRE_MUTATION_BASELINE {
                    saw_x12_baseline = true;
                    previous_x12_sequence = Some(0);
                } else if let Some(previous) = previous_x12_sequence {
                    if command_sequence != previous + 1 {
                        errors.push(format!(
                            "event row {row_index}: X12 sequence {command_sequence} does not follow {previous}"
                        ));
                    }
                    previous_x12_sequence = Some(command_sequence);
                } else {
                    previous_x12_sequence = Some(command_sequence);
                }
            }
            match kind {
                SessionEventKind::GameSpeed if !(0..=4).contains(&value_i32) => {
                    errors.push(format!(
                        "event row {row_index}: game speed {value_i32} outside 0..4"
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
        off = row_end;
    }

    let expected_min_event_hour_raw = observed_min_event_hour_raw.unwrap_or(summary.game_hour_raw);
    let expected_max_event_hour_raw = observed_max_event_hour_raw.unwrap_or(summary.game_hour_raw);
    if summary.min_event_hour_raw != Some(expected_min_event_hour_raw) {
        errors.push(format!(
            "declared min_event_hour_raw {:?} != decoded {expected_min_event_hour_raw}",
            summary.min_event_hour_raw
        ));
    }
    if summary.max_event_hour_raw != Some(expected_max_event_hour_raw) {
        errors.push(format!(
            "declared max_event_hour_raw {:?} != decoded {expected_max_event_hour_raw}",
            summary.max_event_hour_raw
        ));
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
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        );
        sequence += 1;
        append_compact_record(
            &mut out,
            sequence,
            frame.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0u8; FRAME_END_PAYLOAD_LEN],
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
    if let Err(error) =
        compact_contract::stream_validator::validate_freeze_census_stream_placement(raw)
    {
        gate.errors
            .push(format!("stateful stream placement: {error}"));
    }
    let mut offset = 0usize;
    let mut previous_frame_hour: Option<i32> = None;
    let mut previous_sequence: Option<u64> = None;
    let mut open_frame_hour = None;
    let mut saw_frame_end = false;
    let mut final_record_is_clean_world_boundary = false;
    let mut release_epilogue_seen = false;
    let mut frame_domain_data_records = 0u32;
    let mut frame_domain_perf_records = 0u32;
    while offset < raw.len() {
        let previous_record_closed_world = final_record_is_clean_world_boundary;
        final_record_is_clean_world_boundary = false;
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(_)) => {
                gate.errors
                    .push(format!("offset {offset}: truncated compact record header"));
                break;
            }
            Err(RecordFramingError::BadMagic) => {
                gate.errors
                    .push(format!("offset {offset}: invalid compact record magic"));
                break;
            }
            Err(
                RecordFramingError::UnsupportedVersion(_)
                | RecordFramingError::UnsupportedHeaderLen(_),
            ) => {
                gate.errors
                    .push(format!("offset {offset}: unsupported compact header"));
                break;
            }
            Err(error) => {
                gate.errors.push(format!("offset {offset}: {error}"));
                break;
            }
        };
        let game_hour_raw = header.game_hour_raw;
        let domain_id = header.domain_id;
        let record_type = header.record_type;
        if let Some(previous) = previous_sequence {
            match previous.checked_add(1) {
                Some(expected) if header.sequence == expected => {}
                Some(expected) => gate.errors.push(format!(
                    "offset {offset}: compact record sequence discontinuity: expected {expected}, got {}",
                    header.sequence
                )),
                None => gate.errors.push(format!(
                    "offset {offset}: compact record sequence overflow after {previous}; wrapping is forbidden"
                )),
            }
        } else if header.sequence != 1 {
            gate.errors.push(format!(
                "offset {offset}: first compact record sequence is {}, expected 1",
                header.sequence
            ));
        }
        previous_sequence = Some(header.sequence);
        let record_len = header.record_len();
        if record_len > raw.len().saturating_sub(offset) {
            gate.errors
                .push(format!("offset {offset}: truncated compact record payload"));
            break;
        }
        let payload = &raw[offset + COMPACT_RECORD_HEADER_LEN..offset + record_len];
        gate.record_count = gate.record_count.saturating_add(1);
        if let Err(error) =
            validate_compact_record_semantics(domain_id, record_type, header.record_flags, payload)
        {
            gate.errors.push(format!("offset {offset}: {error}"));
            offset += record_len;
            continue;
        }
        let known_record_type = match compact_contract::KnownRecordType::try_from(record_type) {
            Ok(record_type) => record_type,
            Err(unknown) => {
                gate.errors.push(format!(
                    "offset {offset}: unknown record type {}",
                    unknown.0
                ));
                offset += record_len;
                continue;
            }
        };
        match known_record_type {
            compact_contract::KnownRecordType::DomainData if domain_id == DOMAIN_ID => {
                observe_open_frame(&mut gate, &mut open_frame_hour, game_hour_raw, offset);
                gate.domain_data_records = gate.domain_data_records.saturating_add(1);
                frame_domain_data_records = frame_domain_data_records.saturating_add(1);
                let payload_gate = gate_payload(payload);
                if !payload_gate.ok {
                    gate.errors.extend(
                        payload_gate
                            .errors
                            .iter()
                            .map(|err| format!("offset {offset}: {err}")),
                    );
                }
                if payload_gate.summary.game_hour_raw != game_hour_raw {
                    gate.errors.push(format!(
                        "offset {offset}: domain57 payload hour {} does not match compact record hour {game_hour_raw}",
                        payload_gate.summary.game_hour_raw
                    ));
                }
                merge_payload_summary(&mut gate.payload_summary, &payload_gate.summary);
                observe_stream_status(&mut gate, payload_gate.summary.status_flags);
            }
            compact_contract::KnownRecordType::DomainPerf if domain_id == DOMAIN_ID => {
                observe_open_frame(&mut gate, &mut open_frame_hour, game_hour_raw, offset);
                gate.domain_perf_records = gate.domain_perf_records.saturating_add(1);
                frame_domain_perf_records = frame_domain_perf_records.saturating_add(1);
                observe_stream_status(&mut gate, read_u32(payload, 32));
            }
            compact_contract::KnownRecordType::FrameEnd => {
                saw_frame_end = true;
                gate.frame_end_records = gate.frame_end_records.saturating_add(1);
                observe_stream_status(&mut gate, read_u32(payload, 24));
                validate_frame_cardinality(
                    &mut gate,
                    game_hour_raw,
                    frame_domain_data_records,
                    frame_domain_perf_records,
                    offset,
                );
                match open_frame_hour {
                    Some(open) if open == game_hour_raw => open_frame_hour = None,
                    Some(open) => gate.errors.push(format!(
                        "offset {offset}: FRAME_END hour {game_hour_raw} does not close open frame hour {open}"
                    )),
                    None => gate.errors.push(format!(
                        "offset {offset}: FRAME_END hour {game_hour_raw} has no open frame"
                    )),
                }
                if let Some(previous) = previous_frame_hour {
                    match previous.checked_add(1) {
                        Some(expected) if game_hour_raw == expected => {}
                        Some(expected) => {
                            if game_hour_raw > expected {
                                gate.skipped_hours = gate.skipped_hours.saturating_add(
                                    (i64::from(game_hour_raw) - i64::from(expected)) as u64,
                                );
                            }
                            gate.errors.push(format!(
                                "offset {offset}: closed frame hour {game_hour_raw} is not consecutive after {previous} (expected {expected})"
                            ));
                        }
                        None => gate.errors.push(format!(
                            "offset {offset}: closed frame hour overflow after {previous}"
                        )),
                    }
                }
                previous_frame_hour = Some(game_hour_raw);
                gate.first_hour_raw.get_or_insert(game_hour_raw);
                gate.last_hour_raw = Some(game_hour_raw);
                frame_domain_data_records = 0;
                frame_domain_perf_records = 0;
                final_record_is_clean_world_boundary = true;
            }
            compact_contract::KnownRecordType::DomainData => {
                observe_open_frame(&mut gate, &mut open_frame_hour, game_hour_raw, offset);
            }
            compact_contract::KnownRecordType::DomainPerf => {
                observe_open_frame(&mut gate, &mut open_frame_hour, game_hour_raw, offset);
            }
            compact_contract::KnownRecordType::FreezeCensus => {
                if let Ok(census) =
                    compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload)
                {
                    final_record_is_clean_world_boundary = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(previous_record_closed_world, census.phase(), release_epilogue_seen);
                    release_epilogue_seen |= census.phase()
                        == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                }
            }
            compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StreamHeader => {}
            compact_contract::KnownRecordType::DomainGap
            | compact_contract::KnownRecordType::FrameRefused
            | compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => {
                gate.errors.push(format!(
                    "offset {offset}: record type {} is outside the session-metadata gate grammar",
                    known_record_type.canonical_name()
                ))
            }
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
    if !saw_frame_end {
        gate.errors.push("no FRAME_END record".to_string());
    }
    if let Some(open) = open_frame_hour {
        gate.errors
            .push(format!("compact stream ended with open frame hour {open}"));
    }
    if !final_record_is_clean_world_boundary {
        gate.errors
            .push("compact stream does not end at a clean FRAME_END boundary".to_string());
    }
    gate.zero_skipped_hours = gate.skipped_hours == 0;
    if gate.failure_status_flags_or != 0 {
        gate.errors.push(format!(
            "domain57 stream contains fatal status flags 0x{:X}",
            gate.failure_status_flags_or
        ));
    }
    let fatal_status_seen = gate.partial_data_seen
        || gate.source_unavailable_seen
        || gate.worker_panic_seen
        || gate.bridge_unavailable_seen;
    gate.ok = gate.errors.is_empty()
        && gate.zero_skipped_hours
        && !fatal_status_seen
        && gate.failure_status_flags_or == 0;
    gate
}

fn observe_stream_status(gate: &mut StreamGate, status_flags: u32) {
    gate.status_flags_or |= status_flags;
    gate.failure_status_flags_or |=
        compact_status_failure_flags_after_expected_partial_tolerance(DOMAIN_ID, status_flags);
    gate.partial_data_seen |= status_flags & STATUS_PARTIAL_DATA != 0;
    gate.source_unavailable_seen |= status_flags & STATUS_SOURCE_UNAVAILABLE != 0;
    gate.worker_panic_seen |= status_flags & STATUS_WORKER_PANIC != 0;
    gate.bridge_unavailable_seen |= status_flags & STATUS_BRIDGE_UNAVAILABLE != 0;
}

fn validate_frame_cardinality(
    gate: &mut StreamGate,
    game_hour_raw: i32,
    domain_data_records: u32,
    domain_perf_records: u32,
    offset: usize,
) {
    if domain_data_records != 1 {
        gate.errors.push(format!(
            "offset {offset}: closed frame hour {game_hour_raw} has {domain_data_records} domain57 DATA records, expected exactly 1"
        ));
    }
    if domain_perf_records != 1 {
        gate.errors.push(format!(
            "offset {offset}: closed frame hour {game_hour_raw} has {domain_perf_records} domain57 PERF records, expected exactly 1"
        ));
    }
}

fn observe_open_frame(
    gate: &mut StreamGate,
    open_frame_hour: &mut Option<i32>,
    game_hour_raw: i32,
    offset: usize,
) {
    match *open_frame_hour {
        Some(open) if open != game_hour_raw => gate.errors.push(format!(
            "offset {offset}: record hour {game_hour_raw} encountered before FRAME_END closed open frame hour {open}"
        )),
        Some(_) => {}
        None => *open_frame_hour = Some(game_hour_raw),
    }
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
                SessionEventKind::Hotjoin
                    | SessionEventKind::TagSwitch
                    | SessionEventKind::GameSpeed
                    | SessionEventKind::Pause
            ) && row.flags & EVENT_FLAG_TRANSITION_HOOK == 0
        });
        if source_flags & SOURCE_FLAG_COMMAND_HOOK_VALIDATED == 0 && has_command_rows {
            status_flags |= STATUS_PARTIAL_DATA;
        }
        let has_x12_rows = inputs
            .event_rows
            .iter()
            .any(|row| row.flags & EVENT_FLAG_TRANSITION_HOOK != 0);
        if source_flags & SOURCE_FLAG_SPEED_PAUSE_HOOK_VALIDATED == 0 && has_x12_rows {
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
            world_tension_fixed5: None,
            world_tension_details: None,
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
    total.world_tension_fixed5 = next.world_tension_fixed5;
    total.world_tension_country_rows = total
        .world_tension_country_rows
        .saturating_add(next.world_tension_country_rows);
    total.world_tension_ledger_rows = total
        .world_tension_ledger_rows
        .saturating_add(next.world_tension_ledger_rows);
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
    let Some(end) = start.checked_add(field.len as usize) else {
        errors.push(format!(
            "row {row_index}: {field_name} string ref out of bounds"
        ));
        return None;
    };
    let Some(slice) = strings.get(start..end) else {
        errors.push(format!(
            "row {row_index}: {field_name} string ref out of bounds"
        ));
        return None;
    };
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
    compact_contract::country_tag::is_engine_country_tag_or_no_country_sentinel(value)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("u16 range"))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32 range"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldTensionCountryRow {
    pub tag_db_id: u32,
    pub flags: u32,
    pub generated_threat_fixed: i64,
    pub generate_wargoal_threshold_fixed5: i64,
    pub join_faction_threshold_fixed5: i64,
    pub lend_lease_threshold_fixed5: i64,
    pub send_volunteers_threshold_fixed5: i64,
    pub guarantee_threshold_fixed5: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldTensionLedgerRow {
    pub source_tag_db_id: u32,
    pub date_raw: i32,
    pub current_threat_fixed: i64,
    pub original_threat_fixed: i64,
    pub reason: String,
    pub source_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldTensionDetails {
    pub current_fixed5: i64,
    pub prior_fixed5: i64,
    pub increased: bool,
    pub monthly_decrease_fixed5: i64,
    pub time_scale_fixed5: i64,
    pub start_date_raw: i32,
    pub country_rows: Vec<WorldTensionCountryRow>,
    pub ledger_rows: Vec<WorldTensionLedgerRow>,
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("u64 range"))
}

fn read_i64(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("i64 range"))
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
    fn payload_gate_rejects_declared_event_hour_extrema_that_disagree_with_rows() {
        let mut frame = sample_frame();
        frame.event_rows[0].event_hour_raw = frame.game_hour_raw - 3;
        frame.event_rows[1].event_hour_raw = frame.game_hour_raw + 5;
        let mut payload = encode_payload(&frame);
        payload[48..52].copy_from_slice(&(frame.game_hour_raw - 2).to_le_bytes());
        payload[52..56].copy_from_slice(&(frame.game_hour_raw + 4).to_le_bytes());

        let gate = gate_payload(&payload);

        assert!(!gate.ok);
        assert!(gate.errors.iter().any(|error| {
            error.contains("declared min_event_hour_raw")
                && error.contains(&(frame.game_hour_raw - 3).to_string())
        }));
        assert!(gate.errors.iter().any(|error| {
            error.contains("declared max_event_hour_raw")
                && error.contains(&(frame.game_hour_raw + 5).to_string())
        }));
    }

    #[test]
    fn payload_gate_enforces_game_hour_extrema_for_zero_event_rows() {
        let mut frame = sample_frame();
        frame.event_rows.clear();
        let payload = encode_payload(&frame);
        let gate = gate_payload(&payload);
        assert!(gate.ok, "{:?}", gate.errors);
        assert_eq!(gate.summary.min_event_hour_raw, Some(frame.game_hour_raw));
        assert_eq!(gate.summary.max_event_hour_raw, Some(frame.game_hour_raw));

        let mut malformed = payload;
        malformed[48..52].copy_from_slice(&(frame.game_hour_raw - 1).to_le_bytes());
        malformed[52..56].copy_from_slice(&(frame.game_hour_raw + 1).to_le_bytes());
        let malformed_gate = gate_payload(&malformed);
        assert!(!malformed_gate.ok);
        assert!(malformed_gate
            .errors
            .iter()
            .any(|error| error.contains("declared min_event_hour_raw")));
        assert!(malformed_gate
            .errors
            .iter()
            .any(|error| error.contains("declared max_event_hour_raw")));
    }

    #[test]
    fn payload_gate_rejects_noncanonical_row_lengths_before_row_decoding() {
        let mut frame = sample_frame();
        frame.player_country_rows.clear();
        frame.event_rows.clear();
        let canonical = encode_payload(&frame);

        let mut player_payload = canonical.clone();
        player_payload[10..12].copy_from_slice(&1u16.to_le_bytes());
        player_payload[28..32].copy_from_slice(&1u32.to_le_bytes());
        player_payload.push(0);
        let player_gate = gate_payload(&player_payload);
        assert!(!player_gate.ok);
        assert!(player_gate
            .errors
            .iter()
            .any(|error| error == "unexpected player-country row length"));

        let mut event_payload = canonical;
        event_payload[12..14].copy_from_slice(&1u16.to_le_bytes());
        event_payload[32..36].copy_from_slice(&1u32.to_le_bytes());
        event_payload.push(0);
        let event_gate = gate_payload(&event_payload);
        assert!(!event_gate.ok);
        assert!(event_gate
            .errors
            .iter()
            .any(|error| error == "unexpected session event row length"));
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
        assert_eq!(gate.domain_perf_records, 2);
        assert_eq!(gate.frame_end_records, 2);
        assert!(gate.zero_skipped_hours);
        assert_eq!(gate.payload_summary.player_country_rows, 4);
    }

    #[test]
    fn compact_stream_gate_rejects_record_sequence_gap() {
        let frame = sample_frame();
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            7,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(&frame),
        );
        append_compact_record(
            &mut stream,
            9,
            frame.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0u8; FRAME_END_PAYLOAD_LEN],
        );

        let gate = gate_compact_stream(&stream);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("sequence discontinuity: expected 8, got 9")));
    }

    #[test]
    fn compact_stream_gate_rejects_wrapping_record_sequence() {
        let frame = sample_frame();
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            u64::MAX,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(&frame),
        );
        append_compact_record(
            &mut stream,
            0,
            frame.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0u8; FRAME_END_PAYLOAD_LEN],
        );

        let gate = gate_compact_stream(&stream);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("wrapping is forbidden")));
    }

    #[test]
    fn compact_stream_gate_rejects_missing_sequence_one_prefix() {
        let frame = sample_frame();
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            2,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(&frame),
        );
        append_compact_record(
            &mut stream,
            3,
            frame.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0u8; FRAME_END_PAYLOAD_LEN],
        );

        let gate = gate_compact_stream(&stream);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("first compact record sequence is 2, expected 1")));
    }

    #[test]
    fn compact_stream_gate_rejects_perf_from_a_prior_hour() {
        let frame = sample_frame();
        let mut stream = Vec::new();
        append_record_with_flags(
            &mut stream,
            1,
            frame.game_hour_raw - 1,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_payload_with_trailer(
                compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC,
            ),
        );
        append_compact_record(
            &mut stream,
            2,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(&frame),
        );
        append_compact_record(
            &mut stream,
            3,
            frame.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0u8; FRAME_END_PAYLOAD_LEN],
        );

        let gate = gate_compact_stream(&stream);
        assert!(!gate.ok, "{:?}", gate.errors);
        assert!(gate.errors.iter().any(|error| {
            error.contains("record hour") && error.contains("before FRAME_END closed open frame")
        }));
    }

    #[test]
    fn compact_stream_gate_requires_one_target_data_and_perf_per_closed_frame() {
        let frame = sample_frame();
        let payload = encode_payload(&frame);

        let mut missing_perf = Vec::new();
        append_compact_record(
            &mut missing_perf,
            1,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        append_compact_record(
            &mut missing_perf,
            2,
            frame.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );
        let gate = gate_compact_stream(&missing_perf);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("0 domain57 PERF records, expected exactly 1")));

        let mut missing_data = Vec::new();
        append_compact_record(
            &mut missing_data,
            1,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_base_payload(),
        );
        append_compact_record(
            &mut missing_data,
            2,
            frame.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );
        let gate = gate_compact_stream(&missing_data);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("0 domain57 DATA records, expected exactly 1")));
    }

    #[test]
    fn compact_stream_gate_rejects_duplicate_target_data_and_perf() {
        let frame = sample_frame();
        let payload = encode_payload(&frame);
        let mut stream = Vec::new();
        for sequence in 1..=2 {
            append_compact_record(
                &mut stream,
                sequence,
                frame.game_hour_raw,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                &payload,
            );
        }
        for sequence in 3..=4 {
            append_compact_record(
                &mut stream,
                sequence,
                frame.game_hour_raw,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_PERF,
                &perf_base_payload(),
            );
        }
        append_compact_record(
            &mut stream,
            5,
            frame.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );

        let gate = gate_compact_stream(&stream);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("2 domain57 DATA records, expected exactly 1")));
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("2 domain57 PERF records, expected exactly 1")));
    }

    #[test]
    fn compact_stream_gate_binds_payload_hour_to_record_hour() {
        let frame = sample_frame();
        let outer_hour = frame.game_hour_raw + 1;
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            outer_hour,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(&frame),
        );
        append_compact_record(
            &mut stream,
            2,
            outer_hour,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_base_payload(),
        );
        append_compact_record(
            &mut stream,
            3,
            outer_hour,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );

        let gate = gate_compact_stream(&stream);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("payload hour") && error.contains("compact record hour")));
    }

    #[test]
    fn compact_stream_gate_rejects_gapped_and_regressed_closed_hours() {
        let first = sample_frame();
        for next_hour in [first.game_hour_raw + 2, first.game_hour_raw - 1] {
            let mut second = sample_frame();
            second.game_hour_raw = next_hour;
            for row in &mut second.event_rows {
                row.event_hour_raw = next_hour;
            }
            let gate = gate_compact_stream(&encode_domain57_stream(&[first.clone(), second]));
            assert!(!gate.ok, "next_hour={next_hour}, errors={:?}", gate.errors);
            assert!(gate
                .errors
                .iter()
                .any(|error| error.contains("is not consecutive")));
            assert_eq!(
                gate.skipped_hours,
                if next_hour > first.game_hour_raw + 1 {
                    1
                } else {
                    0
                }
            );
        }
    }

    #[test]
    fn compact_stream_gate_rejects_every_canonical_failure_from_each_status_origin() {
        let fatal_bits = [
            compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PROFILE_MISMATCH,
            compact_contract::COMPACT_STATUS_NO_CLOCK,
            compact_contract::COMPACT_STATUS_NO_SCALAR_SOURCE,
            compact_contract::COMPACT_STATUS_WORKER_PANIC,
            compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            compact_contract::COMPACT_STATUS_CONTROL_INVALID,
        ];
        for bit in fatal_bits {
            for statuses in [(bit, 0, 0), (0, bit, 0), (0, 0, bit)] {
                let gate = gate_compact_stream(&stream_with_status_origins(
                    statuses.0, statuses.1, statuses.2,
                ));
                assert!(!gate.ok, "bit=0x{bit:X}, errors={:?}", gate.errors);
                assert_ne!(gate.failure_status_flags_or & bit, 0);
                assert!(gate
                    .errors
                    .iter()
                    .any(|error| error.contains("fatal status flags")));
            }
        }
    }

    #[test]
    fn compact_stream_gate_accepts_canonical_informational_status_from_each_origin() {
        let informational_bits = [
            compact_contract::COMPACT_STATUS_WAR_BRIDGE_APPLIED,
            compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE,
            compact_contract::COMPACT_STATUS_DELTA_DATA,
            compact_contract::COMPACT_STATUS_TRANSITION_BASELINE_RESET,
            compact_contract::COMPACT_STATUS_WAR_SEGMENT_ABSENT,
            compact_contract::COMPACT_STATUS_SOURCE_EMPTY,
        ];
        for bit in informational_bits {
            for statuses in [(bit, 0, 0), (0, bit, 0), (0, 0, bit)] {
                let gate = gate_compact_stream(&stream_with_status_origins(
                    statuses.0, statuses.1, statuses.2,
                ));
                assert!(gate.ok, "bit=0x{bit:X}, errors={:?}", gate.errors);
                assert_eq!(gate.failure_status_flags_or, 0);
                assert_ne!(gate.status_flags_or & bit, 0);
            }
        }
    }

    #[test]
    fn compact_stream_gate_rejects_exact_data_record_without_frame_end() {
        let frame = sample_frame();
        let stream = encode_domain57_data_record(1, &frame);

        let gate = gate_compact_stream(&stream);
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error == "no FRAME_END record"));
        assert!(gate
            .errors
            .iter()
            .any(|error| { error == "compact stream does not end at a clean FRAME_END boundary" }));
    }

    #[test]
    fn compact_stream_gate_accepts_canonical_record_flags_in_their_contexts() {
        let frame = sample_frame();
        let mut stream = Vec::new();
        append_record_with_flags(
            &mut stream,
            1,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            &encode_payload(&frame),
        );
        append_record_with_flags(
            &mut stream,
            2,
            frame.game_hour_raw,
            45,
            RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &[],
        );
        append_record_with_flags(
            &mut stream,
            3,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_payload_with_trailer(
                compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC,
            ),
        );
        append_record_with_flags(
            &mut stream,
            4,
            frame.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &[0u8; FRAME_END_PAYLOAD_LEN],
        );

        let gate = gate_compact_stream(&stream);
        assert!(gate.ok, "{:?}", gate.errors);
    }

    #[test]
    fn compact_stream_gate_rejects_bit30_without_a_perf_trailer() {
        let mut stream = Vec::new();
        append_record_with_flags(
            &mut stream,
            1,
            sample_frame().game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_base_payload(),
        );

        let gate = gate_compact_stream(&stream);
        assert!(gate
            .errors
            .iter()
            .any(|error| { error.contains("DOMAIN_PERF diagnostic trailer header is truncated") }));
    }

    #[test]
    fn compact_stream_gate_rejects_unannounced_perf_trailer() {
        let mut stream = Vec::new();
        append_record_with_flags(
            &mut stream,
            1,
            sample_frame().game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload_with_trailer(
                compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC,
            ),
        );

        let gate = gate_compact_stream(&stream);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("unannounced trailer byte(s)")));
    }

    #[test]
    fn compact_stream_gate_rejects_malformed_announced_perf_trailer() {
        let mut stream = Vec::new();
        append_record_with_flags(
            &mut stream,
            1,
            sample_frame().game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_payload_with_trailer(b"BAD!"),
        );

        let gate = gate_compact_stream(&stream);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("diagnostic trailer magic is invalid")));
    }

    #[test]
    fn compact_stream_gate_accepts_valid_announced_perf_trailer() {
        let frame = sample_frame();
        let mut stream = Vec::new();
        append_record_with_flags(
            &mut stream,
            1,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &perf_payload_with_trailer(
                compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC,
            ),
        );
        append_compact_record(
            &mut stream,
            2,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(&frame),
        );
        append_compact_record(
            &mut stream,
            3,
            frame.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &[0; FRAME_END_PAYLOAD_LEN],
        );

        let gate = gate_compact_stream(&stream);
        assert!(gate.ok, "{:?}", gate.errors);
    }

    #[test]
    fn compact_stream_gate_rejects_unknown_and_context_invalid_record_flags() {
        for flags in [
            1 << 29,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
        ] {
            let frame = sample_frame();
            let mut stream = Vec::new();
            append_record_with_flags(
                &mut stream,
                1,
                frame.game_hour_raw,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                flags,
                &encode_payload(&frame),
            );
            append_record_with_flags(
                &mut stream,
                2,
                frame.game_hour_raw,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                0,
                &[0u8; FRAME_END_PAYLOAD_LEN],
            );

            let gate = gate_compact_stream(&stream);
            assert!(!gate.ok, "flags=0x{flags:08X}");
            assert!(
                gate.errors
                    .iter()
                    .any(|error| error.contains("contain rejected bits")),
                "flags=0x{flags:08X}, errors={:?}",
                gate.errors
            );
        }
    }

    #[test]
    fn compact_stream_gate_rejects_short_and_oversized_frame_end_payloads() {
        for payload_len in [FRAME_END_PAYLOAD_LEN - 1, FRAME_END_PAYLOAD_LEN + 1] {
            let frame = sample_frame();
            let mut stream = Vec::new();
            append_compact_record(
                &mut stream,
                1,
                frame.game_hour_raw,
                DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                &encode_payload(&frame),
            );
            append_compact_record(
                &mut stream,
                2,
                frame.game_hour_raw,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                &vec![0u8; payload_len],
            );

            let gate = gate_compact_stream(&stream);
            assert!(!gate.ok, "payload_len={payload_len}");
            assert!(
                gate.errors.iter().any(|error| {
                    error.contains(&format!(
                        "FRAME_END payload length {payload_len} is not {FRAME_END_PAYLOAD_LEN}"
                    ))
                }),
                "payload_len={payload_len}, errors={:?}",
                gate.errors
            );
        }
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
    fn gate_x12_transition_rows_require_authenticated_consecutive_order() {
        let mut frame = sample_frame();
        frame.source_flags |= SOURCE_FLAG_SPEED_PAUSE_HOOK_VALIDATED;
        let frame_hour = frame.game_hour_raw;
        frame.event_rows.extend([
            SessionEventRow {
                kind: SessionEventKind::GameSpeed,
                flags: EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_TRANSITION_HOOK,
                actor_key_hash: 7,
                actor_name: String::new(),
                text: String::new(),
                tag_db_id: 0,
                value_i32: 3,
                related_value_i32: 0,
                command_sequence: 1,
                event_hour_raw: frame_hour,
                event_source_id: EVENT_SOURCE_X12_SPEED_SETTER,
                event_state: 0x3E80_0000,
            },
            SessionEventRow {
                kind: SessionEventKind::Pause,
                flags: EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_TRANSITION_HOOK,
                actor_key_hash: 7,
                actor_name: String::new(),
                text: String::new(),
                tag_db_id: 0,
                value_i32: 1,
                related_value_i32: 3,
                command_sequence: 2,
                event_hour_raw: frame_hour,
                event_source_id: EVENT_SOURCE_X12_PAUSE_TOGGLE,
                event_state: 0x3E80_0000,
            },
        ]);
        let gate = gate_payload(&encode_payload(&frame));
        assert!(gate.ok, "{:?}", gate.errors);

        frame.event_rows.last_mut().unwrap().command_sequence = 3;
        let gate = gate_payload(&encode_payload(&frame));
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("does not follow")));
    }

    #[test]
    fn gate_x12_accepts_one_pre_mutation_baseline_before_sequence_one() {
        let mut frame = sample_frame();
        frame.source_flags |= SOURCE_FLAG_SPEED_PAUSE_HOOK_VALIDATED;
        let frame_hour = frame.game_hour_raw;
        frame.event_rows.extend([
            SessionEventRow {
                kind: SessionEventKind::GameSpeed,
                flags: EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_TRANSITION_HOOK,
                actor_key_hash: 7,
                actor_name: String::new(),
                text: String::new(),
                tag_db_id: 0,
                value_i32: 2,
                related_value_i32: 0,
                command_sequence: 0,
                event_hour_raw: frame_hour,
                event_source_id: EVENT_SOURCE_X12_PRE_MUTATION_BASELINE,
                event_state: 0x3E00_0000,
            },
            SessionEventRow {
                kind: SessionEventKind::GameSpeed,
                flags: EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_TRANSITION_HOOK,
                actor_key_hash: 7,
                actor_name: String::new(),
                text: String::new(),
                tag_db_id: 0,
                value_i32: 3,
                related_value_i32: 0,
                command_sequence: 1,
                event_hour_raw: frame_hour,
                event_source_id: EVENT_SOURCE_X12_SPEED_SETTER,
                event_state: 0x3E80_0000,
            },
        ]);
        let gate = gate_payload(&encode_payload(&frame));
        assert!(gate.ok, "{:?}", gate.errors);

        let baseline_index = frame.event_rows.len() - 2;
        frame.event_rows[baseline_index].command_sequence = 1;
        let gate = gate_payload(&encode_payload(&frame));
        assert!(!gate.ok);
        assert!(gate
            .errors
            .iter()
            .any(|error| error.contains("malformed X12")));
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

    #[test]
    fn collector_adapter_requires_player_event_source_for_hotjoin() {
        let output = collect_domain57(SessionMetadataInputs {
            game_hour_raw: 43_800_024,
            source_flags: SOURCE_FLAG_SESSION_ROOT_VALIDATED | SOURCE_FLAG_PROOF_FIXTURE,
            root_evidence_mask: 0x10,
            player_country_rows: Vec::new(),
            event_rows: vec![sample_event(SessionEventKind::Hotjoin, 0)],
        });
        assert!(output.frame.status_flags & STATUS_PARTIAL_DATA != 0);

        let validated = collect_domain57(SessionMetadataInputs {
            game_hour_raw: 43_800_024,
            source_flags: SOURCE_FLAG_SESSION_ROOT_VALIDATED
                | SOURCE_FLAG_COMMAND_HOOK_VALIDATED
                | SOURCE_FLAG_PROOF_FIXTURE,
            root_evidence_mask: 0x10,
            player_country_rows: Vec::new(),
            event_rows: vec![sample_event(SessionEventKind::Hotjoin, 0)],
        });
        assert_eq!(validated.frame.status_flags & STATUS_PARTIAL_DATA, 0);
        assert!(validated.gate.ok, "{:?}", validated.gate.errors);
    }

    #[test]
    fn world_tension_details_round_trip_signed_engine_contributions() {
        let mut frame = sample_frame();
        frame.world_tension_details = Some(WorldTensionDetails {
            current_fixed5: 42_000,
            prior_fixed5: 42_000,
            increased: false,
            monthly_decrease_fixed5: 1_000,
            time_scale_fixed5: 100_000,
            start_date_raw: 43_800_000,
            country_rows: Vec::new(),
            ledger_rows: vec![WorldTensionLedgerRow {
                source_tag_db_id: 1,
                date_raw: 60_827_497,
                current_threat_fixed: -44_465,
                original_threat_fixed: -44_465,
                reason: "exact_negative_contribution".to_string(),
                source_sequence: 0,
            }],
        });
        let payload = encode_payload(&frame);
        let decoded = decode_world_tension_details(&payload)
            .expect("signed engine contribution must decode")
            .expect("world-tension details must be present");
        assert_eq!(
            decoded.ledger_rows,
            frame
                .world_tension_details
                .as_ref()
                .expect("fixture details")
                .ledger_rows
        );
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
            root_evidence_mask: 0x1F | ROOT_EVIDENCE_WORLD_TENSION,
            world_tension_fixed5: Some(42_000),
            world_tension_details: None,
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
                    value_i32: 4,
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

    fn append_record_with_flags(
        stream: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) {
        let record_start = stream.len();
        append_compact_record(
            stream,
            sequence,
            game_hour_raw,
            domain_id,
            record_type,
            payload,
        );
        stream[record_start + 28..record_start + 32].copy_from_slice(&record_flags.to_le_bytes());
    }

    fn perf_base_payload() -> Vec<u8> {
        vec![0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN]
    }

    fn perf_payload_with_status(status_flags: u32) -> Vec<u8> {
        let mut payload = perf_base_payload();
        payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }

    fn frame_end_payload_with_status(status_flags: u32) -> [u8; FRAME_END_PAYLOAD_LEN] {
        let mut payload = [0; FRAME_END_PAYLOAD_LEN];
        payload[24..28].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }

    fn stream_with_status_origins(
        data_status: u32,
        perf_status: u32,
        frame_status: u32,
    ) -> Vec<u8> {
        let mut frame = sample_frame();
        frame.status_flags = data_status;
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(&frame),
        );
        append_compact_record(
            &mut stream,
            2,
            frame.game_hour_raw,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            &perf_payload_with_status(perf_status),
        );
        append_compact_record(
            &mut stream,
            3,
            frame.game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            &frame_end_payload_with_status(frame_status),
        );
        stream
    }

    fn perf_payload_with_trailer(magic: &[u8; 4]) -> Vec<u8> {
        let mut payload = perf_base_payload();
        payload.extend_from_slice(magic);
        payload.extend_from_slice(
            &compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes(),
        );
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }
}
