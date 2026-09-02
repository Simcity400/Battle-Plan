use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
use compact_contract::diagnostic_channel::{
    decode_units, DiagUnit, FRAME_TAIL_SEGMENTS, OUTSIDE_BUILD_SEGMENTS,
};
#[cfg(test)]
use compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED;
use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, KnownRecordType,
    COMPACT_DOMAIN_PERF_BASE_LEN, COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC,
    COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2, COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

const RC_PROBE_DOMAIN_ID: u16 = 250;
const TAIL_DEPTH: usize = 12;
const DOMAIN_TABLE_LIMIT: usize = 24;
/// The retired lane-wall tag of the country bundle's gated parent (lane PS,
/// 2026-09-02, unbundled the eight children into gated workers under their own
/// domain ids). A stream recorded before that carries the parent's critical
/// busy time under this tag and nothing under the children's; counting it as
/// country critical time is what makes `country_crit_us` comparable across the
/// change.
const RETIRED_COUNTRY_PARENT_TAG: u16 = 200;

fn main() {
    if let Err(error) = run() {
        eprintln!("sweep_span_audit: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let Args {
        input,
        json_output,
        domain,
    } = parse_args()?;
    let file = File::open(&input).map_err(|error| format!("open {}: {error}", input.display()))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut state = ScanState {
        inspect_domain: domain,
        ..ScanState::default()
    };
    scan_stream(&mut reader, &mut state)?;
    let report = state.finish(input.display().to_string());
    print_report(&report);
    if let Some(path) = json_output {
        let file =
            File::create(&path).map_err(|error| format!("create {}: {error}", path.display()))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &report)
            .map_err(|error| format!("encode {}: {error}", path.display()))?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    Ok(())
}

struct Args {
    input: PathBuf,
    json_output: Option<PathBuf>,
    domain: Option<u16>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = env::args_os().skip(1);
    let input = args.next().map(PathBuf::from).ok_or_else(usage)?;
    let mut json_output = None;
    let mut domain = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--json") => {
                if json_output.is_some() {
                    return Err(format!("--json may be given only once\n{}", usage()));
                }
                json_output = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| format!("--json needs an output path\n{}", usage()))?,
                );
            }
            Some("--domain") => {
                if domain.is_some() {
                    return Err(format!("--domain may be given only once\n{}", usage()));
                }
                let raw = args
                    .next()
                    .ok_or_else(|| format!("--domain needs a domain id\n{}", usage()))?;
                domain = Some(
                    raw.to_str()
                        .and_then(|text| text.parse::<u16>().ok())
                        .ok_or_else(|| format!("--domain {raw:?} is not a domain id"))?,
                );
            }
            _ => return Err(format!("unknown argument {:?}\n{}", arg, usage())),
        }
    }
    Ok(Args {
        input,
        json_output,
        domain,
    })
}

fn usage() -> String {
    "usage: sweep_span_audit <collector_stream.bin> [--json out.json] [--domain N]".to_string()
}

#[derive(Clone, Copy)]
struct PerfRow {
    domain_id: u16,
    read_time_us: u64,
    sequence: u64,
}

#[derive(Default)]
struct FrameAccumulator {
    probe: Option<ProbeUnits>,
    perf: Vec<PerfRow>,
}

impl FrameAccumulator {
    fn is_empty(&self) -> bool {
        self.probe.is_none() && self.perf.is_empty()
    }
}

#[derive(Clone, Default)]
struct ProbeUnits {
    worker_span_us: Option<u64>,
    prewarm_us: Option<u64>,
    fanout_origin_us: Option<u64>,
    crit_wall_us: Option<u64>,
    crit_busy_us: Option<u64>,
    main_wall_us: Option<u64>,
    mismatch_domains: Option<u64>,
    bracketed_domains: Option<u64>,
    frame_time_us: Option<u64>,
    pub_start_us: Option<u64>,
    pub_end_us: Option<u64>,
    readers_done_us: Option<u64>,
    scope_joined_us: Option<u64>,
    lanes: Vec<(u16, u64, u64)>,
    outside: BTreeMap<u16, u64>,
    frame_tail: BTreeMap<u16, u64>,
    crit_domains: Vec<(u16, u64)>,
    /// `lw_cb{tag}_us`: each country child's own read time, by domain id.
    country_reads: Vec<(u16, u64)>,
    lanes_truncated: bool,
    /// Unit names that occurred more than once -- ANY name, the fixed ones
    /// and the `lw_o*` / `lw_t*` / `lw_l*` / `lw_d*` families alike. A probe
    /// is one record with one value per name; a second occurrence is a
    /// malformed probe, and the frame is refused rather than read last-wins
    /// (Codex rounds 5 and 6).
    duplicated_units: Vec<String>,
}

fn probe_units_from(units: &[DiagUnit]) -> ProbeUnits {
    let mut probe = ProbeUnits::default();
    let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for unit in units {
        let name = unit.name.as_str();
        let value = unit.value;
        if !seen.insert(name) {
            probe.duplicated_units.push(name.to_string());
        }
        match name {
            "lw_worker_span_us" => probe.worker_span_us = Some(value),
            "lw_prewarm_us" => probe.prewarm_us = Some(value),
            "lw_fanout_origin_us" => probe.fanout_origin_us = Some(value),
            "lw_crit_wall_us" => probe.crit_wall_us = Some(value),
            "lw_crit_busy_us" => probe.crit_busy_us = Some(value),
            "lw_lmain_wall_us" => probe.main_wall_us = Some(value),
            "lw_hour_label_mismatch_domains" => probe.mismatch_domains = Some(value),
            "lw_bracketed_domains" => probe.bracketed_domains = Some(value),
            "lw_ft_us" => probe.frame_time_us = Some(value),
            "lw_pub_start_us" => probe.pub_start_us = Some(value),
            "lw_pub_end_us" => probe.pub_end_us = Some(value),
            "lw_readers_done_us" => probe.readers_done_us = Some(value),
            "lw_scope_joined_us" => probe.scope_joined_us = Some(value),
            "lw_lother_wall_us" => probe.lanes_truncated = true,
            _ => {
                // The contract's own prefix and suffix, not a second copy of
                // them: this binary spelled `"lw_o"` and `"_us"` itself, so
                // editing the table's `unit_prefix` renamed every span and
                // left this looking for the old name (Codex round 32).
                if let Some(code) = name
                    .strip_prefix(
                        compact_contract::diagnostic_channel::OUTSIDE_BUILD_UNIT_PREFIX)
                    .and_then(|rest| rest.strip_suffix(
                        compact_contract::diagnostic_channel::OUTSIDE_BUILD_UNIT_SUFFIX))
                    .and_then(|digits| digits.parse::<u16>().ok())
                {
                    probe.outside.insert(code, value);
                } else if let Some(code) = name
                    .strip_prefix(
                        compact_contract::diagnostic_channel::FRAME_TAIL_UNIT_PREFIX)
                    .and_then(|rest| rest.strip_suffix(
                        compact_contract::diagnostic_channel::FRAME_TAIL_UNIT_SUFFIX))
                    .and_then(|digits| digits.parse::<u16>().ok())
                {
                    probe.frame_tail.insert(code, value);
                } else if let Some(tag) = name
                    .strip_prefix(
                        compact_contract::diagnostic_channel::DOMAIN_CRIT_UNIT_PREFIX)
                    .and_then(|rest| rest.strip_suffix(
                        compact_contract::diagnostic_channel::DOMAIN_CRIT_UNIT_SUFFIX))
                    .and_then(|digits| digits.parse::<u16>().ok())
                {
                    probe.crit_domains.push((tag, value));
                } else if let Some(tag) = name
                    .strip_prefix(compact_contract::diagnostic_channel::CALLBACK_UNIT_PREFIX)
                    .and_then(|rest| rest.strip_suffix(
                        compact_contract::diagnostic_channel::CALLBACK_UNIT_SUFFIX))
                    .and_then(|digits| digits.parse::<u16>().ok())
                {
                    probe.country_reads.push((tag, value));
                } else if let Some(lane) = name
                    .strip_prefix(
                        compact_contract::diagnostic_channel::LANE_WALL_UNIT_PREFIX)
                    .and_then(|rest| rest.strip_suffix(
                        compact_contract::diagnostic_channel::LANE_WALL_UNIT_SUFFIX))
                    .and_then(|digits| digits.parse::<u16>().ok())
                {
                    upsert_lane(&mut probe.lanes, lane).1 = value;
                } else if let Some(lane) = name
                    .strip_prefix(
                        compact_contract::diagnostic_channel::LANE_WALL_UNIT_PREFIX)
                    .and_then(|rest| rest.strip_suffix(
                        compact_contract::diagnostic_channel::LANE_BUSY_UNIT_SUFFIX))
                    .and_then(|digits| digits.parse::<u16>().ok())
                {
                    upsert_lane(&mut probe.lanes, lane).2 = value;
                }
            }
        }
    }
    probe
}

