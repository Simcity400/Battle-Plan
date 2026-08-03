//! Measure the wire format against the shipping compact stream on REAL d63
//! `country_core_scalars_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d63-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d63 DATA records are the
//! shipping format's cost for the domain: a 20-byte header plus a fixed 48-byte
//! row per country, full-replace every hour, exactly what the recorder stores
//! today. This tool replays those records into per-hour country states — the
//! same fold `world_fold::fold_country_core_scalars` performs — and re-encodes
//! every hour with this crate's grammar: a BASELINE at each 48-hour block start,
//! field-granular stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** Row and payload decoding
//! go through `wire_format::d63`'s public boundary; H4CR framing, record
//! semantics and sequence continuity go through `compact_contract`'s own
//! parser and stateful validator. A harness that reimplements either drifts from
//! the recorder without anything failing.
//!
//! **The whole stream is validated before any artifact is written.** A stream
//! that is truncated, spliced, or corrupt would otherwise yield a smaller but
//! entirely plausible measurement — a silently wrong number, which is worse than
//! a crash. `scan_stream` therefore enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`
//!   (magic, version, header length, payload bound);
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`
//!   (sequence continuity from the stream's first record, record-type and
//!   record-flag semantics, FRAME_END shape);
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`): a DOMAIN_DATA record opens its
//!   game hour, a FRAME_END closes the open hour and only that hour, and closed
//!   hours strictly increase;
//! - that tracker's **DOMAIN_PERF hour binding** (`:328`-`:365`): no duplicate
//!   non-probe PERF domain inside one hour, no PERF for an hour other than the
//!   open one, no PERF for an hour already closed, and no conflict with the
//!   pending PERF hour before a DATA record opens one;
//! - the **FRAME_END seal fields**, `validate_frame_end_seal_fields`
//!   (`:46`-`:89`), re-implemented here as [`check_frame_end_seal`]: the 32-byte
//!   payload length, `reserved == 0`, `emitted_domains == scheduled_domains`, no
//!   status bit above 13, and no fatal status bit; plus the seal's **domain-count
//!   cross-check** against the distinct non-probe PERF domains actually observed
//!   in the hour (`:411`-`:419`). A seal violation is a `violation` in the
//!   production tracker (`:402`-`:409`), i.e. a refused stream, so it is one here
//!   too;
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. **Exactly one production check is outside this subset, and it
//! is a crate-boundary limitation rather than a choice**: production calls
//! `validate_domain_data_payload_at` on EVERY DOMAIN_DATA record (`:593`),
//! decoding each domain's payload grammar through
//! `recorder_host::domain_data_authority`, so a stream carrying a malformed
//! payload for some *other* domain is refused there and accepted here.
//! `wire_format` cannot depend on `recorder_host` (it pulls in every capture
//! lane) and this crate implements one domain's grammar, so the only DOMAIN_DATA
//! payload this harness can validate is d63's — which it does, fully and
//! fail-closed, on every record whatever its status (see
//! [`check_authority_payload`]). The measured capture is a recorder-validated
//! run, so its other domains' payloads have been through that check; a hand-made
//! stream carrying a corrupt non-d63 payload would measure here and not there.
//! The DATA payloads of every domain other than 63 therefore pass UNVALIDATED
//! beyond header sanity, the stream validator and the hour-boundary rules. That
//! unvalidated remainder is explicitly BOUNDED: every closed game hour must
//! carry exactly one d63 DATA record of any status (the `d63_records ==
//! closed_hours` completeness check in [`scan_stream`]), so a d63 record
//! relabelled to another domain id — or dropped — refuses the run instead of
//! silently shrinking it.
//!
//! The seal fields, the PERF hour binding, the DATA arm's pending-PERF rule, the
//! status-independent payload authority and the stated identity mode arrived
//! together in the five-check sweep, which carried d38's round-1..5 findings
//! (`runs/batch2/d38_air_superiority_inputs/`) back across every earlier bin.
//! None of them moved a measured number on this capture: its 903 FRAME_END
//! records all carry `emitted == scheduled == 20`, `status == 0`, `reserved == 0`,
//! its PERF records show 20 distinct non-probe domains per hour plus the probe
//! domain, it has zero hour-binding violations, and every d63 record it carries
//! is a success record whose rows the fold already validated.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d63.bin   — the d63 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d63;

const D63: u16 = 63;
const BLOCK_HOURS: usize = 48;

/// One usable d63 hour: its game hour, the folded state of that hour, and its
/// byte span in the source stream (the shipping arm is those bytes verbatim,
/// header included).
///
/// The state is built during the scan, not later: every way a row can be
/// rejected — a zero or duplicated `tag_db_id`, an unknown flag bit, an
/// out-of-range value — is a property of the stream, so it belongs to the pass
/// that either accepts the stream or refuses it. Deferring it would leave the
/// harness able to accept a stream and then unwind while measuring it.
struct D63Hour {
    game_hour: i32,
    state: KeyedState,
    ship_span: Range<usize>,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded failure-status hour or a non-contiguous hour step — which is
    /// what forces the encoder to open a fresh baseline (`opens_baseline`).
    follows_previous: bool,
}

struct StreamScan {
    hours: Vec<D63Hour>,
    /// d63 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d63 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences:
/// the 48-hour block cadence (every block opens with its own baseline, §1.5's
/// closing paragraph), and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap
/// affecting the state part forces the next state observation to be a
/// `BASELINE`. State may have moved and returned while nobody was looking, so
/// neither a delta nor an `UNCHANGED` can be justified across unobserved
/// time." Every hour this domain did not successfully sample is such a gap —
/// the excluded failure-status hour, and any non-contiguous hour step — and
/// d63's cadence is hourly, so a measured hour that does not follow the
/// previous measured one by exactly one is preceded by one.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
}

/// Every globally-defined compact status occupies one of bits 0..=13. FRAME_END
/// carries the frame-wide aggregate, so lane-local interpretations do not apply
/// there and any higher bit is an unknown wire-contract extension
/// (`recorder_host/src/stream_fold.rs:43`).
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;

