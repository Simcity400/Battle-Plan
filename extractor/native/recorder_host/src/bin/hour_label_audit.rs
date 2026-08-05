use compact_contract::campaign_identity::{parse_cid1_record, COMPACT_RECORD_TYPE_STREAM_HEADER};
use compact_contract::diagnostic_channel::{decode_units, DiagUnit};
use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, DomainGapReason,
    DomainGapRecordV1, COMPACT_DOMAIN_PERF_BASE_LEN, COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC,
    COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2, COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_CAPTURE_TERMINATED,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_GAP,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_DEGRADED,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_TYPE_FRAME_REFUSED,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

const RC_PROBE_DOMAIN_ID: u16 = 250;
const TARGET_FRAME_LABEL: i32 = 60_832_037;
const GAP_EXAMPLE_LIMIT: usize = 50;
const NOTABLE_FRAME_LIMIT: usize = 200;

#[derive(Serialize)]
struct AuditReport {
    input_path: String,
    bytes_consumed: u64,
    records_seen: u64,
    parse_error_count: usize,
    parse_errors: Vec<ParseError>,
    section_a_frame_census: FrameCensus,
    section_b_domain_gaps: GapCensus,
    section_c_per_frame: PerFrameSection,
    section_d_classification: ClassificationSection,
    section_e_target_label: TargetLabelSection,
}

#[derive(Clone, Debug, Serialize)]
struct ParseError {
    offset: u64,
    error: String,
    framing_lost: bool,
}

#[derive(Serialize)]
struct FrameCensus {
    total_frames: u64,
    frames_with_lane_wall_probe: u64,
    lw_hour_label_skew_abs_distribution: Vec<ValueCount>,
    frames_with_lw_hour_label_skew_missing_eq_1: u64,
    lw_worker_span_us: Stats,
    lw_prewarm_us: Stats,
    exact_instrument: ExactInstrument,
}

/// `lw_hour_label_mismatch_domains` / `lw_bracketed_domains`, summed over the
/// capture. THE number this whole tool is a substitute for: every other section
/// here infers the defect from a second boundary happening to tear a later
/// bracket, which needs luck; this one is counted directly on every frame.
///
/// `frames_carrying_it` is what makes the rest readable. A capture from a
/// recorder that predates the instrument reports zeroes for want of the unit,
/// not for want of mismatches, and the two must never be confused.
#[derive(Default, Serialize)]
struct ExactInstrument {
    frames_carrying_it: u64,
    frames_with_a_mismatch: u64,
    mismatched_domain_hours: u64,
    bracketed_domain_hours: u64,
    unusable_label_frames: u64,
    /// Frames carrying SOME but not all of the three units. Never zero silently:
    /// a nonzero value here means the numbers above are built from a population
    /// the producer did not describe, and must not be read as a rate.
    partial_frames: u64,
    /// How many domain-hours each frame mislabelled, as a distribution.
    ///
    /// The totals above cannot answer the question the defect is actually
    /// diagnosed by: 2,005 mislabelled domain-hours is a frame that lost its
    /// whole sweep 30 times over, or 250 frames that each lost a handful, and
    /// those have different causes and different fixes. t21 is the second
    /// shape sharpened -- 62 frames lost exactly 8 (one serial tail), 36 lost
    /// 17..67, and one lost 67 of 68.
    mismatched_domain_hours_per_frame: Vec<ValueCount>,
    /// Frames whose EVERY bracketed domain read outside the label. The sweep
    /// ran entirely in another hour; nothing in the frame is attributable.
    wholly_mislabelled_frames: u64,
    /// Frames that mislabelled some but not all of their bracketed domains.
    partially_mislabelled_frames: u64,
}

#[derive(Serialize)]
struct ValueCount {
    value: u64,
    count: u64,
}

#[derive(Serialize)]
struct Stats {
    count: u64,
    min: Option<u64>,
    median: Option<f64>,
    mean: Option<f64>,
    max: Option<u64>,
}

#[derive(Serialize)]
struct GapCensus {
    reason_counts: Vec<ReasonCount>,
    undecoded_gap_records: u64,
    clock_witness_delta_counts: Vec<WitnessDeltaCount>,
    clock_witness_examples: Vec<WitnessExample>,
    clock_witness_examples_suppressed: u64,
}

#[derive(Serialize)]
struct ReasonCount {
    reason: &'static str,
    count: u64,
}

#[derive(Serialize)]
struct WitnessDeltaCount {
    reason: &'static str,
    opening_delta: i128,
    closing_delta: i128,
    count: u64,
}

#[derive(Clone, Serialize)]
struct WitnessExample {
    offset: u64,
    sequence: u64,
    domain_id: u16,
    reason: &'static str,
    frame_label: i32,
    opening_witness: u64,
    closing_witness: u64,
    opening_delta: i128,
    closing_delta: i128,
}

#[derive(Serialize)]
struct PerFrameSection {
    frames: Vec<FrameSummary>,
    frames_with_torn_gap: u64,
    domain_data_records_in_frames_with_torn_gap: u64,
}

#[derive(Clone, Serialize)]
struct FrameSummary {
    sequence_start: u64,
    sequence_end: u64,
    label: i32,
    domain_data_records: u64,
    domain_gap_records: u64,
    gap_reasons: Vec<&'static str>,
}

#[derive(Serialize)]
struct ClassificationSection {
    table: ClassificationTable,
    notable_frames: Vec<NotableFrame>,
    notable_frames_suppressed: u64,
}

/// Every frame, counted by which of the THREE per-frame witnesses it carries.
///
/// It was two axes, and the missing one was the only one that can see the
/// defect this tool is named after. `lw_hour_label_skew_abs` is a frame-START
/// reading: it fires only when the label was already wrong before the sweep
/// began, which the producer makes nearly impossible -- the label is claimed
/// from the latest observed hour a few microseconds earlier. On t21 it is 0 on
/// all 315 probe-carrying frames, so the whole `label_skew=yes` row was empty
/// while 98 frames mislabelled 2,005 domain-hours. A Torn gap caught 93 of
/// those by luck (some OTHER reader happened to straddle the same boundary);
/// the remaining 5 -- including the worst frame in the capture, which
/// mislabelled 67 of its 68 domain-hours and tore nothing -- were counted in
/// `neither`, the same cell as a perfectly clean frame.
///
/// So the exact instrument is an axis now, and `mismatch_with_no_other_witness`
/// states the size of the old blind spot in every report rather than leaving it
/// to be rediscovered.
#[derive(Default, Serialize)]
struct ClassificationTable {
    neither: u64,
    label_skew_only: u64,
    torn_witness_only: u64,
    both: u64,
    mismatch_only: u64,
    mismatch_and_torn: u64,
    mismatch_and_label_skew: u64,
    mismatch_and_label_skew_and_torn: u64,
    /// Frames a `label_skew x torn` view alone calls clean while the exact
    /// per-frame instrument says they mislabelled at least one domain-hour.
    /// Identical to `mismatch_only` by construction, and named separately
    /// because that is the number the reader needs, not an inference from a
    /// cell name.
    mismatch_with_no_other_witness: u64,
    /// Frames the exact instrument CANNOT SPEAK FOR, split by the other two
    /// axes. `mismatch` is a three-valued thing -- yes, no, and no witness --
    /// and the eight cells above only have room for two of them, so an
    /// uninstrumented frame used to be flattened into the `mismatch=no` row and
    /// counted beside frames that were actually checked and found clean. On t21
    /// that put 3 probe-less frames in `neither`: 315 frames carried the
    /// instrument and 98 of them mismatched, so 217 frames were checked clean
    /// while the cell read 220. The unknown population is reported here instead
    /// so the `mismatch=no` row means "checked, and clean".
    mismatch_unknown_neither: u64,
    mismatch_unknown_label_skew_only: u64,
    mismatch_unknown_torn_witness_only: u64,
    mismatch_unknown_both: u64,
}