fn upsert_lane(lanes: &mut Vec<(u16, u64, u64)>, lane: u16) -> &mut (u16, u64, u64) {
    if let Some(index) = lanes.iter().position(|entry| entry.0 == lane) {
        return &mut lanes[index];
    }
    lanes.push((lane, 0, 0));
    lanes.last_mut().expect("just pushed")
}

#[derive(Clone, Serialize)]
struct FrameSpan {
    span_us: u64,
    prewarm_us: u64,
    fanout_origin_us: u64,
    main_wall_us: u64,
    crit_wall_us: u64,
    crit_busy_us: u64,
    lane_count: u64,
    lane_busy_sum_us: u64,
    lane_wall_sum_us: u64,
    uncovered_us: u64,
    balanced_us: u64,
    longest_job_us: u64,
    makespan_floor_us: u64,
    job_total_us: u64,
    job_count: u64,
    mismatch_domains: u64,
    bracketed_domains: u64,
    outside_total_us: u64,
    outside_wait_us: u64,
    /// The country children this frame read (one `lw_cb{tag}_us` each), the
    /// sum and the maximum of their own read times, and the critical-lane busy
    /// time the frame charged to country tags -- the children's own domain
    /// ids, or the retired parent tag on a stream recorded before lane PS.
    country_child_count: u64,
    country_read_sum_us: u64,
    country_read_max_us: u64,
    country_crit_us: u64,
    frame_time_us: u64,
    /// `lw_ft_us - lw_fanout_origin_us - lw_worker_span_us`: the serial tail
    /// between the SPAN close and the frame-time close -- the interval the
    /// seven `lw_t*_us` marks partition, by a different instrument. The span
    /// closes after the scope join AND the post-scope main-thread reads, which
    /// `post_scope_main_us` carries separately.
    serial_tail_us: u64,
    frame_tail_sum_us: u64,
    deferred_publication_us: u64,
    /// `lw_readers_done_us` / `lw_scope_joined_us`: when the last reader
    /// joined and when the scope (readers plus the previous hour's publisher)
    /// joined, on the lane origin. Absent on streams recorded before the units
    /// existed.
    readers_done_us: Option<u64>,
    scope_joined_us: Option<u64>,
    /// `lw_worker_span_us - lw_scope_joined_us`: the main-thread reads between
    /// the scope join and the span close. Not a wait, and not in any `lw_t*`.
    post_scope_main_us: Option<u64>,
    /// `lw_scope_joined_us - lw_readers_done_us` on a frame that ran a
    /// publisher: what the sweep thread waited after its readers for the scope
    /// to join the previous hour's publisher. `None` when no publisher ran --
    /// the same interval is then the join's own overhead, not a wait.
    publisher_wait_us: Option<u64>,
    /// `lw_pub_end_us - lw_readers_done_us`: how far the previous hour's sink
    /// write outlived this frame's readers. Zero when it finished inside them.
    deferred_publication_beyond_readers_us: Option<u64>,
    lanes_truncated: bool,
}

#[derive(Default)]
struct ScanState {
    bytes_consumed: u64,
    records_seen: u64,
    freeze_census_records: u64,
    unknown_record_type_records: u64,
    parse_errors: Vec<String>,
    current: FrameAccumulator,
    frames: Vec<FrameSpan>,
    frames_seen: u64,
    frames_without_probe: u64,
    frames_with_duplicate_units: u64,
    domain_read_us: BTreeMap<u16, Vec<u64>>,
    tail_rank_counts: BTreeMap<u16, BTreeMap<usize, u64>>,
    bundle_groups: BTreeMap<Vec<u16>, u64>,
    lane_busy_us: BTreeMap<u16, Vec<u64>>,
    lane_wall_us: BTreeMap<u16, Vec<u64>>,
    crit_domain_us: BTreeMap<u16, Vec<u64>>,
    outside_us: BTreeMap<u16, Vec<u64>>,
    country_read_us: BTreeMap<u16, Vec<u64>>,
    frame_tail_us: BTreeMap<u16, Vec<u64>>,
    inspect_domain: Option<u16>,
    inspect_units: BTreeMap<String, Vec<u64>>,
    inspect_read_us: Vec<u64>,
}

