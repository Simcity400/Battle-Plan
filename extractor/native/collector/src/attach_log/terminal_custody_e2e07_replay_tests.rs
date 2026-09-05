//! The terminal hook custody merge replayed on the WHOLE `captures/e2e_07`
//! recording, never a prefix: the recording's own terminal frame (the one its
//! close published with a `hour_label_mismatch` gap for each custody lane) is
//! taken through the merges the live close runs, with each lane's drain
//! continued from the recording's last record of that lane, and the assembled
//! stream is written for the production fold (`stream_fold`) to certify
//! end to end. `runs/term_terminal_custody/check_e2e07_replay.py` asserts the
//! fold's verdict against `merge_report.json`.
//!
//! Two closes are replayed: a QUIET close (every lane drained nothing at
//! teardown) and a DRAINED close (each lane that seals rows at teardown sealed
//! one). D86 and D75 re-serve their retained histories on either close, as the
//! producers do.
use std::path::{Path, PathBuf};

use compact_contract::domain_gap::{DomainGapReason, DomainGapRecordV1};
use compact_contract::domain_outcome::DomainOutcomeLedger;
use compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP;
use direct_frd_transitions_live::{
    decode_authoritative_trailer, DirectEventBatch, D45_V5_DIRECT_EVENT_TRAILER_MAGIC,
};
use land_detail55::x2_carriers::{LossTerminalDrain, PanelDrain, WidthTerminalDrain};
use naval4950::{
    decode_convoy_transport_attack_batch, encode_convoy_transport_attack_batch,
    ConvoyTransportAttackWitness,
};

use super::terminal_custody_tests::{
    d45_focus_completed_rows, d55_sealed_rows, del2_trailer, retained_decrement_lifecycle_row,
    zero_removal,
};
use super::*;

const E2E_07_STREAM_SHA256: &str =
    "50d4499108c87b5c4ec68b8acf352fd3b0fb9a0865fdde33ca6d4bbc3c4388c7";
const CUSTODY_DOMAINS: [u16; 7] = [45, 55, 64, 67, 75, 86, 90];
/// The close's wall clock is not in the recording; any value that dates the
/// terminal records after the last hourly sweep serves the merges.
const TERMINAL_ELAPSED_WALL_MS: u64 = 61_000;

#[derive(Clone, Copy, Debug)]
enum Close {
    Quiet,
    Drained,
}

