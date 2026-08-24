use compact_contract::campaign_identity::{parse_cid1_record, COMPACT_RECORD_TYPE_STREAM_HEADER};
use compact_contract::diagnostic_channel::{decode_units, DiagUnit};
use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, DomainGapReason,
    DomainGapRecordV1, DomainGapWitnessProvenance, DomainRepeatBaselines,
    COMPACT_DOMAIN_DATA_REPEAT_KEYFRAME_CADENCE_HOURS, COMPACT_DOMAIN_PERF_BASE_LEN,
    COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC, COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2,
    COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_CAPTURE_TERMINATED, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_GAP, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_DEGRADED, COMPACT_RECORD_TYPE_FRAME_END,
    COMPACT_RECORD_TYPE_FRAME_REFUSED,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

const RC_PROBE_DOMAIN_ID: u16 = 250;
/// Cap on `--units-domain` rows held in memory. Generous next to any capture
/// this tool is pointed at (a 313-frame capture asking for one domain produces
/// 313 rows), and never silent: what does not fit is counted in
/// `rows_suppressed`.
const UNIT_SERIES_LIMIT: usize = 100_000;
const TARGET_FRAME_LABEL: i32 = 60_832_037;
const GAP_EXAMPLE_LIMIT: usize = 50;
const NOTABLE_FRAME_LIMIT: usize = 200;
const REPEAT_ERROR_EXAMPLE_LIMIT: usize = 50;

/// Mirror of collector's `gateable_domain` (domain_repeat_gate.rs): the ids
/// that never carry gated data records and are never keyed by the repeat
/// gate. Used to decide whether a named DOMAIN_GAP forces the next record
/// of its domain full.
const fn repeat_gate_managed_domain(domain_id: u16) -> bool {
    domain_id != 0
        && domain_id != RC_PROBE_DOMAIN_ID
        && domain_id != compact_contract::COMPACT_DOMAIN_FRAME
}

/// Per-domain DOMAIN_DATA_REPEAT accounting for one stream scan.
#[derive(Default)]
struct RepeatDomainTally {
    repeat_records: u64,
    first_repeat_hour_raw: Option<i32>,
    last_repeat_hour_raw: Option<i32>,
    /// Full DOMAIN_DATA records seen for this domain (the keyframe anchors).
    full_data_records: u64,
    /// Calendar hour index of the domain's last FULL record; the cadence
    /// anchor, mirroring `last_full_hour_index` in domain_repeat_gate.rs.
    last_full_hour_index: Option<i64>,
    /// A named DOMAIN_GAP forces the next published record of the domain
    /// FULL (force-full law); a repeat while this is set is a violation.
    gap_since_full: bool,
    keyframe_cadence_violations: u64,
    repeats_after_named_gap: u64,
    /// Consecutive repeats since the domain's last FULL record (reset by
    /// every full record).
    repeat_run: u64,
    /// Longest consecutive-repeat chain seen for this domain in this stream:
    /// the offline non-vacuity measure for the keyframe law.
    longest_repeat_run: u64,
}

#[derive(Serialize)]
struct AuditReport {
    input_path: String,
    collector_build: serde_json::Value,
    bytes_consumed: u64,
    records_seen: u64,
    parse_error_count: usize,
    parse_errors: Vec<ParseError>,
    section_a_frame_census: FrameCensus,
    section_b_domain_gaps: GapCensus,
    section_c_per_frame: PerFrameSection,
    section_d_classification: ClassificationSection,
    section_e_target_label: TargetLabelSection,
    section_f_repeat_census: RepeatCensusSection,
    section_g_domain_units: DomainUnitSection,
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
    build_start_lateness: BuildStartLateness,
}

#[derive(Default, Serialize)]
struct BuildStartLateness {
    frames_carrying_it: u64,
    frames_in_joint_table: u64,
    frames_missing_it: u64,
    frames_excluded_unpaired: u64,
    frontier_ahead_distribution: Vec<ValueCount>,
    lw_hour_age_us: Stats,
    buckets: Vec<LatenessBucket>,
}

#[derive(Serialize)]
struct LatenessBucket {
    lateness_ms_from: u64,
    lateness_ms_to: u64,
    frames: u64,
    mismatched_domains_mean: f64,
    mismatched_domains_max: u64,
    bracketed_domains: u64,
}

#[derive(Default, Serialize)]
struct ExactInstrument {
    frames_carrying_it: u64,
    frames_with_a_mismatch: u64,
    mismatched_domain_hours: u64,
    bracketed_domain_hours: u64,
    unusable_label_frames: u64,
    partial_frames: u64,
    mismatched_domain_hours_per_frame: Vec<ValueCount>,
    wholly_mislabelled_frames: u64,
    partially_mislabelled_frames: u64,
}

#[derive(Serialize)]
struct ValueCount {
    value: u64,
    count: u64,
}

#[derive(Default, Serialize)]
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
    witness_provenance_counts: Vec<ProvenanceCount>,
    clock_witness_delta_counts: Vec<WitnessDeltaCount>,
    clock_witness_examples: Vec<WitnessExample>,
    clock_witness_examples_suppressed: u64,
}

#[derive(Serialize)]
struct ReasonCount {
    reason: &'static str,
    domain_hours: u64,
    distinct_hours: u64,
    count: u64,
}

#[derive(Debug, Serialize)]
struct ProvenanceCount {
    reason: &'static str,
    witness_provenance: &'static str,
    count: u64,
    witnesses_admitted: bool,
}

#[derive(Serialize)]
struct WitnessDeltaCount {
    reason: &'static str,
    witness_provenance: &'static str,
    opening_delta: i128,
    closing_delta: i128,
    count: u64,
}

