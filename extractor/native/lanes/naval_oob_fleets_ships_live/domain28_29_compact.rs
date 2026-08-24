use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::process;

#[allow(dead_code)]
#[path = "../../compact_contract/src/build_profile.rs"]
mod compact_build_profile;
#[path = "../../compact_contract/src/record_header.rs"]
mod compact_record_header;
#[path = "../../compact_contract/src/record_semantics.rs"]
mod compact_record_semantics;
#[allow(dead_code)]
#[path = "../../compact_contract/src/status_flags_base.rs"]
mod compact_status_flags;
use compact_record_header::{parse_compact_record_header, RecordFramingError};
pub use compact_record_header::{
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC, COMPACT_RECORD_VERSION,
};
use compact_record_semantics::validate_compact_record_semantics;
#[cfg(test)]
use compact_record_semantics::{
    COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
};

pub const DOMAIN_OOB_COMMAND: u16 = 28;
pub const DOMAIN_SHIPS: u16 = 29;
pub const RC_PROBE_DOMAIN_ID: u16 = 250;

pub const COMPACT_RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const COMPACT_RECORD_TYPE_FRAME_END: u16 = 2;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;
pub const COMPACT_DOMAIN_FRAME: u16 = 1000;

pub use compact_status_flags::{
    compact_status_unknown_flags_v1, COMPACT_STATUS_BRIDGE_UNAVAILABLE,
    COMPACT_STATUS_CONTROL_INVALID, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_NO_CLOCK,
    COMPACT_STATUS_NO_SCALAR_SOURCE, COMPACT_STATUS_PARTIAL_DATA, COMPACT_STATUS_PROFILE_MISMATCH,
    COMPACT_STATUS_SOURCE_UNAVAILABLE, COMPACT_STATUS_STATE_UNAVAILABLE,
    COMPACT_STATUS_WORKER_PANIC,
};
pub const COMPACT_STATUS_FAILURE_MASK: u32 = COMPACT_STATUS_STATE_UNAVAILABLE
    | COMPACT_STATUS_PROFILE_MISMATCH
    | COMPACT_STATUS_NO_CLOCK
    | COMPACT_STATUS_NO_SCALAR_SOURCE
    | COMPACT_STATUS_WORKER_PANIC
    | COMPACT_STATUS_BRIDGE_UNAVAILABLE
    | COMPACT_STATUS_SOURCE_UNAVAILABLE
    | COMPACT_STATUS_PARTIAL_DATA
    | COMPACT_STATUS_CONTROL_INVALID;

const fn lane_local_status_mask(domain_id: u16) -> u32 {
    match domain_id {
        49 => 1 << 11,
        51 => compact_status_flags::DOMAIN51_LANE_LOCAL_STATUS_MASK,
        _ => 0,
    }
}

const fn status_for_global_interpretation(domain_id: u16, status_flags: u32) -> u32 {
    status_flags & !lane_local_status_mask(domain_id)
}

pub const COMPACT_FIXED_SCALE_RAW: u32 = 100_000;

pub const COMPACT_OOB_ROW_LEN: usize = 80;
pub const COMPACT_OOB_MEMBER_ROW_LEN: usize = 8;
pub const COMPACT_OOB_KIND_ARMY: u32 = 1;
pub const COMPACT_OOB_KIND_ARMY_GROUP: u32 = 2;
pub const COMPACT_OOB_KIND_FLEET: u32 = 3;
pub const COMPACT_OOB_KIND_TASK_FORCE: u32 = 4;
pub const COMPACT_OOB_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_OOB_ROW_FLAG_PARENT: u32 = 1 << 1;
pub const COMPACT_OOB_ROW_FLAG_OWNER: u32 = 1 << 2;
pub const COMPACT_OOB_ROW_FLAG_LEADER: u32 = 1 << 3;
pub const COMPACT_OOB_ROW_FLAG_LEADER_SKILL: u32 = 1 << 4;
pub const COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE: u32 = 1 << 5;
pub const COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT: u32 = 1 << 6;
pub const COMPACT_OOB_ROW_FLAG_LOCATION: u32 = 1 << 7;
/// Task-force rows only: `CTaskForce+0x18` could not be read, or its id fell
/// outside map space. Both location flags clear on a task-force row is the
/// engine's own absence marker for a task force that is not deployed to a
/// province -- measured state, not a gap.
pub const COMPACT_OOB_ROW_FLAG_LOCATION_UNREADABLE: u32 = 1 << 8;
pub const COMPACT_OOB_MAX_ID: u32 = 4_000_000;
pub const COMPACT_OOB_MAX_LOCATION_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
pub const COMPACT_OOB_MAX_MEMBER_ROWS: usize = 250_000;
pub const COMPACT_OOB_MAX_LEADER_SKILL: u32 = 15;
pub const COMPACT_OOB_STATUS_OVERFLOW: u32 = 3;

pub const COMPACT_SHIP_ROW_BASE_LEN: usize = 40;
pub const COMPACT_SHIP_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_SHIP_ROW_FLAG_TASK_FORCE: u32 = 1 << 1;
pub const COMPACT_SHIP_ROW_FLAG_DEFINITION: u32 = 1 << 2;
pub const COMPACT_SHIP_ROW_FLAG_OWNER: u32 = 1 << 3;
pub const COMPACT_SHIP_MAX_ID: u32 = 4_000_000;
pub const COMPACT_SHIP_MAX_OWNER_TAG_DB_ID: u32 =
    compact_build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const COMPACT_SHIP_MAX_CATEGORY: u32 = 128;
