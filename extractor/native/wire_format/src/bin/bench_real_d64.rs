//! Real-capture measurement for d64's §9 DEL2 lifecycle event part.
//!
//! The input is scanned sequentially because `deepwar_a/collector_stream.bin`
//! is 3.6 GB. Before any artifact is created, every H4CR header goes through
//! `compact_contract::parse_compact_record_header`, every record goes through
//! `StatefulStreamValidator`, and this bin enforces the documented
//! `recorder_host::stream_fold::BoundaryTracker`/`eof_check` subset used by the
//! recipe-conformant bins (`stream_fold.rs:277`-`:459`): DATA/PERF hour binding,
//! unique PERF domains, FRAME_END seal and observed-domain counts, strictly
//! increasing closed hours, a final FRAME_END, and no open hour at EOF.
//!
//! Production's status-independent d64 payload authority
//! (`domain_data_authority.rs:12083`-`:12298`) runs on every d64 DATA payload,
//! failure status included, through `d64::check_authority_payload`. The fold then
//! runs through `d64::fold_payload`; this bin never re-derives DEL2's walk.
//!
//! # Three outcomes
//!
//! - **stream rejection:** H4CR/boundary failure or unassigned status;
//! - **excluded hour (event part):** d64 payload-authority rejection (production
//!   skips only that domain record at `stream_fold.rs:1058`-`:1093`), incomplete
//!   DEL2, epoch/sequence discontinuity, sticky production history poison, or a
//!   missing drain whose table path failed and therefore retained queued events
//!   (`world_fold.rs:9634`-`:9683`);
//! - **measured hour:** production publishes a complete, continuous lifecycle
//!   ledger, including beside a table-row refusal: the generic fold error is
//!   only noted (`world_fold.rs:5026`) and the independent report gate remains
//!   live (`:6055`-`:6065`). A measured zero-event hour emits no `EVENTS` frame
//!   because §9 forbids empty batches.
//!
//! The table status is independent: a failure excludes the stopped state half
//! but does not exclude a valid event trailer (`:9530`-`:9558`).
//!
//! Outputs:
//! - `ship_d64.bin`: exact d64 H4CR DATA records;
//! - `ship_d64_lifecycle.bin`: size-accounting cut arm containing only each
//!   record's DEL2 suffix (not a valid H4CR stream);
//! - `ship_d64_measured.bin`: exact d64 H4CR DATA records restricted to the
//!   completeness-admitted hours;
//! - `ship_d64_lifecycle_measured.bin`: DEL2-only cut restricted to those same
//!   completeness-admitted hours;
//! - `wire_new.bin`: canonical 48-hour wire blocks containing `EVENTS` frames;
//! - matching `.offsets`, one boundary per captured d64 hour.
//!
//! A forward observation-hour gap is rejected before 48-hour grouping. Grouping
//! by vector position is valid only after that consecutiveness proof.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, Read, Write};

use compact_contract::calendar::hour_index_from_raw;
use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance,
    compact_status_for_global_interpretation, compact_status_unknown_flags_v1,
    parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::d64::{self, LedgerPart, LifecyclePayload, TablePart};
use wire_format::frame::{read_block, BlockHeader, BlockWriter, Frame};
use wire_format::{
    check_frontier, decode_event_body, encode_event_body, FrontierOutcome,
};

const D64: u16 = 64;
const BLOCK_HOURS: usize = 48;
const FRAME_END_KNOWN_STATUS_MASK: u32 = (1 << 14) - 1;
const PROBE_DOMAIN_ID: u16 = 250;

#[derive(Debug)]
enum HourLedger {
    Published(LifecyclePayload),
    Unknown,
    NoObservation,
}

#[derive(Debug)]
struct D64Hour {
    game_hour_raw: i32,
    ship_record: Vec<u8>,
    trailer: Vec<u8>,
    ledger: HourLedger,
}

#[derive(Debug)]
struct StreamScan {
    hours: Vec<D64Hour>,
    records_total: usize,
    closed_hours: usize,
    table_failure_hours: usize,
    table_rows_total: usize,
    status_counts: BTreeMap<u32, usize>,
    complete_hours: usize,
    complete_prefix_hours: usize,
    complete_after_incomplete: usize,
    incomplete_hours: usize,
    total_trailer_events: usize,
    complete_trailer_events: usize,
    published_events: usize,
    tick_mismatch_events: usize,
    empty_trailers: usize,
    complete_empty_trailers: usize,
    incomplete_empty_trailers: usize,
    reason_counts: [usize; 4],
    occurrence_hours: BTreeSet<i32>,
    due_before_min: Option<u32>,
    due_before_max: Option<u32>,
    first_sequence: Option<u64>,
    last_sequence: Option<u64>,
    sequence_breaks: usize,
    hook_epochs: BTreeSet<u64>,
    published_hours: usize,
    unknown_hours: usize,
    no_observation_hours: usize,
    authority_rejected_hours: usize,
    preledger_fold_refused_hours: usize,
    table_refused_hours: usize,
}

fn classify_production_fold(
    production_table_failure: bool,
    outcome: d64::FoldOutcome,
    parsed_trailer: Option<&LifecyclePayload>,
) -> Result<(bool, HourLedger), String> {
    let table_refused = match &outcome.table {
        TablePart::StoppedUnavailable if production_table_failure => false,
        TablePart::StoppedPresent { .. } if !production_table_failure => false,
        TablePart::Refused { .. } if !production_table_failure => true,
        _ => return Err("d64 table status classification disagrees".into()),
    };
    let ledger = match outcome.ledger {
        LedgerPart::Published(published) => {
            if Some(&published) != parsed_trailer {
                return Err("d64 fold published a ledger other than the parsed trailer".into());
            }
            HourLedger::Published(published)
        }
        LedgerPart::Unknown => HourLedger::Unknown,
        LedgerPart::NoObservation => HourLedger::NoObservation,
    };
    Ok((table_refused, ledger))
}

