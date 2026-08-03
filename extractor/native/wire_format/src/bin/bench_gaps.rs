//! Real-capture measurement for the container-level §8 gap stream.
//!
//! This walks the complete H4CR stream through the shipping framing validator
//! and the recorder's hour-boundary subset before deriving gaps from four public
//! domain fold boundaries. It writes no artifact until the full scan and the
//! anti-drift reconciliation have succeeded.

use std::collections::{BTreeSet, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use compact_contract::stream_validator::StatefulStreamValidator;
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance,
    compact_status_for_global_interpretation, compact_status_unknown_flags_v1,
    expected_partial_tolerance_applies, parse_compact_record_header, CompactRecordHeader,
    GapReason as H4GapReason, GapRecord as H4GapRecord, GapWitness as H4GapWitness,
    COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END,
    COMPACT_STATUS_BRIDGE_UNAVAILABLE, COMPACT_STATUS_FAILURE_MASK, COMPACT_STATUS_PARTIAL_DATA,
    COMPACT_STATUS_SOURCE_UNAVAILABLE, COMPACT_STATUS_WORKER_PANIC,
};
use wire_format::d49;
use wire_format::d50;
use wire_format::d51::{self, FoldOutcome as D51FoldOutcome};
use wire_format::d54;
use wire_format::gap::{
    decode_record, encode_record, normalise, GapPart, GapReason, GapRecord, GapWitness,
};

const D49: u16 = 49;
const D50: u16 = 50;
const D51: u16 = 51;
const D54: u16 = 54;
const DOMAINS: [u16; 4] = [D49, D50, D51, D54];
const BLOCK_HOURS: usize = 48;
const PROBE_DOMAIN_ID: u16 = 250;
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
const EXPECTED_CAPTURE_BYTES: u64 = 3_598_209_757;
const EXPECTED_CAMPAIGN_START_RAW: i32 = 60_759_371;
// This corpus contains hourly observations only. The measurement therefore uses
// the explicit hourly §1.1 grid rather than silently treating engine-date hours
// as ticks.
const TICKS_PER_HOUR_LOG2: u8 = 0;

#[derive(Clone, Copy)]
enum GapSource {
    PresentRefusal,
    ClosedHourMissingRecord,
    ClockHole,
}

#[derive(Default)]
struct DomainStats {
    records: usize,
    measured_hours: Vec<i32>,
    present_refusals: usize,
    closed_missing: usize,
    clock_holes: usize,
}

impl DomainStats {
    fn non_contiguous(&self) -> usize {
        self.measured_hours
            .windows(2)
            .filter(|pair| pair[1] != pair[0].saturating_add(1))
            .count()
    }

    fn baseline_counts(&self) -> (usize, usize) {
        let mut baselines = 0usize;
        let mut gap_forced = 0usize;
        for (index, hour) in self.measured_hours.iter().copied().enumerate() {
            let gap_before = index
                .checked_sub(1)
                .map(|prior| hour != self.measured_hours[prior].saturating_add(1))
                .unwrap_or(false);
            let block_start = index % BLOCK_HOURS == 0;
            if block_start || gap_before || index == 0 {
                baselines += 1;
                if gap_before && !block_start {
                    gap_forced += 1;
                }
            }
        }
        (baselines, gap_forced)
    }
}

#[derive(Default)]
struct OutcomeCounts {
    d49_location: usize,
    d49_failure: usize,
    d49_side_scalars: usize,
    d50_failure: usize,
    d50_orphan: usize,
    d50_duplicate: usize,
    d50_resurrection: usize,
    d54_failure: usize,
    d54_delta_before: usize,
    d54_tolerated_partial: usize,
    d51_failure: usize,
    d51_delta_before: usize,
}

struct Scan {
    closed_hours: usize,
    observations: Vec<GapRecord>,
    // The retired H4GP comparison uses engine game-hour coordinates, while §8
    // uses ticks relative to campaign hour zero. Keep the two identities paired
    // instead of coercing either format onto the other's clock.
    raw_observation_hours: Vec<i32>,
    domains: [DomainStats; 4],
    outcomes: OutcomeCounts,
    status_or: [u32; 4],
    first_hour: Option<i32>,
    last_hour: Option<i32>,
}

impl Default for Scan {
    fn default() -> Self {
        Self {
            closed_hours: 0,
            observations: Vec::new(),
            raw_observation_hours: Vec::new(),
            domains: std::array::from_fn(|_| DomainStats::default()),
            outcomes: OutcomeCounts::default(),
            status_or: [0; 4],
            first_hour: None,
            last_hour: None,
        }
    }
}

fn raw_hour_to_tick(raw_hour: i32) -> Result<u64, String> {
    let campaign_hour = raw_hour.checked_sub(EXPECTED_CAMPAIGN_START_RAW).ok_or_else(|| {
        format!(
            "game hour {raw_hour} is before campaign start {EXPECTED_CAMPAIGN_START_RAW}"
        )
    })?;
    let campaign_hour = u64::try_from(campaign_hour).map_err(|_| {
        format!(
            "game hour {raw_hour} is before campaign start {EXPECTED_CAMPAIGN_START_RAW}"
        )
    })?;
    let ticks_per_hour = 1u64
        .checked_shl(u32::from(TICKS_PER_HOUR_LOG2))
        .ok_or_else(|| format!("ticks_per_hour_log2 {TICKS_PER_HOUR_LOG2} is not representable"))?;
    campaign_hour.checked_mul(ticks_per_hour).ok_or_else(|| {
        format!("campaign hour {campaign_hour} overflows the {TICKS_PER_HOUR_LOG2}-log2 tick grid")
    })
}

impl Scan {
    fn domain_mut(&mut self, domain: u16) -> &mut DomainStats {
        &mut self.domains[domain_index(domain)]
    }

    fn add_gap(
        &mut self,
        domain: u16,
        hour: i32,
        reason: GapReason,
        witness: GapWitness,
        source: GapSource,
    ) -> Result<(), String> {
        let tick = raw_hour_to_tick(hour)?;
        let tick_end = tick
            .checked_add(1)
            .ok_or_else(|| format!("domain {domain} gap hour {hour} overflows tick range"))?;
        let stats = self.domain_mut(domain);
        match source {
            GapSource::PresentRefusal => stats.present_refusals += 1,
            GapSource::ClosedHourMissingRecord => stats.closed_missing += 1,
            GapSource::ClockHole => stats.clock_holes += 1,
        }
        self.observations.push(GapRecord {
            tick_start: tick,
            tick_end,
            branch: 0,
            reason,
            part: GapPart::State,
            domains: vec![u32::from(domain)],
            witness,
        });
        self.raw_observation_hours.push(hour);
        Ok(())
    }
}