pub const COMPACT_SHIP_MAX_STRENGTH_RAW: u32 = 1_000_000_000;
pub const COMPACT_SHIP_MAX_ORGANISATION_RAW: u32 = 100_000_000;
pub const COMPACT_SHIP_MAX_EXPERIENCE_RAW: u32 = 100_000_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainGateSummary {
    pub record_count: u64,
    pub data_record_count: u64,
    pub perf_record_count: u64,
    pub row_count_total: u64,
    pub changed_rows_total: u64,
    pub unit_count_total: u64,
    pub changed_unit_count_total: u64,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub status_flags_or: u32,
    pub max_read_time_us: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavalOobGateSummary {
    pub record_count: u64,
    pub domain_perf_record_count: u64,
    pub domain_data_record_count: u64,
    pub frame_end_record_count: u64,
    pub frame_end_status_flags_or: u32,
    pub total_payload_bytes: u64,
    pub max_payload_len: usize,
    pub max_record_len: usize,
    pub first_game_hour_raw: Option<i32>,
    pub last_game_hour_raw: Option<i32>,
    pub unique_game_hour_count: usize,
    pub required_game_hour_advancements: i64,
    pub observed_game_hour_advancements: Option<i64>,
    pub required_closed_frame_count: u64,
    pub domains_seen: BTreeSet<u16>,
    pub per_domain: BTreeMap<u16, DomainGateSummary>,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct OwnedFrameAuthority {
    all_perf_records: u64,
    all_data_records: u64,
    all_perf_status_or: u32,
    perf_domains: BTreeSet<u16>,
    data_domains: BTreeSet<u16>,
    oob_perf_records: u64,
    oob_data_records: u64,
    ships_perf_records: u64,
    ships_data_records: u64,
    oob_perf_status: Option<u32>,
    oob_data_status: Option<u32>,
    ships_perf_status: Option<u32>,
    ships_data_status: Option<u32>,
}

impl NavalOobGateSummary {
    pub fn gate_passed(&self) -> bool {
        self.parse_errors.is_empty()
            && self.ordering_errors.is_empty()
            && self.frame_end_status_flags_or & COMPACT_STATUS_FAILURE_MASK == 0
            && required_domain_ok(self.per_domain.get(&DOMAIN_OOB_COMMAND))
            && required_domain_ok(self.per_domain.get(&DOMAIN_SHIPS))
            && self
                .per_domain
                .values()
                .all(|domain| domain.status_flags_or & COMPACT_STATUS_FAILURE_MASK == 0)
    }
}

pub fn decode_stream(raw: &[u8], minimum_game_hour_advancements: i64) -> NavalOobGateSummary {
    let required_closed_frame_count = minimum_game_hour_advancements
        .checked_add(1)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(u64::MAX);
    let mut summary = NavalOobGateSummary {
        required_game_hour_advancements: minimum_game_hour_advancements,
        required_closed_frame_count,
        ..NavalOobGateSummary::default()
    };
    if minimum_game_hour_advancements <= 0 {
        summary.parse_errors.push(format!(
            "minimum game-hour advancements must be explicitly positive, got {minimum_game_hour_advancements}"
        ));
    }
    if let Err(error) = compact_contract::stream_validator::validate_freeze_census_stream_placement(raw) {
        summary.parse_errors.push(format!("stateful stream placement: {error}"));
    }
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut open_hour: Option<i32> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut saw_frame_end = false;
    let mut last_record_was_clean_frame_end = false;
    let mut release_epilogue_seen = false;
    let mut hours = BTreeSet::new();
    let mut owned_frame = OwnedFrameAuthority::default();
    let mut repeat_baselines = compact_contract::DomainRepeatBaselines::new();

    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(actual)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: trailing {actual} byte(s), expected compact record header"
                ));
                break;
            }
            Err(RecordFramingError::BadMagic) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: invalid record magic"));
                break;
            }
            Err(RecordFramingError::UnsupportedVersion(version)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: unsupported record version {version}"
                ));
                break;
            }
            Err(RecordFramingError::UnsupportedHeaderLen(header_len)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: unsupported header length {header_len}"
                ));
                break;
            }
            Err(err @ RecordFramingError::PayloadTooLarge(payload_len)) => {
                let payload_len = payload_len as usize;
                if COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len) > remaining {
                    summary.parse_errors.push(format!(
                        "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
                    ));
                } else {
                    summary.parse_errors.push(format!("offset {offset}: {err}"));
                }
                break;
            }
        };
        let payload_len = header.payload_len;
        let record_len = header.record_len();
        if record_len > remaining {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        match previous_sequence {
            None if header.sequence != 1 => summary.ordering_errors.push(format!(
                "offset {offset}: first sequence is {}, expected 1",
                header.sequence
            )),
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if header.sequence != expected => {
                    summary.ordering_errors.push(format!(
                        "offset {offset}: sequence discontinuity: expected {expected}, got {}",
                        header.sequence
                    ))
                }
                None => summary.ordering_errors.push(format!(
                    "offset {offset}: sequence cannot advance after {previous} without overflow"
                )),
                _ => {}
            },
            _ => {}
        }
        previous_sequence = Some(header.sequence);
        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + payload_len;
        let payload = &raw[payload_start..payload_end];
        let mut repeat_reconstruction: Option<Vec<u8>> = None;
        let mut semantics_valid = match validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        ) {
            Ok(()) => {
                // Verify/refresh the repeat baseline on every d28/d29 data
                // record; a corrupted or orphaned repeat refuses the stream
                // by name instead of passing as unauthenticated authority.
                match repeat_baselines.observe(header.domain_id, header.record_type, payload) {
                    Ok(reconstructed) => repeat_reconstruction = reconstructed,
                    Err(error) => {
                        summary
                            .parse_errors
                            .push(format!("offset {offset}: {error}"));
                        false
                    }
                }
            }
            Err(error) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: {error}"));
                false
            }
        };

        summary.record_count = summary.record_count.saturating_add(1);
        summary.total_payload_bytes = summary
            .total_payload_bytes
            .saturating_add(payload_len as u64);
        summary.max_payload_len = summary.max_payload_len.max(payload_len);
        summary.max_record_len = summary.max_record_len.max(record_len);
        let previous_record_closed_world = last_record_was_clean_frame_end;
        last_record_was_clean_frame_end = false;
        let known_record_type =
            match compact_contract::KnownRecordType::try_from(header.record_type) {
                Ok(record_type) => record_type,
                Err(unknown) => {
                    summary.parse_errors.push(format!(
                        "offset {offset}: unsupported record type {}",
                        unknown.0
                    ));
                    offset = payload_end;
                    continue;
                }
            };
        if !matches!(
            known_record_type,
            compact_contract::KnownRecordType::FreezeCensus
                | compact_contract::KnownRecordType::StreamHeader
        ) {
            summary.domains_seen.insert(header.domain_id);
        }

        match known_record_type {
            compact_contract::KnownRecordType::DomainPerf if semantics_valid => {
                if header.domain_id != RC_PROBE_DOMAIN_ID {
                    owned_frame.all_perf_records = owned_frame.all_perf_records.saturating_add(1);
                    owned_frame.perf_domains.insert(header.domain_id);
                    let status_flags = read_u32_le(payload, 32);
                    owned_frame.all_perf_status_or |=
                        status_for_global_interpretation(header.domain_id, status_flags);
                    if header.domain_id != DOMAIN_OOB_COMMAND && header.domain_id != DOMAIN_SHIPS {
                        validate_known_status_flags(
                            offset,
                            header.domain_id,
                            "DOMAIN_PERF",
                            status_flags,
                            &mut summary.parse_errors,
                        );
                    }
                }
                match (open_hour, pending_perf_hour) {
                    (Some(open), _) if open == header.game_hour_raw => {}
                    (Some(open), _) => summary.parse_errors.push(format!(
                        "offset {offset}: domain perf hour {} does not match open hour {open}",
                        header.game_hour_raw
                    )),
                    (None, Some(pending)) if pending == header.game_hour_raw => {}
                    (None, Some(pending)) => summary.parse_errors.push(format!(
                        "offset {offset}: domain perf hour {} does not match pending perf hour {pending}",
                        header.game_hour_raw
                    )),
                    (None, None) => pending_perf_hour = Some(header.game_hour_raw),
                }
                if header.domain_id != RC_PROBE_DOMAIN_ID {
                    summary.domain_perf_record_count =
                        summary.domain_perf_record_count.saturating_add(1);
                }
                if header.domain_id == DOMAIN_OOB_COMMAND || header.domain_id == DOMAIN_SHIPS {
                    match header.domain_id {
                        DOMAIN_OOB_COMMAND => {
                            owned_frame.oob_perf_records =
                                owned_frame.oob_perf_records.saturating_add(1);
                            owned_frame.oob_perf_status = Some(read_u32_le(payload, 32));
                        }
                        DOMAIN_SHIPS => {
                            owned_frame.ships_perf_records =
                                owned_frame.ships_perf_records.saturating_add(1);
                            owned_frame.ships_perf_status = Some(read_u32_le(payload, 32));
                        }
                        _ => unreachable!("owned domain guard"),
                    }
                    decode_domain_perf_record(offset, header.domain_id, payload, &mut summary);
                }
            }
            compact_contract::KnownRecordType::FrameEnd if semantics_valid => {
                let open_matches = match open_hour {
                    Some(open) if open == header.game_hour_raw => true,
                    Some(open) => {
                        summary.parse_errors.push(format!(
                            "offset {offset}: frame-end hour {} does not close open hour {open}",
                            header.game_hour_raw
                        ));
                        false
                    }
                    None => {
                        summary.parse_errors.push(format!(
                            "offset {offset}: frame-end hour {} has no open data hour",
                            header.game_hour_raw
                        ));
                        false
                    }
                };
                if let Some(pending) = pending_perf_hour {
                    summary.parse_errors.push(format!(
                        "offset {offset}: frame-end hour {} encountered with pending perf-only hour {pending}",
                        header.game_hour_raw
                    ));
                }
                let successor_matches = closed_hour_is_successor(
                    offset,
                    last_closed_hour,
                    header.game_hour_raw,
                    &mut summary.ordering_errors,
                );
                if open_matches && pending_perf_hour.is_none() {
                    let authority_error_count = summary.parse_errors.len();
                    validate_owned_frame_authority(
                        offset,
                        header.game_hour_raw,
                        &owned_frame,
                        &mut summary.parse_errors,
                    );
                    validate_frame_end_authority(
                        offset,
                        payload,
                        &owned_frame,
                        &mut summary.parse_errors,
                    );
                    let authority_valid = summary.parse_errors.len() == authority_error_count;
                    open_hour = None;
                    if successor_matches {
                        last_closed_hour = Some(header.game_hour_raw);
                        summary.frame_end_record_count =
                            summary.frame_end_record_count.saturating_add(1);
                        summary.frame_end_status_flags_or |= status_for_global_interpretation(
                            COMPACT_DOMAIN_FRAME,
                            read_u32_le(payload, 24),
                        );
                        if authority_valid {
                            saw_frame_end = true;
                            last_record_was_clean_frame_end = true;
                            summary
                                .first_game_hour_raw
                                .get_or_insert(header.game_hour_raw);
                            summary.last_game_hour_raw = Some(header.game_hour_raw);
                            hours.insert(header.game_hour_raw);
                        }
                    }
                    owned_frame = OwnedFrameAuthority::default();
                }
            }
            compact_contract::KnownRecordType::DomainData if semantics_valid => {
                match open_hour {
                    Some(open) if open != header.game_hour_raw => summary.parse_errors.push(format!(
                        "offset {offset}: data hour {} encountered before frame-end closed hour {open}",
                        header.game_hour_raw
                    )),
                    None => match pending_perf_hour {
                        Some(pending) if pending != header.game_hour_raw => summary
                            .parse_errors
                            .push(format!(
                                "offset {offset}: data hour {} does not match pending perf hour {pending}",
                                header.game_hour_raw
                            )),
                        _ => {
                            pending_perf_hour = None;
                            open_hour = Some(header.game_hour_raw);
                        }
                    },
                    _ => {}
                }
                summary.domain_data_record_count =
                    summary.domain_data_record_count.saturating_add(1);
                if header.domain_id == RC_PROBE_DOMAIN_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: diagnostic probe domain 250 is PERF-only; DATA is forbidden"
                    ));
                } else {
                    owned_frame.all_data_records = owned_frame.all_data_records.saturating_add(1);
                    owned_frame.data_domains.insert(header.domain_id);
                }
                match header.domain_id {
                    DOMAIN_OOB_COMMAND => {
                        owned_frame.oob_data_records =
                            owned_frame.oob_data_records.saturating_add(1);
                        if payload.len() >= 20 {
                            owned_frame.oob_data_status = Some(read_u32_le(payload, 16));
                        }
                        summary
                            .per_domain
                            .entry(header.domain_id)
                            .or_default()
                            .data_record_count += 1;
                        decode_oob_command_data_payload(offset, payload, &mut summary);
                    }
                    DOMAIN_SHIPS => {
                        owned_frame.ships_data_records =
                            owned_frame.ships_data_records.saturating_add(1);
                        if payload.len() >= 20 {
                            owned_frame.ships_data_status = Some(read_u32_le(payload, 16));
                        }
                        summary
                            .per_domain
                            .entry(header.domain_id)
                            .or_default()
                            .data_record_count += 1;
                        decode_ships_data_payload(offset, payload, &mut summary);
                    }
                    _ => {}
                }
            }
            // A repeat covers its hour like DATA (same open-hour rules) and
            // counts toward the frame's data authority; the grammar runs on
            // the reconstruction verified above, so row/status totals stay
            // exact.
            compact_contract::KnownRecordType::DomainDataRepeat if semantics_valid => {
                match open_hour {
                    Some(open) if open != header.game_hour_raw => summary.parse_errors.push(format!(
                        "offset {offset}: repeat hour {} encountered before frame-end closed hour {open}",
                        header.game_hour_raw
                    )),
                    None => match pending_perf_hour {
                        Some(pending) if pending != header.game_hour_raw => summary
                            .parse_errors
                            .push(format!(
                                "offset {offset}: repeat hour {} does not match pending perf hour {pending}",
                                header.game_hour_raw
                            )),
                        _ => {
                            pending_perf_hour = None;
                            open_hour = Some(header.game_hour_raw);
                        }
                    },
                    _ => {}
                }
                summary.domain_data_record_count =
                    summary.domain_data_record_count.saturating_add(1);
                if header.domain_id == RC_PROBE_DOMAIN_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: diagnostic probe domain 250 is PERF-only; DATA/REPEAT is forbidden"
                    ));
                } else {
                    owned_frame.all_data_records = owned_frame.all_data_records.saturating_add(1);
                    owned_frame.data_domains.insert(header.domain_id);
                }
                let Some(reconstructed) = repeat_reconstruction.as_ref() else {
                    continue;
                };
                match header.domain_id {
                    DOMAIN_OOB_COMMAND => {
                        owned_frame.oob_data_records =
                            owned_frame.oob_data_records.saturating_add(1);
                        if reconstructed.len() >= 20 {
                            owned_frame.oob_data_status = Some(read_u32_le(reconstructed, 16));
                        }
                        summary
                            .per_domain
                            .entry(header.domain_id)
                            .or_default()
                            .data_record_count += 1;
                        decode_oob_command_data_payload(offset, reconstructed, &mut summary);
                    }
                    DOMAIN_SHIPS => {
                        owned_frame.ships_data_records =
                            owned_frame.ships_data_records.saturating_add(1);
                        if reconstructed.len() >= 20 {
                            owned_frame.ships_data_status = Some(read_u32_le(reconstructed, 16));
                        }
                        summary
                            .per_domain
                            .entry(header.domain_id)
                            .or_default()
                            .data_record_count += 1;
                        decode_ships_data_payload(offset, reconstructed, &mut summary);
                    }
                    _ => {}
                }
            }
            compact_contract::KnownRecordType::DomainPerf
            | compact_contract::KnownRecordType::FrameEnd
            | compact_contract::KnownRecordType::DomainData
            | compact_contract::KnownRecordType::DomainDataRepeat => {}
            compact_contract::KnownRecordType::FreezeCensus => {
                if let Ok(census) = compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload) {
                    last_record_was_clean_frame_end = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(previous_record_closed_world, census.phase(), release_epilogue_seen);
                    release_epilogue_seen |= census.phase() == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                }
            }
            compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StateTopologyCensus
            | compact_contract::KnownRecordType::StreamHeader => {}
            compact_contract::KnownRecordType::DomainGap
            | compact_contract::KnownRecordType::FrameRefused
            | compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => {
                summary.parse_errors.push(format!(
                "offset {offset}: unsupported record type {}",
                    known_record_type.canonical_name()
                ))
            }
        }

        offset = payload_end;
    }

    if !raw.is_empty() {
        if !saw_frame_end {
            summary
                .parse_errors
                .push("stream has no frame-end record".to_string());
        }
        if let Some(open) = open_hour {
            summary
                .parse_errors
                .push(format!("stream ended with open data hour {open}"));
        }
        if !last_record_was_clean_frame_end {
            summary
                .parse_errors
                .push("stream does not end at a clean frame-end boundary".to_string());
        }
    }

    summary.unique_game_hour_count = hours.len();
    summary.observed_game_hour_advancements =
        match (summary.first_game_hour_raw, summary.last_game_hour_raw) {
            (Some(first), Some(last)) => Some(i64::from(last) - i64::from(first)),
            _ => None,
        };
    let observed_advancements = summary.observed_game_hour_advancements.unwrap_or(-1);
    let observed_closed_frames = u64::try_from(summary.unique_game_hour_count).unwrap_or(u64::MAX);
    if observed_advancements < minimum_game_hour_advancements
        || observed_closed_frames < required_closed_frame_count
    {
        summary.parse_errors.push(format!(
            "production proof duration is {observed_advancements} advancement(s) across {observed_closed_frames} closed frame(s), below required {minimum_game_hour_advancements} advancement(s) / {required_closed_frame_count} closed frame(s)"
        ));
    }
    summary
}