fn check_frame_end_seal(payload: &[u8], at_byte: u64) -> Result<(u32, u32), String> {
    if payload.len() != COMPACT_FRAME_END_PAYLOAD_LEN {
        return Err(format!(
            "FRAME_END at byte {at_byte}: payload length {} != {COMPACT_FRAME_END_PAYLOAD_LEN}",
            payload.len()
        ));
    }
    let emitted = u32::from_le_bytes(payload[16..20].try_into().expect("checked length"));
    let scheduled = u32::from_le_bytes(payload[20..24].try_into().expect("checked length"));
    let status = u32::from_le_bytes(payload[24..28].try_into().expect("checked length"));
    let reserved = u32::from_le_bytes(payload[28..32].try_into().expect("checked length"));
    if reserved != 0 {
        return Err(format!("FRAME_END at byte {at_byte}: reserved != 0"));
    }
    if emitted != scheduled {
        return Err(format!(
            "FRAME_END at byte {at_byte}: emitted {emitted} != scheduled {scheduled}"
        ));
    }
    let unknown = status & !FRAME_END_KNOWN_STATUS_MASK;
    if unknown != 0 {
        return Err(format!(
            "FRAME_END at byte {at_byte}: unknown status bits {unknown:#x}"
        ));
    }
    let fatal = status & COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        return Err(format!(
            "FRAME_END at byte {at_byte}: fatal status bits {fatal:#x}"
        ));
    }
    Ok((emitted, scheduled))
}

fn require_consecutive_d64_tick(previous: Option<i64>, current: i64) -> Result<(), String> {
    let Some(previous) = previous else {
        return Ok(());
    };
    let expected = previous
        .checked_add(1)
        .ok_or_else(|| "d64 campaign-hour sequence overflow".to_string())?;
    if current != expected {
        return Err(format!(
            "d64 campaign hour {current} is not consecutive after {previous} (expected {expected})"
        ));
    }
    Ok(())
}

fn d64_unknown_status(payload: &[u8]) -> Option<u32> {
    if payload.len() < d64::BASE_HEADER_LEN {
        return None;
    }
    let raw_status =
        u32::from_le_bytes([payload[16], payload[17], payload[18], payload[19]]);
    let global_status = compact_status_for_global_interpretation(D64, raw_status);
    Some(compact_status_unknown_flags_v1(global_status)).filter(|unknown| *unknown != 0)
}

fn assert_reachable_status_contract() {
    // The DEL2 constants live in `recorder_host`, which this bin cannot depend
    // on; the per-real-record stride check in `scan_stream` is the explicitly
    // weaker stand-in. The status predicate is reachable through the leaf
    // `compact_contract` dependency and is machine-checked here.
    // d64 has no lane-local mask and no expected-partial tolerance. Prove the
    // library copy equals the production predicate over every assigned word.
    for status in 0..(1u32 << 14) {
        let copied = status & d64::FAILURE_STATUS_MASK;
        let shared = compact_status_failure_flags_after_expected_partial_tolerance(D64, status);
        assert_eq!(copied, shared, "status {status:#x}");
    }
}

