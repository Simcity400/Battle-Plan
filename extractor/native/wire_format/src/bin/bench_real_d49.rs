//! Measure the wire format against the shipping compact stream on REAL d49
//! `naval_combat_lifecycle_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d49-scheduled capture
//! (`runs/batch3/deepwar_a`). The stream's own d49 DATA records are the shipping
//! format's cost for the domain: a 28-byte header, a fixed 72-byte row per
//! **changed** battle, and a two-byte empty string table — exactly what the
//! recorder stores today.
//!
//! **d49 is a pure cumulative delta with no baseline record and no mode word, and
//! that shapes the whole measurement.** `fold_naval_combat_lifecycle` says so
//! itself (`world_fold.rs:20091`-`:20095`): the collector emits every active row
//! on the first frame against an empty previous baseline and only changes
//! thereafter, so nothing ever clears the accumulator, there is no
//! `reject_delta_before_baseline` path, and an hour's state is the accumulator
//! after that hour's record rather than the record's rows. This tool folds through
//! `wire_format::d49::State::fold_payload` — the same public boundary the module's
//! own tests use. The shipping arm consequently re-baselines **never** while the
//! wire arm re-baselines every 48 measured hours; the deliverable reports the
//! baseline/delta split so the grammar comparison stays recoverable underneath
//! that policy difference.
//!
//! **The stream is read incrementally, not slurped.** `batch3/deepwar_a` is
//! 3.6 GB and several ports measure it concurrently, so this bin walks the file
//! through a `BufReader` one record at a time and retains only its own domain's
//! bytes (42 KB over the whole capture). Every validation below still sees every
//! record in order; the only difference from the slurping bins is that a record's
//! payload lives in a reused buffer instead of a slice of one huge allocation.
//!
//! **Nothing here re-derives a contract it can call.** Header, row and frame
//! decoding go through `wire_format::d49`'s public boundary; H4CR framing, record
//! semantics and sequence continuity go through `compact_contract`'s own parser
//! and stateful validator. A harness that reimplements either drifts from the
//! recorder without anything failing.
//!
//! **The whole stream is validated before any artifact is written.** A stream that
//! is truncated, spliced, or corrupt would otherwise yield a smaller but entirely
//! plausible measurement — a silently wrong number, which is worse than a crash.
//! `scan_stream` therefore enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`
//!   (magic, version, header length, payload bound);
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`
//!   (sequence continuity from the stream's first record, record-type and
//!   record-flag semantics, FRAME_END shape);
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`): a DOMAIN_DATA record opens its game
//!   hour, a FRAME_END closes the open hour and only that hour, and closed hours
//!   strictly increase;
//! - that tracker's **DOMAIN_PERF hour binding** (`:328`-`:365`): no duplicate
//!   non-probe PERF domain inside one hour, no PERF for an hour other than the
//!   open one, no PERF for an already closed hour, and no conflict with the
//!   pending PERF hour before a DATA record opens one;
//! - the **FRAME_END seal fields**, `validate_frame_end_seal_fields` (`:46`-`:89`),
//!   re-implemented here as [`check_frame_end_seal`]: the 32-byte payload length,
//!   `reserved == 0`, `emitted_domains == scheduled_domains`, no status bit above
//!   13, and no fatal status bit; plus the seal's domain-count cross-check against
//!   the distinct non-probe PERF domains actually observed in the hour
//!   (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. Production additionally calls `validate_domain_data_payload_at`
//! on every DOMAIN_DATA record (`:594`), so a malformed payload for another domain
//! remains outside this single-domain harness.
//!
//! **d49's own independent payload authority, and the ONE predicate this harness
//! deliberately does not treat as a stream rejection.**
//! `decode_compact_naval_combat_lifecycle_live_data_payload`
//! (`domain_data_authority.rs:8654`) is exactly
//! `naval4950::decode_lifecycle_data_payload` — the 28-byte header predicates, the
//! row-bounds walk, the empty string table with no trailing bytes, and
//! `validate_decoded_lifecycle_row` on every row — **plus**
//! `validate_d49_promoted_location_region` (`:8690`). The first part is
//! reimplemented by nothing: `d49::split_payload` and `d49::validate_row` ARE those
//! predicates, and [`check_authority_payload`] runs them on every DOMAIN_DATA
//! record whatever its status, so a malformed d49 payload refuses the stream.
//!
//! The second part is different, and the difference is the reason this capture
//! exists as a documented case. `validate_d49_promoted_location_region` refuses a
//! record whose status carries the lane-local bit 11, or whose live rows omit the
//! promoted location/region pair — the identical predicate the FOLD applies at
//! `world_fold.rs:20047`-`:20062`. The two authorities disagree about what that
//! means: the authority makes it a certification failure (this capture's
//! `collector_record.stderr.log` says *"refusing to seal uncertified compact
//! stream: 72 DOMAIN_DATA payload record(s) failed exact validation"*, and those
//! 72 records are precisely d49's bit-11 hours), while the production fold path
//! marks the domain-hour unknown, **retains the accumulator and returns `Ok`**.
//!
//! This harness measures what the fold produces, so it takes the fold's
//! classification: those hours are **excluded hours**, not stream rejections
//! (`runs/batch3/domain_classification.md`, Finding 1, states the same
//! requirement). They are counted and reported by name, never silently skipped,
//! and the certification consequence is reported beside the size numbers rather
//! than absorbed. Every OTHER authority predicate is fail-closed here.
//!
//! **d50 is the only other writer of the recorder's `naval_combats` accumulator,
//! and it is proved inert for this collection** — the recipe's second treatment,
//! by construction rather than per capture. `fold_naval_combat_members`
//! (`world_fold.rs:20131`-`:20222`) reaches the map only through
//! `self.naval_combats.get_mut(&combat_key)` and then does
//! `combat.members.insert(..)` or `combat.members.remove(..)`
//! (`world_fold.rs:20205`-`:20217`). It never inserts an entry, never removes one,
//! and never assigns a lifecycle scalar; its `members` sub-map is d50's own
//! collection, which this port does not carry. A grep over `recorder_host` and
//! `collector` finds no third writer. The bin still counts the d50 DATA records it
//! walks past, so the report shows the writer was live in this capture rather than
//! absent from it.
//!
//! Four exclusion paths, all taken from the fold's own call order, and their
//! POSITIONS are not guessable — two of them straddle the failure gate:
//!
//! - refusal 1, location/region promotion incomplete, BEFORE the failure gate
//!   (`world_fold.rs:20047`);
//! - failure status, via `compact_contract`'s own predicate, never a copied mask
//!   (`:20066`);
//! - refusal 3, side scalars incomplete, AFTER the failure gate (`:20076`);
//! - (there is no delta-before-baseline path: a pure incremental payload has no
//!   baseline to gate against, and asserting one was the d10 round-1 finding.)
//!
//! Hour uniqueness is enforced across ALL d49 DATA records, of any status, and
//! BEFORE any exclusion, so a success+failure pair inside one hour cannot slip
//! through.
//!
//! Outputs, for the compression step (`scripts/compress_arms.py`, index mode —
//! this is an index-based bin):
//!   ship_d49.bin   — the d49 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line
//!   *.hours        — the per-hour game clock, one per measured hour