#[derive(Serialize)]
struct NotableFrame {
    sequence_start: u64,
    sequence_end: u64,
    label: i32,
    skew_abs: Option<u64>,
    /// The exact instrument, on the line. A notable frame that is notable
    /// BECAUSE of it must say so; without these two the reader can only count
    /// `HourLabelMismatch` gap entries, which is the same number only while the
    /// producer refuses every mismatch.
    mismatch_domains: Option<u64>,
    bracketed_domains: Option<u64>,
    worker_span_us: Option<u64>,
    prewarm_us: Option<u64>,
    gaps: Vec<FrameGap>,
    domain_data_records: u64,
}

#[derive(Clone, Serialize)]
struct FrameGap {
    offset: u64,
    sequence: u64,
    domain_id: u16,
    reason: &'static str,
    opening_witness: Option<u64>,
    closing_witness: Option<u64>,
    opening_delta: Option<i128>,
    closing_delta: Option<i128>,
}

#[derive(Serialize)]
struct TargetLabelSection {
    label: i32,
    frames: Vec<TargetFrame>,
}

#[derive(Serialize)]
struct TargetFrame {
    sequence_start: u64,
    sequence_end: u64,
    records: Vec<RecordDetail>,
}

#[derive(Clone, Serialize)]
struct RecordDetail {
    offset: u64,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    record_type_name: &'static str,
    payload_len: usize,
    record_flags: u32,
    gap: Option<GapRecordDetail>,
    lane_wall_probe: Option<LaneWallProbe>,
    decode_error: Option<String>,
}

#[derive(Clone, Serialize)]
struct GapRecordDetail {
    part: &'static str,
    reason: &'static str,
    target_hour_raw: i32,
    range_start_hour_raw: i32,
    range_end_hour_raw: i32,
    tick_before: u64,
    tick_after: u64,
    queue_capacity: u32,
    queue_depth: u32,
    queue_dropped: u64,
    outcome_ordinal: u64,
}

#[derive(Clone, Default, Serialize)]
struct LaneWallProbe {
    lw_frames: Option<u64>,
    lw_worker_span_us: Option<u64>,
    lw_prewarm_us: Option<u64>,
    lw_hour_label_skew_abs: Option<u64>,
    lw_hour_label_skew_missing: Option<u64>,
    /// The exact per-frame instrument. Absent in captures older than it, which
    /// is why these stay `Option` rather than defaulting to zero: "this
    /// recorder did not emit the counter" and "this frame mismatched nothing"
    /// are different facts and a zero would merge them.
    lw_hour_label_mismatch_domains: Option<u64>,
    lw_bracketed_domains: Option<u64>,
    lw_hour_label_unusable: Option<u64>,
}

#[derive(Default)]
struct FrameAccumulator {
    sequence_start: Option<u64>,
    sequence_end: u64,
    label: Option<i32>,
    data_count: u64,
    gap_count: u64,
    gaps: Vec<FrameGap>,
    lane_wall: Option<LaneWallProbe>,
    records: Vec<RecordDetail>,
}

impl FrameAccumulator {
    fn is_empty(&self) -> bool {
        self.sequence_start.is_none()
    }

    fn push_record(&mut self, record: RecordDetail) {
        self.sequence_start.get_or_insert(record.sequence);
        self.sequence_end = record.sequence;
        self.label.get_or_insert(record.game_hour_raw);
        self.records.push(record);
    }
}

#[derive(Default)]
struct AuditState {
    bytes_consumed: u64,
    records_seen: u64,
    previous_sequence: Option<u64>,
    errors: Vec<ParseError>,
    current: FrameAccumulator,
    total_frames: u64,
    frames_with_probe: u64,
    skew_distribution: BTreeMap<u64, u64>,
    skew_missing_frames: u64,
    worker_spans: Vec<u64>,
    prewarm_times: Vec<u64>,
    reason_counts: BTreeMap<&'static str, u64>,
    undecoded_gaps: u64,
    witness_delta_counts: BTreeMap<(&'static str, i128, i128), u64>,
    witness_examples: Vec<WitnessExample>,
    witness_examples_suppressed: u64,
    frame_summaries: Vec<FrameSummary>,
    torn_frames: u64,
    data_in_torn_frames: u64,
    classification: ClassificationTable,
    notable_frames: Vec<NotableFrame>,
    notable_suppressed: u64,
    target_frames: Vec<TargetFrame>,
    exact_instrument_frames: u64,
    exact_mismatch_frames: u64,
    exact_mismatch_domains: u64,
    exact_bracketed_domains: u64,
    exact_unusable_label_frames: u64,
    exact_instrument_partial_frames: u64,
    exact_mismatch_distribution: BTreeMap<u64, u64>,
    exact_wholly_mislabelled_frames: u64,
    exact_partially_mislabelled_frames: u64,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("hour_label_audit: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (input, json_output) = parse_args()?;
    let file =
        File::open(&input).map_err(|error| format!("open input {}: {error}", input.display()))?;
    let mut reader = BufReader::new(file);
    let mut state = AuditState::default();
    scan_stream(&mut reader, &mut state)?;
    let report = finish_report(&input, state);
    print_report(&report).map_err(|error| format!("write report: {error}"))?;

    if let Some(path) = json_output {
        let file = File::create(&path)
            .map_err(|error| format!("create JSON output {}: {error}", path.display()))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &report)
            .map_err(|error| format!("encode JSON output {}: {error}", path.display()))?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|error| format!("write JSON output {}: {error}", path.display()))?;
    }
    Ok(())
}

fn parse_args() -> Result<(PathBuf, Option<PathBuf>), String> {
    let mut args = env::args_os().skip(1);
    let input = args.next().map(PathBuf::from).ok_or_else(usage)?;
    let mut json = None;
    while let Some(arg) = args.next() {
        if arg != "--json" {
            return Err(format!("unknown argument {:?}\n{}", arg, usage()));
        }
        if json.is_some() {
            return Err(format!("--json may be specified only once\n{}", usage()));
        }
        json = Some(
            args.next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("--json requires an output path\n{}", usage()))?,
        );
    }
    Ok((input, json))
}

fn usage() -> String {
    "usage: hour_label_audit <collector_stream.bin> [--json out.json]".to_string()
}

fn scan_stream(reader: &mut impl Read, state: &mut AuditState) -> Result<(), String> {
    let mut header_bytes = [0u8; COMPACT_RECORD_HEADER_LEN];
    loop {
        let offset = state.bytes_consumed;
        let header_read = read_up_to(reader, &mut header_bytes)
            .map_err(|error| format!("read header at offset {offset}: {error}"))?;
        state.bytes_consumed = state.bytes_consumed.saturating_add(header_read as u64);
        if header_read == 0 {
            break;
        }
        if header_read != COMPACT_RECORD_HEADER_LEN {
            state.errors.push(ParseError {
                offset,
                error: format!(
                    "truncated record header: {header_read} byte(s), expected {COMPACT_RECORD_HEADER_LEN}"
                ),
                framing_lost: true,
            });
            break;
        }
        let header = match parse_compact_record_header(&header_bytes) {
            Ok(header) => header,
            Err(error) => {
                state.errors.push(ParseError {
                    offset,
                    error: error.to_string(),
                    framing_lost: true,
                });
                break;
            }
        };
        let mut payload = Vec::new();
        payload
            .try_reserve_exact(header.payload_len)
            .map_err(|error| {
                format!(
                    "reserve {} payload bytes at offset {offset}: {error}",
                    header.payload_len
                )
            })?;
        payload.resize(header.payload_len, 0);
        let payload_read = read_up_to(reader, &mut payload)
            .map_err(|error| format!("read payload at offset {offset}: {error}"))?;
        state.bytes_consumed = state.bytes_consumed.saturating_add(payload_read as u64);
        if payload_read != header.payload_len {
            state.errors.push(ParseError {
                offset,
                error: format!(
                    "truncated payload: declared {} byte(s), only {payload_read} available",
                    header.payload_len
                ),
                framing_lost: true,
            });
            break;
        }
        state.records_seen = state.records_seen.saturating_add(1);
        check_sequence(state, offset, header.sequence);
        process_record(state, offset, &header, &payload);
    }

    if !state.current.is_empty() {
        let start = state.current.sequence_start.unwrap_or(0);
        state.errors.push(ParseError {
            offset: state.bytes_consumed,
            error: format!(
                "stream ended with an unclosed frame beginning at sequence {start} (no FRAME_END)"
            ),
            framing_lost: false,
        });
    }
    Ok(())
}

fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0usize;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

fn check_sequence(state: &mut AuditState, offset: u64, sequence: u64) {
    if let Some(previous) = state.previous_sequence {
        if sequence != previous.wrapping_add(1) {
            state.errors.push(ParseError {
                offset,
                error: format!(
                    "record sequence {sequence} is not the exact successor of {previous}"
                ),
                framing_lost: false,
            });
        }
    }
    state.previous_sequence = Some(sequence);
}

fn process_record(
    state: &mut AuditState,
    offset: u64,
    header: &compact_contract::CompactRecordHeader,
    payload: &[u8],
) {
    let mut detail = RecordDetail {
        offset,
        sequence: header.sequence,
        game_hour_raw: header.game_hour_raw,
        domain_id: header.domain_id,
        record_type: header.record_type,
        record_type_name: record_type_name(header.record_type),
        payload_len: header.payload_len,
        record_flags: header.record_flags,
        gap: None,
        lane_wall_probe: None,
        decode_error: None,
    };

    if header.record_type == COMPACT_RECORD_TYPE_STREAM_HEADER {
        if let Err(error) = parse_cid1_record(header, payload) {
            record_error(
                state,
                offset,
                format!("invalid CID1 stream header: {error}"),
            );
        }
        return;
    }

    let semantic_error = validate_compact_record_semantics(
        header.domain_id,
        header.record_type,
        header.record_flags,
        payload,
    )
    .err();
    if let Some(error) = semantic_error.as_ref() {
        let message = format!("invalid H4CR record semantics: {error}");
        detail.decode_error = Some(message.clone());
        record_error(state, offset, message);
    }

    if header.record_type == COMPACT_RECORD_TYPE_CAPTURE_TERMINATED {
        return;
    }
    if header.record_type == COMPACT_RECORD_TYPE_FRAME_REFUSED {
        if !state.current.is_empty() {
            record_error(
                state,
                offset,
                "FRAME_REFUSED arrived while a published frame was open; discarding the open frame"
                    .to_string(),
            );
            state.current = FrameAccumulator::default();
        }
        return;
    }

    if let Some(label) = state.current.label {
        if label != header.game_hour_raw {
            record_error(
                state,
                offset,
                format!(
                    "record hour {} differs from open frame label {label}",
                    header.game_hour_raw
                ),
            );
        }
    }

    if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
        state.current.data_count = state.current.data_count.saturating_add(1);
    }
    if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_GAP {
        state.current.gap_count = state.current.gap_count.saturating_add(1);
        if semantic_error.is_none() {
            match DomainGapRecordV1::decode(payload) {
                Ok(gap) => {
                    if gap.target_hour_raw != header.game_hour_raw {
                        let message = format!(
                            "DOMAIN_GAP target hour {} differs from frame label {}",
                            gap.target_hour_raw, header.game_hour_raw
                        );
                        detail.decode_error = Some(message.clone());
                        record_error(state, offset, message);
                    }
                    detail.gap = Some(gap_record_detail(gap));
                    account_gap(
                        state,
                        offset,
                        header.sequence,
                        header.domain_id,
                        header.game_hour_raw,
                        gap,
                    );
                }
                Err(error) => {
                    let message = format!("DOMAIN_GAP decode failed: {error}");
                    detail.decode_error = Some(message.clone());
                    state.undecoded_gaps = state.undecoded_gaps.saturating_add(1);
                    record_error(state, offset, message);
                }
            }
        } else {
            state.undecoded_gaps = state.undecoded_gaps.saturating_add(1);
        }
    }

    if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF
        && header.domain_id == RC_PROBE_DOMAIN_ID
        && semantic_error.is_none()
    {
        match decode_probe_units(header.record_flags, payload) {
            Ok(Some(units)) => {
                if units.iter().any(|unit| unit.name == "lw_frames") {
                    let probe = lane_wall_probe(&units);
                    if state.current.lane_wall.is_some() {
                        record_error(
                            state,
                            offset,
                            "frame contains more than one lane-wall probe record".to_string(),
                        );
                    } else {
                        state.current.lane_wall = Some(probe.clone());
                    }
                    detail.lane_wall_probe = Some(probe);
                }
            }
            Ok(None) => {}
            Err(error) => {
                let message = format!("domain-250 diagnostic decode failed: {error}");
                detail.decode_error = Some(message.clone());
                record_error(state, offset, message);
            }
        }
    }

    let is_frame_end = header.record_type == COMPACT_RECORD_TYPE_FRAME_END;
    state.current.push_record(detail);
    if is_frame_end {
        finalize_frame(state);
    }
}

fn record_error(state: &mut AuditState, offset: u64, error: String) {
    state.errors.push(ParseError {
        offset,
        error,
        framing_lost: false,
    });
}

fn decode_probe_units(record_flags: u32, payload: &[u8]) -> Result<Option<Vec<DiagUnit>>, String> {
    if record_flags & COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER == 0 {
        return Ok(None);
    }
    let trailer = payload
        .get(COMPACT_DOMAIN_PERF_BASE_LEN..)
        .ok_or_else(|| "payload is shorter than the canonical DOMAIN_PERF base".to_string())?;
    if trailer.len() < 8 {
        return Err(format!(
            "PDU1 trailer is {} byte(s), expected at least 8",
            trailer.len()
        ));
    }
    if &trailer[..4] != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC {
        return Err("PDU1 trailer magic is invalid".to_string());
    }
    let version = u16::from_le_bytes([trailer[4], trailer[5]]);
    if version != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2 {
        return Err(format!(
            "PDU1 trailer version {version} cannot be decoded by the public typed decode_units helper (expected version {COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2})"
        ));
    }
    let (units, consumed) = decode_units(&trailer[6..])?;
    if consumed != trailer.len() - 6 {
        return Err(format!(
            "PDU1 unit list left {} trailing byte(s)",
            trailer.len() - 6 - consumed
        ));
    }
    Ok(Some(units))
}

fn lane_wall_probe(units: &[DiagUnit]) -> LaneWallProbe {
    let find = |name: &str| {
        units
            .iter()
            .find(|unit| unit.name == name)
            .map(|unit| unit.value)
    };
    LaneWallProbe {
        lw_frames: find("lw_frames"),
        lw_worker_span_us: find("lw_worker_span_us"),
        lw_prewarm_us: find("lw_prewarm_us"),
        lw_hour_label_skew_abs: find("lw_hour_label_skew_abs"),
        lw_hour_label_skew_missing: find("lw_hour_label_skew_missing"),
        lw_hour_label_mismatch_domains: find("lw_hour_label_mismatch_domains"),
        lw_bracketed_domains: find("lw_bracketed_domains"),
        lw_hour_label_unusable: find("lw_hour_label_unusable"),
    }
}