fn validate_owned_frame_authority(
    offset: usize,
    game_hour_raw: i32,
    owned: &OwnedFrameAuthority,
    errors: &mut Vec<String>,
) {
    for (domain_id, label, perf_count, data_count, perf_status, data_status) in [
        (
            DOMAIN_OOB_COMMAND,
            "oob_command",
            owned.oob_perf_records,
            owned.oob_data_records,
            owned.oob_perf_status,
            owned.oob_data_status,
        ),
        (
            DOMAIN_SHIPS,
            "ships",
            owned.ships_perf_records,
            owned.ships_data_records,
            owned.ships_perf_status,
            owned.ships_data_status,
        ),
    ] {
        if perf_count != 1 {
            errors.push(format!(
                "offset {offset}: closed hour {game_hour_raw} domain {domain_id} ({label}) PERF record count {perf_count} != 1"
            ));
        }
        if data_count != 1 {
            errors.push(format!(
                "offset {offset}: closed hour {game_hour_raw} domain {domain_id} ({label}) DATA record count {data_count} != 1"
            ));
        }
        if perf_count == 1 && data_count == 1 {
            if let (Some(perf_status), Some(data_status)) = (perf_status, data_status) {
                if perf_status != data_status {
                    errors.push(format!(
                        "offset {offset}: closed hour {game_hour_raw} domain {domain_id} ({label}) DATA status {data_status:#010x} does not match PERF status {perf_status:#010x}"
                    ));
                }
            }
        }
    }
}

fn validate_frame_end_authority(
    offset: usize,
    payload: &[u8],
    owned: &OwnedFrameAuthority,
    errors: &mut Vec<String>,
) {
    if payload.len() != 32 {
        errors.push(format!(
            "offset {offset}: FRAME_END payload length {} is not 32",
            payload.len()
        ));
        return;
    }
    let emitted_domains = read_u32_le(payload, 16);
    let scheduled_domains = read_u32_le(payload, 20);
    let status_flags = read_u32_le(payload, 24);
    let reserved = read_u32_le(payload, 28);
    if u64::from(emitted_domains) != owned.all_perf_records {
        errors.push(format!(
            "offset {offset}: FRAME_END emitted domain count {emitted_domains} does not match independently observed DOMAIN_PERF count {}",
            owned.all_perf_records
        ));
    }
    if u64::from(scheduled_domains) != owned.all_data_records {
        errors.push(format!(
            "offset {offset}: FRAME_END scheduled domain count {scheduled_domains} does not match independently observed DOMAIN_DATA count {}",
            owned.all_data_records
        ));
    }
    if emitted_domains != scheduled_domains {
        errors.push(format!(
            "offset {offset}: FRAME_END emitted domain count {emitted_domains} does not match scheduled domain count {scheduled_domains}"
        ));
    }
    if emitted_domains < 2 {
        errors.push(format!(
            "offset {offset}: FRAME_END emitted domain count {emitted_domains} cannot account for required domains 28 and 29"
        ));
    }
    if owned.perf_domains != owned.data_domains {
        errors.push(format!(
            "offset {offset}: closed-frame PERF domains {:?} do not match DATA domains {:?}",
            owned.perf_domains, owned.data_domains
        ));
    }
    let distinct_domains = u32::try_from(owned.perf_domains.len()).unwrap_or(u32::MAX);
    if emitted_domains != distinct_domains {
        errors.push(format!(
            "offset {offset}: FRAME_END emitted domain count {emitted_domains} does not match {distinct_domains} distinct paired domain(s)"
        ));
    }
    let unknown = compact_status_unknown_flags_v1(status_flags);
    if unknown != 0 {
        errors.push(format!(
            "offset {offset}: FRAME_END status flags {status_flags:#010x} contain unknown bits {unknown:#010x}"
        ));
    }
    let fatal = status_flags & COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        errors.push(format!(
            "offset {offset}: FRAME_END status flags {status_flags:#010x} contain fatal bits {fatal:#010x}"
        ));
    }
    if status_flags != owned.all_perf_status_or {
        errors.push(format!(
            "offset {offset}: FRAME_END status {status_flags:#010x} does not equal closed-frame PERF status aggregate {:#010x}",
            owned.all_perf_status_or
        ));
    }
    if reserved != 0 {
        errors.push(format!(
            "offset {offset}: FRAME_END reserved word is nonzero ({reserved:#010x})"
        ));
    }
}

fn closed_hour_is_successor(
    offset: usize,
    previous: Option<i32>,
    current: i32,
    errors: &mut Vec<String>,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    match previous.checked_add(1) {
        Some(expected) if current == expected => true,
        Some(expected) => {
            errors.push(format!(
                "offset {offset}: closed hour discontinuity: expected {expected}, got {current}"
            ));
            false
        }
        None => {
            errors.push(format!(
                "offset {offset}: closed hour cannot advance after {previous} without overflow"
            ));
            false
        }
    }
}

fn decode_domain_perf_record(
    offset: usize,
    domain_id: u16,
    payload: &[u8],
    summary: &mut NavalOobGateSummary,
) {
    let read_time_us = read_u64_le(payload, 8);
    let row_count = read_u32_le(payload, 16) as u64;
    let changed_rows = read_u32_le(payload, 20) as u64;
    let unit_count = read_u32_le(payload, 24) as u64;
    let changed_unit_count = read_u32_le(payload, 28) as u64;
    let raw_status_flags = read_u32_le(payload, 32);
    validate_known_status_flags(
        offset,
        domain_id,
        "DOMAIN_PERF",
        raw_status_flags,
        &mut summary.parse_errors,
    );
    let status_flags = status_for_global_interpretation(domain_id, raw_status_flags);

    let domain = summary.per_domain.entry(domain_id).or_default();
    domain.record_count = domain.record_count.saturating_add(1);
    domain.perf_record_count = domain.perf_record_count.saturating_add(1);
    domain.row_count_total = domain.row_count_total.saturating_add(row_count);
    domain.changed_rows_total = domain.changed_rows_total.saturating_add(changed_rows);
    domain.unit_count_total = domain.unit_count_total.saturating_add(unit_count);
    domain.changed_unit_count_total = domain
        .changed_unit_count_total
        .saturating_add(changed_unit_count);
    domain.max_read_time_us = domain.max_read_time_us.max(read_time_us);
    domain.status_flags_or |= status_flags;
}

