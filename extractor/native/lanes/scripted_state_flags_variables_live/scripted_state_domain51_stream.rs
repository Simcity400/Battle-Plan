#[path = "scripted_state_domain51_collector.rs"]
pub mod collector;
#[allow(dead_code)]
#[path = "../../compact_contract/src/build_profile.rs"]
mod compact_build_profile;
#[path = "../../compact_contract/src/record_header.rs"]
mod compact_record_header;
#[path = "../../compact_contract/src/record_semantics.rs"]
mod compact_record_semantics;
#[path = "../../compact_contract/src/status_flags_base.rs"]
mod compact_status_flags;
#[path = "scripted_state_domain51_gate.rs"]
pub mod gate;

extern crate self as compact_contract;
pub use compact_status_flags::*;

pub mod build_profile {
    pub use crate::compact_build_profile::*;
}

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::Path;

use compact_record_header::{parse_compact_record_header, RecordFramingError};
pub use compact_record_header::{
    COMPACT_RECORD_HEADER_LEN as H4CR_HEADER_LEN, COMPACT_RECORD_MAGIC as H4CR_MAGIC,
    COMPACT_RECORD_VERSION as H4CR_VERSION,
};
#[cfg(test)]
use compact_record_semantics::COMPACT_DOMAIN_PERF_BASE_LEN as DOMAIN_PERF_PAYLOAD_LEN;
use compact_record_semantics::{
    validate_compact_record_semantics, KnownRecordType,
    COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
};
#[cfg(test)]
const RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER: u32 = COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
pub const DOMAIN_FRAME: u16 = 1000;
pub const DOMAIN_DIAGNOSTIC_PROBE: u16 = 250;

const STATUS_FAILURE_MASK: u32 = compact_status_flags::COMPACT_STATUS_STATE_UNAVAILABLE
    | compact_status_flags::COMPACT_STATUS_PROFILE_MISMATCH
    | compact_status_flags::COMPACT_STATUS_NO_CLOCK
    | compact_status_flags::COMPACT_STATUS_NO_SCALAR_SOURCE
    | compact_status_flags::COMPACT_STATUS_WORKER_PANIC
    | compact_status_flags::COMPACT_STATUS_BRIDGE_UNAVAILABLE
    | compact_status_flags::COMPACT_STATUS_SOURCE_UNAVAILABLE
    | compact_status_flags::COMPACT_STATUS_PARTIAL_DATA
    | compact_status_flags::COMPACT_STATUS_CONTROL_INVALID;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct H4crRecord {
    pub offset: usize,
    pub sequence: u64,
    pub game_hour_raw: i32,
    pub domain_id: u16,
    pub record_type: u16,
    pub payload_len: usize,
    pub flags: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain51StreamReport {
    pub ok: bool,
    pub errors: Vec<String>,
    pub total_bytes: usize,
    pub record_count: u32,
    pub domain_perf_records: u32,
    pub domain_data_records: u32,
    pub frame_end_records: u32,
    pub domain51_perf_records: u32,
    pub domain51_data_records: u32,
    pub domain51_payload_bytes: u64,
    pub domain51_rows_total: u64,
    pub domain51_changed_rows_total: u64,
    pub status_flags_or: u32,
    pub partial_failure_seen: bool,
    pub source_unavailable_seen: bool,
    pub unresolved_delayed_events_badged: bool,
    pub unresolved_scripted_gui_badged: bool,
    pub domains_seen: Vec<u16>,
    pub unique_hours: Vec<i32>,
    pub skipped_hours: u32,
    pub minimum_game_hour_advancements: i64,
    pub required_closed_hours: u64,
    pub certified_game_hour_advancements: i64,
    pub terminal_frame_end_authoritative: bool,
    pub per_hour_domain51_perf: BTreeMap<i32, u32>,
    pub per_hour_domain51_data: BTreeMap<i32, u32>,
    pub per_hour_frame_end: BTreeMap<i32, u32>,
}

impl Domain51StreamReport {
    fn new(total_bytes: usize, minimum_game_hour_advancements: i64) -> Self {
        Self {
            ok: false,
            errors: Vec::new(),
            total_bytes,
            record_count: 0,
            domain_perf_records: 0,
            domain_data_records: 0,
            frame_end_records: 0,
            domain51_perf_records: 0,
            domain51_data_records: 0,
            domain51_payload_bytes: 0,
            domain51_rows_total: 0,
            domain51_changed_rows_total: 0,
            status_flags_or: 0,
            partial_failure_seen: false,
            source_unavailable_seen: false,
            unresolved_delayed_events_badged: false,
            unresolved_scripted_gui_badged: false,
            domains_seen: Vec::new(),
            unique_hours: Vec::new(),
            skipped_hours: 0,
            minimum_game_hour_advancements,
            required_closed_hours: minimum_game_hour_advancements
                .checked_add(1)
                .and_then(|value| u64::try_from(value).ok())
                .unwrap_or(0),
            certified_game_hour_advancements: 0,
            terminal_frame_end_authoritative: false,
            per_hour_domain51_perf: BTreeMap::new(),
            per_hour_domain51_data: BTreeMap::new(),
            per_hour_frame_end: BTreeMap::new(),
        }
    }

    pub fn to_json(&self) -> String {
        let errors = self
            .errors
            .iter()
            .map(|error| quote_json(error))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            concat!(
                "{{",
                "\"ok\":{},",
                "\"errors\":[{}],",
                "\"total_bytes\":{},",
                "\"record_count\":{},",
                "\"domain_perf_records\":{},",
                "\"domain_data_records\":{},",
                "\"frame_end_records\":{},",
                "\"domain51_perf_records\":{},",
                "\"domain51_data_records\":{},",
                "\"domain51_payload_bytes\":{},",
                "\"domain51_rows_total\":{},",
                "\"domain51_changed_rows_total\":{},",
                "\"status_flags_or\":{},",
                "\"partial_failure_seen\":{},",
                "\"source_unavailable_seen\":{},",
                "\"unresolved_delayed_events_badged\":{},",
                "\"unresolved_scripted_gui_badged\":{},",
                "\"domains_seen\":{},",
                "\"unique_hours\":{},",
                "\"skipped_hours\":{},",
                "\"minimum_game_hour_advancements\":{},",
                "\"required_closed_hours\":{},",
                "\"certified_game_hour_advancements\":{},",
                "\"terminal_frame_end_authoritative\":{},",
                "\"per_hour_domain51_perf\":{},",
                "\"per_hour_domain51_data\":{},",
                "\"per_hour_frame_end\":{}",
                "}}"
            ),
            self.ok,
            errors,
            self.total_bytes,
            self.record_count,
            self.domain_perf_records,
            self.domain_data_records,
            self.frame_end_records,
            self.domain51_perf_records,
            self.domain51_data_records,
            self.domain51_payload_bytes,
            self.domain51_rows_total,
            self.domain51_changed_rows_total,
            self.status_flags_or,
            self.partial_failure_seen,
            self.source_unavailable_seen,
            self.unresolved_delayed_events_badged,
            self.unresolved_scripted_gui_badged,
            u16_vec_json(&self.domains_seen),
            i32_vec_json(&self.unique_hours),
            self.skipped_hours,
            self.minimum_game_hour_advancements,
            self.required_closed_hours,
            self.certified_game_hour_advancements,
            self.terminal_frame_end_authoritative,
            i32_u32_map_json(&self.per_hour_domain51_perf),
            i32_u32_map_json(&self.per_hour_domain51_data),
            i32_u32_map_json(&self.per_hour_frame_end),
        )
    }
}