use std::collections::BTreeSet;
use std::io::{BufReader, Read, Write};

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance,
    compact_status_for_global_interpretation, compact_status_unknown_flags_v1,
    lane_local_status_mask, parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_DELTA_DATA,
    COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d49;

const D49: u16 = 49;
const D50: u16 = 50;
const BLOCK_HOURS: usize = 48;

/// One usable d49 hour: its game hour, the folded state **after** that hour's
/// record, and the record's bytes (the shipping arm is those bytes verbatim,
/// header included).
struct D49Hour {
    game_hour: i32,
    state: KeyedState,
    ship_bytes: Vec<u8>,
    /// Rows the record carried, split by kind — the shipping arm's own churn,
    /// which for this domain is not the same quantity as the wire arm's.
    upserts: usize,
    deletes: usize,
}

/// Every non-folding path, counted by the fold's own reason. None of these is a
/// stream rejection: all four retain the accumulator.
#[derive(Default)]
struct Exclusions {
    /// Refusal 1 fired on the lane-local status badge (bit 11).
    location_status_badge: usize,
    /// Refusal 1 fired on a live row omitting `location_id` or `region_id`.
    location_row_incomplete: usize,
    /// Both halves of refusal 1 fired on the same record.
    location_both: usize,
    /// `handle_compact_failure`.
    failure: usize,
    /// Refusal 3.
    side_scalars: usize,
    /// The shipping bytes of every excluded record, so the accounting is complete.
    bytes: usize,
}

impl Exclusions {
    fn total(&self) -> usize {
        self.location_status_badge
            + self.location_row_incomplete
            + self.location_both
            + self.failure
            + self.side_scalars
    }
}

struct StreamScan {
    hours: Vec<D49Hour>,
    excluded: Exclusions,
    /// Measured d49 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// d49 DATA records seen, of any status.
    d49_records: usize,
    /// d50 DATA records seen — the other writer of the recorder's accumulator,
    /// counted so the report shows it was live in this capture.
    d50_records: usize,
    /// The OR of every d49 status word, so the report can state the vocabulary.
    status_or: u32,
}

/// Every globally-defined compact status occupies one of bits 0..=13
/// (`recorder_host/src/stream_fold.rs:43`).
const FRAME_END_KNOWN_STATUS_MASK: u32 = (1 << 14) - 1;

/// The reader-context probe domain, excluded from seal domain accounting
/// (`recorder_host/src/stream_fold.rs:330`).
const PROBE_DOMAIN_ID: u16 = 250;

/// Mirror `validate_frame_end_seal_fields`
/// (`recorder_host/src/stream_fold.rs:46`-`:89`).
fn check_frame_end_seal(payload: &[u8], at_byte: usize) -> Result<(u32, u32), String> {
    if payload.len() != COMPACT_FRAME_END_PAYLOAD_LEN {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: payload length {} is not \
             {COMPACT_FRAME_END_PAYLOAD_LEN}",
            payload.len()
        ));
    }
    let emitted_domains = u32::from_le_bytes(payload[16..20].try_into().expect("4 bytes"));
    let scheduled_domains = u32::from_le_bytes(payload[20..24].try_into().expect("4 bytes"));
    let status_flags = u32::from_le_bytes(payload[24..28].try_into().expect("4 bytes"));
    let reserved = u32::from_le_bytes(payload[28..32].try_into().expect("4 bytes"));
    if reserved != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: reserved word is nonzero \
             ({reserved:#010x})"
        ));
    }
    if emitted_domains != scheduled_domains {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: emitted domain count \
             {emitted_domains} does not match scheduled domain count {scheduled_domains}"
        ));
    }
    let unknown = status_flags & !FRAME_END_KNOWN_STATUS_MASK;
    if unknown != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: status flags {status_flags:#010x} \
             contain unknown bits {unknown:#010x}"
        ));
    }
    let fatal = status_flags & COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: status flags {status_flags:#010x} \
             contain fatal bits {fatal:#010x}"
        ));
    }
    Ok((emitted_domains, scheduled_domains))
}

/// Replay the structural predicates of d49's status-independent second authority
/// (`domain_data_authority.rs:8654`, which is `decode_lifecycle_data_payload` plus
/// `validate_d49_promoted_location_region`).
///
/// The domain module owns every one of them, so they are called rather than
/// re-derived: `split_payload` is the header, row-bounds and string-table walk,
/// and `validate_row` is `validate_decoded_lifecycle_row`. The location/region
/// predicate is **not** here — see the module docs for why it is classified as an
/// excluded hour instead, and `fold_payload` is what applies it.
fn check_authority_payload(payload: &[u8], at_byte: usize) -> Result<(), String> {
    let (_, body) =
        d49::split_payload(payload).map_err(|e| format!("d49 payload at byte {at_byte}: {e}"))?;
    for row in body.chunks_exact(d49::ROW_LEN) {
        d49::validate_row(row).map_err(|e| format!("d49 row at byte {at_byte}: {e}"))?;
    }
    Ok(())
}

/// One framed record, read out of the stream into reusable buffers.
struct RecordReader<R: Read> {
    inner: R,
    header: [u8; COMPACT_RECORD_HEADER_LEN],
    payload: Vec<u8>,
    pos: usize,
}

impl<R: Read> RecordReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            header: [0u8; COMPACT_RECORD_HEADER_LEN],
            payload: Vec::new(),
            pos: 0,
        }
    }

    /// Read the next record, or `Ok(None)` at a clean record boundary. A stream
    /// ending mid-record is an `Err`, exactly as `eof_check` treats it: that is
    /// not a short stream, it is a corrupt one.
    fn next(&mut self) -> Result<Option<(compact_contract::CompactRecordHeader, usize)>, String> {
        let mut filled = 0usize;
        while filled < COMPACT_RECORD_HEADER_LEN {
            let n = self
                .inner
                .read(&mut self.header[filled..])
                .map_err(|e| format!("read at byte {}: {e}", self.pos + filled))?;
            if n == 0 {
                if filled == 0 {
                    return Ok(None);
                }
                return Err(format!(
                    "stream ends mid-record at byte {}: {filled} header byte(s) present, \
                     record needs {COMPACT_RECORD_HEADER_LEN}",
                    self.pos
                ));
            }
            filled += n;
        }
        let header = parse_compact_record_header(&self.header)
            .map_err(|e| format!("record at byte {}: {e}", self.pos))?;
        self.payload.clear();
        self.payload.resize(header.payload_len, 0);
        let mut filled = 0usize;
        while filled < header.payload_len {
            let n = self
                .inner
                // The absolute position of the next payload byte is the record's
                // start PLUS the header this reader has already consumed. Naming
                // `self.pos + filled` here would report every payload I/O error
                // 32 bytes early, which sends a reader of the message to the
                // wrong record.
                .read(&mut self.payload[filled..])
                .map_err(|e| {
                    format!(
                        "read at byte {}: {e}",
                        self.pos + COMPACT_RECORD_HEADER_LEN + filled
                    )
                })?;
            if n == 0 {
                return Err(format!(
                    "stream ends mid-record at byte {}: {} payload byte(s) present, record \
                     needs {}",
                    self.pos, filled, header.payload_len
                ));
            }
            filled += n;
        }
        let start = self.pos;
        self.pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        Ok(Some((header, start)))
    }
}