fn account_gap(
    state: &mut AuditState,
    offset: u64,
    sequence: u64,
    domain_id: u16,
    frame_label: i32,
    gap: DomainGapRecordV1,
) {
    let reason = reason_name(gap.reason);
    *state.reason_counts.entry(reason).or_default() += 1;
    // A delta is only meaningful when a clock was actually observed. Zero is
    // the wire's ABSENT-witness value -- `ClockUnreadable` and `ReaderSilent`
    // are required to carry it -- and subtracting the label from it produced a
    // `-frame_label` entry that reads as an enormous clock excursion.
    let clock_derived = is_clock_derived(gap.reason) && gap.tick_before != 0 && gap.tick_after != 0;
    let opening_delta = witness_delta(gap.tick_before, frame_label);
    let closing_delta = witness_delta(gap.tick_after, frame_label);
    if clock_derived {
        *state
            .witness_delta_counts
            .entry((reason, opening_delta, closing_delta))
            .or_default() += 1;
        let example = WitnessExample {
            offset,
            sequence,
            domain_id,
            reason,
            frame_label,
            opening_witness: gap.tick_before,
            closing_witness: gap.tick_after,
            opening_delta,
            closing_delta,
        };
        if state.witness_examples.len() < GAP_EXAMPLE_LIMIT {
            state.witness_examples.push(example);
        } else {
            state.witness_examples_suppressed = state.witness_examples_suppressed.saturating_add(1);
        }
    }
    state.current.gaps.push(FrameGap {
        offset,
        sequence,
        domain_id,
        reason,
        opening_witness: clock_derived.then_some(gap.tick_before),
        closing_witness: clock_derived.then_some(gap.tick_after),
        opening_delta: clock_derived.then_some(opening_delta),
        closing_delta: clock_derived.then_some(closing_delta),
    });
}

fn finalize_frame(state: &mut AuditState) {
    let frame = std::mem::take(&mut state.current);
    let sequence_start = frame.sequence_start.expect("nonempty frame at FRAME_END");
    let label = frame.label.expect("nonempty frame has a label");
    state.total_frames = state.total_frames.saturating_add(1);

    if let Some(probe) = frame.lane_wall.as_ref() {
        state.frames_with_probe = state.frames_with_probe.saturating_add(1);
        if let Some(skew) = probe.lw_hour_label_skew_abs {
            *state.skew_distribution.entry(skew).or_default() += 1;
        }
        if probe.lw_hour_label_skew_missing == Some(1) {
            state.skew_missing_frames = state.skew_missing_frames.saturating_add(1);
        }
        if let Some(value) = probe.lw_worker_span_us {
            state.worker_spans.push(value);
        }
        if let Some(value) = probe.lw_prewarm_us {
            state.prewarm_times.push(value);
        }
    }

    let torn = frame.gaps.iter().any(|gap| gap.reason == "Torn");
    // NOT "label ahead". The producer stores `current.abs_diff(label)`, so the
    // wire cannot tell a clock that is one hour AHEAD of the label from one an
    // hour BEHIND it -- both encode 1. The classification is a skew, and
    // calling it a direction would be a claim the capture does not carry.
    let label_skew = frame
        .lane_wall
        .as_ref()
        .and_then(|probe| probe.lw_hour_label_skew_abs)
        .is_some_and(|skew| skew > 0);
    // The exact instrument, when the capture is new enough to carry it. This is
    // the number the whole tool is a substitute for: unlike the skew above it
    // is per bracket rather than per frame, and unlike a Torn witness it does
    // not need a second boundary to become visible.
    // ALL THREE OR NONE. The numerator, the denominator and the applicability
    // flag are one statement about one frame, and summing them from different
    // presence sets builds a ratio out of parts of different populations: a
    // probe carrying only the denominator would print "0 of 9" under a "NOT
    // PRESENT" banner, and one carrying only the numerator would print "N of
    // 0". The producer emits the three together, but nothing on the wire forces
    // it to, so the reader requires it and counts a frame that breaks the
    // contract as a parse error rather than folding it in.
    // Whether the exact instrument says THIS frame mislabelled anything. `None`
    // when the capture predates the instrument or the probe is incomplete --
    // which is not `false`, and is why the classification below treats it as
    // "no witness available" rather than folding it into the clean cell.
    let mut exact_mismatch = None;
    if let Some(probe) = frame.lane_wall.as_ref() {
        match (
            probe.lw_hour_label_mismatch_domains,
            probe.lw_bracketed_domains,
            probe.lw_hour_label_unusable,
        ) {
            (Some(domains), Some(bracketed), Some(unusable)) => {
                state.exact_instrument_frames = state.exact_instrument_frames.saturating_add(1);
                state.exact_mismatch_domains =
                    state.exact_mismatch_domains.saturating_add(domains);
                state.exact_bracketed_domains =
                    state.exact_bracketed_domains.saturating_add(bracketed);
                *state.exact_mismatch_distribution.entry(domains).or_default() += 1;
                if domains > 0 {
                    state.exact_mismatch_frames = state.exact_mismatch_frames.saturating_add(1);
                    // WHOLLY means every bracketed domain, not "a lot". The
                    // comparison is against this frame's own denominator
                    // because a frame does not always bracket the same number
                    // of domains -- a disabled or unscheduled domain holds no
                    // bracket, so a fixed 68 would misclassify the moment the
                    // roster changes.
                    if domains >= bracketed && bracketed > 0 {
                        state.exact_wholly_mislabelled_frames =
                            state.exact_wholly_mislabelled_frames.saturating_add(1);
                    } else {
                        state.exact_partially_mislabelled_frames =
                            state.exact_partially_mislabelled_frames.saturating_add(1);
                    }
                }
                if unusable != 0 {
                    state.exact_unusable_label_frames =
                        state.exact_unusable_label_frames.saturating_add(1);
                }
                exact_mismatch = Some(domains > 0);
            }
            (None, None, None) => {}
            _ => {
                state.exact_instrument_partial_frames =
                    state.exact_instrument_partial_frames.saturating_add(1);
            }
        }
    }
    // `None` STAYS `None` HERE. `exact_mismatch.unwrap_or(false)` was the whole
    // defect: it turned "this frame carries no exact instrument, or carries an
    // incomplete one" into "this frame was checked and mislabelled nothing", and
    // the `(false, false, false)` arm then filed it under `neither` -- the
    // clean cell. The comment thirty lines above already said `None` "is not
    // `false`"; the code disagreed. An uncheckable frame goes to the unknown
    // row, where it cannot inflate the clean count the refusal policy was
    // priced against.
    match (exact_mismatch, label_skew, torn) {
        (Some(false), false, false) => state.classification.neither += 1,
        (Some(false), true, false) => state.classification.label_skew_only += 1,
        (Some(false), false, true) => state.classification.torn_witness_only += 1,
        (Some(false), true, true) => state.classification.both += 1,
        (Some(true), false, false) => {
            state.classification.mismatch_only += 1;
            state.classification.mismatch_with_no_other_witness += 1;
        }
        (Some(true), false, true) => state.classification.mismatch_and_torn += 1,
        (Some(true), true, false) => state.classification.mismatch_and_label_skew += 1,
        (Some(true), true, true) => state.classification.mismatch_and_label_skew_and_torn += 1,
        (None, false, false) => state.classification.mismatch_unknown_neither += 1,
        (None, true, false) => state.classification.mismatch_unknown_label_skew_only += 1,
        (None, false, true) => state.classification.mismatch_unknown_torn_witness_only += 1,
        (None, true, true) => state.classification.mismatch_unknown_both += 1,
    }
    if torn {
        state.torn_frames = state.torn_frames.saturating_add(1);
        state.data_in_torn_frames = state.data_in_torn_frames.saturating_add(frame.data_count);
    }

    let gap_reasons = frame
        .gaps
        .iter()
        .map(|gap| gap.reason)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    state.frame_summaries.push(FrameSummary {
        sequence_start,
        sequence_end: frame.sequence_end,
        label,
        domain_data_records: frame.data_count,
        domain_gap_records: frame.gap_count,
        gap_reasons,
    });

    // `mismatch` joins the gate for the reason the axis was added: a frame that
    // mislabelled 67 of 68 domain-hours and happened to tear nothing was not
    // listed here at all, so the ONE frame a reader most needs to see was the
    // one the report omitted.
    if label_skew || torn || exact_mismatch == Some(true) {
        let notable = NotableFrame {
            sequence_start,
            sequence_end: frame.sequence_end,
            label,
            skew_abs: frame
                .lane_wall
                .as_ref()
                .and_then(|probe| probe.lw_hour_label_skew_abs),
            mismatch_domains: frame
                .lane_wall
                .as_ref()
                .and_then(|probe| probe.lw_hour_label_mismatch_domains),
            bracketed_domains: frame
                .lane_wall
                .as_ref()
                .and_then(|probe| probe.lw_bracketed_domains),
            worker_span_us: frame
                .lane_wall
                .as_ref()
                .and_then(|probe| probe.lw_worker_span_us),
            prewarm_us: frame
                .lane_wall
                .as_ref()
                .and_then(|probe| probe.lw_prewarm_us),
            gaps: frame.gaps.clone(),
            domain_data_records: frame.data_count,
        };
        if state.notable_frames.len() < NOTABLE_FRAME_LIMIT {
            state.notable_frames.push(notable);
        } else {
            state.notable_suppressed = state.notable_suppressed.saturating_add(1);
        }
    }

    if label == TARGET_FRAME_LABEL {
        state.target_frames.push(TargetFrame {
            sequence_start,
            sequence_end: frame.sequence_end,
            records: frame.records,
        });
    }
}

