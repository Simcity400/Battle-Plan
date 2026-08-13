//! WHERE THE SWEEP'S WALL GOES, derived from a capture's own wire.
//!
//! # Why this exists
//!
//! `lw_worker_span_us` — the fan-out wall — sat at a median of 60.5..63.9 ms
//! across four consecutive trials (t27 61.8, t28 60.5, t29 63.9, t30 60.7) that
//! varied everything reachable from outside the collector: cutting the longest
//! single read (lane CW's d81 foreign-fold cut, t28), raising reader
//! concurrency 12 -> 16 (t29), and halving the scheduler poll period (t30).
//! None of the three moved it. A floor that is invariant to the longest job, to
//! the worker count and to the poll rate is not a property of any one domain's
//! read, and no aggregate the existing tooling prints can say what it IS.
//!
//! `hour_label_audit` answers a different question — which domain-hours were
//! mislabelled and why — and deliberately reduces the lane-wall probe to the
//! eight units that question needs. This tool reads the SAME probe records for
//! the span decomposition instead, and adds the per-domain `DOMAIN_PERF`
//! stream, which `hour_label_audit` ignores entirely.
//!
//! # The decomposition, exactly
//!
//! Every quantity below is a wire unit or a difference of wire units. Nothing
//! here is modelled, and nothing is read from the collector source: if a term
//! is not on the wire it is reported as absent rather than inferred.
//!
//! Per frame, from the domain-250 lane-wall probe:
//!
//! * `span` = `lw_worker_span_us`, the fan-out wall. THE number under study.
//! * `prewarm` = `lw_prewarm_us`, the d47 root prewarm, which runs serially
//!   BEFORE the span opens and is therefore not part of it.
//! * `main_wall` = `lw_lmain_wall_us`, the sum of the MAIN-lane serial reads.
//!   These run on the sweep thread inside the span — some before
//!   `thread::scope` opens, some after it joins — so they are serial with the
//!   whole fan-out.
//! * `crit_wall` = `lw_crit_wall_us`, the widest single gated lane's span
//!   (last end - first start on that lane).
//! * `lane_busy_sum` = the sum of every `lw_l{N}_busy_us`, i.e. the total time
//!   gated workers spent holding a permit. `lane_count` is how many lanes
//!   reported.
//!
//! and the two derived terms the trials could not otherwise separate:
//!
//! * `uncovered` = `span - main_wall - crit_wall`. The part of the fan-out wall
//!   that is neither a serial MAIN read nor covered by the busiest lane's own
//!   span. It is an UPPER bound on ramp-plus-drain and it is COMPOUND: it
//!   contains the spawn ramp before the critical lane's first worker started,
//!   the drain after that lane's last worker ended, and every serial
//!   main-thread stretch inside the span that records no lane span at all. It
//!   is reported as the compound it is, never split by assertion.
//! * `balanced` = `lane_busy_sum / lane_count`, the makespan a perfect
//!   scheduler would reach if every job were divisible. With `longest_job` (the
//!   largest per-domain `read_time_us` in the frame, bundles collapsed) this is
//!   the standard `max(longest, total/machines)` list-scheduling LOWER BOUND —
//!   a wall no ordering could beat, which is not the same as a wall some
//!   ordering achieves. `gap_above_lower_bound = span - makespan_floor` is an
//!   UPPER BOUND on what reordering could recover and never a measure of waste
//!   (Codex review 019fd678-67c9-7703-a908-cca95f048ced, finding 3). Both
//!   inherit the bundle heuristic below, so both are reported as heuristic.
//!
//! # Bundles are collapsed, and the collapse is a HEURISTIC
//!
//! `read_time_us` is NOT additive across domains: several domains share one
//! read and publish the SAME elapsed time on each member's PERF record, so
//! summing the column counts one read several times. This tool detects the
//! sharing from the wire rather than from a hardcoded roster: within each
//! frame, domains that publish a byte-identical nonzero `read_time_us` AND
//! whose PERF records are emitted consecutively are treated as one job. The
//! groups it finds are printed so a reader can check them against the source.
//!
//! WHAT THIS IS NOT (Codex review 019fd678-67c9-7703-a908-cca95f048ced,
//! finding 2). It is not producer provenance, and it does not recover the
//! scheduler's job list:
//!
//!  * The country worker is ONE top-level lane job, but inside it the producer
//!    fans out again into up to seven `CompactBundleRead`s, each timing itself.
//!    Only the members of the same INNER read necessarily match, which is why
//!    t28 reports `[12, 13]` and never `[10..=17]`. The outer worker's lane
//!    hold is not on the wire as a `read_time_us` at all, so THESE QUANTITIES
//!    DO NOT DESCRIBE THE SCHEDULER'S JOB and their error has no reliable
//!    direction: `job_count` normally OVERSTATES it (up to seven inner reads
//!    counted where the schedule saw one job), `job_total` may land either side
//!    (overlapping inner timings can exceed the outer wall, while the outer
//!    spawn/join overhead is in none of them), and only `longest_job` has a
//!    known direction — it can understate the outer lane hold.
//!  * Two unrelated adjacent domains whose microsecond timings coincide are
//!    over-grouped; a shared refusal publishing 0 is deliberately not grouped
//!    at all (see `zero_read_times_never_collapse`).
//!
//! So `bundle_groups`, `job_count`, `job_total`, `longest_job`,
//! `makespan_floor` and `gap_above_lower_bound` are HEURISTIC and are labelled in
//! the printed report. The span decomposition itself — `span`, `main_wall`,
//! `crit_wall`, `crit_busy`, `uncovered` and the per-domain critical-lane table
//! — is not: every one of those is a producer unit or a difference of producer
//! units, and no conclusion about where the span goes depends on the
//! collapse.
//!
//! # The emission tail
//!
//! `DOMAIN_PERF` records carry the frame's emission `sequence`. The per-domain
//! distribution of "rank counted from the end of the frame" is printed for the
//! last positions, which is what turns "d54, d55, d81, d86 and d90 look like a
//! fixed tail" into a count over every frame in the capture.
//!
//! # `--domain N`: inside one domain's read
//!
//! Once the decomposition names a domain as the span, the next question is where
//! THAT domain's milliseconds go, and several producers already answer it: d81
//! publishes a `d81_phase_*` block on its own PERF record, d53 publishes
//! per-family timers, and so on. `--domain N` aggregates every diagnostic unit
//! on domain N's own trailer across the capture, so the inside of the critical
//! read is derived from the same stream as the outside — no new capture, and no
//! second instrument to keep in step with the producer.