fn scan_stream(reader: &mut impl Read, state: &mut ScanState) -> Result<(), String> {
    let mut header_bytes = [0u8; COMPACT_RECORD_HEADER_LEN];
    loop {
        let offset = state.bytes_consumed;
        let header_read = read_up_to(reader, &mut header_bytes)
            .map_err(|error| format!("read header at {offset}: {error}"))?;
        state.bytes_consumed = state.bytes_consumed.saturating_add(header_read as u64);
        if header_read == 0 {
            break;
        }
        if header_read != COMPACT_RECORD_HEADER_LEN {
            state
                .parse_errors
                .push(format!("truncated header at {offset}"));
            break;
        }
        let header = match parse_compact_record_header(&header_bytes) {
            Ok(header) => header,
            Err(error) => {
                state
                    .parse_errors
                    .push(format!("bad header at {offset}: {error}"));
                break;
            }
        };
        let mut payload = Vec::new();
        payload
            .try_reserve_exact(header.payload_len)
            .map_err(|error| {
                format!("reserve {} bytes at {offset}: {error}", header.payload_len)
            })?;
        payload.resize(header.payload_len, 0);
        let payload_read = read_up_to(reader, &mut payload)
            .map_err(|error| format!("read payload at {offset}: {error}"))?;
        state.bytes_consumed = state.bytes_consumed.saturating_add(payload_read as u64);
        if payload_read != header.payload_len {
            state
                .parse_errors
                .push(format!("truncated payload at {offset}"));
            break;
        }
        state.records_seen = state.records_seen.saturating_add(1);
        process_record(state, &header, &payload);
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

fn process_record(
    state: &mut ScanState,
    header: &compact_contract::CompactRecordHeader,
    payload: &[u8],
) {
    if header.record_type == COMPACT_RECORD_TYPE_STREAM_HEADER {
        return;
    }
    let known_record_type = match KnownRecordType::try_from(header.record_type) {
        Ok(record_type) => record_type,
        Err(error) => {
            state.unknown_record_type_records = state.unknown_record_type_records.saturating_add(1);
            state.parse_errors.push(format!(
                "unknown record type at sequence {}: {error}",
                header.sequence
            ));
            return;
        }
    };
    let semantics = validate_compact_record_semantics(
        header.domain_id,
        header.record_type,
        header.record_flags,
        payload,
    );
    let semantics_ok = semantics.is_ok();

    if known_record_type == KnownRecordType::FreezeCensus {
        state.freeze_census_records = state.freeze_census_records.saturating_add(1);
        if let Err(error) = semantics {
            state.parse_errors.push(format!(
                "FREEZE_CENSUS at sequence {} is unreadable: {error}",
                header.sequence
            ));
        }
        return;
    }

    if known_record_type == KnownRecordType::FrameRefused {
        if let Err(error) = semantics {
            state.parse_errors.push(format!(
                "FRAME_REFUSED at sequence {} dropped the open frame but its payload is \
                 unreadable: {error}",
                header.sequence
            ));
        }
        state.current = FrameAccumulator::default();
        return;
    }

    if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF && semantics_ok {
        if header.domain_id == RC_PROBE_DOMAIN_ID {
            match decode_probe_units(header.record_flags, payload) {
                Ok(Some(units)) if units.iter().any(|unit| unit.name == "lw_frames") => {
                    state.current.probe = Some(probe_units_from(&units));
                }
                Ok(_) => {}
                Err(error) => state.parse_errors.push(error),
            }
        } else if let Some(read_time_us) = perf_read_time_us(payload) {
            state.current.perf.push(PerfRow {
                domain_id: header.domain_id,
                read_time_us,
                sequence: header.sequence,
            });
            if state.inspect_domain == Some(header.domain_id) {
                state.inspect_read_us.push(read_time_us);
                if let Ok(Some(units)) = decode_probe_units(header.record_flags, payload) {
                    for unit in units {
                        state
                            .inspect_units
                            .entry(unit.name.to_string())
                            .or_default()
                            .push(unit.value);
                    }
                }
            }
        }
    }

    if header.record_type == COMPACT_RECORD_TYPE_FRAME_END {
        finalize_frame(state);
    }
}

fn perf_read_time_us(payload: &[u8]) -> Option<u64> {
    if payload.len() < COMPACT_DOMAIN_PERF_BASE_LEN {
        return None;
    }
    let bytes: [u8; 8] = payload.get(8..16)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn decode_probe_units(record_flags: u32, payload: &[u8]) -> Result<Option<Vec<DiagUnit>>, String> {
    if record_flags & COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER == 0 {
        return Ok(None);
    }
    let trailer = payload
        .get(COMPACT_DOMAIN_PERF_BASE_LEN..)
        .ok_or_else(|| "PERF payload shorter than the canonical base".to_string())?;
    if trailer.len() < 8 || &trailer[..4] != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC {
        return Err("PDU1 trailer missing or misframed".to_string());
    }
    let version = u16::from_le_bytes([trailer[4], trailer[5]]);
    if version != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2 {
        return Err(format!(
            "PDU1 trailer version {version} is not decodable here"
        ));
    }
    let (units, _consumed) = decode_units(&trailer[6..])?;
    Ok(Some(units))
}

fn collapse_bundles(perf: &[PerfRow]) -> Vec<(Vec<u16>, u64)> {
    let mut jobs: Vec<(Vec<u16>, u64)> = Vec::new();
    for row in perf {
        match jobs.last_mut() {
            Some((members, cost)) if *cost == row.read_time_us && row.read_time_us != 0 => {
                members.push(row.domain_id);
            }
            _ => jobs.push((vec![row.domain_id], row.read_time_us)),
        }
    }
    jobs
}

fn derive_frame_span(probe: &ProbeUnits, perf_by_sequence: &[PerfRow]) -> Option<FrameSpan> {
    let span_us = probe.worker_span_us?;
    let main_wall_us = probe.main_wall_us.unwrap_or(0);
    let crit_wall_us = probe.crit_wall_us.unwrap_or(0);
    let lane_count = probe.lanes.len() as u64;
    let lane_busy_sum_us: u64 = probe.lanes.iter().map(|entry| entry.2).sum();
    let lane_wall_sum_us: u64 = probe.lanes.iter().map(|entry| entry.1).sum();
    let jobs = collapse_bundles(perf_by_sequence);
    let longest_job_us = jobs.iter().map(|(_, cost)| *cost).max().unwrap_or(0);
    let job_total_us: u64 = jobs.iter().map(|(_, cost)| *cost).sum();
    let balanced_us = if lane_count == 0 {
        0
    } else {
        lane_busy_sum_us / lane_count
    };
    let country_read_sum_us: u64 = probe.country_reads.iter().map(|(_, us)| *us).sum();
    let country_read_max_us = probe
        .country_reads
        .iter()
        .map(|(_, us)| *us)
        .max()
        .unwrap_or(0);
    let country_crit_us: u64 = probe
        .crit_domains
        .iter()
        .filter(|(tag, _)| {
            *tag == RETIRED_COUNTRY_PARENT_TAG
                || probe.country_reads.iter().any(|(read_tag, _)| read_tag == tag)
        })
        .map(|(_, us)| *us)
        .sum();
    let frame_time_us = probe.frame_time_us.unwrap_or(0);
    let fanout_origin_us = probe.fanout_origin_us.unwrap_or(0);
    let frame_tail_sum_us: u64 = probe
        .frame_tail
        .values()
        .fold(0u64, |acc, value| acc.saturating_add(*value));
    let deferred_publication_us = match (probe.pub_start_us, probe.pub_end_us) {
        (Some(start), Some(end)) => end.saturating_sub(start),
        _ => 0,
    };
    let readers_done_us = probe.readers_done_us;
    let scope_joined_us = probe.scope_joined_us;
    let post_scope_main_us = scope_joined_us.map(|joined| span_us.saturating_sub(joined));
    // The wait is the join interval only when the publisher outlived the
    // readers' close; a publisher that had already ended was not waited for,
    // and the interval is then the sweep thread's own cleanup (Codex round 5).
    let publisher_wait_us = match (readers_done_us, scope_joined_us, probe.pub_end_us) {
        (Some(done), Some(joined), Some(end)) if end > done => Some(joined.saturating_sub(done)),
        (Some(_), Some(_), Some(_)) => Some(0),
        _ => None,
    };
    let deferred_publication_beyond_readers_us = match (readers_done_us, probe.pub_end_us) {
        (Some(done), Some(end)) => Some(end.saturating_sub(done)),
        _ => None,
    };
    Some(FrameSpan {
        span_us,
        prewarm_us: probe.prewarm_us.unwrap_or(0),
        fanout_origin_us: probe.fanout_origin_us.unwrap_or(0),
        main_wall_us,
        crit_wall_us,
        crit_busy_us: probe.crit_busy_us.unwrap_or(0),
        lane_count,
        lane_busy_sum_us,
        lane_wall_sum_us,
        uncovered_us: span_us
            .saturating_sub(main_wall_us)
            .saturating_sub(crit_wall_us),
        balanced_us,
        longest_job_us,
        makespan_floor_us: longest_job_us.max(balanced_us),
        job_total_us,
        job_count: jobs.len() as u64,
        mismatch_domains: probe.mismatch_domains.unwrap_or(0),
        bracketed_domains: probe.bracketed_domains.unwrap_or(0),
        outside_total_us: probe.outside.get(&0).copied().unwrap_or(0),
        outside_wait_us: probe.outside.get(&1).copied().unwrap_or(0),
        country_child_count: probe.country_reads.len() as u64,
        country_read_sum_us,
        country_read_max_us,
        country_crit_us,
        frame_time_us,
        serial_tail_us: frame_time_us
            .saturating_sub(fanout_origin_us)
            .saturating_sub(span_us),
        frame_tail_sum_us,
        deferred_publication_us,
        readers_done_us,
        scope_joined_us,
        post_scope_main_us,
        publisher_wait_us,
        deferred_publication_beyond_readers_us,
        lanes_truncated: probe.lanes_truncated,
    })
}

fn finalize_frame(state: &mut ScanState) {
    let frame = std::mem::take(&mut state.current);
    if frame.is_empty() {
        return;
    }
    state.frames_seen = state.frames_seen.saturating_add(1);
    let Some(probe) = frame.probe.clone() else {
        state.frames_without_probe = state.frames_without_probe.saturating_add(1);
        return;
    };
    if probe.worker_span_us.is_none() {
        state.frames_without_probe = state.frames_without_probe.saturating_add(1);
        return;
    }
    if !probe.duplicated_units.is_empty() {
        state.frames_with_duplicate_units = state.frames_with_duplicate_units.saturating_add(1);
        state.parse_errors.push(format!(
            "frame {}: the lane-wall probe carries {:?} more than once; refused rather than read last-wins",
            state.frames_seen, probe.duplicated_units
        ));
        return;
    }

    for &(lane, wall, busy) in &probe.lanes {
        state.lane_busy_us.entry(lane).or_default().push(busy);
        state.lane_wall_us.entry(lane).or_default().push(wall);
    }
    for &(tag, busy) in &probe.crit_domains {
        state.crit_domain_us.entry(tag).or_default().push(busy);
    }
    for (&code, &value) in &probe.outside {
        state.outside_us.entry(code).or_default().push(value);
    }
    for &(tag, read_us) in &probe.country_reads {
        state.country_read_us.entry(tag).or_default().push(read_us);
    }
    for (&code, &value) in &probe.frame_tail {
        state.frame_tail_us.entry(code).or_default().push(value);
    }

    let mut sorted = frame.perf.clone();
    sorted.sort_by_key(|row| row.sequence);
    for (rank_from_end, row) in sorted.iter().rev().take(TAIL_DEPTH).enumerate() {
        *state
            .tail_rank_counts
            .entry(row.domain_id)
            .or_default()
            .entry(rank_from_end)
            .or_default() += 1;
    }
    for row in &sorted {
        state
            .domain_read_us
            .entry(row.domain_id)
            .or_default()
            .push(row.read_time_us);
    }
    for (members, _cost) in &collapse_bundles(&sorted) {
        if members.len() > 1 {
            *state.bundle_groups.entry(members.clone()).or_default() += 1;
        }
    }
    if let Some(frame_span) = derive_frame_span(&probe, &sorted) {
        state.frames.push(frame_span);
    }
}

#[derive(Clone, Serialize)]
struct Stats {
    count: u64,
    min: u64,
    p25: u64,
    p50: u64,
    p75: u64,
    p90: u64,
    max: u64,
    mean: f64,
}

fn stats(values: &[u64]) -> Stats {
    if values.is_empty() {
        return Stats {
            count: 0,
            min: 0,
            p25: 0,
            p50: 0,
            p75: 0,
            p90: 0,
            max: 0,
            mean: 0.0,
        };
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let at = |q: f64| sorted[(((sorted.len() - 1) as f64) * q).round() as usize];
    let sum: u128 = sorted.iter().map(|value| u128::from(*value)).sum();
    Stats {
        count: sorted.len() as u64,
        min: sorted[0],
        p25: at(0.25),
        p50: at(0.50),
        p75: at(0.75),
        p90: at(0.90),
        max: sorted[sorted.len() - 1],
        mean: (sum as f64) / (sorted.len() as f64),
    }
}

#[derive(Serialize)]
struct DomainCost {
    domain_id: u16,
    read_time_us: Stats,
    tail_positions: Vec<(usize, u64)>,
}

#[derive(Serialize)]
struct BundleGroup {
    members: Vec<u16>,
    frames: u64,
}

#[derive(Serialize)]
struct LaneCost {
    lane: u16,
    busy_us: Stats,
    wall_us: Stats,
}

#[derive(Serialize)]
struct OutsideSegment {
    code: u16,
    name: String,
    us: Stats,
}

#[derive(Serialize)]
struct Report {
    input_path: String,
    collector_build: serde_json::Value,
    bytes_consumed: u64,
    records_seen: u64,
    freeze_census_records: u64,
    unknown_record_type_records: u64,
    parse_errors: Vec<String>,
    frames_seen: u64,
    frames_without_probe: u64,
    frames_with_duplicate_units: u64,
    frames_analysed: u64,
    frames_with_truncated_lanes: u64,
    span_us: Stats,
    prewarm_us: Stats,
    fanout_origin_us: Stats,
    main_wall_us: Stats,
    crit_wall_us: Stats,
    crit_busy_us: Stats,
    uncovered_us: Stats,
    balanced_us: Stats,
    longest_job_us: Stats,
    makespan_floor_us: Stats,
    job_total_us: Stats,
    job_count: Stats,
    lane_count: Stats,
    lane_busy_sum_us: Stats,
    lane_wall_sum_us: Stats,
    mismatch_domains: Stats,
    bracketed_domains: Stats,
    outside_total_us: Stats,
    outside_wait_us: Stats,
    gap_above_lower_bound_us: Stats,
    frame_time_us: Stats,
    serial_tail_us: Stats,
    frame_tail_sum_us: Stats,
    deferred_publication_us: Stats,
    /// Frames whose probe carried the scope marks (`lw_readers_done_us` and
    /// `lw_scope_joined_us`), and among them the frames that ran a publisher;
    /// `post_scope_main_us` is over the former, the two wait columns over the
    /// latter.
    frames_with_scope_marks: u64,
    frames_with_publisher: u64,
    post_scope_main_us: Stats,
    publisher_wait_us: Stats,
    deferred_publication_beyond_readers_us: Stats,
    domains: Vec<DomainCost>,
    bundle_groups: Vec<BundleGroup>,
    lanes: Vec<LaneCost>,
    outside_segments: Vec<OutsideSegment>,
    frame_tail_segments: Vec<OutsideSegment>,
    crit_domains: Vec<DomainCost>,
    /// Per country domain: that child's own read time (`lw_cb{tag}_us`).
    country_reads: Vec<DomainCost>,
    /// Per frame: children read, the sum and maximum of their read times, and
    /// the critical-lane busy time charged to country tags (children's own
    /// ids, or the retired parent tag 200 on an older stream).
    country_child_count: Stats,
    country_read_sum_us: Stats,
    country_read_max_us: Stats,
    country_crit_us: Stats,
    /// Frames in which any country tag was on the critical lane.
    country_crit_frames: u64,
    inspect_domain: Option<u16>,
    inspect_read_time_us: Stats,
    inspect_units: Vec<InspectUnit>,
}

#[derive(Serialize)]
struct InspectUnit {
    name: String,
    stats: Stats,
}

impl ScanState {
    fn finish(self, input_path: String) -> Report {
        let column = |get: fn(&FrameSpan) -> u64| -> Stats {
            stats(&self.frames.iter().map(get).collect::<Vec<_>>())
        };
        let lane_column = |get: fn(&FrameSpan) -> u64| -> Stats {
            stats(
                &self
                    .frames
                    .iter()
                    .filter(|frame| !frame.lanes_truncated)
                    .map(get)
                    .collect::<Vec<_>>(),
            )
        };
        let frames_with_truncated_lanes = self
            .frames
            .iter()
            .filter(|frame| frame.lanes_truncated)
            .count() as u64;
        let gap_above_lower_bound: Vec<u64> = self
            .frames
            .iter()
            .filter(|frame| !frame.lanes_truncated)
            .map(|frame| frame.span_us.saturating_sub(frame.makespan_floor_us))
            .collect();
        let mut domains: Vec<DomainCost> = self
            .domain_read_us
            .iter()
            .map(|(&domain_id, samples)| DomainCost {
                domain_id,
                read_time_us: stats(samples),
                tail_positions: self
                    .tail_rank_counts
                    .get(&domain_id)
                    .map(|ranks| ranks.iter().map(|(&rank, &count)| (rank, count)).collect())
                    .unwrap_or_default(),
            })
            .collect();
        domains.sort_by(|a, b| b.read_time_us.p50.cmp(&a.read_time_us.p50));
        let mut crit_domains: Vec<DomainCost> = self
            .crit_domain_us
            .iter()
            .map(|(&domain_id, samples)| DomainCost {
                domain_id,
                read_time_us: stats(samples),
                tail_positions: Vec::new(),
            })
            .collect();
        crit_domains.sort_by(|a, b| b.read_time_us.p50.cmp(&a.read_time_us.p50));
        let mut bundle_groups: Vec<BundleGroup> = self
            .bundle_groups
            .iter()
            .map(|(members, &frames)| BundleGroup {
                members: members.clone(),
                frames,
            })
            .collect();
        bundle_groups.sort_by(|a, b| b.frames.cmp(&a.frames));
        let lanes: Vec<LaneCost> = self
            .lane_busy_us
            .iter()
            .map(|(&lane, busy)| LaneCost {
                lane,
                busy_us: stats(busy),
                wall_us: stats(
                    self.lane_wall_us
                        .get(&lane)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                ),
            })
            .collect();
        let outside_segments: Vec<OutsideSegment> = self
            .outside_us
            .iter()
            .map(|(&code, samples)| OutsideSegment {
                code,
                name: OUTSIDE_BUILD_SEGMENTS
                    .iter()
                    .find(|(segment_code, _)| *segment_code == code)
                    .map(|(_, name)| (*name).to_string())
                    .unwrap_or_else(|| format!("code{code}")),
                us: stats(samples),
            })
            .collect();
        let mut country_reads: Vec<DomainCost> = self
            .country_read_us
            .iter()
            .map(|(&domain_id, samples)| DomainCost {
                domain_id,
                read_time_us: stats(samples),
                tail_positions: Vec::new(),
            })
            .collect();
        country_reads.sort_by(|a, b| b.read_time_us.p50.cmp(&a.read_time_us.p50));
        let country_crit_frames = self
            .frames
            .iter()
            .filter(|frame| frame.country_crit_us != 0)
            .count() as u64;
        let frame_tail_segments: Vec<OutsideSegment> = self
            .frame_tail_us
            .iter()
            .map(|(&code, samples)| OutsideSegment {
                code,
                name: FRAME_TAIL_SEGMENTS
                    .iter()
                    .find(|(segment_code, _)| *segment_code == code)
                    .map(|(_, name)| (*name).to_string())
                    .unwrap_or_else(|| format!("code{code}")),
                us: stats(samples),
            })
            .collect();
        Report {
            collector_build: recorder_host::build_declaration::summary_json(
                &recorder_host::build_declaration::for_stream(std::path::Path::new(&input_path)),
            ),
            input_path,
            bytes_consumed: self.bytes_consumed,
            records_seen: self.records_seen,
            freeze_census_records: self.freeze_census_records,
            unknown_record_type_records: self.unknown_record_type_records,
            parse_errors: self.parse_errors.clone(),
            frames_seen: self.frames_seen,
            frames_without_probe: self.frames_without_probe,
            frames_with_duplicate_units: self.frames_with_duplicate_units,
            frames_analysed: self.frames.len() as u64,
            span_us: column(|frame| frame.span_us),
            prewarm_us: column(|frame| frame.prewarm_us),
            fanout_origin_us: column(|frame| frame.fanout_origin_us),
            main_wall_us: column(|frame| frame.main_wall_us),
            crit_wall_us: column(|frame| frame.crit_wall_us),
            crit_busy_us: column(|frame| frame.crit_busy_us),
            uncovered_us: column(|frame| frame.uncovered_us),
            balanced_us: lane_column(|frame| frame.balanced_us),
            longest_job_us: column(|frame| frame.longest_job_us),
            makespan_floor_us: lane_column(|frame| frame.makespan_floor_us),
            job_total_us: column(|frame| frame.job_total_us),
            job_count: column(|frame| frame.job_count),
            lane_count: lane_column(|frame| frame.lane_count),
            lane_busy_sum_us: lane_column(|frame| frame.lane_busy_sum_us),
            lane_wall_sum_us: lane_column(|frame| frame.lane_wall_sum_us),
            frames_with_truncated_lanes,
            mismatch_domains: column(|frame| frame.mismatch_domains),
            bracketed_domains: column(|frame| frame.bracketed_domains),
            outside_total_us: column(|frame| frame.outside_total_us),
            outside_wait_us: column(|frame| frame.outside_wait_us),
            gap_above_lower_bound_us: stats(&gap_above_lower_bound),
            frame_time_us: column(|frame| frame.frame_time_us),
            serial_tail_us: column(|frame| frame.serial_tail_us),
            frame_tail_sum_us: column(|frame| frame.frame_tail_sum_us),
            deferred_publication_us: column(|frame| frame.deferred_publication_us),
            frames_with_scope_marks: self
                .frames
                .iter()
                .filter(|frame| frame.readers_done_us.is_some() && frame.scope_joined_us.is_some())
                .count() as u64,
            frames_with_publisher: self
                .frames
                .iter()
                .filter(|frame| frame.publisher_wait_us.is_some())
                .count() as u64,
            post_scope_main_us: stats(
                &self
                    .frames
                    .iter()
                    .filter_map(|frame| frame.post_scope_main_us)
                    .collect::<Vec<_>>(),
            ),
            publisher_wait_us: stats(
                &self
                    .frames
                    .iter()
                    .filter_map(|frame| frame.publisher_wait_us)
                    .collect::<Vec<_>>(),
            ),
            deferred_publication_beyond_readers_us: stats(
                &self
                    .frames
                    .iter()
                    .filter_map(|frame| frame.deferred_publication_beyond_readers_us)
                    .collect::<Vec<_>>(),
            ),
            domains,
            bundle_groups,
            lanes,
            outside_segments,
            frame_tail_segments,
            crit_domains,
            country_reads,
            country_child_count: column(|frame| frame.country_child_count),
            country_read_sum_us: column(|frame| frame.country_read_sum_us),
            country_read_max_us: column(|frame| frame.country_read_max_us),
            country_crit_us: column(|frame| frame.country_crit_us),
            country_crit_frames,
            inspect_domain: self.inspect_domain,
            inspect_read_time_us: stats(&self.inspect_read_us),
            inspect_units: self
                .inspect_units
                .iter()
                .map(|(name, samples)| InspectUnit {
                    name: name.clone(),
                    stats: stats(samples),
                })
                .collect(),
        }
    }
}

fn line(label: &str, stats: &Stats) {
    println!(
        "{label:<24} n={:<5} min={:<8} p25={:<8} p50={:<8} p75={:<8} p90={:<8} max={:<9} mean={:.1}",
        stats.count, stats.min, stats.p25, stats.p50, stats.p75, stats.p90, stats.max, stats.mean
    );
}

fn print_report(report: &Report) {
    print!(
        "{}",
        recorder_host::build_declaration::banner(&recorder_host::build_declaration::for_stream(
            std::path::Path::new(&report.input_path)
        ))
    );
    println!("input                    {}", report.input_path);
    println!(
        "records                  {} ({} bytes)",
        report.records_seen, report.bytes_consumed
    );
    println!(
        "frames                   seen={} analysed={} without_probe={} duplicate_units={} lane_truncated={}",
        report.frames_seen,
        report.frames_analysed,
        report.frames_without_probe,
        report.frames_with_duplicate_units,
        report.frames_with_truncated_lanes
    );
    if report.frames_with_truncated_lanes != 0 {
        println!(
            "WARNING                  {} frame(s) ran >32 gated lanes; every lane-derived column below \
             (balanced, makespan_floor, gap_above_lower_bound, lane_*) excludes them",
            report.frames_with_truncated_lanes
        );
    }
    if !report.parse_errors.is_empty() {
        println!("parse_errors             {}", report.parse_errors.len());
        for error in report.parse_errors.iter().take(5) {
            println!("  {error}");
        }
    }
    println!("\n-- span decomposition (microseconds) --");
    line("span (worker fan-out)", &report.span_us);
    line("  main_wall (serial)", &report.main_wall_us);
    line("  crit_wall (widest)", &report.crit_wall_us);
    line("  uncovered (compound)", &report.uncovered_us);
    line("crit_busy", &report.crit_busy_us);
    line("prewarm (before span)", &report.prewarm_us);
    line("fanout_origin", &report.fanout_origin_us);
    println!(
        "\n-- what the schedule could not beat (HEURISTIC: bundle collapse, see module docs) --"
    );
    line("longest_job", &report.longest_job_us);
    line("balanced (busy/lanes)", &report.balanced_us);
    line("makespan_floor (lower)", &report.makespan_floor_us);
    line("gap_above_lower_bound", &report.gap_above_lower_bound_us);
    line("job_total", &report.job_total_us);
    line("job_count", &report.job_count);
    line("lane_count", &report.lane_count);
    line("lane_busy_sum", &report.lane_busy_sum_us);
    line("lane_wall_sum", &report.lane_wall_sum_us);
    println!("\n-- hour-label loss --");
    line("mismatch_domains", &report.mismatch_domains);
    line("bracketed_domains", &report.bracketed_domains);
    println!("\n-- outside-build segments --");
    for segment in &report.outside_segments {
        line(
            &format!("  o{} {}", segment.code, segment.name),
            &segment.us,
        );
    }
    println!("\n-- serial tail (span close -> frame-time close; the lw_t* partition) --");
    line("frame_time (lw_ft_us)", &report.frame_time_us);
    line("post_scope_main (span-scope_joined)", &report.post_scope_main_us);
    line("serial_tail (ft-org-span)", &report.serial_tail_us);
    line("frame_tail_sum (lw_t*)", &report.frame_tail_sum_us);
    for segment in &report.frame_tail_segments {
        line(
            &format!("  t{} {}", segment.code, segment.name),
            &segment.us,
        );
    }
    println!("\n-- overlapped publication (previous hour's sink beside this fan-out) --");
    line("deferred_publication", &report.deferred_publication_us);
    println!(
        "frames_with_scope_marks {} / frames_with_publisher {} (the two lines below are over the publisher frames; absent on streams without lw_readers_done_us + lw_scope_joined_us)",
        report.frames_with_scope_marks, report.frames_with_publisher
    );
    line("  publisher_wait (scope_joined-readers_done; 0 if pub_end<=readers_done)", &report.publisher_wait_us);
    line("  beyond_readers (pub_end-readers_done)", &report.deferred_publication_beyond_readers_us);
    println!("\n-- lanes --");
    for lane in &report.lanes {
        println!(
            "  lane {:<3} busy p50={:<7} p90={:<7} | wall p50={:<7} p90={:<7} n={}",
            lane.lane,
            lane.busy_us.p50,
            lane.busy_us.p90,
            lane.wall_us.p50,
            lane.wall_us.p90,
            lane.busy_us.count
        );
    }
    println!("\n-- bundle groups (identical consecutive read_time_us) --");
    for group in report.bundle_groups.iter().take(12) {
        println!("  {:?} in {} frames", group.members, group.frames);
    }
    println!("\n-- per-domain read_time_us (top {DOMAIN_TABLE_LIMIT} by p50) --");
    for domain in report.domains.iter().take(DOMAIN_TABLE_LIMIT) {
        println!(
            "  d{:<3} n={:<5} p50={:<8} p90={:<8} max={:<9} tail={:?}",
            domain.domain_id,
            domain.read_time_us.count,
            domain.read_time_us.p50,
            domain.read_time_us.p90,
            domain.read_time_us.max,
            domain.tail_positions
        );
    }
    println!("\n-- emission tail (position from end -> frames) --");
    let mut tail: Vec<&DomainCost> = report
        .domains
        .iter()
        .filter(|domain| !domain.tail_positions.is_empty())
        .collect();
    tail.sort_by_key(|domain| {
        domain
            .tail_positions
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(rank, _)| *rank)
            .unwrap_or(usize::MAX)
    });
    for domain in tail {
        println!(
            "  d{:<3} read p50={:<8} positions={:?}",
            domain.domain_id, domain.read_time_us.p50, domain.tail_positions
        );
    }
    println!("\n-- per-domain busy ON THE CRITICAL LANE --");
    for domain in report.crit_domains.iter().take(DOMAIN_TABLE_LIMIT) {
        println!(
            "  d{:<3} frames={:<5} p50={:<8} p90={:<8} max={}",
            domain.domain_id,
            domain.read_time_us.count,
            domain.read_time_us.p50,
            domain.read_time_us.p90,
            domain.read_time_us.max
        );
    }
    println!(
        "\n-- country children (lw_cb{{tag}}_us; crit charged to country tags in {} frames) --",
        report.country_crit_frames
    );
    line("children per frame", &report.country_child_count);
    line("read sum", &report.country_read_sum_us);
    line("read max", &report.country_read_max_us);
    line("country crit busy", &report.country_crit_us);
    for domain in &report.country_reads {
        println!(
            "  cb{:<3} n={:<5} p50={:<8} p90={:<8} max={}",
            domain.domain_id,
            domain.read_time_us.count,
            domain.read_time_us.p50,
            domain.read_time_us.p90,
            domain.read_time_us.max
        );
    }
    if let Some(domain_id) = report.inspect_domain {
        println!("\n-- inside d{domain_id} (its own PERF trailer) --");
        line("read_time_us", &report.inspect_read_time_us);
        for unit in &report.inspect_units {
            line(&format!("  {}", unit.name), &unit.stats);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal_header(payload_len: usize) -> compact_contract::CompactRecordHeader {
        compact_contract::CompactRecordHeader {
            sequence: 77,
            game_hour_raw: 60_832_228,
            domain_id: compact_contract::COMPACT_DOMAIN_FRAME,
            record_type: COMPACT_RECORD_TYPE_FRAME_REFUSED,
            payload_len,
            record_flags: 0,
        }
    }

    fn refusal_payload_v2() -> Vec<u8> {
        use compact_contract::diagnostic_channel::{Authority, Predicate};
        compact_contract::frame_refusal::FrameRefusalRecordV2 {
            refused_hour: 60_832_228,
            offenders: Vec::new(),
            frame_findings: vec![compact_contract::frame_refusal::FrameRefusalFinding {
                predicate: Predicate::new(0x0001_0003, "frame_clock_is_readable").unwrap(),
                authority: Authority::Host,
                locator: Vec::new(),
                reason: "framing: the frame's own clock could not be read".to_string(),
            }],
        }
        .encode()
        .expect("a frame-level-only refusal is a complete record")
    }

    #[test]
    fn a_refusal_this_build_cannot_read_drops_the_frame_but_says_so() {
        let good = refusal_payload_v2();

        let mut state = ScanState::default();
        state.current.perf.push(PerfRow {
            domain_id: 10,
            read_time_us: 1_000,
            sequence: 1,
        });
        process_record(&mut state, &refusal_header(good.len()), &good);
        assert!(state.current.perf.is_empty(), "a refusal drops the frame");
        assert!(
            state.parse_errors.is_empty(),
            "a legal refusal is not an error: {:?}",
            state.parse_errors
        );

        let mut future = good.clone();
        future[4..6].copy_from_slice(&3u16.to_le_bytes());
        for (label, payload) in [
            ("future version", future),
            ("truncated", good[..8].to_vec()),
        ] {
            let mut state = ScanState::default();
            state.current.perf.push(PerfRow {
                domain_id: 10,
                read_time_us: 1_000,
                sequence: 1,
            });
            process_record(&mut state, &refusal_header(payload.len()), &payload);
            assert!(
                state.current.perf.is_empty(),
                "{label}: the frame boundary is still legible from the header"
            );
            assert_eq!(
                state.parse_errors.len(),
                1,
                "{label}: an unreadable refusal must be reported, not silently obeyed"
            );
            assert!(
                state.parse_errors[0].contains("FRAME_REFUSED at sequence 77"),
                "{label}: {:?}",
                state.parse_errors
            );
        }
    }

    #[test]
    fn the_serial_tail_and_the_overlapped_publication_are_derived_from_the_probe() {
        use compact_contract::diagnostic_channel::{UnitPresence, UnitRole};
        // kills: the `lw_t*` family decoded under the outside-build prefix (one
        // letter apart); `lw_ft_us` ignored; the serial tail derived from the
        // segment sum instead of from `lw_ft_us` (the sum is a tautology, the
        // subtraction is the cross-instrument check); the publisher wait
        // derived from `pub_end - span` (zero by construction: the span closes
        // after the scope joins the publisher) or from `span - readers_done`
        // (inflated by the post-scope main-thread reads, and positive with no
        // publisher at all) instead of from the two scope marks; a wait
        // reported for a frame that ran no publisher.
        //
        // A PRODUCTION-POSSIBLE TUPLE: readers_done <= pub_end <= scope_joined
        // <= span, which is the only order the producer can emit; the 15 ms
        // between the join and the span close are the post-scope reads.
        let units: Vec<DiagUnit> = [
            ("lw_frames", 1u64),
            ("lw_worker_span_us", 135_000),
            ("lw_fanout_origin_us", 2_000),
            ("lw_ft_us", 144_500),
            ("lw_t0_us", 400),
            ("lw_t1_us", 300),
            ("lw_t2_us", 350),
            ("lw_t3_us", 900),
            ("lw_t4_us", 600),
            ("lw_t5_us", 2_600),
            ("lw_t6_us", 2_100),
            ("lw_readers_done_us", 80_000),
            ("lw_scope_joined_us", 120_000),
            ("lw_pub_start_us", 50),
            ("lw_pub_end_us", 119_950),
            ("lw_o0_us", 10),
            ("lw_o1_us", 5),
        ]
        .into_iter()
        .map(|(name, value)| DiagUnit::new(name, value, UnitRole::Undeclared, UnitPresence::Undeclared))
        .collect();
        let probe = probe_units_from(&units);
        assert_eq!(probe.frame_time_us, Some(144_500));
        assert_eq!(probe.frame_tail.len(), 7);
        assert_eq!(probe.frame_tail.get(&6), Some(&2_100));
        assert_eq!(probe.outside.len(), 2, "the two families must not alias");
        let span = derive_frame_span(&probe, &[]).expect("a probed frame");
        assert_eq!(span.serial_tail_us, 7_500);
        assert_eq!(span.frame_tail_sum_us, 7_250);
        assert_eq!(span.deferred_publication_us, 119_900);
        assert_eq!(span.post_scope_main_us, Some(15_000));
        assert_eq!(
            span.publisher_wait_us,
            Some(40_000),
            "scope_joined - readers_done: the 15 ms of post-scope reads are not in it"
        );
        assert_eq!(span.deferred_publication_beyond_readers_us, Some(39_950));

        // A publisher that ended before the readers' close was not waited for:
        // the join interval is the sweep thread's own cleanup, and the wait is
        // zero -- not the interval (Codex round 5).
        for ended_us in [79_000, 80_000] {
            let mut early = probe.clone();
            early.pub_end_us = Some(ended_us);
            let early_span = derive_frame_span(&early, &[]).expect("a probed frame");
            assert_eq!(
                early_span.publisher_wait_us,
                Some(0),
                "a publisher that ended at {ended_us}us was not waited for past the readers' \
                 close at 80_000us -- the same microsecond included (Codex round 6)"
            );
            assert_eq!(early_span.deferred_publication_beyond_readers_us, Some(0));
            assert_eq!(early_span.post_scope_main_us, Some(15_000));
        }

        // No publisher on this frame: the join interval is not a wait.
        let mut quiet = probe.clone();
        quiet.pub_start_us = None;
        quiet.pub_end_us = None;
        let quiet_span = derive_frame_span(&quiet, &[]).expect("a probed frame");
        assert_eq!(quiet_span.publisher_wait_us, None);
        assert_eq!(quiet_span.deferred_publication_beyond_readers_us, None);
        assert_eq!(quiet_span.post_scope_main_us, Some(15_000));

        // Without the scope marks everything derived from them is UNKNOWN,
        // not zero.
        let mut older = probe.clone();
        older.readers_done_us = None;
        older.scope_joined_us = None;
        let older_span = derive_frame_span(&older, &[]).expect("a probed frame");
        assert_eq!(older_span.publisher_wait_us, None);
        assert_eq!(older_span.deferred_publication_beyond_readers_us, None);
        assert_eq!(older_span.post_scope_main_us, None);

        let mut state = ScanState::default();
        state.current.probe = Some(older);
        finalize_frame(&mut state);
        state.current.probe = Some(quiet);
        finalize_frame(&mut state);
        state.current.probe = Some(probe);
        finalize_frame(&mut state);
        let report = state.finish("probe".to_string());
        let late = report
            .frame_tail_segments
            .iter()
            .find(|segment| segment.code == 6)
            .expect("publish_late is reported");
        assert_eq!(late.name, "publish_late");
        assert_eq!(late.us.max, 2_100);
        assert_eq!(report.serial_tail_us.max, 7_500);
        assert_eq!(report.frames_analysed, 3);
        assert_eq!(report.frames_with_scope_marks, 2);
        assert_eq!(report.frames_with_publisher, 1);
        assert_eq!(report.post_scope_main_us.count, 2);
        assert_eq!(report.publisher_wait_us.count, 1);
        assert_eq!(report.publisher_wait_us.max, 40_000);
        assert_eq!(report.deferred_publication_beyond_readers_us.max, 39_950);
    }

    /// A probe is one value per name. A second `lw_scope_joined_us` carrying
    /// the span close would otherwise be read last-wins here while every
    /// first-wins reader saw the true value (Codex round 5, finding 2): the
    /// frame is refused and counted, and the duplicate is named.
    #[test]
    fn a_probe_that_repeats_a_unit_is_refused_rather_than_read_last_wins() {
        use compact_contract::diagnostic_channel::{UnitPresence, UnitRole};
        let units: Vec<DiagUnit> = [
            ("lw_frames", 1u64),
            ("lw_worker_span_us", 135_000),
            ("lw_readers_done_us", 80_000),
            ("lw_scope_joined_us", 120_000),
            ("lw_scope_joined_us", 135_000),
        ]
        .into_iter()
        .map(|(name, value)| DiagUnit::new(name, value, UnitRole::Undeclared, UnitPresence::Undeclared))
        .collect();
        let probe = probe_units_from(&units);
        assert_eq!(probe.duplicated_units, vec!["lw_scope_joined_us".to_string()]);
        let mut state = ScanState::default();
        state.current.probe = Some(probe);
        finalize_frame(&mut state);
        assert_eq!(state.frames_seen, 1);
        assert_eq!(state.frames_with_duplicate_units, 1);
        assert!(state.frames.is_empty(), "a malformed probe derives nothing");
        assert_eq!(state.parse_errors.len(), 1);
        assert!(state.parse_errors[0].contains("lw_scope_joined_us"), "{:?}", state.parse_errors);
        // The same names once each: nothing to refuse.
        let clean: Vec<DiagUnit> = units[..4].to_vec();
        let mut state = ScanState::default();
        state.current.probe = Some(probe_units_from(&clean));
        finalize_frame(&mut state);
        assert_eq!(state.frames_with_duplicate_units, 0);
        assert_eq!(state.frames.len(), 1);
        // EVERY name, not the thirteen fixed ones: a repeated segment, lane or
        // frame count is the same malformed probe (Codex round 6, finding 3).
        for repeated in ["lw_o0_us", "lw_t0_us", "lw_l0_wall_us", "lw_d48_crit_us", "lw_frames"] {
            let mut repeated_units = clean.clone();
            repeated_units.push(DiagUnit::new(repeated, 7, UnitRole::Undeclared, UnitPresence::Undeclared));
            repeated_units.push(DiagUnit::new(repeated, 8, UnitRole::Undeclared, UnitPresence::Undeclared));
            let probe = probe_units_from(&repeated_units);
            assert!(
                !probe.duplicated_units.is_empty()
                    && probe.duplicated_units.iter().all(|name| name == repeated),
                "{repeated} more than once is a duplicate: {:?}",
                probe.duplicated_units
            );
            let mut state = ScanState::default();
            state.current.probe = Some(probe);
            finalize_frame(&mut state);
            assert_eq!(state.frames_with_duplicate_units, 1, "{repeated}");
            assert!(state.frames.is_empty(), "{repeated}: refused, not read last-wins");
        }
    }

    #[test]
    fn consecutive_identical_read_times_collapse_to_one_job() {
        let perf: Vec<PerfRow> = [
            (10u16, 9_000u64),
            (11, 9_000),
            (12, 9_000),
            (13, 9_000),
            (14, 9_000),
            (15, 9_000),
            (16, 9_000),
            (17, 9_000),
            (21, 1_500),
            (22, 500),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (domain_id, read_time_us))| PerfRow {
            domain_id,
            read_time_us,
            sequence: index as u64,
        })
        .collect();
        let jobs = collapse_bundles(&perf);
        assert_eq!(jobs.len(), 3, "bundle plus two singletons");
        assert_eq!(jobs[0].0, vec![10, 11, 12, 13, 14, 15, 16, 17]);
        assert_eq!(jobs[0].1, 9_000);
        assert_eq!(jobs.iter().map(|(_, cost)| *cost).sum::<u64>(), 11_000);
    }

    #[test]
    fn zero_read_times_never_collapse() {
        let perf: Vec<PerfRow> = [30u16, 31, 32]
            .into_iter()
            .enumerate()
            .map(|(index, domain_id)| PerfRow {
                domain_id,
                read_time_us: 0,
                sequence: index as u64,
            })
            .collect();
        assert_eq!(collapse_bundles(&perf).len(), 3);
    }

    #[test]
    fn equal_but_separated_read_times_stay_separate_jobs() {
        let perf: Vec<PerfRow> = [(40u16, 2_000u64), (41, 7_000), (42, 2_000)]
            .into_iter()
            .enumerate()
            .map(|(index, (domain_id, read_time_us))| PerfRow {
                domain_id,
                read_time_us,
                sequence: index as u64,
            })
            .collect();
        let jobs = collapse_bundles(&perf);
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs.iter().map(|(_, cost)| *cost).sum::<u64>(), 11_000);
    }

    #[test]
    fn lane_wall_and_busy_units_fold_into_one_lane_entry() {
        let units = vec![
            DiagUnit::counter("lw_l0_wall_us", 30_000),
            DiagUnit::counter("lw_l1_wall_us", 20_000),
            DiagUnit::counter("lw_l0_busy_us", 25_000),
            DiagUnit::counter("lw_l1_busy_us", 18_000),
        ];
        let probe = probe_units_from(&units);
        assert_eq!(probe.lanes.len(), 2);
        let mut lanes = probe.lanes.clone();
        lanes.sort_by_key(|entry| entry.0);
        assert_eq!(lanes[0], (0, 30_000, 25_000));
        assert_eq!(lanes[1], (1, 20_000, 18_000));
    }

    #[test]
    fn country_read_units_fold_into_the_frames_country_columns() {
        // A frame after lane PS: eight children under their own domain ids,
        // tech on the critical lane.
        let units = vec![
            DiagUnit::counter("lw_frames", 1),
            DiagUnit::counter("lw_worker_span_us", 60_000),
            DiagUnit::counter("lw_l0_wall_us", 40_000),
            DiagUnit::counter("lw_l0_busy_us", 40_000),
            DiagUnit::counter("lw_cb10_us", 700),
            DiagUnit::counter("lw_cb13_us", 14_000),
            DiagUnit::counter("lw_cb14_us", 10_000),
            DiagUnit::counter("lw_d13_crit_us", 14_500),
            DiagUnit::counter("lw_d43_crit_us", 25_500),
        ];
        let probe = probe_units_from(&units);
        assert_eq!(probe.country_reads, vec![(10, 700), (13, 14_000), (14, 10_000)]);
        assert_eq!(probe.crit_domains, vec![(13, 14_500), (43, 25_500)]);
        assert_eq!(probe.lanes, vec![(0, 40_000, 40_000)]);
        let derived = derive_frame_span(&probe, &[]).expect("decomposes");
        assert_eq!(derived.country_child_count, 3);
        assert_eq!(derived.country_read_sum_us, 24_700);
        assert_eq!(derived.country_read_max_us, 14_000);
        assert_eq!(
            derived.country_crit_us, 14_500,
            "only the country child's critical busy time, not d43's"
        );

        // A frame before lane PS: the parent on the critical lane under the
        // retired tag, the children reporting their reads under lw_cb.
        let units = vec![
            DiagUnit::counter("lw_frames", 1),
            DiagUnit::counter("lw_worker_span_us", 60_000),
            DiagUnit::counter("lw_cb13_us", 14_000),
            DiagUnit::counter("lw_cb14_us", 10_000),
            DiagUnit::counter("lw_d200_crit_us", 57_000),
        ];
        let derived = derive_frame_span(&probe_units_from(&units), &[]).expect("decomposes");
        assert_eq!(derived.country_crit_us, 57_000);
        assert_eq!(derived.country_read_sum_us, 24_000);

        let mut state = ScanState::default();
        state.frames.push(derived);
        for &(tag, read_us) in &probe_units_from(&units).country_reads {
            state.country_read_us.entry(tag).or_default().push(read_us);
        }
        let report = state.finish("fixture".to_string());
        assert_eq!(report.country_crit_frames, 1);
        assert_eq!(report.country_crit_us.max, 57_000);
        assert_eq!(
            report
                .country_reads
                .iter()
                .map(|domain| (domain.domain_id, domain.read_time_us.max))
                .collect::<Vec<_>>(),
            vec![(13, 14_000), (14, 10_000)]
        );
    }

    #[test]
    fn per_domain_crit_units_are_not_mistaken_for_lane_units() {
        let units = vec![
            DiagUnit::counter("lw_d81_crit_us", 12_345),
            DiagUnit::counter("lw_l3_busy_us", 6_000),
        ];
        let probe = probe_units_from(&units);
        assert_eq!(probe.crit_domains, vec![(81, 12_345)]);
        assert_eq!(probe.lanes, vec![(3, 0, 6_000)]);
    }

    #[test]
    fn the_span_decomposition_is_derived_from_the_probe_units_it_names() {
        let units = vec![
            DiagUnit::counter("lw_frames", 1),
            DiagUnit::counter("lw_worker_span_us", 60_000),
            DiagUnit::counter("lw_prewarm_us", 24_000),
            DiagUnit::counter("lw_lmain_wall_us", 2_000),
            DiagUnit::counter("lw_crit_wall_us", 53_000),
            DiagUnit::counter("lw_crit_busy_us", 52_900),
            DiagUnit::counter("lw_l0_wall_us", 53_000),
            DiagUnit::counter("lw_l0_busy_us", 40_000),
            DiagUnit::counter("lw_l1_wall_us", 31_000),
            DiagUnit::counter("lw_l1_busy_us", 30_000),
            DiagUnit::counter("lw_l2_wall_us", 21_000),
            DiagUnit::counter("lw_l2_busy_us", 20_000),
            DiagUnit::counter("lw_l3_wall_us", 11_000),
            DiagUnit::counter("lw_l3_busy_us", 10_000),
        ];
        let perf: Vec<PerfRow> = [(81u16, 41_000u64), (12, 9_000), (13, 9_000), (22, 1_000)]
            .into_iter()
            .enumerate()
            .map(|(index, (domain_id, read_time_us))| PerfRow {
                domain_id,
                read_time_us,
                sequence: index as u64,
            })
            .collect();

        let derived = derive_frame_span(&probe_units_from(&units), &perf)
            .expect("a probe carrying lw_worker_span_us decomposes");

        assert_eq!(derived.span_us, 60_000);
        assert_eq!(derived.main_wall_us, 2_000);
        assert_eq!(derived.crit_wall_us, 53_000);
        assert_eq!(derived.uncovered_us, 5_000);
        assert_eq!(derived.lane_count, 4);
        assert_eq!(derived.lane_busy_sum_us, 100_000);
        assert_eq!(derived.balanced_us, 25_000);
        assert_eq!(derived.longest_job_us, 41_000);
        assert_eq!(derived.makespan_floor_us, 41_000);
        assert_eq!(derived.job_count, 3);
        assert_eq!(derived.job_total_us, 51_000);
        assert!(!derived.lanes_truncated);
    }

    #[test]
    fn a_frame_that_outran_the_named_lanes_marks_its_lane_columns_partial() {
        let units = vec![
            DiagUnit::counter("lw_frames", 1),
            DiagUnit::counter("lw_worker_span_us", 60_000),
            DiagUnit::counter("lw_l0_wall_us", 50_000),
            DiagUnit::counter("lw_l0_busy_us", 50_000),
            DiagUnit::counter("lw_lother_wall_us", 120_000),
        ];
        let derived = derive_frame_span(&probe_units_from(&units), &[])
            .expect("a probe carrying lw_worker_span_us decomposes");
        assert!(
            derived.lanes_truncated,
            "the surplus-lane unit must disqualify this frame's lane columns"
        );
    }

    #[test]
    fn the_reducer_drops_truncated_frames_from_lane_columns_and_keeps_them_elsewhere() {
        let base = vec![
            DiagUnit::counter("lw_frames", 1),
            DiagUnit::counter("lw_worker_span_us", 60_000),
            DiagUnit::counter("lw_crit_wall_us", 50_000),
            DiagUnit::counter("lw_l0_wall_us", 50_000),
            DiagUnit::counter("lw_l0_busy_us", 50_000),
        ];
        let mut truncated = base.clone();
        truncated.push(DiagUnit::counter("lw_lother_wall_us", 120_000));

        let mut state = ScanState::default();
        for units in [base, truncated] {
            state.frames.push(
                derive_frame_span(&probe_units_from(&units), &[])
                    .expect("a probe carrying lw_worker_span_us decomposes"),
            );
        }
        let report = state.finish("fixture".to_string());

        assert_eq!(report.frames_analysed, 2);
        assert_eq!(report.frames_with_truncated_lanes, 1);
        for (name, stats) in [
            ("balanced", &report.balanced_us),
            ("makespan_floor", &report.makespan_floor_us),
            ("gap_above_lower_bound", &report.gap_above_lower_bound_us),
            ("lane_count", &report.lane_count),
            ("lane_busy_sum", &report.lane_busy_sum_us),
            ("lane_wall_sum", &report.lane_wall_sum_us),
        ] {
            assert_eq!(stats.count, 1, "{name} must exclude the truncated frame");
        }
        for (name, stats) in [
            ("span", &report.span_us),
            ("crit_wall", &report.crit_wall_us),
            ("uncovered", &report.uncovered_us),
        ] {
            assert_eq!(stats.count, 2, "{name} describes every analysed frame");
        }
    }

    #[test]
    fn perf_read_time_is_read_from_the_declared_offset() {
        let mut payload = vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN];
        payload[0..8].copy_from_slice(&777u64.to_le_bytes());
        payload[8..16].copy_from_slice(&4_242u64.to_le_bytes());
        assert_eq!(perf_read_time_us(&payload), Some(4_242));
    }
}