pub fn decode_oob_command_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut NavalOobGateSummary,
) {
    if payload.len() < 48 {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command data payload is {} byte(s), expected at least 48",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let raw_status_flags = read_u32_le(payload, 16);
    validate_known_status_flags(
        offset,
        DOMAIN_OOB_COMMAND,
        "DOMAIN_DATA",
        raw_status_flags,
        &mut summary.parse_errors,
    );
    let status_flags = status_for_global_interpretation(DOMAIN_OOB_COMMAND, raw_status_flags);
    let root_rows = read_u32_le(payload, 20) as usize;
    let observed_army_rows = read_u32_le(payload, 24) as usize;
    let observed_army_group_rows = read_u32_le(payload, 28) as usize;
    let member_count = read_u32_le(payload, 32) as usize;
    let deleted_rows = read_u32_le(payload, 36) as usize;
    let observed_fleet_rows = read_u32_le(payload, 40) as usize;
    let observed_task_force_rows = read_u32_le(payload, 44) as usize;

    {
        let domain = summary.per_domain.entry(DOMAIN_OOB_COMMAND).or_default();
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }

    if member_count > COMPACT_OOB_MAX_MEMBER_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command member_count {member_count} exceeds limit {COMPACT_OOB_MAX_MEMBER_ROWS}"
        ));
    }
    let expected_len = 48usize
        .saturating_add(row_count.saturating_mul(COMPACT_OOB_ROW_LEN))
        .saturating_add(member_count.saturating_mul(COMPACT_OOB_MEMBER_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if observed_army_rows > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command observed_army_rows {observed_army_rows} > root_rows {root_rows}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }

    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_command_rows", row_count);
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_command_root_rows",
        root_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_observed_army_rows",
        observed_army_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_observed_army_group_rows",
        observed_army_group_rows,
    );
    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_member_rows", member_count);
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_deleted_rows",
        deleted_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_observed_fleet_rows",
        observed_fleet_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_observed_task_force_rows",
        observed_task_force_rows,
    );

    let mut cursor = 48usize;
    let mut seen_rows = BTreeSet::new();
    let mut declared_members = 0usize;
    let mut emitted_army_rows = 0usize;
    let mut emitted_army_group_rows = 0usize;
    let mut emitted_fleet_rows = 0usize;
    let mut emitted_task_force_rows = 0usize;
    let mut member_division_rows = 0usize;
    let mut member_army_rows = 0usize;
    let mut member_task_force_rows = 0usize;
    let mut member_ship_rows = 0usize;
    let mut leader_rows = 0usize;
    let mut leader_skill_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut location_rows = 0usize;
    let mut engine_count_rows = 0usize;
    let mut missing_count_rows = 0usize;

    for row_index in 0..row_count {
        let kind = read_u32_le(payload, cursor);
        let object_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let parent_id = read_u32_le(payload, cursor + 12);
        let owner_tag_db_id = read_u32_le(payload, cursor + 16);
        let leader_id = read_u32_le(payload, cursor + 20);
        let leader_skill = read_u32_le(payload, cursor + 24);
        let member_first = read_u32_le(payload, cursor + 28) as usize;
        let row_member_count = read_u32_le(payload, cursor + 32) as usize;
        let logical_member_count = read_u32_le(payload, cursor + 36) as usize;
        let member_count_engine = read_u32_le(payload, cursor + 40);
        let missing_member_count = read_u32_le(payload, cursor + 44);
        let leader_trait_count = read_u32_le(payload, cursor + 48);
        let parent_status = read_u32_le(payload, cursor + 52);
        let member_status = read_u32_le(payload, cursor + 56);
        let owner_status = read_u32_le(payload, cursor + 60);
        let leader_status = read_u32_le(payload, cursor + 64);
        let leader_trait_status = read_u32_le(payload, cursor + 68);
        let location_province_id = read_u32_le(payload, cursor + 72);
        let row_reserved1 = read_u32_le(payload, cursor + 76);
        cursor += COMPACT_OOB_ROW_LEN;

        let deleted = (flags & COMPACT_OOB_ROW_FLAG_DELETED) != 0;
        match kind {
            COMPACT_OOB_KIND_ARMY => emitted_army_rows = emitted_army_rows.saturating_add(1),
            COMPACT_OOB_KIND_ARMY_GROUP => {
                emitted_army_group_rows = emitted_army_group_rows.saturating_add(1)
            }
            COMPACT_OOB_KIND_FLEET => emitted_fleet_rows = emitted_fleet_rows.saturating_add(1),
            COMPACT_OOB_KIND_TASK_FORCE => {
                emitted_task_force_rows = emitted_task_force_rows.saturating_add(1)
            }
            _ => summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid kind {kind}"
            )),
        }
        if !valid_oob_id(object_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid object_id {object_id}"
            ));
        } else if !seen_rows.insert((kind, object_id, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} duplicates {kind}:{object_id}"
            ));
        }
        if flags
            & !(COMPACT_OOB_ROW_FLAG_DELETED
                | COMPACT_OOB_ROW_FLAG_PARENT
                | COMPACT_OOB_ROW_FLAG_OWNER
                | COMPACT_OOB_ROW_FLAG_LEADER
                | COMPACT_OOB_ROW_FLAG_LEADER_SKILL
                | COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE
                | COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT
                | COMPACT_OOB_ROW_FLAG_LOCATION
                | COMPACT_OOB_ROW_FLAG_LOCATION_UNREADABLE)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has unknown flags {flags}"
            ));
        }
        if deleted
            && (parent_id != 0
                || owner_tag_db_id != 0
                || leader_id != 0
                || leader_skill != 0
                || row_member_count != 0
                || logical_member_count != 0
                || member_count_engine != 0
                || missing_member_count != 0
                || leader_trait_count != 0
                || location_province_id != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command deleted row {row_index} has live fields"
            ));
        }

        validate_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_PARENT) != 0,
            parent_id,
            "parent_id",
        );
        validate_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_OWNER) != 0,
            owner_tag_db_id,
            "owner_tag_db_id",
        );
        validate_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_LEADER) != 0,
            leader_id,
            "leader_id",
        );
        if (flags & COMPACT_OOB_ROW_FLAG_OWNER) != 0 {
            owner_rows = owner_rows.saturating_add(1);
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LEADER) != 0 {
            leader_rows = leader_rows.saturating_add(1);
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LOCATION) != 0 {
            location_rows = location_rows.saturating_add(1);
            if location_province_id == 0
                || location_province_id > COMPACT_OOB_MAX_LOCATION_PROVINCE_ID
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid location_province_id {location_province_id}"
                ));
            }
        } else if location_province_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has location_province_id without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LOCATION_UNREADABLE) != 0 {
            if (flags & COMPACT_OOB_ROW_FLAG_LOCATION) != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} is both located and unreadable"
                ));
            }
            if kind != COMPACT_OOB_KIND_TASK_FORCE {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} claims an unreadable location on a kind that never carries one"
                ));
            }
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LEADER_SKILL) != 0 {
            leader_skill_rows = leader_skill_rows.saturating_add(1);
            if leader_skill > COMPACT_OOB_MAX_LEADER_SKILL {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid leader_skill {leader_skill}"
                ));
            }
        } else if leader_skill != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has leader_skill without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE) != 0 {
            engine_count_rows = engine_count_rows.saturating_add(1);
        } else if member_count_engine != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has engine member count without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT) != 0 {
            missing_count_rows = missing_count_rows.saturating_add(1);
        } else if missing_member_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has missing member count without flag"
            ));
        }
        // The producer's vocabulary is `COMPACT_OOB_STATUS_*`, 0..=6.
        // `SECOND_OBSERVATION_DISAGREED` (6) is producible on `member_status`
        // ONLY -- it says d28's two observations of a division disagreed -- so a
        // 6 on any other word is a row the collector cannot have written, and
        // this stays fail-closed about it (lane SD2).
        for (status_name, status_value, allows_second_observation) in [
            ("parent_status", parent_status, false),
            ("member_status", member_status, true),
            ("owner_status", owner_status, false),
            ("leader_status", leader_status, false),
            ("leader_trait_status", leader_trait_status, false),
        ] {
            // Code 6 rides on `member_status` AND only on an ARMY row: a
            // division second-observation disagreement is a statement about
            // army membership, and the collector never writes it to a fleet,
            // army-group or task-force row (Codex round 2, finding 5).
            let ceiling = if allows_second_observation && kind == COMPACT_OOB_KIND_ARMY {
                6
            } else {
                5
            };
            if status_value > ceiling {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid {status_name} {status_value}"
                ));
            }
        }
        if row_member_count == 0 {
            if member_first != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has member_first without members"
                ));
            }
        } else {
            let Some(member_end) = member_first.checked_add(row_member_count) else {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} member range overflows"
                ));
                continue;
            };
            if member_end > member_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} member range {member_first}..{member_end} exceeds {member_count}"
                ));
            }
            declared_members = declared_members.saturating_add(row_member_count);
            match kind {
                COMPACT_OOB_KIND_ARMY => {
                    member_division_rows = member_division_rows.saturating_add(row_member_count)
                }
                COMPACT_OOB_KIND_ARMY_GROUP => {
                    member_army_rows = member_army_rows.saturating_add(row_member_count)
                }
                COMPACT_OOB_KIND_FLEET => {
                    member_task_force_rows = member_task_force_rows.saturating_add(row_member_count)
                }
                COMPACT_OOB_KIND_TASK_FORCE => {
                    member_ship_rows = member_ship_rows.saturating_add(row_member_count)
                }
                _ => {}
            }
        }
        if member_status != COMPACT_OOB_STATUS_OVERFLOW && row_member_count != logical_member_count
        {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} member_count {row_member_count} != logical_member_count {logical_member_count}"
            ));
        }
        if row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} reserved field is nonzero"
            ));
        }
    }

    if declared_members != member_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command member_count {member_count} != declared {declared_members}"
        ));
    }

    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_army_rows",
        emitted_army_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_army_group_rows",
        emitted_army_group_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_fleet_rows",
        emitted_fleet_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_task_force_rows",
        emitted_task_force_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_member_division_rows",
        member_division_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_member_army_rows",
        member_army_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_member_task_force_rows",
        member_task_force_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_member_ship_rows",
        member_ship_rows,
    );
    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_leader_rows", leader_rows);
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_leader_skill_rows",
        leader_skill_rows,
    );
    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_owner_rows", owner_rows);
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_location_rows",
        location_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_engine_member_count_rows",
        engine_count_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_missing_member_count_rows",
        missing_count_rows,
    );

    let mut seen_member_rows = BTreeSet::new();
    for row_index in 0..member_count {
        let member_id = read_u32_le(payload, cursor);
        let row_reserved = read_u32_le(payload, cursor + 4);
        cursor += COMPACT_OOB_MEMBER_ROW_LEN;
        if !valid_oob_id(member_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} has invalid member_id {member_id}"
            ));
        }
        if !seen_member_rows.insert(row_index) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} duplicates member row index"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} reserved field is nonzero"
            ));
        }
    }
}

pub fn decode_ships_data_payload(offset: usize, payload: &[u8], summary: &mut NavalOobGateSummary) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let raw_status_flags = read_u32_le(payload, 16);
    validate_known_status_flags(
        offset,
        DOMAIN_SHIPS,
        "DOMAIN_DATA",
        raw_status_flags,
        &mut summary.parse_errors,
    );
    let status_flags = status_for_global_interpretation(DOMAIN_SHIPS, raw_status_flags);
    let observed_ship_rows = read_u32_le(payload, 20) as usize;
    let observed_task_force_rows = read_u32_le(payload, 24) as usize;
    let deleted_rows = read_u32_le(payload, 28) as usize;
    let scale_raw = read_u32_le(payload, 32);
    let reserved0 = read_u32_le(payload, 36);

    {
        let domain = summary.per_domain.entry(DOMAIN_SHIPS).or_default();
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }

    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: ships changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: ships deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_ship_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships row_count {row_count} > observed_ship_rows {observed_ship_rows}"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: ships scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships header reserved field is nonzero"
        ));
    }

    add_unit(summary, DOMAIN_SHIPS, "ship_rows", row_count);
    add_unit(
        summary,
        DOMAIN_SHIPS,
        "ship_observed_ship_rows",
        observed_ship_rows,
    );
    add_unit(
        summary,
        DOMAIN_SHIPS,
        "ship_observed_task_force_rows",
        observed_task_force_rows,
    );
    add_unit(summary, DOMAIN_SHIPS, "ship_deleted_rows", deleted_rows);

    let mut cursor = 40usize;
    let mut seen_rows = BTreeSet::new();
    let mut task_force_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut definition_rows = 0usize;

    for row_index in 0..row_count {
        if cursor.saturating_add(COMPACT_SHIP_ROW_BASE_LEN) > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row needs {COMPACT_SHIP_ROW_BASE_LEN} byte(s), only {} remain",
                payload.len().saturating_sub(cursor)
            ));
            return;
        }

        let instance_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let task_force_id = read_u32_le(payload, cursor + 8);
        let category_enum = read_u32_le(payload, cursor + 12);
        let strength_raw = read_u32_le(payload, cursor + 16);
        let organisation_raw = read_u32_le(payload, cursor + 20);
        let experience_raw = read_u32_le(payload, cursor + 24);
        let sunk_convoys = read_u32_le(payload, cursor + 28);
        let owner_tag_db_id = read_u32_le(payload, cursor + 32);
        let row_reserved1 = read_u32_le(payload, cursor + 36);
        cursor += COMPACT_SHIP_ROW_BASE_LEN;

        let definition_len = match skip_compact_string(payload, &mut cursor, "ships definition") {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };

        let deleted = (flags & COMPACT_SHIP_ROW_FLAG_DELETED) != 0;
        if !valid_ship_id(instance_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid instance_id {instance_id}"
            ));
        } else if !seen_rows.insert((instance_id, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} duplicates instance_id {instance_id}"
            ));
        }
        if flags
            & !(COMPACT_SHIP_ROW_FLAG_DELETED
                | COMPACT_SHIP_ROW_FLAG_TASK_FORCE
                | COMPACT_SHIP_ROW_FLAG_DEFINITION
                | COMPACT_SHIP_ROW_FLAG_OWNER)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_TASK_FORCE) != 0 {
            task_force_rows = task_force_rows.saturating_add(1);
            if !valid_oob_id(task_force_id) {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has invalid task_force_id {task_force_id}"
                ));
            }
        } else if task_force_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has task_force_id without flag"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_DEFINITION) != 0 {
            definition_rows = definition_rows.saturating_add(1);
            if definition_len == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has definition flag without definition"
                ));
            }
        } else if definition_len != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has definition without flag"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_OWNER) != 0 {
            owner_rows = owner_rows.saturating_add(1);
            if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_SHIP_MAX_OWNER_TAG_DB_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
                ));
            }
        } else if owner_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has owner_tag_db_id without flag"
            ));
        }
        if category_enum > COMPACT_SHIP_MAX_CATEGORY {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid category_enum {category_enum}"
            ));
        }
        if strength_raw > COMPACT_SHIP_MAX_STRENGTH_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid strength_raw {strength_raw}"
            ));
        }
        if organisation_raw > COMPACT_SHIP_MAX_ORGANISATION_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid organisation_raw {organisation_raw}"
            ));
        }
        if experience_raw > COMPACT_SHIP_MAX_EXPERIENCE_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid experience_raw {experience_raw}"
            ));
        }
        if deleted
            && (task_force_id != 0
                || category_enum != 0
                || strength_raw != 0
                || organisation_raw != 0
                || experience_raw != 0
                || sunk_convoys != 0
                || owner_tag_db_id != 0
                || definition_len != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: ships deleted row {row_index} has live fields"
            ));
        }
        if row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} reserved field is nonzero"
            ));
        }
    }

    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: ships data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    add_unit(
        summary,
        DOMAIN_SHIPS,
        "ship_task_force_rows",
        task_force_rows,
    );
    add_unit(summary, DOMAIN_SHIPS, "ship_owner_rows", owner_rows);
    add_unit(
        summary,
        DOMAIN_SHIPS,
        "ship_definition_rows",
        definition_rows,
    );
}