use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
use compact_contract::diagnostic_channel::{decode_units, DiagUnit, OUTSIDE_BUILD_SEGMENTS};
use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, COMPACT_DOMAIN_PERF_BASE_LEN,
    COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC, COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2,
    COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END,
    COMPACT_RECORD_TYPE_FRAME_REFUSED,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

/// The reserved probe domain the lane-wall record rides on.
const RC_PROBE_DOMAIN_ID: u16 = 250;
/// How many trailing emission positions the tail census reports.
const TAIL_DEPTH: usize = 12;
/// How many domains the per-domain read-cost table prints.
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

// ---- per-frame accumulation ------------------------------------------------

/// One domain's PERF record as it appeared in the frame.
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

/// Every lane-wall unit this tool reads, kept as `Option` so "the collector did
/// not emit it" and "it was zero" stay distinguishable — the same rule the
/// producer applies when it omits a value unit and raises a `_missing` flag.
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
    /// (lane id, wall_us, busy_us) for every named lane that reported.
    lanes: Vec<(u16, u64, u64)>,
    /// `lw_o{n}_us`, indexed by segment code.
    outside: BTreeMap<u16, u64>,
    /// `lw_d{tag}_crit_us` — per-domain busy on the critical lane only.
    crit_domains: Vec<(u16, u64)>,
    /// The producer emitted `lw_lother_wall_us`, i.e. this frame ran more than
    /// `LW_MAX_NAMED_LANES` (32) gated lanes and folded the surplus into one
    /// WALL-only unit with no busy term and no lane count.
    ///
    /// Every lane-derived quantity is then built from a SUBSET of the frame's
    /// lanes, so `lane_count`, `lane_busy_sum`, `balanced` and `makespan_floor`
    /// are not what they claim (Codex review
    /// 019fd678-67c9-7703-a908-cca95f048ced, finding 4). Production has never
    /// run above 16 lanes, but this binary is handed arbitrary captures, and
    /// silently reporting a partial sum as a total is exactly the drift the
    /// rest of this tool refuses.
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
            // Presence alone is the signal: the producer emits it only when
            // some lane id was too high to name, and it carries no busy term.
            "lw_lother_wall_us" => probe.lanes_truncated = true,
            _ => {
                // ORDER MATTERS. `lw_d81_crit_us` and `lw_l3_busy_us` share the
                // `lw_` prefix and the `_us` suffix, so the most specific
                // suffix is tested first; see the unit test that pins it.
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

/// One frame, reduced to the terms the decomposition is stated in.
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
    /// `span - main_wall - crit_wall`, saturating. See the module docs: this is
    /// COMPOUND and is never split here.
    uncovered_us: u64,
    /// `lane_busy_sum / lane_count`.
    balanced_us: u64,
    /// Largest single job in the frame, bundles collapsed.
    longest_job_us: u64,
    /// `max(longest_job, balanced)` — the list-scheduling lower bound.
    makespan_floor_us: u64,
    /// Sum of every collapsed job's `read_time_us` in the frame.
    job_total_us: u64,
    job_count: u64,
    mismatch_domains: u64,
    bracketed_domains: u64,
    outside_total_us: u64,
    outside_wait_us: u64,
    /// See [`ProbeUnits::lanes_truncated`]. Frames carrying it are excluded
    /// from every lane-derived distribution rather than folded in as if their
    /// partial lane set were the whole one.
    lanes_truncated: bool,
}