fn scan_stream(path: &str, identity_mode: StreamIdentityMode) -> Result<StreamScan, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("open {path}: {error}"))?;
    let mut reader = BufReader::with_capacity(8 * 1024 * 1024, file);
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours = Vec::new();
    let mut fold_state = d64::State::new();
    let mut open_hour: Option<i32> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut non_probe_perf_domains = BTreeSet::new();
    let mut last_d64_tick: Option<i64> = None;
    let mut d64_records = 0usize;
    let mut records_total = 0usize;
    let mut closed_hours = 0usize;
    let mut byte_offset = 0u64;

    let mut table_failure_hours = 0usize;
    let mut table_rows_total = 0usize;
    let mut status_counts = BTreeMap::new();
    let mut complete_hours = 0usize;
    let mut complete_prefix_hours = 0usize;
    let mut complete_after_incomplete = 0usize;
    let mut saw_incomplete = false;
    let mut incomplete_hours = 0usize;
    let mut total_trailer_events = 0usize;
    let mut complete_trailer_events = 0usize;
    let mut published_events = 0usize;
    let mut tick_mismatch_events = 0usize;
    let mut empty_trailers = 0usize;
    let mut complete_empty_trailers = 0usize;
    let mut incomplete_empty_trailers = 0usize;
    let mut reason_counts = [0usize; 4];
    let mut occurrence_hours = BTreeSet::new();
    let mut due_before_min = None::<u32>;
    let mut due_before_max = None::<u32>;
    let mut first_sequence = None::<u64>;
    let mut last_sequence = None::<u64>;
    let mut sequence_breaks = 0usize;
    let mut hook_epochs = BTreeSet::new();
    let mut published_hours = 0usize;
    let mut unknown_hours = 0usize;
    let mut no_observation_hours = 0usize;
    let mut authority_rejected_hours = 0usize;
    let mut preledger_fold_refused_hours = 0usize;
    let mut table_refused_hours = 0usize;

    loop {
        let record_offset = byte_offset;
        let mut header_bytes = [0u8; COMPACT_RECORD_HEADER_LEN];
        match reader.read(&mut header_bytes[..1]) {
            Ok(0) => break,
            Ok(1) => {}
            Ok(_) => unreachable!("one-byte read"),
            Err(error) => return Err(format!("read header at byte {record_offset}: {error}")),
        }
        reader
            .read_exact(&mut header_bytes[1..])
            .map_err(|error| format!("truncated header at byte {record_offset}: {error}"))?;
        byte_offset = byte_offset
            .checked_add(COMPACT_RECORD_HEADER_LEN as u64)
            .ok_or_else(|| "stream byte offset overflow".to_string())?;
        let header = parse_compact_record_header(&header_bytes)
            .map_err(|error| format!("record at byte {record_offset}: {error}"))?;

        let mut payload = Vec::new();
        payload
            .try_reserve_exact(header.payload_len)
            .map_err(|_| format!("record at byte {record_offset}: payload allocation refused"))?;
        payload.resize(header.payload_len, 0);
        reader.read_exact(&mut payload).map_err(|error| {
            format!("record at byte {record_offset}: truncated payload: {error}")
        })?;
        byte_offset = byte_offset
            .checked_add(header.payload_len as u64)
            .ok_or_else(|| "stream byte offset overflow".to_string())?;

        validator
            .validate_record(&header, &payload)
            .map_err(|error| format!("record at byte {record_offset}: {error:?}"))?;
        records_total += 1;

        match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                if header.domain_id != PROBE_DOMAIN_ID
                    && !non_probe_perf_domains.insert(header.domain_id)
                {
                    return Err(format!(
                        "duplicate DOMAIN_PERF domain {} in hour {} at byte {record_offset}",
                        header.domain_id, header.game_hour_raw
                    ));
                }
                match open_hour {
                    Some(open) if open != header.game_hour_raw => {
                        return Err(format!(
                            "PERF hour {} interleaves open hour {open} at byte {record_offset}",
                            header.game_hour_raw
                        ))
                    }
                    Some(_) => {}
                    None => {
                        if last_closed_hour.is_some_and(|closed| header.game_hour_raw <= closed) {
                            return Err(format!(
                                "PERF reopens closed hour {} at byte {record_offset}",
                                header.game_hour_raw
                            ));
                        }
                        match pending_perf_hour {
                            Some(pending) if pending != header.game_hour_raw => {
                                return Err(format!(
                                    "PERF hour conflict {pending} vs {} at byte {record_offset}",
                                    header.game_hour_raw
                                ))
                            }
                            Some(_) => {}
                            None => pending_perf_hour = Some(header.game_hour_raw),
                        }
                    }
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA => match open_hour {
                Some(open) if open != header.game_hour_raw => {
                    return Err(format!(
                        "DATA hour {} interleaves open hour {open} at byte {record_offset}",
                        header.game_hour_raw
                    ))
                }
                Some(_) => {}
                None => {
                    if pending_perf_hour.is_some_and(|pending| pending != header.game_hour_raw) {
                        return Err(format!(
                            "DATA hour {} conflicts with pending PERF hour at byte {record_offset}",
                            header.game_hour_raw
                        ));
                    }
                    if last_closed_hour.is_some_and(|closed| header.game_hour_raw <= closed) {
                        return Err(format!(
                            "DATA reopens hour {} at byte {record_offset}",
                            header.game_hour_raw
                        ));
                    }
                    open_hour = Some(header.game_hour_raw);
                }
            },
            COMPACT_RECORD_TYPE_FRAME_END => {
                let (emitted, scheduled) = check_frame_end_seal(&payload, record_offset)?;
                let observed = non_probe_perf_domains.len() as u32;
                if observed != emitted || observed != scheduled {
                    return Err(format!(
                        "FRAME_END at byte {record_offset}: seal claims {emitted}/{scheduled}, \
                         observed {observed} PERF domains"
                    ));
                }
                if open_hour != Some(header.game_hour_raw) {
                    return Err(format!(
                        "FRAME_END at byte {record_offset} does not close its open hour"
                    ));
                }
                if last_closed_hour.is_some_and(|closed| header.game_hour_raw <= closed) {
                    return Err(format!(
                        "non-monotonic FRAME_END hour {} at byte {record_offset}",
                        header.game_hour_raw
                    ));
                }
                open_hour = None;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
                last_closed_hour = Some(header.game_hour_raw);
                closed_hours += 1;
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D64 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d64_records += 1;
            let d64_tick = hour_index_from_raw(header.game_hour_raw).ok_or_else(|| {
                format!(
                    "d64 game hour {} cannot map to a campaign hour at byte {record_offset}",
                    header.game_hour_raw
                )
            })?;
            require_consecutive_d64_tick(last_d64_tick, d64_tick)
                .map_err(|error| format!("{error} at byte {record_offset}"))?;
            last_d64_tick = Some(d64_tick);

            let mut ship_record = Vec::new();
            ship_record.extend_from_slice(&header_bytes);
            ship_record.extend_from_slice(&payload);

            // Status identity is a stream-level contract. Check it before the
            // domain-local authority path so a payload carrying both an
            // unassigned status and malformed leaf bytes cannot downgrade the
            // former into an excluded hour.
            if let Some(unknown_status) = d64_unknown_status(&payload) {
                return Err(format!(
                    "d64 payload at byte {record_offset}: unassigned status \
                     {unknown_status:#x}"
                ));
            }

            if d64::check_authority_payload(&payload).is_err() {
                // `stream_fold.rs:1058`-`:1093` treats exact payload rejection as
                // domain-local: it records the malformed d64 leaf and continues
                // without invoking the fold. No bytes from that payload are
                // admitted to the event arm.
                authority_rejected_hours += 1;
                unknown_hours += 1;
                hours.push(D64Hour {
                    game_hour_raw: header.game_hour_raw,
                    ship_record,
                    trailer: Vec::new(),
                    ledger: HourLedger::Unknown,
                });
                continue;
            }

            let parts = match d64::split_payload(&payload) {
                Ok(parts) => parts,
                Err(_error) => {
                    // The authority deliberately permits `observed_rows <
                    // row_count`, while the production fold refuses it before
                    // reaching the trailer (`world_fold.rs:9500`-`:9510`).
                    // `apply_domain_data` records the generic fold error at
                    // `:5026` and continues the stream. No ledger was settled.
                    preledger_fold_refused_hours += 1;
                    unknown_hours += 1;
                    hours.push(D64Hour {
                        game_hour_raw: header.game_hour_raw,
                        ship_record,
                        trailer: Vec::new(),
                        ledger: HourLedger::Unknown,
                    });
                    continue;
                }
            };
            let global_status =
                compact_status_for_global_interpretation(D64, parts.header.status_flags);
            let unknown_status = compact_status_unknown_flags_v1(global_status);
            if unknown_status != 0 {
                return Err(format!(
                    "d64 payload at byte {record_offset}: unassigned status {unknown_status:#x}"
                ));
            }
            let production_table_failure =
                compact_status_failure_flags_after_expected_partial_tolerance(
                    D64,
                    parts.header.status_flags,
                ) != 0;
            *status_counts
                .entry(parts.header.status_flags)
                .or_insert(0) += 1;
            table_rows_total = table_rows_total
                .checked_add(parts.header.row_count as usize)
                .ok_or_else(|| "d64 table row count overflow".to_string())?;
            if production_table_failure {
                table_failure_hours += 1;
            }

            let parsed_trailer = parts
                .trailer
                .map(d64::parse_trailer)
                .transpose()
                .map_err(|error| format!("d64 trailer at byte {record_offset}: {error}"))?;
            let trailer_bytes = parts.trailer.unwrap_or(&[]).to_vec();
            if let Some(trailer) = &parsed_trailer {
                // Empirical stride stand-in: every real record must reproduce
                // its exact trailer length from the transcribed constants.
                let expected = d64::TRAILER_HEADER_LEN
                    .checked_add(
                        trailer
                            .events
                            .len()
                            .checked_mul(d64::TRAILER_ROW_LEN)
                            .ok_or_else(|| "d64 trailer stride overflow".to_string())?,
                    )
                    .ok_or_else(|| "d64 trailer stride overflow".to_string())?;
                if expected != trailer_bytes.len() {
                    return Err(format!(
                        "d64 trailer at byte {record_offset}: stride says {expected}, \
                         actual {}",
                        trailer_bytes.len()
                    ));
                }
                hook_epochs.insert(trailer.hook_epoch);
                total_trailer_events += trailer.events.len();
                if trailer.complete {
                    complete_hours += 1;
                    complete_trailer_events += trailer.events.len();
                    if saw_incomplete {
                        complete_after_incomplete += 1;
                    } else {
                        complete_prefix_hours += 1;
                    }
                } else {
                    incomplete_hours += 1;
                    saw_incomplete = true;
                }
                if trailer.events.is_empty() {
                    empty_trailers += 1;
                    if trailer.complete {
                        complete_empty_trailers += 1;
                    } else {
                        incomplete_empty_trailers += 1;
                    }
                }
                tick_mismatch_events += trailer
                    .events
                    .iter()
                    .filter(|event| event.game_hour_raw != header.game_hour_raw)
                    .count();
                for event in &trailer.events {
                    reason_counts[event.reason.enum_code() as usize] += 1;
                    occurrence_hours.insert(event.game_hour_raw);
                    due_before_min = Some(
                        due_before_min
                            .map_or(event.due_before, |value| value.min(event.due_before)),
                    );
                    due_before_max = Some(
                        due_before_max
                            .map_or(event.due_before, |value| value.max(event.due_before)),
                    );
                }
                if !trailer.events.is_empty() {
                    if let Some(previous) = last_sequence {
                        if previous.checked_add(1) != Some(trailer.first_sequence) {
                            sequence_breaks += 1;
                        }
                    } else {
                        first_sequence = Some(trailer.first_sequence);
                    }
                    last_sequence = Some(trailer.last_sequence);
                }
            }

            let outcome = d64::fold_payload(&mut fold_state, &payload)
                .map_err(|error| format!("d64 fold at byte {record_offset}: {error}"))?;
            let (table_refused, ledger) =
                classify_production_fold(production_table_failure, outcome, parsed_trailer.as_ref())
                    .map_err(|error| format!("{error} at byte {record_offset}"))?;
            if table_refused {
                // Production's generic fold-Err arm only records a note
                // (`world_fold.rs:5026`). The DEL2 report has no
                // fold-domain-unknown gate (`:6055`-`:6065`).
                table_refused_hours += 1;
            }
            let ledger = match ledger {
                HourLedger::Published(published) => {
                    published_hours += 1;
                    published_events += published.events.len();
                    HourLedger::Published(published)
                }
                HourLedger::Unknown => {
                    unknown_hours += 1;
                    HourLedger::Unknown
                }
                HourLedger::NoObservation => {
                    no_observation_hours += 1;
                    HourLedger::NoObservation
                }
            };

            hours.push(D64Hour {
                game_hour_raw: header.game_hour_raw,
                ship_record,
                trailer: trailer_bytes,
                ledger,
            });
        }
    }

    if open_hour.is_some() {
        return Err("stream ends with an open game hour".into());
    }
    if last_closed_hour.is_none() {
        return Err("stream contains no FRAME_END".into());
    }
    if last_record_type != Some(COMPACT_RECORD_TYPE_FRAME_END) {
        return Err("stream final record is not FRAME_END".into());
    }
    if d64_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} hours but carries {d64_records} d64 DATA records"
        ));
    }
    if hours.is_empty() {
        return Err("stream contains no d64 DATA records".into());
    }

    Ok(StreamScan {
        hours,
        records_total,
        closed_hours,
        table_failure_hours,
        table_rows_total,
        status_counts,
        complete_hours,
        complete_prefix_hours,
        complete_after_incomplete,
        incomplete_hours,
        total_trailer_events,
        complete_trailer_events,
        published_events,
        tick_mismatch_events,
        empty_trailers,
        complete_empty_trailers,
        incomplete_empty_trailers,
        reason_counts,
        occurrence_hours,
        due_before_min,
        due_before_max,
        first_sequence,
        last_sequence,
        sequence_breaks,
        hook_epochs,
        published_hours,
        unknown_hours,
        no_observation_hours,
        authority_rejected_hours,
        preledger_fold_refused_hours,
        table_refused_hours,
    })
}