fn finish_report(input: &Path, mut state: AuditState) -> AuditReport {
    let worker_stats = stats(&mut state.worker_spans);
    let prewarm_stats = stats(&mut state.prewarm_times);
    AuditReport {
        input_path: input.display().to_string(),
        bytes_consumed: state.bytes_consumed,
        records_seen: state.records_seen,
        parse_error_count: state.errors.len(),
        parse_errors: state.errors,
        section_a_frame_census: FrameCensus {
            total_frames: state.total_frames,
            frames_with_lane_wall_probe: state.frames_with_probe,
            lw_hour_label_skew_abs_distribution: state
                .skew_distribution
                .into_iter()
                .map(|(value, count)| ValueCount { value, count })
                .collect(),
            frames_with_lw_hour_label_skew_missing_eq_1: state.skew_missing_frames,
            lw_worker_span_us: worker_stats,
            lw_prewarm_us: prewarm_stats,
            exact_instrument: ExactInstrument {
                frames_carrying_it: state.exact_instrument_frames,
                frames_with_a_mismatch: state.exact_mismatch_frames,
                mismatched_domain_hours: state.exact_mismatch_domains,
                bracketed_domain_hours: state.exact_bracketed_domains,
                unusable_label_frames: state.exact_unusable_label_frames,
                partial_frames: state.exact_instrument_partial_frames,
                mismatched_domain_hours_per_frame: state
                    .exact_mismatch_distribution
                    .into_iter()
                    .map(|(value, count)| ValueCount { value, count })
                    .collect(),
                wholly_mislabelled_frames: state.exact_wholly_mislabelled_frames,
                partially_mislabelled_frames: state.exact_partially_mislabelled_frames,
            },
        },
        section_b_domain_gaps: GapCensus {
            reason_counts: DomainGapReason::ALL
                .into_iter()
                .map(|reason| ReasonCount {
                    reason: reason_name(reason),
                    count: state
                        .reason_counts
                        .get(reason_name(reason))
                        .copied()
                        .unwrap_or(0),
                })
                .collect(),
            undecoded_gap_records: state.undecoded_gaps,
            clock_witness_delta_counts: state
                .witness_delta_counts
                .into_iter()
                .map(
                    |((reason, opening_delta, closing_delta), count)| WitnessDeltaCount {
                        reason,
                        opening_delta,
                        closing_delta,
                        count,
                    },
                )
                .collect(),
            clock_witness_examples: state.witness_examples,
            clock_witness_examples_suppressed: state.witness_examples_suppressed,
        },
        section_c_per_frame: PerFrameSection {
            frames: state.frame_summaries,
            frames_with_torn_gap: state.torn_frames,
            domain_data_records_in_frames_with_torn_gap: state.data_in_torn_frames,
        },
        section_d_classification: ClassificationSection {
            table: state.classification,
            notable_frames: state.notable_frames,
            notable_frames_suppressed: state.notable_suppressed,
        },
        section_e_target_label: TargetLabelSection {
            label: TARGET_FRAME_LABEL,
            frames: state.target_frames,
        },
    }
}

fn stats(values: &mut [u64]) -> Stats {
    if values.is_empty() {
        return Stats {
            count: 0,
            min: None,
            median: None,
            mean: None,
            max: None,
        };
    }
    values.sort_unstable();
    let len = values.len();
    let median = if len % 2 == 1 {
        values[len / 2] as f64
    } else {
        (values[len / 2 - 1] as f64 + values[len / 2] as f64) / 2.0
    };
    let sum = values.iter().map(|value| *value as f64).sum::<f64>();
    Stats {
        count: len as u64,
        min: values.first().copied(),
        median: Some(median),
        mean: Some(sum / len as f64),
        max: values.last().copied(),
    }
}