// ---- scanning --------------------------------------------------------------

#[derive(Default)]
struct ScanState {
    bytes_consumed: u64,
    records_seen: u64,
    parse_errors: Vec<String>,
    current: FrameAccumulator,
    frames: Vec<FrameSpan>,
    frames_seen: u64,
    frames_without_probe: u64,
    /// Per-domain `read_time_us` samples, one per frame the domain appeared in.
    domain_read_us: BTreeMap<u16, Vec<u64>>,
    /// Per-domain count of "was in the last N emitted PERF records", by depth.
    tail_rank_counts: BTreeMap<u16, BTreeMap<usize, u64>>,
    /// Bundle groups keyed by their member list, counted over frames.
    bundle_groups: BTreeMap<Vec<u16>, u64>,
    /// Per-lane busy samples, so lane imbalance is visible.
    lane_busy_us: BTreeMap<u16, Vec<u64>>,
    lane_wall_us: BTreeMap<u16, Vec<u64>>,
    /// Per-domain critical-lane busy samples.
    crit_domain_us: BTreeMap<u16, Vec<u64>>,
    outside_us: BTreeMap<u16, Vec<u64>>,
    /// `--domain N`: which producer's own trailer to aggregate, if any.
    inspect_domain: Option<u16>,
    /// That producer's own diagnostic units, one sample per record that carried
    /// the unit. Collected UNDECODED of meaning — this tool knows nothing about
    /// what `d81_phase_hub_foreign_us` is, only that the producer published it.
    inspect_units: BTreeMap<String, Vec<u64>>,
    /// `read_time_us` on the inspected domain's own records, so the phase block
    /// can be checked against the total it decomposes.
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
            .map_err(|error| format!("reserve {} bytes at {offset}: {error}", header.payload_len))?;
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
    // Validate BEFORE any type-specific arm returns. FRAME_REFUSED used to be
    // handled above this call and returned early, so this audit accepted every
    // type-6 record as a refusal without ever reading its payload: a truncated
    // one, or one written at an H4FR version this build cannot decode, dropped
    // the open frame and produced no `parse_error` at all. An audit that
    // silently discards a frame on a record it never checked is reporting a
    // conclusion it did not reach.
    let semantics = validate_compact_record_semantics(
        header.domain_id,
        header.record_type,
        header.record_flags,
        payload,
    );
    let semantics_ok = semantics.is_ok();