fn add_unit(summary: &mut NavalOobGateSummary, domain_id: u16, key: &'static str, value: usize) {
    let domain = summary.per_domain.entry(domain_id).or_default();
    *domain.data_units_total.entry(key).or_default() += value as u64;
}

fn validate_known_status_flags(
    offset: usize,
    domain_id: u16,
    record_label: &str,
    status_flags: u32,
    errors: &mut Vec<String>,
) {
    let normalized = status_for_global_interpretation(domain_id, status_flags);
    let unknown = compact_status_unknown_flags_v1(normalized);
    if unknown != 0 {
        errors.push(format!(
            "offset {offset}: domain {domain_id} {record_label} status flags {status_flags:#010x} contain unknown bits {unknown:#010x}"
        ));
    }
}

fn required_domain_ok(domain: Option<&DomainGateSummary>) -> bool {
    domain.is_some_and(|domain| {
        domain.data_record_count > 0
            && domain.data_rows_total > 0
            && domain.status_flags_or & COMPACT_STATUS_FAILURE_MASK == 0
    })
}

fn valid_oob_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_OOB_MAX_ID
}

fn valid_ship_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_SHIP_MAX_ID
}

fn validate_oob_optional_id(
    summary: &mut NavalOobGateSummary,
    offset: usize,
    row_index: usize,
    present: bool,
    value: u32,
    field: &str,
) {
    if present {
        if !valid_oob_id(value) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid {field} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command row {row_index} has {field} without flag"
        ));
    }
}

fn skip_compact_string(payload: &[u8], cursor: &mut usize, field: &str) -> Result<usize, String> {
    if cursor.saturating_add(2) > payload.len() {
        return Err(format!("{field} string length exceeds payload"));
    }
    let len = read_u16_le(payload, *cursor) as usize;
    *cursor += 2;
    if cursor.saturating_add(len) > payload.len() {
        return Err(format!(
            "{field} string length {len} exceeds remaining payload"
        ));
    }
    let bytes = &payload[*cursor..*cursor + len];
    if std::str::from_utf8(bytes).is_err() {
        return Err(format!("{field} string is not valid UTF-8"));
    }
    *cursor += len;
    Ok(len)
}

fn read_u16_le(payload: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(payload[offset..offset + 2].try_into().expect("u16 range"))
}

fn read_u32_le(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_u64_le(payload: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(payload[offset..offset + 8].try_into().expect("u64 range"))
}

fn main() {
    let program = env::args()
        .next()
        .unwrap_or_else(|| "naval_oob_gate".to_string());
    let mut stream_path = None;
    let mut minimum_game_hour_advancements = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--min-game-hours" {
            if minimum_game_hour_advancements.is_some() {
                eprintln!("--min-game-hours may be supplied only once");
                process::exit(2);
            }
            let Some(value) = args.next() else {
                eprintln!("--min-game-hours requires a value");
                process::exit(2);
            };
            let parsed = match value.parse::<i64>() {
                Ok(parsed) if parsed > 0 => parsed,
                Ok(parsed) => {
                    eprintln!("--min-game-hours must be explicitly positive, got {parsed}");
                    process::exit(2);
                }
                Err(error) => {
                    eprintln!("invalid --min-game-hours {value:?}: {error}");
                    process::exit(2);
                }
            };
            minimum_game_hour_advancements = Some(parsed);
        } else if stream_path.is_none() {
            stream_path = Some(arg);
        } else {
            eprintln!(
                "usage: {program} <collector_stream.bin> --min-game-hours <positive advancements>"
            );
            process::exit(2);
        }
    }
    let Some(stream_path) = stream_path else {
        eprintln!(
            "usage: {program} <collector_stream.bin> --min-game-hours <positive advancements>"
        );
        process::exit(2);
    };
    let Some(minimum_game_hour_advancements) = minimum_game_hour_advancements else {
        eprintln!(
            "--min-game-hours is required explicitly; mutable manifest fallback is forbidden"
        );
        process::exit(2);
    };
    let raw = match fs::read(&stream_path) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!("failed to read {stream_path}: {err}");
            process::exit(2);
        }
    };
    let summary = decode_stream(&raw, minimum_game_hour_advancements);
    println!("{}", summary_json(&summary));
    if !summary.gate_passed() {
        process::exit(1);
    }
}

fn summary_json(summary: &NavalOobGateSummary) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    push_json_bool_field(&mut out, "gate_passed", summary.gate_passed(), true);
    push_json_u64_field(&mut out, "record_count", summary.record_count, true);
    push_json_u64_field(
        &mut out,
        "domain_perf_record_count",
        summary.domain_perf_record_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "domain_data_record_count",
        summary.domain_data_record_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "frame_end_record_count",
        summary.frame_end_record_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "frame_end_status_flags_or",
        u64::from(summary.frame_end_status_flags_or),
        true,
    );
    push_json_u64_field(
        &mut out,
        "total_payload_bytes",
        summary.total_payload_bytes,
        true,
    );
    push_json_usize_field(&mut out, "max_payload_len", summary.max_payload_len, true);
    push_json_usize_field(&mut out, "max_record_len", summary.max_record_len, true);
    push_json_opt_i32_field(
        &mut out,
        "first_game_hour_raw",
        summary.first_game_hour_raw,
        true,
    );
    push_json_opt_i32_field(
        &mut out,
        "last_game_hour_raw",
        summary.last_game_hour_raw,
        true,
    );
    push_json_usize_field(
        &mut out,
        "unique_game_hour_count",
        summary.unique_game_hour_count,
        true,
    );
    push_json_i64_field(
        &mut out,
        "required_game_hour_advancements",
        summary.required_game_hour_advancements,
        true,
    );
    push_json_opt_i64_field(
        &mut out,
        "observed_game_hour_advancements",
        summary.observed_game_hour_advancements,
        true,
    );
    push_json_u64_field(
        &mut out,
        "required_closed_frame_count",
        summary.required_closed_frame_count,
        true,
    );

    out.push_str("  \"domains_seen\": [");
    for (index, domain_id) in summary.domains_seen.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&domain_id.to_string());
    }
    out.push_str("],\n");

    out.push_str("  \"per_domain\": {\n");
    for (idx, domain_id) in [DOMAIN_OOB_COMMAND, DOMAIN_SHIPS].iter().enumerate() {
        if idx > 0 {
            out.push_str(",\n");
        }
        out.push_str("    ");
        push_json_str(&mut out, &domain_id.to_string());
        out.push_str(": ");
        if let Some(domain) = summary.per_domain.get(domain_id) {
            out.push_str(&domain_summary_json(*domain_id, domain));
        } else {
            out.push_str("null");
        }
    }
    out.push_str("\n  },\n");

    out.push_str("  \"parse_errors\": ");
    push_json_str_array(&mut out, &summary.parse_errors);
    out.push_str(",\n");
    out.push_str("  \"ordering_errors\": ");
    push_json_str_array(&mut out, &summary.ordering_errors);
    out.push_str("\n}\n");
    out
}

fn domain_summary_json(domain_id: u16, domain: &DomainGateSummary) -> String {
    let mut out = String::new();
    out.push_str("{");
    out.push_str("\"name\":");
    push_json_str(&mut out, domain_name(domain_id));
    out.push_str(",\"record_count\":");
    out.push_str(&domain.record_count.to_string());
    out.push_str(",\"perf_record_count\":");
    out.push_str(&domain.perf_record_count.to_string());
    out.push_str(",\"data_record_count\":");
    out.push_str(&domain.data_record_count.to_string());
    out.push_str(",\"row_count_total\":");
    out.push_str(&domain.row_count_total.to_string());
    out.push_str(",\"changed_rows_total\":");
    out.push_str(&domain.changed_rows_total.to_string());
    out.push_str(",\"unit_count_total\":");
    out.push_str(&domain.unit_count_total.to_string());
    out.push_str(",\"changed_unit_count_total\":");
    out.push_str(&domain.changed_unit_count_total.to_string());
    out.push_str(",\"data_rows_total\":");
    out.push_str(&domain.data_rows_total.to_string());
    out.push_str(",\"data_changed_rows_total\":");
    out.push_str(&domain.data_changed_rows_total.to_string());
    out.push_str(",\"status_flags_or\":");
    out.push_str(&domain.status_flags_or.to_string());
    out.push_str(",\"failure_flags_or\":");
    out.push_str(&(domain.status_flags_or & COMPACT_STATUS_FAILURE_MASK).to_string());
    out.push_str(",\"max_read_time_us\":");
    out.push_str(&domain.max_read_time_us.to_string());
    out.push_str(",\"data_units_total\":{");
    for (idx, (key, value)) in domain.data_units_total.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        push_json_str(&mut out, key);
        out.push(':');
        out.push_str(&value.to_string());
    }
    out.push_str("}}");
    out
}

fn domain_name(domain_id: u16) -> &'static str {
    match domain_id {
        DOMAIN_OOB_COMMAND => "oob_command",
        DOMAIN_SHIPS => "ships",
        _ => "unknown",
    }
}