fn print_report(report: &AuditReport) -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    writeln!(out, "hour_label_audit")?;
    writeln!(out, "input: {}", report.input_path)?;
    writeln!(out, "bytes consumed: {}", report.bytes_consumed)?;
    writeln!(out, "records seen: {}", report.records_seen)?;
    writeln!(out, "parse/decode errors: {}", report.parse_error_count)?;
    for error in &report.parse_errors {
        writeln!(
            out,
            "  offset {}: {}{}",
            error.offset,
            error.error,
            if error.framing_lost {
                " [framing lost; scan stopped]"
            } else {
                ""
            }
        )?;
    }

    let a = &report.section_a_frame_census;
    writeln!(out, "\nA. Frame census")?;
    writeln!(out, "total frames: {}", a.total_frames)?;
    writeln!(
        out,
        "frames with domain-250 lane-wall probe: {}",
        a.frames_with_lane_wall_probe
    )?;
    writeln!(out, "lw_hour_label_skew_abs distribution:")?;
    if a.lw_hour_label_skew_abs_distribution.is_empty() {
        writeln!(out, "  (none)")?;
    }
    for row in &a.lw_hour_label_skew_abs_distribution {
        writeln!(out, "  {} -> {}", row.value, row.count)?;
    }
    writeln!(
        out,
        "frames with lw_hour_label_skew_missing == 1: {}",
        a.frames_with_lw_hour_label_skew_missing_eq_1
    )?;
    print_stats(&mut out, "lw_worker_span_us", &a.lw_worker_span_us)?;
    print_stats(&mut out, "lw_prewarm_us", &a.lw_prewarm_us)?;
    let e = &a.exact_instrument;
    writeln!(out, "exact hour-label instrument:")?;
    if e.frames_carrying_it == 0 {
        writeln!(
            out,
            "  NOT PRESENT: the recorder that wrote this capture predates \
             lw_hour_label_mismatch_domains, so the counts below are ABSENT, not zero"
        )?;
    }
    writeln!(out, "  frames carrying it: {}", e.frames_carrying_it)?;
    writeln!(out, "  frames with a mismatch: {}", e.frames_with_a_mismatch)?;
    writeln!(
        out,
        "  mismatched domain-hours: {} of {} bracketed",
        e.mismatched_domain_hours, e.bracketed_domain_hours
    )?;
    writeln!(
        out,
        "  frames whose label the calendar refused (rule not applicable): {}",
        e.unusable_label_frames
    )?;
    writeln!(
        out,
        "  wholly mislabelled frames (every bracketed domain): {}",
        e.wholly_mislabelled_frames
    )?;
    writeln!(
        out,
        "  partially mislabelled frames: {}",
        e.partially_mislabelled_frames
    )?;
    writeln!(out, "  mislabelled domain-hours per frame -> frames:")?;
    if e.mismatched_domain_hours_per_frame.is_empty() {
        writeln!(out, "    (none)")?;
    }
    for row in &e.mismatched_domain_hours_per_frame {
        writeln!(out, "    {} -> {}", row.value, row.count)?;
    }
    if e.partial_frames != 0 {
        writeln!(
            out,
            "  WARNING: {} frame(s) carried only some of the three units; the              counts above are NOT a rate over a described population",
            e.partial_frames
        )?;
    }

    let b = &report.section_b_domain_gaps;
    writeln!(out, "\nB. DOMAIN_GAP census")?;
    writeln!(out, "reason -> count:")?;
    for row in &b.reason_counts {
        writeln!(out, "  {} -> {}", row.reason, row.count)?;
    }
    writeln!(
        out,
        "undecoded DOMAIN_GAP records: {}",
        b.undecoded_gap_records
    )?;
    writeln!(
        out,
        "clock witness deltas (reason, opening-label, closing-label) -> count:"
    )?;
    if b.clock_witness_delta_counts.is_empty() {
        writeln!(out, "  (none)")?;
    }
    for row in &b.clock_witness_delta_counts {
        writeln!(
            out,
            "  ({}, {:+}, {:+}) -> {}",
            row.reason, row.opening_delta, row.closing_delta, row.count
        )?;
    }
    writeln!(
        out,
        "raw clock-witness examples (up to {GAP_EXAMPLE_LIMIT}):"
    )?;
    if b.clock_witness_examples.is_empty() {
        writeln!(out, "  (none)")?;
    }
    for example in &b.clock_witness_examples {
        writeln!(
            out,
            "  offset={} seq={} d{} {} (label={}, opening={}, closing={}) deltas=({:+},{:+})",
            example.offset,
            example.sequence,
            example.domain_id,
            example.reason,
            example.frame_label,
            example.opening_witness,
            example.closing_witness,
            example.opening_delta,
            example.closing_delta
        )?;
    }
    writeln!(
        out,
        "clock-witness examples suppressed: {}",
        b.clock_witness_examples_suppressed
    )?;

    let c = &report.section_c_per_frame;
    writeln!(out, "\nC. Per-frame outcomes")?;
    for frame in &c.frames {
        writeln!(
            out,
            "  seq={}..{} label={} data={} gaps={} reasons={}",
            frame.sequence_start,
            frame.sequence_end,
            frame.label,
            frame.domain_data_records,
            frame.domain_gap_records,
            join_names(&frame.gap_reasons)
        )?;
    }
    writeln!(
        out,
        "frames containing at least one Torn gap: {}",
        c.frames_with_torn_gap
    )?;
    writeln!(
        out,
        "DOMAIN_DATA records published within Torn frames: {}",
        c.domain_data_records_in_frames_with_torn_gap
    )?;

    let d = &report.section_d_classification;
    writeln!(
        out,
        "\nD. Exact-mismatch x Label-skew x Torn-witness classification"
    )?;
    writeln!(
        out,
        "(skew is |current - label|, so this is a DISAGREEMENT, not a direction)"
    )?;
    writeln!(
        out,
        "(mismatch is lw_hour_label_mismatch_domains > 0 -- the per-bracket \
         instrument, which needs no luck to fire)"
    )?;
    writeln!(out, "                                      torn=no  torn=yes")?;
    writeln!(
        out,
        "mismatch=no   label_skew=no   {:>12} {:>9}",
        d.table.neither, d.table.torn_witness_only
    )?;
    writeln!(
        out,
        "mismatch=no   label_skew=yes  {:>12} {:>9}",
        d.table.label_skew_only, d.table.both
    )?;
    writeln!(
        out,
        "mismatch=yes  label_skew=no   {:>12} {:>9}",
        d.table.mismatch_only, d.table.mismatch_and_torn
    )?;
    writeln!(
        out,
        "mismatch=yes  label_skew=yes  {:>12} {:>9}",
        d.table.mismatch_and_label_skew, d.table.mismatch_and_label_skew_and_torn
    )?;
    writeln!(
        out,
        "mismatch=unknown             {:>12} {:>9}",
        d.table.mismatch_unknown_neither, d.table.mismatch_unknown_torn_witness_only
    )?;
    writeln!(
        out,
        "  (label_skew=yes)           {:>12} {:>9}",
        d.table.mismatch_unknown_label_skew_only, d.table.mismatch_unknown_both
    )?;
    writeln!(
        out,
        "frames a label_skew x torn view alone would call clean while they \
         mislabelled domain-hours: {}",
        d.table.mismatch_with_no_other_witness
    )?;
    writeln!(out, "notable frames (up to {NOTABLE_FRAME_LIMIT}):")?;
    if d.notable_frames.is_empty() {
        writeln!(out, "  (none)")?;
    }
    for frame in &d.notable_frames {
        let gaps = frame
            .gaps
            .iter()
            .map(render_frame_gap)
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            out,
            "  seq={}..{} label={} mismatch={}/{} skew_abs={} worker_span_us={} prewarm_us={} gaps={} data={}",
            frame.sequence_start,
            frame.sequence_end,
            frame.label,
            optional_u64(frame.mismatch_domains),
            optional_u64(frame.bracketed_domains),
            optional_u64(frame.skew_abs),
            optional_u64(frame.worker_span_us),
            optional_u64(frame.prewarm_us),
            if gaps.is_empty() { "none" } else { &gaps },
            frame.domain_data_records
        )?;
    }
    writeln!(
        out,
        "notable frame lines suppressed: {}",
        d.notable_frames_suppressed
    )?;

    let e = &report.section_e_target_label;
    writeln!(out, "\nE. Every frame labelled {}", e.label)?;
    if e.frames.is_empty() {
        writeln!(out, "  (none)")?;
    }
    for (index, frame) in e.frames.iter().enumerate() {
        writeln!(
            out,
            "frame {} seq={}..{} records={}",
            index + 1,
            frame.sequence_start,
            frame.sequence_end,
            frame.records.len()
        )?;
        for record in &frame.records {
            writeln!(
                out,
                "  offset={} seq={} label={} type={}({}) domain={} payload_len={} flags={:#010x}",
                record.offset,
                record.sequence,
                record.game_hour_raw,
                record.record_type_name,
                record.record_type,
                record.domain_id,
                record.payload_len,
                record.record_flags
            )?;
            if let Some(gap) = &record.gap {
                writeln!(
                    out,
                    "    gap part={} reason={} target={} range={}..={} opening={} closing={} queue(capacity={},depth={},dropped={}) ordinal={}",
                    gap.part,
                    gap.reason,
                    gap.target_hour_raw,
                    gap.range_start_hour_raw,
                    gap.range_end_hour_raw,
                    gap.tick_before,
                    gap.tick_after,
                    gap.queue_capacity,
                    gap.queue_depth,
                    gap.queue_dropped,
                    gap.outcome_ordinal
                )?;
            }
            if let Some(probe) = &record.lane_wall_probe {
                writeln!(
                    out,
                    "    lane_wall lw_frames={} worker_span_us={} prewarm_us={} skew_abs={} skew_missing={}",
                    optional_u64(probe.lw_frames),
                    optional_u64(probe.lw_worker_span_us),
                    optional_u64(probe.lw_prewarm_us),
                    optional_u64(probe.lw_hour_label_skew_abs),
                    optional_u64(probe.lw_hour_label_skew_missing)
                )?;
            }
            if let Some(error) = &record.decode_error {
                writeln!(out, "    decode_error={error}")?;
            }
        }
    }
    out.flush()
}

fn print_stats(out: &mut impl Write, name: &str, stats: &Stats) -> io::Result<()> {
    match (stats.min, stats.median, stats.mean, stats.max) {
        (Some(min), Some(median), Some(mean), Some(max)) => writeln!(
            out,
            "{name}: count={} min={} median={:.3} mean={:.3} max={}",
            stats.count, min, median, mean, max
        ),
        _ => writeln!(out, "{name}: count=0 min=n/a median=n/a mean=n/a max=n/a"),
    }
}