    // A refused frame published no sweep, so its partial accumulation describes
    // nothing and is dropped rather than folded into the distributions. Exactly
    // one H4FR version is legal on the wire and the validator above is what says
    // so; a refusal this build cannot read still DROPS the frame -- the record
    // type is the frame boundary and that much is legible from the header --
    // but it no longer does so silently.
    //
    // How the other record types treat a semantics failure is deliberately
    // unchanged: `semantics_ok` gates the PERF arm exactly as before.
    if header.record_type == COMPACT_RECORD_TYPE_FRAME_REFUSED {
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

/// Group this frame's PERF rows into JOBS. Domains that publish a byte-identical
/// nonzero `read_time_us` in consecutively-emitted records share one read (see
/// the module docs) and count once.
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

/// THE DERIVATION, as a pure function of one frame's wire records.
///
/// Extracted from the scan so the whole chain — probe units in, `uncovered` /
/// `balanced` / `makespan_floor` out — can be asserted from a synthetic frame
/// rather than only through a 2 GB capture (Codex review
/// 019fd678-67c9-7703-a908-cca95f048ced, finding 5). `None` when the probe
/// carries no `lw_worker_span_us`, which is the one unit the whole
/// decomposition is stated against.
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

// ---- reduction -------------------------------------------------------------

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
    /// How often this domain was among the last [`TAIL_DEPTH`] PERF records,
    /// and at which position from the end (0 == last).
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
    /// WHAT BUILD PRODUCED THIS STREAM, read from the session manifest beside
    /// it. Every column below is a span in microseconds; a diagnostic collector
    /// changes what those spans cost. `diagnostic_build` is null whenever the
    /// answer is unknown — absence is never a clean build.
    collector_build: serde_json::Value,
    bytes_consumed: u64,
    records_seen: u64,
    parse_errors: Vec<String>,
    frames_seen: u64,
    frames_without_probe: u64,
    frames_analysed: u64,
    /// Frames excluded from every lane-derived column because the producer
    /// folded lanes 32+ into `lw_lother_wall_us`. Nonzero means `balanced`,
    /// `makespan_floor`, `gap_above_lower_bound` and the `lane_*` columns describe fewer
    /// frames than `frames_analysed`.
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
    /// `span - makespan_floor`: an UPPER BOUND on the wall any reordering of
    /// the same jobs on the same lanes could recover, and not a measure of what
    /// it would recover. `makespan_floor` is a lower bound on the achievable
    /// makespan, so the distance to it is a bound on the opportunity and never
    /// an observation of waste — and it inherits the bundle heuristic above, so
    /// it is not exact either. Named for what it is rather than for what a
    /// reader would like it to be.
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
        // Lane-derived columns see only the frames whose lane set was WHOLE.
        // A frame that folded lanes 32+ into `lw_lother_wall_us` contributes a
        // partial busy sum and an undercounted lane count, and averaging that
        // in with complete frames produces a number that is neither.
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

// ---- printing --------------------------------------------------------------

fn line(label: &str, stats: &Stats) {
    println!(
        "{label:<24} n={:<5} min={:<8} p25={:<8} p50={:<8} p75={:<8} p90={:<8} max={:<9} mean={:.1}",
        stats.count, stats.min, stats.p25, stats.p50, stats.p75, stats.p90, stats.max, stats.mean
    );
}

fn print_report(report: &Report) {
    // BEFORE the spans it qualifies. See `hour_label_audit` for why the banner
    // leads rather than trails.
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
    println!("\n-- what the schedule could not beat (HEURISTIC: bundle collapse, see module docs) --");
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

    /// This audit drops the open frame on a FRAME_REFUSED, and it used to do
    /// that from the record TYPE alone -- the arm returned before the semantics
    /// call, so a truncated refusal, or one written at an H4FR version this
    /// build cannot decode, discarded a frame and produced no `parse_error`.
    /// An audit that reports a conclusion it never checked is the shape this
    /// closes.
    ///
    /// MUTATION: move the FRAME_REFUSED arm back above
    /// `validate_compact_record_semantics`, or drop the `parse_errors.push`,
    /// and the unreadable case below reports nothing.
    #[test]
    fn a_refusal_this_build_cannot_read_drops_the_frame_but_says_so() {
        let good = refusal_payload_v2();

        // The legal version 2 record: frame dropped, nothing to report.
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

        // The same record restamped at a version this build does not decode,
        // and a truncated one. Both still drop the frame -- the boundary is
        // legible from the header -- and both are now named.
        let mut future = good.clone();
        future[4..6].copy_from_slice(&3u16.to_le_bytes());
        for (label, payload) in [("future version", future), ("truncated", good[..8].to_vec())] {
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

    /// Bundle members publish the SAME elapsed time on consecutive records, and
    /// collapsing them is what keeps `longest_job` and `job_total` from
    /// counting one read up to eight times.
    ///
    /// MUTATION: drop the `*cost == row.read_time_us` arm in `collapse_bundles`
    /// and the eight country-bundle members become eight 9,000 us jobs, so
    /// `job_total` reports 74,000 instead of 11,000 here.
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

    /// A zero read time is NOT evidence of sharing — a rowless refusal and a
    /// genuinely instant read both publish 0, and gluing those together would
    /// invent a bundle out of the frame's failures.
    ///
    /// MUTATION: remove the `row.read_time_us != 0` guard and these three
    /// unrelated domains collapse into one job.
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

    /// Two domains with the same cost that are NOT emitted next to each other
    /// are two jobs. Sharing is a property of one read publishing twice, which
    /// shows up as adjacency in the emission order; equal costs alone are a
    /// coincidence that the schedule must still pay for twice.
    ///
    /// MUTATION: match on cost alone, ignoring `jobs.last_mut()` adjacency, and
    /// this returns 2 jobs instead of 3.
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

    /// The lane units arrive as two independent `lw_l{N}_wall_us` /
    /// `lw_l{N}_busy_us` pairs in producer order, and both must land on the
    /// same lane entry.
    ///
    /// MUTATION: make `upsert_lane` push unconditionally and `lane_count`
    /// doubles, which halves `balanced` and would understate the schedule floor
    /// by 2x.
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

    /// `lw_d{tag}_crit_us` and `lw_l{lane}_wall_us` both start `lw_` and both
    /// end `_us`; parsing one as the other silently moves a domain's busy time
    /// into the lane table.
    ///
    /// MUTATION: reorder the `strip_prefix` arms so `lw_d81_crit_us` is tested
    /// against the lane pattern first and this assertion fails.
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

    /// THE WHOLE DERIVATION, from probe units to the published columns.
    ///
    /// The other tests here pin one helper each; this one asserts the numbers
    /// the report is actually read from, on a frame whose every term is chosen
    /// so that no two of them coincide (Codex review
    /// 019fd678-67c9-7703-a908-cca95f048ced, finding 5).
    ///
    /// The frame: span 60,000; MAIN serial reads 2,000; widest lane 53,000; four
    /// lanes busy 40,000 + 30,000 + 20,000 + 10,000 = 100,000. So
    /// `uncovered = 60,000 - 2,000 - 53,000 = 5,000`,
    /// `balanced = 100,000 / 4 = 25,000`, the longest collapsed job is 41,000
    /// (the two-member bundle), `makespan_floor = max(41,000, 25,000) = 41,000`.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: subtracting `crit_busy` instead
    /// of `crit_wall` in `uncovered` (they are within 41 us of each other on
    /// real captures, so a real capture would NOT reveal the swap and this
    /// fixture does); dividing `lane_wall_sum` rather than `lane_busy_sum` by
    /// the lane count; or taking `min` instead of `max` for the makespan floor,
    /// which would report the bound as 25,000 and make the fan-out look 16 ms
    /// further from optimal than it is.
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
        // d12 and d13 collapse; d81 and d22 do not.
        assert_eq!(derived.job_count, 3);
        assert_eq!(derived.job_total_us, 51_000);
        assert!(!derived.lanes_truncated);
    }

    /// A frame that ran more than 32 gated lanes reports its surplus as one
    /// wall-only `lw_lother_wall_us`, so its busy sum and lane count are
    /// PARTIAL and every lane-derived column has to exclude it.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: dropping the
    /// `lw_lother_wall_us` arm from the parser, which is the state this tool
    /// shipped in — `balanced` would then be computed from a subset of lanes
    /// and reported as if it were the whole schedule.
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

    /// ...and the REDUCER must actually drop it. Flagging the frame and then
    /// averaging it in anyway is the failure this pair exists to prevent
    /// (Codex review 019fd678-67c9-7703-a908-cca95f048ced, round 2).
    ///
    /// Two frames in, one complete and one truncated. Every lane-derived column
    /// must report `count == 1`, every other column `count == 2`, and
    /// `frames_analysed` must still be 2 — the frame is excluded from six
    /// columns, not from the capture.
    ///
    /// WHAT PRODUCTION CHANGE MAKES THIS FAIL: swapping any of the six
    /// `lane_column(...)` calls back to `column(...)`, or dropping the
    /// `filter` from `gap_above_lower_bound`. Each would silently average a
    /// partial lane set in with complete ones.
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

    /// The offset of `read_time_us` inside the DOMAIN_PERF payload is a wire
    /// fact, and reading the wrong eight bytes yields `elapsed_wall_ms` — a
    /// plausible-looking number that is not a read time at all.
    ///
    /// MUTATION: slice `0..8` instead of `8..16` and this returns 777.
    #[test]
    fn perf_read_time_is_read_from_the_declared_offset() {
        let mut payload = vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN];
        payload[0..8].copy_from_slice(&777u64.to_le_bytes());
        payload[8..16].copy_from_slice(&4_242u64.to_le_bytes());
        assert_eq!(perf_read_time_us(&payload), Some(4_242));
    }
}
