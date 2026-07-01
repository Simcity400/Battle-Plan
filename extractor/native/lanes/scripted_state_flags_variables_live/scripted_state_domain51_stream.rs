#[path = "scripted_state_domain51_collector.rs"]
pub mod collector;
#[path = "scripted_state_domain51_gate.rs"]
pub mod gate;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::Path;

pub const H4CR_MAGIC: &[u8; 4] = b"H4CR";
pub const H4CR_VERSION: u16 = 1;
pub const H4CR_HEADER_LEN: usize = 32;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;
pub const DOMAIN_FRAME: u16 = 1000;

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
    pub per_hour_domain51_perf: BTreeMap<i32, u32>,
    pub per_hour_domain51_data: BTreeMap<i32, u32>,
    pub per_hour_frame_end: BTreeMap<i32, u32>,
}

impl Domain51StreamReport {
    fn new(total_bytes: usize) -> Self {
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
            frame.row_count,
            frame.row_count,
            frame.observed_scopes,
            frame.measured_scopes,
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
        sequence = sequence.saturating_add(1);

        append_h4cr_record(
            &mut out,
            sequence,
            *game_hour_raw,
            frame.domain_id,
            RECORD_TYPE_DOMAIN_DATA,
            0,
            &frame.payload,
        );
        sequence = sequence.saturating_add(1);

        let frame_end = frame_end_payload(frame.status_flags);
        append_h4cr_record(
            &mut out,
            sequence,
            *game_hour_raw,
            DOMAIN_FRAME,
            RECORD_TYPE_FRAME_END,
            0,
            &frame_end,
        );
        sequence = sequence.saturating_add(1);
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
                delayed_events: collector::OpaqueRootState::UnresolvedFixtureBlocked,
                scripted_gui_open_state: collector::OpaqueRootState::UnresolvedFixtureBlocked,
            }),
        ),
    ]
}

pub fn build_sample_domain51_h4cr_stream() -> Vec<u8> {
    build_domain51_h4cr_stream(&sample_domain51_frames())
}

pub fn verify_domain51_h4cr_stream(raw: &[u8]) -> Domain51StreamReport {
    let mut report = Domain51StreamReport::new(raw.len());
    let mut offset = 0usize;
    let mut previous_sequence = None;
    let mut domains_seen = BTreeSet::new();
    let mut unique_hours = BTreeSet::new();

    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        if remaining < H4CR_HEADER_LEN {
            report.errors.push(format!(
                "offset {offset}: truncated H4CR header with {remaining} byte(s) remaining"
            ));
            break;
        }
        let header = &raw[offset..offset + H4CR_HEADER_LEN];
        if &header[0..4] != H4CR_MAGIC {
            report
                .errors
                .push(format!("offset {offset}: invalid H4CR magic"));
            break;
        }
        let version = read_u16_le(header, 4);
        let header_len = read_u16_le(header, 6) as usize;
        if version != H4CR_VERSION {
            report.errors.push(format!(
                "offset {offset}: unsupported H4CR version {version}"
            ));
            break;
        }
        if header_len != H4CR_HEADER_LEN {
            report.errors.push(format!(
                "offset {offset}: unsupported H4CR header length {header_len}"
            ));
            break;
        }
        let record = H4crRecord {
            offset,
            sequence: read_u64_le(header, 8),
            game_hour_raw: read_i32_le(header, 16),
            domain_id: read_u16_le(header, 20),
            record_type: read_u16_le(header, 22),
            payload_len: read_u32_le(header, 24) as usize,
            flags: read_u32_le(header, 28),
        };
        if let Some(previous) = previous_sequence {
            if record.sequence <= previous {
                report.errors.push(format!(
                    "offset {offset}: sequence {} did not advance past {previous}",
                    record.sequence
                ));
            }
        }
        previous_sequence = Some(record.sequence);
        let record_len = H4CR_HEADER_LEN.saturating_add(record.payload_len);
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

        report.record_count = report.record_count.saturating_add(1);
        domains_seen.insert(record.domain_id);
        unique_hours.insert(record.game_hour_raw);

        match record.record_type {
            RECORD_TYPE_DOMAIN_PERF => {
                report.domain_perf_records = report.domain_perf_records.saturating_add(1);
                if record.domain_id == collector::compact::DOMAIN_ID {
                    report.domain51_perf_records = report.domain51_perf_records.saturating_add(1);
                    *report
                        .per_hour_domain51_perf
                        .entry(record.game_hour_raw)
                        .or_default() += 1;
                }
                if payload.len() < 40 {
                    report.errors.push(format!(
                        "offset {offset}: domain perf payload is {} byte(s), expected at least 40",
                        payload.len()
                    ));
                } else {
                    let status_flags = read_u32_le(payload, 32);
                    report.status_flags_or |= status_flags;
                    update_status_bools(&mut report, status_flags);
                }
            }
            RECORD_TYPE_DOMAIN_DATA => {
                report.domain_data_records = report.domain_data_records.saturating_add(1);
                if record.domain_id != collector::compact::DOMAIN_ID {
                    report.errors.push(format!(
                        "offset {offset}: unsupported domain data payload for domain {}",
                        record.domain_id
                    ));
                } else {
                    report.domain51_data_records = report.domain51_data_records.saturating_add(1);
                    report.domain51_payload_bytes = report
                        .domain51_payload_bytes
                        .saturating_add(record.payload_len as u64);
                    *report
                        .per_hour_domain51_data
                        .entry(record.game_hour_raw)
                        .or_default() += 1;
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
                    if !verdict.accepted {
                        report.errors.extend(verdict.summary.parse_errors);
                    }
                }
            }
            RECORD_TYPE_FRAME_END => {
                report.frame_end_records = report.frame_end_records.saturating_add(1);
                *report
                    .per_hour_frame_end
                    .entry(record.game_hour_raw)
                    .or_default() += 1;
                if record.domain_id != DOMAIN_FRAME {
                    report.errors.push(format!(
                        "offset {offset}: frame-end domain {} != {DOMAIN_FRAME}",
                        record.domain_id
                    ));
                }
                if payload.len() < 32 {
                    report.errors.push(format!(
                        "offset {offset}: frame-end payload is {} byte(s), expected at least 32",
                        payload.len()
                    ));
                } else {
                    let status_flags = read_u32_le(payload, 24);
                    report.status_flags_or |= status_flags;
                    update_status_bools(&mut report, status_flags);
                }
            }
            other => report
                .errors
                .push(format!("offset {offset}: unknown H4CR record type {other}")),
        }
        offset = offset.saturating_add(record_len);
    }

    report.domains_seen = domains_seen.into_iter().collect();
    report.unique_hours = unique_hours.into_iter().collect();
    report.skipped_hours = skipped_hours(&report.unique_hours);
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
    report.ok = report.errors.is_empty()
        && report.domain51_data_records > 0
        && !report.partial_failure_seen
        && !report.source_unavailable_seen;
    report
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

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
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