struct WireArm {
    bytes: Vec<u8>,
    offsets: Vec<usize>,
    frames: usize,
    blocks: usize,
    body_bytes: usize,
    replayed_hours: usize,
    replayed_events: usize,
    gap_hours: usize,
    no_observation_hours: usize,
}

struct ShippingArms {
    all: Vec<u8>,
    all_offsets: Vec<usize>,
    lifecycle_all: Vec<u8>,
    lifecycle_all_offsets: Vec<usize>,
    measured: Vec<u8>,
    measured_offsets: Vec<usize>,
    lifecycle_measured: Vec<u8>,
    lifecycle_measured_offsets: Vec<usize>,
}

fn build_shipping_arms(hours: &[D64Hour]) -> ShippingArms {
    let mut arms = ShippingArms {
        all: Vec::new(),
        all_offsets: vec![0],
        lifecycle_all: Vec::new(),
        lifecycle_all_offsets: vec![0],
        measured: Vec::new(),
        measured_offsets: vec![0],
        lifecycle_measured: Vec::new(),
        lifecycle_measured_offsets: vec![0],
    };
    for hour in hours {
        arms.all.extend_from_slice(&hour.ship_record);
        arms.all_offsets.push(arms.all.len());
        arms.lifecycle_all.extend_from_slice(&hour.trailer);
        arms.lifecycle_all_offsets.push(arms.lifecycle_all.len());
        if matches!(&hour.ledger, HourLedger::Published(_)) {
            arms.measured.extend_from_slice(&hour.ship_record);
            arms.lifecycle_measured.extend_from_slice(&hour.trailer);
        }
        // Preserve all capture-hour boundaries in the restricted arms. The
        // excluded hours contribute zero bytes, so 48-hour spans align exactly
        // with `wire_new` while both sides carry only the 175 measured hours.
        arms.measured_offsets.push(arms.measured.len());
        arms.lifecycle_measured_offsets
            .push(arms.lifecycle_measured.len());
    }
    arms
}