fn render_frame_gap(gap: &FrameGap) -> String {
    match (gap.opening_delta, gap.closing_delta) {
        (Some(opening), Some(closing)) => format!(
            "{}[d{}:{:+}->{:+}]",
            gap.reason, gap.domain_id, opening, closing
        ),
        _ => format!("{}[d{}]", gap.reason, gap.domain_id),
    }
}

fn optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "missing".to_string(), |value| value.to_string())
}

fn join_names(values: &[&str]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(",")
    }
}

fn witness_delta(witness: u64, frame_label: i32) -> i128 {
    i128::from(witness) - i128::from(frame_label)
}

/// Every reason whose two tick witnesses are a real ENGINE-CLOCK OBSERVATION,
/// so comparing them against the frame's label is evidence rather than
/// arithmetic.
///
/// `HourLabelMismatch` belongs here and was missing: its entire content is an
/// observed hour that disagrees with the label, and it would have been dropped
/// from this table silently -- invisible in t17 only because t17 predates the
/// reason, so the tool would have begun under-reporting the moment it became
/// useful.
///
/// `WorkerPanic` does not belong here either, for the same reason and found the
/// same way: d81 maps its own panic path to that reason and fills both ticks
/// from the label, so the reason does not establish provenance -- even though
/// the ORDINARY panic path, `ReaderClockBracketGuard`'s drop during the unwind,
/// does carry real bracket reads. Nothing on the wire separates the two
/// producers, and the nonzero gate cannot: a label copy is nonzero.
///
/// `SourceUnavailable` does NOT belong here, and briefly did. Its only producer
/// is d81, which fills BOTH ticks from `game_hour_raw` -- the frame label
/// itself -- and never consults its bracket reads. Every such row is therefore
/// `(+0, +0)` by construction and can be nothing else. t17's 501 of them are a
/// tautology, not 501 observations that the reader ran in its labelled hour,
/// and presenting them beside the Torn witnesses would have manufactured
/// corroboration this capture does not contain.
///
/// The payload grammar cannot rescue it either: `SOURCE_UNAVAILABLE` requires
/// only that the two ticks are EQUAL, which a label copy satisfies, so nothing
/// on the wire distinguishes an observed hour from a restated label for that
/// reason. `ReaderSilent` and `ClockUnreadable` are required to carry zeros,
/// and `Overrun`/`Backpressure` carry a coalesced RANGE rather than one frame's
/// observation, so a delta against a single label is meaningless for all four.
fn is_clock_derived(reason: DomainGapReason) -> bool {
    matches!(
        reason,
        DomainGapReason::Torn | DomainGapReason::HourLabelMismatch
    )
}

fn reason_name(reason: DomainGapReason) -> &'static str {
    match reason {
        DomainGapReason::Torn => "Torn",
        DomainGapReason::Overrun => "Overrun",
        DomainGapReason::Backpressure => "Backpressure",
        DomainGapReason::ClockUnreadable => "ClockUnreadable",
        DomainGapReason::WorkerPanic => "WorkerPanic",
        DomainGapReason::SourceUnavailable => "SourceUnavailable",
        DomainGapReason::ReaderSilent => "ReaderSilent",
        DomainGapReason::HourLabelMismatch => "HourLabelMismatch",
    }
}