/// Validate the whole stream against the canonical contract and fold its d49 DATA
/// records into per-hour states. Every rejection path is an `Err`, never a
/// silently shorter measurement and never a panic.
fn scan_stream<R: Read>(
    reader: R,
    schema: &wire_format::Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // The constants `d49` names locally, against the contract crate that owns
    // them. A drift here would silently change which hours are measured.
    if d49::STATUS_DELTA_DATA != COMPACT_STATUS_DELTA_DATA {
        return Err("d49::STATUS_DELTA_DATA disagrees with compact_contract".to_string());
    }
    if d49::FAILURE_STATUS_MASK != COMPACT_STATUS_FAILURE_MASK {
        return Err("d49::FAILURE_STATUS_MASK disagrees with compact_contract".to_string());
    }
    if d49::LANE_LOCAL_STATUS_MASK != lane_local_status_mask(D49) {
        return Err("d49::LANE_LOCAL_STATUS_MASK disagrees with compact_contract".to_string());
    }

    // The protocol is stated by the caller, never inferred from the first record
    // (`stream_fold.rs:220`; `stream_validator.rs:328`). A decapitated IdentityV1
    // stream otherwise looks exactly like legacy.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D49Hour> = Vec::new();
    let mut excluded = Exclusions::default();
    let mut non_contiguous = 0usize;
    let mut d49_records = 0usize;
    let mut d50_records = 0usize;
    let mut status_or = 0u32;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d49 DATA record of ANY status — the uniqueness rule.
    let mut last_d49_hour: Option<i32> = None;
    // Hour of the last d49 record that entered the measurement — the contiguity
    // report, which a skipped excluded hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // The accumulator, carried across records exactly as the recorder carries it.
    let mut state = d49::empty_state();

    let mut records = RecordReader::new(reader);
    while let Some((header, pos)) = records.next()? {
        let payload = &records.payload[..];

        validator
            .validate_record(&header, payload)
            .map_err(|e| format!("record at byte {pos}: {e:?}"))?;

        // The hour-boundary subset of `BoundaryTracker::observe`.
        match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                if header.domain_id != PROBE_DOMAIN_ID
                    && !non_probe_perf_domains.insert(header.domain_id)
                {
                    return Err(format!(
                        "duplicate DOMAIN_PERF for domain {} in hour {} at byte {pos}",
                        header.domain_id, header.game_hour_raw
                    ));
                }
                match open_hour {
                    Some(open) if open != header.game_hour_raw => {
                        return Err(format!(
                            "hour interleaving at byte {pos}: DOMAIN_PERF for hour {} while \
                             hour {open} is still open",
                            header.game_hour_raw
                        ))
                    }
                    Some(_) => {}
                    None => {
                        if let Some(last_closed) = last_closed_hour {
                            if header.game_hour_raw <= last_closed {
                                return Err(format!(
                                    "non-monotonic DOMAIN_PERF at byte {pos}: closed hour \
                                     {last_closed}, then {}",
                                    header.game_hour_raw
                                ));
                            }
                        }
                        match pending_perf_hour {
                            Some(pending) if pending != header.game_hour_raw => {
                                return Err(format!(
                                    "PERF hour conflict at byte {pos}: pending hour {pending}, \
                                     then {}",
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
                        "DOMAIN_DATA at byte {pos} carries hour {} while hour {open} is open",
                        header.game_hour_raw
                    ))
                }
                Some(_) => {}
                None => {
                    // `stream_fold.rs:378`-`:386`: DATA must open the hour any
                    // pending PERF records already bound.
                    if let Some(pending) = pending_perf_hour {
                        if pending != header.game_hour_raw {
                            return Err(format!(
                                "DATA hour conflict at byte {pos}: pending PERF hour \
                                 {pending}, then DATA hour {}",
                                header.game_hour_raw
                            ));
                        }
                    }
                    if let Some(last_closed) = last_closed_hour {
                        if header.game_hour_raw <= last_closed {
                            return Err(format!(
                                "DOMAIN_DATA at byte {pos} reopens hour {} after hour \
                                 {last_closed} closed",
                                header.game_hour_raw
                            ));
                        }
                    }
                    open_hour = Some(header.game_hour_raw);
                }
            },
            COMPACT_RECORD_TYPE_FRAME_END => {
                // Seal validation precedes the open-hour match in production
                // (`stream_fold.rs:401`-`:419`).
                let (emitted_domains, scheduled_domains) = check_frame_end_seal(payload, pos)?;
                let observed_perf_domains =
                    non_probe_perf_domains.len().min(u32::MAX as usize) as u32;
                if observed_perf_domains != emitted_domains
                    || observed_perf_domains != scheduled_domains
                {
                    return Err(format!(
                        "FRAME_END domain counts at byte {pos} claim emitted={emitted_domains} \
                         scheduled={scheduled_domains}, but observed {observed_perf_domains} \
                         unique non-probe DOMAIN_PERF domains"
                    ));
                }
                match open_hour {
                    Some(open) if open == header.game_hour_raw => {}
                    Some(open) => {
                        return Err(format!(
                            "FRAME_END at byte {pos} closes hour {} while hour {open} is open",
                            header.game_hour_raw
                        ))
                    }
                    None => {
                        return Err(format!(
                            "FRAME_END at byte {pos} closes hour {} without an open \
                             DOMAIN_DATA hour",
                            header.game_hour_raw
                        ))
                    }
                }
                if let Some(last_closed) = last_closed_hour {
                    if header.game_hour_raw <= last_closed {
                        return Err(format!(
                            "non-monotonic FRAME_END at byte {pos}: closed hour \
                             {last_closed}, then {}",
                            header.game_hour_raw
                        ));
                    }
                }
                open_hour = None;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
                last_closed_hour = Some(header.game_hour_raw);
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D50 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d50_records += 1;
        }

        if header.domain_id == D49 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d49_records += 1;
            // One d49 DATA record per game hour, checked over EVERY d49 record and
            // BEFORE any exclusion. d49 accumulates, so a second record in one hour
            // would fold twice into one measurement step — and gating this on the
            // record being usable would let a success/excluded pair through the
            // rule entirely.
            if let Some(previous) = last_d49_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d49 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d49_hour = Some(header.game_hour_raw);

            let d49_header = d49::parse_header(payload)
                .map_err(|e| format!("d49 payload at byte {pos}: {e}"))?;
            status_or |= d49_header.status_flags;

            // VALIDITY BEFORE CLASSIFICATION. Everything below that reads the
            // status word classifies the v1 vocabulary; none of it is a validity
            // predicate. `compact_contract/src/lib.rs:121`-`:123` says so in as
            // many words: "Callers handling untrusted status words must first
            // reject a non-zero `compact_status_unknown_flags_v1` result;
            // otherwise an unassigned bit has no defined classification." Bits
            // 14..=31 are unassigned (`status_flags_base.rs:21`-`:24`).
            //
            // The unassigned test runs on the GLOBALLY interpretable view — the
            // lane-local mask stripped first — because bit 11 is d49's own badge
            // and is not up for global classification at all. (Both give the same
            // answer today, since the lane-local mask is a subset of the v1 known
            // mask; taking the global view states the intent rather than relying
            // on that coincidence.)
            let global_status =
                compact_status_for_global_interpretation(D49, d49_header.status_flags);
            let unknown_status = compact_status_unknown_flags_v1(global_status);
            if unknown_status != 0 {
                return Err(format!(
                    "d49 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // The structural half of the independent payload authority, on every
            // DOMAIN_DATA record whatever its status (`stream_fold.rs:594`).
            check_authority_payload(payload, pos)?;

            // The fold's own predicate, not a copied bit mask: it applies d49's
            // lane-local mask and expected-partial tolerance (none), so it cannot
            // drift from the recorder. `d49::PayloadHeader::is_failure` reaches the
            // same verdict from its own constants; both are asserted to agree.
            let contract_failure = compact_status_failure_flags_after_expected_partial_tolerance(
                D49,
                d49_header.status_flags,
            ) != 0;
            if contract_failure != d49_header.is_failure() {
                return Err(format!(
                    "d49 payload at byte {pos}: failure verdicts disagree"
                ));
            }

            let record_len = COMPACT_RECORD_HEADER_LEN + header.payload_len;
            let outcome = state
                .fold_payload(schema, payload, 0)
                .map_err(|e| format!("d49 payload at byte {pos}: {e}"))?;

            match outcome {
                d49::FoldOutcome::LocationRegionIncomplete {
                    status_badge,
                    row_incomplete,
                } => {
                    match (status_badge, row_incomplete) {
                        (true, true) => excluded.location_both += 1,
                        (true, false) => excluded.location_status_badge += 1,
                        (false, true) => excluded.location_row_incomplete += 1,
                        (false, false) => {
                            return Err(format!(
                                "d49 payload at byte {pos}: refusal 1 with neither cause"
                            ))
                        }
                    }
                    excluded.bytes += record_len;
                }
                d49::FoldOutcome::FailureRetained => {
                    if !contract_failure {
                        return Err(format!(
                            "d49 payload at byte {pos}: failure verdicts disagree"
                        ));
                    }
                    excluded.failure += 1;
                    excluded.bytes += record_len;
                }
                d49::FoldOutcome::SideScalarsIncomplete => {
                    excluded.side_scalars += 1;
                    excluded.bytes += record_len;
                }
                d49::FoldOutcome::Folded { upserts, deletes } => {
                    if contract_failure {
                        return Err(format!(
                            "d49 payload at byte {pos}: failure verdicts disagree"
                        ));
                    }
                    if let Some(previous) = last_measured_hour {
                        if header.game_hour_raw != previous + 1 {
                            non_contiguous += 1;
                        }
                    }
                    last_measured_hour = Some(header.game_hour_raw);
                    let mut ship_bytes =
                        Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + header.payload_len);
                    ship_bytes.extend_from_slice(&records.header);
                    ship_bytes.extend_from_slice(payload);
                    hours.push(D49Hour {
                        game_hour: header.game_hour_raw,
                        state: state.combats.clone(),
                        ship_bytes,
                        upserts,
                        deletes,
                    });
                }
            }
        }
    }

    // `BoundaryTracker::eof_check` (`recorder_host/src/stream_fold.rs:459`).
    if let Some(open) = open_hour {
        return Err(format!("game hour {open} left open at EOF"));
    }
    if last_closed_hour.is_none() {
        return Err("stream closes no game hour (no FRAME_END records)".to_string());
    }
    if last_record_type != Some(COMPACT_RECORD_TYPE_FRAME_END) {
        return Err(format!(
            "stream ends after record type {}; final record must be a canonical FRAME_END",
            last_record_type.map_or_else(|| "none".to_string(), |k| k.to_string())
        ));
    }
    if hours.is_empty() {
        return Err("stream carries no usable d49 DATA records".to_string());
    }

    Ok(StreamScan {
        hours,
        excluded,
        non_contiguous,
        d49_records,
        d50_records,
        status_or,
    })
}

fn main() {
    const USAGE: &str =
        "usage: bench_real_d49 <collector_stream.bin> <out dir> [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let file = std::fs::File::open(&input).expect("open stream");
    let reader = BufReader::with_capacity(1 << 22, file);
    let schema = d49::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d49 DATA records. ----
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(reader, &schema, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads the stream again. ------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];
    let mut hour_clock: Vec<i32> = Vec::new();

    let mut prior: Option<&KeyedState> = None;
    let mut reader_state: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut delta_changed_rows = 0usize;
    let mut delta_tombstones = 0usize;
    let mut ship_upserts_total = 0usize;
    let mut ship_deletes_total = 0usize;
    let mut rows_total = 0usize;
    let mut max_rows = 0usize;
    // Cadence-independent churn: changed rows over MEASURED-HOUR TRANSITIONS,
    // taken before the block decision, so a change landing on a block boundary
    // still counts. The delta-path numbers above divide by emitted-delta hours
    // instead; dividing an idle domain's churn by that denominator made d17 look
    // ~900x busier, a round-2 review finding.
    let mut transitions = 0usize;
    let mut transition_changed_rows = 0usize;
    let mut transition_tombstones = 0usize;

    for hour in &scan.hours {
        ship.extend_from_slice(&hour.ship_bytes);
        ship_offsets.push(ship.len());
        hour_clock.push(hour.game_hour);
        ship_upserts_total += hour.upserts;
        ship_deletes_total += hour.deletes;

        // The hour's state: the accumulator AFTER this hour's record.
        let state: &KeyedState = &hour.state;
        rows_total += state.len();
        max_rows = max_rows.max(state.len());

        if let Some(prev) = prior {
            let d = diff(collection, prev, state);
            transitions += 1;
            transition_changed_rows += d.changed.len();
            transition_tombstones += d.tombstones.len();
        }

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let block_start = hours_seen % BLOCK_HOURS == 0;
        let mut emitted = true;
        if block_start || prior.is_none() {
            encode_baseline(&schema, std::slice::from_ref(state), &mut wire)
                .expect("baseline encodes");
            baselines += 1;
            baseline_bytes += wire.len() - hour_start;
        } else {
            let prev = prior.unwrap();
            let d = diff(collection, prev, state);
            if d.is_empty() {
                empty_hours += 1;
                emitted = false;
            } else {
                delta_changed_rows += d.changed.len();
                delta_tombstones += d.tombstones.len();
                encode_delta(&schema, std::slice::from_ref(prev), &[Some(d)], &mut wire)
                    .expect("delta encodes");
                delta_bytes += wire.len() - hour_start;
            }
        }
        wire_offsets.push(wire.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require
        // the result to be stored-equal to this hour's state. This is what makes
        // the size claim a claim about decodable bytes. An hour that emitted
        // nothing is still checked: the reader's retained state must already equal
        // the fold's, which is precisely what "nothing changed" asserts.
        if emitted {
            let hour_bytes = &wire[hour_start..];
            if block_start || reader_state.is_none() {
                let (states, consumed) =
                    decode_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                reader_state = Some(states.into_iter().next().unwrap());
            } else {
                let reader_prior = reader_state.take().unwrap();
                let (deltas, consumed) =
                    decode_delta(&schema, std::slice::from_ref(&reader_prior), hour_bytes, 0)
                        .expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                let mut next = reader_prior;
                apply(&mut next, deltas[0].as_ref().unwrap());
                reader_state = Some(next);
            }
        }
        let replayed = reader_state.as_ref().unwrap();
        assert_eq!(
            replayed.len(),
            state.len(),
            "game hour {}: row count",
            hour.game_hour
        );
        let residual = diff(collection, replayed, state);
        assert!(
            residual.is_empty(),
            "game hour {}: reader state is not stored-equal to the fold",
            hour.game_hour
        );

        prior = Some(state);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d49.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d49.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }
    for name in ["ship_d49.hours", "wire_new.hours"] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for h in &hour_clock {
            writeln!(f, "{h}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    let e = &scan.excluded;
    println!(
        "d49 DATA records     {} ({} measured, {} excluded)   d50 records {}",
        scan.d49_records,
        hours_seen,
        e.total(),
        scan.d50_records
    );
    println!("status vocabulary    {:#x} (OR over every d49 record)", scan.status_or);
    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        scan.non_contiguous
    );
    println!(
        "excluded hours       refusal1-badge {} + refusal1-row {} + refusal1-both {} \
         + failure {} + refusal3-side-scalars {}  = {} ({} shipping bytes)",
        e.location_status_badge,
        e.location_row_incomplete,
        e.location_both,
        e.failure,
        e.side_scalars,
        e.total(),
        e.bytes
    );
    println!(
        "shipping churn       {:.3} upserts/hour, {:.3} deletes/hour ({} + {} rows total)",
        ship_upserts_total as f64 / hours_seen as f64,
        ship_deletes_total as f64 / hours_seen as f64,
        ship_upserts_total,
        ship_deletes_total
    );
    println!(
        "table rows avg       {:.3} (max {max_rows})",
        rows_total as f64 / hours_seen as f64
    );
    println!("baselines            {baselines} (every {BLOCK_HOURS} measured hours)");
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn (cadence-independent) {:.4} changed rows/hour, {:.4} tombstones/hour \
         over {transitions} measured-hour transitions",
        transition_changed_rows as f64 / transitions.max(1) as f64,
        transition_tombstones as f64 / transitions.max(1) as f64
    );
    println!(
        "real churn (delta path)          {:.4} changed rows/hour, {:.4} tombstones/hour \
         over {delta_hours} emitted-delta hours",
        delta_changed_rows as f64 / delta_hours.max(1) as f64,
        delta_tombstones as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d49.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.2}%)   baseline {} + delta {}",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64,
        baseline_bytes,
        delta_bytes
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_DOMAIN_PERF_BASE_LEN};

    /// Build one H4CR record the way the recorder frames it
    /// (`compact_contract/src/record_header.rs:60`).
    fn record(sequence: u64, game_hour: i32, domain: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes()); // version
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // record flags
        out.extend_from_slice(payload);
        out
    }

    fn frame_end(sequence: u64, game_hour: i32) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        )
    }

    fn perf(sequence: u64, game_hour: i32, domain: u16) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN],
        )
    }

    fn frame_end_sealed(
        sequence: u64,
        game_hour: i32,
        emitted: u32,
        scheduled: u32,
        status: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::with_capacity(COMPACT_FRAME_END_PAYLOAD_LEN);
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&emitted.to_le_bytes());
        payload.extend_from_slice(&scheduled.to_le_bytes());
        payload.extend_from_slice(&status.to_le_bytes());
        payload.extend_from_slice(&reserved.to_le_bytes());
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        combat_id: u32,
        generation: u64,
        flags: u32,
        location_id: u32,
        region_id: u32,
        counts: [u32; 2],
        positioning: [i32; 2],
        anti_air: [i64; 2],
    ) -> Vec<u8> {
        let mut r = Vec::with_capacity(d49::ROW_LEN);
        r.extend_from_slice(&combat_id.to_le_bytes());
        r.extend_from_slice(&generation.to_le_bytes());
        r.extend_from_slice(&0u64.to_le_bytes()); // body_ptr
        r.extend_from_slice(&flags.to_le_bytes());
        r.extend_from_slice(&location_id.to_le_bytes());
        r.extend_from_slice(&region_id.to_le_bytes());
        for v in counts {
            r.extend_from_slice(&v.to_le_bytes());
        }
        for v in positioning {
            r.extend_from_slice(&v.to_le_bytes());
        }
        for v in anti_air {
            r.extend_from_slice(&v.to_le_bytes());
        }
        r.extend_from_slice(&0u64.to_le_bytes()); // reserved_tail
        assert_eq!(r.len(), d49::ROW_LEN);
        r
    }

    fn live(combat_id: u32, generation: u64, counts: [u32; 2]) -> Vec<u8> {
        row(
            combat_id,
            generation,
            d49::KNOWN_ROW_FLAGS & !d49::ROW_FLAG_DELETED,
            2_757,
            41,
            counts,
            [50_000, 60_000],
            [123_456, -654_321],
        )
    }

    fn tombstone(combat_id: u32, generation: u64) -> Vec<u8> {
        row(
            combat_id,
            generation,
            d49::ROW_FLAG_DELETED,
            0,
            0,
            [0, 0],
            [0, 0],
            [0, 0],
        )
    }

    /// A d49 DATA payload: the 28-byte header, the rows, the empty string table.
    fn d49_payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&d49::SCHEMA_VERSION.to_le_bytes());
        p.extend_from_slice(&(d49::ROW_LEN as u16).to_le_bytes());
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&d49::STRING_COUNT.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(p.len(), d49::HEADER_LEN);
        for r in rows {
            p.extend_from_slice(r);
        }
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    /// Two closed hours: a first patch, then a second patch that accumulates.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(11, 1, [3, 4]), live(12, 1, [5, 6])]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(COMPACT_STATUS_DELTA_DATA, &[live(11, 1, [9, 9])]),
        ));
        s.extend(frame_end(4, 101));
        s
    }

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        scan_stream(stream, &d49::schema(), StreamIdentityMode::Legacy)
    }

    /// A `Read` that hands back at most `chunk` bytes per call, so the tests
    /// exercise genuine positive short reads.
    ///
    /// A `&[u8]` reader fills every buffer it is given in one call, so a scan
    /// driven only by slices would pass even with a one-shot reader that treated
    /// `0 < n < requested` as an error — the defect this wrapper exists to catch.
    /// A real `BufReader` over a 3.6 GB file short-reads at every buffer boundary.
    struct ChunkedReader<'a> {
        data: &'a [u8],
        pos: usize,
        chunk: usize,
    }

    impl Read for ChunkedReader<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.chunk).min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    /// A `Read` that delivers `ok_bytes` and then fails, so the byte position in
    /// an I/O error message is checkable.
    struct FailingReader<'a> {
        data: &'a [u8],
        pos: usize,
        ok_bytes: usize,
    }

    impl Read for FailingReader<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.pos >= self.ok_bytes {
                return Err(std::io::Error::other("injected read failure"));
            }
            let n = buf
                .len()
                .min(self.ok_bytes - self.pos)
                .min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    #[test]
    fn a_chunked_reader_produces_the_same_scan_as_a_whole_slice() {
        // The incremental reader's own contract, swept over chunk sizes that
        // straddle every boundary that matters: inside the 32-byte header, at it,
        // just past it, and across the payload.
        let stream = good_stream();
        let expected = scan(&stream).expect("accepted");
        for chunk in [1usize, 2, 3, 5, 7, 8, 31, 32, 33, 60, 61, 62, 63, 97, 1024] {
            let reader = ChunkedReader {
                data: &stream,
                pos: 0,
                chunk,
            };
            let got = scan_stream(reader, &d49::schema(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("chunk {chunk}: {e}"));
            assert_eq!(got.hours.len(), expected.hours.len(), "chunk {chunk}");
            assert_eq!(got.d49_records, expected.d49_records, "chunk {chunk}");
            assert_eq!(got.excluded.total(), expected.excluded.total(), "chunk {chunk}");
            assert_eq!(got.status_or, expected.status_or, "chunk {chunk}");
            for (a, b) in got.hours.iter().zip(expected.hours.iter()) {
                assert_eq!(a.game_hour, b.game_hour, "chunk {chunk}");
                assert_eq!(a.state, b.state, "chunk {chunk}");
                assert_eq!(a.ship_bytes, b.ship_bytes, "chunk {chunk}");
                assert_eq!(a.upserts, b.upserts, "chunk {chunk}");
                assert_eq!(a.deletes, b.deletes, "chunk {chunk}");
            }
        }
    }

    #[test]
    fn the_record_reader_itself_tells_truncation_from_a_clean_boundary() {
        // `RecordReader` is driven DIRECTLY here, not through `scan_stream`.
        // Going through the scan proves nothing about the reader: a reader that
        // returned `Ok(None)` inside the final record's payload would still make
        // the scan fail, because the hour would be left open at EOF — the scan's
        // error masks the reader's defect. This test asserts on the reader's own
        // terminal result.
        let stream = good_stream();
        // Every record-boundary prefix length, including 0 and the whole stream.
        // A prefix that ends ON one of these is a well-formed shorter stream and
        // must terminate with `Ok(None)`; every other prefix is a torn record and
        // must terminate with an `Err`. Asserting the exact outcome — rather than
        // "an error somewhere" — is what makes this a proof about the reader.
        let boundaries: Vec<usize> = {
            let mut v = vec![0usize];
            let mut pos = 0usize;
            while pos < stream.len() {
                let h = parse_compact_record_header(&stream[pos..]).expect("framed");
                pos += COMPACT_RECORD_HEADER_LEN + h.payload_len;
                v.push(pos);
            }
            v
        };
        let record_count = boundaries.len() - 1;
        assert_eq!(record_count, 4, "two DATA records and two FRAME_ENDs");

        for chunk in [1usize, 7, 31, 32, 33, 1024] {
            // The whole stream: exactly `record_count` records, then a clean
            // `Ok(None)` at the boundary.
            let mut reader = RecordReader::new(ChunkedReader {
                data: &stream,
                pos: 0,
                chunk,
            });
            let mut seen = 0usize;
            loop {
                match reader.next() {
                    Ok(Some(_)) => seen += 1,
                    Ok(None) => break,
                    Err(e) => panic!("chunk {chunk}: unexpected error {e}"),
                }
            }
            assert_eq!(seen, record_count, "chunk {chunk}");

            // EVERY prefix of the stream, one byte at a time — not a handful of
            // sampled cuts.
            for len in 0..stream.len() {
                let short = &stream[..len];
                let mut reader = RecordReader::new(ChunkedReader {
                    data: short,
                    pos: 0,
                    chunk,
                });
                let mut complete = 0usize;
                let terminal = loop {
                    match reader.next() {
                        Ok(Some(_)) => complete += 1,
                        other => break other,
                    }
                };
                let on_boundary = boundaries.contains(&len);
                match terminal {
                    Ok(None) => {
                        assert!(
                            on_boundary,
                            "len {len} chunk {chunk}: a torn record was mistaken for a \
                             clean EOF after {complete} record(s)"
                        );
                        assert_eq!(
                            complete,
                            boundaries.iter().position(|&b| b == len).expect("boundary"),
                            "len {len} chunk {chunk}: wrong number of complete records"
                        );
                    }
                    Err(e) => {
                        assert!(
                            !on_boundary,
                            "len {len} chunk {chunk}: a record-boundary prefix is a \
                             well-formed shorter stream, not an error: {e}"
                        );
                        assert!(
                            e.contains("ends mid-record"),
                            "len {len} chunk {chunk}: wrong error {e}"
                        );
                    }
                    Ok(Some(_)) => unreachable!("the loop breaks on anything but Ok(Some)"),
                }
            }
        }
    }

    /// `StreamScan` is not `Debug`, so `expect_err` is unavailable; this says the
    /// same thing without requiring the success value to be printable.
    fn expect_scan_error<R: Read>(reader: R) -> String {
        match scan_stream(reader, &d49::schema(), StreamIdentityMode::Legacy) {
            Ok(_) => panic!("the injected failure must surface as an Err"),
            Err(e) => e,
        }
    }

    #[test]
    fn an_io_error_names_the_absolute_byte_it_failed_at() {
        // A payload read fails at `record start + 32 + bytes already read`, not at
        // the record start: reporting the latter sends a reader of the message to
        // the wrong record. Checked for a failure inside the FIRST record's
        // payload and inside the SECOND record's header, so both arms are pinned.
        let stream = good_stream();
        // Fail on the first payload byte of record 0, which begins at byte 0.
        let reader = FailingReader {
            data: &stream,
            pos: 0,
            ok_bytes: COMPACT_RECORD_HEADER_LEN,
        };
        let err = expect_scan_error(reader);
        assert!(
            err.contains(&format!("read at byte {COMPACT_RECORD_HEADER_LEN}")),
            "payload read error must name byte {COMPACT_RECORD_HEADER_LEN}, got: {err}"
        );
        // Fail part way into a payload.
        let reader = FailingReader {
            data: &stream,
            pos: 0,
            ok_bytes: COMPACT_RECORD_HEADER_LEN + 5,
        };
        let err = expect_scan_error(reader);
        assert!(
            err.contains(&format!("read at byte {}", COMPACT_RECORD_HEADER_LEN + 5)),
            "payload read error must name byte {}, got: {err}",
            COMPACT_RECORD_HEADER_LEN + 5
        );
        // Fail inside the SECOND record's header. Record 0 is a d49 DATA record;
        // its total length is the header plus its payload.
        let first_len = COMPACT_RECORD_HEADER_LEN
            + parse_compact_record_header(&stream)
                .expect("framed")
                .payload_len;
        let reader = FailingReader {
            data: &stream,
            pos: 0,
            ok_bytes: first_len + 3,
        };
        let err = expect_scan_error(reader);
        assert!(
            err.contains(&format!("read at byte {}", first_len + 3)),
            "header read error must name byte {}, got: {err}",
            first_len + 3
        );
    }

    #[test]
    fn a_well_formed_stream_yields_its_d49_hours_as_accumulated_states() {
        let s = scan(&good_stream()).expect("accepted");
        assert_eq!(s.hours.len(), 2);
        assert_eq!(s.hours[0].game_hour, 100);
        assert_eq!(s.hours[1].game_hour, 101);
        assert_eq!(s.excluded.total(), 0);
        assert_eq!(s.non_contiguous, 0);
        assert_eq!(s.d49_records, 2);
        // The second record carries ONE row and the second hour still holds TWO:
        // the accumulated state, not the record's rows. This is the whole point of
        // a pure cumulative delta, and a harness that treated a record as a
        // snapshot would report 1 here.
        assert_eq!(s.hours[0].state.len(), 2);
        assert_eq!(s.hours[1].state.len(), 2);
        assert_eq!(s.hours[1].upserts, 1);
    }

    #[test]
    fn the_location_badge_is_an_excluded_hour_not_a_stream_rejection() {
        // Finding 1 of `runs/batch3/domain_classification.md`, as an executable
        // requirement: 72 of the capture's 514 hours carry `0xc00` and the shipping
        // certifier calls them "failed exact validation". They must classify as
        // excluded hours here, and the surrounding stream must still be measured.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(11, 1, [3, 4])]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(
                COMPACT_STATUS_DELTA_DATA | d49::STATUS_LOCATION_CANDIDATE_UNVALIDATED,
                &[live(12, 1, [1, 1])],
            ),
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(COMPACT_STATUS_DELTA_DATA, &[]),
        ));
        s.extend(frame_end(6, 102));
        let scanned = scan(&s).expect("the stream is well formed");
        assert_eq!(scanned.excluded.location_status_badge, 1);
        assert_eq!(scanned.excluded.total(), 1);
        assert_eq!(scanned.hours.len(), 2, "the other two hours are measured");
        // The badge hour folded nothing, so combat 12 never entered the table.
        assert_eq!(scanned.hours[1].state.len(), 1);
        // ...and the skipped hour is reported as a contiguity break, not hidden.
        assert_eq!(scanned.non_contiguous, 1);
    }

    #[test]
    fn a_live_row_missing_its_region_is_an_excluded_hour() {
        // The other half of refusal 1, which fires on every one of the capture's
        // 72 bit-11 hours as well, because no row in this capture ever carries a
        // region id at all.
        let flags =
            (d49::KNOWN_ROW_FLAGS & !d49::ROW_FLAG_DELETED) & !d49::ROW_FLAG_REGION_PRESENT;
        let r = row(11, 1, flags, 2_757, 0, [1, 1], [1, 2], [3, 4]);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(COMPACT_STATUS_DELTA_DATA, &[r]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(COMPACT_STATUS_DELTA_DATA, &[live(12, 1, [1, 1])]),
        ));
        s.extend(frame_end(4, 101));
        let scanned = scan(&s).expect("the stream is well formed");
        assert_eq!(scanned.excluded.location_row_incomplete, 1);
        assert_eq!(scanned.hours.len(), 1);
        assert_eq!(scanned.hours[0].state.len(), 1);
    }

    #[test]
    fn a_failure_status_hour_is_excluded_and_the_accumulator_is_retained() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(11, 1, [3, 4])]),
        ));
        s.extend(frame_end(2, 100));
        // PARTIAL_DATA — the capture's one `0x600` hour.
        s.extend(record(
            3,
            101,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(COMPACT_STATUS_DELTA_DATA | (1 << 9), &[]),
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(COMPACT_STATUS_DELTA_DATA, &[]),
        ));
        s.extend(frame_end(6, 102));
        let scanned = scan(&s).expect("accepted");
        assert_eq!(scanned.excluded.failure, 1);
        assert_eq!(scanned.hours.len(), 2);
        assert_eq!(scanned.hours[1].state.len(), 1, "accumulator retained");
    }

    #[test]
    fn the_refusal_positions_around_the_failure_gate_are_reproduced() {
        // Refusal 1 is BEFORE the failure gate and refusal 3 is AFTER it
        // (`world_fold.rs:20047` and `:20076` against `:20066`), so the SAME
        // failure status classifies two payloads differently depending on which
        // gate their rows trip. Copying one domain's order into another was a
        // measured review finding; this is the test that would catch it.
        let mut with_location = Vec::new();
        with_location.extend(record(
            1,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(
                (1 << 9) | d49::STATUS_LOCATION_CANDIDATE_UNVALIDATED,
                &[live(11, 1, [1, 1])],
            ),
        ));
        with_location.extend(frame_end(2, 100));
        with_location.extend(record(
            3,
            101,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(12, 1, [1, 1])]),
        ));
        with_location.extend(frame_end(4, 101));
        let s = scan(&with_location).expect("accepted");
        assert_eq!(s.excluded.location_status_badge, 1, "refusal 1 wins");
        assert_eq!(s.excluded.failure, 0);

        // The same failure status over a row that trips only refusal 3: the fold
        // exits at the failure gate and refusal 3 is never reached.
        let missing_aa = {
            let flags = (d49::KNOWN_ROW_FLAGS & !d49::ROW_FLAG_DELETED)
                & !d49::ROW_FLAG_ATTACKER_ANTI_AIR_PRESENT;
            row(11, 1, flags, 2_757, 41, [1, 1], [5, 6], [0, 8])
        };
        let mut with_side = Vec::new();
        with_side.extend(record(
            1,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(1 << 9, &[missing_aa.clone()]),
        ));
        with_side.extend(frame_end(2, 100));
        with_side.extend(record(
            3,
            101,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(12, 1, [1, 1])]),
        ));
        with_side.extend(frame_end(4, 101));
        let s = scan(&with_side).expect("accepted");
        assert_eq!(s.excluded.failure, 1, "the failure gate wins");
        assert_eq!(s.excluded.side_scalars, 0);

        // And with a clean status word the same payload reaches refusal 3.
        let mut clean = Vec::new();
        clean.extend(record(
            1,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[missing_aa]),
        ));
        clean.extend(frame_end(2, 100));
        clean.extend(record(
            3,
            101,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(12, 1, [1, 1])]),
        ));
        clean.extend(frame_end(4, 101));
        let s = scan(&clean).expect("accepted");
        assert_eq!(s.excluded.side_scalars, 1);
        assert_eq!(s.excluded.failure, 0);
    }

    #[test]
    fn tombstones_fold_and_are_counted_as_shipping_deletes() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(11, 1, [3, 4])]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(COMPACT_STATUS_DELTA_DATA, &[tombstone(11, 1)]),
        ));
        s.extend(frame_end(4, 101));
        let scanned = scan(&s).expect("accepted");
        assert_eq!(scanned.hours[1].deletes, 1);
        assert_eq!(scanned.hours[1].upserts, 0);
        assert!(scanned.hours[1].state.is_empty());
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper classifies
        // the v1 vocabulary and is not a validity predicate, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure" and the hour is measured as healthy.
        for bit in 14..32u32 {
            let status = 1u32 << bit;
            for extra in [
                0u32,
                1,
                COMPACT_STATUS_DELTA_DATA,
                d49::STATUS_LOCATION_CANDIDATE_UNVALIDATED,
            ] {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D49,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d49_payload(status | extra, &[live(11, 1, [1, 1])]),
                ));
                s.extend(frame_end(2, 100));
                assert!(
                    scan(&s).is_err(),
                    "status bit {bit} (+{extra:#x}) must be refused"
                );
            }
        }
    }

    #[test]
    fn a_malformed_payload_refuses_the_stream_whatever_its_status() {
        // The structural half of the independent payload authority runs on every
        // record, so a bad row inside a FAILURE-status record still refuses the
        // stream rather than being skipped with the hour.
        for status in [
            0u32,
            1 << 9,
            COMPACT_STATUS_DELTA_DATA,
            d49::STATUS_LOCATION_CANDIDATE_UNVALIDATED,
        ] {
            // combat_id 0.
            let bad = row(
                0,
                1,
                d49::KNOWN_ROW_FLAGS & !d49::ROW_FLAG_DELETED,
                2_757,
                41,
                [1, 1],
                [1, 2],
                [3, 4],
            );
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D49,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d49_payload(status, &[bad]),
            ));
            s.extend(frame_end(2, 100));
            assert!(scan(&s).is_err(), "status {status:#x}: bad combat_id");

            // A trailing byte after the string table.
            let mut payload = d49_payload(status, &[live(11, 1, [1, 1])]);
            payload.push(0);
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D49,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(scan(&s).is_err(), "status {status:#x}: trailing byte");
        }
    }

    #[test]
    fn two_d49_records_in_one_hour_are_refused_before_any_exclusion() {
        // The uniqueness rule runs over EVERY d49 record. A measured record paired
        // with an excluded one inside one hour must not slip through.
        for second_status in [
            0u32,
            1 << 9,
            d49::STATUS_LOCATION_CANDIDATE_UNVALIDATED,
        ] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D49,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d49_payload(0, &[live(11, 1, [1, 1])]),
            ));
            s.extend(record(
                2,
                100,
                D49,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d49_payload(second_status, &[live(12, 1, [1, 1])]),
            ));
            s.extend(frame_end(3, 100));
            assert!(scan(&s).is_err(), "second status {second_status:#x}");
        }
    }

    #[test]
    fn the_frame_end_seal_and_perf_binding_are_enforced() {
        // One hour with one non-probe PERF record and a seal claiming it.
        let mut good = Vec::new();
        good.extend(perf(1, 100, D49));
        good.extend(record(
            2,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(11, 1, [1, 1])]),
        ));
        good.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan(&good).is_ok());

        // A seal whose counts disagree with the observed PERF domains.
        let mut bad = Vec::new();
        bad.extend(perf(1, 100, D49));
        bad.extend(record(
            2,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(11, 1, [1, 1])]),
        ));
        bad.extend(frame_end_sealed(3, 100, 2, 2, 0, 0));
        assert!(scan(&bad).is_err());

        // emitted != scheduled, a nonzero reserved word, an unknown status bit and
        // a fatal status bit each refuse the seal.
        for (emitted, scheduled, status, reserved) in [
            (1u32, 2u32, 0u32, 0u32),
            (1, 1, 0, 1),
            (1, 1, 1 << 14, 0),
            (1, 1, 1 << 9, 0),
        ] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, D49));
            s.extend(record(
                2,
                100,
                D49,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d49_payload(0, &[live(11, 1, [1, 1])]),
            ));
            s.extend(frame_end_sealed(3, 100, emitted, scheduled, status, reserved));
            assert!(
                scan(&s).is_err(),
                "seal ({emitted},{scheduled},{status:#x},{reserved})"
            );
        }

        // A duplicate non-probe PERF domain inside one hour.
        let mut dup = Vec::new();
        dup.extend(perf(1, 100, D49));
        dup.extend(perf(2, 100, D49));
        dup.extend(record(
            3,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(11, 1, [1, 1])]),
        ));
        dup.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan(&dup).is_err());
    }

    #[test]
    fn a_stream_cut_at_a_frame_end_boundary_is_accepted_and_a_torn_one_is_not() {
        let full = good_stream();
        // Cut after the first FRAME_END: a well-formed shorter stream.
        let mut pos = 0usize;
        for _ in 0..2 {
            let h = parse_compact_record_header(&full[pos..]).expect("framed");
            pos += COMPACT_RECORD_HEADER_LEN + h.payload_len;
        }
        let cut = scan(&full[..pos]).expect("a FRAME_END boundary cut is well formed");
        assert_eq!(cut.hours.len(), 1);
        // Cut one byte short of that boundary: a corrupt stream, not a short one.
        assert!(scan(&full[..pos - 1]).is_err());
        // Cut mid-header.
        assert!(scan(&full[..pos + 5]).is_err());
        // A stream that does not end on a FRAME_END.
        let mut no_seal = Vec::new();
        no_seal.extend(record(
            1,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(11, 1, [1, 1])]),
        ));
        assert!(scan(&no_seal).is_err());
        // ...and an hour left open at EOF is the same rejection.
        no_seal.extend(frame_end(2, 100));
        no_seal.extend(record(
            3,
            101,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d49_payload(0, &[live(12, 1, [1, 1])]),
        ));
        assert!(scan(&no_seal).is_err());
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        let schema = d49::schema();
        let mut state = 0x1234_5678_9ABC_DEF0u64;
        for _ in 0..3_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 512;
            let mut bytes = vec![0u8; len];
            for b in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *b = state as u8;
            }
            let _ = scan_stream(&bytes[..], &schema, StreamIdentityMode::Legacy);
        }
    }
}