fn verify_json(path: &str) -> Result<String, String> {
    let raw = fs::read(path).map_err(|err| format!("read stream failed: {err}"))?;
    Ok(verify_domain51_h4cr_stream(&raw).to_json())
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
                    eprintln!("usage: scripted_state_domain51_stream --verify-json <stream.bin>");
                    std::process::exit(2);
                }
            };
            verify_json(path)
        }
        _ => {
            eprintln!(
                "usage:\n  scripted_state_domain51_stream --write-sample <stream.bin>\n  scripted_state_domain51_stream --verify-json <stream.bin>"
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

    #[test]
    fn sample_stream_is_h4cr_domain51_with_frame_end_records() {
        let stream = build_sample_domain51_h4cr_stream();
        assert_eq!(&stream[0..4], H4CR_MAGIC);

        let report = verify_domain51_h4cr_stream(&stream);

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
    }

    #[test]
    fn unresolved_opaque_roots_are_badges_not_partial_or_source_failures() {
        let stream = build_sample_domain51_h4cr_stream();
        let report = verify_domain51_h4cr_stream(&stream);

        assert!(report.ok, "{:?}", report.errors);
        assert!(report.unresolved_delayed_events_badged);
        assert!(report.unresolved_scripted_gui_badged);
        assert!(!report.partial_failure_seen);
        assert!(!report.source_unavailable_seen);
    }

    #[test]
    fn stream_gate_rejects_skipped_hour() {
        let frames = sample_domain51_frames();
        let stream = build_domain51_h4cr_stream(&[
            frames[0].clone(),
            (frames[1].0 + 2, frames[1].1.clone()),
        ]);
        let report = verify_domain51_h4cr_stream(&stream);

        assert_eq!(report.skipped_hours, 2);
        assert!(!report.ok);
    }

    #[test]
    fn stream_gate_rejects_bad_magic() {
        let mut stream = build_sample_domain51_h4cr_stream();
        stream[0] = b'X';
        let report = verify_domain51_h4cr_stream(&stream);

        assert!(!report.ok);
        assert!(report.errors.iter().any(|error| error.contains("magic")));
    }
}