fn gap_record_detail(gap: DomainGapRecordV1) -> GapRecordDetail {
    GapRecordDetail {
        part: match gap.part {
            compact_contract::DomainGapPart::State => "State",
            compact_contract::DomainGapPart::Event => "Event",
            compact_contract::DomainGapPart::Both => "Both",
        },
        reason: reason_name(gap.reason),
        target_hour_raw: gap.target_hour_raw,
        range_start_hour_raw: gap.range_start_hour_raw,
        range_end_hour_raw: gap.range_end_hour_raw,
        tick_before: gap.tick_before,
        tick_after: gap.tick_after,
        queue_capacity: gap.queue_capacity,
        queue_depth: gap.queue_depth,
        queue_dropped: gap.queue_dropped,
        outcome_ordinal: gap.outcome_ordinal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::diagnostic_channel::{encode_units, DiagUnit};
    use compact_contract::{
        COMPACT_DOMAIN_FRAME, COMPACT_DOMAIN_PERF_BASE_LEN,
        COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC, COMPACT_RECORD_MAGIC, COMPACT_RECORD_VERSION,
    };

    /// A real t21 label, so the calendar admits it and `lw_hour_label_unusable`
    /// stays 0 -- a rejected label would suppress the very rule under test.
    const HOUR_A: i32 = 60_831_056;
    const HOUR_B: i32 = 60_831_057;

    fn record(sequence: u64, hour: i32, domain: u16, rtype: u16, flags: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&rtype.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn lane_wall_payload(units: &[DiagUnit]) -> Vec<u8> {
        let mut payload = vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN];
        payload.extend_from_slice(COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
        payload.extend_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2.to_le_bytes());
        payload.extend_from_slice(&encode_units(units).expect("units encode"));
        payload
    }

    fn probe_units(exact: Option<(u64, u64)>, skew_abs: u64) -> Vec<DiagUnit> {
        let mut units = vec![
            DiagUnit::counter("lw_frames", 1),
            DiagUnit::counter("lw_worker_span_us", 57_554),
            DiagUnit::counter("lw_prewarm_us", 23_641),
            DiagUnit::counter("lw_hour_label_skew_abs", skew_abs),
            DiagUnit::counter("lw_hour_label_skew_missing", 0),
        ];
        if let Some((mismatch, bracketed)) = exact {
            units.push(DiagUnit::counter("lw_hour_label_mismatch_domains", mismatch));
            units.push(DiagUnit::counter("lw_bracketed_domains", bracketed));
            units.push(DiagUnit::counter("lw_hour_label_unusable", 0));
        }
        units
    }

    /// One frame: its lane-wall probe, then FRAME_END. No gap records, so the
    /// ONLY thing that can make the frame notable is the exact instrument.
    fn frame(sequence: &mut u64, hour: i32, exact: Option<(u64, u64)>, skew_abs: u64) -> Vec<u8> {
        let mut out = record(
            *sequence,
            hour,
            RC_PROBE_DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &lane_wall_payload(&probe_units(exact, skew_abs)),
        );
        *sequence += 1;
        out.extend_from_slice(&record(
            *sequence,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; 32],
        ));
        *sequence += 1;
        out
    }

    fn audit(stream: &[u8]) -> AuditReport {
        let mut state = AuditState::default();
        scan_stream(&mut &stream[..], &mut state).expect("scan");
        finish_report(Path::new("synthetic"), state)
    }

    /// THE REGRESSION. A frame that mislabelled 67 of its 68 domain-hours, tore
    /// nothing, and started its sweep in its own hour (`skew_abs == 0`, which is
    /// what t21 measured on all 315 probe-carrying frames) must not land in the
    /// same cell as a clean frame, and must be listed.
    ///
    /// MUTATION RUN: restore the classification to `match (label_skew, torn)`
    /// and drop `mismatch` from the notable gate -- i.e. the code as it stood
    /// before this lane. Observed failure:
    ///   `the mislabelled frame was counted clean: neither=2` (left 2, right 1)
    /// and, with only the notable gate reverted:
    ///   `the mislabelled frame was not listed` (left 0, right 1).
    #[test]
    fn a_mislabelled_frame_with_no_tear_is_neither_counted_clean_nor_omitted() {
        let mut sequence = 0u64;
        let mut stream = frame(&mut sequence, HOUR_A, Some((67, 68)), 0);
        stream.extend_from_slice(&frame(&mut sequence, HOUR_B, Some((0, 68)), 0));
        let report = audit(&stream);

        assert_eq!(report.parse_error_count, 0, "{:?}", report.parse_errors);
        assert_eq!(report.section_a_frame_census.total_frames, 2);
        let table = &report.section_d_classification.table;
        assert_eq!(
            table.neither, 1,
            "the mislabelled frame was counted clean: neither={}",
            table.neither
        );
        assert_eq!(table.mismatch_only, 1);
        assert_eq!(table.mismatch_with_no_other_witness, 1);
        assert_eq!(table.torn_witness_only, 0);
        assert_eq!(table.mismatch_and_torn, 0);

        let notable = &report.section_d_classification.notable_frames;
        assert_eq!(notable.len(), 1, "the mislabelled frame was not listed");
        assert_eq!(notable[0].label, HOUR_A);
        assert_eq!(notable[0].mismatch_domains, Some(67));
        assert_eq!(notable[0].bracketed_domains, Some(68));
    }

    /// A frame the exact instrument cannot speak for is UNKNOWN, not clean.
    ///
    /// This is the t21 discrepancy in miniature: that capture had 318 frames,
    /// 315 carrying the instrument and 98 of those mismatching, so 217 frames
    /// were checked and clean -- yet `neither` read 220, because the 3
    /// probe-less frames were folded into the clean cell by
    /// `exact_mismatch.unwrap_or(false)`. The clean cell is what the refusal
    /// policy's price is read against, so padding it understates the loss.
    ///
    /// MUTATION RUN: restore `let mismatch = exact_mismatch.unwrap_or(false);`
    /// and match on `(mismatch, label_skew, torn)`. Observed failure:
    ///   `an uninstrumented frame was counted as checked-and-clean: neither=2`
    ///   (left 2, right 1).
    #[test]
    fn a_frame_without_the_exact_instrument_is_unknown_not_clean() {
        let mut sequence = 0u64;
        // One frame that WAS checked and found clean.
        let mut stream = frame(&mut sequence, HOUR_A, Some((0, 68)), 0);
        // One frame carrying no exact instrument at all: nothing checked it.
        stream.extend_from_slice(&frame(&mut sequence, HOUR_B, None, 0));
        let report = audit(&stream);

        assert_eq!(report.parse_error_count, 0, "{:?}", report.parse_errors);
        assert_eq!(report.section_a_frame_census.total_frames, 2);
        let table = &report.section_d_classification.table;
        assert_eq!(
            table.neither, 1,
            "an uninstrumented frame was counted as checked-and-clean: neither={}",
            table.neither
        );
        assert_eq!(
            table.mismatch_unknown_neither, 1,
            "the uninstrumented frame was not reported as unknown"
        );
        assert_eq!(table.mismatch_only, 0);
        assert_eq!(table.torn_witness_only, 0);
        // Every frame is still accounted for exactly once.
        assert_eq!(
            table.neither
                + table.label_skew_only
                + table.torn_witness_only
                + table.both
                + table.mismatch_only
                + table.mismatch_and_torn
                + table.mismatch_and_label_skew
                + table.mismatch_and_label_skew_and_torn
                + table.mismatch_unknown_neither
                + table.mismatch_unknown_label_skew_only
                + table.mismatch_unknown_torn_witness_only
                + table.mismatch_unknown_both,
            report.section_a_frame_census.total_frames,
            "the classification cells no longer partition the frames"
        );
    }

    /// MUTATION RUN: change `domains >= bracketed` to `domains > bracketed` in
    /// `finalize_frame`. Observed failure:
    ///   `assertion `left == right` failed: a frame that mislabelled every
    ///    bracketed domain was called partial` left: 0 right: 1.
    #[test]
    fn wholly_and_partially_mislabelled_frames_are_told_apart() {
        let mut sequence = 0u64;
        let mut stream = frame(&mut sequence, HOUR_A, Some((68, 68)), 0);
        stream.extend_from_slice(&frame(&mut sequence, HOUR_B, Some((8, 68)), 0));
        let report = audit(&stream);
        let exact = &report.section_a_frame_census.exact_instrument;

        assert_eq!(
            exact.wholly_mislabelled_frames, 1,
            "a frame that mislabelled every bracketed domain was called partial"
        );
        assert_eq!(exact.partially_mislabelled_frames, 1);
        assert_eq!(exact.frames_with_a_mismatch, 2);
        let distribution: Vec<(u64, u64)> = exact
            .mismatched_domain_hours_per_frame
            .iter()
            .map(|row| (row.value, row.count))
            .collect();
        assert_eq!(distribution, vec![(8, 1), (68, 1)]);
    }

    /// The instrument being ABSENT is not the instrument saying zero -- and it
    /// is not the instrument saying one either. A capture from a recorder that
    /// predates the instrument must be reported as UNCHECKED.
    ///
    /// This test asserted `neither == 1` when it was written, i.e. it pinned the
    /// absent frame into the CLEAN cell. That was the same conflation from the
    /// other side: "no evidence" was being reported as "evidence of no
    /// mismatch", which is what let 3 probe-less t21 frames pad `neither` from
    /// 217 to 220. The claim the test exists to make -- absent is not a
    /// mismatch -- is unchanged and is asserted below.
    ///
    /// MUTATION RUN: make `mismatch` default to `true` when the instrument is
    /// absent (`exact_mismatch.unwrap_or(true)` with the old two-valued match).
    /// Observed failure:
    ///   `assertion `left == right` failed: a frame with no exact instrument
    ///    was classified as mismatching` left: 0 right: 1.
    #[test]
    fn an_absent_exact_instrument_is_not_read_as_a_mismatch() {
        let mut sequence = 0u64;
        let stream = frame(&mut sequence, HOUR_A, None, 0);
        let report = audit(&stream);
        let table = &report.section_d_classification.table;

        assert_eq!(
            table.mismatch_unknown_neither, 1,
            "a frame with no exact instrument was classified as mismatching"
        );
        assert_eq!(
            table.neither, 0,
            "a frame with no exact instrument was counted as checked-and-clean"
        );
        assert_eq!(table.mismatch_only, 0);
        assert_eq!(
            report
                .section_a_frame_census
                .exact_instrument
                .frames_carrying_it,
            0
        );
        assert!(report.section_d_classification.notable_frames.is_empty());
    }

    /// The old two axes still work and still occupy their own cells -- the fix
    /// ADDS an axis, and a frame whose skew really is nonzero must not be
    /// swallowed by the new one.
    #[test]
    fn a_nonzero_label_skew_still_has_its_own_cell() {
        let mut sequence = 0u64;
        let mut stream = frame(&mut sequence, HOUR_A, Some((0, 68)), 1);
        stream.extend_from_slice(&frame(&mut sequence, HOUR_B, Some((4, 68)), 1));
        let report = audit(&stream);
        let table = &report.section_d_classification.table;

        assert_eq!(table.label_skew_only, 1);
        assert_eq!(table.mismatch_and_label_skew, 1);
        assert_eq!(table.mismatch_with_no_other_witness, 0);
        assert_eq!(table.neither, 0);
    }
}

fn record_type_name(record_type: u16) -> &'static str {
    match record_type {
        COMPACT_RECORD_TYPE_DOMAIN_PERF => "DOMAIN_PERF",
        COMPACT_RECORD_TYPE_FRAME_END => "FRAME_END",
        COMPACT_RECORD_TYPE_DOMAIN_DATA => "DOMAIN_DATA",
        COMPACT_RECORD_TYPE_STREAM_HEADER => "STREAM_HEADER",
        COMPACT_RECORD_TYPE_DOMAIN_GAP => "DOMAIN_GAP",
        COMPACT_RECORD_TYPE_FRAME_REFUSED => "FRAME_REFUSED",
        COMPACT_RECORD_TYPE_FRAME_DEGRADED => "FRAME_DEGRADED",
        COMPACT_RECORD_TYPE_CAPTURE_TERMINATED => "CAPTURE_TERMINATED",
        // The two clock-recovery records. Both were reaching this audit as
        // "UNKNOWN", which is the least helpful thing an hour-label audit can
        // say about a record that exists precisely to explain an hour gap.
        compact_contract::COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL => {
            "CLOCK_UNREADABLE_INTERVAL"
        }
        compact_contract::COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY => {
            "UNTRUSTED_CLOCK_RECOVERY"
        }
        _ => "UNKNOWN",
    }
}