/// The reader-context probe domain, excluded from the seal's domain accounting
/// (`recorder_host/src/stream_fold.rs:330`).
const PROBE_DOMAIN_ID: u16 = 250;

/// The FRAME_END seal check, re-implemented from
/// `recorder_host::stream_fold::validate_frame_end_seal_fields`
/// (`recorder_host/src/stream_fold.rs:46`-`:89`) because `wire_format` must not
/// depend on `recorder_host`.
///
/// Wire layout: `elapsed_wall_ms:u64`, `frame_time_us:u64`, `emitted_domains:u32`,
/// `scheduled_domains:u32`, `status_flags:u32`, `reserved:u32`. The two timing
/// fields carry no seal-time predicate; the rest are authoritative. A violation
/// is a refused stream, exactly as it is in production (`:402`-`:409`) — a
/// corrupt seal means the hour did not close cleanly, and measuring the hours
/// behind it would report a plausible number for a stream the recorder rejects.
///
/// Returns the seal's `(emitted_domains, scheduled_domains)` so the caller can
/// run production's cross-check against the PERF domains it actually observed
/// (`:411`-`:419`).
///
/// Total: no payload of any length panics.
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

/// The d63 checks production's **second** authority makes on every DOMAIN_DATA
/// record — status-independent, and therefore also on the records this harness
/// excludes.
///
/// Production runs two independent passes over a d63 DATA record and a stream is
/// refused if EITHER complains:
///
/// 1. `WorldFold::ingest_record` → `fold_country_core_scalars`, whose `Err`
///    aborts the stream fold. Its header checks run before the status early exit
///    and are what `d63::split_payload` mirrors; its row loop runs after it, so
///    an excluded record never reaches the rows there.
/// 2. `validate_domain_data_payload_at` (`recorder_host/src/stream_fold.rs:594`)
///    → `decode_compact_country_core_scalars_data_payload`
///    (`domain_data_authority.rs:11969`-`:12081`). This one runs on EVERY
///    DOMAIN_DATA record whatever its status, and any error it reports makes the
///    record `malformed_payload_records`, which
///    `strict_stream_certification_verdict` (`stream_fold.rs:507`-`:510`)
///    refuses.
///
/// **Unlike d38's, this domain's second authority DOES walk the rows**
/// (`:12000`-`:12064`): zero `tag_db_id`, unknown row flags, each optional u32
/// against its maximum and its zero-pack rule, and the seven experience channels
/// zero-packed when the experience flag is clear. Every one of those is
/// `d63::key_from_row`/`d63::record_from_row`'s own check, so this function
/// replays the domain module's boundary rather than re-deriving the predicates —
/// and it runs it on every record, not only the folded ones.
///
/// The header predicates (minimum length, exact length, `changed_rows ==
/// row_count`) are already `d63::split_payload`'s and already run before the
/// exclusion, so they are not repeated here.
///
/// **Where this deliberately stops:** the authority does NOT check
/// `tag_db_id` uniqueness within a frame. That rule is the fold's alone
/// (`world_fold.rs:9363`) and the fold returns before its row loop on an excluded
/// record, so enforcing it here would be stricter than production — which is its
/// own defect. It therefore stays where the fold puts it, on folded records only.
fn check_authority_payload(
    collection: &wire_format::Collection,
    body: &[u8],
    at_byte: usize,
) -> Result<(), String> {
    for row in body.chunks_exact(d63::ROW_LEN) {
        d63::key_from_row(row).map_err(|e| format!("d63 row at byte {at_byte}: {e}"))?;
        d63::record_from_row(collection, row, 0)
            .map_err(|e| format!("d63 row at byte {at_byte}: {e}"))?;
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold the usable
/// d63 DATA records into per-hour states. Every rejection path is an `Err`,
/// never a silently shorter measurement and never a panic — see the module docs
/// for what is enforced and what the recorder's own validator additionally
/// checks.
fn scan_stream(
    stream: &[u8],
    collection: &wire_format::Collection,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // The identity protocol is an INPUT, never sniffed. `StatefulStreamValidator`
    // deliberately never sniffs (`compact_contract/src/stream_validator.rs:328`)
    // and production takes the mode from its caller
    // (`recorder_host/src/stream_fold.rs:220`), for a reason this harness has to
    // honour: an `IdentityV1` stream whose mandatory CID1 record has been spliced
    // off is byte-indistinguishable from a legacy stream — both begin with an
    // ordinary record at sequence 1. A harness that sniffs therefore accepts a
    // decapitated identity stream as a healthy legacy one and measures it, which
    // production (invoked with `IdentityV1`) refuses as `WrongRecordSequence(1)`.
    // The mode is a typed parameter, so the caller states which protocol the
    // capture is and a mismatch fails closed in both directions — a legacy call on
    // an identity stream trips on the CID1 record, an identity call on a legacy
    // stream trips on the missing one.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D63Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;
    // COMPLETENESS accounting: every d63 DATA record of ANY status, against
    // every closed game hour — see the check after the EOF rules.
    let mut d63_records = 0usize;
    let mut closed_hours = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // The DOMAIN_PERF hour-binding state, `BoundaryTracker`'s own
    // (`stream_fold.rs:328`-`:365`, reset at `:449`-`:451`).
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d63 DATA record of ANY status — the uniqueness rule.
    let mut last_d63_hour: Option<i32> = None;
    // Hour of the last d63 record that entered the measurement — the
    // contiguity report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|e| format!("record at byte {pos}: {e}"))?;
        let payload_start = pos + COMPACT_RECORD_HEADER_LEN;
        let record_end = payload_start
            .checked_add(header.payload_len)
            .ok_or_else(|| format!("record at byte {pos}: length overflow"))?;
        if record_end > stream.len() {
            // §`eof_check`: a stream ending mid-record is not a short stream, it
            // is a corrupt one.
            return Err(format!(
                "stream ends mid-record at byte {pos}: {} byte(s) present, record needs {}",
                stream.len() - pos,
                record_end - pos
            ));
        }
        let payload = &stream[payload_start..record_end];

        validator
            .validate_record(&header, payload)
            .map_err(|e| format!("record at byte {pos}: {e:?}"))?;

        // The hour-boundary subset of `BoundaryTracker::observe`.
        match header.record_type {
            // `stream_fold.rs:328`-`:365`. PERF records carry no d63 data, but
            // they bind the hour: a duplicate non-probe domain, a PERF for an
            // hour other than the open one, a PERF for an already-closed hour, or
            // two different pending hours before any DATA record opens one are
            // each a refused stream in production, so each is one here.
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
                    // `stream_fold.rs:378`-`:386`, and it comes BEFORE the
                    // monotonicity check, as it does there: a DATA record may only
                    // open the hour the pending PERF records already bound. The
                    // PERF arm alone does not cover this, because a single pending
                    // PERF hour is legal until a DATA record disagrees with it.
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
                // The seal fields come first, as they do in the production tracker
                // (`stream_fold.rs:401`-`:409`): a FRAME_END that does not seal is
                // a refused stream whatever hour it claims to close.
                let (emitted_domains, scheduled_domains) = check_frame_end_seal(payload, pos)?;
                // `:411`-`:419`: the seal's counts must equal the distinct
                // non-probe PERF domains actually observed in the hour it closes.
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
                // `stream_fold.rs:449`-`:451`.
                open_hour = None;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
                last_closed_hour = Some(header.game_hour_raw);
                closed_hours += 1;
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D63 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d63_records += 1;
            let (d63_header, body) = d63::split_payload(payload)
                .map_err(|e| format!("d63 payload at byte {pos}: {e}"))?;

            // One d63 DATA record per game hour, checked over EVERY d63 record
            // and BEFORE any status-based exclusion. d63 is full-replace, so a
            // second record for the same hour would become a second measurement
            // hour built from the same clock time — and gating this on the
            // record being usable would let a success/failure pair through the
            // rule entirely.
            if let Some(previous) = last_d63_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d63 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d63_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. The failure predicate below reads
            // the status word to classify the v1 vocabulary; it is not a validity
            // predicate. `compact_contract/src/lib.rs:121`-`:123` says so in as
            // many words: "Callers handling untrusted status words must first
            // reject a non-zero `compact_status_unknown_flags_v1` result;
            // otherwise an unassigned bit has no defined classification." Bits
            // 14..=31 are unassigned (`status_flags_base.rs:21`-`:24`), and an
            // unassigned bit "is not evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not
            // a failure" — the failure helper masks with the *known* failure set
            // and returns zero for a bit it does not know — and the hour is
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness does the same rather than re-deriving a lenient rule.
            // No capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d63_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d63 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // Production's SECOND authority, which runs on every DOMAIN_DATA
            // record whatever its status and whose complaint refuses the stream at
            // certification. This must come BEFORE the exclusion, or a record the
            // recorder would refuse gets quietly skipped instead. For d63 that
            // authority walks the rows, so the excluded record's rows are checked
            // here too — the fold's own row loop below never sees them.
            check_authority_payload(collection, body, pos)?;

            // `world_fold::handle_compact_failure` (`world_fold.rs:2843`, called
            // at `:9252`) marks the domain-hour unknown and RETAINS the prior
            // accumulator on a failure status, folding nothing from this record.
            // The predicate is `compact_contract`'s own, not a copied bit mask:
            // it already applies d63's lane-local mask (none) and expected
            // partial tolerance (none), so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D63,
                d63_header.status_flags,
            ) != 0
            {
                failure_records += 1;
            } else {
                let follows_previous =
                    last_measured_hour.is_some_and(|previous| header.game_hour_raw == previous + 1);
                if last_measured_hour.is_some() && !follows_previous {
                    non_contiguous += 1;
                }
                last_measured_hour = Some(header.game_hour_raw);

                // Fold exactly as `fold_country_core_scalars` does. Every row
                // the fold rejects is rejected here, at the same point in the
                // pass — including a duplicated `tag_db_id` (`:9363`), which is
                // a rejected frame and never a last-writer-wins overwrite.
                let mut rows = 0usize;
                let mut state: KeyedState = BTreeMap::new();
                for row in body.chunks_exact(d63::ROW_LEN) {
                    rows += 1;
                    let key = d63::key_from_row(row)
                        .map_err(|e| format!("d63 row at byte {pos}: {e}"))?;
                    let record = d63::record_from_row(collection, row, 0)
                        .map_err(|e| format!("d63 row at byte {pos}: {e}"))?;
                    if state.insert(key, record).is_some() {
                        return Err(format!(
                            "d63 payload at byte {pos}: duplicate tag_db_id in one frame"
                        ));
                    }
                }
                if rows != d63_header.row_count as usize {
                    return Err(format!("d63 payload at byte {pos}: row count mismatch"));
                }
                hours.push(D63Hour {
                    game_hour: header.game_hour_raw,
                    state,
                    ship_span: pos..record_end,
                    follows_previous,
                });
            }
        }

        pos = record_end;
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
        return Err("stream carries no usable d63 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d63 DATA
    // record, of any status. This bounds what this harness canNOT validate:
    // production runs `validate_domain_data_payload_at` on EVERY domain's DATA
    // payload (`stream_fold.rs:593`), while this harness owns only d63's
    // grammar, so a d63 record whose header was relabelled to another domain
    // id — or lost outright — would otherwise be silently ignored and the run
    // would report a plausible measurement over fewer hours. A per-capture
    // completeness assertion, NOT a grammar rule: a capture where d63 is not
    // scheduled every hour fails here loudly and a human adjusts, instead of a
    // number quietly shrinking (`bench_real_d65.rs` precedent).
    if d63_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d63_records} d63 DATA \
             record(s); every closed hour of a d63-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        non_contiguous,
    })
}