fn add_clock_hole(scan: &mut Scan, last_closed: i32, current: i32) -> Result<(), String> {
    let previous_tick = raw_hour_to_tick(last_closed)?;
    let next_tick = raw_hour_to_tick(current)?;
    for missing in last_closed.saturating_add(1)..current {
        for domain in DOMAINS {
            scan.add_gap(
                domain,
                missing,
                GapReason::Skipped,
                GapWitness::TickDiscontinuity {
                    previous_tick,
                    next_tick,
                },
                GapSource::ClockHole,
            )?;
        }
    }
    Ok(())
}

fn domain_index(domain: u16) -> usize {
    match domain {
        D49 => 0,
        D50 => 1,
        D51 => 2,
        D54 => 3,
        _ => unreachable!("only the four measured domains are indexed"),
    }
}

fn coverage_witness() -> GapWitness {
    // "Observed" is a usable fold observation here, not merely an H4CR DATA
    // envelope. A present record whose public fold refuses supplies 0 of 1.
    GapWitness::Coverage {
        observed: 0,
        expected: 1,
    }
}

fn status_cause(domain: u16, status: u32) -> (GapReason, GapWitness) {
    let global = compact_status_for_global_interpretation(domain, status);
    let witness = GapWitness::StatusBits {
        bit_width: 32,
        bits: global.to_le_bytes().to_vec(),
    };
    let reason = if global & COMPACT_STATUS_SOURCE_UNAVAILABLE != 0 {
        GapReason::SourceUnavailable
    } else if global & COMPACT_STATUS_PARTIAL_DATA != 0 {
        GapReason::PartialData
    } else if global & COMPACT_STATUS_BRIDGE_UNAVAILABLE != 0 {
        GapReason::BridgeUnavailable
    } else if global & COMPACT_STATUS_WORKER_PANIC != 0 {
        GapReason::WorkerPanic
    } else {
        // §8 assigns no reason to the other shipping failure bits. Preserve the
        // gap as a usable-observation shortfall instead of inventing a code.
        return (GapReason::CoverageShortfall, coverage_witness());
    };
    (reason, witness)
}

fn classify_status(domain: u16, status: u32, at: usize) -> Result<(bool, bool), String> {
    let global = compact_status_for_global_interpretation(domain, status);
    let unknown = compact_status_unknown_flags_v1(global);
    if unknown != 0 {
        return Err(format!(
            "domain {domain} DATA at byte {at} carries unassigned status bits {unknown:#x}"
        ));
    }
    Ok((
        compact_status_failure_flags_after_expected_partial_tolerance(domain, status) != 0,
        expected_partial_tolerance_applies(domain, status).is_some(),
    ))
}

struct RecordReader<R: Read> {
    inner: R,
    header_bytes: [u8; COMPACT_RECORD_HEADER_LEN],
    payload: Vec<u8>,
    pos: usize,
}

impl<R: Read> RecordReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            header_bytes: [0; COMPACT_RECORD_HEADER_LEN],
            payload: Vec::new(),
            pos: 0,
        }
    }

    fn next(&mut self) -> Result<Option<(CompactRecordHeader, usize)>, String> {
        let mut filled = 0usize;
        while filled < COMPACT_RECORD_HEADER_LEN {
            let read = self
                .inner
                .read(&mut self.header_bytes[filled..])
                .map_err(|error| format!("read at byte {}: {error}", self.pos + filled))?;
            if read == 0 {
                if filled == 0 {
                    return Ok(None);
                }
                return Err(format!(
                    "stream ends mid-header at byte {} ({filled}/{COMPACT_RECORD_HEADER_LEN})",
                    self.pos
                ));
            }
            filled += read;
        }
        let header = parse_compact_record_header(&self.header_bytes)
            .map_err(|error| format!("record at byte {}: {error}", self.pos))?;
        self.payload.clear();
        self.payload.resize(header.payload_len, 0);
        let mut filled = 0usize;
        while filled < header.payload_len {
            let read = self
                .inner
                .read(&mut self.payload[filled..])
                .map_err(|error| {
                    format!(
                        "read at byte {}: {error}",
                        self.pos + COMPACT_RECORD_HEADER_LEN + filled
                    )
                })?;
            if read == 0 {
                return Err(format!(
                    "stream ends mid-payload at byte {} ({filled}/{})",
                    self.pos, header.payload_len
                ));
            }
            filled += read;
        }
        let start = self.pos;
        self.pos = self
            .pos
            .checked_add(COMPACT_RECORD_HEADER_LEN + header.payload_len)
            .ok_or_else(|| "stream byte position overflows usize".to_string())?;
        Ok(Some((header, start)))
    }
}

fn check_frame_end_seal(payload: &[u8], at: usize) -> Result<(u32, u32), String> {
    if payload.len() != COMPACT_FRAME_END_PAYLOAD_LEN {
        return Err(format!(
            "FRAME_END at byte {at} has {} payload bytes, expected {COMPACT_FRAME_END_PAYLOAD_LEN}",
            payload.len()
        ));
    }
    let emitted = u32::from_le_bytes(payload[16..20].try_into().expect("four-byte slice"));
    let scheduled = u32::from_le_bytes(payload[20..24].try_into().expect("four-byte slice"));
    let status = u32::from_le_bytes(payload[24..28].try_into().expect("four-byte slice"));
    let reserved = u32::from_le_bytes(payload[28..32].try_into().expect("four-byte slice"));
    if reserved != 0 || emitted != scheduled {
        return Err(format!(
            "FRAME_END at byte {at} has reserved={reserved:#x}, emitted={emitted}, scheduled={scheduled}"
        ));
    }
    let unknown = status & !FRAME_END_KNOWN_STATUS_MASK;
    let failure = status & COMPACT_STATUS_FAILURE_MASK;
    if unknown != 0 || failure != 0 {
        return Err(format!(
            "FRAME_END at byte {at} has unknown={unknown:#x}, failure={failure:#x}"
        ));
    }
    Ok((emitted, scheduled))
}