#[derive(Clone, Debug, Serialize)]
struct WitnessExample {
    offset: u64,
    sequence: u64,
    domain_id: u16,
    reason: &'static str,
    witness_provenance: &'static str,
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
    mismatch_with_no_other_witness: u64,
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
    witness_provenance: &'static str,
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

/// Single-stream DOMAIN_DATA_REPEAT census (lane RQ): per-domain counts,
/// the hours repeats cover, and the shared-codec consistency verdict.
/// `baseline_errors_total` counts records where the referenced baseline was
/// missing or the recorded digest did not reconstruct (verified through
/// `compact_contract::DomainRepeatBaselines`, never a local re-derivation).
#[derive(Serialize)]
struct RepeatCensusSection {
    domain_data_repeat_records: u64,
    domains_with_repeats: u64,
    gate_excluded_repeat_records: u64,
    per_domain: Vec<RepeatDomainRow>,
    baseline_errors_total: u64,
    baseline_error_examples: Vec<ParseError>,
}

#[derive(Serialize)]
struct RepeatDomainRow {
    domain_id: u16,
    repeat_records: u64,
    first_repeat_hour_raw: Option<i32>,
    last_repeat_hour_raw: Option<i32>,
    full_data_records: u64,
    keyframe_cadence_violations: u64,
    repeats_after_named_gap: u64,
    longest_repeat_run: u64,
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

/// One decoded DOMAIN_PERF diagnostic unit, flattened for the report.
#[derive(Clone, Debug, Serialize)]
struct UnitDetail {
    name: String,
    value: u64,
    role: &'static str,
    presence: &'static str,
    /// The unit as `render()` writes it, so a reader sees `absent` and
    /// `sentinel(0x..)` rather than a bare number that means neither.
    rendered: String,
}

/// One DOMAIN_PERF record of a requested domain.
///
/// A row is emitted for EVERY matching record, including the ones that carried
/// no trailer, because a consumer building a per-frame series has to be able to
/// tell "this frame reported zero" from "this frame is not in the series".
#[derive(Clone, Debug, Serialize)]
struct UnitSeriesRow {
    offset: u64,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_flags: u32,
    /// `None` == the record carried no diagnostic trailer at all. An empty Vec
    /// == it carried one and it named nothing.
    units: Option<Vec<UnitDetail>>,
    decode_error: Option<String>,
}

/// `--units-domain`: the per-record read-out this tool could not previously
/// give for any domain but 250.
///
/// The manifest aggregates DOMAIN_PERF units into capture totals, so a
/// per-frame series of a named unit had no producer anywhere in the repo. This
/// section is that producer.
#[derive(Default, Serialize)]
struct DomainUnitSection {
    requested_domain_ids: Vec<u16>,
    perf_records_matched: u64,
    perf_records_without_trailer: u64,
    perf_records_with_decode_error: u64,
    /// Matching records skipped because the record failed semantic validation,
    /// whose payload offsets are therefore not trustworthy. Counted rather
    /// than dropped silently.
    perf_records_skipped_semantic_error: u64,
    rows: Vec<UnitSeriesRow>,
    rows_suppressed: u64,
}

#[derive(Clone, Serialize)]
struct GapRecordDetail {
    part: &'static str,
    reason: &'static str,
    witness_provenance: &'static str,
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
    lw_hour_label_mismatch_domains: Option<u64>,
    lw_bracketed_domains: Option<u64>,
    lw_hour_label_unusable: Option<u64>,
    lw_hour_age_us: Option<u64>,
    lw_hour_frontier_ahead: Option<u64>,
    lw_hour_age_missing: Option<u64>,
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

const MAX_ENUMERATED_RANGE_HOURS: u64 = 1_000_000;

#[derive(Default)]
struct AuditState {
    /// Domains whose DOMAIN_PERF units are to be read out per record. Empty
    /// unless `--units-domain` was given, and empty means the scan does no
    /// extra work at all.
    units_domains: BTreeSet<u16>,
    unit_series: Vec<UnitSeriesRow>,
    unit_series_suppressed: u64,
    unit_records_matched: u64,
    unit_records_without_trailer: u64,
    unit_records_decode_error: u64,
    unit_records_skipped_semantic_error: u64,
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
    lateness_samples: Vec<(u64, u64, u64)>,
    lateness_frames: u64,
    lateness_missing_frames: u64,
    lateness_unpaired_frames: u64,
    lateness_carrier_frames: u64,
    frontier_ahead_distribution: BTreeMap<u64, u64>,
    reason_counts: BTreeMap<&'static str, u64>,
    reason_domain_hours: BTreeMap<&'static str, u64>,
    reason_hours: BTreeMap<&'static str, BTreeSet<i32>>,
    undecoded_gaps: u64,
    provenance_counts: BTreeMap<(&'static str, &'static str), u64>,
    witness_delta_counts: BTreeMap<(&'static str, &'static str, i128, i128), u64>,
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
    domain_data_repeat_records: u64,
    repeat_tallies: BTreeMap<u16, RepeatDomainTally>,
    /// Type-14 records for ids the collector never gates (0/250/1000): not a
    /// legitimate gate emission, refused by name, never census rows.
    repeat_gate_excluded_repeat_records: u64,
    /// Shared codec (compact_contract::DomainRepeatBaselines): the SAME
    /// baseline store the collector gate and production fold keep. Feeding
    /// every semantically valid data/repeat record through it verifies, per
    /// repeat, that a published baseline exists earlier in THIS stream and
    /// that the recorded tail digest reconstructs.
    repeat_baselines: DomainRepeatBaselines,
    repeat_baseline_error_examples: Vec<ParseError>,
    repeat_baseline_errors_total: u64,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("hour_label_audit: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (input, json_output, units_domains) = parse_args()?;
    let file =
        File::open(&input).map_err(|error| format!("open input {}: {error}", input.display()))?;
    let mut reader = BufReader::new(file);
    let mut state = AuditState {
        units_domains,
        ..AuditState::default()
    };
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

fn parse_args() -> Result<(PathBuf, Option<PathBuf>, BTreeSet<u16>), String> {
    let mut args = env::args_os().skip(1);
    let input = args.next().map(PathBuf::from).ok_or_else(usage)?;
    let mut json = None;
    let mut units_domains: BTreeSet<u16> = BTreeSet::new();
    while let Some(arg) = args.next() {
        if arg == "--json" {
            if json.is_some() {
                return Err(format!("--json may be specified only once\n{}", usage()));
            }
            json = Some(
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| format!("--json requires an output path\n{}", usage()))?,
            );
            continue;
        }
        if arg == "--units-domain" {
            let value = args
                .next()
                .ok_or_else(|| format!("--units-domain requires a domain id\n{}", usage()))?;
            let value = value
                .into_string()
                .map_err(|_| format!("--units-domain value is not UTF-8\n{}", usage()))?;
            for field in value.split(',') {
                let field = field.trim();
                if field.is_empty() {
                    return Err(format!(
                        "--units-domain carries an empty domain id in {value:?}\n{}",
                        usage()
                    ));
                }
                let id: u16 = field.parse().map_err(|error| {
                    format!(
                        "--units-domain {field:?} is not a domain id: {error}\n{}",
                        usage()
                    )
                })?;
                units_domains.insert(id);
            }
            continue;
        }
        return Err(format!("unknown argument {:?}\n{}", arg, usage()));
    }
    Ok((input, json, units_domains))
}

fn usage() -> String {
    "usage: hour_label_audit <collector_stream.bin> [--json out.json] \
     [--units-domain <id>[,<id>...]]"
        .to_string()
}

fn unit_detail(unit: &DiagUnit) -> UnitDetail {
    UnitDetail {
        name: unit.name.clone(),
        value: unit.value,
        role: unit.role.label(),
        presence: unit.presence.label(),
        rendered: unit.render(),
    }
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

    if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
        || header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
    {
        // A repeat is a data record of a new kind and counts toward the
        // hour's data tally exactly like a full record.
        state.current.data_count = state.current.data_count.saturating_add(1);
        if header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT {
            // Census rows exist only for gate-managed domains; a type-14
            // record for an excluded id is refused in
            // audit_repeat_baseline_state and counted separately here.
            if repeat_gate_managed_domain(header.domain_id) {
                state.domain_data_repeat_records =
                    state.domain_data_repeat_records.saturating_add(1);
                let tally = state.repeat_tallies.entry(header.domain_id).or_default();
                tally.repeat_records = tally.repeat_records.saturating_add(1);
                tally.first_repeat_hour_raw.get_or_insert(header.game_hour_raw);
                tally.last_repeat_hour_raw = Some(header.game_hour_raw);
            }
        }
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
                    let provenance = DomainGapRecordV1::decode_witness_provenance(payload)
                        .unwrap_or(DomainGapWitnessProvenance::Unstated);
                    detail.gap = Some(gap_record_detail(gap, provenance));
                    // Force-full law (domain_repeat_gate.rs): any named gap
                    // for a gate-managed domain forces that domain's next
                    // published record FULL. The tally is created here so a
                    // gap that precedes the domain's first data record is
                    // still remembered.
                    if repeat_gate_managed_domain(header.domain_id) {
                        state
                            .repeat_tallies
                            .entry(header.domain_id)
                            .or_default()
                            .gap_since_full = true;
                    }
                    account_gap(
                        state,
                        offset,
                        header.sequence,
                        header.domain_id,
                        header.game_hour_raw,
                        gap,
                        provenance,
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

    if semantic_error.is_none()
        && matches!(
            header.record_type,
            COMPACT_RECORD_TYPE_DOMAIN_DATA
                | compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
        )
    {
        audit_repeat_baseline_state(state, offset, header, payload);
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

    // The same decode, for any domain the caller asked for. Deliberately a
    // SEPARATE arm rather than a widened gate on the block above: that block
    // owns the lane-wall probe, which is domain 250's and only 250's, and
    // folding the two would make a requested domain 250 either lose its probe
    // or take it twice.
    //
    // A decode failure here is recorded in this section and NOT pushed into
    // the global parse-error census: `--units-domain` is a read-out a caller
    // opts into, and a malformed trailer on some domain must not change the
    // error count other consumers compare between runs. It is still named,
    // per row and as a total.
    if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF
        && state.units_domains.contains(&header.domain_id)
    {
        if semantic_error.is_some() {
            // The record's own framing is in doubt, so its payload offsets
            // are too. Counted, not dropped.
            state.unit_records_skipped_semantic_error =
                state.unit_records_skipped_semantic_error.saturating_add(1);
        } else {
            state.unit_records_matched = state.unit_records_matched.saturating_add(1);
            let (units, decode_error) = match decode_probe_units(header.record_flags, payload) {
                Ok(Some(units)) => (Some(units.iter().map(unit_detail).collect()), None),
                Ok(None) => {
                    state.unit_records_without_trailer =
                        state.unit_records_without_trailer.saturating_add(1);
                    (None, None)
                }
                Err(error) => {
                    state.unit_records_decode_error =
                        state.unit_records_decode_error.saturating_add(1);
                    (None, Some(error))
                }
            };
            if state.unit_series.len() < UNIT_SERIES_LIMIT {
                state.unit_series.push(UnitSeriesRow {
                    offset,
                    sequence: header.sequence,
                    game_hour_raw: header.game_hour_raw,
                    domain_id: header.domain_id,
                    record_flags: header.record_flags,
                    units,
                    decode_error,
                });
            } else {
                state.unit_series_suppressed = state.unit_series_suppressed.saturating_add(1);
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

/// Keyframe-cadence, force-full and shared-codec verification for one
/// semantically valid DOMAIN_DATA / DOMAIN_DATA_REPEAT record. Full records
/// anchor the cadence; repeats are checked against the anchor and resolved
/// through `DomainRepeatBaselines` -- the same reconstruction the collector
/// gate and the production fold run -- so a repeat whose baseline is absent
/// from this stream or whose tail digest does not reconstruct is refused by
/// name here.
fn audit_repeat_baseline_state(
    state: &mut AuditState,
    offset: u64,
    header: &compact_contract::CompactRecordHeader,
    payload: &[u8],
) {
    // The collector never gates ids 0/250/1000 (domain_repeat_gate.rs
    // `gateable_domain`), so a type-14 record for one cannot be a legitimate
    // gate emission; envelope validation alone does not refuse it. Full DATA
    // records of excluded domains simply never take part in the census.
    if !repeat_gate_managed_domain(header.domain_id) {
        if header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT {
            state.repeat_gate_excluded_repeat_records =
                state.repeat_gate_excluded_repeat_records.saturating_add(1);
            record_error(
                state,
                offset,
                format!(
                    "DOMAIN_DATA_REPEAT for gate-excluded domain {} is not a legitimate gate emission",
                    header.domain_id
                ),
            );
        }
        return;
    }

    let hour_index = compact_contract::calendar::hour_index_from_raw(header.game_hour_raw);
    if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
        // Refresh the tally anchors AND seed the shared baseline store: a
        // later repeat resolves against the LAST full record of its domain.
        {
            let tally = state.repeat_tallies.entry(header.domain_id).or_default();
            tally.full_data_records = tally.full_data_records.saturating_add(1);
            tally.last_full_hour_index = hour_index;
            tally.gap_since_full = false;
            tally.repeat_run = 0;
        }
        if let Err(reason) = state
            .repeat_baselines
            .observe(header.domain_id, header.record_type, payload)
        {
            record_baseline_error(state, offset, reason);
        }
        return;
    }

    // Decide all named refusals inside one scoped borrow, then record them.
    let cadence = i64::from(COMPACT_DOMAIN_DATA_REPEAT_KEYFRAME_CADENCE_HOURS);
    let mut refusals: Vec<String> = Vec::new();
    {
        let tally = state.repeat_tallies.entry(header.domain_id).or_default();
        if tally.gap_since_full {
            tally.repeats_after_named_gap = tally.repeats_after_named_gap.saturating_add(1);
            refusals.push(format!(
                "DOMAIN_DATA_REPEAT for domain {} follows a named gap for its domain; the force-full law requires the next published record to be FULL",
                header.domain_id
            ));
        }
        match (tally.last_full_hour_index, hour_index) {
            (Some(last), Some(current))
                if !(current < last || current - last >= cadence) => {}
            (last, current) => {
                tally.keyframe_cadence_violations =
                    tally.keyframe_cadence_violations.saturating_add(1);
                refusals.push(format!(
                    "DOMAIN_DATA_REPEAT for domain {} violates the keyframe cadence: current hour index {:?} against last full record {:?} under the {cadence}-hour law (the collector gate publishes FULL whenever either index is unanchorable)",
                    header.domain_id, current, last
                ));
            }
        }
        tally.repeat_run = tally.repeat_run.saturating_add(1);
        if tally.repeat_run > tally.longest_repeat_run {
            tally.longest_repeat_run = tally.repeat_run;
        }
    }
    for refusal in refusals {
        record_error(state, offset, refusal);
    }
    if let Err(reason) = state
        .repeat_baselines
        .observe(header.domain_id, header.record_type, payload)
    {
        record_baseline_error(state, offset, reason);
    }
}

/// Baseline-chain failures go to BOTH the capped census examples and the
/// stream-wide parse_errors, so the standard health-check bar catches them.
fn record_baseline_error(state: &mut AuditState, offset: u64, reason: String) {
    state.repeat_baseline_errors_total = state.repeat_baseline_errors_total.saturating_add(1);
    let message = format!("DOMAIN_DATA_REPEAT baseline check failed: {reason}");
    if state.repeat_baseline_error_examples.len() < REPEAT_ERROR_EXAMPLE_LIMIT {
        state.repeat_baseline_error_examples.push(ParseError {
            offset,
            error: message.clone(),
            framing_lost: false,
        });
    }
    record_error(state, offset, message);
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
        lw_hour_age_us: find("lw_hour_age_us"),
        lw_hour_frontier_ahead: find("lw_hour_frontier_ahead"),
        lw_hour_age_missing: find("lw_hour_age_missing"),
    }
}

fn account_gap(
    state: &mut AuditState,
    offset: u64,
    sequence: u64,
    domain_id: u16,
    frame_label: i32,
    gap: DomainGapRecordV1,
    provenance: DomainGapWitnessProvenance,
) {
    let reason = reason_name(gap.reason);
    *state
        .provenance_counts
        .entry((reason, provenance_name(provenance)))
        .or_default() += 1;
    *state.reason_counts.entry(reason).or_default() += 1;
    let covered = i64::from(gap.range_end_hour_raw)
        .checked_sub(i64::from(gap.range_start_hour_raw))
        .and_then(|delta| delta.checked_add(1))
        .filter(|delta| *delta > 0)
        .and_then(|delta| u64::try_from(delta).ok())
        .unwrap_or(0);
    if covered == 0 {
        record_error(
            state,
            offset,
            format!(
                "DOMAIN_GAP range {}..={} covers no hour; it cannot account for the \
                 loss it names",
                gap.range_start_hour_raw, gap.range_end_hour_raw
            ),
        );
    }
    *state.reason_domain_hours.entry(reason).or_default() += covered;
    if covered <= MAX_ENUMERATED_RANGE_HOURS {
        let hours = state.reason_hours.entry(reason).or_default();
        for hour in gap.range_start_hour_raw..=gap.range_end_hour_raw {
            hours.insert(hour);
        }
    } else {
        record_error(
            state,
            offset,
            format!(
                "DOMAIN_GAP range {}..={} covers {covered} hours, past the \
                 {MAX_ENUMERATED_RANGE_HOURS} this audit will enumerate; its \
                 hours are counted in domain_hours but not in distinct_hours",
                gap.range_start_hour_raw, gap.range_end_hour_raw
            ),
        );
    }
    let clock_derived = witnesses_are_a_clock_observation(gap.reason, provenance)
        && gap.tick_before != 0
        && gap.tick_after != 0;
    let opening_delta = witness_delta(gap.tick_before, frame_label);
    let closing_delta = witness_delta(gap.tick_after, frame_label);
    if clock_derived {
        *state
            .witness_delta_counts
            .entry((
                reason,
                provenance_name(provenance),
                opening_delta,
                closing_delta,
            ))
            .or_default() += 1;
        let example = WitnessExample {
            offset,
            sequence,
            domain_id,
            reason,
            witness_provenance: provenance_name(provenance),
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
        witness_provenance: provenance_name(provenance),
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
        if probe.lw_hour_age_us.is_some()
            || probe.lw_hour_frontier_ahead.is_some()
            || probe.lw_hour_age_missing.is_some()
        {
            state.lateness_carrier_frames = state.lateness_carrier_frames.saturating_add(1);
        }
        match (
            probe.lw_hour_age_us,
            probe.lw_hour_label_mismatch_domains,
            probe.lw_bracketed_domains,
        ) {
            (Some(age_us), Some(mismatched), Some(bracketed)) => {
                state.lateness_frames = state.lateness_frames.saturating_add(1);
                state.lateness_samples.push((age_us, mismatched, bracketed));
            }
            _ if probe.lw_hour_age_missing == Some(1) => {
                state.lateness_missing_frames = state.lateness_missing_frames.saturating_add(1);
            }
            (Some(_), _, _) => {
                state.lateness_unpaired_frames = state.lateness_unpaired_frames.saturating_add(1);
            }
            _ => {}
        }
        if let Some(ahead) = probe.lw_hour_frontier_ahead {
            *state.frontier_ahead_distribution.entry(ahead).or_default() += 1;
        }
    }

    let torn = frame.gaps.iter().any(|gap| gap.reason == "Torn");
    let label_skew = frame
        .lane_wall
        .as_ref()
        .and_then(|probe| probe.lw_hour_label_skew_abs)
        .is_some_and(|skew| skew > 0);
    let mut exact_mismatch = None;
    if let Some(probe) = frame.lane_wall.as_ref() {
        match (
            probe.lw_hour_label_mismatch_domains,
            probe.lw_bracketed_domains,
            probe.lw_hour_label_unusable,
        ) {
            (Some(domains), Some(bracketed), Some(unusable)) => {
                state.exact_instrument_frames = state.exact_instrument_frames.saturating_add(1);
                state.exact_mismatch_domains = state.exact_mismatch_domains.saturating_add(domains);
                state.exact_bracketed_domains =
                    state.exact_bracketed_domains.saturating_add(bracketed);
                *state
                    .exact_mismatch_distribution
                    .entry(domains)
                    .or_default() += 1;
                if domains > 0 {
                    state.exact_mismatch_frames = state.exact_mismatch_frames.saturating_add(1);
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
        collector_build: recorder_host::build_declaration::summary_json(
            &recorder_host::build_declaration::for_stream(input),
        ),
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
            build_start_lateness: build_start_lateness(
                state.lateness_carrier_frames,
                state.lateness_frames,
                state.lateness_missing_frames,
                state.lateness_unpaired_frames,
                state.frontier_ahead_distribution,
                state.lateness_samples,
            ),
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
                    domain_hours: state
                        .reason_domain_hours
                        .get(reason_name(reason))
                        .copied()
                        .unwrap_or(0),
                    distinct_hours: state
                        .reason_hours
                        .get(reason_name(reason))
                        .map(|hours| hours.len() as u64)
                        .unwrap_or(0),
                })
                .collect(),
            undecoded_gap_records: state.undecoded_gaps,
            witness_provenance_counts: state
                .provenance_counts
                .into_iter()
                .map(|((reason, witness_provenance), count)| ProvenanceCount {
                    reason,
                    witness_provenance,
                    count,
                    witnesses_admitted: admitted_by_name(reason, witness_provenance),
                })
                .collect(),
            clock_witness_delta_counts: state
                .witness_delta_counts
                .into_iter()
                .map(
                    |((reason, witness_provenance, opening_delta, closing_delta), count)| {
                        WitnessDeltaCount {
                            reason,
                            witness_provenance,
                            opening_delta,
                            closing_delta,
                            count,
                        }
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
        section_f_repeat_census: RepeatCensusSection {
            domain_data_repeat_records: state.domain_data_repeat_records,
            domains_with_repeats: state
                .repeat_tallies
                .values()
                .filter(|tally| tally.repeat_records > 0)
                .count() as u64,
            gate_excluded_repeat_records: state.repeat_gate_excluded_repeat_records,
            per_domain: state
                .repeat_tallies
                .iter()
                .map(|(domain_id, tally)| RepeatDomainRow {
                    domain_id: *domain_id,
                    repeat_records: tally.repeat_records,
                    first_repeat_hour_raw: tally.first_repeat_hour_raw,
                    last_repeat_hour_raw: tally.last_repeat_hour_raw,
                    full_data_records: tally.full_data_records,
                    keyframe_cadence_violations: tally.keyframe_cadence_violations,
                    repeats_after_named_gap: tally.repeats_after_named_gap,
                    longest_repeat_run: tally.longest_repeat_run,
                })
                .collect(),
            baseline_errors_total: state.repeat_baseline_errors_total,
            baseline_error_examples: state.repeat_baseline_error_examples,
        },
        section_g_domain_units: DomainUnitSection {
            requested_domain_ids: state.units_domains.iter().copied().collect(),
            perf_records_matched: state.unit_records_matched,
            perf_records_without_trailer: state.unit_records_without_trailer,
            perf_records_with_decode_error: state.unit_records_decode_error,
            perf_records_skipped_semantic_error: state.unit_records_skipped_semantic_error,
            rows: state.unit_series,
            rows_suppressed: state.unit_series_suppressed,
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

fn build_start_lateness(
    frames_carrying_it: u64,
    frames_in_joint_table: u64,
    frames_missing_it: u64,
    frames_excluded_unpaired: u64,
    frontier_ahead: BTreeMap<u64, u64>,
    samples: Vec<(u64, u64, u64)>,
) -> BuildStartLateness {
    const BUCKET_US: u64 = 10_000;
    let mut ages: Vec<u64> = samples.iter().map(|(age, _, _)| *age).collect();
    let mut buckets: BTreeMap<u64, (u64, u64, u64, u64)> = BTreeMap::new();
    for (age_us, mismatched, bracketed) in &samples {
        let entry = buckets.entry(age_us / BUCKET_US).or_default();
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry.1.saturating_add(*mismatched);
        entry.2 = entry.2.max(*mismatched);
        entry.3 = entry.3.saturating_add(*bracketed);
    }
    BuildStartLateness {
        frames_carrying_it,
        frames_in_joint_table,
        frames_missing_it,
        frames_excluded_unpaired,
        frontier_ahead_distribution: frontier_ahead
            .into_iter()
            .map(|(value, count)| ValueCount { value, count })
            .collect(),
        lw_hour_age_us: stats(&mut ages),
        buckets: buckets
            .into_iter()
            .map(
                |(bucket, (frames, mismatch_sum, mismatch_max, bracketed))| LatenessBucket {
                    lateness_ms_from: bucket * (BUCKET_US / 1_000),
                    lateness_ms_to: (bucket + 1) * (BUCKET_US / 1_000),
                    frames,
                    mismatched_domains_mean: mismatch_sum as f64 / frames as f64,
                    mismatched_domains_max: mismatch_max,
                    bracketed_domains: bracketed,
                },
            )
            .collect(),
    }
}

fn print_report(report: &AuditReport) -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    let declaration = recorder_host::build_declaration::for_stream(Path::new(&report.input_path));
    write!(
        out,
        "{}",
        recorder_host::build_declaration::banner(&declaration)
    )?;
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
    writeln!(
        out,
        "  frames with a mismatch: {}",
        e.frames_with_a_mismatch
    )?;
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

    let l = &report.section_a_frame_census.build_start_lateness;
    writeln!(out, "build-start lateness (lw_hour_age_us):")?;
    if l.frames_carrying_it == 0 {
        writeln!(
            out,
            "  NOT PRESENT: the recorder that wrote this capture predates \
             lw_hour_age_us, so the table below is ABSENT, not empty"
        )?;
    }
    writeln!(out, "  frames carrying it: {}", l.frames_carrying_it)?;
    writeln!(out, "  frames in the buckets: {}", l.frames_in_joint_table)?;
    writeln!(
        out,
        "  frames whose label the observer held no stamp for: {}",
        l.frames_missing_it
    )?;
    if l.frames_excluded_unpaired != 0 {
        writeln!(
            out,
            "  WARNING: {} frame(s) carried a lateness without both mismatch \
             counters and are NOT in the buckets below",
            l.frames_excluded_unpaired
        )?;
    }
    print_stats(&mut out, "  lw_hour_age_us", &l.lw_hour_age_us)?;
    writeln!(
        out,
        "  hours the frontier ran ahead of the label -> frames:"
    )?;
    if l.frontier_ahead_distribution.is_empty() {
        writeln!(out, "    (none)")?;
    }
    for row in &l.frontier_ahead_distribution {
        writeln!(out, "    {} -> {}", row.value, row.count)?;
    }
    writeln!(
        out,
        "  lateness bucket -> frames, mean/max mislabelled domains, bracketed:"
    )?;
    if l.buckets.is_empty() {
        writeln!(out, "    (none)")?;
    }
    for row in &l.buckets {
        writeln!(
            out,
            "    [{:3},{:3}) ms  frames={:5}  mismatch mean={:6.1} max={:3}  bracketed={}",
            row.lateness_ms_from,
            row.lateness_ms_to,
            row.frames,
            row.mismatched_domains_mean,
            row.mismatched_domains_max,
            row.bracketed_domains
        )?;
    }

    let b = &report.section_b_domain_gaps;
    writeln!(out, "\nB. DOMAIN_GAP census")?;
    writeln!(
        out,
        "reason -> records / domain-hours / distinct hours\n  \
         (records COALESCE: one record can stand for many hours. Only the \
         domain-hours\n   column is comparable with the scheduler probe's \
         rs_*_domain_hours counters.)"
    )?;
    for row in &b.reason_counts {
        writeln!(
            out,
            "  {} -> {} records / {} domain-hours / {} distinct hours",
            row.reason, row.count, row.domain_hours, row.distinct_hours
        )?;
    }
    writeln!(
        out,
        "undecoded DOMAIN_GAP records: {}",
        b.undecoded_gap_records
    )?;
    writeln!(out, "gap witness provenance (reason, provenance) -> count:")?;
    if b.witness_provenance_counts.is_empty() {
        writeln!(out, "  (none)")?;
    }
    for row in &b.witness_provenance_counts {
        writeln!(
            out,
            "  ({}, {}) -> {} [witnesses {}]",
            row.reason,
            row.witness_provenance,
            row.count,
            if row.witnesses_admitted {
                "admitted"
            } else {
                "excluded"
            }
        )?;
    }
    writeln!(
        out,
        "clock witness deltas (reason, provenance, opening-label, closing-label) -> count:"
    )?;
    if b.clock_witness_delta_counts.is_empty() {
        writeln!(out, "  (none)")?;
    }
    for row in &b.clock_witness_delta_counts {
        writeln!(
            out,
            "  ({}, {}, {:+}, {:+}) -> {}",
            row.reason, row.witness_provenance, row.opening_delta, row.closing_delta, row.count
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
            "  offset={} seq={} d{} {} [{}] (label={}, opening={}, closing={})              deltas=({:+},{:+})",
            example.offset,
            example.sequence,
            example.domain_id,
            example.reason,
            example.witness_provenance,
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
    writeln!(
        out,
        "                                      torn=no  torn=yes"
    )?;
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

    let f = &report.section_f_repeat_census;
    writeln!(out, "\nF. Domain-data-repeat census")?;
    writeln!(
        out,
        "DOMAIN_DATA_REPEAT records: {}",
        f.domain_data_repeat_records
    )?;
    writeln!(out, "domains carrying repeats: {}", f.domains_with_repeats)?;
    writeln!(
        out,
        "gate-excluded type-14 records (refused): {}",
        f.gate_excluded_repeat_records
    )?;
    for row in &f.per_domain {
        writeln!(
            out,
            "  domain {}: repeats={} hours={:?}..={:?} full_records={} longest_repeat_run={} cadence_violations={} repeats_after_named_gap={}",
            row.domain_id,
            row.repeat_records,
            row.first_repeat_hour_raw,
            row.last_repeat_hour_raw,
            row.full_data_records,
            row.longest_repeat_run,
            row.keyframe_cadence_violations,
            row.repeats_after_named_gap
        )?;
    }
    writeln!(
        out,
        "baseline consistency errors: {} (verified via compact_contract::DomainRepeatBaselines)",
        f.baseline_errors_total
    )?;
    for error in &f.baseline_error_examples {
        writeln!(out, "  offset {}: {}", error.offset, error.error)?;
    }

    let g = &report.section_g_domain_units;
    write_domain_unit_section(&mut out, g)?;
    out.flush()
}

/// Section G, rendered for a human.
///
/// SPLIT OUT of `print_report` so it can be tested: `print_report` writes
/// straight to a locked stdout, so nothing could assert on what a caller
/// actually SEES, and the read-out silently printed less than this lane
/// promised for it (Codex r5). Every field the JSON carries is now printed:
/// the record's flags on every row shape, and each unit's name, value, role,
/// presence and rendered form.
///
/// The rendered form is kept ALONGSIDE the parts rather than instead of them.
/// It is the only field that distinguishes `absent` and `sentinel(0x..)` from
/// a bare number that means neither, and the parts are what a reader greps.
fn write_domain_unit_section(out: &mut impl Write, g: &DomainUnitSection) -> io::Result<()> {
    if g.requested_domain_ids.is_empty() {
        return Ok(());
    }
    writeln!(out, "\nG. Per-record DOMAIN_PERF units")?;
    writeln!(out, "requested domains: {:?}", g.requested_domain_ids)?;
    writeln!(
        out,
        "PERF records matched: {} (no trailer: {}, decode errors: {}, skipped for a semantic error: {})",
        g.perf_records_matched,
        g.perf_records_without_trailer,
        g.perf_records_with_decode_error,
        g.perf_records_skipped_semantic_error
    )?;
    if g.rows_suppressed != 0 {
        writeln!(
            out,
            "rows beyond the {UNIT_SERIES_LIMIT}-row cap, not listed: {}",
            g.rows_suppressed
        )?;
    }
    for row in &g.rows {
        write!(
            out,
            "  hour={} domain={} seq={} offset={} flags={:#010x}",
            row.game_hour_raw, row.domain_id, row.sequence, row.offset, row.record_flags
        )?;
        match (&row.units, &row.decode_error) {
            (_, Some(error)) => writeln!(out, " decode_error={error}")?,
            (None, None) => writeln!(out, " no diagnostic trailer")?,
            (Some(units), None) => {
                writeln!(out, " units={}", units.len())?;
                for unit in units {
                    writeln!(
                        out,
                        "      name={} value={} role={} presence={} rendered={}",
                        unit.name, unit.value, unit.role, unit.presence, unit.rendered
                    )?;
                }
            }
        }
    }
    Ok(())
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

fn witnesses_are_a_clock_observation(
    reason: DomainGapReason,
    provenance: DomainGapWitnessProvenance,
) -> bool {
    match provenance {
        DomainGapWitnessProvenance::BracketObserved => true,
        DomainGapWitnessProvenance::Unobserved => false,
        DomainGapWitnessProvenance::Unstated => is_clock_derived(reason),
    }
}

fn is_clock_derived(reason: DomainGapReason) -> bool {
    matches!(
        reason,
        DomainGapReason::Torn | DomainGapReason::HourLabelMismatch
    )
}

fn provenance_name(provenance: DomainGapWitnessProvenance) -> &'static str {
    provenance.wire_name()
}

fn admitted_by_name(reason: &str, provenance: &str) -> bool {
    let Some(reason) = DomainGapReason::ALL
        .into_iter()
        .find(|candidate| reason_name(*candidate) == reason)
    else {
        return false;
    };
    let Some(provenance) = DomainGapWitnessProvenance::ALL
        .into_iter()
        .find(|candidate| provenance_name(*candidate) == provenance)
    else {
        return false;
    };
    witnesses_are_a_clock_observation(reason, provenance)
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

fn gap_record_detail(
    gap: DomainGapRecordV1,
    provenance: DomainGapWitnessProvenance,
) -> GapRecordDetail {
    GapRecordDetail {
        part: match gap.part {
            compact_contract::DomainGapPart::State => "State",
            compact_contract::DomainGapPart::Event => "Event",
            compact_contract::DomainGapPart::Both => "Both",
        },
        reason: reason_name(gap.reason),
        witness_provenance: provenance_name(provenance),
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

    const HOUR_A: i32 = 60_831_056;
    const HOUR_B: i32 = 60_831_057;

    fn record(
        sequence: u64,
        hour: i32,
        domain: u16,
        rtype: u16,
        flags: u32,
        payload: &[u8],
    ) -> Vec<u8> {
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
            units.push(DiagUnit::counter(
                "lw_hour_label_mismatch_domains",
                mismatch,
            ));
            units.push(DiagUnit::counter("lw_bracketed_domains", bracketed));
            units.push(DiagUnit::counter("lw_hour_label_unusable", 0));
        }
        units
    }

    fn probe_units_with_lateness(
        exact: Option<(u64, u64)>,
        skew_abs: u64,
        lateness: Option<(u64, u64)>,
    ) -> Vec<DiagUnit> {
        let mut units = probe_units(exact, skew_abs);
        match lateness {
            Some((age_us, ahead)) => {
                units.push(DiagUnit::counter("lw_hour_age_us", age_us));
                units.push(DiagUnit::counter("lw_hour_frontier_ahead", ahead));
                units.push(DiagUnit::counter("lw_hour_age_missing", 0));
            }
            None => units.push(DiagUnit::counter("lw_hour_age_missing", 1)),
        }
        units
    }

    fn frame_with_lateness(
        sequence: &mut u64,
        hour: i32,
        exact: Option<(u64, u64)>,
        lateness: Option<(u64, u64)>,
    ) -> Vec<u8> {
        let mut out = record(
            *sequence,
            hour,
            RC_PROBE_DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &lane_wall_payload(&probe_units_with_lateness(exact, 0, lateness)),
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

    fn audit_with_units(stream: &[u8], domains: &[u16]) -> AuditReport {
        let mut state = AuditState {
            units_domains: domains.iter().copied().collect(),
            ..AuditState::default()
        };
        scan_stream(&mut &stream[..], &mut state).expect("scan");
        finish_report(Path::new("synthetic"), state)
    }

    /// A DOMAIN_PERF record carrying a diagnostic trailer, for any domain.
    fn perf_with_units(sequence: &mut u64, hour: i32, domain: u16, units: &[DiagUnit]) -> Vec<u8> {
        let out = record(
            *sequence,
            hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &lane_wall_payload(units),
        );
        *sequence += 1;
        out
    }

    /// A DOMAIN_PERF record carrying a WELL-FORMED trailer behind a base whose
    /// reserved word is nonzero, which `validate_domain_perf` rejects.
    ///
    /// The point of the shape: the units behind it decode perfectly, so a
    /// reader that ignored the semantic error would happily publish them.
    fn perf_with_broken_reserved_word(
        sequence: &mut u64,
        hour: i32,
        domain: u16,
        units: &[DiagUnit],
    ) -> Vec<u8> {
        let mut payload = lane_wall_payload(units);
        // The reserved base word DOMAIN_PERF requires to be zero.
        payload[36..40].copy_from_slice(&1u32.to_le_bytes());
        let out = record(
            *sequence,
            hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &payload,
        );
        *sequence += 1;
        out
    }

    /// A DOMAIN_PERF record with no trailer at all - a real shape, and the one
    /// a per-frame series must be able to tell from "reported zero".
    fn perf_without_units(sequence: &mut u64, hour: i32, domain: u16) -> Vec<u8> {
        let out = record(
            *sequence,
            hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN],
        );
        *sequence += 1;
        out
    }

    fn frame_end_record(sequence: &mut u64, hour: i32) -> Vec<u8> {
        let out = record(
            *sequence,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; 32],
        );
        *sequence += 1;
        out
    }

    /// One frame carrying the lane-wall probe on domain 250, a d53 PERF record
    /// with named units, a domain-300 record with units, and a domain-300
    /// record with no trailer.
    fn mixed_units_frame(sequence: &mut u64, hour: i32) -> Vec<u8> {
        let mut out = perf_with_units(sequence, hour, RC_PROBE_DOMAIN_ID, &probe_units(None, 0));
        out.extend_from_slice(&perf_with_units(
            sequence,
            hour,
            53,
            &[
                DiagUnit::counter("domain53_relation_uncommitted_candidate_base", 3),
                DiagUnit::counter("domain53_operative_skipped", 71),
            ],
        ));
        out.extend_from_slice(&perf_with_units(
            sequence,
            hour,
            300,
            &[DiagUnit::counter("d300_rows", 9)],
        ));
        out.extend_from_slice(&perf_without_units(sequence, hour, 300));
        out.extend_from_slice(&frame_end_record(sequence, hour));
        out
    }

    fn frame_with_d81_source_unavailable_gap(
        sequence: &mut u64,
        hour: i32,
        observed: u64,
        provenance: DomainGapWitnessProvenance,
    ) -> Vec<u8> {
        let payload = DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: DomainGapReason::SourceUnavailable,
            target_hour_raw: hour,
            range_start_hour_raw: hour,
            range_end_hour_raw: hour,
            tick_before: observed,
            tick_after: observed,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode_with_witness_provenance(provenance)
        .expect("the fixture gap must be one the wire can carry");
        let mut out = record(
            *sequence,
            hour,
            RC_PROBE_DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &lane_wall_payload(&probe_units(Some((1, 68)), 0)),
        );
        *sequence += 1;
        out.extend_from_slice(&record(
            *sequence,
            hour,
            81,
            COMPACT_RECORD_TYPE_DOMAIN_GAP,
            0,
            &payload,
        ));
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

    #[test]
    fn only_a_bracket_stamped_source_unavailable_row_is_read_as_an_observation() {
        let observed = u64::try_from(HOUR_B).expect("fixture hour fits u64");

        let mut sequence = 0u64;
        let stamped = frame_with_d81_source_unavailable_gap(
            &mut sequence,
            HOUR_A,
            observed,
            DomainGapWitnessProvenance::BracketObserved,
        );
        let report = audit(&stamped);
        assert_eq!(report.parse_error_count, 0, "{:?}", report.parse_errors);
        let b = &report.section_b_domain_gaps;
        assert_eq!(
            b.clock_witness_examples.len(),
            1,
            "a bracket-stamped SourceUnavailable row must reach the witness \
             table: {:?}",
            b.clock_witness_examples
        );
        assert_eq!(
            b.clock_witness_examples[0].witness_provenance,
            "bracket_observed"
        );
        assert_eq!(b.clock_witness_examples[0].opening_witness, observed);
        assert_eq!(
            (
                b.clock_witness_delta_counts[0].opening_delta,
                b.clock_witness_delta_counts[0].closing_delta
            ),
            (1, 1),
            "and it must report the excursion it actually observed, not a \
             comparison of the label with itself"
        );
        assert!(
            b.witness_provenance_counts
                .iter()
                .any(|row| row.reason == "SourceUnavailable"
                    && row.witness_provenance == "bracket_observed"
                    && row.count == 1
                    && row.witnesses_admitted),
            "the census must say WHY the row was believed: {:?}",
            b.witness_provenance_counts
        );

        let mut sequence = 0u64;
        let unstamped = frame_with_d81_source_unavailable_gap(
            &mut sequence,
            HOUR_A,
            observed,
            DomainGapWitnessProvenance::Unstated,
        );
        let report = audit(&unstamped);
        assert_eq!(report.parse_error_count, 0, "{:?}", report.parse_errors);
        let b = &report.section_b_domain_gaps;
        assert!(
            b.clock_witness_examples.is_empty() && b.clock_witness_delta_counts.is_empty(),
            "an unstated row must stay excluded -- believing it is exactly the \
             corroboration the capture does not contain: {:?}",
            b.clock_witness_examples
        );
        assert!(
            b.witness_provenance_counts
                .iter()
                .any(|row| row.reason == "SourceUnavailable"
                    && row.witness_provenance == "unstated"
                    && row.count == 1
                    && !row.witnesses_admitted),
            "and it must still be COUNTED, with its exclusion stated: {:?}",
            b.witness_provenance_counts
        );

        assert_eq!(stamped.len(), unstamped.len());
        assert_eq!(
            stamped
                .iter()
                .zip(unstamped.iter())
                .filter(|(a, b)| a != b)
                .count(),
            1,
            "the fixtures must differ in the provenance byte and nothing else, \
             or this test is comparing two different captures"
        );
    }

    #[test]
    fn the_inclusion_rule_keys_on_provenance_and_never_manufactures_one() {
        for reason in DomainGapReason::ALL {
            assert!(
                witnesses_are_a_clock_observation(
                    reason,
                    DomainGapWitnessProvenance::BracketObserved
                ),
                "{reason:?}: a producer's own bracket stamp is the fact this \
                 tool asks for; the reason may not veto it"
            );
            assert!(
                !witnesses_are_a_clock_observation(reason, DomainGapWitnessProvenance::Unobserved),
                "{reason:?}: `Unobserved` is a statement that there is nothing \
                 to compare"
            );
            assert_eq!(
                witnesses_are_a_clock_observation(reason, DomainGapWitnessProvenance::Unstated),
                is_clock_derived(reason),
                "{reason:?}: an unstated row must be judged EXACTLY as the \
                 pre-provenance tool judged it, or old captures change meaning"
            );
        }

        let forgeable = DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: DomainGapReason::SourceUnavailable,
            target_hour_raw: HOUR_A,
            range_start_hour_raw: HOUR_A,
            range_end_hour_raw: HOUR_A,
            tick_before: 7,
            tick_after: 7,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode_with_witness_provenance(DomainGapWitnessProvenance::Unobserved);
        assert!(forgeable.is_err());
    }

    #[test]
    fn a_capture_written_before_the_field_existed_decodes_as_unstated() {
        let mut wire = DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: DomainGapReason::SourceUnavailable,
            target_hour_raw: HOUR_A,
            range_start_hour_raw: HOUR_A,
            range_end_hour_raw: HOUR_A,
            tick_before: u64::try_from(HOUR_A).unwrap(),
            tick_after: u64::try_from(HOUR_A).unwrap(),
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode()
        .expect("a t17-shaped d81 row");
        wire[12..16].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            DomainGapRecordV1::decode_witness_provenance(&wire).unwrap(),
            DomainGapWitnessProvenance::Unstated
        );
        assert!(!witnesses_are_a_clock_observation(
            DomainGapReason::SourceUnavailable,
            DomainGapWitnessProvenance::Unstated
        ));
    }

    #[test]
    fn build_start_lateness_is_bucketed_only_against_the_counts_it_pairs_with() {
        let mut sequence = 0u64;
        let mut stream =
            frame_with_lateness(&mut sequence, HOUR_A, Some((0, 68)), Some((5_000, 0)));
        stream.extend_from_slice(&frame_with_lateness(
            &mut sequence,
            HOUR_B,
            Some((67, 68)),
            Some((65_000, 0)),
        ));
        stream.extend_from_slice(&frame_with_lateness(
            &mut sequence,
            HOUR_B + 1,
            None,
            Some((62_000, 1)),
        ));
        stream.extend_from_slice(&frame_with_lateness(
            &mut sequence,
            HOUR_B + 2,
            Some((4, 68)),
            None,
        ));
        stream.extend_from_slice(&frame(&mut sequence, HOUR_B + 3, Some((4, 68)), 0));
        let report = audit(&stream);
        assert_eq!(report.parse_error_count, 0, "{:?}", report.parse_errors);

        let lateness = &report.section_a_frame_census.build_start_lateness;
        assert_eq!(
            lateness.frames_in_joint_table, 2,
            "a frame with no mismatch counters must not enter the joint table"
        );
        assert_eq!(
            lateness.frames_carrying_it, 4,
            "every frame that carried ANY of the three units counts as carrying \
             the instrument; keying that on the paired count made a capture \
             report the recorder as predating a unit it plainly emitted"
        );

        let mut only_flag = Vec::new();
        let mut seq = 0u64;
        only_flag.extend_from_slice(&record(
            seq,
            HOUR_A,
            RC_PROBE_DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &lane_wall_payload(&{
                let mut units = probe_units(Some((4, 68)), 0);
                units.push(DiagUnit::counter("lw_hour_age_missing", 0));
                units
            }),
        ));
        seq += 1;
        only_flag.extend_from_slice(&record(
            seq,
            HOUR_A,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; 32],
        ));
        let flag_only = &audit(&only_flag)
            .section_a_frame_census
            .build_start_lateness;
        assert_eq!(
            flag_only.frames_carrying_it, 1,
            "a frame carrying only the presence flag still carried the instrument"
        );
        assert_eq!(flag_only.frames_in_joint_table, 0);
        assert_eq!(flag_only.frames_missing_it, 0);
        assert_eq!(flag_only.frames_excluded_unpaired, 0);
        assert_eq!(
            lateness.frames_missing_it, 1,
            "the frame whose observer held no stamp is counted apart, not as a zero"
        );
        assert_eq!(
            lateness.frames_excluded_unpaired, 1,
            "the frame with a lateness but no mismatch counters must be NAMED, \
             not silently dropped from the population the buckets describe"
        );
        assert_eq!(lateness.lw_hour_age_us.count, 2);
        assert_eq!(lateness.lw_hour_age_us.max, Some(65_000));

        let buckets: Vec<_> = lateness
            .buckets
            .iter()
            .map(|row| {
                (
                    row.lateness_ms_from,
                    row.frames,
                    row.mismatched_domains_mean,
                    row.bracketed_domains,
                )
            })
            .collect();
        assert_eq!(buckets, vec![(0, 1, 0.0, 68), (60, 1, 67.0, 68)]);

        let ahead: Vec<_> = lateness
            .frontier_ahead_distribution
            .iter()
            .map(|row| (row.value, row.count))
            .collect();
        assert_eq!(ahead, vec![(0, 2), (1, 1)]);
    }

    #[test]
    fn a_capture_predating_the_lateness_unit_reports_it_absent() {
        let mut sequence = 0u64;
        let stream = frame(&mut sequence, HOUR_A, Some((67, 68)), 0);
        let lateness = &audit(&stream).section_a_frame_census.build_start_lateness;
        assert_eq!(lateness.frames_carrying_it, 0);
        assert_eq!(lateness.frames_in_joint_table, 0);
        assert_eq!(lateness.frames_missing_it, 0);
        assert_eq!(lateness.lw_hour_age_us.count, 0);
        assert_eq!(lateness.lw_hour_age_us.median, None);
        assert!(lateness.buckets.is_empty());
        assert!(lateness.frontier_ahead_distribution.is_empty());
    }

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

    #[test]
    fn a_frame_without_the_exact_instrument_is_unknown_not_clean() {
        let mut sequence = 0u64;
        let mut stream = frame(&mut sequence, HOUR_A, Some((0, 68)), 0);
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

    #[test]
    fn units_domain_reads_records_above_and_below_the_lane_wall_probe_domain() {
        // kills: the `header.domain_id == RC_PROBE_DOMAIN_ID` gate that made
        // this tool able to decode DOMAIN_PERF units for domain 250 and
        // nothing else. The manifest only ever aggregates those units into
        // capture totals, so before this option no artefact in the repo could
        // produce a PER-FRAME series of a named unit for any other domain -
        // which is the measurement lane RW's FIX.md 5b had to leave open.
        //
        // Two domains on purpose: 53, BELOW the probe domain, and 300, ABOVE
        // it. A gate written as `<= 250`, `>= 250` or `== 250` fails on one of
        // them, and reverting the widening fails on both.
        let mut sequence = 1u64;
        let stream = mixed_units_frame(&mut sequence, HOUR_A);
        let report = audit_with_units(&stream, &[53, 300]);

        let g = &report.section_g_domain_units;
        assert_eq!(g.requested_domain_ids, vec![53, 300]);
        assert_eq!(
            g.perf_records_matched, 3,
            "two domain-300 records and one domain-53 record match; domain 250 \
             was not requested"
        );
        assert_eq!(g.perf_records_with_decode_error, 0);
        assert_eq!(g.perf_records_skipped_semantic_error, 0);
        assert_eq!(g.rows_suppressed, 0);
        assert_eq!(g.rows.len(), 3);
        assert!(
            g.rows.iter().all(|row| row.domain_id != RC_PROBE_DOMAIN_ID),
            "an unrequested domain must not appear in the series"
        );

        let d53 = g
            .rows
            .iter()
            .find(|row| row.domain_id == 53)
            .expect("the domain-53 record is in the series");
        assert_eq!(d53.game_hour_raw, HOUR_A, "the row carries its own hour");
        let units = d53
            .units
            .as_ref()
            .expect("the domain-53 record carried a trailer");
        assert_eq!(units.len(), 2);
        assert_eq!(
            units[0].name,
            "domain53_relation_uncommitted_candidate_base"
        );
        assert_eq!(units[0].value, 3);
        assert_eq!(units[0].presence, "observed");
        assert_eq!(
            units[0].rendered,
            "domain53_relation_uncommitted_candidate_base=3"
        );
        assert_eq!(units[1].name, "domain53_operative_skipped");
        assert_eq!(units[1].value, 71);

        // The domain-300 pair is the whole point of emitting a row per RECORD
        // rather than per record-with-a-trailer: one reported a value, one
        // carried no trailer, and a series that showed only the first would
        // read as "domain 300 was absent that frame".
        let d300: Vec<&UnitSeriesRow> = g.rows.iter().filter(|row| row.domain_id == 300).collect();
        assert_eq!(d300.len(), 2);
        assert_eq!(
            d300[0]
                .units
                .as_ref()
                .expect("the first domain-300 record carried a trailer")[0]
                .value,
            9
        );
        assert!(
            d300[1].units.is_none(),
            "the trailerless record is present as a row with no units, not absent"
        );
        assert_eq!(g.perf_records_without_trailer, 1);

        // And the lane-wall probe, which is domain 250's alone, is untouched.
        assert_eq!(report.section_a_frame_census.frames_with_lane_wall_probe, 1);
    }

    #[test]
    fn units_domain_250_keeps_its_lane_wall_probe_and_still_reports() {
        // kills: failing to emit a section-G row for domain 250 when it is
        // REQUESTED, while still delivering its lane-wall probe and adding no
        // parse error. Those are the two things that can go wrong when the one
        // domain that already had a decoder is also asked for by the new
        // option: the probe gets handed out twice (a second probe record, which
        // is a parse error here), or the requested row is dropped because 250
        // was already spoken for.
        //
        // It does NOT enforce a particular source arrangement (Codex r4 finding
        // 2). A single combined block that decodes once, handles the probe under
        // `domain_id == 250` and emits section G under `units_domains` would
        // pass every assertion below. The separate arm is a design choice
        // FIX.md argues for; this test pins the behaviour, not the shape.
        let mut sequence = 1u64;
        let stream = mixed_units_frame(&mut sequence, HOUR_A);
        let report = audit_with_units(&stream, &[RC_PROBE_DOMAIN_ID]);

        assert_eq!(
            report.section_a_frame_census.frames_with_lane_wall_probe, 1,
            "requesting the probe domain must not disturb the probe"
        );
        assert!(
            report.parse_errors.is_empty(),
            "and must not look like a second probe record: {:?}",
            report.parse_errors
        );
        let g = &report.section_g_domain_units;
        assert_eq!(g.perf_records_matched, 1);
        assert_eq!(g.rows.len(), 1);
        assert_eq!(g.rows[0].domain_id, RC_PROBE_DOMAIN_ID);
        assert!(g.rows[0]
            .units
            .as_ref()
            .expect("the probe record carried a trailer")
            .iter()
            .any(|unit| unit.name == "lw_frames"));
    }

    #[test]
    fn the_printed_read_out_carries_every_field_the_json_carries() {
        // kills: printing a SUBSET of the per-record read-out this lane
        // documented. Section G promises "offset, sequence, hour, flags, and
        // every decoded unit (name, value, role, presence, rendered form)", and
        // the console formatter printed neither the record flags nor any unit's
        // role or presence - only the rendered shorthand (Codex r5). Nothing
        // could catch it, because every other test in this file asserts on the
        // report STRUCT and `print_report` wrote straight to a locked stdout.
        //
        // The fixture carries a non-default presence on purpose: an ABSENT unit
        // renders as `absent` with no number, so a formatter that dropped
        // `presence` and kept only `rendered` would still look right on a
        // counter and wrong on this one.
        use compact_contract::diagnostic_channel::{UnitPresence, UnitRole};

        let mut sequence = 1u64;
        let mut stream = perf_with_units(
            &mut sequence,
            HOUR_A,
            53,
            &[
                DiagUnit::counter("domain53_operative_skipped", 71),
                DiagUnit::new(
                    "domain53_agency_roster_members",
                    0,
                    UnitRole::Counter,
                    UnitPresence::Absent,
                ),
            ],
        );
        stream.extend_from_slice(&perf_without_units(&mut sequence, HOUR_A, 53));
        stream.extend_from_slice(&frame_end_record(&mut sequence, HOUR_A));
        let report = audit_with_units(&stream, &[53]);

        let mut printed = Vec::new();
        write_domain_unit_section(&mut printed, &report.section_g_domain_units)
            .expect("the section renders");
        let printed = String::from_utf8(printed).expect("the section is utf-8");

        let trailer_flags = format!("flags={COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER:#010x}");
        assert!(
            printed.contains(&trailer_flags),
            "the record's flag word must be on the row, as {trailer_flags}: {printed}"
        );
        assert!(
            printed.contains("flags=0x00000000"),
            "including the trailerless record's zero, which is not the \
             same thing as the row being absent: {printed}"
        );
        let observed = printed
            .lines()
            .find(|line| line.contains("name=domain53_operative_skipped"))
            .expect("the observed unit has a line of its own");
        for field in [
            "value=71",
            "role=counter",
            "presence=observed",
            "rendered=domain53_operative_skipped=71",
        ] {
            assert!(
                observed.contains(field),
                "the observed unit's line is missing {field}: {observed}"
            );
        }
        let absent = printed
            .lines()
            .find(|line| line.contains("name=domain53_agency_roster_members"))
            .expect("the absent unit has a line of its own");
        assert!(
            absent.contains("presence=absent"),
            "a non-default presence must be printed as such: {absent}"
        );
        assert!(
            absent.contains("rendered=domain53_agency_roster_members=absent"),
            "and its rendered form still says absent rather than 0: {absent}"
        );
        assert!(
            printed.contains("no diagnostic trailer"),
            "the trailerless row is still distinguishable from a zero: {printed}"
        );
        assert_eq!(
            printed.matches("flags=").count(),
            2,
            "BOTH rows carry flags, not just the one with units: {printed}"
        );
    }

    #[test]
    fn a_requested_domains_record_whose_framing_is_in_doubt_is_counted_not_decoded() {
        // kills: decoding a requested domain's units out of a record whose own
        // framing failed validation - and, with it, the claim that
        // `perf_records_skipped_semantic_error` is a live counter at all. Every
        // other assertion on it requires it to stay ZERO, so nothing in this
        // suite reached the arm that increments it and the documented decision
        // ("the record's own framing is in doubt, so its payload offsets are
        // too - counted, not dropped") had no test that could fail (Codex r2
        // finding 6).
        //
        // The break is the DOMAIN_PERF reserved base word, which
        // `validate_domain_perf` requires to be zero. The trailer behind it is
        // perfectly well-formed and carries two named units, so a reader that
        // ignored the semantic error would publish them - which is exactly the
        // behaviour the assertions below pin shut.
        let mut sequence = 1u64;
        let mut stream = perf_with_units(
            &mut sequence,
            HOUR_A,
            RC_PROBE_DOMAIN_ID,
            &probe_units(None, 0),
        );
        stream.extend_from_slice(&perf_with_broken_reserved_word(
            &mut sequence,
            HOUR_A,
            53,
            &[
                DiagUnit::counter("domain53_relation_uncommitted_candidate_base", 3),
                DiagUnit::counter("domain53_operative_skipped", 71),
            ],
        ));
        stream.extend_from_slice(&frame_end_record(&mut sequence, HOUR_A));
        let report = audit_with_units(&stream, &[53]);

        let g = &report.section_g_domain_units;
        assert_eq!(
            g.perf_records_skipped_semantic_error, 1,
            "the requested record reached the semantic-error arm"
        );
        assert_eq!(
            g.perf_records_matched, 0,
            "and was not counted among the records this section read"
        );
        assert!(
            g.rows.is_empty(),
            "no units may be published from a record whose payload offsets are \
             in doubt: {:?}",
            g.rows
        );
        assert_eq!(g.perf_records_with_decode_error, 0);
        assert_eq!(g.perf_records_without_trailer, 0);
        // And it is still NAMED where every malformed record is named. The
        // read-out declines to decode it; it does not hide it.
        assert!(
            report.parse_errors.iter().any(|error| {
                error.error.contains("invalid H4CR record semantics")
                    && error.error.contains("reserved base word")
            }),
            "the malformed record must still reach the global census: {:?}",
            report.parse_errors
        );
    }

    #[test]
    fn units_domain_reads_nothing_unless_it_is_asked_to() {
        // MUTATION GUARD, not a strict revert kill: it also passes if section G
        // is removed entirely, because the base produces no section G either
        // (Codex r1 finding 7, restated by r3 finding 5). It still catches the
        // change that would matter.
        //
        // guards: making the per-record read-out unconditional. Every caller of
        // this tool would then pay to build and hold a row per DOMAIN_PERF
        // record, and the JSON every existing consumer parses would grow a
        // section it never asked for.
        let mut sequence = 1u64;
        let stream = mixed_units_frame(&mut sequence, HOUR_A);
        let report = audit(&stream);

        let g = &report.section_g_domain_units;
        assert!(g.requested_domain_ids.is_empty());
        assert!(g.rows.is_empty());
        assert_eq!(g.perf_records_matched, 0);
        assert_eq!(
            report.section_a_frame_census.frames_with_lane_wall_probe, 1,
            "the default path is otherwise exactly what it was"
        );
    }
}

fn record_type_name(record_type: u16) -> &'static str {
    match record_type {
        COMPACT_RECORD_TYPE_DOMAIN_PERF => "DOMAIN_PERF",
        COMPACT_RECORD_TYPE_FRAME_END => "FRAME_END",
        COMPACT_RECORD_TYPE_DOMAIN_DATA => "DOMAIN_DATA",
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT => "DOMAIN_DATA_REPEAT",
        COMPACT_RECORD_TYPE_STREAM_HEADER => "STREAM_HEADER",
        COMPACT_RECORD_TYPE_DOMAIN_GAP => "DOMAIN_GAP",
        COMPACT_RECORD_TYPE_FRAME_REFUSED => "FRAME_REFUSED",
        COMPACT_RECORD_TYPE_FRAME_DEGRADED => "FRAME_DEGRADED",
        COMPACT_RECORD_TYPE_CAPTURE_TERMINATED => "CAPTURE_TERMINATED",
        compact_contract::COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL => {
            "CLOCK_UNREADABLE_INTERVAL"
        }
        compact_contract::COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY => {
            "UNTRUSTED_CLOCK_RECOVERY"
        }
        compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS => "FREEZE_CENSUS",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod freeze_census_name_tests {
    use super::*;

    #[test]
    fn freeze_census_has_a_name_and_type_twelve_does_not() {
        assert_eq!(
            record_type_name(compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS),
            "FREEZE_CENSUS"
        );
        assert_eq!(record_type_name(12), "UNKNOWN");
    }
}

#[cfg(test)]
mod repeat_census_tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_RECORD_MAGIC, COMPACT_RECORD_VERSION};
    use sha2::{Digest, Sha256};

    const REPEAT_HOUR_A: i32 = 60_831_056;
    const REPEAT_HOUR_B: i32 = 60_831_057;

    // Domain 10's declared volatile prefix is 8 (record_semantics.rs), so a
    // domain-10 repeat payload is exactly 8 + 32 bytes.
    const DOMAIN: u16 = 10;
    const PREFIX: usize = 8;

    fn raw_record(
        sequence: u64,
        hour: i32,
        domain: u16,
        rtype: u16,
        flags: u32,
        payload: &[u8],
    ) -> Vec<u8> {
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

    fn audit(stream: &[u8]) -> AuditReport {
        let mut state = AuditState::default();
        scan_stream(&mut &stream[..], &mut state).expect("scan");
        finish_report(Path::new("synthetic"), state)
    }

    fn full_record(sequence: u64, hour: i32, payload: &[u8]) -> Vec<u8> {
        let mut out = raw_record(
            sequence,
            hour,
            DOMAIN,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            payload,
        );
        out.extend_from_slice(&raw_record(
            sequence + 1,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; 32],
        ));
        out
    }

    fn repeat_record(
        sequence: u64,
        hour: i32,
        baseline_tail_digest: [u8; 32],
    ) -> Vec<u8> {
        let mut payload = vec![0u8; PREFIX];
        payload.extend_from_slice(&baseline_tail_digest);
        let mut out = raw_record(
            sequence,
            hour,
            DOMAIN,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT,
            0,
            &payload,
        );
        out.extend_from_slice(&raw_record(
            sequence + 1,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; 32],
        ));
        out
    }

    fn tail_digest(baseline_payload: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&baseline_payload[PREFIX..]);
        hasher.finalize().into()
    }

    fn gap_record(sequence: u64, hour: i32) -> Vec<u8> {
        let payload = DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: DomainGapReason::SourceUnavailable,
            target_hour_raw: hour,
            range_start_hour_raw: hour,
            range_end_hour_raw: hour,
            tick_before: 7,
            tick_after: 7,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        }
        .encode_with_witness_provenance(DomainGapWitnessProvenance::Unstated)
        .expect("fixture gap must be wire-encodable");
        raw_record(
            sequence,
            hour,
            DOMAIN,
            COMPACT_RECORD_TYPE_DOMAIN_GAP,
            0,
            &payload,
        )
    }

    #[test]
    fn verified_repeat_is_counted_and_consistent() {
        let baseline_payload = vec![7u8; 48];
        let mut stream = full_record(0, REPEAT_HOUR_A, &baseline_payload);
        stream.extend_from_slice(&repeat_record(2, REPEAT_HOUR_B, tail_digest(&baseline_payload)));
        let report = audit(&stream);
        assert_eq!(report.parse_error_count, 0, "{:?}", report.parse_errors);
        let f = &report.section_f_repeat_census;
        assert_eq!(f.domain_data_repeat_records, 1);
        assert_eq!(f.domains_with_repeats, 1);
        assert_eq!(
            f.baseline_errors_total, 0,
            "{:?}",
            f.baseline_error_examples
        );
        assert_eq!(f.per_domain.len(), 1);
        let row = &f.per_domain[0];
        assert_eq!(row.domain_id, DOMAIN);
        assert_eq!(row.repeat_records, 1);
        assert_eq!(row.first_repeat_hour_raw, Some(REPEAT_HOUR_B));
        assert_eq!(row.last_repeat_hour_raw, Some(REPEAT_HOUR_B));
        assert_eq!(row.full_data_records, 1);
        assert_eq!(row.longest_repeat_run, 1);
        assert_eq!(row.keyframe_cadence_violations, 0);
        assert_eq!(row.repeats_after_named_gap, 0);
    }

    #[test]
    fn repeat_with_corrupt_digest_is_refused_by_name() {
        let baseline_payload = vec![7u8; 48];
        let mut stream = full_record(0, REPEAT_HOUR_A, &baseline_payload);
        stream.extend_from_slice(&repeat_record(2, REPEAT_HOUR_B, [0u8; 32]));
        let report = audit(&stream);
        assert_eq!(
            report.section_f_repeat_census.baseline_errors_total, 1,
            "{:?}",
            report.parse_errors
        );
        assert!(report
            .parse_errors
            .iter()
            .any(|error| error.error.contains("fails its baseline digest")));
    }

    #[test]
    fn repeat_without_a_baseline_is_refused_by_name() {
        let stream = repeat_record(0, REPEAT_HOUR_A, [9u8; 32]);
        let report = audit(&stream);
        assert_eq!(report.section_f_repeat_census.baseline_errors_total, 1);
        assert!(report
            .parse_errors
            .iter()
            .any(|error| error.error.contains("no stored baseline")));
    }

    #[test]
    fn cadence_overdue_repeat_is_flagged_but_in_cadence_is_not() {
        let baseline_payload = vec![7u8; 48];
        // One hour short of the cadence boundary: legal.
        let mut stream = full_record(0, REPEAT_HOUR_A, &baseline_payload);
        let last_legal =
            REPEAT_HOUR_A + (COMPACT_DOMAIN_DATA_REPEAT_KEYFRAME_CADENCE_HOURS as i32) - 1;
        stream.extend_from_slice(&repeat_record(2, last_legal, tail_digest(&baseline_payload)));
        let report = audit(&stream);
        assert_eq!(report.parse_error_count, 0, "{:?}", report.parse_errors);
        // At the boundary itself the keyframe is overdue.
        let mut stream = full_record(0, REPEAT_HOUR_A, &baseline_payload);
        let overdue = REPEAT_HOUR_A + (COMPACT_DOMAIN_DATA_REPEAT_KEYFRAME_CADENCE_HOURS as i32);
        stream.extend_from_slice(&repeat_record(2, overdue, tail_digest(&baseline_payload)));
        let report = audit(&stream);
        let row = &report.section_f_repeat_census.per_domain[0];
        assert_eq!(row.keyframe_cadence_violations, 1);
        assert!(report
            .parse_errors
            .iter()
            .any(|error| error.error.contains("keyframe cadence")));
    }

    #[test]
    fn repeat_after_a_named_gap_is_refused_by_name() {
        let baseline_payload = vec![7u8; 48];
        let mut stream = full_record(0, REPEAT_HOUR_A, &baseline_payload);
        stream.extend_from_slice(&gap_record(2, REPEAT_HOUR_B));
        stream.extend_from_slice(&raw_record(
            3,
            REPEAT_HOUR_B,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; 32],
        ));
        stream.extend_from_slice(&repeat_record(4, REPEAT_HOUR_B, tail_digest(&baseline_payload)));
        let report = audit(&stream);
        let row = &report.section_f_repeat_census.per_domain[0];
        assert_eq!(row.repeats_after_named_gap, 1);
        assert!(report
            .parse_errors
            .iter()
            .any(|error| error.error.contains("force-full law")));
    }

    #[test]
    fn gate_excluded_domain_repeat_is_refused_and_never_becomes_a_census_row() {
        // The RC probe domain 250 is PERF-only and never gated. Its declared
        // prefix is 0, so this repeat is ENVELOPE-valid (domain not 0/1000,
        // length exactly 32) yet can never be a legitimate gate emission.
        let mut stream = raw_record(
            0,
            REPEAT_HOUR_A,
            RC_PROBE_DOMAIN_ID,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT,
            0,
            &[0u8; 32],
        );
        stream.extend_from_slice(&raw_record(
            1,
            REPEAT_HOUR_A,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0u8; 32],
        ));
        let report = audit(&stream);
        assert!(report.parse_errors.iter().any(|error| {
            error.error.contains("gate-excluded domain")
        }), "{:?}", report.parse_errors);
        let f = &report.section_f_repeat_census;
        assert_eq!(f.gate_excluded_repeat_records, 1);
        assert_eq!(f.domain_data_repeat_records, 0);
        assert_eq!(f.baseline_errors_total, 0);
        assert!(f.per_domain.is_empty());
    }
}
