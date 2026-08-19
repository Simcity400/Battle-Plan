use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
use compact_contract::diagnostic_channel::{decode_units, DiagUnit, OUTSIDE_BUILD_SEGMENTS};
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
    lanes: Vec<(u16, u64, u64)>,
    outside: BTreeMap<u16, u64>,
    crit_domains: Vec<(u16, u64)>,
    lanes_truncated: bool,
}

fn probe_units_from(units: &[DiagUnit]) -> ProbeUnits {
    let mut probe = ProbeUnits::default();
    for unit in units {
        let name = unit.name.as_str();
        let value = unit.value;
        match name {
            "lw_worker_span_us" => probe.worker_span_us = Some(value),
            "lw_prewarm_us" => probe.prewarm_us = Some(value),
            "lw_fanout_origin_us" => probe.fanout_origin_us = Some(value),
            "lw_crit_wall_us" => probe.crit_wall_us = Some(value),
            "lw_crit_busy_us" => probe.crit_busy_us = Some(value),
            "lw_lmain_wall_us" => probe.main_wall_us = Some(value),
            "lw_hour_label_mismatch_domains" => probe.mismatch_domains = Some(value),
            "lw_bracketed_domains" => probe.bracketed_domains = Some(value),
            "lw_lother_wall_us" => probe.lanes_truncated = true,
            _ => {
                if let Some(code) = name
                    .strip_prefix("lw_o")
                    .and_then(|rest| rest.strip_suffix("_us"))
                    .and_then(|digits| digits.parse::<u16>().ok())
                {
                    probe.outside.insert(code, value);
                } else if let Some(tag) = name
                    .strip_prefix("lw_d")
                    .and_then(|rest| rest.strip_suffix("_crit_us"))
                    .and_then(|digits| digits.parse::<u16>().ok())
                {
                    probe.crit_domains.push((tag, value));
                } else if let Some(lane) = name
                    .strip_prefix("lw_l")
                    .and_then(|rest| rest.strip_suffix("_wall_us"))
                    .and_then(|digits| digits.parse::<u16>().ok())
                {
                    upsert_lane(&mut probe.lanes, lane).1 = value;
                } else if let Some(lane) = name
                    .strip_prefix("lw_l")
                    .and_then(|rest| rest.strip_suffix("_busy_us"))
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
    domain_read_us: BTreeMap<u16, Vec<u64>>,
    tail_rank_counts: BTreeMap<u16, BTreeMap<usize, u64>>,
    bundle_groups: BTreeMap<Vec<u16>, u64>,
    lane_busy_us: BTreeMap<u16, Vec<u64>>,
    lane_wall_us: BTreeMap<u16, Vec<u64>>,
    crit_domain_us: BTreeMap<u16, Vec<u64>>,
    outside_us: BTreeMap<u16, Vec<u64>>,
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
    domains: Vec<DomainCost>,
    bundle_groups: Vec<BundleGroup>,
    lanes: Vec<LaneCost>,
    outside_segments: Vec<OutsideSegment>,
    crit_domains: Vec<DomainCost>,
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
            domains,
            bundle_groups,
            lanes,
            outside_segments,
            crit_domains,
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
        "frames                   seen={} analysed={} without_probe={} lane_truncated={}",
        report.frames_seen,
        report.frames_analysed,
        report.frames_without_probe,
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