fn push_json_bool_field(out: &mut String, name: &str, value: bool, comma: bool) {
    out.push_str("  ");
    push_json_str(out, name);
    out.push_str(": ");
    out.push_str(if value { "true" } else { "false" });
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_u64_field(out: &mut String, name: &str, value: u64, comma: bool) {
    out.push_str("  ");
    push_json_str(out, name);
    out.push_str(": ");
    out.push_str(&value.to_string());
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_usize_field(out: &mut String, name: &str, value: usize, comma: bool) {
    out.push_str("  ");
    push_json_str(out, name);
    out.push_str(": ");
    out.push_str(&value.to_string());
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_i64_field(out: &mut String, name: &str, value: i64, comma: bool) {
    out.push_str("  ");
    push_json_str(out, name);
    out.push_str(": ");
    out.push_str(&value.to_string());
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_opt_i64_field(out: &mut String, name: &str, value: Option<i64>, comma: bool) {
    out.push_str("  ");
    push_json_str(out, name);
    out.push_str(": ");
    match value {
        Some(value) => out.push_str(&value.to_string()),
        None => out.push_str("null"),
    }
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_opt_i32_field(out: &mut String, name: &str, value: Option<i32>, comma: bool) {
    out.push_str("  ");
    push_json_str(out, name);
    out.push_str(": ");
    match value {
        Some(value) => out.push_str(&value.to_string()),
        None => out.push_str("null"),
    }
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_str_array(out: &mut String, values: &[String]) {
    out.push('[');
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        push_json_str(out, value);
    }
    out.push(']');
}

fn push_json_str(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_oob_command_fleet_and_task_force_members() {
        let payload = test_oob_command_data_payload(COMPACT_STATUS_DELTA_DATA);
        let mut summary = NavalOobGateSummary::default();
        summary
            .per_domain
            .entry(DOMAIN_OOB_COMMAND)
            .or_default()
            .data_record_count = 1;
        decode_oob_command_data_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let domain = summary.per_domain.get(&DOMAIN_OOB_COMMAND).unwrap();
        assert_eq!(domain.data_rows_total, 4);
        assert_eq!(domain.data_changed_rows_total, 4);
        assert_eq!(domain.status_flags_or, COMPACT_STATUS_DELTA_DATA);
        assert_eq!(domain.data_units_total.get("oob_fleet_rows"), Some(&1));
        assert_eq!(domain.data_units_total.get("oob_task_force_rows"), Some(&1));
        assert_eq!(
            domain.data_units_total.get("oob_member_task_force_rows"),
            Some(&1)
        );
        assert_eq!(
            domain.data_units_total.get("oob_member_ship_rows"),
            Some(&2)
        );
        assert_eq!(domain.data_units_total.get("oob_location_rows"), Some(&1));
    }

    #[test]
    fn decodes_ship_rows_with_owner_definition_and_delete() {
        let payload = test_ships_data_payload(COMPACT_STATUS_DELTA_DATA);
        let mut summary = NavalOobGateSummary::default();
        summary
            .per_domain
            .entry(DOMAIN_SHIPS)
            .or_default()
            .data_record_count = 1;
        decode_ships_data_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let domain = summary.per_domain.get(&DOMAIN_SHIPS).unwrap();
        assert_eq!(domain.data_rows_total, 2);
        assert_eq!(domain.data_changed_rows_total, 2);
        assert_eq!(
            domain.data_units_total.get("ship_task_force_rows"),
            Some(&1)
        );
        assert_eq!(domain.data_units_total.get("ship_owner_rows"), Some(&1));
        assert_eq!(
            domain.data_units_total.get("ship_definition_rows"),
            Some(&1)
        );
        assert_eq!(domain.data_units_total.get("ship_deleted_rows"), Some(&1));
    }

    #[test]
    fn decodes_compact_stream_and_passes_required_gate() {
        let mut raw = Vec::new();
        let mut sequence = 1;
        append_valid_owned_frame(&mut raw, &mut sequence, 60787539);
        append_valid_owned_frame(&mut raw, &mut sequence, 60787540);

        let summary = decode_stream(&raw, 1);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(
            summary.ordering_errors.is_empty(),
            "{:?}",
            summary.ordering_errors
        );
        assert!(summary.gate_passed(), "{}", summary_json(&summary));
        assert_eq!(summary.record_count, 10);
        assert_eq!(summary.frame_end_record_count, 2);
        assert_eq!(summary.unique_game_hour_count, 2);
        assert_eq!(summary.observed_game_hour_advancements, Some(1));
    }

    #[test]
    fn rejects_sequence_flags_payload_lengths_and_unclean_tail() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&compact_record_with_flags(
            2,
            100,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            1 << 31,
            &test_oob_command_data_payload(0),
        ));
        raw.extend_from_slice(&compact_record(
            3,
            100,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &[0; 39],
        ));
        raw.extend_from_slice(&compact_record(
            4,
            101,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0; 31],
        ));
        raw.extend_from_slice(&compact_record(
            5,
            101,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &[0; 40],
        ));

        let summary = decode_stream(&raw, 1);
        assert!(summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("expected 1")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("record flags")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("shorter than 40")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("is not 32")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("clean frame-end boundary")));

        let mut gap = Vec::new();
        gap.extend_from_slice(&compact_record(
            1,
            100,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_oob_command_data_payload(0),
        ));
        gap.extend_from_slice(&compact_record(
            3,
            100,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(1, 4, 4, 4, 4, 0),
        ));
        assert!(decode_stream(&gap, 1)
            .ordering_errors
            .iter()
            .any(|error| error.contains("sequence discontinuity: expected 2, got 3")));

        let mut exhausted = Vec::new();
        for sequence in [1, u64::MAX, 0] {
            exhausted.extend_from_slice(&compact_record(
                sequence,
                100,
                DOMAIN_OOB_COMMAND,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &test_oob_command_data_payload(0),
            ));
        }
        assert!(decode_stream(&exhausted, 1)
            .ordering_errors
            .iter()
            .any(|error| error.contains("sequence cannot advance after 18446744073709551615")));
    }

    #[test]
    fn accepts_unowned_records_and_perf_before_data() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&compact_record(
            1,
            100,
            51,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &[0; 40],
        ));
        raw.extend_from_slice(&compact_record(
            2,
            100,
            51,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0xA5; 7],
        ));
        raw.extend_from_slice(&compact_record(
            3,
            100,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(1, 4, 4, 4, 4, 0),
        ));
        raw.extend_from_slice(&compact_record(
            4,
            100,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_oob_command_data_payload(0),
        ));
        raw.extend_from_slice(&compact_record(
            5,
            100,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(1, 2, 2, 2, 2, 0),
        ));
        raw.extend_from_slice(&compact_record(
            6,
            100,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_ships_data_payload(0),
        ));
        raw.extend_from_slice(&compact_record(
            7,
            100,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_end_payload(0, 3, 3, 0),
        ));
        let mut sequence = 8;
        append_valid_owned_frame(&mut raw, &mut sequence, 101);

        let summary = decode_stream(&raw, 1);
        assert!(summary.gate_passed(), "{}", summary_json(&summary));
        assert!(!summary.per_domain.contains_key(&51));
    }

    #[test]
    fn rejects_nonconsecutive_physical_closures() {
        let mut raw = Vec::new();
        let mut sequence = 1u64;
        for hour in [100i32, 99, 101] {
            raw.extend_from_slice(&compact_record(
                sequence,
                hour,
                DOMAIN_OOB_COMMAND,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                &test_domain_perf_payload(1, 4, 4, 4, 4, 0),
            ));
            sequence += 1;
            raw.extend_from_slice(&compact_record(
                sequence,
                hour,
                DOMAIN_OOB_COMMAND,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &test_oob_command_data_payload(0),
            ));
            sequence += 1;
            raw.extend_from_slice(&compact_record(
                sequence,
                hour,
                DOMAIN_SHIPS,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                &test_domain_perf_payload(1, 2, 2, 2, 2, 0),
            ));
            sequence += 1;
            raw.extend_from_slice(&compact_record(
                sequence,
                hour,
                DOMAIN_SHIPS,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &test_ships_data_payload(0),
            ));
            sequence += 1;
            raw.extend_from_slice(&compact_record(
                sequence,
                hour,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &test_frame_end_payload(0, 2, 2, 0),
            ));
            sequence += 1;
        }

        let summary = decode_stream(&raw, 1);
        assert!(summary
            .ordering_errors
            .iter()
            .any(|error| error.contains("closed hour discontinuity: expected 101, got 99")));
    }

    #[test]
    fn uses_full_canonical_failure_mask_after_normalization() {
        let fatal = (1 << 0)
            | (1 << 1)
            | (1 << 2)
            | (1 << 3)
            | (1 << 6)
            | (1 << 7)
            | (1 << 8)
            | (1 << 9)
            | (1 << 12);
        assert_eq!(fatal, COMPACT_STATUS_FAILURE_MASK);
        assert_eq!(status_for_global_interpretation(51, 1 << 12), 0);
        assert_eq!(
            status_for_global_interpretation(DOMAIN_SHIPS, 1 << 12),
            1 << 12
        );

        let mut raw = Vec::new();
        raw.extend_from_slice(&compact_record(
            1,
            100,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_oob_command_data_payload(fatal),
        ));
        raw.extend_from_slice(&compact_record(
            2,
            100,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_ships_data_payload(0),
        ));
        raw.extend_from_slice(&compact_record(
            3,
            100,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_end_payload(0, 2, 2, 0),
        ));
        let summary = decode_stream(&raw, 1);
        assert_eq!(
            summary.per_domain[&DOMAIN_OOB_COMMAND].status_flags_or & COMPACT_STATUS_FAILURE_MASK,
            fatal
        );
        assert!(!summary.gate_passed());
    }

    #[test]
    fn validates_announced_perf_trailer_envelopes() {
        fn stream_with_perf(flags: u32, perf: &[u8]) -> NavalOobGateSummary {
            let mut raw = Vec::new();
            raw.extend_from_slice(&compact_record(
                1,
                100,
                DOMAIN_OOB_COMMAND,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &test_oob_command_data_payload(0),
            ));
            raw.extend_from_slice(&compact_record_with_flags(
                2,
                100,
                DOMAIN_OOB_COMMAND,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                flags,
                perf,
            ));
            raw.extend_from_slice(&compact_record(
                3,
                100,
                DOMAIN_SHIPS,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                &test_domain_perf_payload(1, 2, 2, 2, 2, 0),
            ));
            raw.extend_from_slice(&compact_record(
                4,
                100,
                DOMAIN_SHIPS,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &test_ships_data_payload(0),
            ));
            raw.extend_from_slice(&compact_record(
                5,
                100,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &test_frame_end_payload(0, 2, 2, 0),
            ));
            let mut sequence = 6;
            append_valid_owned_frame(&mut raw, &mut sequence, 101);
            decode_stream(&raw, 1)
        }

        let base = vec![0; 40];
        let trailer = empty_pdu1_perf_payload();
        assert!(
            stream_with_perf(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &trailer)
                .parse_errors
                .is_empty()
        );
        assert!(
            stream_with_perf(COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, &base)
                .parse_errors
                .iter()
                .any(|error| error.contains("trailer header is truncated"))
        );
        assert!(stream_with_perf(0, &trailer)
            .parse_errors
            .iter()
            .any(|error| error.contains("unannounced trailer")));
    }

    #[test]
    fn requires_explicit_positive_minimum_duration_and_both_closed_endpoints() {
        let mut one_frame = Vec::new();
        let mut sequence = 1;
        append_valid_owned_frame(&mut one_frame, &mut sequence, 100);

        let missing_advancement = decode_stream(&one_frame, 1);
        assert!(missing_advancement.parse_errors.iter().any(|error| {
            error.contains("production proof duration is 0 advancement(s) across 1 closed frame(s)")
        }));
        assert!(decode_stream(&one_frame, 0)
            .parse_errors
            .iter()
            .any(|error| error.contains("must be explicitly positive")));

        let mut two_frames = one_frame;
        append_valid_owned_frame(&mut two_frames, &mut sequence, 101);
        assert!(decode_stream(&two_frames, 1).gate_passed());
        assert!(decode_stream(&two_frames, 2)
            .parse_errors
            .iter()
            .any(|error| error.contains("below required 2 advancement(s) / 3 closed frame(s)")));
    }

    #[test]
    fn requires_exactly_one_perf_and_data_for_both_owned_domains_per_hour() {
        for (counts, expected) in [
            (
                (0, 1, 1, 1),
                "domain 28 (oob_command) PERF record count 0 != 1",
            ),
            (
                (2, 1, 1, 1),
                "domain 28 (oob_command) PERF record count 2 != 1",
            ),
            (
                (1, 0, 1, 1),
                "domain 28 (oob_command) DATA record count 0 != 1",
            ),
            (
                (1, 2, 1, 1),
                "domain 28 (oob_command) DATA record count 2 != 1",
            ),
            ((1, 1, 0, 1), "domain 29 (ships) PERF record count 0 != 1"),
            ((1, 1, 2, 1), "domain 29 (ships) PERF record count 2 != 1"),
            ((1, 1, 1, 0), "domain 29 (ships) DATA record count 0 != 1"),
            ((1, 1, 1, 2), "domain 29 (ships) DATA record count 2 != 1"),
        ] {
            let mut raw = Vec::new();
            let mut sequence = 1;
            append_owned_frame_with_counts(&mut raw, &mut sequence, 100, counts);
            append_valid_owned_frame(&mut raw, &mut sequence, 101);
            let summary = decode_stream(&raw, 1);
            assert!(
                summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "expected {expected:?}, got {:?}",
                summary.parse_errors
            );
            assert!(!summary.gate_passed());
        }
    }

    #[test]
    fn rejects_unhealthy_frame_end_counts_status_reserved_and_trailing_records() {
        for (frame_payload, expected) in [
            (
                test_frame_end_payload(0, 3, 3, 0),
                "does not match independently observed DOMAIN_PERF count 2",
            ),
            (
                test_frame_end_payload(0, 1, 2, 0),
                "emitted domain count 1 does not match scheduled domain count 2",
            ),
            (
                test_frame_end_payload(0, 1, 1, 0),
                "cannot account for required domains 28 and 29",
            ),
            (
                test_frame_end_payload(COMPACT_STATUS_SOURCE_UNAVAILABLE, 2, 2, 0),
                "contain fatal bits",
            ),
            (
                test_frame_end_payload(1 << 31, 2, 2, 0),
                "contain unknown bits",
            ),
            (
                test_frame_end_payload(0, 2, 2, 7),
                "reserved word is nonzero",
            ),
        ] {
            let mut raw = Vec::new();
            let mut sequence = 1;
            append_owned_frame_with_payload(&mut raw, &mut sequence, 100, &frame_payload);
            append_valid_owned_frame(&mut raw, &mut sequence, 101);
            let summary = decode_stream(&raw, 1);
            assert!(
                summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "expected {expected:?}, got {:?}",
                summary.parse_errors
            );
        }

        let mut trailing = Vec::new();
        let mut sequence = 1;
        append_valid_owned_frame(&mut trailing, &mut sequence, 100);
        append_valid_owned_frame(&mut trailing, &mut sequence, 101);
        trailing.extend_from_slice(&compact_record(
            sequence,
            102,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(1, 4, 4, 4, 4, 0),
        ));
        let trailing_summary = decode_stream(&trailing, 1);
        assert!(trailing_summary
            .parse_errors
            .iter()
            .any(|error| error.contains("clean frame-end boundary")));

        let mut trailing_bytes = Vec::new();
        let mut sequence = 1;
        append_valid_owned_frame(&mut trailing_bytes, &mut sequence, 100);
        append_valid_owned_frame(&mut trailing_bytes, &mut sequence, 101);
        trailing_bytes.extend_from_slice(&[0xA5; 17]);
        assert!(decode_stream(&trailing_bytes, 1)
            .parse_errors
            .iter()
            .any(|error| error.contains("trailing 17 byte(s)")));
    }

    #[test]
    fn rejects_frame_roster_or_status_aggregate_forgery() {
        let mut mismatched_roster = Vec::new();
        let mut sequence = 1u64;
        for hour in [100, 101] {
            for (domain_id, record_type, payload) in [
                (
                    DOMAIN_OOB_COMMAND,
                    COMPACT_RECORD_TYPE_DOMAIN_PERF,
                    test_domain_perf_payload(1, 4, 4, 4, 4, 0),
                ),
                (
                    DOMAIN_OOB_COMMAND,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    test_oob_command_data_payload(0),
                ),
                (
                    DOMAIN_SHIPS,
                    COMPACT_RECORD_TYPE_DOMAIN_PERF,
                    test_domain_perf_payload(1, 2, 2, 2, 2, 0),
                ),
                (77, COMPACT_RECORD_TYPE_DOMAIN_DATA, vec![0xA5]),
            ] {
                mismatched_roster.extend_from_slice(&compact_record(
                    sequence,
                    hour,
                    domain_id,
                    record_type,
                    &payload,
                ));
                sequence += 1;
            }
            mismatched_roster.extend_from_slice(&compact_record(
                sequence,
                hour,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &test_frame_end_payload(0, 2, 2, 0),
            ));
            sequence += 1;
        }
        assert!(decode_stream(&mismatched_roster, 1)
            .parse_errors
            .iter()
            .any(|error| error.contains("closed-frame PERF domains")
                && error.contains("DATA domains")));

        let mut aggregate = Vec::new();
        let mut sequence = 1;
        append_owned_frame_with_payload(
            &mut aggregate,
            &mut sequence,
            100,
            &test_frame_end_payload(COMPACT_STATUS_DELTA_DATA, 2, 2, 0),
        );
        append_valid_owned_frame(&mut aggregate, &mut sequence, 101);
        assert!(decode_stream(&aggregate, 1)
            .parse_errors
            .iter()
            .any(|error| error.contains("does not equal closed-frame PERF status aggregate")));
    }

    #[test]
    fn rejects_probe_data_and_duplicate_unowned_frame_records() {
        let mut raw = Vec::new();
        let mut sequence = 1u64;
        for hour in [100, 101] {
            append_owned_frame_with_extra_unowned_duplicates(
                &mut raw,
                &mut sequence,
                hour,
                hour == 100,
            );
        }
        let summary = decode_stream(&raw, 1);
        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("does not match independently observed DOMAIN_PERF count 4")
        }));
        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("does not match independently observed DOMAIN_DATA count 4")
        }));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("domain 250 is PERF-only; DATA is forbidden")));
    }

    #[test]
    fn reserved_probe_perf_is_ordered_but_excluded_from_frame_authority() {
        let mut raw = Vec::new();
        let mut sequence = 1;
        for (domain_id, record_type, payload) in [
            (
                DOMAIN_OOB_COMMAND,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                test_domain_perf_payload(1, 4, 4, 4, 4, 0),
            ),
            (
                DOMAIN_OOB_COMMAND,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                test_oob_command_data_payload(0),
            ),
            (
                DOMAIN_SHIPS,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                test_domain_perf_payload(1, 2, 2, 2, 2, 0),
            ),
            (
                DOMAIN_SHIPS,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                test_ships_data_payload(0),
            ),
            (
                RC_PROBE_DOMAIN_ID,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                test_domain_perf_payload(1, 0, 0, 0, 0, 0),
            ),
        ] {
            raw.extend_from_slice(&compact_record(
                sequence,
                100,
                domain_id,
                record_type,
                &payload,
            ));
            sequence += 1;
        }
        raw.extend_from_slice(&compact_record(
            sequence,
            100,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_end_payload(0, 2, 2, 0),
        ));
        sequence += 1;
        append_valid_owned_frame(&mut raw, &mut sequence, 101);

        let summary = decode_stream(&raw, 1);
        assert!(summary.gate_passed(), "{}", summary_json(&summary));
        assert_eq!(summary.domain_perf_record_count, 4);
    }

    #[test]
    fn rejects_unknown_owned_status_flags() {
        let mut raw = Vec::new();
        let mut sequence = 1;
        append_owned_frame_with_status(&mut raw, &mut sequence, 100, 1 << 31);
        append_valid_owned_frame(&mut raw, &mut sequence, 101);
        let summary = decode_stream(&raw, 1);
        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("domain 28 DOMAIN_DATA status flags")
                && error.contains("unknown bits 0x80000000")
        }));

        let mut mismatch = Vec::new();
        let mut sequence = 1;
        append_owned_frame_with_status(
            &mut mismatch,
            &mut sequence,
            100,
            COMPACT_STATUS_DELTA_DATA,
        );
        append_valid_owned_frame(&mut mismatch, &mut sequence, 101);
        assert!(decode_stream(&mismatch, 1)
            .parse_errors
            .iter()
            .any(|error| {
                error.contains("domain 28 (oob_command) DATA status")
                    && error.contains("does not match PERF status")
            }));
    }

    #[test]
    fn malformed_owned_data_fails_closed_without_panicking() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&compact_record(
            1,
            100,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(1, 0, 0, 0, 0, 0),
        ));
        raw.extend_from_slice(&compact_record(
            2,
            100,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[],
        ));
        raw.extend_from_slice(&compact_record(
            3,
            100,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(1, 2, 2, 2, 2, 0),
        ));
        raw.extend_from_slice(&compact_record(
            4,
            100,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_ships_data_payload(0),
        ));
        raw.extend_from_slice(&compact_record(
            5,
            100,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_end_payload(0, 2, 2, 0),
        ));
        let mut sequence = 6;
        append_valid_owned_frame(&mut raw, &mut sequence, 101);

        let result = std::panic::catch_unwind(|| decode_stream(&raw, 1));
        let summary = result.expect("short owned DATA must fail closed, not panic");
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("expected at least 48")));
        assert!(!summary.gate_passed());
    }

    #[test]
    fn rejects_bad_ship_scale() {
        let mut payload = test_ships_data_payload(COMPACT_STATUS_DELTA_DATA);
        payload[32..36].copy_from_slice(&1u32.to_le_bytes());
        let mut summary = NavalOobGateSummary::default();
        decode_ships_data_payload(7, &payload, &mut summary);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("scale_raw 1")));
    }

    #[test]
    fn rejects_bad_oob_member_range() {
        let mut payload = test_oob_command_data_payload(COMPACT_STATUS_DELTA_DATA);
        let first_row_member_count_offset = 48 + 32;
        payload[first_row_member_count_offset..first_row_member_count_offset + 4]
            .copy_from_slice(&999u32.to_le_bytes());
        let mut summary = NavalOobGateSummary::default();
        decode_oob_command_data_payload(11, &payload, &mut summary);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("member range")));
    }

    fn test_domain_perf_payload(
        read_time_us: u64,
        row_count: u32,
        changed_rows: u32,
        unit_count: u32,
        changed_unit_count: u32,
        status_flags: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&read_time_us.to_le_bytes());
        payload.extend_from_slice(&row_count.to_le_bytes());
        payload.extend_from_slice(&changed_rows.to_le_bytes());
        payload.extend_from_slice(&unit_count.to_le_bytes());
        payload.extend_from_slice(&changed_unit_count.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn test_frame_end_payload(
        status_flags: u32,
        emitted_domains: u32,
        scheduled_domains: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::with_capacity(COMPACT_FRAME_END_PAYLOAD_LEN);
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&emitted_domains.to_le_bytes());
        payload.extend_from_slice(&scheduled_domains.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&reserved.to_le_bytes());
        payload
    }

    fn append_valid_owned_frame(raw: &mut Vec<u8>, sequence: &mut u64, hour: i32) {
        append_owned_frame_with_payload(raw, sequence, hour, &test_frame_end_payload(0, 2, 2, 0));
    }

    fn append_owned_frame_with_extra_unowned_duplicates(
        raw: &mut Vec<u8>,
        sequence: &mut u64,
        hour: i32,
        include_probe_data: bool,
    ) {
        for (domain_id, record_type, payload) in [
            (
                DOMAIN_OOB_COMMAND,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                test_domain_perf_payload(1, 4, 4, 4, 4, 0),
            ),
            (
                DOMAIN_OOB_COMMAND,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                test_oob_command_data_payload(0),
            ),
            (
                DOMAIN_SHIPS,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                test_domain_perf_payload(1, 2, 2, 2, 2, 0),
            ),
            (
                DOMAIN_SHIPS,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                test_ships_data_payload(0),
            ),
            (77, COMPACT_RECORD_TYPE_DOMAIN_PERF, vec![0; 40]),
            (77, COMPACT_RECORD_TYPE_DOMAIN_PERF, vec![0; 40]),
            (77, COMPACT_RECORD_TYPE_DOMAIN_DATA, vec![1]),
            (77, COMPACT_RECORD_TYPE_DOMAIN_DATA, vec![2]),
        ] {
            raw.extend_from_slice(&compact_record(
                *sequence,
                hour,
                domain_id,
                record_type,
                &payload,
            ));
            *sequence += 1;
        }
        if include_probe_data {
            raw.extend_from_slice(&compact_record(
                *sequence,
                hour,
                RC_PROBE_DOMAIN_ID,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &[0],
            ));
            *sequence += 1;
        }
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_end_payload(0, 3, 3, 0),
        ));
        *sequence += 1;
    }

    fn append_owned_frame_with_payload(
        raw: &mut Vec<u8>,
        sequence: &mut u64,
        hour: i32,
        frame_payload: &[u8],
    ) {
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(44_000, 4, 4, 4, 4, 0),
        ));
        *sequence += 1;
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_oob_command_data_payload(0),
        ));
        *sequence += 1;
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(33_000, 2, 2, 2, 2, 0),
        ));
        *sequence += 1;
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_ships_data_payload(0),
        ));
        *sequence += 1;
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            frame_payload,
        ));
        *sequence += 1;
    }

    fn append_owned_frame_with_counts(
        raw: &mut Vec<u8>,
        sequence: &mut u64,
        hour: i32,
        counts: (u64, u64, u64, u64),
    ) {
        for _ in 0..counts.0 {
            raw.extend_from_slice(&compact_record(
                *sequence,
                hour,
                DOMAIN_OOB_COMMAND,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                &test_domain_perf_payload(1, 4, 4, 4, 4, 0),
            ));
            *sequence += 1;
        }
        for _ in 0..counts.1 {
            raw.extend_from_slice(&compact_record(
                *sequence,
                hour,
                DOMAIN_OOB_COMMAND,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &test_oob_command_data_payload(0),
            ));
            *sequence += 1;
        }
        for _ in 0..counts.2 {
            raw.extend_from_slice(&compact_record(
                *sequence,
                hour,
                DOMAIN_SHIPS,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                &test_domain_perf_payload(1, 2, 2, 2, 2, 0),
            ));
            *sequence += 1;
        }
        for _ in 0..counts.3 {
            raw.extend_from_slice(&compact_record(
                *sequence,
                hour,
                DOMAIN_SHIPS,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &test_ships_data_payload(0),
            ));
            *sequence += 1;
        }
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_end_payload(0, 2, 2, 0),
        ));
        *sequence += 1;
    }

    fn append_owned_frame_with_status(
        raw: &mut Vec<u8>,
        sequence: &mut u64,
        hour: i32,
        data_status_flags: u32,
    ) {
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(1, 4, 4, 4, 4, 0),
        ));
        *sequence += 1;
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_oob_command_data_payload(data_status_flags),
        ));
        *sequence += 1;
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(1, 2, 2, 2, 2, 0),
        ));
        *sequence += 1;
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_ships_data_payload(0),
        ));
        *sequence += 1;
        raw.extend_from_slice(&compact_record(
            *sequence,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &test_frame_end_payload(0, 2, 2, 0),
        ));
        *sequence += 1;
    }

    fn compact_record(
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) -> Vec<u8> {
        compact_record_with_flags(sequence, game_hour_raw, domain_id, record_type, 0, payload)
    }

    fn compact_record_with_flags(
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&record_flags.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn empty_pdu1_perf_payload() -> Vec<u8> {
        let mut payload = vec![0; 40];
        payload.extend_from_slice(b"PDU1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }

    /// One d28 DATA payload with one row, for the (kind x status-word) sweep
    /// below. Everything the row does not need is zero.
    fn sd2_one_row_oob_payload(kind: u32, status_offset: usize, status_value: u32) -> Vec<u8> {
        let mut payload = vec![0u8; 48 + COMPACT_OOB_ROW_LEN];
        payload[8..12].copy_from_slice(&1u32.to_le_bytes());
        payload[12..16].copy_from_slice(&1u32.to_le_bytes());
        let row = 48usize;
        payload[row..row + 4].copy_from_slice(&kind.to_le_bytes());
        payload[row + 4..row + 8].copy_from_slice(&17u32.to_le_bytes());
        let at = row + status_offset;
        payload[at..at + 4].copy_from_slice(&status_value.to_le_bytes());
        payload
    }

    /// `SECOND_OBSERVATION_DISAGREED` is legal on an ARMY row's `member_status`
    /// and nowhere else, in THIS decoder.
    ///
    /// `wire_format::d28` enforces the same rule, and its own sweep covers only
    /// itself: reverting this decoder left that test green (Codex round 3,
    /// finding 4). Two decoders, two sweeps.
    ///
    /// kills: raise the ceiling for a status word other than `member_status`, or
    /// for a kind other than ARMY, and the corresponding arm fails; lower it for
    /// an army's `member_status` and the accepted arm fails.
    #[test]
    fn code_six_is_legal_only_on_an_army_rows_member_status() {
        const SECOND_OBSERVATION_DISAGREED: u32 = 6;
        for (status_name, status_offset) in [
            ("parent_status", 52usize),
            ("member_status", 56),
            ("owner_status", 60),
            ("leader_status", 64),
            ("leader_trait_status", 68),
        ] {
            for kind in [
                COMPACT_OOB_KIND_ARMY,
                COMPACT_OOB_KIND_ARMY_GROUP,
                COMPACT_OOB_KIND_FLEET,
                COMPACT_OOB_KIND_TASK_FORCE,
            ] {
                let payload =
                    sd2_one_row_oob_payload(kind, status_offset, SECOND_OBSERVATION_DISAGREED);
                let mut summary = NavalOobGateSummary::default();
                summary
                    .per_domain
                    .entry(DOMAIN_OOB_COMMAND)
                    .or_default()
                    .data_record_count = 1;
                decode_oob_command_data_payload(0, &payload, &mut summary);
                let complained = summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(&format!("invalid {status_name} 6")));
                let allowed = kind == COMPACT_OOB_KIND_ARMY && status_offset == 56;
                assert_eq!(
                    complained, !allowed,
                    "kind {kind}, {status_name}: allowed={allowed}, errors={:?}",
                    summary.parse_errors
                );
            }
        }
        // And the control: 5 is legal everywhere, so the sweep above is about
        // the SIXTH code and not about the sweep's own plumbing.
        for (_, status_offset) in [("parent_status", 52usize), ("member_status", 56)] {
            let payload = sd2_one_row_oob_payload(COMPACT_OOB_KIND_FLEET, status_offset, 5);
            let mut summary = NavalOobGateSummary::default();
            summary
                .per_domain
                .entry(DOMAIN_OOB_COMMAND)
                .or_default()
                .data_record_count = 1;
            decode_oob_command_data_payload(0, &payload, &mut summary);
            assert!(
                !summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains("status 5")),
                "{:?}",
                summary.parse_errors
            );
        }
    }

    fn test_oob_command_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&22u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&6u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());

        for (
            kind,
            object_id,
            flags,
            parent_id,
            owner_tag_db_id,
            leader_id,
            leader_skill,
            member_first,
            row_member_count,
            logical_member_count,
            member_count_engine,
            missing_member_count,
            leader_trait_count,
            location_province_id,
            parent_status,
            member_status,
            owner_status,
            leader_status,
            leader_trait_status,
        ) in [
            (
                COMPACT_OOB_KIND_ARMY,
                17u32,
                COMPACT_OOB_ROW_FLAG_PARENT
                    | COMPACT_OOB_ROW_FLAG_OWNER
                    | COMPACT_OOB_ROW_FLAG_LEADER
                    | COMPACT_OOB_ROW_FLAG_LEADER_SKILL
                    | COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE
                    | COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT,
                301u32,
                47u32,
                9001u32,
                4u32,
                0u32,
                2u32,
                2u32,
                3u32,
                1u32,
                3u32,
                0u32,
                1u32,
                1u32,
                1u32,
                1u32,
                1u32,
            ),
            (
                COMPACT_OOB_KIND_ARMY_GROUP,
                301u32,
                COMPACT_OOB_ROW_FLAG_OWNER,
                0u32,
                47u32,
                0u32,
                0u32,
                2u32,
                1u32,
                1u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                1u32,
                1u32,
                0u32,
                0u32,
            ),
            (
                COMPACT_OOB_KIND_FLEET,
                615u32,
                COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE,
                0u32,
                0u32,
                0u32,
                0u32,
                3u32,
                1u32,
                1u32,
                1u32,
                0u32,
                0u32,
                0u32,
                0u32,
                1u32,
                2u32,
                0u32,
                0u32,
            ),
            (
                COMPACT_OOB_KIND_TASK_FORCE,
                617u32,
                COMPACT_OOB_ROW_FLAG_PARENT
                    | COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE
                    | COMPACT_OOB_ROW_FLAG_LOCATION,
                615u32,
                0u32,
                0u32,
                0u32,
                4u32,
                2u32,
                2u32,
                2u32,
                0u32,
                0u32,
                12389u32,
                1u32,
                1u32,
                2u32,
                0u32,
                0u32,
            ),
        ] {
            payload.extend_from_slice(&kind.to_le_bytes());
            payload.extend_from_slice(&object_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&parent_id.to_le_bytes());
            payload.extend_from_slice(&owner_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&leader_id.to_le_bytes());
            payload.extend_from_slice(&leader_skill.to_le_bytes());
            payload.extend_from_slice(&member_first.to_le_bytes());
            payload.extend_from_slice(&row_member_count.to_le_bytes());
            payload.extend_from_slice(&logical_member_count.to_le_bytes());
            payload.extend_from_slice(&member_count_engine.to_le_bytes());
            payload.extend_from_slice(&missing_member_count.to_le_bytes());
            payload.extend_from_slice(&leader_trait_count.to_le_bytes());
            payload.extend_from_slice(&parent_status.to_le_bytes());
            payload.extend_from_slice(&member_status.to_le_bytes());
            payload.extend_from_slice(&owner_status.to_le_bytes());
            payload.extend_from_slice(&leader_status.to_le_bytes());
            payload.extend_from_slice(&leader_trait_status.to_le_bytes());
            payload.extend_from_slice(&location_province_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        for member_id in [101u32, 102u32, 17u32, 617u32, 1001u32, 1002u32] {
            payload.extend_from_slice(&member_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_ships_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        for (
            instance_id,
            flags,
            task_force_id,
            owner_tag_db_id,
            category_enum,
            strength_raw,
            organisation_raw,
            experience_raw,
            sunk_convoys,
            definition,
        ) in [
            (
                1001u32,
                COMPACT_SHIP_ROW_FLAG_TASK_FORCE
                    | COMPACT_SHIP_ROW_FLAG_DEFINITION
                    | COMPACT_SHIP_ROW_FLAG_OWNER,
                617u32,
                47u32,
                3u32,
                10_000_000u32,
                9_000_000u32,
                500_000u32,
                2u32,
                "hull",
            ),
            (
                1002u32,
                COMPACT_SHIP_ROW_FLAG_DELETED,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                "",
            ),
        ] {
            payload.extend_from_slice(&instance_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&task_force_id.to_le_bytes());
            payload.extend_from_slice(&category_enum.to_le_bytes());
            payload.extend_from_slice(&strength_raw.to_le_bytes());
            payload.extend_from_slice(&organisation_raw.to_le_bytes());
            payload.extend_from_slice(&experience_raw.to_le_bytes());
            payload.extend_from_slice(&sunk_convoys.to_le_bytes());
            payload.extend_from_slice(&owner_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            append_compact_string(&mut payload, definition);
        }
        payload
    }

    fn append_compact_string(payload: &mut Vec<u8>, value: &str) {
        payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
        payload.extend_from_slice(value.as_bytes());
    }
}