fn build_wire_arm(scan: &StreamScan) -> Result<WireArm, String> {
    let schema = d64::schema();
    let part = schema.event_part.as_ref().expect("d64 event part");
    let mut prepared: Vec<Option<(u64, Vec<u8>)>> = Vec::new();
    let mut previous_frontier: Option<Vec<u64>> = None;
    let mut body_bytes = 0usize;
    let mut replayed_hours = 0usize;
    let mut replayed_events = 0usize;
    let mut gap_hours = 0usize;
    let mut no_observation_hours = 0usize;

    for hour in &scan.hours {
        match &hour.ledger {
            HourLedger::Published(published) => {
                replayed_hours += 1;
                if published.events.is_empty() {
                    // §9 forbids empty frames. The per-hour reader ledger is an
                    // empty vector, exactly what the fold published.
                    let replayed: Vec<d64::LifecycleRow> = Vec::new();
                    if replayed != published.events {
                        return Err(format!(
                            "hour {}: empty reader ledger differs from fold",
                            hour.game_hour_raw
                        ));
                    }
                    prepared.push(None);
                    continue;
                }
                let batch = d64::event_batch_from_lifecycle_payload(part, published, 0)?;
                match check_frontier(
                    part,
                    previous_frontier.as_deref(),
                    &batch.frontier,
                    batch.records.len(),
                )
                .map_err(|error| format!("frontier check failed: {error:?}"))?
                {
                    FrontierOutcome::Established | FrontierOutcome::Continuous => {}
                    FrontierOutcome::Discontinuous(reason) => {
                        return Err(format!(
                            "hour {}: production-published ledger has §9 discontinuity {reason:?}",
                            hour.game_hour_raw
                        ))
                    }
                }
                let body = encode_event_body(part, &batch)
                    .map_err(|error| format!("event body encode failed: {error:?}"))?;
                let decoded = decode_event_body(part, &body)
                    .map_err(|error| format!("event body decode failed: {error:?}"))?;
                let replayed = d64::rows_from_event_batch(part, &decoded)?;
                if replayed != published.events {
                    return Err(format!(
                        "hour {}: decoded event batch differs from fold ledger",
                        hour.game_hour_raw
                    ));
                }
                previous_frontier = Some(decoded.frontier);
                body_bytes += body.len();
                replayed_events += replayed.len();
                let tick = hour_index_from_raw(hour.game_hour_raw)
                    .and_then(|hour| u64::try_from(hour).ok())
                    .ok_or_else(|| {
                        format!(
                            "hour {} cannot map to a nonnegative campaign tick",
                            hour.game_hour_raw
                        )
                    })?;
                prepared.push(Some((tick, body)));
            }
            HourLedger::Unknown => {
                // The caller turns the typed condition into an event-part gap.
                // §8's gap-stream bytes are outside this crate; clearing the
                // prior makes the next batch establish, as §9 requires.
                previous_frontier = None;
                gap_hours += 1;
                prepared.push(None);
            }
            HourLedger::NoObservation => {
                no_observation_hours += 1;
                prepared.push(None);
            }
        }
    }

    let mut bytes = Vec::new();
    let mut offsets = vec![0usize];
    let mut frames = 0usize;
    let mut blocks = 0usize;
    for chunk_start in (0..prepared.len()).step_by(BLOCK_HOURS) {
        let chunk_end = (chunk_start + BLOCK_HOURS).min(prepared.len());
        let chunk = &prepared[chunk_start..chunk_end];
        let first = chunk.iter().flatten().next().map(|(tick, _)| *tick);
        let mut encoded_block = None;
        let mut first_frame_index = None;
        if let Some(first_tick) = first {
            let expected_header = BlockHeader {
                domain: d64::DOMAIN,
                branch: 0,
                schema_hash: schema.schema_hash(),
                first_tick,
            };
            let expected_frames = chunk
                .iter()
                .enumerate()
                .filter_map(|(relative, frame)| {
                    frame.as_ref().map(|(tick, body)| {
                        first_frame_index.get_or_insert(relative);
                        Frame::events(*tick, body.clone())
                    })
                })
                .collect::<Vec<_>>();
            let mut writer = BlockWriter::new(expected_header.clone(), 1);
            for frame in &expected_frames {
                writer
                    .push(frame.clone())
                    .map_err(|error| format!("EVENTS frame rejected: {error:?}"))?;
            }
            frames += expected_frames.len();
            let block = writer
                .finish()
                .map_err(|error| format!("event block rejected: {error:?}"))?;
            let (header, decoded_frames) = read_block(&block, 1)
                .map_err(|error| format!("event block replay failed: {error:?}"))?;
            if header != expected_header || decoded_frames != expected_frames {
                return Err(
                    "event block replay changed its header, frame tick, kind, or body".into(),
                );
            }
            blocks += 1;
            encoded_block = Some(block);
        }
        for relative in 0..chunk.len() {
            if Some(relative) == first_frame_index {
                bytes.extend_from_slice(encoded_block.as_ref().expect("block exists"));
            }
            offsets.push(bytes.len());
        }
    }

    Ok(WireArm {
        bytes,
        offsets,
        frames,
        blocks,
        body_bytes,
        replayed_hours,
        replayed_events,
        gap_hours,
        no_observation_hours,
    })
}