/// The wire arm — BASELINE at each block start and at every §1.5 gap,
/// stored-minimal DELTA between — with the per-hour reader replay.
///
/// Factored out of `main` so the tests execute the PRODUCTION encode path over
/// a scanned stream: a mutant that reverts the baseline decision at this
/// call chain to block-position-only fails the gap test (a Codex round-1
/// finding — the earlier tests exercised the decision function but not this
/// callsite).
struct WireMeasurement {
    wire: Vec<u8>,
    wire_offsets: Vec<usize>,
    hours_seen: usize,
    baselines: usize,
    gap_forced_baselines: usize,
    empty_hours: usize,
    changed_rows_total: usize,
    tombstones_total: usize,
    countries_total: usize,
}

fn measure_wire_arm(
    schema: &wire_format::Schema,
    collection: &wire_format::Collection,
    hours: &[D63Hour],
) -> WireMeasurement {
    let mut m = WireMeasurement {
        wire: Vec::new(),
        wire_offsets: vec![0usize],
        hours_seen: 0,
        baselines: 0,
        gap_forced_baselines: 0,
        empty_hours: 0,
        changed_rows_total: 0,
        tombstones_total: 0,
        countries_total: 0,
    };
    let mut prior: Option<&KeyedState> = None;
    let mut reader: Option<KeyedState> = None;

    for hour in hours {
        // The hour's state, folded during the scan: d63 is full-replace, so the
        // hour's rows ARE the hour's whole state.
        let state: &KeyedState = &hour.state;
        m.countries_total += state.len();

        // New arm: BASELINE at each block start — and at every §1.5 gap, which
        // forces a fresh baseline and restarts the delta chain (`opens_baseline`).
        let hour_start = m.wire.len();
        let block_start = opens_baseline(m.hours_seen, hour.follows_previous);
        if block_start && m.hours_seen % BLOCK_HOURS != 0 {
            m.gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if block_start || prior.is_none() {
            encode_baseline(schema, std::slice::from_ref(state), &mut m.wire)
                .expect("baseline encodes");
            m.baselines += 1;
        } else {
            let prev = prior.unwrap();
            let d = diff(collection, prev, state);
            if d.is_empty() {
                m.empty_hours += 1;
                emitted = false;
            } else {
                m.changed_rows_total += d.changed.len();
                m.tombstones_total += d.tombstones.len();
                encode_delta(schema, std::slice::from_ref(prev), &[Some(d)], &mut m.wire)
                    .expect("delta encodes");
            }
        }
        m.wire_offsets.push(m.wire.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require
        // the result to be stored-equal to this hour's state. This is what makes
        // the size claim a claim about decodable bytes. An hour that emitted
        // nothing is still checked: the reader's retained state must already
        // equal the fold's, which is precisely what "nothing changed" asserts.
        if emitted {
            let hour_bytes = &m.wire[hour_start..];
            if block_start || reader.is_none() {
                let (states, consumed) =
                    decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                reader = Some(states.into_iter().next().unwrap());
            } else {
                let reader_prior = reader.take().unwrap();
                let (deltas, consumed) =
                    decode_delta(schema, std::slice::from_ref(&reader_prior), hour_bytes, 0)
                        .expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                let mut next = reader_prior;
                apply(&mut next, deltas[0].as_ref().unwrap());
                reader = Some(next);
            }
        }
        let replayed = reader.as_ref().unwrap();
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
        m.hours_seen += 1;
    }
    m
}

fn main() {
    const USAGE: &str = "usage: bench_real_d63 <collector_stream.bin> <out dir> \
                         [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    // The identity protocol is stated, never sniffed — see `scan_stream`. The
    // default is `legacy`, which is what every capture measured so far is
    // (`runs/batch1/deepwar_t1`, `runs/batch2/deepwar_t1`: both open with an
    // ordinary record at sequence 1, no CID1 header).
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d63::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d63 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result. Everything that
    // can fail on stream content fails here, with a diagnosis and no unwind.
    let scan = match scan_stream(&stream, collection, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads stream content again,
    // so every assertion below is about the codec, not about the input. -------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
    }

    let m = measure_wire_arm(&schema, collection, &scan.hours);
    let wire = m.wire;
    let wire_offsets = m.wire_offsets;
    let hours_seen = m.hours_seen;
    let baselines = m.baselines;
    let gap_forced_baselines = m.gap_forced_baselines;
    let empty_hours = m.empty_hours;
    let changed_rows_total = m.changed_rows_total;
    let tombstones_total = m.tombstones_total;
    let countries_total = m.countries_total;
    for (name, bytes) in [("ship_d63.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d63.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        scan.non_contiguous
    );
    println!(
        "failure-status hours {} (excluded from both arms)",
        scan.failure_records
    );
    println!(
        "countries avg        {:.1}",
        countries_total as f64 / hours_seen as f64
    );
    println!(
        "baselines            {baselines} ({} at a {BLOCK_HOURS}-h block start, \
         {gap_forced_baselines} forced by a WIRE_FORMAT \u{a7}1.5 state gap)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d63.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

    fn test_collection() -> wire_format::Collection {
        d63::schema().collections[0].clone()
    }

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

    /// A one-country d63 DATA payload: 20-byte header then one 48-byte row.
    fn d63_payload(status_flags: u32, tag_db_id: u32, manpower: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&1u32.to_le_bytes()); // row_count
        payload.extend_from_slice(&1u32.to_le_bytes()); // changed_rows
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&tag_db_id.to_le_bytes());
        payload.extend_from_slice(&d63::FLAG_MANPOWER_POOL.to_le_bytes());
        payload.extend_from_slice(&manpower.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // convoys_destroyed
        payload.extend_from_slice(&0u32.to_le_bytes()); // divisions_lost
        payload.extend_from_slice(&[0u8; 28]); // seven zero-packed channels
        assert_eq!(payload.len(), d63::HEADER_LEN + d63::ROW_LEN);
        payload
    }

    /// A d63 DATA payload with no country rows: header only.
    fn d63_payload_rowless(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&0u32.to_le_bytes()); // row_count
        payload.extend_from_slice(&0u32.to_le_bytes()); // changed_rows
        payload.extend_from_slice(&status_flags.to_le_bytes());
        assert_eq!(payload.len(), d63::HEADER_LEN);
        payload
    }

    /// Two closed hours, each carrying one d63 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 11),
        ));
        s.extend(frame_end(4, 101));
        s
    }

    /// The byte offset of the Nth record boundary in a stream of framed records.
    fn boundary(stream: &[u8], records: usize) -> usize {
        let mut pos = 0usize;
        for _ in 0..records {
            let h = parse_compact_record_header(&stream[pos..]).expect("framed");
            pos += COMPACT_RECORD_HEADER_LEN + h.payload_len;
        }
        pos
    }

    #[test]
    fn a_well_formed_stream_yields_its_d63_hours() {
        let scan = scan_stream(
            &good_stream(),
            &test_collection(),
            StreamIdentityMode::Legacy,
        )
        .expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.hours[0].state.len(), 1);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper classifies
        // the v1 vocabulary and is not a validity predicate, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure" and the hour is measured as healthy.
        for bit in 15..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with a known
            // informational bit; with a row and without. The rejection must not
            // depend on any of it.
            for extra in [0u32, 1, 1 << 10] {
                for (label, payload) in [
                    ("rowless", d63_payload_rowless(status | extra)),
                    ("one row", d63_payload(status | extra, 7, 10)),
                ] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D63,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &payload,
                    ));
                    s.extend(frame_end(2, 100));
                    assert!(
                        scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                        "status bit {bit} (+{extra:#x}), {label}, must be refused"
                    );
                }
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else.
        // `fold_country_core_scalars` never reads an informational bit at all.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(informational, 7, 10),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // The reviewer's input, generalized: a cut immediately after a d63 DATA
        // record leaves that hour open and ends the stream on a record that is
        // not a FRAME_END. That used to produce a plausible shorter number; it
        // must now be refused. Records 1 and 3 of `good_stream` are the DATA
        // records.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary is a different thing: that IS a
        // well-formed shorter stream, and `eof_check` accepts it too. Refusing it
        // would be wrong. What matters is that it reports the hours it actually
        // closed, not the hours the untruncated file had.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy)
                .expect("closed stream is well-formed");
            assert_eq!(scan.hours.len(), expected_hours, "cut after {records}");
        }
    }

    #[test]
    fn a_stream_ending_mid_record_is_rejected() {
        let full = good_stream();
        // Every cut that is not a record boundary, across the whole stream.
        let boundaries: Vec<usize> = (0..=4).map(|n| boundary(&full, n)).collect();
        for cut in 1..full.len() {
            if boundaries.contains(&cut) {
                continue;
            }
            assert!(
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "mid-record cut at {cut}"
            );
        }
        assert!(
            scan_stream(&[], &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "an empty stream is not a measurement"
        );
    }

    #[test]
    fn a_header_the_canonical_parser_rejects_is_rejected_here() {
        // Version, magic and header length, at every record in the stream —
        // the canonical parser owns these and the harness must not read past it.
        for records in 0..4 {
            let at = boundary(&good_stream(), records);
            for (offset, patch) in [
                (0usize, vec![b'X']),              // magic
                (4, 2u16.to_le_bytes().to_vec()),  // unsupported version
                (6, 33u16.to_le_bytes().to_vec()), // unsupported header length
            ] {
                let mut s = good_stream();
                s[at + offset..at + offset + patch.len()].copy_from_slice(&patch);
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "record {records} patched at +{offset} must be refused"
                );
            }
        }
    }

    #[test]
    fn a_sequence_discontinuity_is_rejected() {
        // A swallowed or reordered record changes which hours the measurement
        // sees, so the sequence chain is load-bearing, not decoration.
        for records in 0..4 {
            let at = boundary(&good_stream(), records);
            for wrong in [0u64, 9] {
                let mut s = good_stream();
                s[at + 8..at + 16].copy_from_slice(&wrong.to_le_bytes());
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "record {records} sequence {wrong}"
                );
            }
        }
    }

    #[test]
    fn a_frame_end_that_does_not_close_its_open_hour_is_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 10),
        ));
        s.extend(frame_end(2, 999)); // closes an hour that is not open
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        // A FRAME_END with nothing open at all.
        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        // A closed hour that repeats or goes backwards would fold two different
        // clock times into one measurement step.
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 10),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 11),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d63_record_in_one_hour_is_rejected() {
        // d63 is full-replace; two records in one hour would become two
        // measurement hours built from the same clock time. The rule is checked
        // over EVERY d63 DATA record and before status-based exclusion, so all
        // four success/failure orderings are refused — gating it on the record
        // being usable let a success + failure pair through, which is the
        // reviewer's reported input.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(first, 7, 10),
            ));
            s.extend(record(
                2,
                100,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(second, 7, 11),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "two d63 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time:
        // the fold retains the accumulator and folds nothing, so the hour is not
        // d63 data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(1 << bit, 7, 10),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(0, 7, 11),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
        }
        // And a non-failure bit (DELTA_DATA, 1<<10) is NOT treated as failure.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(1 << 10, 7, 10),
        ));
        s.extend(frame_end(2, 100));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." Both gap shapes d63
        // can meet — an excluded failure-status hour, and a whole hour the
        // capture never scheduled — must clear `follows_previous`, and
        // `opens_baseline` must open a baseline on it wherever it lands in a
        // block. A delta would relate two states across unobserved time.
        //
        // Shape 1: hour 101 carries a failure-status d63 record (excluded).
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(1 << 0, 7, 11), // failure status -> excluded
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 12),
        ));
        s.extend(frame_end(6, 102));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.non_contiguous, 1);
        assert!(!scan.hours[0].follows_previous, "the first hour follows none");
        assert!(
            !scan.hours[1].follows_previous,
            "hour 102 does not follow hour 100"
        );
        // ...and that is what makes the encoder open a baseline for it, even
        // though it is only the second measured hour of the block.
        assert!(opens_baseline(1, scan.hours[1].follows_previous));
        // Through the PRODUCTION encode path, not just the decision function:
        // the second measured hour must be a gap-forced baseline, not a delta.
        let m = measure_wire_arm(&d63::schema(), &test_collection(), &scan.hours);
        assert_eq!(m.hours_seen, 2);
        assert_eq!(m.baselines, 2, "hour 100 opens the block; hour 102 is gap-forced");
        assert_eq!(m.gap_forced_baselines, 1);
        assert_eq!(m.empty_hours, 0);
        // The gap hour's bytes decode as a BASELINE on their own.
        let (states, consumed) = decode_baseline(
            &d63::schema(),
            &m.wire[m.wire_offsets[1]..m.wire_offsets[2]],
            0,
        )
        .expect("the post-gap hour decodes as a baseline");
        assert_eq!(consumed, m.wire_offsets[2] - m.wire_offsets[1]);
        assert_eq!(states[0].len(), 1);

        // Shape 2: hour 101 is never scheduled at all — the capture closes
        // hours 100 and 102 only.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            102,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 12),
        ));
        s.extend(frame_end(4, 102));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.non_contiguous, 1);
        assert!(!scan.hours[1].follows_previous);
        assert!(opens_baseline(1, scan.hours[1].follows_previous));

        // The decision function over its whole input space: a block boundary
        // always opens one, and so does any gap, at every position in a block.
        for hours_seen in 0..(2 * BLOCK_HOURS) {
            assert!(
                opens_baseline(hours_seen, false),
                "a gap at position {hours_seen} must open a baseline"
            );
            assert_eq!(
                opens_baseline(hours_seen, true),
                hours_seen % BLOCK_HOURS == 0,
                "contiguous position {hours_seen}"
            );
        }

        // A contiguous run is unaffected: only the block boundary baselines —
        // through the production encode path too.
        let scan = scan_stream(
            &good_stream(),
            &test_collection(),
            StreamIdentityMode::Legacy,
        )
        .expect("well-formed");
        assert_eq!(scan.non_contiguous, 0);
        assert!(scan.hours[1].follows_previous);
        assert!(!opens_baseline(1, scan.hours[1].follows_previous));
        let m = measure_wire_arm(&d63::schema(), &test_collection(), &scan.hours);
        assert_eq!(m.baselines, 1, "only the block boundary baselines");
        assert_eq!(m.gap_forced_baselines, 0);
    }

    #[test]
    fn a_closed_hour_without_a_d63_record_refuses_the_stream() {
        // The cross-domain bound: this harness validates only d63 payloads, so
        // a d63 record relabelled to another domain — or dropped — would
        // otherwise be silently ignored and the run would report a plausible
        // measurement over fewer hours. Hour 100 carries a d63 record; hour
        // 101 carries only another domain's DATA record. Every hour is
        // structurally well formed — a DOMAIN_DATA record opens it and a
        // FRAME_END closes it — so only the completeness check can see it.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(4, 101));
        let err = match scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d63 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );
    }

    #[test]
    fn a_stream_with_no_d63_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_malformed_d63_payload_is_refused_not_skipped() {
        // A short or inconsistent d63 payload is a corrupt stream, not an hour
        // to drop quietly.
        let mut payload = d63_payload(0, 7, 10);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        // A row the FOLD rejects must make the SCAN fail, not be accepted and
        // then unwind while measuring. The reviewer's input was a structurally
        // sized payload whose row is invalid; the whole row-rejection space is
        // covered here, not just that one case.
        let mut invalid_rows: Vec<Vec<u8>> = Vec::new();
        // zero tag_db_id (`world_fold.rs:9259`)
        invalid_rows.push(d63_payload(0, 0, 10));
        // an unknown flag bit (`:9263`)
        let mut unknown_flag = d63_payload(0, 7, 10);
        unknown_flag[d63::HEADER_LEN + 4..d63::HEADER_LEN + 8]
            .copy_from_slice(&(1u32 << 20).to_le_bytes());
        invalid_rows.push(unknown_flag);
        // an out-of-range present value (`:9286`)
        invalid_rows.push(d63_payload(0, 7, d63::MAX_MANPOWER_POOL + 1));
        // a gated-off field carrying a non-zero word (`:30112`)
        let mut ungated = d63_payload(0, 7, 10);
        ungated[d63::HEADER_LEN + 4..d63::HEADER_LEN + 8].copy_from_slice(&0u32.to_le_bytes());
        invalid_rows.push(ungated);
        for (i, payload) in invalid_rows.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "invalid row {i} must make the scan fail"
            );
        }

        // A tag_db_id repeated within ONE frame is a rejected frame
        // (`world_fold.rs:9363`), never a last-writer-wins overwrite — and it
        // must be caught in the scan, not asserted in the measuring pass.
        let mut payload = d63_payload(0, 7, 10);
        let row: Vec<u8> = payload[d63::HEADER_LEN..].to_vec();
        payload.extend_from_slice(&row);
        payload[8..12].copy_from_slice(&2u32.to_le_bytes()); // row_count
        payload[12..16].copy_from_slice(&2u32.to_le_bytes()); // changed_rows
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "duplicate tag_db_id in one frame"
        );
    }

    /// One DOMAIN_PERF record with the minimal valid payload
    /// (`COMPACT_DOMAIN_PERF_BASE_LEN`).
    fn perf(sequence: u64, game_hour: i32, domain: u16) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        )
    }

    /// One FRAME_END with explicit seal fields, so a test can perturb exactly one.
    fn frame_end_sealed(
        sequence: u64,
        game_hour: i32,
        emitted: u32,
        scheduled: u32,
        status: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::with_capacity(COMPACT_FRAME_END_PAYLOAD_LEN);
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&0u64.to_le_bytes()); // frame_time_us
        payload.extend_from_slice(&emitted.to_le_bytes());
        payload.extend_from_slice(&scheduled.to_le_bytes());
        payload.extend_from_slice(&status.to_le_bytes());
        payload.extend_from_slice(&reserved.to_le_bytes());
        assert_eq!(payload.len(), COMPACT_FRAME_END_PAYLOAD_LEN);
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        // The seal predicate of `validate_frame_end_seal_fields`
        // (`stream_fold.rs:46`-`:89`), swept over the whole predicate rather than
        // one input. Before this check a FRAME_END with a nonzero reserved word —
        // a stream production refuses — produced an unchanged, plausible
        // measurement here.
        //
        // A well-formed seal, for contrast. These streams carry no DOMAIN_PERF
        // records, so the seal's domain counts must be 0 to match the observed
        // non-probe PERF domains; `perf_hour_binding_is_enforced` covers the
        // cross-check with real PERF records. (The capture's own shape is
        // `emitted == scheduled == 20` against 20 observed PERF domains.)
        let good = |seq, hour| frame_end_sealed(seq, hour, 0, 0, 0, 0);
        let mut ok = Vec::new();
        ok.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 10),
        ));
        ok.extend(good(2, 100));
        assert_eq!(
            scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy)
                .expect("a sealed stream is accepted")
                .hours
                .len(),
            1
        );

        // Every way the seal can fail, each on its own.
        let mut bad_seals: Vec<(String, Vec<u8>)> = vec![
            (
                "reserved nonzero".to_string(),
                frame_end_sealed(2, 100, 0, 0, 0, 1),
            ),
            (
                "emitted != scheduled".to_string(),
                frame_end_sealed(2, 100, 0, 1, 0, 0),
            ),
        ];
        // Every unassigned status bit, 14..=31.
        for bit in 15..32u32 {
            bad_seals.push((
                format!("unknown seal status bit {bit}"),
                frame_end_sealed(2, 100, 0, 0, 1 << bit, 0),
            ));
        }
        // Every fatal status bit in COMPACT_STATUS_FAILURE_MASK.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad_seals.push((
                format!("fatal seal status bit {bit}"),
                frame_end_sealed(2, 100, 0, 0, 1 << bit, 0),
            ));
        }
        for (label, sealed) in bad_seals {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(0, 7, 10),
            ));
            s.extend(sealed);
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "{label} must refuse the stream, not yield a measurement"
            );
        }

        // A seal payload of the wrong length is refused at every length, not only
        // the short ones — and never panics.
        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(0, 7, 10),
            ));
            s.extend(record(
                2,
                100,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0u8; len],
            ));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a {len}-byte FRAME_END payload is not a seal"
            );
        }

        // A non-fatal informational seal status is still accepted — the check must
        // reject the unknown and the fatal, and nothing else.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(0, 7, 10),
            ));
            s.extend(frame_end_sealed(2, 100, 0, 0, informational, 0));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_ok(),
                "informational seal status {informational:#x} must be accepted"
            );
        }

        // And a seal violation on a LATER frame refuses the whole stream rather
        // than silently truncating the measurement to the hours before it.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 10),
        ));
        s.extend(good(2, 100));
        s.extend(record(
            3,
            101,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 11),
        ));
        s.extend(frame_end_sealed(4, 101, 0, 0, 0, 1));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        // DOMAIN_PERF records carry no d63 data, but they bind the hour, and
        // production refuses a stream that violates that binding
        // (`stream_fold.rs:328`-`:365`). Swept over the whole predicate.
        //
        // The shape of a healthy hour: PERF for each scheduled domain, the d63
        // DATA record, then a FRAME_END whose seal counts equal the observed
        // non-probe PERF domains. The capture's own shape, at 2 domains instead
        // of 20.
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(0, 7, 10),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D63));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID)); // the probe is not counted
        ok.extend(data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert_eq!(
            scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy)
                .expect("a PERF-bound hour is accepted")
                .hours
                .len(),
            1
        );
        // The probe domain may repeat without counting as a duplicate.
        let mut probe_twice = Vec::new();
        probe_twice.extend(perf(1, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(data(3, 100));
        probe_twice.extend(frame_end_sealed(4, 100, 0, 0, 0, 0));
        assert!(scan_stream(&probe_twice, &test_collection(), StreamIdentityMode::Legacy).is_ok());

        // Duplicate non-probe PERF domain inside one hour.
        let mut dup = Vec::new();
        dup.extend(perf(1, 100, 20));
        dup.extend(perf(2, 100, 20));
        dup.extend(data(3, 100));
        dup.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(
            scan_stream(&dup, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "duplicate PERF domain"
        );

        // A PERF for an hour other than the open one.
        let mut interleaved = Vec::new();
        interleaved.extend(data(1, 100));
        interleaved.extend(perf(2, 999, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(
            scan_stream(&interleaved, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "PERF for an hour other than the open one"
        );

        // A PERF for an hour already closed.
        let mut stale = Vec::new();
        stale.extend(data(1, 100));
        stale.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        stale.extend(perf(3, 100, 20));
        stale.extend(data(4, 101));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        assert!(
            scan_stream(&stale, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "non-monotonic PERF"
        );

        // Two different pending PERF hours before any DATA record opens one.
        let mut conflict = Vec::new();
        conflict.extend(perf(1, 100, 20));
        conflict.extend(perf(2, 101, 21));
        conflict.extend(data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(
            scan_stream(&conflict, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "PERF hour conflict"
        );

        // And the seal's domain counts must equal the observed non-probe PERF
        // domains (`stream_fold.rs:411`-`:419`), in both directions.
        for (emitted, scheduled) in [(1u32, 1u32), (3, 3), (2, 3), (3, 2)] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, 20));
            s.extend(perf(2, 100, D63));
            s.extend(data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "seal claiming emitted={emitted} scheduled={scheduled} against 2 observed \
                 PERF domains must be refused"
            );
        }

        // The PERF domain set resets at each FRAME_END: the same domain may appear
        // again in the next hour.
        let mut two_hours = Vec::new();
        two_hours.extend(perf(1, 100, 20));
        two_hours.extend(data(2, 100));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 20));
        two_hours.extend(record(
            5,
            101,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 11),
        ));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&two_hours, &test_collection(), StreamIdentityMode::Legacy)
                .expect("the PERF domain set resets per hour")
                .hours
                .len(),
            2
        );
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        // `stream_fold.rs:378`-`:386`. The PERF arm alone does not cover this,
        // because one pending PERF hour is legal until a DATA record disagrees
        // with it — this reproducer passes with the PERF arm present and the DATA
        // arm's `pending_perf_hour` consultation missing.
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(0, 7, 10),
            )
        };
        let mut s = Vec::new();
        s.extend(perf(1, 100, 20));
        s.extend(data(2, 101));
        s.extend(frame_end_sealed(3, 101, 1, 1, 0, 0));
        assert!(
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "DATA hour conflict against the pending PERF hour"
        );
        // Both directions of the disagreement, and with the DATA hour on either
        // side of the pending one, so the rule is not accidentally a monotonicity
        // check.
        for (perf_hour, data_hour) in [(100i32, 101i32), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "pending PERF hour {perf_hour} then DATA hour {data_hour}"
            );
        }
        // ...and the agreeing case is still accepted.
        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, 100, 20));
        agreeing.extend(data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&agreeing, &test_collection(), StreamIdentityMode::Legacy)
                .expect("a DATA record matching the pending PERF hour is accepted")
                .hours
                .len(),
            1
        );
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        // `validate_domain_data_payload_at` runs
        // `decode_compact_country_core_scalars_data_payload`
        // (`domain_data_authority.rs:11969`) on EVERY DOMAIN_DATA record whatever
        // its status, and any complaint it makes is `malformed_payload_records`,
        // which `strict_stream_certification_verdict` refuses. Excluding a record
        // before checking its payload let a stream production refuses produce a
        // plausible measurement here.
        //
        // Unlike d38's, this domain's authority DOES walk the rows, so a bad row
        // inside an EXCLUDED record refuses the stream too.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        let mut bad_payloads: Vec<(String, Vec<u8>)> = Vec::new();
        // Zero tag_db_id (`domain_data_authority.rs:12007`).
        bad_payloads.push(("zero tag_db_id".to_string(), d63_payload(FAILED, 0, 10)));
        // An unknown row flag bit (`:12013`).
        let mut unknown_flag = d63_payload(FAILED, 7, 10);
        unknown_flag[d63::HEADER_LEN + 4..d63::HEADER_LEN + 8]
            .copy_from_slice(&(1u32 << 20).to_le_bytes());
        bad_payloads.push(("unknown row flags".to_string(), unknown_flag));
        // An out-of-range present value (`:12022`, `validate_compact_optional_u32`).
        bad_payloads.push((
            "manpower_pool over its maximum".to_string(),
            d63_payload(FAILED, 7, d63::MAX_MANPOWER_POOL + 1),
        ));
        // A gated-off field carrying a non-zero word (same helper).
        let mut ungated = d63_payload(FAILED, 7, 10);
        ungated[d63::HEADER_LEN + 4..d63::HEADER_LEN + 8].copy_from_slice(&0u32.to_le_bytes());
        bad_payloads.push(("gated-off manpower_pool is nonzero".to_string(), ungated));
        // The experience channels carrying a value without the flag (`:12054`).
        let mut ungated_experience = d63_payload(FAILED, 7, 10);
        ungated_experience[d63::HEADER_LEN + 20..d63::HEADER_LEN + 24]
            .copy_from_slice(&1i32.to_le_bytes());
        bad_payloads.push((
            "experience present without its flag".to_string(),
            ungated_experience,
        ));

        for (label, payload) in &bad_payloads {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                payload,
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(0, 7, 11),
            ));
            s.extend(frame_end(4, 101));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "an EXCLUDED record with {label} must refuse the stream, not be skipped"
            );
        }
        // Every failure bit, not just the one: the authority is status-independent,
        // so the exclusion reason cannot matter.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D63,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d63_payload(1 << bit, 0, 10),
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "an excluded record (status bit {bit}) with a zero tag_db_id must refuse"
            );
        }
        // And a WELL-FORMED excluded record is still excluded, not refused — the
        // authority must reject bad payloads and nothing else.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(FAILED, 7, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 11),
        ));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 101);

        // The authority stops where production's does: a `tag_db_id` repeated
        // inside ONE frame is the FOLD's rule (`world_fold.rs:9363`), not the
        // authority's, and the fold returns before its row loop on an excluded
        // record — so an excluded record carrying a duplicate is a stream
        // production certifies, and refusing it here would be stricter than
        // production.
        let mut duplicated = d63_payload(FAILED, 7, 10);
        let row: Vec<u8> = duplicated[d63::HEADER_LEN..].to_vec();
        duplicated.extend_from_slice(&row);
        duplicated[8..12].copy_from_slice(&2u32.to_le_bytes()); // row_count
        duplicated[12..16].copy_from_slice(&2u32.to_le_bytes()); // changed_rows
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &duplicated,
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D63,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d63_payload(0, 7, 11),
        ));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).unwrap_or_else(|e| {
                panic!("an excluded record's frame-level rule is not the authority's: {e}")
            });
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        // A sniffing harness reads a decapitated IdentityV1 stream — one whose
        // mandatory CID1 record has been spliced off — as a healthy legacy stream
        // and measures it, because both begin with an ordinary record at sequence
        // 1. Production takes the mode from its caller (`stream_fold.rs:220`) and
        // the validator never sniffs (`stream_validator.rs:328`), so the mode is a
        // parameter here too and a mismatch fails closed in BOTH directions.
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(
            scan_stream(
                &decapitated,
                &test_collection(),
                StreamIdentityMode::IdentityV1
            )
            .is_err(),
            "an identity-mode stream whose CID1 record is missing must be refused, \
             not measured as legacy"
        );
        // And the same bytes under the legacy claim are a healthy legacy stream —
        // which is the point: the bytes cannot tell the two apart, only the caller
        // can.
        assert_eq!(
            scan_stream(&decapitated, &test_collection(), StreamIdentityMode::Legacy)
                .expect("a genuine legacy stream")
                .hours
                .len(),
            2
        );

        // The other direction: an identity-bearing stream read under the legacy
        // claim trips on its CID1 record instead of being silently accepted.
        let mut identity_stream = Vec::new();
        identity_stream.extend(record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        ));
        identity_stream.extend(&good_stream());
        assert!(
            scan_stream(
                &identity_stream,
                &test_collection(),
                StreamIdentityMode::Legacy
            )
            .is_err(),
            "a CID1 record under the legacy claim must be refused"
        );
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the harness reads a file from disk and must
        // return a diagnosis, never unwind.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..4_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 400;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            // Half the cases keep a valid magic so the parse gets further in.
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(b"H4CR");
            }
            let _ = scan_stream(&bytes, &test_collection(), StreamIdentityMode::Legacy);
        }
        // And corruption of a genuinely well-formed stream, byte by byte.
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy);
        }
    }
}