pub fn append_h4cr_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    flags: u32,
    payload: &[u8],
) {
    out.extend_from_slice(H4CR_MAGIC);
    out.extend_from_slice(&H4CR_VERSION.to_le_bytes());
    out.extend_from_slice(&(H4CR_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(payload);
}

pub fn build_domain51_h4cr_stream(frames: &[(i32, collector::Domain51CollectedFrame)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut sequence = 1u64;
    for (game_hour_raw, frame) in frames {
        let perf = domain_perf_payload(
            frame.observed_scopes,
            frame.row_count,
            frame.row_count,
            frame.row_count,
            frame.status_flags,
        );
        append_h4cr_record(
            &mut out,
            sequence,
            *game_hour_raw,
            collector::compact::DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf,
        );
        sequence = sequence
            .checked_add(1)
            .expect("domain51 stream sequence overflow");

        append_h4cr_record(
            &mut out,
            sequence,
            *game_hour_raw,
            frame.domain_id,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            &frame.payload,
        );
        sequence = sequence
            .checked_add(1)
            .expect("domain51 stream sequence overflow");

        // FRAME_END status is frame-global authority. Preserve canonical
        // global statuses such as DELTA_DATA while stripping only D51's
        // lane-local opaque-root/name badges.
        let frame_end = frame_end_payload(status_for_global_interpretation(
            collector::compact::DOMAIN_ID,
            frame.status_flags,
        ));
        append_h4cr_record(
            &mut out,
            sequence,
            *game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &frame_end,
        );
        sequence = sequence
            .checked_add(1)
            .expect("domain51 stream sequence overflow");
    }
    out
}

pub fn sample_domain51_frames() -> Vec<(i32, collector::Domain51CollectedFrame)> {
    vec![
        (
            43_800_024,
            collector::collect_domain51_frame(collector::Domain51FrameInput {
                elapsed_wall_ms: 12,
                variable_scopes: vec![
                    collector::VariableScopeRead {
                        scope_kind: collector::compact::SCOPE_GLOBAL,
                        owner_id: 0,
                        status: collector::ScopeReadStatus::Measured,
                        entries: vec![collector::VariableEntry {
                            key: "global.test_variable".to_string(),
                            value_raw: -100_000,
                        }],
                    },
                    collector::VariableScopeRead {
                        scope_kind: collector::compact::SCOPE_COUNTRY,
                        owner_id: 64,
                        status: collector::ScopeReadStatus::Measured,
                        entries: vec![collector::VariableEntry {
                            key: "country.test_support".to_string(),
                            value_raw: 5_250_000,
                        }],
                    },
                    collector::VariableScopeRead {
                        scope_kind: collector::compact::SCOPE_STATE,
                        owner_id: 123,
                        status: collector::ScopeReadStatus::Measured,
                        entries: vec![collector::VariableEntry {
                            key: "state.test_pressure".to_string(),
                            value_raw: 250_000,
                        }],
                    },
                ],
                flag_scopes: vec![
                    collector::FlagScopeRead {
                        scope_kind: collector::compact::SCOPE_GLOBAL,
                        owner_id: 0,
                        status: collector::ScopeReadStatus::Measured,
                        entries: vec![collector::FlagEntry {
                            key: Some("global_test_flag".to_string()),
                            token_id: Some(0x1001),
                            date_hour_raw: Some(1_936_010_101),
                            value: Some(1),
                        }],
                    },
                    collector::FlagScopeRead {
                        scope_kind: collector::compact::SCOPE_COUNTRY,
                        owner_id: 64,
                        status: collector::ScopeReadStatus::Measured,
                        entries: vec![
                            collector::FlagEntry {
                                key: Some("country_named_flag".to_string()),
                                token_id: Some(0x2002),
                                date_hour_raw: Some(1_936_010_201),
                                value: Some(1),
                            },
                            collector::FlagEntry {
                                key: None,
                                token_id: Some(60_658),
                                date_hour_raw: Some(1_936_052_001),
                                value: Some(1),
                            },
                        ],
                    },
                    collector::FlagScopeRead {
                        scope_kind: collector::compact::SCOPE_STATE,
                        owner_id: 123,
                        status: collector::ScopeReadStatus::MeasuredEmpty,
                        entries: Vec::new(),
                    },
                ],
                delayed_event_rows: Vec::new(),
                delayed_events: collector::OpaqueRootState::UnresolvedFixtureBlocked,
                scripted_gui_open_state: collector::OpaqueRootState::UnresolvedFixtureBlocked,
            }),
        ),
        (
            43_800_025,
            collector::collect_domain51_frame(collector::Domain51FrameInput {
                elapsed_wall_ms: 10,
                variable_scopes: vec![
                    collector::VariableScopeRead {
                        scope_kind: collector::compact::SCOPE_GLOBAL,
                        owner_id: 0,
                        status: collector::ScopeReadStatus::MeasuredEmpty,
                        entries: Vec::new(),
                    },
                    collector::VariableScopeRead {
                        scope_kind: collector::compact::SCOPE_COUNTRY,
                        owner_id: 64,
                        status: collector::ScopeReadStatus::Measured,
                        entries: vec![collector::VariableEntry {
                            key: "country.test_support".to_string(),
                            value_raw: 5_350_000,
                        }],
                    },
                    collector::VariableScopeRead {
                        scope_kind: collector::compact::SCOPE_STATE,
                        owner_id: 123,
                        status: collector::ScopeReadStatus::Measured,
                        entries: vec![collector::VariableEntry {
                            key: "state.test_pressure".to_string(),
                            value_raw: 225_000,
                        }],
                    },
                ],
                flag_scopes: vec![
                    collector::FlagScopeRead {
                        scope_kind: collector::compact::SCOPE_GLOBAL,
                        owner_id: 0,
                        status: collector::ScopeReadStatus::MeasuredEmpty,
                        entries: Vec::new(),
                    },
                    collector::FlagScopeRead {
                        scope_kind: collector::compact::SCOPE_COUNTRY,
                        owner_id: 64,
                        status: collector::ScopeReadStatus::Measured,
                        entries: vec![collector::FlagEntry {
                            key: Some("country_named_flag".to_string()),
                            token_id: Some(0x2002),
                            date_hour_raw: Some(1_936_010_201),
                            value: Some(1),
                        }],
                    },
                    collector::FlagScopeRead {
                        scope_kind: collector::compact::SCOPE_STATE,
                        owner_id: 123,
                        status: collector::ScopeReadStatus::MeasuredEmpty,
                        entries: Vec::new(),
                    },
                ],
                delayed_event_rows: Vec::new(),
                delayed_events: collector::OpaqueRootState::UnresolvedFixtureBlocked,
                scripted_gui_open_state: collector::OpaqueRootState::UnresolvedFixtureBlocked,
            }),
        ),
    ]
}

pub fn build_sample_domain51_h4cr_stream() -> Vec<u8> {
    build_domain51_h4cr_stream(&sample_domain51_frames())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PerfAuthority {
    observed_rows: u64,
    changed_rows: u64,
    units: u64,
    changed_units: u64,
    status: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DataAuthority {
    rows: u64,
    changed_rows: u64,
    status: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct OwnedHourAuthority {
    perf_count: u32,
    data_count: u32,
    perf: Option<PerfAuthority>,
    data: Option<DataAuthority>,
    scheduled_perf_record_count: u32,
    scheduled_data_record_count: u32,
    scheduled_perf_domains: BTreeSet<u16>,
    scheduled_data_domains: BTreeSet<u16>,
    global_perf_status_or: u32,
}

/// Verify domain 51 inside a complete collector stream.
///
/// `minimum_game_hour_advancements` is explicit certification authority: `N`
/// advancements require `N + 1` consecutive, healthy closed hours. The gate
/// never derives that requirement from a mutable manifest or from the stream.
pub fn verify_domain51_h4cr_stream(
    raw: &[u8],
    minimum_game_hour_advancements: i64,
) -> Domain51StreamReport {
    let mut report = Domain51StreamReport::new(raw.len(), minimum_game_hour_advancements);
    if minimum_game_hour_advancements <= 0 {
        report.errors.push(format!(
            "minimum_game_hour_advancements must be explicitly positive, got {minimum_game_hour_advancements}"
        ));
    } else if minimum_game_hour_advancements.checked_add(1).is_none() {
        report.errors.push(format!(
            "minimum_game_hour_advancements {minimum_game_hour_advancements} cannot be converted to an inclusive closed-hour requirement"
        ));
    }
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut pending_hour: Option<i32> = None;
    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut saw_frame_end = false;
    let mut terminal_frame_end_authoritative = false;
    let mut release_epilogue_seen = false;
    let mut owned = OwnedHourAuthority::default();
    let mut domains_seen = BTreeSet::new();

    while offset < raw.len() {
        // Any bytes after a previously authoritative FRAME_END make that
        // record non-terminal, even if the trailing bytes do not frame.
        let previous_record_closed_world = terminal_frame_end_authoritative;
        terminal_frame_end_authoritative = false;
        let remaining = raw.len().saturating_sub(offset);
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(actual)) => {
                report.errors.push(format!(
                    "offset {offset}: truncated H4CR header with {actual} byte(s) remaining"
                ));
                break;
            }
            Err(RecordFramingError::BadMagic) => {
                report
                    .errors
                    .push(format!("offset {offset}: invalid H4CR magic"));
                break;
            }
            Err(RecordFramingError::UnsupportedVersion(version)) => {
                report.errors.push(format!(
                    "offset {offset}: unsupported H4CR version {version}"
                ));
                break;
            }
            Err(RecordFramingError::UnsupportedHeaderLen(header_len)) => {
                report.errors.push(format!(
                    "offset {offset}: unsupported H4CR header length {header_len}"
                ));
                break;
            }
            Err(RecordFramingError::PayloadTooLarge(payload_len)) => {
                report.errors.push(format!(
                    "offset {offset}: payload length {payload_len} exceeds canonical H4CR bound"
                ));
                break;
            }
        };
        let record = H4crRecord {
            offset,
            sequence: header.sequence,
            game_hour_raw: header.game_hour_raw,
            domain_id: header.domain_id,
            record_type: header.record_type,
            payload_len: header.payload_len,
            flags: header.record_flags,
        };
        validate_sequence(
            &mut report.errors,
            previous_sequence,
            record.sequence,
            offset,
        );
        previous_sequence = Some(record.sequence);
        let record_len = header.record_len();
        if record_len > remaining {
            report.errors.push(format!(
                "offset {offset}: payload length {} exceeds remaining stream bytes",
                record.payload_len
            ));
            break;
        }
        let payload_start = offset + H4CR_HEADER_LEN;
        let payload_end = payload_start + record.payload_len;
        let payload = &raw[payload_start..payload_end];
        let semantics_valid = match validate_compact_record_semantics(
            record.domain_id,
            record.record_type,
            record.flags,
            payload,
        ) {
            Ok(()) => true,
            Err(error) => {
                report.errors.push(format!("offset {offset}: {error}"));
                false
            }
        };

        report.record_count = report.record_count.saturating_add(1);
        domains_seen.insert(record.domain_id);

        let known_record_type = match KnownRecordType::try_from(record.record_type) {
            Ok(record_type) => record_type,
            Err(error) => {
                report.errors.push(format!("offset {offset}: {error}"));
                offset = offset.saturating_add(record_len);
                continue;
            }
        };
        match known_record_type {
            KnownRecordType::DomainPerf if semantics_valid => {
                observe_perf_hour(
                    &mut report.errors,
                    &mut pending_hour,
                    open_hour,
                    last_closed_hour,
                    record.game_hour_raw,
                    offset,
                );
                report.domain_perf_records = report.domain_perf_records.saturating_add(1);
                let perf_status = read_u32_le(payload, 32);
                if record.domain_id != DOMAIN_DIAGNOSTIC_PROBE {
                    owned.scheduled_perf_record_count =
                        owned.scheduled_perf_record_count.saturating_add(1);
                    owned.scheduled_perf_domains.insert(record.domain_id);
                    owned.global_perf_status_or |=
                        status_for_global_interpretation(record.domain_id, perf_status);
                } else if record.flags & COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER == 0 {
                    report.errors.push(format!(
                        "offset {offset}: diagnostic probe domain 250 PERF lacks the canonical PDU1 trailer flag"
                    ));
                }
                validate_domain_status(&mut report.errors, record.domain_id, perf_status, offset);
                if record.domain_id == collector::compact::DOMAIN_ID {
                    report.domain51_perf_records = report.domain51_perf_records.saturating_add(1);
                    *report
                        .per_hour_domain51_perf
                        .entry(record.game_hour_raw)
                        .or_default() += 1;
                    owned.perf_count = owned.perf_count.saturating_add(1);
                    let authority = decode_perf_authority(payload);
                    report.status_flags_or |= authority.status;
                    update_status_bools(&mut report, authority.status);
                    if owned.perf.is_none() {
                        owned.perf = Some(authority);
                    }
                }
            }
            KnownRecordType::DomainData if semantics_valid => {
                observe_data_hour(
                    &mut report.errors,
                    pending_hour,
                    &mut open_hour,
                    last_closed_hour,
                    record.game_hour_raw,
                    offset,
                );
                report.domain_data_records = report.domain_data_records.saturating_add(1);
                if record.domain_id == DOMAIN_DIAGNOSTIC_PROBE {
                    report.errors.push(format!(
                        "offset {offset}: diagnostic probe domain 250 is PERF-only; DATA is forbidden"
                    ));
                } else {
                    owned.scheduled_data_record_count =
                        owned.scheduled_data_record_count.saturating_add(1);
                    owned.scheduled_data_domains.insert(record.domain_id);
                }
                if record.domain_id == collector::compact::DOMAIN_ID {
                    report.domain51_data_records = report.domain51_data_records.saturating_add(1);
                    report.domain51_payload_bytes = report
                        .domain51_payload_bytes
                        .saturating_add(record.payload_len as u64);
                    *report
                        .per_hour_domain51_data
                        .entry(record.game_hour_raw)
                        .or_default() += 1;
                    owned.data_count = owned.data_count.saturating_add(1);
                    let verdict = gate::gate_domain51_payload(offset, payload);
                    report.status_flags_or |= verdict.summary.status_flags_or;
                    update_status_bools(&mut report, verdict.summary.status_flags_or);
                    report.partial_failure_seen |= verdict.partial_failure;
                    report.unresolved_delayed_events_badged |= verdict.unresolved_delayed_events;
                    report.unresolved_scripted_gui_badged |= verdict.unresolved_scripted_gui;
                    report.domain51_rows_total = report
                        .domain51_rows_total
                        .saturating_add(verdict.summary.data_rows_total);
                    report.domain51_changed_rows_total = report
                        .domain51_changed_rows_total
                        .saturating_add(verdict.summary.data_changed_rows_total);
                    if owned.data.is_none() {
                        owned.data = Some(DataAuthority {
                            rows: verdict.summary.data_rows_total,
                            changed_rows: verdict.summary.data_changed_rows_total,
                            status: verdict.summary.status_flags_or,
                        });
                    }
                    if !verdict.accepted {
                        report.errors.extend(verdict.summary.parse_errors);
                    }
                }
            }
            KnownRecordType::FrameEnd => {
                saw_frame_end = true;
                report.frame_end_records = report.frame_end_records.saturating_add(1);
                *report
                    .per_hour_frame_end
                    .entry(record.game_hour_raw)
                    .or_default() += 1;
                let structurally_closed = validate_frame_end(
                    &mut report.errors,
                    semantics_valid,
                    open_hour,
                    last_closed_hour,
                    record.game_hour_raw,
                    offset,
                );
                if structurally_closed {
                    let errors_before = report.errors.len();
                    let frame_status =
                        validate_frame_end_payload(&mut report.errors, payload, offset, &owned);
                    validate_owned_authority(
                        &mut report.errors,
                        record.game_hour_raw,
                        offset,
                        &owned,
                    );
                    report.status_flags_or |= frame_status;
                    update_status_bools(&mut report, frame_status);
                    report.unique_hours.push(record.game_hour_raw);
                    open_hour = None;
                    pending_hour = None;
                    last_closed_hour = Some(record.game_hour_raw);
                    terminal_frame_end_authoritative = report.errors.len() == errors_before;
                    owned = OwnedHourAuthority::default();
                }
            }
            KnownRecordType::DomainPerf | KnownRecordType::DomainData => {}
            KnownRecordType::FreezeCensus => {
                if let Ok(census) = crate::freeze_census_record::FreezeCensusRecordV1::decode(payload) {
                    terminal_frame_end_authoritative = crate::freeze_census_record::release_epilogue_closes_stream_policy(crate::freeze_census_record::RELEASE_FREEZE_CENSUS_EPILOGUE_ENABLED, previous_record_closed_world, census.phase(), release_epilogue_seen);
                    release_epilogue_seen |= census.phase() == crate::freeze_census_record::FreezeCensusPhase::Release;
                }
            }
            KnownRecordType::StreamHeader
            | KnownRecordType::DomainGap
            | KnownRecordType::FrameRefused
            | KnownRecordType::FrameDegraded
            | KnownRecordType::CaptureTerminated
            | KnownRecordType::ClockUnreadableInterval
            | KnownRecordType::UntrustedClockRecovery => {}
        }
        offset = offset.saturating_add(record_len);
    }

    if !raw.is_empty() {
        if !saw_frame_end {
            report
                .errors
                .push("stream has no frame-end record".to_string());
        }
        if let Some(open) = open_hour {
            report
                .errors
                .push(format!("stream ended with open data hour {open}"));
        }
        if let Some(pending) = pending_hour.filter(|_| open_hour.is_none()) {
            report.errors.push(format!(
                "stream ended with pending perf hour {pending} without data/frame-end"
            ));
        }
        if !terminal_frame_end_authoritative {
            report
                .errors
                .push("stream does not end at a healthy authoritative terminal FRAME_END with no trailing records".to_string());
        }
    }

    report.domains_seen = domains_seen.into_iter().collect();
    let observed_frame_end_hours = report
        .per_hour_frame_end
        .keys()
        .copied()
        .collect::<Vec<_>>();
    report.skipped_hours = skipped_hours(&observed_frame_end_hours);
    if report.skipped_hours != 0 {
        report
            .errors
            .push(format!("skipped_hours {} != 0", report.skipped_hours));
    }
    for hour in &report.unique_hours {
        let perf = report
            .per_hour_domain51_perf
            .get(hour)
            .copied()
            .unwrap_or_default();
        let data = report
            .per_hour_domain51_data
            .get(hour)
            .copied()
            .unwrap_or_default();
        let frame = report
            .per_hour_frame_end
            .get(hour)
            .copied()
            .unwrap_or_default();
        if perf != 1 {
            report.errors.push(format!(
                "game_hour_raw {hour}: domain51 perf_record_count {perf} != 1"
            ));
        }
        if data != 1 {
            report.errors.push(format!(
                "game_hour_raw {hour}: domain51 data_record_count {data} != 1"
            ));
        }
        if frame != 1 {
            report.errors.push(format!(
                "game_hour_raw {hour}: frame_end_count {frame} != 1"
            ));
        }
    }
    report.certified_game_hour_advancements = match (
        report.unique_hours.first().copied(),
        report.unique_hours.last().copied(),
    ) {
        (Some(first), Some(last)) => i64::from(last) - i64::from(first),
        _ => 0,
    };
    if minimum_game_hour_advancements > 0 {
        let actual_closed_hours = report.unique_hours.len() as u64;
        if actual_closed_hours < report.required_closed_hours {
            report.errors.push(format!(
                "closed hour count {actual_closed_hours} is below required {} for {minimum_game_hour_advancements} advancements",
                report.required_closed_hours
            ));
        }
        if report.certified_game_hour_advancements < minimum_game_hour_advancements {
            report.errors.push(format!(
                "certified game-hour advancements {} is below required {minimum_game_hour_advancements}",
                report.certified_game_hour_advancements
            ));
        }
    }
    report.terminal_frame_end_authoritative = terminal_frame_end_authoritative;
    report.ok = report.errors.is_empty()
        && report.domain51_data_records > 0
        && !report.partial_failure_seen
        && !report.source_unavailable_seen
        && report.terminal_frame_end_authoritative;
    report
}

fn decode_perf_authority(payload: &[u8]) -> PerfAuthority {
    PerfAuthority {
        observed_rows: read_u32_le(payload, 16) as u64,
        changed_rows: read_u32_le(payload, 20) as u64,
        units: read_u32_le(payload, 24) as u64,
        changed_units: read_u32_le(payload, 28) as u64,
        status: read_u32_le(payload, 32),
    }
}

fn validate_sequence(errors: &mut Vec<String>, previous: Option<u64>, current: u64, offset: usize) {
    match previous {
        None if current != 1 => errors.push(format!(
            "offset {offset}: first sequence is {current}, expected 1"
        )),
        Some(previous) => match previous.checked_add(1) {
            Some(expected) if current != expected => errors.push(format!(
                "offset {offset}: sequence discontinuity: expected {expected}, got {current}"
            )),
            None => errors.push(format!(
                "offset {offset}: sequence cannot advance after {previous} without overflow"
            )),
            _ => {}
        },
        _ => {}
    }
}

fn validate_next_hour(
    errors: &mut Vec<String>,
    last_closed: Option<i32>,
    hour: i32,
    label: &str,
    offset: usize,
) {
    let Some(last) = last_closed else { return };
    match last.checked_add(1) {
        Some(expected) if hour == expected => {}
        Some(expected) => errors.push(format!(
            "offset {offset}: {label} hour {hour} follows closed hour {last}; expected {expected}"
        )),
        None => errors.push(format!(
            "offset {offset}: closed hour {last} exhausted; another record is forbidden"
        )),
    }
}

fn observe_perf_hour(
    errors: &mut Vec<String>,
    pending: &mut Option<i32>,
    open: Option<i32>,
    last_closed: Option<i32>,
    hour: i32,
    offset: usize,
) {
    if let Some(open) = open {
        if open != hour {
            errors.push(format!(
                "offset {offset}: domain perf hour {hour} does not match open data hour {open}"
            ));
        }
        return;
    }
    validate_next_hour(errors, last_closed, hour, "domain perf", offset);
    match *pending {
        Some(existing) if existing != hour => errors.push(format!(
            "offset {offset}: domain perf hour {hour} conflicts with pending hour {existing}"
        )),
        None => *pending = Some(hour),
        _ => {}
    }
}

fn observe_data_hour(
    errors: &mut Vec<String>,
    pending: Option<i32>,
    open: &mut Option<i32>,
    last_closed: Option<i32>,
    hour: i32,
    offset: usize,
) {
    match *open {
        Some(existing) if existing != hour => errors.push(format!(
            "offset {offset}: data hour {hour} encountered before frame-end closed hour {existing}"
        )),
        Some(_) => {}
        None => {
            validate_next_hour(errors, last_closed, hour, "domain data", offset);
            if let Some(pending) = pending {
                if pending != hour {
                    errors.push(format!(
                        "offset {offset}: domain data hour {hour} conflicts with pending perf hour {pending}"
                    ));
                }
            }
            *open = Some(hour);
        }
    }
}

fn validate_frame_end(
    errors: &mut Vec<String>,
    semantics_valid: bool,
    open: Option<i32>,
    last_closed: Option<i32>,
    hour: i32,
    offset: usize,
) -> bool {
    let mut clean = semantics_valid;
    match open {
        Some(existing) if existing == hour => {}
        Some(existing) => {
            clean = false;
            errors.push(format!(
                "offset {offset}: frame-end hour {hour} does not close open hour {existing}"
            ));
        }
        None => {
            clean = false;
            errors.push(format!(
                "offset {offset}: frame-end hour {hour} has no open data hour"
            ));
        }
    }
    if let Some(last) = last_closed {
        if last.checked_add(1) != Some(hour) {
            clean = false;
        }
    }
    clean
}

fn validate_domain_status(errors: &mut Vec<String>, domain_id: u16, status: u32, offset: usize) {
    let unknown = compact_status_flags::compact_status_unknown_flags_v1(status);
    if unknown != 0 {
        errors.push(format!(
            "offset {offset}: domain {domain_id} PERF status contains unknown v1 bits {unknown:#010x}"
        ));
    }
    let fatal = status_for_global_interpretation(domain_id, status) & STATUS_FAILURE_MASK;
    if fatal != 0 {
        errors.push(format!(
            "offset {offset}: domain {domain_id} PERF status contains fatal bits {fatal:#010x}"
        ));
    }
}

const fn status_for_global_interpretation(domain_id: u16, status: u32) -> u32 {
    let lane_local_mask = match domain_id {
        49 => 1 << 11,
        51 => compact_status_flags::DOMAIN51_LANE_LOCAL_STATUS_MASK,
        _ => 0,
    };
    status & !lane_local_mask
}

fn validate_frame_end_payload(
    errors: &mut Vec<String>,
    payload: &[u8],
    offset: usize,
    owned: &OwnedHourAuthority,
) -> u32 {
    let emitted = read_u32_le(payload, 16);
    let scheduled = read_u32_le(payload, 20);
    let status = read_u32_le(payload, 24);
    let reserved = read_u32_le(payload, 28);
    if emitted == 0 || emitted != scheduled {
        errors.push(format!(
            "offset {offset}: frame-end emitted/scheduled counts {emitted}/{scheduled} are not equal and nonzero"
        ));
    }
    if owned.scheduled_perf_domains != owned.scheduled_data_domains {
        errors.push(format!(
            "offset {offset}: frame domain roster mismatch: PERF={:?} DATA={:?}",
            owned.scheduled_perf_domains, owned.scheduled_data_domains
        ));
    }
    let observed_domains = owned.scheduled_perf_domains.len().min(u32::MAX as usize) as u32;
    if emitted != observed_domains {
        errors.push(format!(
            "offset {offset}: frame-end emitted count {emitted} != independently observed paired-domain count {observed_domains}"
        ));
    }
    if owned.scheduled_perf_record_count != observed_domains {
        errors.push(format!(
            "offset {offset}: non-probe PERF record count {} != distinct scheduled-domain count {observed_domains}",
            owned.scheduled_perf_record_count
        ));
    }
    if owned.scheduled_data_record_count != observed_domains {
        errors.push(format!(
            "offset {offset}: non-probe DATA record count {} != distinct scheduled-domain count {observed_domains}",
            owned.scheduled_data_record_count
        ));
    }
    if reserved != 0 {
        errors.push(format!(
            "offset {offset}: frame-end reserved word is nonzero"
        ));
    }
    let unknown = compact_status_flags::compact_status_unknown_flags_v1(status);
    if unknown != 0 {
        errors.push(format!(
            "offset {offset}: frame-end status contains unknown v1 bits {unknown:#010x}"
        ));
    }
    let fatal = status & STATUS_FAILURE_MASK;
    if fatal != 0 {
        errors.push(format!(
            "offset {offset}: frame-end status contains fatal bits {fatal:#010x}"
        ));
    }
    if status != owned.global_perf_status_or {
        errors.push(format!(
            "offset {offset}: frame-end status {status:#010x} does not equal closed-frame PERF status aggregate {:#010x}",
            owned.global_perf_status_or
        ));
    }
    status
}

fn validate_owned_authority(
    errors: &mut Vec<String>,
    hour: i32,
    offset: usize,
    owned: &OwnedHourAuthority,
) {
    if owned.perf_count != 1 {
        errors.push(format!(
            "offset {offset}: hour {hour} domain 51 perf_record_count {} != 1",
            owned.perf_count
        ));
    }
    if owned.data_count != 1 {
        errors.push(format!(
            "offset {offset}: hour {hour} domain 51 data_record_count {} != 1",
            owned.data_count
        ));
    }
    if let (1, 1, Some(perf), Some(data)) =
        (owned.perf_count, owned.data_count, owned.perf, owned.data)
    {
        // `observed_rows` is scope-read work and can legitimately exceed the
        // emitted DATA rows. The other three counters are DATA authority and
        // must retain their delta semantics exactly.
        let matches = perf.changed_rows == data.changed_rows
            && perf.units == data.rows
            && perf.changed_units == data.changed_rows
            && perf.status == data.status;
        if !matches {
            errors.push(format!(
                "offset {offset}: hour {hour} domain 51 PERF/DATA cardinality or status mismatch: perf={perf:?} data={data:?}"
            ));
        }
        let unknown = compact_status_flags::compact_status_unknown_flags_v1(data.status);
        if unknown != 0 {
            errors.push(format!(
                "offset {offset}: hour {hour} domain 51 status contains unknown v1 bits {unknown:#010x}"
            ));
        }
        let fatal = (data.status & !compact_status_flags::DOMAIN51_LANE_LOCAL_STATUS_MASK)
            & STATUS_FAILURE_MASK;
        if fatal != 0 {
            errors.push(format!(
                "offset {offset}: hour {hour} domain 51 status contains fatal bits {fatal:#010x}"
            ));
        }
    }
}

fn domain_perf_payload(
    row_count: u32,
    changed_rows: u32,
    unit_count: u32,
    changed_unit_count: u32,
    status_flags: u32,
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(40);
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&1_250u64.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&changed_rows.to_le_bytes());
    payload.extend_from_slice(&unit_count.to_le_bytes());
    payload.extend_from_slice(&changed_unit_count.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn frame_end_payload(status_flags: u32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(32);
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&2_000u64.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn update_status_bools(report: &mut Domain51StreamReport, status_flags: u32) {
    report.partial_failure_seen |= status_flags & collector::compact::STATUS_PARTIAL_DATA != 0;
    report.source_unavailable_seen |=
        status_flags & collector::compact::STATUS_SOURCE_UNAVAILABLE != 0;
    report.unresolved_delayed_events_badged |=
        status_flags & collector::compact::STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED != 0;
    report.unresolved_scripted_gui_badged |=
        status_flags & collector::compact::STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED != 0;
}

fn skipped_hours(hours: &[i32]) -> u32 {
    let mut skipped = 0u32;
    for pair in hours.windows(2) {
        let gap = pair[1].saturating_sub(pair[0]).saturating_sub(1);
        if gap > 0 {
            skipped = skipped.saturating_add(gap as u32);
        }
    }
    skipped
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn quote_json(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn u16_vec_json(values: &[u16]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn i32_vec_json(values: &[i32]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn i32_u32_map_json(values: &BTreeMap<i32, u32>) -> String {
    let body = values
        .iter()
        .map(|(hour, count)| format!("\"{hour}\":{count}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}

fn write_sample_json(path: &str) -> Result<String, String> {
    let stream = build_sample_domain51_h4cr_stream();
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("create output directory failed: {err}"))?;
    }
    fs::write(path, &stream).map_err(|err| format!("write stream failed: {err}"))?;
    Ok(format!(
        "{{\"ok\":true,\"stream_path\":{},\"bytes\":{}}}",
        quote_json(path),
        stream.len()
    ))
}

fn verify_json(path: &str, minimum_game_hour_advancements: i64) -> Result<String, String> {
    let raw = fs::read(path).map_err(|err| format!("read stream failed: {err}"))?;
    let report = verify_domain51_h4cr_stream(&raw, minimum_game_hour_advancements);
    let json = report.to_json();
    if report.ok {
        Ok(json)
    } else {
        Err(json)
    }
}

fn parse_minimum_game_hour_advancements(raw: Option<&String>) -> Result<i64, String> {
    let raw = raw.ok_or_else(|| {
        "--min-game-hours is required explicitly; mutable manifest fallback is forbidden"
            .to_string()
    })?;
    let value = raw
        .parse::<i64>()
        .map_err(|_| format!("--min-game-hours needs an integer, got {raw:?}"))?;
    if value <= 0 {
        return Err(format!(
            "--min-game-hours must be explicitly positive, got {value}"
        ));
    }
    Ok(value)
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    let result = match args.get(1).map(String::as_str) {
        Some("--write-sample") => {
            let path = match args.get(2) {
                Some(path) => path,
                None => {
                    eprintln!("usage: scripted_state_domain51_stream --write-sample <stream.bin>");
                    std::process::exit(2);
                }
            };
            write_sample_json(path)
        }
        Some("--verify-json") => {
            let path = match args.get(2) {
                Some(path) => path,
                None => {
                    eprintln!("usage: scripted_state_domain51_stream --verify-json <stream.bin> --min-game-hours <positive advancements>");
                    std::process::exit(2);
                }
            };
            if args.get(3).map(String::as_str) != Some("--min-game-hours") || args.len() != 5 {
                Err("usage: scripted_state_domain51_stream --verify-json <stream.bin> --min-game-hours <positive advancements>".to_string())
            } else {
                parse_minimum_game_hour_advancements(args.get(4))
                    .and_then(|minimum| verify_json(path, minimum))
            }
        }
        _ => {
            eprintln!(
                "usage:\n  scripted_state_domain51_stream --write-sample <stream.bin>\n  scripted_state_domain51_stream --verify-json <stream.bin> --min-game-hours <positive advancements>"
            );
            std::process::exit(2);
        }
    };

    match result {
        Ok(json) => println!("{json}"),
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offsets(bytes: &[u8]) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cursor = 0usize;
        while cursor < bytes.len() {
            out.push(cursor);
            let header = parse_compact_record_header(&bytes[cursor..]).unwrap();
            cursor += header.record_len();
        }
        out
    }

    fn append_domain51_hour(
        out: &mut Vec<u8>,
        sequence: &mut u64,
        hour: i32,
        frame: &collector::Domain51CollectedFrame,
    ) {
        let perf = domain_perf_payload(
            frame.observed_scopes,
            frame.row_count,
            frame.row_count,
            frame.row_count,
            frame.status_flags,
        );
        append_h4cr_record(
            out,
            *sequence,
            hour,
            collector::compact::DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf,
        );
        *sequence += 1;
        append_h4cr_record(
            out,
            *sequence,
            hour,
            collector::compact::DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            &frame.payload,
        );
        *sequence += 1;
        append_h4cr_record(
            out,
            *sequence,
            hour,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(status_for_global_interpretation(
                collector::compact::DOMAIN_ID,
                frame.status_flags,
            )),
        );
        *sequence += 1;
    }

    fn repeated_frame_stream(frame: &collector::Domain51CollectedFrame) -> Vec<u8> {
        let mut stream = Vec::new();
        let mut sequence = 1;
        append_domain51_hour(&mut stream, &mut sequence, 100, frame);
        append_domain51_hour(&mut stream, &mut sequence, 101, frame);
        stream
    }

    fn delta_frame() -> collector::Domain51CollectedFrame {
        let mut frame = sample_domain51_frames()[0].1.clone();
        frame.rows.truncate(1);
        frame.row_count = 1;
        frame.status_flags = collector::compact::STATUS_DELTA_DATA;
        frame.payload = collector::compact::encode_payload(
            &collector::compact::ScriptedStateHeader {
                elapsed_wall_ms: frame.elapsed_wall_ms,
                status_flags: frame.status_flags,
                changed_rows: 1,
                observed_scopes: frame.observed_scopes,
                measured_scopes: frame.measured_scopes,
                rejected_scopes: frame.rejected_scopes,
            },
            &frame.rows,
        );
        frame
    }

    #[test]
    fn sample_stream_is_h4cr_domain51_with_frame_end_records() {
        let stream = build_sample_domain51_h4cr_stream();
        assert_eq!(&stream[0..4], H4CR_MAGIC);

        let report = verify_domain51_h4cr_stream(&stream, 1);

        assert!(report.ok, "{:?}", report.errors);
        assert_eq!(report.domain51_perf_records, 2);
        assert_eq!(report.domain51_data_records, 2);
        assert_eq!(report.frame_end_records, 2);
        assert_eq!(
            report.domains_seen,
            vec![collector::compact::DOMAIN_ID, DOMAIN_FRAME]
        );
        assert_eq!(report.unique_hours, vec![43_800_024, 43_800_025]);
        assert_eq!(report.skipped_hours, 0);
        assert_eq!(report.required_closed_hours, 2);
        assert_eq!(report.certified_game_hour_advancements, 1);
        assert!(report.terminal_frame_end_authoritative);
    }

    #[test]
    fn unresolved_opaque_roots_are_badges_not_partial_or_source_failures() {
        let stream = build_sample_domain51_h4cr_stream();
        let report = verify_domain51_h4cr_stream(&stream, 1);

        assert!(report.ok, "{:?}", report.errors);
        assert!(report.unresolved_delayed_events_badged);
        assert!(report.unresolved_scripted_gui_badged);
        assert!(!report.partial_failure_seen);
        assert!(!report.source_unavailable_seen);
    }

    #[test]
    fn certification_duration_is_explicit_positive_and_inclusive() {
        let stream = build_sample_domain51_h4cr_stream();
        let absent = verify_domain51_h4cr_stream(&stream, 0);
        assert!(absent
            .errors
            .iter()
            .any(|error| error
                .contains("minimum_game_hour_advancements must be explicitly positive")));

        let too_short = verify_domain51_h4cr_stream(&stream, 2);
        assert!(too_short
            .errors
            .iter()
            .any(|error| error.contains("closed hour count 2 is below required 3")));
        assert!(too_short
            .errors
            .iter()
            .any(|error| error.contains("advancements 1 is below required 2")));

        assert!(parse_minimum_game_hour_advancements(None).is_err());
        assert!(parse_minimum_game_hour_advancements(Some(&"0".to_string())).is_err());
        assert_eq!(
            parse_minimum_game_hour_advancements(Some(&"48".to_string())).unwrap(),
            48
        );
    }

    #[test]
    fn stream_gate_rejects_skipped_hour() {
        let frames = sample_domain51_frames();
        let stream = build_domain51_h4cr_stream(&[
            frames[0].clone(),
            (frames[1].0 + 2, frames[1].1.clone()),
        ]);
        let report = verify_domain51_h4cr_stream(&stream, 1);

        assert_eq!(report.skipped_hours, 2);
        assert!(!report.ok);
    }

    #[test]
    fn stream_gate_rejects_bad_magic() {
        let mut stream = build_sample_domain51_h4cr_stream();
        stream[0] = b'X';
        let report = verify_domain51_h4cr_stream(&stream, 1);

        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| error.contains("magic")));
    }

    #[test]
    fn stream_gate_rejects_sequence_flags_lengths_and_unclean_tail() {
        let frames = sample_domain51_frames();
        let frame = &frames[0].1;
        let mut stream = Vec::new();
        append_h4cr_record(
            &mut stream,
            2,
            100,
            collector::compact::DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            1 << 31,
            &frame.payload,
        );
        append_h4cr_record(
            &mut stream,
            3,
            100,
            collector::compact::DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &[0; 39],
        );
        append_h4cr_record(
            &mut stream,
            4,
            101,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &[0; 31],
        );
        append_h4cr_record(
            &mut stream,
            5,
            101,
            collector::compact::DOMAIN_ID,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &[0; DOMAIN_PERF_PAYLOAD_LEN],
        );

        let report = verify_domain51_h4cr_stream(&stream, 1);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("expected 1")));
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("record flags")));
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("shorter than 40")));
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("is not 32")));
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("has no open data hour")));
        assert!(report.errors.iter().any(|error| error
            .contains("healthy authoritative terminal FRAME_END with no trailing records")));
    }

    #[test]
    fn stream_gate_validates_announced_perf_trailer_envelopes() {
        fn stream_with_perf(flags: u32, perf: &[u8]) -> Domain51StreamReport {
            let frames = sample_domain51_frames();
            let frame = &frames[0].1;
            let mut stream = Vec::new();
            let mut sequence = 1;
            for hour in [100, 101] {
                append_h4cr_record(
                    &mut stream,
                    sequence,
                    hour,
                    collector::compact::DOMAIN_ID,
                    RECORD_TYPE_DOMAIN_PERF,
                    flags,
                    perf,
                );
                sequence += 1;
                append_h4cr_record(
                    &mut stream,
                    sequence,
                    hour,
                    collector::compact::DOMAIN_ID,
                    RECORD_TYPE_DOMAIN_DATA,
                    0,
                    &frame.payload,
                );
                sequence += 1;
                append_h4cr_record(
                    &mut stream,
                    sequence,
                    hour,
                    DOMAIN_FRAME,
                    RECORD_TYPE_FRAME_END,
                    0,
                    &frame_end_payload(0),
                );
                sequence += 1;
            }
            verify_domain51_h4cr_stream(&stream, 1)
        }

        let frame = &sample_domain51_frames()[0].1;
        let base = domain_perf_payload(
            frame.observed_scopes,
            frame.row_count,
            frame.row_count,
            frame.row_count,
            frame.status_flags,
        );
        let trailer = pdu1_perf_payload(base.clone());
        assert!(
            stream_with_perf(RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &trailer)
                .errors
                .is_empty()
        );
        assert!(stream_with_perf(RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &base)
            .errors
            .iter()
            .any(|error| error.contains("trailer header is truncated")));
        assert!(stream_with_perf(0, &trailer)
            .errors
            .iter()
            .any(|error| error.contains("unannounced trailer")));
    }

    fn pdu1_perf_payload(mut payload: Vec<u8>) -> Vec<u8> {
        payload.extend_from_slice(b"PDU1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }

    #[test]
    fn full_stream_allows_unrelated_domains_but_requires_one_d51_pair_per_hour() {
        let frame = &sample_domain51_frames()[0].1;
        let mut stream = Vec::new();
        let mut sequence = 1u64;
        for hour in [100, 101] {
            append_h4cr_record(
                &mut stream,
                sequence,
                hour,
                77,
                RECORD_TYPE_DOMAIN_PERF,
                0,
                &domain_perf_payload(1, 1, 1, 1, 0),
            );
            sequence += 1;
            append_h4cr_record(
                &mut stream,
                sequence,
                hour,
                77,
                RECORD_TYPE_DOMAIN_DATA,
                0,
                b"unrelated",
            );
            sequence += 1;
            let perf = domain_perf_payload(
                frame.observed_scopes,
                frame.row_count,
                frame.row_count,
                frame.row_count,
                frame.status_flags,
            );
            append_h4cr_record(
                &mut stream,
                sequence,
                hour,
                collector::compact::DOMAIN_ID,
                RECORD_TYPE_DOMAIN_PERF,
                0,
                &perf,
            );
            sequence += 1;
            append_h4cr_record(
                &mut stream,
                sequence,
                hour,
                collector::compact::DOMAIN_ID,
                RECORD_TYPE_DOMAIN_DATA,
                0,
                &frame.payload,
            );
            sequence += 1;
            append_h4cr_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_DIAGNOSTIC_PROBE,
                RECORD_TYPE_DOMAIN_PERF,
                RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
                &pdu1_perf_payload(domain_perf_payload(0, 0, 0, 0, 0)),
            );
            sequence += 1;
            let mut frame_end = frame_end_payload(0);
            frame_end[16..20].copy_from_slice(&2u32.to_le_bytes());
            frame_end[20..24].copy_from_slice(&2u32.to_le_bytes());
            append_h4cr_record(
                &mut stream,
                sequence,
                hour,
                DOMAIN_FRAME,
                RECORD_TYPE_FRAME_END,
                0,
                &frame_end,
            );
            sequence += 1;
        }

        let report = verify_domain51_h4cr_stream(&stream, 1);
        assert!(report.ok, "{:?}", report.errors);
        assert_eq!(
            report.domains_seen,
            vec![51, 77, DOMAIN_DIAGNOSTIC_PROBE, DOMAIN_FRAME]
        );

        let mut fatal_partial = stream.clone();
        let partial_offsets = offsets(&fatal_partial);
        fatal_partial[partial_offsets[0] + 20..partial_offsets[0] + 22]
            .copy_from_slice(&26u16.to_le_bytes());
        fatal_partial[partial_offsets[1] + 20..partial_offsets[1] + 22]
            .copy_from_slice(&26u16.to_le_bytes());
        let first_partial_perf = partial_offsets[0] + H4CR_HEADER_LEN;
        fatal_partial[first_partial_perf + 32..first_partial_perf + 36]
            .copy_from_slice(&collector::compact::STATUS_PARTIAL_DATA.to_le_bytes());
        let partial_report = verify_domain51_h4cr_stream(&fatal_partial, 1);
        assert!(!partial_report.ok);
        assert!(partial_report.errors.iter().any(|error| {
            error.contains("domain 26 PERF status contains fatal bits")
                && error.contains("0x00000200")
        }));

        let mut forbidden_probe_data = stream.clone();
        let first_other_data = offsets(&forbidden_probe_data)[1];
        forbidden_probe_data[first_other_data + 20..first_other_data + 22]
            .copy_from_slice(&DOMAIN_DIAGNOSTIC_PROBE.to_le_bytes());
        assert!(verify_domain51_h4cr_stream(&forbidden_probe_data, 1)
            .errors
            .iter()
            .any(|error| error.contains("domain 250 is PERF-only; DATA is forbidden")));

        let mut forged_count = stream.clone();
        let first_frame = offsets(&forged_count)[5] + H4CR_HEADER_LEN;
        forged_count[first_frame + 16..first_frame + 20].copy_from_slice(&1u32.to_le_bytes());
        forged_count[first_frame + 20..first_frame + 24].copy_from_slice(&1u32.to_le_bytes());
        assert!(verify_domain51_h4cr_stream(&forged_count, 1)
            .errors
            .iter()
            .any(|error| error.contains("paired-domain count 2")));

        let mut unrelated_fatal = stream;
        let first_other_perf = offsets(&unrelated_fatal)[0] + H4CR_HEADER_LEN;
        unrelated_fatal[first_other_perf + 32..first_other_perf + 36]
            .copy_from_slice(&collector::compact::STATUS_SOURCE_UNAVAILABLE.to_le_bytes());
        assert!(verify_domain51_h4cr_stream(&unrelated_fatal, 1)
            .errors
            .iter()
            .any(|error| error.contains("domain 77 PERF status contains fatal bits")));
    }

    #[test]
    fn diagnostic_probe_requires_canonical_pdu1_perf_and_never_counts_as_scheduled() {
        let frame = &sample_domain51_frames()[0].1;
        let mut stream = repeated_frame_stream(frame);
        let insert_at = offsets(&stream)[2];
        let mut bare_probe = Vec::new();
        append_h4cr_record(
            &mut bare_probe,
            3,
            100,
            DOMAIN_DIAGNOSTIC_PROBE,
            RECORD_TYPE_DOMAIN_PERF,
            0,
            &domain_perf_payload(0, 0, 0, 0, 0),
        );
        stream.splice(insert_at..insert_at, bare_probe);
        for (index, offset) in offsets(&stream).into_iter().enumerate() {
            stream[offset + 8..offset + 16].copy_from_slice(&(1 + index as u64).to_le_bytes());
        }
        assert!(verify_domain51_h4cr_stream(&stream, 1)
            .errors
            .iter()
            .any(|error| error.contains("domain 250 PERF lacks the canonical PDU1")));
    }

    #[test]
    fn rejects_duplicate_or_missing_domain51_authority() {
        let frame = &sample_domain51_frames()[0].1;
        let mut duplicate = repeated_frame_stream(frame);
        let record_offsets = offsets(&duplicate);
        let first_perf_end = record_offsets[1];
        let first_perf = duplicate[..first_perf_end].to_vec();
        duplicate.splice(first_perf_end..first_perf_end, first_perf);
        for (index, offset) in offsets(&duplicate).into_iter().enumerate() {
            duplicate[offset + 8..offset + 16].copy_from_slice(&(1 + index as u64).to_le_bytes());
        }
        assert!(verify_domain51_h4cr_stream(&duplicate, 1)
            .errors
            .iter()
            .any(|error| error.contains("perf_record_count 2 != 1")));

        let mut missing = repeated_frame_stream(frame);
        let first_data = offsets(&missing)[1];
        missing[first_data + 20..first_data + 22].copy_from_slice(&77u16.to_le_bytes());
        assert!(verify_domain51_h4cr_stream(&missing, 1)
            .errors
            .iter()
            .any(|error| error.contains("data_record_count 0 != 1")));
    }

    #[test]
    fn rejects_unhealthy_or_non_authoritative_frame_end() {
        let frame = &sample_domain51_frames()[0].1;
        let valid = repeated_frame_stream(frame);
        let frame_offset = offsets(&valid)[2] + H4CR_HEADER_LEN;

        let mut counts = valid.clone();
        counts[frame_offset + 20..frame_offset + 24].copy_from_slice(&2u32.to_le_bytes());
        assert!(verify_domain51_h4cr_stream(&counts, 1)
            .errors
            .iter()
            .any(|error| error.contains("not equal and nonzero")));

        let mut reserved = valid.clone();
        reserved[frame_offset + 28..frame_offset + 32].copy_from_slice(&1u32.to_le_bytes());
        assert!(verify_domain51_h4cr_stream(&reserved, 1)
            .errors
            .iter()
            .any(|error| error.contains("reserved word")));

        let mut fatal = valid;
        fatal[frame_offset + 24..frame_offset + 28]
            .copy_from_slice(&collector::compact::STATUS_SOURCE_UNAVAILABLE.to_le_bytes());
        assert!(verify_domain51_h4cr_stream(&fatal, 1)
            .errors
            .iter()
            .any(|error| error.contains("fatal bits")));

        let mut aggregate_mismatch = repeated_frame_stream(frame);
        let first_perf_payload = offsets(&aggregate_mismatch)[0] + H4CR_HEADER_LEN;
        aggregate_mismatch[first_perf_payload + 32..first_perf_payload + 36]
            .copy_from_slice(&collector::compact::STATUS_DELTA_DATA.to_le_bytes());
        assert!(verify_domain51_h4cr_stream(&aggregate_mismatch, 1)
            .errors
            .iter()
            .any(|error| error.contains("does not equal closed-frame PERF status aggregate")));

        let mut frame_flags = repeated_frame_stream(frame);
        let first_frame_record = offsets(&frame_flags)[2];
        frame_flags[first_frame_record + 28..first_frame_record + 32]
            .copy_from_slice(&1u32.to_le_bytes());
        assert!(verify_domain51_h4cr_stream(&frame_flags, 1)
            .errors
            .iter()
            .any(|error| error.contains("record flags")));
    }

    #[test]
    fn rejects_trailing_record_or_unframed_bytes_after_terminal_frame_end() {
        let frame = &sample_domain51_frames()[0].1;
        let mut trailing_record = repeated_frame_stream(frame);
        append_h4cr_record(
            &mut trailing_record,
            7,
            101,
            77,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            b"tail",
        );
        assert!(verify_domain51_h4cr_stream(&trailing_record, 1)
            .errors
            .iter()
            .any(|error| error.contains("no trailing records")));

        let mut trailing_bytes = repeated_frame_stream(frame);
        trailing_bytes.extend_from_slice(b"tail");
        let errors = verify_domain51_h4cr_stream(&trailing_bytes, 1).errors;
        assert!(errors
            .iter()
            .any(|error| error.contains("truncated H4CR header")));
        assert!(errors
            .iter()
            .any(|error| error.contains("no trailing records")));
    }

    #[test]
    fn delta_rows_retain_data_perf_authority_without_full_frame_assumptions() {
        let frame = delta_frame();
        let stream = repeated_frame_stream(&frame);
        let report = verify_domain51_h4cr_stream(&stream, 1);
        assert!(report.ok, "{:?}", report.errors);
        assert_eq!(report.domain51_rows_total, 2);
        assert_eq!(report.domain51_changed_rows_total, 2);
        assert_ne!(
            report.status_flags_or & collector::compact::STATUS_DELTA_DATA,
            0
        );

        let mut mismatched = stream;
        let first_perf = offsets(&mismatched)[0] + H4CR_HEADER_LEN;
        mismatched[first_perf + 20..first_perf + 24].copy_from_slice(&0u32.to_le_bytes());
        assert!(verify_domain51_h4cr_stream(&mismatched, 1)
            .errors
            .iter()
            .any(|error| error.contains("PERF/DATA cardinality or status mismatch")));
    }

    #[test]
    fn every_nonempty_short_header_preserves_the_lane_diagnostic() {
        for len in 1..H4CR_HEADER_LEN {
            let report = verify_domain51_h4cr_stream(&vec![0; len], 1);

            assert!(!report.ok, "length {len} unexpectedly passed");
            assert_eq!(
                report.errors.first(),
                Some(&format!(
                    "offset 0: truncated H4CR header with {len} byte(s) remaining"
                )),
                "length {len}: {:?}",
                report.errors
            );
        }

        let empty = verify_domain51_h4cr_stream(&[], 1);
        assert!(!empty.ok);
        assert!(empty
            .errors
            .iter()
            .any(|error| error.contains("closed hour count 0 is below required 2")));
    }
}