impl Close {
    fn name(self) -> &'static str {
        match self {
            Close::Quiet => "quiet_close",
            Close::Drained => "drained_close",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Record {
    offset: usize,
    end: usize,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    record_flags: u32,
}

fn corpus_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("BATTLE_PLAN_E2E07_COLLECTOR_STREAM").or_else(|_| std::env::var("HOI4_E2E07_COLLECTOR_STREAM")) {
        let path = PathBuf::from(explicit);
        return path.is_file().then_some(path);
    }
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = dir.join("captures/e2e_07/collector_stream.bin");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn output_dir() -> PathBuf {
    std::env::var_os("BATTLE_PLAN_E2E07_REPLAY_DIR").or_else(|| std::env::var_os("HOI4_E2E07_REPLAY_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("hoi4_e2e07_terminal_custody"))
}

fn walk(stream: &[u8]) -> Vec<Record> {
    let mut records = Vec::new();
    let mut offset = 0usize;
    while offset < stream.len() {
        let header = compact_contract::parse_compact_record_header(&stream[offset..])
            .unwrap_or_else(|error| panic!("record at {offset}: {error:?}"));
        let end = offset + header.record_len();
        assert!(end <= stream.len(), "record at {offset} runs past the stream");
        records.push(Record {
            offset,
            end,
            sequence: header.sequence,
            game_hour_raw: header.game_hour_raw,
            domain_id: header.domain_id,
            record_type: header.record_type,
            record_flags: header.record_flags,
        });
        offset = end;
    }
    records
}

fn payload<'a>(stream: &'a [u8], record: &Record) -> &'a [u8] {
    &stream[record.offset + compact_contract::COMPACT_RECORD_HEADER_LEN..record.end]
}

/// The last hourly DATA record of `domain_id` before the terminal frame, raw
/// grammar: none of the custody lanes is delta-enabled.
fn last_data<'a>(stream: &'a [u8], history: &[Record], domain_id: u16) -> &'a [u8] {
    let record = history
        .iter()
        .rev()
        .find(|record| {
            record.domain_id == domain_id && record.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
        })
        .unwrap_or_else(|| panic!("the recording has no domain {domain_id} DATA record"));
    assert_eq!(
        record.record_flags & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
        0,
        "domain {domain_id} rides raw grammar"
    );
    payload(stream, record)
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

/// The D45E authority batch of the recording's last D45 record that carries
/// one: the fold's hook epoch, drain sequence, kind mask and row cursor all
/// continue from it.
fn last_d45_batch(stream: &[u8], history: &[Record]) -> DirectEventBatch {
    for record in history.iter().rev() {
        if record.domain_id != 45 || record.record_type != COMPACT_RECORD_TYPE_DOMAIN_DATA {
            continue;
        }
        let bytes = payload(stream, record);
        let Some(at) = bytes
            .windows(D45_V5_DIRECT_EVENT_TRAILER_MAGIC.len())
            .rposition(|window| window == D45_V5_DIRECT_EVENT_TRAILER_MAGIC)
        else {
            continue;
        };
        let trailer = &bytes[at..];
        let first_sequence = if read_u32(trailer, 12) == 0 {
            read_u64(trailer, 32)
        } else {
            read_u64(trailer, 56)
        };
        return decode_authoritative_trailer(trailer, first_sequence)
            .unwrap_or_else(|error| panic!("D45E trailer at sequence {}: {error:?}", record.sequence));
    }
    panic!("the recording has no D45 record with a D45E authority batch");
}

/// The fold's D64 lifecycle continuity state after the whole history: the hook
/// epoch and the last non-zero `last_sequence` any DEL2 trailer declared.
fn d64_lifecycle_continuity(stream: &[u8], history: &[Record]) -> (u64, u64) {
    let mut hook_epoch = None;
    let mut last_sequence = 0u64;
    for record in history {
        if record.domain_id != 64 || record.record_type != COMPACT_RECORD_TYPE_DOMAIN_DATA {
            continue;
        }
        let parts = wire_format::d64::split_payload(payload(stream, record)).unwrap();
        let Some(trailer) = parts.trailer else { continue };
        let lifecycle = wire_format::d64::parse_trailer(trailer).unwrap();
        hook_epoch = Some(lifecycle.hook_epoch);
        if lifecycle.last_sequence != 0 {
            last_sequence = lifecycle.last_sequence;
        }
    }
    (hook_epoch.expect("the recording has a D64 DEL2 trailer"), last_sequence)
}

struct MergedClose {
    frame: Vec<u8>,
    verdict: FrameCompletenessVerdict,
    findings: Vec<String>,
    lanes: serde_json::Value,
}

/// The merges the live close runs (`write_attach_heartbeat`'s terminal arm),
/// each lane continuing the recording's own producer state.
fn merge_terminal_frame(
    stream: &[u8],
    history: &[Record],
    terminal: &[u8],
    game_hour_raw: i32,
    close: Close,
) -> MergedClose {
    let frame_hours = D75TerminalFrameHours::from_raw(game_hour_raw).unwrap();
    let frame_hour = frame_hours.index;
    assert_ne!(frame_hours.raw, frame_hour);
    let elapsed = TERMINAL_ELAPSED_WALL_MS;
    let mut frame = terminal.to_vec();
    let mut outcomes = DomainOutcomeLedger::new();
    let drained = matches!(close, Close::Drained);

    // D45: the sealed batch continues the hook epoch and drain sequence.
    let last = last_d45_batch(stream, history);
    let mut rows = if drained { d45_focus_completed_rows(game_hour_raw) } else { Vec::new() };
    for (index, row) in rows.iter_mut().enumerate() {
        row.sequence = last.sequence_cursor + index as u64;
    }
    let observed_kind_mask = rows
        .iter()
        .fold(last.observed_kind_mask, |mask, row| mask | row.kind.root_mask_bit());
    let batch = DirectEventBatch {
        hook_epoch: last.hook_epoch,
        drain_sequence: last.drain_sequence + 1,
        installed_root_mask: last.installed_root_mask,
        observed_kind_mask,
        sequence_cursor: last.sequence_cursor + rows.len() as u64,
        producer_complete: true,
        evidence_fixture: last.evidence_fixture,
        rows,
    };
    let d45_rows = batch.rows.len();
    merge_terminal_d45_records(
        &mut frame,
        game_hour_raw,
        elapsed,
        &mut None,
        direct_frd_transitions_live::a729_hook::DirectEventBatchReservation {
            reservation_id: 1,
            batch,
            overflow_dropped_rows: 0,
        },
    )
    .unwrap();

    // D55: the four terminal carriers.
    let (width, loss, panels) = d55_sealed_rows(game_hour_raw);
    let (width_rows, loss_rows, panel_rows) = if drained {
        (vec![width], vec![loss], panels)
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };
    let d55_rows = width_rows.len() + loss_rows.len() + panel_rows.len();
    merge_terminal_d55_records(
        &mut frame,
        game_hour_raw,
        elapsed,
        Some(&land_detail55::a729_hook::TerminalDrain::default()),
        Some(WidthTerminalDrain { rows: width_rows, ..Default::default() }),
        Some(LossTerminalDrain { rows: loss_rows, ..Default::default() }),
        Some(PanelDrain { observations: panel_rows, ..Default::default() }),
    )
    .unwrap();

    // D67: a closed zero-removal set.
    let zero_removals = if drained { vec![zero_removal(7, game_hour_raw)] } else { Vec::new() };
    merge_terminal_d67_records(&mut frame, game_hour_raw, elapsed, &zero_removals, 0, &mut outcomes)
        .unwrap();

    // D86: the retained history the hook re-serves at teardown, dated to the
    // terminal frame.
    let last86 = sunk_ship_history_live_lane::decode_payload(last_data(stream, history, 86)).unwrap();
    let snapshot = last86.snapshot.expect("the recording's last D86 record carries its history");
    let d86_events = snapshot.events.len();
    let frame86 = sunk_ship_history_live_lane::SunkShipFrame {
        elapsed_wall_ms: elapsed,
        frame_hour,
        status_flags: sunk_ship_history_live_lane::receipt_status_word(&snapshot),
        authority: sunk_ship_history_live_lane::AuthorityState::StaticExact20Receipt,
        snapshot: Some(snapshot),
    };
    let payload86 = sunk_ship_history_live_lane::encode_payload(&frame86).unwrap();
    replace_terminal_hook_domain_records(
        &mut frame, game_hour_raw, elapsed, 86, &payload86, d86_events as u32, 0,
        "d86_terminal_custody",
    )
    .unwrap();

    // D90: an exact terminal drain that emptied the queue, continuing the
    // producer sequence the recording's last witness declared.
    let last90 = decode_convoy_transport_attack_batch(last_data(stream, history, 90)).unwrap();
    let producer_sequence_next = last90.witness.producer_sequence_next;
    let witness = ConvoyTransportAttackWitness {
        hook_installed: true,
        queue_drain_complete: true,
        drained_rows: 0,
        remaining_events: 0,
        active_callbacks: 0,
        events_seen_this_drain: 0,
        events_emitted_this_drain: 0,
        drops_this_drain: 0,
        ..last90.witness
    };
    let payload90 = encode_convoy_transport_attack_batch(&[], witness).unwrap();
    assert!(decode_convoy_transport_attack_batch(&payload90).unwrap().exact_events_complete);
    replace_terminal_hook_domain_records(
        &mut frame, game_hour_raw, elapsed, 90, &payload90, 0, 0, "d90_terminal_custody",
    )
    .unwrap();
    record_read_succeeded_superseding(
        &mut outcomes,
        90,
        compact_contract::domain_outcome::READ_PREDICATE_PAYLOAD_DECODED_EXACT,
    );

    // D75/D64: the fired-event ledger re-served at the terminal hour index and
    // the final D64 lifecycle drain under the recording's hook epoch.
    let last75 = fired_event_popups_live_lane::decode_payload(last_data(stream, history, 75)).unwrap();
    let frame75 = fired_event_popups_live_lane::FiredEventFrame {
        elapsed_wall_ms: elapsed,
        frame_hour,
        ..last75
    };
    let payload75 = fired_event_popups_live_lane::encode_payload(&frame75).unwrap();
    let (hook_epoch, last_sequence) = d64_lifecycle_continuity(stream, history);
    let lifecycle_rows = if drained {
        vec![retained_decrement_lifecycle_row(last_sequence + 1, game_hour_raw)]
    } else {
        Vec::new()
    };
    let del2 = del2_trailer(hook_epoch, &lifecycle_rows);
    merge_terminal_d75_d64_payloads(
        &mut frame,
        game_hour_raw,
        &frame75,
        &payload75,
        1,
        lifecycle_rows.len() as u64,
        Sha256::digest(&payload75).into(),
        [0x64; 32],
        &del2,
        Sha256::digest(&del2).into(),
    )
    .unwrap();

    // The live tail: re-derive the cursor, judge the frame, publish the notice.
    let first = compact_contract::parse_compact_record_header(&frame).unwrap().sequence;
    let mut next_sequence = resequence_compact_records(&mut frame, first).unwrap();
    let findings = compact_frame_completeness_findings(&frame, game_hour_raw, &outcomes);
    let verdict = classify_frame_completeness(&findings);
    if let FrameCompletenessVerdict::Degraded(domains) = &verdict {
        insert_frame_degraded_notice_before_frame_end(
            &mut frame, &mut next_sequence, game_hour_raw, domains.clone(),
        )
        .unwrap();
    }
    validate_terminal_frame_end(&frame).unwrap();

    let lanes = serde_json::json!({
        "45": { "rows": d45_rows, "hook_epoch": last.hook_epoch, "drain_sequence": last.drain_sequence + 1, "sequence_cursor": last.sequence_cursor + d45_rows as u64 },
        "55": { "rows": d55_rows },
        "67": { "rows": zero_removals.len() },
        "86": { "history_events": d86_events, "frame_hour": frame_hour },
        "90": { "rows": 0, "producer_sequence_next": producer_sequence_next },
        "75": { "rows": frame75.rows.len(), "frame_hour": frame_hour, "fired_events_complete": frame75.fired_events_complete },
        "64": { "rows": lifecycle_rows.len(), "hook_epoch": hook_epoch, "first_sequence": lifecycle_rows.first().map(|row| row.sequence) },
    });
    MergedClose {
        frame,
        verdict,
        findings: findings.iter().map(|finding| finding.text.clone()).collect(),
        lanes,
    }
}

/// PERF status word, DATA payload length and GAP count of `domain_id` in one
/// frame; the PERF and DATA counts are asserted to be exactly one each.
fn frame_domain_evidence(frame: &[u8], domain_id: u16) -> serde_json::Value {
    let mut perf = Vec::new();
    let mut data = Vec::new();
    let mut gaps = 0usize;
    for record in walk(frame) {
        if record.domain_id != domain_id {
            continue;
        }
        let bytes = payload(frame, &record);
        match record.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF => perf.push(read_u32(bytes, 32)),
            COMPACT_RECORD_TYPE_DOMAIN_DATA => data.push((bytes.len(), record.record_flags)),
            COMPACT_RECORD_TYPE_DOMAIN_GAP => gaps += 1,
            _ => {}
        }
    }
    assert_eq!((perf.len(), data.len(), gaps), (1, 1, 0), "domain {domain_id} custody pair");
    assert!(
        compact_contract::compact_status_is_terminal_custody(perf[0]),
        "domain {domain_id} PERF word {:#010x} carries the custody bit",
        perf[0]
    );
    serde_json::json!({
        "perf_status": perf[0],
        "data_len": data[0].0,
        "data_flags": data[0].1,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

#[test]
#[ignore = "needs the 924 MB e2e_07 capture; set BATTLE_PLAN_E2E07_COLLECTOR_STREAM"]
fn e2e07_terminal_hook_custody_merges_the_recordings_own_terminal_frame() {
    let path = corpus_path().expect(
        "the e2e_07 collector stream was not found; set BATTLE_PLAN_E2E07_COLLECTOR_STREAM to \
         captures/e2e_07/collector_stream.bin",
    );
    let stream = std::fs::read(&path).unwrap();
    assert_eq!(sha256_hex(&stream), E2E_07_STREAM_SHA256, "this is capture e2e_07");
    let records = walk(&stream);
    assert_eq!(records.last().unwrap().end, stream.len(), "the whole recording frames");
    let frame_ends: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, record)| record.record_type == COMPACT_RECORD_TYPE_FRAME_END)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(frame_ends.len(), 399);
    let history_end = records[frame_ends[frame_ends.len() - 2]].end;
    let history = &records[..=frame_ends[frame_ends.len() - 2]];
    let terminal = &stream[history_end..];
    let terminal_records = &records[frame_ends[frame_ends.len() - 2] + 1..];
    let game_hour_raw = terminal_records.last().unwrap().game_hour_raw;
    assert!(terminal_records.iter().all(|record| record.game_hour_raw == game_hour_raw));

    // The premise, on the recording: every custody lane closed as a gap.
    let mut premise = serde_json::Map::new();
    for domain_id in CUSTODY_DOMAINS {
        let mut kinds = (0usize, 0usize, Vec::new());
        for record in terminal_records.iter().filter(|record| record.domain_id == domain_id) {
            match record.record_type {
                COMPACT_RECORD_TYPE_DOMAIN_PERF => kinds.0 += 1,
                COMPACT_RECORD_TYPE_DOMAIN_DATA => kinds.1 += 1,
                COMPACT_RECORD_TYPE_DOMAIN_GAP => {
                    let gap = DomainGapRecordV1::decode(payload(&stream, record)).unwrap();
                    assert_eq!(gap.reason, DomainGapReason::HourLabelMismatch);
                    kinds.2.push(gap.reason.wire_name());
                }
                _ => {}
            }
        }
        assert_eq!((kinds.0, kinds.1, kinds.2.len()), (0, 0, 1), "domain {domain_id} before the merge");
        premise.insert(domain_id.to_string(), serde_json::json!({ "perf": 0, "data": 0, "gap": kinds.2[0] }));
    }

    let out_dir = output_dir();
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut closes = serde_json::Map::new();
    for close in [Close::Quiet, Close::Drained] {
        let merged = merge_terminal_frame(&stream, history, terminal, game_hour_raw, close);
        let (verdict, degraded) = match &merged.verdict {
            FrameCompletenessVerdict::Complete => ("complete", Vec::new()),
            FrameCompletenessVerdict::Degraded(domains) => {
                ("degraded", domains.iter().map(|domain| domain.domain_id).collect())
            }
            FrameCompletenessVerdict::Refused => panic!("{close:?} refused: {:?}", merged.findings),
        };
        assert!(
            degraded.iter().all(|domain| !CUSTODY_DOMAINS.contains(domain)),
            "{close:?} degraded a custody lane: {:?}",
            merged.findings
        );
        let mut domains = serde_json::Map::new();
        for domain_id in CUSTODY_DOMAINS {
            domains.insert(domain_id.to_string(), frame_domain_evidence(&merged.frame, domain_id));
        }
        let mut assembled = stream[..history_end].to_vec();
        assembled.extend_from_slice(&merged.frame);
        let output = out_dir.join(format!("{}.bin", close.name()));
        write(&output, &assembled);
        closes.insert(
            close.name().to_string(),
            serde_json::json!({
                "stream": output,
                "bytes": assembled.len(),
                "sha256": sha256_hex(&assembled),
                "terminal_frame_records": walk(&merged.frame).len(),
                "verdict": verdict,
                "degraded_domains": degraded,
                "findings": merged.findings,
                "lanes": merged.lanes,
                "domains": domains,
            }),
        );
    }
    let report = serde_json::json!({
        "input": {
            "path": path,
            "bytes": stream.len(),
            "sha256": E2E_07_STREAM_SHA256,
            "records": records.len(),
            "frame_ends": frame_ends.len(),
            "history_bytes": history_end,
            "terminal_hour_raw": game_hour_raw,
            "terminal_hour_index": compact_contract::calendar::hour_index_from_raw(game_hour_raw),
            "terminal_frame_records": terminal_records.len(),
        },
        "custody_domains": CUSTODY_DOMAINS,
        "premise": premise,
        "closes": closes,
    });
    write(&out_dir.join("merge_report.json"), format!("{}\n", serde_json::to_string_pretty(&report).unwrap()).as_bytes());
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