fn scan_capture(path: &Path) -> Result<Scan, String> {
    let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut records = RecordReader::new(BufReader::with_capacity(8 * 1024 * 1024, file));
    let mut validator = StatefulStreamValidator::legacy();
    let mut scan = Scan::default();

    let d49_schema = d49::schema();
    let d50_schema = d50::schema();
    let d54_schema = d54::schema();
    let d50_collection = &d50_schema.collections[0];
    let mut d49_state = d49::empty_state();
    let mut d50_state = d50::State::new();
    let mut d51_scanner = d51::KeyScanner::default();
    let mut d51_vocabulary = BTreeSet::new();
    let mut d54_state = d54::State::new();

    let mut open_hour: Option<i32> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut perf_domains = HashSet::new();
    let mut saw = [false; 4];

    while let Some((header, at)) = records.next()? {
        let payload = &records.payload;
        validator
            .validate_record(&header, payload)
            .map_err(|error| format!("record at byte {at}: {error:?}"))?;

        match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                if header.domain_id != PROBE_DOMAIN_ID && !perf_domains.insert(header.domain_id) {
                    return Err(format!(
                        "duplicate DOMAIN_PERF for domain {} in hour {}",
                        header.domain_id, header.game_hour_raw
                    ));
                }
                match open_hour {
                    Some(open) if open != header.game_hour_raw => {
                        return Err(format!(
                            "DOMAIN_PERF hour {} interleaves open hour {open}",
                            header.game_hour_raw
                        ))
                    }
                    Some(_) => {}
                    None => {
                        if last_closed_hour.is_some_and(|last| header.game_hour_raw <= last) {
                            return Err(format!(
                                "DOMAIN_PERF hour {} is not after the last closed hour",
                                header.game_hour_raw
                            ));
                        }
                        match pending_perf_hour {
                            Some(pending) if pending != header.game_hour_raw => {
                                return Err(format!(
                                    "pending PERF hour {pending}, then {}",
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
                        "DOMAIN_DATA hour {} interleaves open hour {open}",
                        header.game_hour_raw
                    ))
                }
                Some(_) => {}
                None => {
                    if pending_perf_hour.is_some_and(|pending| pending != header.game_hour_raw) {
                        return Err(format!(
                            "pending PERF hour {:?}, then DATA hour {}",
                            pending_perf_hour, header.game_hour_raw
                        ));
                    }
                    if last_closed_hour.is_some_and(|last| header.game_hour_raw <= last) {
                        return Err(format!(
                            "DOMAIN_DATA hour {} reopens a closed hour",
                            header.game_hour_raw
                        ));
                    }
                    open_hour = Some(header.game_hour_raw);
                }
            },
            COMPACT_RECORD_TYPE_FRAME_END => {
                let (emitted, scheduled) = check_frame_end_seal(payload, at)?;
                let observed = u32::try_from(perf_domains.len())
                    .map_err(|_| "PERF domain count exceeds u32".to_string())?;
                if observed != emitted || observed != scheduled {
                    return Err(format!(
                        "FRAME_END claims {emitted}/{scheduled} domains but {observed} PERF domains were observed"
                    ));
                }
                if open_hour != Some(header.game_hour_raw) {
                    return Err(format!(
                        "FRAME_END hour {} does not close open hour {open_hour:?}",
                        header.game_hour_raw
                    ));
                }
                if last_closed_hour.is_some_and(|last| header.game_hour_raw <= last) {
                    return Err(format!(
                        "FRAME_END hour {} is not after the last closed hour",
                        header.game_hour_raw
                    ));
                }

                if let Some(last) = last_closed_hour {
                    add_clock_hole(&mut scan, last, header.game_hour_raw)?;
                }
                for domain in DOMAINS {
                    if !saw[domain_index(domain)] {
                        scan.add_gap(
                            domain,
                            header.game_hour_raw,
                            GapReason::CoverageShortfall,
                            coverage_witness(),
                            GapSource::ClosedHourMissingRecord,
                        )?;
                    }
                }
                scan.first_hour.get_or_insert(header.game_hour_raw);
                scan.last_hour = Some(header.game_hour_raw);
                scan.closed_hours += 1;
                saw = [false; 4];
                perf_domains.clear();
                pending_perf_hour = None;
                open_hour = None;
                last_closed_hour = Some(header.game_hour_raw);
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.record_type != COMPACT_RECORD_TYPE_DOMAIN_DATA
            || !DOMAINS.contains(&header.domain_id)
        {
            continue;
        }
        let index = domain_index(header.domain_id);
        if saw[index] {
            return Err(format!(
                "domain {} has a second DATA record in hour {}",
                header.domain_id, header.game_hour_raw
            ));
        }
        saw[index] = true;
        scan.domains[index].records += 1;

        match header.domain_id {
            D49 => {
                let header49 = d49::parse_header(payload)
                    .map_err(|error| format!("d49 payload at byte {at}: {error}"))?;
                scan.status_or[index] |= header49.status_flags;
                let (failure, _) = classify_status(D49, header49.status_flags, at)?;
                let contract_failure = compact_status_failure_flags_after_expected_partial_tolerance(
                    D49,
                    header49.status_flags,
                ) != 0;
                if failure != contract_failure || failure != header49.is_failure() {
                    return Err(format!("d49 failure classifiers disagree at byte {at}"));
                }
                let (_, rows) = d49::split_payload(payload)
                    .map_err(|error| format!("d49 payload at byte {at}: {error}"))?;
                for row in rows.chunks_exact(d49::ROW_LEN) {
                    d49::validate_row(row)
                        .map_err(|error| format!("d49 row at byte {at}: {error}"))?;
                }
                let outcome = d49_state
                    .fold_payload(&d49_schema, payload, 0)
                    .map_err(|error| format!("d49 fold at byte {at}: {error}"))?;

                let mirror_header = d50::peek_header(payload, d50::LIFECYCLE_ROW_LEN)
                    .map_err(|error| format!("d49 parent header at byte {at}: {error}"))?;
                let mirror_failure = compact_status_failure_flags_after_expected_partial_tolerance(
                    D49,
                    mirror_header.status_flags,
                ) != 0;
                d50_state
                    .fold_lifecycle_payload(
                        payload,
                        mirror_failure,
                        d50::ParentPolicy::IdentityOnly,
                    )
                    .map_err(|error| format!("d49 parent fold at byte {at}: {error}"))?;

                match outcome {
                    d49::FoldOutcome::Folded { .. } => {
                        scan.domain_mut(D49)
                            .measured_hours
                            .push(header.game_hour_raw);
                    }
                    d49::FoldOutcome::LocationRegionIncomplete { .. } => {
                        scan.outcomes.d49_location += 1;
                        scan.add_gap(
                            D49,
                            header.game_hour_raw,
                            GapReason::CoverageShortfall,
                            coverage_witness(),
                            GapSource::PresentRefusal,
                        )?;
                    }
                    d49::FoldOutcome::FailureRetained => {
                        scan.outcomes.d49_failure += 1;
                        let (reason, witness) = status_cause(D49, header49.status_flags);
                        scan.add_gap(
                            D49,
                            header.game_hour_raw,
                            reason,
                            witness,
                            GapSource::PresentRefusal,
                        )?;
                    }
                    d49::FoldOutcome::SideScalarsIncomplete => {
                        scan.outcomes.d49_side_scalars += 1;
                        scan.add_gap(
                            D49,
                            header.game_hour_raw,
                            GapReason::CoverageShortfall,
                            coverage_witness(),
                            GapSource::PresentRefusal,
                        )?;
                    }
                }
            }
            D50 => {
                let header50 = d50::peek_header(payload, d50::ROW_LEN)
                    .map_err(|error| format!("d50 payload at byte {at}: {error}"))?;
                scan.status_or[index] |= header50.status_flags;
                let (failure, _) = classify_status(D50, header50.status_flags, at)?;
                d50::split_payload(payload)
                    .map_err(|error| format!("d50 payload at byte {at}: {error}"))?;
                if failure {
                    scan.outcomes.d50_failure += 1;
                    let (reason, witness) = status_cause(D50, header50.status_flags);
                    scan.add_gap(
                        D50,
                        header.game_hour_raw,
                        reason,
                        witness,
                        GapSource::PresentRefusal,
                    )?;
                    continue;
                }
                match d50_state
                    .fold_payload(d50_collection, payload)
                    .map_err(|error| format!("d50 fold at byte {at}: {error}"))?
                {
                    d50::FoldOutcome::Applied => scan
                        .domain_mut(D50)
                        .measured_hours
                        .push(header.game_hour_raw),
                    d50::FoldOutcome::OrphanMemberReference => {
                        scan.outcomes.d50_orphan += 1;
                        scan.add_gap(
                            D50,
                            header.game_hour_raw,
                            GapReason::CoverageShortfall,
                            coverage_witness(),
                            GapSource::PresentRefusal,
                        )?;
                    }
                    d50::FoldOutcome::DuplicateMemberIdentity => {
                        scan.outcomes.d50_duplicate += 1;
                        scan.add_gap(
                            D50,
                            header.game_hour_raw,
                            GapReason::CoverageShortfall,
                            coverage_witness(),
                            GapSource::PresentRefusal,
                        )?;
                    }
                    d50::FoldOutcome::RetiredMemberResurrection => {
                        scan.outcomes.d50_resurrection += 1;
                        scan.add_gap(
                            D50,
                            header.game_hour_raw,
                            GapReason::CoverageShortfall,
                            coverage_witness(),
                            GapSource::PresentRefusal,
                        )?;
                    }
                }
            }
            D51 => {
                let header51 = d51::peek_header(payload)
                    .map_err(|error| format!("d51 payload at byte {at}: {error}"))?;
                scan.status_or[index] |= header51.status_flags;
                let (failure, _) = classify_status(D51, header51.status_flags, at)?;
                d51::check_authority_payload(payload)
                    .map_err(|error| format!("d51 authority at byte {at}: {error}"))?;
                if failure {
                    scan.outcomes.d51_failure += 1;
                    let (reason, witness) = status_cause(D51, header51.status_flags);
                    scan.add_gap(
                        D51,
                        header.game_hour_raw,
                        reason,
                        witness,
                        GapSource::PresentRefusal,
                    )?;
                    continue;
                }
                match d51_scanner
                    .scan_payload(payload, &mut d51_vocabulary)
                    .map_err(|error| format!("d51 fold at byte {at}: {error}"))?
                    .0
                {
                    D51FoldOutcome::DeltaBeforeBaseline => {
                        scan.outcomes.d51_delta_before += 1;
                        scan.add_gap(
                            D51,
                            header.game_hour_raw,
                            GapReason::CoverageShortfall,
                            coverage_witness(),
                            GapSource::PresentRefusal,
                        )?;
                    }
                    D51FoldOutcome::Baseline | D51FoldOutcome::Delta => scan
                        .domain_mut(D51)
                        .measured_hours
                        .push(header.game_hour_raw),
                }
            }
            D54 => {
                let header54 = d54::peek_header(payload)
                    .map_err(|error| format!("d54 payload at byte {at}: {error}"))?;
                scan.status_or[index] |= header54.status_flags;
                let (failure, tolerated) = classify_status(D54, header54.status_flags, at)?;
                d54::split_payload(payload)
                    .map_err(|error| format!("d54 payload at byte {at}: {error}"))?;
                match d54_state
                    .fold_payload(&d54_schema, payload, failure, tolerated)
                    .map_err(|error| format!("d54 fold at byte {at}: {error}"))?
                {
                    d54::FoldOutcome::Applied => scan
                        .domain_mut(D54)
                        .measured_hours
                        .push(header.game_hour_raw),
                    d54::FoldOutcome::FailureStatus => {
                        scan.outcomes.d54_failure += 1;
                        let (reason, witness) = status_cause(D54, header54.status_flags);
                        scan.add_gap(
                            D54,
                            header.game_hour_raw,
                            reason,
                            witness,
                            GapSource::PresentRefusal,
                        )?;
                    }
                    d54::FoldOutcome::DeltaBeforeBaseline => {
                        scan.outcomes.d54_delta_before += 1;
                        scan.add_gap(
                            D54,
                            header.game_hour_raw,
                            GapReason::CoverageShortfall,
                            coverage_witness(),
                            GapSource::PresentRefusal,
                        )?;
                    }
                    d54::FoldOutcome::ToleratedPartialWithheld => {
                        scan.outcomes.d54_tolerated_partial += 1;
                        let (reason, witness) = status_cause(D54, header54.status_flags);
                        scan.add_gap(
                            D54,
                            header.game_hour_raw,
                            reason,
                            witness,
                            GapSource::PresentRefusal,
                        )?;
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    if open_hour.is_some() {
        return Err(format!("game hour {open_hour:?} is left open at EOF"));
    }
    if last_closed_hour.is_none() {
        return Err("stream closes no game hour".to_string());
    }
    if last_record_type != Some(COMPACT_RECORD_TYPE_FRAME_END) {
        return Err("final record is not FRAME_END".to_string());
    }
    Ok(scan)
}

fn reconcile(scan: &Scan) -> Result<(), String> {
    let d49 = &scan.domains[domain_index(D49)];
    let d50 = &scan.domains[domain_index(D50)];
    let d51 = &scan.domains[domain_index(D51)];
    let d54 = &scan.domains[domain_index(D54)];

    let checks = [
        (
            scan.closed_hours == 514
                && scan.first_hour == Some(60_832_555)
                && scan.last_hour == Some(60_833_068)
                && scan.observations.len() == 322,
            format!(
                "stream closed/first/last/observations = {}/{:?}/{:?}/{}",
                scan.closed_hours,
                scan.first_hour,
                scan.last_hour,
                scan.observations.len()
            ),
        ),
        (
            scan.outcomes.d49_location == 72
                && scan.outcomes.d49_failure == 1
                && scan.outcomes.d49_side_scalars == 0
                && d49.records == 514
                && d49.measured_hours.len() == 441
                && d49.present_refusals == 73,
            format!(
                "d49 location/failure/side/records/measured/refusals = {}/{}/{}/{}/{}/{}",
                scan.outcomes.d49_location,
                scan.outcomes.d49_failure,
                scan.outcomes.d49_side_scalars,
                d49.records,
                d49.measured_hours.len(),
                d49.present_refusals
            ),
        ),
        (
            d49.closed_missing == 0
                && d49.clock_holes == 0
                && d49.non_contiguous() == 52
                && d49.baseline_counts() == (59, 49),
            format!(
                "d49 missing/holes/non-contiguous/baselines/forced = {}/{}/{}/{}/{}",
                d49.closed_missing,
                d49.clock_holes,
                d49.non_contiguous(),
                d49.baseline_counts().0,
                d49.baseline_counts().1
            ),
        ),
        (
            scan.outcomes.d50_failure == 1
                && scan.outcomes.d50_orphan == 0
                && scan.outcomes.d50_duplicate == 0
                && scan.outcomes.d50_resurrection == 0
                && d50.records == 514
                && d50.measured_hours.len() == 513
                && d50.present_refusals == 1
                && d50.closed_missing == 0
                && d50.clock_holes == 0
                && d50.non_contiguous() == 1
                && d50.baseline_counts() == (12, 1),
            format!(
                "d50 failure/orphan/duplicate/resurrection/records/measured/refusals/missing/holes/non-contiguous/baselines/forced = {}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}",
                scan.outcomes.d50_failure,
                scan.outcomes.d50_orphan,
                scan.outcomes.d50_duplicate,
                scan.outcomes.d50_resurrection,
                d50.records,
                d50.measured_hours.len(),
                d50.present_refusals,
                d50.closed_missing,
                d50.clock_holes,
                d50.non_contiguous(),
                d50.baseline_counts().0,
                d50.baseline_counts().1
            ),
        ),
        (
            scan.outcomes.d51_failure == 0
                && scan.outcomes.d51_delta_before == 0
                && d51.records == 514
                && d51.measured_hours.len() == 514
                && d51.present_refusals == 0
                && d51.closed_missing == 0
                && d51.clock_holes == 0
                && d51.non_contiguous() == 0
                && d51.baseline_counts() == (11, 0),
            format!(
                "d51 failure/delta-before/records/measured/refusals/missing/holes/non-contiguous/baselines/forced = {}/{}/{}/{}/{}/{}/{}/{}/{}/{}",
                scan.outcomes.d51_failure,
                scan.outcomes.d51_delta_before,
                d51.records,
                d51.measured_hours.len(),
                d51.present_refusals,
                d51.closed_missing,
                d51.clock_holes,
                d51.non_contiguous(),
                d51.baseline_counts().0,
                d51.baseline_counts().1
            ),
        ),
        (
            scan.outcomes.d54_tolerated_partial == 241
                && scan.outcomes.d54_failure == 7
                && scan.outcomes.d54_delta_before == 0
                && d54.records == 514
                && d54.measured_hours.len() == 266
                && d54.present_refusals == 248
                && d54.closed_missing == 0
                && d54.clock_holes == 0
                && d54.non_contiguous() == 67
                && d54.baseline_counts() == (72, 66),
            format!(
                "d54 tolerated/failure/delta-before/records/measured/refusals/missing/holes/non-contiguous/baselines/forced = {}/{}/{}/{}/{}/{}/{}/{}/{}/{}/{}",
                scan.outcomes.d54_tolerated_partial,
                scan.outcomes.d54_failure,
                scan.outcomes.d54_delta_before,
                d54.records,
                d54.measured_hours.len(),
                d54.present_refusals,
                d54.closed_missing,
                d54.clock_holes,
                d54.non_contiguous(),
                d54.baseline_counts().0,
                d54.baseline_counts().1
            ),
        ),
    ];
    let failures: Vec<String> = checks
        .into_iter()
        .filter_map(|(ok, detail)| (!ok).then_some(detail))
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "anti-drift reconciliation failed; published expectations were not adjusted:\n  {}",
            failures.join("\n  ")
        ))
    }
}

fn encode_records(records: &[GapRecord]) -> Result<(Vec<u8>, Vec<usize>), String> {
    let mut stream = Vec::new();
    let mut lengths = Vec::with_capacity(records.len());
    for record in records {
        let bytes = encode_record(record).map_err(|error| format!("encode §8 gap: {error:?}"))?;
        let decoded =
            decode_record(&bytes).map_err(|error| format!("reader replay §8 gap: {error:?}"))?;
        if decoded != *record {
            return Err("§8 gap reader replay changed a record".to_string());
        }
        lengths.push(bytes.len());
        stream.extend_from_slice(&bytes);
    }
    Ok((stream, lengths))
}

fn retired_record(record: &GapRecord, raw_hour: i32) -> Result<H4GapRecord, String> {
    if record.domains.len() != 1 || record.tick_end != record.tick_start.saturating_add(1) {
        return Err(
            "retired comparison expects one domain and one tick per observation".to_string(),
        );
    }
    if raw_hour_to_tick(raw_hour)? != record.tick_start {
        return Err("retired comparison raw hour does not match its §8 tick".to_string());
    }
    let hour = raw_hour;
    let domain = u16::try_from(record.domains[0])
        .map_err(|_| "gap domain does not fit retired u16 identity".to_string())?;
    let (reason, witness) = match &record.witness {
        GapWitness::StatusBits { bits, .. } if bits.len() == 4 => {
            let raw = u32::from_le_bytes(bits[..4].try_into().expect("four-byte status"));
            let retained = raw
                & (COMPACT_STATUS_SOURCE_UNAVAILABLE
                    | COMPACT_STATUS_PARTIAL_DATA
                    | COMPACT_STATUS_BRIDGE_UNAVAILABLE
                    | COMPACT_STATUS_WORKER_PANIC);
            if let Some(reason) = H4GapReason::canonical_for_status_bits(retained) {
                (
                    reason,
                    H4GapWitness::StatusBits {
                        status_bits: retained,
                    },
                )
            } else {
                (
                    H4GapReason::CoverageShortfall,
                    H4GapWitness::Coverage {
                        expected_perf_records: 1,
                        observed_perf_records: 1,
                        expected_data_records: 1,
                        observed_data_records: 0,
                    },
                )
            }
        }
        GapWitness::TickDiscontinuity { .. } => (
            H4GapReason::SkippedHour,
            H4GapWitness::HourDiscontinuity {
                previous_hour: hour.saturating_sub(1),
                next_hour: hour.saturating_add(1),
            },
        ),
        _ => (
            H4GapReason::CoverageShortfall,
            H4GapWitness::Coverage {
                expected_perf_records: 1,
                observed_perf_records: 1,
                expected_data_records: 1,
                observed_data_records: 0,
            },
        ),
    };
    Ok(H4GapRecord {
        hour_start: hour,
        hour_end: hour,
        reason,
        domains: vec![domain],
        witness,
    })
}

fn encode_retired(
    records: &[GapRecord],
    raw_hours: &[i32],
) -> Result<(Vec<u8>, Vec<usize>), String> {
    if records.len() != raw_hours.len() {
        return Err("retired comparison lost the raw-hour pairing".to_string());
    }
    let mut stream = Vec::new();
    let mut lengths = Vec::with_capacity(records.len());
    for (record, raw_hour) in records.iter().zip(raw_hours.iter().copied()) {
        let retired = retired_record(record, raw_hour)?;
        let bytes = compact_contract::encode_gap_record(&retired)
            .map_err(|error| format!("encode retired H4GP gap: {error:?}"))?;
        let decoded = compact_contract::decode_gap_record(&bytes)
            .map_err(|error| format!("reader replay retired H4GP gap: {error:?}"))?;
        if decoded != retired {
            return Err("retired H4GP reader replay changed a record".to_string());
        }
        lengths.push(bytes.len());
        stream.extend_from_slice(&bytes);
    }
    Ok((stream, lengths))
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file =
        File::create(path).map_err(|error| format!("create {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("write {}: {error}", path.display()))
}

fn write_lengths(path: &Path, lengths: &[usize]) -> Result<(), String> {
    let mut file =
        File::create(path).map_err(|error| format!("create {}: {error}", path.display()))?;
    for length in lengths {
        writeln!(file, "{length}").map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    Ok(())
}

fn zstd3(input: &Path, output: &Path) -> Result<usize, String> {
    let script = "import pathlib,sys,zstandard\n\
data=pathlib.Path(sys.argv[1]).read_bytes()\n\
out=zstandard.ZstdCompressor(level=3).compress(data)\n\
pathlib.Path(sys.argv[2]).write_bytes(out)\n\
print(len(out))";
    let result = Command::new("python")
        .arg("-c")
        .arg(script)
        .arg(input)
        .arg(output)
        .output()
        .map_err(|error| format!("launch Python zstd-3: {error}"))?;
    if !result.status.success() {
        return Err(format!(
            "Python zstd-3 failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    String::from_utf8_lossy(&result.stdout)
        .trim()
        .parse::<usize>()
        .map_err(|error| format!("parse zstd-3 size: {error}"))
}

fn reason_name(reason: usize) -> &'static str {
    match reason {
        0 => "torn sweep",
        1 => "source unavailable",
        2 => "partial data",
        3 => "coalesced",
        4 => "skipped",
        5 => "bridge unavailable",
        6 => "worker panic",
        7 => "coverage shortfall",
        8 => "backpressure",
        _ => "unknown",
    }
}

fn witness_name(kind: usize) -> &'static str {
    match kind {
        0 => "counter",
        1 => "status bits",
        2 => "coverage",
        3 => "tick bracket",
        4 => "tick discontinuity",
        _ => "unknown",
    }
}

fn report(
    scan: &Scan,
    normalised: &[GapRecord],
    unnormalised_raw: usize,
    normalised_raw: usize,
    retired_raw: usize,
    compressed: [usize; 3],
) -> String {
    let merged = scan.observations.len().saturating_sub(normalised.len());
    let domains_total: usize = normalised.iter().map(|record| record.domains.len()).sum();
    let domains_max = normalised
        .iter()
        .map(|record| record.domains.len())
        .max()
        .unwrap_or(0);
    let span_total: u64 = normalised
        .iter()
        .map(|record| record.tick_end - record.tick_start)
        .sum();
    let span_max = normalised
        .iter()
        .map(|record| record.tick_end - record.tick_start)
        .max()
        .unwrap_or(0);

    let mut reasons = [0usize; 9];
    let mut witnesses = [0usize; 5];
    for observation in &scan.observations {
        reasons[observation.reason.code() as usize] += 1;
        witnesses[observation.witness.kind() as usize] += 1;
    }
    let missing_reasons = reasons
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(reason_name(index)))
        .collect::<Vec<_>>()
        .join(", ");
    let missing_witnesses = witnesses
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(witness_name(index)))
        .collect::<Vec<_>>()
        .join(", ");

    let d49 = &scan.domains[domain_index(D49)];
    let d50 = &scan.domains[domain_index(D50)];
    let d51 = &scan.domains[domain_index(D51)];
    let d54 = &scan.domains[domain_index(D54)];
    let d49_gap_bytes: usize = normalised
        .iter()
        .filter(|record| record.domains.contains(&u32::from(D49)))
        .map(|record| {
            encode_record(record)
                .expect("normalised record encodes")
                .len()
        })
        .sum();
    let d54_gap_bytes: usize = normalised
        .iter()
        .filter(|record| record.domains.contains(&u32::from(D54)))
        .map(|record| {
            encode_record(record)
                .expect("normalised record encodes")
                .len()
        })
        .sum();

    format!(
        "# WIRE_FORMAT §8 gap measurement\n\n\
Capture: `deepwar_a/collector_stream.bin`, {EXPECTED_CAPTURE_BYTES} bytes, {} closed game \
hours (`{:?}`..`{:?}`), campaign start raw {EXPECTED_CAMPAIGN_START_RAW}, \
`ticks_per_hour_log2 = {TICKS_PER_HOUR_LOG2}`, legacy H4CR identity mode, branch 0.\n\n\
## Anti-drift reconciliation\n\n\
All checks passed without changing an expected value.\n\n\
| domain | records | measured | record present, fold refused | closed hour, no record | clock-hole ticks | non-contiguous | gap-forced baselines |\n\
|---:|---:|---:|---:|---:|---:|---:|---:|\n\
| d49 | {} | {} | {} | {} | {} | {} | {} |\n\
| d50 | {} | {} | {} | {} | {} | {} | {} |\n\
| d51 | {} | {} | {} | {} | {} | {} | {} |\n\
| d54 | {} | {} | {} | {} | {} | {} | {} |\n\n\
d49 reconciles to 72 location/region refusals plus 1 `PARTIAL_DATA` refusal = 73, 52 \
non-contiguous measured steps, and 49 gap-forced baselines. d50 is deliberately the published \
**parent-identity-only size-accounting arm**, not recorder-faithful parent values: it has 1 \
failure-status refusal, 513 measured hours, 1 non-contiguous step and 1 forced baseline. d51 is \
the negative control: 0 exclusions and 0 non-contiguous steps. d54 has 241 tolerated-partial \
withholds plus 7 hard failure-status exclusions; the published 241/514 withheld anchor and 66 \
of 72 forced-baseline anchor both match.\n\n\
The two source columns are different causes, not double counting: a present refusal came from \
the domain's public `FoldOutcome`; a closed-hour miss means no DATA record existed for that \
domain. Step 2 resolves them only if their tick coverage overlaps.\n\n\
## Layer 1 — structural\n\n\
- observations in: {} (one §8 record per real gapped domain-tick before normalisation)\n\
- records out: {} ({} fewer; {:.2}% of the input record count)\n\
- raw unnormalised §8 stream: {} B\n\
- raw normalised §8 stream: {} B ({:.2}% of unnormalised)\n\
- domains per normalised record: mean {:.3}, max {}\n\
- span length: mean {:.3} ticks, max {} ticks\n\
- every emitted record was replayed through `gap::decode_record` and compared exactly\n\n\
## Layer 2 — whole-stream zstd level 3\n\n\
- unnormalised §8: {} B\n\
- normalised §8: {} B ({:.2}% of unnormalised)\n\
- deviation: `scripts/compress_arms.py` was not used because it requires per-hour offsets and \
always adds a 48-hour block slice. A §8 gap stream is container-level, not a domain-hour arm; \
plain whole-stream zstd **level 3** is the acceptance-authorised path for this shape.\n\n\
## Layer 3 — superseded H4GP\n\n\
- same {} observations as retired `H4GP` records: {} B raw, {} B zstd-3\n\
- normalised §8 is {:.2}% of H4GP raw and {:.2}% of H4GP zstd-3\n\
- every H4GP record was replayed through `compact_contract::decode_gap_record`\n\
- the 48-hour block layer is **inapplicable**: §8 records live in the container-level gap \
stream (`WIRE_FORMAT.md:692`–`:694`, `:166`–`:167`), not inside domain blocks. No block result \
was fabricated.\n\n\
## Encoding consequence\n\n\
The normalised records containing d49 account for {} B of gap-stream encoding (shared records \
are charged whole, not allocated). They accompany the already-published **49** extra one-byte \
empty d49 baselines (+49 B) forced by §1.5 (`bench_sweep_b/sweep_summary.md:51`–`:55`); this bin \
  does not re-run that arm. Records containing d54 account for {} B and accompany the published \
  **66 of 72** gap-forced baselines (`d54_army_battleplans/recipe_gaps.md:73`–`:75`), where \
  baselines are already reported as 99.76% of the wire arm. Churn invariance is established by \
  construction, not measured by this binary: this lane changes no existing `bench_real_*.rs`; no \
  existing churn computation calls the new gap API; and P18 passes empty, ungrouped and grouped \
  gap streams to the same counter while pinning its cross-gap transition.\n\n\
## Real-data coverage caveats\n\n\
- Every real observation is `part = 0`. No ported `dNN` schema declares an event part, so \
`part = 1`, `part = 2`, and real P12 splitting have zero coverage. One more step: implement §9 \
for hybrid candidate d64 delayed events, then take a new d64-enabled capture containing an \
event-side miss (and a state-side miss for `part = 2`). Synthetic P12/P16 tests remain the only \
current evidence.\n\
- All observations are branch 0. P14 cross-branch partition has zero real coverage. One more \
step: record a capture that spans a reload/rollover branch change and derive both sides through \
the same folds.\n\
- Unproduced reasons: {}. One more step is a capture whose collector/bridge/backpressure path \
actually emits the named cause; none is synthesised here.\n\
- Unproduced witnesses: {}. One more step is a capture carrying the corresponding real counter, \
torn-sweep bracket, or discontinuity evidence; none is synthesised here.\n\n\
## Open readings exposed by implementation\n\n\
- §8 does not frame consecutive records. The codec therefore accepts exactly one externally \
framed record; measurement `.lengths` sidecars preserve boundaries. The container grammar must \
choose a record count, byte length, or index before a standalone stream reader exists.\n\
- §8 assigns reason and witness vocabularies but does not state a reason↔witness validity matrix. \
This implementation preserves them independently; the measurement uses status-derived reasons \
when the fold exposes a status, otherwise `coverage shortfall` for a 0-of-1 usable observation. \
A producer contract mapping every acquisition failure to one reason/witness pair would close it.\n\
- Status-bit unused high bits are not assigned semantics. They remain part of the canonical \
witness payload and therefore part of cause ordering; a future padding rule could instead reject \
them, but would need a spec decision and golden vectors.\n\
- Domain 0 is within the stated `u32` bound and is accepted. If 0 is meant to be reserved, the \
domain registry/container contract must say so.\n\
- The retired H4GP `Coverage` witness counts usable DATA coverage in this comparison: a present \
record rejected by the fold is observed 0/expected 1. H4GP cannot name d49's lane-local \
location/region refusal directly. That representational mismatch is part of why the comparison \
is labelled superseded-format accounting, not byte-equivalent semantics.\n\
- d17 war-graph collection membership and d18 CIM1 family availability still require a scope \
narrower than `(domain, part)`. No sub-domain scope is designed here.\n",
        scan.closed_hours,
        scan.first_hour,
        scan.last_hour,
        d49.records,
        d49.measured_hours.len(),
        d49.present_refusals,
        d49.closed_missing,
        d49.clock_holes,
        d49.non_contiguous(),
        d49.baseline_counts().1,
        d50.records,
        d50.measured_hours.len(),
        d50.present_refusals,
        d50.closed_missing,
        d50.clock_holes,
        d50.non_contiguous(),
        d50.baseline_counts().1,
        d51.records,
        d51.measured_hours.len(),
        d51.present_refusals,
        d51.closed_missing,
        d51.clock_holes,
        d51.non_contiguous(),
        d51.baseline_counts().1,
        d54.records,
        d54.measured_hours.len(),
        d54.present_refusals,
        d54.closed_missing,
        d54.clock_holes,
        d54.non_contiguous(),
        d54.baseline_counts().1,
        scan.observations.len(),
        normalised.len(),
        merged,
        100.0 * normalised.len() as f64 / scan.observations.len().max(1) as f64,
        unnormalised_raw,
        normalised_raw,
        100.0 * normalised_raw as f64 / unnormalised_raw.max(1) as f64,
        domains_total as f64 / normalised.len().max(1) as f64,
        domains_max,
        span_total as f64 / normalised.len().max(1) as f64,
        span_max,
        compressed[0],
        compressed[1],
        100.0 * compressed[1] as f64 / compressed[0].max(1) as f64,
        scan.observations.len(),
        retired_raw,
        compressed[2],
        100.0 * normalised_raw as f64 / retired_raw.max(1) as f64,
        100.0 * compressed[1] as f64 / compressed[2].max(1) as f64,
        d49_gap_bytes,
        d54_gap_bytes,
        missing_reasons,
        missing_witnesses,
    )
}

fn run() -> Result<(), String> {
    const USAGE: &str = "usage: bench_gaps <collector_stream.bin> <new output directory>";
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().ok_or(USAGE)?);
    let output = PathBuf::from(args.next().ok_or(USAGE)?);
    if args.next().is_some() {
        return Err(USAGE.to_string());
    }
    if output.exists() {
        return Err(format!(
            "{} already exists; gap measurement artifacts are never overwritten",
            output.display()
        ));
    }
    let input_bytes = input
        .metadata()
        .map_err(|error| format!("stat {}: {error}", input.display()))?
        .len();
    if input_bytes != EXPECTED_CAPTURE_BYTES {
        return Err(format!(
            "{} is {input_bytes} bytes, expected the {EXPECTED_CAPTURE_BYTES}-byte deepwar_a capture",
            input.display()
        ));
    }

    let scan = scan_capture(&input)?;
    reconcile(&scan)?;
    let normalised =
        normalise(&scan.observations).map_err(|error| format!("normalise gaps: {error:?}"))?;
    let (unnormalised_bytes, unnormalised_lengths) = encode_records(&scan.observations)?;
    let (normalised_bytes, normalised_lengths) = encode_records(&normalised)?;
    let (retired_bytes, retired_lengths) =
        encode_retired(&scan.observations, &scan.raw_observation_hours)?;

    std::fs::create_dir(&output)
        .map_err(|error| format!("create {}: {error}", output.display()))?;
    let unnormalised_path = output.join("unnormalised_s8.bin");
    let normalised_path = output.join("normalised_s8.bin");
    let retired_path = output.join("retired_h4gp.bin");
    write_bytes(&unnormalised_path, &unnormalised_bytes)?;
    write_bytes(&normalised_path, &normalised_bytes)?;
    write_bytes(&retired_path, &retired_bytes)?;
    write_lengths(
        &output.join("unnormalised_s8.lengths"),
        &unnormalised_lengths,
    )?;
    write_lengths(&output.join("normalised_s8.lengths"), &normalised_lengths)?;
    write_lengths(&output.join("retired_h4gp.lengths"), &retired_lengths)?;

    let compressed = [
        zstd3(&unnormalised_path, &output.join("unnormalised_s8.bin.zst"))?,
        zstd3(&normalised_path, &output.join("normalised_s8.bin.zst"))?,
        zstd3(&retired_path, &output.join("retired_h4gp.bin.zst"))?,
    ];
    let measurement = report(
        &scan,
        &normalised,
        unnormalised_bytes.len(),
        normalised_bytes.len(),
        retired_bytes.len(),
        compressed,
    );
    write_bytes(&output.join("measurement.md"), measurement.as_bytes())?;
    println!("{measurement}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("bench_gaps: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multi_hour_clock_hole_uses_the_actual_observed_bracket_and_one_maximal_span() {
        let last = EXPECTED_CAMPAIGN_START_RAW + 10;
        let current = EXPECTED_CAMPAIGN_START_RAW + 14;
        let mut scan = Scan::default();
        add_clock_hole(&mut scan, last, current).unwrap();

        let previous_tick = raw_hour_to_tick(last).unwrap();
        let next_tick = raw_hour_to_tick(current).unwrap();
        assert_eq!(scan.observations.len(), 3 * DOMAINS.len());
        assert!(scan.observations.iter().all(|record| {
            record.witness
                == (GapWitness::TickDiscontinuity {
                    previous_tick,
                    next_tick,
                })
        }));

        let normalised = normalise(&scan.observations).unwrap();
        assert_eq!(normalised.len(), 1);
        assert_eq!(normalised[0].tick_start, previous_tick + 1);
        assert_eq!(normalised[0].tick_end, next_tick);
        assert_eq!(
            normalised[0].domains,
            DOMAINS
                .iter()
                .copied()
                .map(u32::from)
                .collect::<Vec<_>>(),
            "all four domains share the same actual discontinuity witness"
        );
    }
}