fn write_arm(out_dir: &str, name: &str, bytes: &[u8], offsets: &[usize]) {
    let base = std::path::Path::new(out_dir);
    let mut file = std::fs::File::create(base.join(format!("{name}.bin"))).expect("create arm");
    file.write_all(bytes).expect("write arm");
    let mut file =
        std::fs::File::create(base.join(format!("{name}.offsets"))).expect("create offsets");
    for offset in offsets {
        writeln!(file, "{offset}").expect("write offset");
    }
}

fn main() {
    const USAGE: &str =
        "usage: bench_real_d64 <collector_stream.bin> <out_dir> [legacy|identity-v1]";
    assert_reachable_status_contract();
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };

    // Pass 1: validate and fold. No output directory exists before this returns.
    let scan = match scan_stream(&input, identity_mode) {
        Ok(scan) => scan,
        Err(error) => {
            eprintln!("{input}: {error}");
            std::process::exit(1);
        }
    };
    let wire = build_wire_arm(&scan).unwrap_or_else(|error| {
        eprintln!("{input}: {error}");
        std::process::exit(1);
    });

    // Pass 2: materialize already-validated arms.
    std::fs::create_dir_all(&out_dir).expect("create output directory");
    let ship = build_shipping_arms(&scan.hours);
    write_arm(&out_dir, "ship_d64", &ship.all, &ship.all_offsets);
    write_arm(
        &out_dir,
        "ship_d64_lifecycle",
        &ship.lifecycle_all,
        &ship.lifecycle_all_offsets,
    );
    write_arm(
        &out_dir,
        "ship_d64_measured",
        &ship.measured,
        &ship.measured_offsets,
    );
    write_arm(
        &out_dir,
        "ship_d64_lifecycle_measured",
        &ship.lifecycle_measured,
        &ship.lifecycle_measured_offsets,
    );
    write_arm(&out_dir, "wire_new", &wire.bytes, &wire.offsets);

    let h4cr_framing = scan.hours.len() * COMPACT_RECORD_HEADER_LEN;
    let base_headers = scan.hours.len() * d64::BASE_HEADER_LEN;
    let state_rows = ship
        .all
        .len()
        .saturating_sub(h4cr_framing + base_headers + ship.lifecycle_all.len());
    println!(
        "stream records/hours  {} records / {} closed hours",
        scan.records_total, scan.closed_hours
    );
    println!(
        "d64 table             {} failure hours / {} rows / statuses {:?}",
        scan.table_failure_hours, scan.table_rows_total, scan.status_counts
    );
    println!(
        "DEL2 complete         {} hours / {} prefix / {} after-incomplete / {} events",
        scan.complete_hours,
        scan.complete_prefix_hours,
        scan.complete_after_incomplete,
        scan.complete_trailer_events
    );
    println!(
        "DEL2 incomplete       {} hours",
        scan.incomplete_hours
    );
    println!(
        "empty trailers        {} total / {} complete / {} incomplete",
        scan.empty_trailers, scan.complete_empty_trailers, scan.incomplete_empty_trailers
    );
    println!(
        "DEL2 all              {} events, {:.3} events/hour",
        scan.total_trailer_events,
        scan.total_trailer_events as f64 / scan.hours.len() as f64
    );
    println!(
        "published by fold     {} hours / {} events",
        scan.published_hours, scan.published_events
    );
    println!(
        "event gaps/no-drain    {} / {} hours",
        scan.unknown_hours, scan.no_observation_hours
    );
    println!(
        "domain-local paths    {} authority-rejected / {} pre-ledger fold-refused / \
         {} table-only-refused hours",
        scan.authority_rejected_hours,
        scan.preledger_fold_refused_hours,
        scan.table_refused_hours
    );
    println!(
        "occurrence tick differs from observation tick: {} / {} events",
        scan.tick_mismatch_events, scan.total_trailer_events
    );
    println!(
        "hook epochs           {} {:?}",
        scan.hook_epochs.len(),
        scan.hook_epochs
    );
    match (scan.first_sequence, scan.last_sequence) {
        (Some(first), Some(last)) => println!(
            "sequence coverage     {first}..={last} / {} breaks",
            scan.sequence_breaks
        ),
        _ => println!("sequence coverage     no events / {} breaks", scan.sequence_breaks),
    }
    println!(
        "reason counts         retained_decrement={} executed={} \
         failed_trigger_consumed={} definition_cancelled={}",
        scan.reason_counts[0],
        scan.reason_counts[1],
        scan.reason_counts[2],
        scan.reason_counts[3]
    );
    match (
        scan.occurrence_hours.first(),
        scan.occurrence_hours.last(),
    ) {
        (Some(first), Some(last)) => println!(
            "occurrence hours      {} distinct / {}..={}",
            scan.occurrence_hours.len(),
            first,
            last
        ),
        _ => println!("occurrence hours      0 distinct"),
    }
    match (scan.due_before_min, scan.due_before_max) {
        (Some(min), Some(max)) => println!("due_before range      {min}..={max}"),
        _ => println!("due_before range      no events"),
    }
    println!(
        "reader replay         {} hours / {} events",
        wire.replayed_hours, wire.replayed_events
    );
    println!(
        "wire EVENTS           {} frames in {} nonempty 48-h blocks; {} body bytes",
        wire.frames, wire.blocks, wire.body_bytes
    );
    println!(
        "wire gap/no-observation hours {} / {} (§8 gap bytes pending integrated remeasurement)",
        wire.gap_hours, wire.no_observation_hours
    );
    println!("ship_d64.bin          {} bytes", ship.all.len());
    println!(
        "  H4CR framing        {} bytes; base headers {} bytes; stopped state rows {} bytes; \
         DEL2 suffix {} bytes",
        h4cr_framing,
        base_headers,
        state_rows,
        ship.lifecycle_all.len()
    );
    println!(
        "ship_d64_lifecycle    {} bytes (size-accounting cut arm, not H4CR)",
        ship.lifecycle_all.len()
    );
    println!(
        "ship_d64_measured     {} bytes / lifecycle cut {} bytes \
         (same {} hours and {} events as wire)",
        ship.measured.len(),
        ship.lifecycle_measured.len(),
        wire.replayed_hours,
        wire.replayed_events
    );
    println!(
        "wire_new.bin          {} bytes ({:.2}% full shipping; {:.2}% lifecycle cut)",
        wire.bytes.len(),
        100.0 * wire.bytes.len() as f64 / ship.all.len() as f64,
        100.0 * wire.bytes.len() as f64 / ship.lifecycle_all.len() as f64
    );
    println!(
        "like-for-like wire    {:.2}% measured shipping; {:.2}% measured lifecycle cut",
        100.0 * wire.bytes.len() as f64 / ship.measured.len() as f64,
        100.0 * wire.bytes.len() as f64 / ship.lifecycle_measured.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reachable_status_predicate_matches() {
        assert_reachable_status_contract();
    }

    #[test]
    fn unknown_status_precedes_domain_local_payload_classification() {
        let mut malformed = vec![0u8; d64::BASE_HEADER_LEN];
        malformed[8..12].copy_from_slice(&1u32.to_le_bytes());
        malformed[12..16].copy_from_slice(&1u32.to_le_bytes());
        malformed[16..20].copy_from_slice(&(1u32 << 14).to_le_bytes());
        assert_eq!(d64_unknown_status(&malformed), Some(1u32 << 14));
        assert!(
            d64::check_authority_payload(&malformed).is_err(),
            "the same payload is also malformed, so scan order is load-bearing"
        );
        assert_eq!(
            d64_unknown_status(&malformed[..d64::BASE_HEADER_LEN - 1]),
            None,
            "a truncated header carries no readable status word"
        );
    }

    #[test]
    fn forty_eight_hour_grouping_requires_consecutive_campaign_hours() {
        assert_eq!(require_consecutive_d64_tick(None, 100), Ok(()));
        assert_eq!(require_consecutive_d64_tick(Some(100), 101), Ok(()));
        assert_eq!(
            require_consecutive_d64_tick(Some(100), 102),
            Err("d64 campaign hour 102 is not consecutive after 100 (expected 101)".into())
        );
        assert_eq!(
            require_consecutive_d64_tick(Some(i64::MAX), i64::MAX),
            Err("d64 campaign-hour sequence overflow".into())
        );
    }

    #[test]
    fn frame_end_seal_is_total_and_rejects_each_authoritative_word() {
        for len in 0..COMPACT_FRAME_END_PAYLOAD_LEN {
            assert!(check_frame_end_seal(&vec![0; len], 0).is_err());
        }
        let mut good = vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        good[16..20].copy_from_slice(&2u32.to_le_bytes());
        good[20..24].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(check_frame_end_seal(&good, 0), Ok((2, 2)));
        for (offset, value) in [(20usize, 3u32), (24, 1), (28, 1)] {
            let mut bad = good.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(check_frame_end_seal(&bad, 0).is_err(), "offset {offset}");
        }
    }

    #[test]
    fn empty_wire_blocks_are_not_fabricated() {
        let scan = StreamScan {
            hours: (0..49)
                .map(|hour| D64Hour {
                    game_hour_raw: 43_799_999 + hour,
                    ship_record: Vec::new(),
                    trailer: Vec::new(),
                    ledger: HourLedger::Unknown,
                })
                .collect(),
            records_total: 49,
            closed_hours: 49,
            table_failure_hours: 49,
            table_rows_total: 0,
            status_counts: BTreeMap::from([(0x100, 49)]),
            complete_hours: 0,
            complete_prefix_hours: 0,
            complete_after_incomplete: 0,
            incomplete_hours: 49,
            total_trailer_events: 0,
            complete_trailer_events: 0,
            published_events: 0,
            tick_mismatch_events: 0,
            empty_trailers: 49,
            complete_empty_trailers: 0,
            incomplete_empty_trailers: 49,
            reason_counts: [0; 4],
            occurrence_hours: BTreeSet::new(),
            due_before_min: None,
            due_before_max: None,
            first_sequence: None,
            last_sequence: None,
            sequence_breaks: 0,
            hook_epochs: BTreeSet::new(),
            published_hours: 0,
            unknown_hours: 49,
            no_observation_hours: 0,
            authority_rejected_hours: 0,
            preledger_fold_refused_hours: 0,
            table_refused_hours: 0,
        };
        let wire = build_wire_arm(&scan).unwrap();
        assert!(wire.bytes.is_empty());
        assert_eq!(wire.offsets, vec![0; 50]);
        assert_eq!(wire.blocks, 0);
    }

    #[test]
    fn measured_shipping_arms_keep_only_published_hours_and_all_hour_boundaries() {
        let published = || {
            HourLedger::Published(LifecyclePayload {
                complete: true,
                hook_epoch: 1,
                first_sequence: 0,
                last_sequence: 0,
                events: Vec::new(),
            })
        };
        let hours = vec![
            D64Hour {
                game_hour_raw: 1,
                ship_record: vec![1, 2],
                trailer: vec![11],
                ledger: published(),
            },
            D64Hour {
                game_hour_raw: 2,
                ship_record: vec![3, 4, 5],
                trailer: vec![12, 13],
                ledger: HourLedger::Unknown,
            },
            D64Hour {
                game_hour_raw: 3,
                ship_record: vec![6],
                trailer: vec![14, 15, 16],
                ledger: published(),
            },
        ];
        let arms = build_shipping_arms(&hours);
        assert_eq!(arms.all, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(arms.all_offsets, vec![0, 2, 5, 6]);
        assert_eq!(arms.lifecycle_all, vec![11, 12, 13, 14, 15, 16]);
        assert_eq!(arms.lifecycle_all_offsets, vec![0, 1, 3, 6]);
        assert_eq!(arms.measured, vec![1, 2, 6]);
        assert_eq!(arms.measured_offsets, vec![0, 2, 2, 3]);
        assert_eq!(arms.lifecycle_measured, vec![11, 14, 15, 16]);
        assert_eq!(arms.lifecycle_measured_offsets, vec![0, 1, 1, 4]);
    }

    #[test]
    fn production_table_refusal_still_classifies_its_independent_ledger_as_measured() {
        let lifecycle = LifecyclePayload {
            complete: true,
            hook_epoch: 7,
            first_sequence: 1,
            last_sequence: 1,
            events: vec![d64::LifecycleRow {
                sequence: 1,
                game_hour_raw: 60_000_000,
                reason: d64::LifecycleReason::Executed,
                owner_tag_db_id: 1,
                event_id: 100_001,
                originator_tag_db_id: 2,
                scope_random: 3,
                due_before: 4,
                due_after: None,
            }],
        };
        let (table_refused, ledger) = classify_production_fold(
            false,
            d64::FoldOutcome {
                table: TablePart::Refused {
                    error: "zero originator".into(),
                },
                ledger: LedgerPart::Published(lifecycle.clone()),
            },
            Some(&lifecycle),
        )
        .unwrap();
        assert!(table_refused);
        assert!(matches!(
            ledger,
            HourLedger::Published(published) if published == lifecycle
        ));
    }
}
