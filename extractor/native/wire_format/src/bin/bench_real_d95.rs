//! Measure the wire format against the shipping compact stream on REAL d95
//! `pov_spotted_units_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d95-scheduled capture that also
//! carries d18 (`runs/batch3/deepwar_b2`). The stream's own d95 DATA records are
//! the shipping format's cost for the domain: a 44-byte SPU1 header and a 12-byte
//! row per spotted unit, full-replace every hour, exactly what the recorder
//! stores today. This tool replays those records into per-hour states — the same
//! fold `world_fold::fold_pov_spotted_units` performs (`:10538`) — and re-encodes
//! every hour with this crate's grammar: a BASELINE at each 48-hour block start,
//! field-granular stored-minimal DELTAs between.
//!
//! **One pass, because the state is three rows.** d95 stores one frame record and
//! whatever units the POV can see; on the reference capture that is one row an
//! hour. Every hour's state is therefore retained during the scan, and the scan
//! either accepts the whole stream or refuses it before any artifact is written —
//! the two-pass shape exists for large states (d22, d83) and buys nothing here.
//!
//! **Nothing here re-derives a contract it can call.** Payload, header and row
//! decoding, the per-hour accumulator and the d18 dependency all go through
//! `wire_format::d95`'s public boundary; H4CR framing, record semantics, sequence
//! continuity and the status helpers go through `compact_contract`'s own parser,
//! stateful validator and predicates.
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
//!   (`recorder_host/src/stream_fold.rs:277`) that do not require DOMAIN_PERF
//!   accounting: a DOMAIN_DATA record opens its game hour, a FRAME_END closes the
//!   open hour and only that hour, and closed hours strictly increase;
//! - that tracker's DOMAIN_PERF hour binding (`:328`-`:365`), including the DATA
//!   arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal validation (`stream_fold.rs:46`-`:89`) and the seal's count
//!   cross-check against distinct non-probe PERF domains (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END;
//! - every d95 payload through `d95::HourAccumulator::fold_payload`, so a payload
//!   the recorder would refuse as malformed stops the scan rather than unwinding
//!   later;
//! - every d18 payload through [`d18_roster`], the documented subset of
//!   `fold_country_identity` (`world_fold.rs:13918`-`:14157`) that decides whether
//!   a d18 record committed the roster d95 depends on — and, ahead of it,
//!   [`check_d18_payload_authority`], the one rule production's status-independent
//!   d18 payload authority makes that the fold does not
//!   (`changed_rows > row_count`, `domain_data_authority.rs:1029`-`:1033`), applied
//!   fail-closed as a stream rejection because the fold has no behaviour there to
//!   preserve. Every OTHER authority refusal keeps the fold's classification and is
//!   reported as a named deviation instead: a refused payload into
//!   `d18_refused_records`, a failure-status record whose body the authority would
//!   reject into `d18_failure_records_with_unwalkable_body`. Both are 0 on the
//!   measured capture. Ported from `bench_real_d96.rs`, whose Codex rounds 1-3
//!   established all three rulings.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. This crate cannot depend on `recorder_host` (it pulls in every
//! capture lane), so the relevant predicates are mirrored here.
//!
//! The DATA payloads of every domain other than 95 and 18 pass UNVALIDATED
//! beyond header sanity, the stream validator and the hour-boundary rules —
//! production validates every domain's payload (`stream_fold.rs:593`). That
//! unvalidated remainder is explicitly BOUNDED: every closed game hour must
//! carry at least one d95 DATA record (several are legal, ordered by the
//! within-hour sequence; the bound is enforced in [`commit_hour`]), and the
//! d18 lane must be all-or-nothing — at least one record per closed hour when
//! scheduled, none at all when not (the check after the EOF rules in
//! [`scan_stream`]) — so a d95 or d18 record relabelled to another domain id,
//! or dropped, refuses the run instead of silently dropping hours or silently
//! converting them into no-roster exclusions.
//!
//! # Exclusions, in the fold's own order — and d95 has no status word
//!
//! `fold_pov_spotted_units` never reads a status word: the SPU1 payload does not
//! carry one, and the fold consults none (`world_fold.rs:10538`-`:10561`). The
//! recipe's step 9 requires an explicit, recorded decision where a fold ignores
//! status, so here it is: **the domain's DOMAIN_PERF status word is parsed and
//! reported, never enforced.** Excluding an hour on it would diverge from the
//! recorder, which folds the payload whatever the perf record says; refusing the
//! stream on it would refuse streams the recorder accepts. The histogram is
//! printed so a status the recorder ignores is still visible.
//!
//! Every path on which the fold returns without folding, and which of the recipe's
//! three outcomes it is:
//!
//! - **A malformed SPU1 payload** (`world_fold.rs:10540`-`:10541`, the codec's
//!   `decode` at `compact_contract/src/spotted_units.rs:88`): the payload is not
//!   this grammar, so it is a **stream rejection** — the same treatment d83 gives
//!   a magic or trailing-byte failure. The recorder marks the domain-hour unknown
//!   and continues; a measurement cannot, because a corrupt payload means the
//!   capture is not the thing being measured.
//! - **No d18 roster this hour** (`:10542`-`:10544`) and **the POV country absent
//!   from the roster** (`:10546`-`:10551`): an **excluded hour**. These are a
//!   dependency state, not a grammar failure — an entire capture can legitimately
//!   be in it (`runs/batch3/domain_classification.md`, Finding 3: 3b never
//!   scheduled d18, so every d67/d94/d95 hour there folds to an error). Counted
//!   and reported by reason; the shipping records of an excluded hour are charged
//!   to neither arm.
//! - **A capture sequence that does not strictly increase within the hour**
//!   (`:10553`-`:10559`): the fold clears the stored snapshot and keeps going, and
//!   the payload itself was well formed, so this is an **excluded hour** — but
//!   only if no LATER d95 record in the same hour stores a snapshot. A cleared
//!   accumulator has no sequence to compare against, so the next record succeeds
//!   whatever it carries (`:10553`, `:10559`) and the hour is measured from it.
//!
//! # Several d95 records in one hour are legal
//!
//! The recipe's "one record per hour, enforced before status exclusion" rule does
//! not transfer: this fold defines multi-record semantics inside the hour — a
//! payload whose `capture_sequence` strictly increases over the stored one
//! replaces it (`world_fold.rs:10553`-`:10559`, the assignment at `:10559`), and
//! one that does not clears the stored snapshot rather than ending the hour: the
//! next record then compares against nothing and stores whatever it carries.
//! Enforcing uniqueness would refuse streams the recorder folds. What replaces
//! it is the
//! accumulator itself ([`d95::HourAccumulator`]) plus the boundary rules above,
//! which already forbid an hour reopening after it closed. The rule's purpose —
//! that two records in one hour cannot become two measurement hours — is met
//! because the hour is committed once, at its FRAME_END.
//!
//! # Other writers of the accumulator
//!
//! `pov_spotted_units_this_hour` is written in exactly four places
//! (`world_fold.rs:2446`, `:3095`, `:4928`, `:10542`-`:10559`): construction, the
//! hour reset, the dispatcher's d95 error arm and the fold itself. No other
//! domain touches it, so there is no foreign writer to model, prove inert or
//! assert inert. The one cross-domain input is d18, and it is **modelled** here
//! rather than asserted inert, because the fold's behaviour genuinely depends on
//! it hour by hour.
//!
//! # Gaps force a baseline
//!
//! An excluded hour is coverage this domain lost, which is §8's gap, and
//! `WIRE_FORMAT.md:187`-`:190` forces the next state observation to be a `BASELINE`
//! rather than a delta across unobserved time. [`opens_block`] implements that
//! beside the 48-hour cadence, and the block chain restarts there. The reference
//! capture has no excluded hour, so no baseline is forced on it and the block
//! spans `compress_arms.py` slices still coincide with the baselines written.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d95.bin   — the d95 DATA records of measured hours verbatim (32-byte
//!                    H4CR header + payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_is_failure, compact_status_unknown_flags_v1, parse_compact_record_header,
    COMPACT_DOMAIN_PERF_BASE_LEN, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, check_minimal, decode_baseline, decode_delta, diff, encode_baseline, encode_delta,
    KeyedDelta, KeyedState,
};
use wire_format::d95::{self, CountryRoster, HourAccumulator, Surface};
use wire_format::Schema;

const D18: u16 = 18;
const D95: u16 = 95;
const BLOCK_HOURS: usize = 48;

/// One measured d95 hour.
struct D95Hour {
    game_hour: i32,
    /// Every d95 DATA record of the hour, header included: the shipping arm is
    /// these bytes. Normally one; the fold admits several (see the module docs).
    ship_spans: Vec<Range<usize>>,
    /// The hour's stored state, folded through `d95`'s own accumulator.
    state: d95::SpottedUnitsState,
    /// Which surfaces the producer did not capture this hour — the four-state
    /// centrepiece, counted on real data rather than assumed absent.
    land_unmeasured: bool,
    taskforce_unmeasured: bool,
}

/// Why an hour that carried d95 records stored nothing.
#[derive(Default)]
struct Excluded {
    no_roster_this_hour: usize,
    pov_absent_from_roster: usize,
    sequence_not_increasing: usize,
}

impl Excluded {
    fn total(&self) -> usize {
        self.no_roster_this_hour + self.pov_absent_from_roster + self.sequence_not_increasing
    }
}

struct StreamScan {
    hours: Vec<D95Hour>,
    excluded: Excluded,
    /// d95 DATA records seen, of any outcome.
    d95_records: usize,
    /// d18 DATA records seen, of any outcome — the completeness bound's
    /// numerator (all-or-nothing against closed hours).
    d18_records: usize,
    /// Measured d95 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// d95 DOMAIN_PERF status words, reported and never enforced.
    perf_status: BTreeMap<u32, usize>,
    /// d18 records whose status marked the domain failed, so the record committed
    /// nothing (`world_fold.rs:13922`-`:13928`). Whatever an earlier record in the
    /// same hour committed still stands, so this does not by itself leave the hour
    /// without a roster.
    d18_failure_records: usize,
    /// d18 records the mirrored fold refused, which likewise commit nothing —
    /// rather than rejecting the stream, and rather than undoing an earlier
    /// commit in the same hour.
    d18_refused_records: usize,
    /// d18 records carrying status bits outside the assigned v1 vocabulary. The
    /// recorder classifies and commits them anyway ([`d18_roster`]), so this is
    /// reported and nothing follows from it.
    d18_unassigned_status_records: usize,
    /// Failure-status d18 records whose BODY the payload authority would reject.
    /// The fold never reads it, so the record keeps the fold's classification and
    /// this is reported as a named certification deviation instead (the recipe's
    /// d49 rule). 0 on the measured capture.
    d18_failure_records_with_unwalkable_body: usize,
    d18_first_failure_body_deviation: Option<String>,
    /// The POV countries the capture observed from.
    pov_countries: BTreeSet<u32>,
    /// Peak per-hour row counts, for the report.
    land_max: usize,
    taskforce_max: usize,
}

const FRAME_END_KNOWN_STATUS_MASK: u32 = (1 << 14) - 1;
const PROBE_DOMAIN_ID: u16 = 250;

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

/// The `country_identity` (d18) roster, as `fold_country_identity` computes it
/// (`world_fold.rs:13918`-`:14157`).
///
/// `Ok(roster: Some(ids))` is a record that folded and full-replaced the roster;
/// `Ok(roster: None)` is the fold's failure-status early return, which commits
/// nothing (`:13922`-`:13928`); `Err` is a payload the fold rejects, which
/// likewise commits nothing — the commit is the last thing
/// `fold_country_identity` does (`:14149`-`:14156`), so any earlier error leaves
/// the roster and `country_identity_known_this_hour` exactly as they were.
///
/// **Committing nothing is not the same as leaving the hour unmeasured.** The
/// known flag is set by any d18 record that *succeeded* this hour, so a failed or
/// refused record after a successful one changes nothing and d95 is still
/// measured; the hour is unmeasured only when no d18 record succeeded in it.
///
/// **A documented subset with one deliberate narrowing**: this returns only the
/// `tag_db_id` set, which is the only column d95's fold reads (`:10546`), but it
/// walks and checks the whole payload — the legacy row table *and* the optional
/// CIM1 extension — because the recorder's commit is all-or-nothing. Reading the
/// rows and ignoring the extension would mark a roster measured that the recorder
/// refused, and the d95 hours behind it would be measured instead of excluded.
///
/// **Unassigned v1 status bits are reported, never refused, and that is a
/// deliberate divergence from the recipe's rule for a domain's OWN status word.**
/// `compact_contract/src/lib.rs:121`-`:123` tells callers of
/// `compact_status_is_failure` to reject an unassigned bit first, and
/// `fold_country_identity` does not: it tests the failure mask alone
/// (`world_fold.rs:13922`) and then parses and commits the roster. Refusing here
/// would make the model stricter than the recorder and would exclude a d95 hour
/// the recorder measures — the very defect this harness exists to avoid. The bits
/// are counted and printed instead, and the gap belongs to a d18 port, not to
/// this one.
struct D18Fold {
    /// The committed roster, or `None` where the fold committed nothing.
    roster: Option<BTreeSet<u32>>,
    /// Status bits outside the assigned v1 vocabulary, for the report.
    unassigned_status_bits: u32,
    /// Set on a failure-status record whose BODY the payload authority would
    /// reject. The fold never reads that body (`world_fold.rs:14158`-`:14164`
    /// returns before the row walk), so the record's CLASSIFICATION is "failure
    /// record" and this harness keeps it — but
    /// `decode_compact_country_identity_data_payload` walks the rows whatever the
    /// status (`domain_data_authority.rs:1042`), so production would not certify
    /// the stream. The recipe's d49 rule: keep the fold's classification, report
    /// the certification consequence as a named deviation. `None` on every record
    /// of the measured capture.
    body_deviation: Option<String>,
}

/// The d18 payload authority's status-independent check that the FOLD does not
/// make — applied fail-closed, as a stream rejection.
///
/// `validate_domain_data_payload_at` (`stream_fold.rs:593`) runs
/// `decode_compact_country_identity_data_payload`
/// (`domain_data_authority.rs:1013`) on EVERY d18 DOMAIN_DATA record whatever its
/// status, and a parse error clears the stream's decodable verdict. Its checks are
/// the same walk [`d18_roster`] performs — row truncation, cosmetic markers,
/// zero/empty/duplicate tags, the whole CIM1 extension, trailing bytes — with
/// **one exception**: `changed_rows > row_count` at offset `+12`
/// (`domain_data_authority.rs:1029`-`:1033`), a word `fold_country_identity`
/// never reads at all (it takes `row_count` from `+8` and `status_flags` from
/// `+16` and nothing else out of the header, `world_fold.rs:14156`-`:14157`).
///
/// That layering decides the treatment, and the two halves are NOT the same
/// (d49's rule where the fold has a behaviour, d65's where it has none):
///
/// - Where the fold HAS a behaviour, the harness keeps the fold's classification
///   and reports the certification consequence. A d18 payload the fold refuses is
///   that case: it commits nothing, the recorder notes it and continues, so this
///   harness counts it into `d18_refused_records` rather than refusing a stream
///   the recorder accepts.
/// - Where the fold has NO behaviour, there is nothing to preserve and the
///   authority applies fail-closed. `changed_rows` is that case: the fold folds
///   such a record normally and commits its roster, so measuring it would measure
///   a stream production does not certify.
///
/// Ported from `bench_real_d96.rs`, where the rule was Codex round-1 finding
/// `019fa530-c013-74a2-b717-31f8e89e8639` and its over-reach — refusing a payload
/// too short to carry the word, which the FOLD refuses too and the recorder
/// tolerates — was round-2 finding `019fa53d-daf5-7810-a470-aeeb710e8166`. A
/// short payload therefore returns `Ok` here and falls through to [`d18_roster`].
fn check_d18_payload_authority(payload: &[u8]) -> Result<(), String> {
    // Not a check: the authority's 20-byte minimum (`domain_data_authority.rs:1018`)
    // is also the FOLD's (`world_fold.rs:14155`), so it is `d18_roster`'s business.
    // The length is read here only to know whether the word below exists at all.
    if payload.len() < 20 {
        return Ok(());
    }
    let row_count = u32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]) as usize;
    let changed_rows =
        u32::from_le_bytes([payload[12], payload[13], payload[14], payload[15]]) as usize;
    // `:1029`-`:1033`. The one rule the fold never applies, because it never reads
    // the word.
    if changed_rows > row_count {
        return Err(format!(
            "country_identity data changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    Ok(())
}

fn d18_roster(payload: &[u8]) -> Result<D18Fold, String> {
    fn need(payload: &[u8], end: usize, label: &str) -> Result<(), String> {
        if payload.len() < end {
            return Err(format!("country_identity {label} is truncated"));
        }
        Ok(())
    }
    fn u16_at(b: &[u8], off: usize) -> u16 {
        u16::from_le_bytes([b[off], b[off + 1]])
    }
    fn u32_at(b: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }
    fn read_string(payload: &[u8], cursor: &mut usize, label: &str) -> Result<usize, String> {
        need(payload, *cursor + 2, label)?;
        let len = u16_at(payload, *cursor) as usize;
        *cursor += 2;
        need(payload, *cursor + len, label)?;
        let start = *cursor;
        *cursor += len;
        core::str::from_utf8(&payload[start..start + len])
            .map_err(|e| format!("country_identity {label}: invalid utf-8: {e}"))?;
        Ok(len)
    }

    // The body walk itself, separated from the status decision below because the
    // two callers need it for different reasons — the clean path to COMMIT the
    // roster it proves, the failure path only to see whether the payload authority
    // would have accepted the body. It mutates nothing, so neither can leave a
    // half-applied roster behind.
    fn walk_body(payload: &[u8]) -> Result<BTreeSet<u32>, String> {
        let row_count = u32_at(payload, 8) as usize;
        let mut cursor = 20usize;
        let mut fresh: BTreeSet<u32> = BTreeSet::new();
        for _ in 0..row_count {
            need(payload, cursor + 4, "row tag_db_id")?;
            let tag_db_id = u32_at(payload, cursor);
            cursor += 4;
            let tag_len = read_string(payload, &mut cursor, "tag")?;
            if tag_db_id == 0 {
                return Err("country_identity row with zero tag_db_id".into());
            }
            if tag_len == 0 {
                return Err(format!("country_identity row {tag_db_id} with empty tag"));
            }
            if !fresh.insert(tag_db_id) {
                return Err(format!(
                    "country_identity duplicates tag_db_id {tag_db_id}"
                ));
            }
            need(payload, cursor + 1, "cosmetic marker")?;
            let marker = payload[cursor];
            cursor += 1;
            match marker {
                0 | 1 => {}
                2 => {
                    let cosmetic_len = read_string(payload, &mut cursor, "cosmetic")?;
                    if cosmetic_len == 0 {
                        return Err(format!(
                            "country_identity row {tag_db_id} present cosmetic marker with empty string"
                        ));
                    }
                }
                other => {
                    return Err(format!(
                        "country_identity row {tag_db_id} invalid cosmetic marker {other}"
                    ))
                }
            }
        }

        // The optional CIM1 extension (`world_fold.rs:13985`-`:14141`). Walked in
        // full: its rejections are the recorder's rejections.
        if cursor < payload.len() {
            const CIM1_HEADER_LEN: usize = 40;
            need(payload, cursor + CIM1_HEADER_LEN, "CIM1 header")?;
            if &payload[cursor..cursor + 4] != b"CIM1"
                || u16_at(payload, cursor + 4) != 1
                || u16_at(payload, cursor + 6) as usize != CIM1_HEADER_LEN
                || u32_at(payload, cursor + 36) != 0
            {
                return Err("country_identity CIM1 header is invalid".into());
            }
            let claims_status = u32_at(payload, cursor + 8);
            let cores_status = u32_at(payload, cursor + 12);
            let major_status = u32_at(payload, cursor + 16);
            let state_count = u32_at(payload, cursor + 20) as usize;
            let expected_claim_memberships = u32_at(payload, cursor + 24) as usize;
            let expected_core_memberships = u32_at(payload, cursor + 28) as usize;
            let major_count = u32_at(payload, cursor + 32) as usize;
            let claims_available = !compact_status_is_failure(claims_status);
            let cores_available = !compact_status_is_failure(cores_status);
            let major_available = !compact_status_is_failure(major_status);
            if state_count > 100_000
                || expected_claim_memberships > 1_000_000
                || expected_core_memberships > 1_000_000
                || major_count > fresh.len()
            {
                return Err("country_identity CIM1 count exceeds safety bound".into());
            }
            if (!claims_available && expected_claim_memberships != 0)
                || (!cores_available && expected_core_memberships != 0)
                || ((!claims_available && !cores_available) && state_count != 0)
                || (!major_available && major_count != 0)
                || (major_available && major_count != fresh.len())
            {
                return Err("country_identity CIM1 status/count contract is invalid".into());
            }
            let mut claim_memberships = 0usize;
            let mut core_memberships = 0usize;
            let mut previous_state_id = 0u32;
            cursor += CIM1_HEADER_LEN;
            for index in 0..state_count {
                need(payload, cursor + 12, "CIM1 state row")?;
                let state_id = u32_at(payload, cursor);
                let claim_count = u32_at(payload, cursor + 4) as usize;
                let core_count = u32_at(payload, cursor + 8) as usize;
                cursor += 12;
                if state_id == 0 || (index != 0 && state_id <= previous_state_id) {
                    return Err(format!(
                        "country_identity CIM1 state row {index} is not strictly ordered"
                    ));
                }
                previous_state_id = state_id;
                if (!claims_available && claim_count != 0) || (!cores_available && core_count != 0) {
                    return Err(format!(
                        "country_identity CIM1 unavailable state row {state_id} carries members"
                    ));
                }
                let row_bytes = claim_count
                    .checked_add(core_count)
                    .and_then(|count| count.checked_mul(4))
                    .ok_or_else(|| "country_identity CIM1 state row size overflow".to_string())?;
                need(payload, cursor + row_bytes, "CIM1 state members")?;
                for (label, count, total) in [
                    ("claim", claim_count, &mut claim_memberships),
                    ("core", core_count, &mut core_memberships),
                ] {
                    let mut previous_tag_id = 0u32;
                    for member_index in 0..count {
                        let tag_db_id = u32_at(payload, cursor);
                        cursor += 4;
                        if tag_db_id == 0
                            || (member_index != 0 && tag_db_id <= previous_tag_id)
                            || !fresh.contains(&tag_db_id)
                        {
                            return Err(format!(
                                "country_identity CIM1 state {state_id} {label} member is invalid"
                            ));
                        }
                        previous_tag_id = tag_db_id;
                        *total += 1;
                    }
                }
            }
            if claim_memberships != expected_claim_memberships
                || core_memberships != expected_core_memberships
            {
                return Err("country_identity CIM1 membership totals do not match header".into());
            }
            let mut majors: BTreeSet<u32> = BTreeSet::new();
            let mut previous_major_id = 0u32;
            for index in 0..major_count {
                need(payload, cursor + 8, "CIM1 major row")?;
                let tag_db_id = u32_at(payload, cursor);
                let marker = payload[cursor + 4];
                if tag_db_id == 0
                    || (index != 0 && tag_db_id <= previous_major_id)
                    || marker > 1
                    || payload[cursor + 5..cursor + 8] != [0, 0, 0]
                    || !fresh.contains(&tag_db_id)
                {
                    return Err(format!(
                        "country_identity CIM1 major row {index} is invalid"
                    ));
                }
                previous_major_id = tag_db_id;
                majors.insert(tag_db_id);
                cursor += 8;
            }
            if major_available && majors != fresh {
                return Err("country_identity CIM1 major roster is incomplete".into());
            }
        }
        if cursor != payload.len() {
            return Err(format!(
                "country_identity len {} != consumed {cursor}",
                payload.len()
            ));
        }
        Ok(fresh)
    }

    need(payload, 20, "header")?;
    let status_flags = u32_at(payload, 16);
    // Reported, never refused — see the type's docs for why mirroring the
    // recorder beats applying the stricter rule to a dependency.
    let unassigned_status_bits = compact_status_unknown_flags_v1(status_flags);
    // `world_fold.rs:14158`-`:14164`: the status word is consulted IMMEDIATELY
    // after the 20-byte header, and the fold returns `Ok` **without reading the
    // body at all** — so a failure-status record is a failure record however
    // unwalkable its body is, and it commits nothing: whatever an earlier record
    // committed this hour stands.
    //
    // The comment this replaces had the row walk running first, on the grounds
    // that production's payload authority is status-independent. Two things were
    // conflated there, and the d96 port's round-3 review separated them (thread
    // 019fa548-5e33-7061-9236-5c7151f92098): the authority's status-independence
    // decides what this harness must REPORT, while the FOLD's order decides how
    // the record is CLASSIFIED. So the body is still walked on the failure path —
    // only to record a body the authority (`domain_data_authority.rs:1042`) would
    // reject as a named certification deviation, per the recipe's d49 rule.
    if compact_status_is_failure(status_flags) {
        return Ok(D18Fold {
            roster: None,
            unassigned_status_bits,
            body_deviation: walk_body(payload).err(),
        });
    }
    Ok(D18Fold {
        roster: Some(walk_body(payload)?),
        unassigned_status_bits,
        body_deviation: None,
    })
}

/// Validate the whole stream against the canonical contract and fold every d95
/// hour. Every rejection path is an `Err`, never a silently shorter measurement
/// and never a panic — see the module docs for what is enforced and what the
/// recorder's own validator additionally checks.
fn scan_stream(
    stream: &[u8],
    schema: &Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // Production receives the identity mode from its caller
    // (`stream_fold.rs:220`); the first record cannot state it safely.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    // COMPLETENESS accounting: closed game hours and the hours in which at
    // least one d18 DATA record landed, for the d18 all-or-nothing bound (the
    // per-hour d95 bound lives in `commit_hour`).
    let mut closed_hours = 0usize;
    let mut d18_covered_hours = 0usize;
    let mut hour_saw_d18 = false;
    let mut scan = StreamScan {
        hours: Vec::new(),
        excluded: Excluded::default(),
        d95_records: 0,
        d18_records: 0,
        non_contiguous: 0,
        perf_status: BTreeMap::new(),
        d18_failure_records: 0,
        d18_refused_records: 0,
        d18_unassigned_status_records: 0,
        d18_failure_records_with_unwalkable_body: 0,
        d18_first_failure_body_deviation: None,
        pov_countries: BTreeSet::new(),
        land_max: 0,
        taskforce_max: 0,
    };

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d95 hour that entered the measurement — the contiguity
    // report, which an excluded hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // The recorder's own per-hour d95 accumulator and its d18 roster, carried
    // across records exactly as the fold carries them: the roster's id map
    // persists across hours while its known flag resets (`world_fold.rs:2999`),
    // and the accumulator resets whole (`:3095`).
    let mut roster = CountryRoster::unmeasured();
    let mut hour_fold = HourAccumulator::new();
    let mut hour_ship_spans: Vec<Range<usize>> = Vec::new();
    let mut hour_excluded: Option<&'static str> = None;

    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|e| format!("record at byte {pos}: {e}"))?;
        let payload_start = pos + COMPACT_RECORD_HEADER_LEN;
        let record_end = payload_start
            .checked_add(header.payload_len)
            .ok_or_else(|| format!("record at byte {pos}: length overflow"))?;
        if record_end > stream.len() {
            // `eof_check`: a stream ending mid-record is not a short stream, it
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
                // Reported, never enforced — see the module docs. The status word
                // sits at payload offset 32 (`collector/src/attach_log.rs:60069`-
                // `:60077`), and `validate_compact_record_semantics` has already
                // made the base length legal.
                if header.domain_id == D95 && payload.len() >= COMPACT_DOMAIN_PERF_BASE_LEN {
                    let status =
                        u32::from_le_bytes(payload[32..36].try_into().expect("4 bytes"));
                    *scan.perf_status.entry(status).or_insert(0) += 1;
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
                let (emitted_domains, scheduled_domains) = check_frame_end_seal(payload, pos)?;
                let observed = non_probe_perf_domains.len().min(u32::MAX as usize) as u32;
                if observed != emitted_domains || observed != scheduled_domains {
                    return Err(format!(
                        "FRAME_END seal count mismatch at byte {pos}: observed {observed} \
                         non-probe PERF domains, emitted {emitted_domains}, scheduled \
                         {scheduled_domains}"
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

                // Commit the closing hour: the state the recorder holds when the
                // hour seals is the hour's d95 state.
                commit_hour(&mut scan, schema, header.game_hour_raw, &hour_fold,
                            &mut hour_ship_spans, hour_excluded, &mut last_measured_hour)?;
                hour_excluded = None;
                hour_fold.begin_hour();
                roster.begin_hour();

                open_hour = None;
                last_closed_hour = Some(header.game_hour_raw);
                closed_hours += 1;
                if hour_saw_d18 {
                    d18_covered_hours += 1;
                }
                hour_saw_d18 = false;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            match header.domain_id {
                D18 => {
                    scan.d18_records += 1;
                    hour_saw_d18 = true;
                    // Production's status-independent payload authority, fail-closed
                    // and BEFORE the fold mirror: its complaint clears the stream's
                    // decodable verdict at seal time, so a record it rejects is not
                    // an hour to skip. Only the checks the fold does not make live
                    // there — see [`check_d18_payload_authority`].
                    check_d18_payload_authority(payload).map_err(|e| {
                        format!(
                            "d18 payload at byte {pos} (hour {}): {e}. Production's \
                             status-independent payload authority refuses this record, and its \
                             complaint refuses the whole stream at seal time.",
                            header.game_hour_raw
                        )
                    })?;
                    match d18_roster(payload) {
                    Ok(fold) => {
                        if fold.unassigned_status_bits != 0 {
                            scan.d18_unassigned_status_records += 1;
                        }
                        if let Some(reason) = fold.body_deviation {
                            scan.d18_failure_records_with_unwalkable_body += 1;
                            if scan.d18_first_failure_body_deviation.is_none() {
                                scan.d18_first_failure_body_deviation = Some(format!(
                                    "hour {} at byte {pos}: {reason}",
                                    header.game_hour_raw
                                ));
                            }
                        }
                        match fold.roster {
                            Some(ids) => roster.measure(ids),
                            None => scan.d18_failure_records += 1,
                        }
                    }
                    // A d18 payload the recorder refuses commits nothing, so
                    // the roster keeps whatever an earlier record in this hour
                    // committed — and stays unmeasured if none did. Refusing the
                    // stream would be louder than the recorder on a domain this
                    // assignment does not port.
                    Err(_) => scan.d18_refused_records += 1,
                    }
                }
                D95 => {
                    scan.d95_records += 1;
                    if let Err(reason) = hour_fold.fold_payload(payload, &roster) {
                        // Which of the three outcomes this is depends on WHY the
                        // fold refused: a malformed payload is not this grammar
                        // and invalidates the run; the semantic refusals are the
                        // recorder's own "clear the hour and continue".
                        hour_excluded = Some(classify_refusal(&reason).ok_or_else(|| {
                            format!(
                                "d95 DATA at byte {pos} (hour {}): {reason}",
                                header.game_hour_raw
                            )
                        })?);
                    }
                    hour_ship_spans.push(pos..record_end);
                }
                _ => {}
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
    // "No d95 records at all" and "records, but every hour excluded" are two
    // different things and only the first refuses the stream. A capture whose d18
    // roster is never scheduled folds to an error on every d95 hour — batch-3's
    // 3b stream is exactly that (`runs/batch3/domain_classification.md`,
    // Finding 3) — and the recorder clears each domain-hour and continues. The
    // caller reports those exclusions; refusing here would call a legitimate
    // capture corrupt.
    if scan.d95_records == 0 {
        return Err("stream carries no d95 DATA records".to_string());
    }

    // COMPLETENESS, the d18 half: the d18 lane must be all-or-nothing — at
    // least one record per closed hour when scheduled, none at all when not
    // (d18 is full-replace, and the mirror already tolerates several records
    // per hour by last-success-wins). A d18 record lost in one hour would
    // silently convert that hour's d95 records into no-roster exclusions, so
    // anything between the two shapes is a partially-lost or relabelled d18
    // lane and refuses the run. (The per-hour d95 bound is enforced in
    // `commit_hour`.)
    if scan.d18_records != 0 && d18_covered_hours != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but only {d18_covered_hours} carry a \
             d18 DATA record; a capture scheduling d18 carries at least one per closed hour, \
             and a partially-missing d18 lane would silently convert its hours' d95 records \
             into no-roster exclusions"
        ));
    }

    Ok(scan)
}

/// Whether this measured hour must open a new block with a `BASELINE`.
///
/// Two reasons, and the second is not the 48-hour cadence:
///
/// 1. the block chain reached its `BLOCK_HOURS` length;
/// 2. **the previous measured hour is not this hour minus one.** An excluded hour
///    is coverage this domain lost, which is §8's gap, and `WIRE_FORMAT.md:187`-`:190`
///    is explicit: "A gap affecting the state part forces the next state
///    observation to be a `BASELINE`. State may have moved and returned while
///    nobody was looking, so neither a delta nor an `UNCHANGED` can be justified
///    across unobserved time." Delta-encoding hour 102 against hour 100's state
///    because hour 101 was excluded would produce bytes that decode — the local
///    replay would not notice, because the reader's prior is never invalidated —
///    while asserting a continuity the capture does not have.
///
/// The block chain restarts at a gap, so the run after it is a full block again.
///
/// **Interaction with `scripts/compress_arms.py`.** Its default index mode slices
/// block spans every `BLOCK_HOURS` offsets, which coincides with the baselines
/// this function places only while the measured hours are contiguous. The
/// reference capture has zero excluded hours, so the two agree exactly there; a
/// capture with gaps would need the clock-mode treatment d38 introduced.
fn opens_block(hours_in_block: usize, previous_measured_hour: Option<i32>, game_hour: i32) -> bool {
    if hours_in_block % BLOCK_HOURS == 0 {
        return true;
    }
    match previous_measured_hour {
        Some(previous) => game_hour != previous.saturating_add(1),
        None => true,
    }
}

/// One wire-arm hour: the §1.5/block baseline decision (`opens_block`), the
/// encode, and the per-hour reader replay over both collections. Factored out
/// of `main` so the tests execute the PRODUCTION encode path over a scanned
/// stream: a mutant that reverts the baseline decision here to
/// chain-position-only fails the gap test (a Codex round-2 finding — the
/// earlier test exercised the decision function but not this callsite).
struct WireHourOutcome {
    opened_baseline: bool,
    /// The baseline was opened by a gap rather than the chain length. The two
    /// reasons are counted apart: reaching the 48-hour chain length is the
    /// cadence, a gap is the spec's forced baseline.
    gap_forced: bool,
    emitted: bool,
    bytes: usize,
    changed_rows: usize,
    tombstones: usize,
}

fn encode_wire_hour(
    schema: &Schema,
    hours_in_block: usize,
    previous_measured_hour: Option<i32>,
    hour: &D95Hour,
    prior: &d95::SpottedUnitsState,
    reader: &mut Vec<KeyedState>,
    wire: &mut Vec<u8>,
) -> WireHourOutcome {
    let state = &hour.state;
    let hour_start = wire.len();
    let block_start = opens_block(hours_in_block, previous_measured_hour, hour.game_hour);
    let gap_forced = block_start
        && previous_measured_hour
            .is_some_and(|previous| hour.game_hour != previous.saturating_add(1));
    let mut emitted = true;
    let mut changed_rows = 0usize;
    let mut tombstones = 0usize;
    if block_start {
        encode_baseline(schema, &state.as_slice(), wire).expect("baseline encodes");
    } else {
        let deltas: Vec<Option<KeyedDelta>> = (0..d95::COLLECTION_COUNT)
            .map(|i| {
                let d = diff(&schema.collections[i], prior.get(i), state.get(i));
                (!d.is_empty()).then_some(d)
            })
            .collect();
        if deltas.iter().all(Option::is_none) {
            emitted = false;
        } else {
            for (i, delta) in deltas.iter().enumerate() {
                let Some(delta) = delta else { continue };
                changed_rows += delta.changed.len();
                tombstones += delta.tombstones.len();
                // §7.1 minimality, checked rather than assumed.
                check_minimal(&schema.collections[i], prior.get(i), state.get(i), delta)
                    .unwrap_or_else(|e| panic!("hour {}: {e:?}", hour.game_hour));
            }
            encode_delta(schema, &prior.as_slice(), &deltas, wire).expect("delta encodes");
        }
    }
    let bytes = wire.len() - hour_start;

    // Reader replay: decode the bytes just written against the reader's OWN
    // reconstructed prior, exactly as a conforming reader would, and require
    // the result to be stored-equal to this hour's state. This is what makes
    // the size claim a claim about decodable bytes. An hour that emitted
    // nothing is still checked: the reader's retained state must already
    // equal the fold's, which is precisely what "nothing changed" asserts.
    if emitted {
        let hour_bytes = &wire[hour_start..];
        if block_start {
            let (states, consumed) =
                decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
            assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
            *reader = states;
        } else {
            let (deltas, consumed) =
                decode_delta(schema, reader, hour_bytes, 0).expect("delta decodes");
            assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
            for (i, delta) in deltas.iter().enumerate() {
                if let Some(delta) = delta {
                    apply(&mut reader[i], delta);
                }
            }
        }
    }
    for i in 0..d95::COLLECTION_COUNT {
        assert_eq!(
            reader[i].len(),
            state.get(i).len(),
            "game hour {}: collection {} row count",
            hour.game_hour,
            schema.collections[i].collection_id
        );
        assert!(
            diff(&schema.collections[i], &reader[i], state.get(i)).is_empty(),
            "game hour {}: collection {} is not stored-equal to the fold",
            hour.game_hour,
            schema.collections[i].collection_id
        );
    }
    WireHourOutcome {
        opened_baseline: block_start,
        gap_forced,
        emitted,
        bytes,
        changed_rows,
        tombstones,
    }
}

/// Split the shipping arm's bytes into the payload slice this port's wire arm
/// replaces and the per-record H4CR framing no per-collection port can claim.
///
/// `ship_records` must be the number of records **actually concatenated into the
/// shipping arm**, not every d95 record in the stream: an excluded hour's records
/// are charged to neither arm, so counting them here would subtract framing the
/// arm does not contain — underflowing in debug and reporting a huge false
/// payload size in release.
fn shipping_split(ship_bytes: usize, ship_records: usize) -> (usize, usize) {
    let framing = ship_records.saturating_mul(COMPACT_RECORD_HEADER_LEN);
    (ship_bytes.saturating_sub(framing), framing)
}

/// The three refusals the fold answers by clearing the hour and continuing. A
/// refusal that is not one of them is a malformed payload: not this grammar, so
/// the caller turns it into a stream rejection.
///
/// Matching on the fold's own message text keeps one authority for the
/// classification: the strings are `d95::HourAccumulator`'s, which are
/// `fold_pov_spotted_units`' (`world_fold.rs:10544`, `:10549`, `:10557`).
fn classify_refusal(reason: &str) -> Option<&'static str> {
    if reason.contains("requires a current-hour D18 identity roster") {
        Some("no_roster_this_hour")
    } else if reason.contains("is absent from current D18 roster") {
        Some("pov_absent_from_roster")
    } else if reason.contains("sequence is not strictly increasing") {
        Some("sequence_not_increasing")
    } else {
        None
    }
}

/// Close one game hour: either it stored a d95 state, or it is accounted for as
/// an exclusion.
fn commit_hour(
    scan: &mut StreamScan,
    schema: &Schema,
    game_hour: i32,
    hour_fold: &HourAccumulator,
    ship_spans: &mut Vec<Range<usize>>,
    excluded: Option<&'static str>,
    last_measured_hour: &mut Option<i32>,
) -> Result<(), String> {
    let spans = std::mem::take(ship_spans);
    if spans.is_empty() {
        // COMPLETENESS: every closed game hour of a d95-scheduled capture
        // carries at least one d95 DATA record (several are legal — the
        // within-hour sequence orders them — and the reference capture carries
        // exactly one per hour, 1,033/1,033). This bounds what this harness
        // canNOT validate: production runs `validate_domain_data_payload_at` on
        // EVERY domain's DATA payload (`stream_fold.rs:593`), while this
        // harness owns only d95's grammar and the d18 mirror, so a d95 record
        // relabelled to another domain id — or lost outright — would otherwise
        // silently drop its hour from the measurement. A per-capture
        // completeness assertion, NOT a grammar rule: a capture where d95 is
        // not scheduled every hour fails here loudly and a human adjusts,
        // instead of a number quietly shrinking (`bench_real_d65.rs`
        // precedent).
        return Err(format!(
            "game hour {game_hour} closed without any d95 DATA record; every closed hour of \
             a d95-scheduled capture must carry at least one, and this harness cannot \
             validate the payloads of unmodelled domains to see where it went"
        ));
    }
    let Some(state) = hour_fold
        .state(schema)
        .map_err(|e| format!("d95 hour {game_hour}: {e}"))?
    else {
        match excluded {
            Some("no_roster_this_hour") => scan.excluded.no_roster_this_hour += 1,
            Some("pov_absent_from_roster") => scan.excluded.pov_absent_from_roster += 1,
            Some("sequence_not_increasing") => scan.excluded.sequence_not_increasing += 1,
            // Unreachable: an hour with d95 records and no stored state stored
            // nothing for one of the three reasons above, all of which set the
            // marker. Failing loudly beats measuring an hour of unknown status.
            _ => {
                return Err(format!(
                    "d95 hour {game_hour} stored no state and no refusal was recorded"
                ))
            }
        }
        return Ok(());
    };
    let stored = hour_fold.stored().expect("a state implies a stored snapshot");
    if let Some(previous) = *last_measured_hour {
        if game_hour != previous + 1 {
            scan.non_contiguous += 1;
        }
    }
    *last_measured_hour = Some(game_hour);
    scan.pov_countries.insert(stored.header.pov_country_id);
    scan.land_max = scan.land_max.max(state.land.len());
    scan.taskforce_max = scan.taskforce_max.max(state.taskforces.len());
    scan.hours.push(D95Hour {
        game_hour,
        ship_spans: spans,
        land_unmeasured: stored.header.frame_sequence(Surface::Land).is_none(),
        taskforce_unmeasured: stored.header.frame_sequence(Surface::Taskforce).is_none(),
        state,
    });
    Ok(())
}

fn main() {
    const USAGE: &str = "usage: bench_real_d95 <collector_stream.bin> <out dir> \
                         [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d95::schema();

    // ---- Validate the WHOLE stream and fold every d95 hour. No artifact is
    // created before this succeeds, so a refused stream leaves no partial
    // measurement behind to be mistaken for a result.
    let scan = match scan_stream(&stream, &schema, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };

    // A well-formed capture in which every d95 hour was excluded is a real
    // outcome, not a corrupt stream (see `scan_stream`'s EOF note) — but it is
    // also not a measurement: there is no hour to encode and the size ratio has
    // no denominator. Report what happened and write nothing, so an empty arm
    // cannot be mistaken for a result.
    if scan.hours.is_empty() {
        eprintln!(
            "{input}: {} d95 DATA record(s), every hour excluded (no d18 roster {}, POV \
             absent {}, sequence not increasing {}); nothing to measure",
            scan.d95_records,
            scan.excluded.no_roster_this_hour,
            scan.excluded.pov_absent_from_roster,
            scan.excluded.sequence_not_increasing
        );
        std::process::exit(3);
    }
    std::fs::create_dir_all(&out_dir).expect("out dir");

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut prior = d95::SpottedUnitsState::new();
    let mut reader: Vec<KeyedState> = vec![KeyedState::new(); d95::COLLECTION_COUNT];
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut delta_changed_rows = 0usize;
    let mut delta_tombstones = 0usize;
    // Cadence-independent churn: changed rows over measured-hour transitions,
    // counted before the block decision so a change landing on a block boundary
    // still counts.
    let mut transition_changed_rows = 0usize;
    let mut transition_tombstones = 0usize;
    let mut transitions = 0usize;
    let mut rows_total = 0usize;
    let mut land_unmeasured_hours = 0usize;
    let mut taskforce_unmeasured_hours = 0usize;
    // The 48-hour block chain, and the previous MEASURED hour it is chained
    // through: an excluded hour is a state-affecting gap, and the spec forces the
    // next state observation to be a baseline (see [`opens_block`]).
    let mut hours_in_block = 0usize;
    let mut previous_measured_hour: Option<i32> = None;
    let mut gap_baselines = 0usize;

    let mut ship_records = 0usize;
    for hour in &scan.hours {
        for span in &hour.ship_spans {
            ship.extend_from_slice(&stream[span.clone()]);
            ship_records += 1;
        }
        ship_offsets.push(ship.len());

        let state = &hour.state;
        rows_total += state.rows();
        land_unmeasured_hours += usize::from(hour.land_unmeasured);
        taskforce_unmeasured_hours += usize::from(hour.taskforce_unmeasured);

        if hours_seen > 0 {
            transitions += 1;
            for i in 0..d95::COLLECTION_COUNT {
                let d = diff(&schema.collections[i], prior.get(i), state.get(i));
                transition_changed_rows += d.changed.len();
                transition_tombstones += d.tombstones.len();
            }
        }

        // The wire-arm hour — block/gap baseline decision, encode and reader
        // replay — through the SAME function the tests execute
        // (`encode_wire_hour`).
        let outcome = encode_wire_hour(
            &schema,
            hours_in_block,
            previous_measured_hour,
            hour,
            &prior,
            &mut reader,
            &mut wire,
        );
        if outcome.opened_baseline {
            if outcome.gap_forced {
                gap_baselines += 1;
            }
            hours_in_block = 0;
            baselines += 1;
            baseline_bytes += outcome.bytes;
        } else if outcome.emitted {
            delta_changed_rows += outcome.changed_rows;
            delta_tombstones += outcome.tombstones;
            delta_bytes += outcome.bytes;
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        prior = state.clone();
        hours_seen += 1;
        hours_in_block += 1;
        previous_measured_hour = Some(hour.game_hour);
    }

    for (name, bytes) in [("ship_d95.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d95.offsets", &ship_offsets),
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
        "d95 DATA records     {} over {hours_seen} measured hours",
        scan.d95_records
    );
    println!(
        "excluded hours       {} (no d18 roster {}, POV absent {}, sequence not increasing {})",
        scan.excluded.total(),
        scan.excluded.no_roster_this_hour,
        scan.excluded.pov_absent_from_roster,
        scan.excluded.sequence_not_increasing
    );
    println!(
        "d18 records          {} over the capture (all-or-nothing bound held)",
        scan.d18_records
    );
    println!(
        "d18 dependency       {} failure-status records, {} refused payloads (each commits \
         nothing; the hour is unmeasured only if no d18 record succeeded in it), {} records \
         with unassigned v1 status bits (committed by the recorder, so committed here)",
        scan.d18_failure_records, scan.d18_refused_records, scan.d18_unassigned_status_records
    );
    println!(
        "d18 certification    {} failure-status record(s) whose body the payload authority \
         would reject (the fold never reads it; a named deviation, not a reclassification){}",
        scan.d18_failure_records_with_unwalkable_body,
        scan.d18_first_failure_body_deviation
            .as_deref()
            .map(|first| format!(" (first: {first})"))
            .unwrap_or_default()
    );
    println!(
        "perf status          {} (parsed and reported; the fold reads no status word)",
        scan.perf_status
            .iter()
            .map(|(status, count)| format!("{status:#x}x{count}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "POV countries        {:?}",
        scan.pov_countries.iter().collect::<Vec<_>>()
    );
    println!(
        "unmeasured surfaces  land {land_unmeasured_hours}/{hours_seen} h, taskforce \
         {taskforce_unmeasured_hours}/{hours_seen} h (flag clear = not captured)"
    );
    println!(
        "rows avg             {:.2} over all three collections (max {} land, {} taskforce)",
        rows_total as f64 / hours_seen as f64,
        scan.land_max,
        scan.taskforce_max
    );
    println!(
        "baselines            {baselines} (every {BLOCK_HOURS} h, plus {gap_baselines} forced \
         by a gap in measured hours)"
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "churn cadence-indep  {:.2} changed rows/transition, {:.3} tombstones/transition over \
         {transitions} measured-hour transitions",
        transition_changed_rows as f64 / transitions.max(1) as f64,
        transition_tombstones as f64 / transitions.max(1) as f64
    );
    println!(
        "churn delta path     {:.2} changed rows/hour, {:.3} tombstones/hour over \
         {delta_hours} emitted-delta hours (stored precision)",
        delta_changed_rows as f64 / delta_hours.max(1) as f64,
        delta_tombstones as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d95.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.2}%)  baseline {} B, delta {} B",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64,
        baseline_bytes,
        delta_bytes
    );
    let (payload_bytes, framing_bytes) = shipping_split(ship.len(), ship_records);
    println!(
        "shipping split       {payload_bytes} B payload (claimed by the wire arm), \
         {framing_bytes} B H4CR framing over {ship_records} measured records (per-record, \
         claimable by no per-collection port)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::spotted_units::{SpottedUnitHandle, SpottedUnitsSnapshot};
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_STATUS_STATE_UNAVAILABLE};

    fn test_schema() -> Schema {
        d95::schema()
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

    fn frame_end_sealed(
        sequence: u64,
        game_hour: i32,
        emitted: u32,
        scheduled: u32,
        status: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        payload[16..20].copy_from_slice(&emitted.to_le_bytes());
        payload[20..24].copy_from_slice(&scheduled.to_le_bytes());
        payload[24..28].copy_from_slice(&status.to_le_bytes());
        payload[28..32].copy_from_slice(&reserved.to_le_bytes());
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    fn perf(sequence: u64, game_hour: i32, domain: u16, status: u32) -> Vec<u8> {
        let mut payload = vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN];
        payload[32..36].copy_from_slice(&status.to_le_bytes());
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &payload,
        )
    }

    /// A d18 payload carrying `tags`, built the way `fold_country_identity` reads
    /// one (`world_fold.rs:13918`-`:13977`). No CIM1 extension unless asked for.
    fn d18_payload(status: u32, tags: &[(u32, &str)], cim1: Option<Vec<u8>>) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(tags.len() as u32).to_le_bytes()); // row_count
        p.extend_from_slice(&(tags.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status.to_le_bytes());
        for (id, tag) in tags {
            p.extend_from_slice(&id.to_le_bytes());
            p.extend_from_slice(&(tag.len() as u16).to_le_bytes());
            p.extend_from_slice(tag.as_bytes());
            p.push(0); // cosmetic absent
        }
        if let Some(cim1) = cim1 {
            p.extend_from_slice(&cim1);
        }
        p
    }

    /// A minimal well-formed CIM1 block for `count` countries: no states, every
    /// country a major row (`world_fold.rs:13985`-`:14141`).
    fn cim1(majors: &[u32]) -> Vec<u8> {
        let mut c = Vec::new();
        c.extend_from_slice(b"CIM1");
        c.extend_from_slice(&1u16.to_le_bytes());
        c.extend_from_slice(&40u16.to_le_bytes());
        c.extend_from_slice(&0u32.to_le_bytes()); // claims status
        c.extend_from_slice(&0u32.to_le_bytes()); // cores status
        c.extend_from_slice(&0u32.to_le_bytes()); // major status
        c.extend_from_slice(&0u32.to_le_bytes()); // state_count
        c.extend_from_slice(&0u32.to_le_bytes()); // claim memberships
        c.extend_from_slice(&0u32.to_le_bytes()); // core memberships
        c.extend_from_slice(&(majors.len() as u32).to_le_bytes());
        c.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(c.len(), 40);
        for id in majors {
            c.extend_from_slice(&id.to_le_bytes());
            c.push(1);
            c.extend_from_slice(&[0u8; 3]);
        }
        c
    }

    /// A d95 payload built through the SHIPPING codec, so the harness is measured
    /// against the bytes the recorder actually produces.
    fn d95_payload(
        capture_sequence: u64,
        pov: u32,
        land: (Option<u64>, &[(u32, u32, u8)]),
        taskforces: (Option<u64>, &[(u32, u32, u8)]),
    ) -> Vec<u8> {
        let rows = |src: &[(u32, u32, u8)]| {
            src.iter()
                .map(|(type_id, stable_id, raw_intel)| SpottedUnitHandle {
                    type_id: *type_id,
                    stable_id: *stable_id,
                    raw_intel: *raw_intel,
                })
                .collect::<Vec<_>>()
        };
        SpottedUnitsSnapshot {
            capture_sequence,
            pov_country_id: pov,
            land_frame_sequence: land.0,
            taskforce_frame_sequence: taskforces.0,
            land: rows(land.1),
            taskforces: rows(taskforces.1),
        }
        .encode()
        .expect("the codec encodes this snapshot")
    }

    fn d18_record(sequence: u64, game_hour: i32) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(0, &[(1, "GER"), (368, "D04")], None),
        )
    }

    /// A d18 record whose failure status commits nothing — the hour then has a
    /// d18 record (the completeness bound holds) but no roster.
    fn d18_failure_record(sequence: u64, game_hour: i32) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(
                COMPACT_STATUS_STATE_UNAVAILABLE,
                &[(1, "GER"), (368, "D04")],
                None,
            ),
        )
    }

    fn d95_record(sequence: u64, game_hour: i32, capture_sequence: u64) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            D95,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d95_payload(
                capture_sequence,
                368,
                (Some(capture_sequence - 1), &[(3, 11, 250)]),
                (Some(capture_sequence - 2), &[]),
            ),
        )
    }

    /// Two closed hours, each carrying a d18 roster and a d95 record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(d18_record(1, 100));
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        s.extend(d18_record(4, 101));
        s.extend(d95_record(5, 101, 20));
        s.extend(frame_end(6, 101));
        s
    }

    fn run_scan(stream: &[u8]) -> Result<StreamScan, String> {
        scan_stream(stream, &test_schema(), StreamIdentityMode::Legacy)
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
    fn a_well_formed_stream_yields_its_d95_hours() {
        let scan = run_scan(&good_stream()).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.d95_records, 2);
        assert_eq!(scan.excluded.total(), 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.land_max, 1);
        assert_eq!(scan.taskforce_max, 0);
        assert_eq!(scan.pov_countries, BTreeSet::from([368]));
        for hour in &scan.hours {
            assert_eq!(hour.state.capture.len(), 1);
            assert!(!hour.land_unmeasured && !hour.taskforce_unmeasured);
        }
    }

    #[test]
    fn the_harness_agrees_with_the_shipping_codec_on_what_a_payload_is() {
        // The differential check the recipe's "one derivation" rule is about: the
        // port's boundary and `compact_contract`'s own decoder must accept and
        // reject the same bytes, and agree field for field on what they carry.
        // Corpus: every structural case the codec distinguishes, plus mutations.
        let mut corpus: Vec<Vec<u8>> = vec![
            d95_payload(1, 1, (Some(1), &[]), (None, &[])),
            d95_payload(9, 368, (Some(1), &[(3, 11, 250)]), (Some(2), &[(4, 1, 150)])),
            d95_payload(
                9,
                2048,
                (Some(7), &[(1, 1, 255), (1, 2, 250), (2, 1, 251)]),
                (None, &[]),
            ),
            d95_payload(u64::MAX, 368, (None, &[]), (Some(u64::MAX), &[(9, 9, 255)])),
        ];
        let mutations = corpus.clone();
        for base in mutations {
            for index in 0..base.len().min(64) {
                for patch in [0x00u8, 0x01, 0xff] {
                    let mut m = base.clone();
                    m[index] = patch;
                    corpus.push(m);
                }
            }
            let mut short = base.clone();
            short.pop();
            corpus.push(short);
            let mut long = base.clone();
            long.push(0);
            corpus.push(long);
        }
        corpus.push(Vec::new());
        corpus.push(vec![0u8; 43]);

        let schema = test_schema();
        let roster = CountryRoster::measured(1..=2048);
        for payload in corpus {
            let codec = SpottedUnitsSnapshot::decode(&payload);
            let mut hour = HourAccumulator::new();
            let port = hour.fold_payload(&payload, &roster);
            assert_eq!(
                codec.is_ok(),
                port.is_ok(),
                "codec and port disagree on payload {payload:02x?}: {codec:?} vs {port:?}"
            );
            let Ok(snapshot) = codec else { continue };
            let stored = hour.stored().expect("a folded payload stores a snapshot");
            assert_eq!(stored.header.capture_sequence, snapshot.capture_sequence);
            assert_eq!(stored.header.pov_country_id, snapshot.pov_country_id);
            assert_eq!(
                stored.header.land_frame_sequence,
                snapshot.land_frame_sequence
            );
            assert_eq!(
                stored.header.taskforce_frame_sequence,
                snapshot.taskforce_frame_sequence
            );
            let state = hour.state(&schema).expect("builds").expect("stored");
            assert_eq!(state.land.len(), snapshot.land.len());
            assert_eq!(state.taskforces.len(), snapshot.taskforces.len());
            for (surface, rows, expected) in [
                (Surface::Land, &state.land, &snapshot.land),
                (Surface::Taskforce, &state.taskforces, &snapshot.taskforces),
            ] {
                for (row, handle) in rows.iter().zip(expected.iter()) {
                    let (key, record) = row;
                    assert_eq!(
                        key.0,
                        vec![
                            snapshot.pov_country_id as i128,
                            handle.type_id as i128,
                            handle.stable_id as i128
                        ],
                        "{} key",
                        surface.name()
                    );
                    assert_eq!(
                        record.values[0],
                        wire_format::Value::Unsigned(handle.raw_intel as u64),
                        "{} raw_intel",
                        surface.name()
                    );
                }
            }
        }
    }

    #[test]
    fn an_unmeasured_surface_is_carried_through_the_scan() {
        // The centrepiece on a stream: the hour whose task-force frame is missing
        // is measured, not excluded, and the fact is visible in the scan.
        let mut s = Vec::new();
        s.extend(d18_record(1, 100));
        s.extend(record(
            2,
            100,
            D95,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d95_payload(10, 368, (Some(9), &[(3, 11, 250)]), (None, &[])),
        ));
        s.extend(frame_end(3, 100));
        s.extend(d18_record(4, 101));
        s.extend(record(
            5,
            101,
            D95,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d95_payload(20, 368, (None, &[]), (Some(19), &[(4, 1, 150)])),
        ));
        s.extend(frame_end(6, 101));
        let scan = run_scan(&s).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert!(!scan.hours[0].land_unmeasured && scan.hours[0].taskforce_unmeasured);
        assert!(scan.hours[1].land_unmeasured && !scan.hours[1].taskforce_unmeasured);
        // And the row collection is empty in both directions, which is exactly
        // why the frame record must carry the answer.
        assert!(scan.hours[0].state.taskforces.is_empty());
        assert!(scan.hours[1].state.land.is_empty());
    }

    #[test]
    fn an_hour_without_a_d18_roster_is_excluded_not_measured() {
        // `world_fold.rs:10542`-`:10544`: the whole capture can be in this state
        // (batch-3 Finding 3), so it is an excluded hour, never a stream
        // rejection. The hour still CARRIES a d18 record — a failure-status one,
        // which commits nothing — so the completeness bound holds and the
        // no-roster path is reached the way production reaches it on a complete
        // capture.
        let mut s = Vec::new();
        s.extend(d18_failure_record(1, 100));
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        s.extend(d18_record(4, 101));
        s.extend(d95_record(5, 101, 20));
        s.extend(frame_end(6, 101));
        let scan = run_scan(&s).expect("the stream itself is well-formed");
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 101);
        assert_eq!(scan.excluded.no_roster_this_hour, 1);
        assert_eq!(scan.d95_records, 2);

        // A d18 roster committed in an EARLIER hour does not carry over: the
        // known flag resets at the boundary while the id map persists
        // (`:2999`-`:3002`). Hour 101's d18 record is a failure-status one, so
        // nothing commits in that hour and its d95 record is excluded.
        let mut stale = Vec::new();
        stale.extend(d18_record(1, 100));
        stale.extend(d95_record(2, 100, 10));
        stale.extend(frame_end(3, 100));
        stale.extend(d18_failure_record(4, 101));
        stale.extend(d95_record(5, 101, 20));
        stale.extend(frame_end(6, 101));
        let scan = run_scan(&stale).expect("well-formed");
        assert_eq!(scan.hours.len(), 1, "hour 101 has no roster of its own");
        assert_eq!(scan.excluded.no_roster_this_hour, 1);
    }

    #[test]
    fn a_pov_outside_the_roster_is_excluded() {
        // `world_fold.rs:10546`-`:10551`.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(0, &[(1, "GER")], None),
        ));
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        s.extend(d18_record(4, 101));
        s.extend(d95_record(5, 101, 20));
        s.extend(frame_end(6, 101));
        let scan = run_scan(&s).expect("well-formed");
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.excluded.pov_absent_from_roster, 1);
    }

    #[test]
    fn a_d18_record_the_recorder_refuses_commits_nothing() {
        // A failure status (`:13922`), a malformed payload and a CIM1 block the
        // fold rejects all commit nothing, so where they are the hour's ONLY d18
        // record the hour has no roster and the d95 hour behind them is excluded
        // rather than measured — and none of the
        // three refuses the stream, because d18 is not the domain being ported.
        let cases: [(&str, Vec<u8>); 3] = [
            (
                "failure status",
                d18_payload(COMPACT_STATUS_STATE_UNAVAILABLE, &[(368, "D04")], None),
            ),
            ("zero tag_db_id", d18_payload(0, &[(0, "D04")], None)),
            (
                "CIM1 major roster incomplete",
                d18_payload(0, &[(1, "GER"), (368, "D04")], Some(cim1(&[1]))),
            ),
        ];
        for (label, payload) in cases {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(d95_record(2, 100, 10));
            s.extend(frame_end(3, 100));
            s.extend(d18_record(4, 101));
            s.extend(d95_record(5, 101, 20));
            s.extend(frame_end(6, 101));
            let scan = run_scan(&s).unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(scan.hours.len(), 1, "{label}");
            assert_eq!(scan.excluded.no_roster_this_hour, 1, "{label}");
        }

        // A well-formed CIM1 block that the fold accepts does commit the roster.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(0, &[(1, "GER"), (368, "D04")], Some(cim1(&[1, 368]))),
        ));
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        let scan = run_scan(&s).expect("well-formed");
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.d18_failure_records + scan.d18_refused_records, 0);

        // Round-2 finding, as a property rather than a comment: a refused d18
        // record AFTER a successful one in the same hour changes nothing, because
        // the fold commits at the end and never un-commits (`:14149`-`:14156`).
        // The hour stays measured.
        for (label, second) in [
            (
                "failure status",
                d18_payload(COMPACT_STATUS_STATE_UNAVAILABLE, &[(368, "D04")], None),
            ),
            ("zero tag_db_id", d18_payload(0, &[(0, "D04")], None)),
        ] {
            let mut s = Vec::new();
            s.extend(d18_record(1, 100));
            s.extend(record(2, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &second));
            s.extend(d95_record(3, 100, 10));
            s.extend(frame_end(4, 100));
            let scan = run_scan(&s).unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(
                scan.hours.len(),
                1,
                "{label}: the earlier commit stands, so the hour is measured"
            );
            assert_eq!(scan.excluded.total(), 0, "{label}");
        }
    }

    #[test]
    fn several_d95_records_in_one_hour_fold_into_one_state() {
        // The fold admits them and keeps the last strictly-increasing one
        // (`world_fold.rs:10553`-`:10559`), so the harness must not enforce
        // one-record-per-hour: that would refuse a stream the recorder folds.
        let mut s = Vec::new();
        s.extend(d18_record(1, 100));
        s.extend(record(
            2,
            100,
            D95,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d95_payload(10, 368, (Some(9), &[(3, 11, 250)]), (None, &[])),
        ));
        s.extend(record(
            3,
            100,
            D95,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d95_payload(11, 368, (Some(10), &[(3, 11, 251), (4, 1, 250)]), (None, &[])),
        ));
        s.extend(frame_end(4, 100));
        let scan = run_scan(&s).expect("well-formed");
        assert_eq!(scan.hours.len(), 1, "two records, one measured hour");
        assert_eq!(scan.d95_records, 2);
        assert_eq!(scan.hours[0].state.land.len(), 2, "the later record wins");
        assert_eq!(scan.hours[0].ship_spans.len(), 2, "both are charged to ship");

        // A non-increasing capture sequence clears the stored snapshot: with no
        // later record in the hour, the hour is excluded and the earlier state is
        // NOT measured in its place.
        for second in [10u64, 9] {
            let mut s = Vec::new();
            s.extend(d18_record(1, 100));
            s.extend(record(
                2,
                100,
                D95,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d95_payload(10, 368, (Some(9), &[(3, 11, 250)]), (None, &[])),
            ));
            s.extend(record(
                3,
                100,
                D95,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d95_payload(second, 368, (Some(8), &[]), (None, &[])),
            ));
            s.extend(frame_end(4, 100));
            s.extend(d18_record(5, 101));
            s.extend(d95_record(6, 101, 20));
            s.extend(frame_end(7, 101));
            let scan = run_scan(&s).expect("well-formed");
            assert_eq!(scan.excluded.sequence_not_increasing, 1, "second {second}");
            assert_eq!(scan.hours.len(), 1, "second {second}");
            assert_eq!(scan.hours[0].game_hour, 101, "second {second}");
        }

        // Round-4 finding, as a property: clearing is not ending the hour. The
        // reviewer's own input — sequences 10, 10, then 1 — leaves the hour
        // MEASURED from the third record, because a cleared accumulator has no
        // sequence to compare against (`world_fold.rs:10553`, `:10559`).
        let mut s = Vec::new();
        s.extend(d18_record(1, 100));
        for (sequence, seq_no) in [(10u64, 2u64), (10, 3), (1, 4)] {
            s.extend(record(
                seq_no,
                100,
                D95,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d95_payload(sequence, 368, (Some(9), &[(3, 11, 250)]), (None, &[])),
            ));
        }
        s.extend(frame_end(5, 100));
        let scan = run_scan(&s).expect("well-formed");
        assert_eq!(scan.hours.len(), 1, "the third record stores a snapshot");
        assert_eq!(scan.excluded.total(), 0, "so the hour is not excluded");
        assert_eq!(scan.hours[0].state.land.len(), 1);
        assert_eq!(scan.hours[0].ship_spans.len(), 3, "all three are charged");
    }

    #[test]
    fn a_gap_in_measured_hours_forces_a_baseline() {
        // Round-5 finding. An excluded hour is a state-affecting gap, and
        // `WIRE_FORMAT.md:187`-`:190` forces the next state observation to be a
        // BASELINE: a delta across unobserved time asserts a continuity the
        // capture does not have, and the local replay cannot see it because the
        // reader's prior is never invalidated.
        assert!(opens_block(0, None, 100), "the first hour is a baseline");
        assert!(
            !opens_block(1, Some(100), 101),
            "a contiguous hour inside the block is a delta"
        );
        assert!(
            opens_block(1, Some(100), 102),
            "one excluded hour between them forces a baseline"
        );
        assert!(
            opens_block(1, Some(100), 100),
            "a non-advancing hour is not contiguity either"
        );
        assert!(
            opens_block(BLOCK_HOURS, Some(100), 101),
            "and the {BLOCK_HOURS}-hour chain still opens a block on its own"
        );
        assert!(
            !opens_block(BLOCK_HOURS - 1, Some(100), 101),
            "the hour before the chain ends is still a delta"
        );
        // The saturating edge: a previous hour at i32::MAX cannot be followed
        // contiguously, so it opens a block rather than overflowing.
        assert!(opens_block(1, Some(i32::MAX), i32::MIN));

        // And through the PRODUCTION encode path, not just the decision
        // function: scan a stream whose middle hour's d95 record is excluded
        // (no d18 roster committed that hour — a failure-status d18 record),
        // then drive the SAME `encode_wire_hour` main uses and require the
        // post-gap hour to be a gap-forced baseline, not a delta.
        let mut s = d18_record(1, 100);
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        s.extend(d18_failure_record(4, 101));
        s.extend(d95_record(5, 101, 20)); // excluded: no roster this hour
        s.extend(frame_end(6, 101));
        s.extend(d18_record(7, 102));
        s.extend(d95_record(8, 102, 30));
        s.extend(frame_end(9, 102));
        let scan = run_scan(&s).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.excluded.no_roster_this_hour, 1);
        let schema = test_schema();
        let mut wire = Vec::new();
        let mut reader: Vec<KeyedState> = vec![KeyedState::new(); d95::COLLECTION_COUNT];
        let empty_prior = d95::SpottedUnitsState::new();
        let o0 = encode_wire_hour(
            &schema,
            0,
            None,
            &scan.hours[0],
            &empty_prior,
            &mut reader,
            &mut wire,
        );
        assert!(o0.opened_baseline && !o0.gap_forced);
        let o1 = encode_wire_hour(
            &schema,
            1,
            Some(scan.hours[0].game_hour),
            &scan.hours[1],
            &scan.hours[0].state,
            &mut reader,
            &mut wire,
        );
        assert!(
            o1.opened_baseline && o1.gap_forced,
            "the post-gap hour must be a gap-forced baseline, not a delta"
        );
    }

    #[test]
    fn a_capture_whose_every_hour_is_excluded_is_reported_not_refused() {
        // Round-4 finding. A capture that never schedules d18 folds to an error on
        // every d95 hour (batch-3's 3b stream is exactly that), and the recorder
        // clears each domain-hour and continues — so the scan must return those
        // exclusions rather than calling a legitimate capture corrupt. `main`
        // then reports and writes no artifact, because there is no hour to encode.
        let mut s = Vec::new();
        s.extend(d95_record(1, 100, 10));
        s.extend(frame_end(2, 100));
        s.extend(d95_record(3, 101, 20));
        s.extend(frame_end(4, 101));
        let scan = run_scan(&s).expect("a d18-less capture is well formed, not corrupt");
        assert!(scan.hours.is_empty(), "no hour is measurable");
        assert_eq!(scan.d95_records, 2);
        assert_eq!(scan.excluded.no_roster_this_hour, 2);
        assert_eq!(scan.excluded.total(), 2);

        // And the distinction the check now makes: a stream with NO d95 records at
        // all is still refused, because it is not a d95 capture.
        let mut none = d18_record(1, 100);
        none.extend(frame_end(2, 100));
        let err = match run_scan(&none) {
            Ok(_) => panic!("a stream with no d95 records must be refused"),
            Err(err) => err,
        };
        assert!(err.contains("d95 DATA record"), "{err}");
    }

    #[test]
    fn a_malformed_d95_payload_is_refused_not_skipped() {
        // Not this grammar: a stream rejection, the same treatment d83 gives a
        // magic or trailing-byte failure. The whole rejection space of the codec,
        // not one case.
        let base = d95_payload(10, 368, (Some(9), &[(3, 11, 250)]), (Some(8), &[(4, 1, 150)]));
        let mut invalid: Vec<(&str, Vec<u8>)> = Vec::new();
        let mut magic = base.clone();
        magic[0] = b'X';
        invalid.push(("magic", magic));
        let mut version = base.clone();
        version[4..6].copy_from_slice(&2u16.to_le_bytes());
        invalid.push(("version", version));
        let mut flags = base.clone();
        flags[6..8].copy_from_slice(&0x8003u16.to_le_bytes());
        invalid.push(("unknown flag bit", flags));
        let mut capture = base.clone();
        capture[8..16].copy_from_slice(&0u64.to_le_bytes());
        invalid.push(("zero capture sequence", capture));
        let mut pov = base.clone();
        pov[16..20].copy_from_slice(&0u32.to_le_bytes());
        invalid.push(("zero POV", pov));
        let mut pov_high = base.clone();
        pov_high[16..20].copy_from_slice(&(d95::MAX_POV_COUNTRY_ID + 1).to_le_bytes());
        invalid.push(("POV above the ceiling", pov_high));
        let mut counts = base.clone();
        counts[20..24].copy_from_slice(&5u32.to_le_bytes());
        invalid.push(("row count disagrees with length", counts));
        let mut intel = base.clone();
        intel[44 + 8] = 249;
        invalid.push(("land intel below 250", intel));
        let mut tf_intel = base.clone();
        tf_intel[44 + 12 + 8] = 149;
        invalid.push(("taskforce intel below 150", tf_intel));
        let mut reserved = base.clone();
        reserved[44 + 9] = 1;
        invalid.push(("non-zero row reserved byte", reserved));
        let mut zero_type = base.clone();
        zero_type[44..48].copy_from_slice(&0u32.to_le_bytes());
        invalid.push(("zero type_id", zero_type));
        let mut truncated = base.clone();
        truncated.pop();
        invalid.push(("truncated", truncated));
        let mut trailing = base.clone();
        trailing.push(0);
        invalid.push(("trailing byte", trailing));
        // Two land rows out of order, which the codec refuses and a keyed set
        // could otherwise silently reorder.
        let unsorted = {
            let mut p = d95_payload(10, 368, (Some(9), &[(3, 11, 250), (4, 1, 250)]), (None, &[]));
            let first = p[44..56].to_vec();
            let second = p[56..68].to_vec();
            p[44..56].copy_from_slice(&second);
            p[56..68].copy_from_slice(&first);
            p
        };
        invalid.push(("unsorted rows", unsorted));

        for (label, payload) in invalid {
            let mut s = Vec::new();
            s.extend(d18_record(1, 100));
            s.extend(record(
                2,
                100,
                D95,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(3, 100));
            assert!(
                run_scan(&s).is_err(),
                "{label} must refuse the stream, not be skipped"
            );
        }
    }

    #[test]
    fn the_perf_status_word_is_reported_and_never_enforced() {
        // d95's fold reads no status word (`world_fold.rs:10538`-`:10561`), so a
        // failure status on the domain's PERF record must not exclude the hour —
        // the recorder folds the payload regardless.
        let mut s = Vec::new();
        s.extend(perf(1, 100, D95, COMPACT_STATUS_STATE_UNAVAILABLE));
        s.extend(d18_record(2, 100));
        s.extend(d95_record(3, 100, 10));
        s.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        let scan = run_scan(&s).expect("well-formed");
        assert_eq!(scan.hours.len(), 1, "the hour is measured, not excluded");
        assert_eq!(
            scan.perf_status.get(&COMPACT_STATUS_STATE_UNAVAILABLE),
            Some(&1),
            "and the status is reported"
        );
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let mut ok = d18_record(1, 100);
        ok.extend(d95_record(2, 100, 10));
        ok.extend(frame_end(3, 100));
        assert!(run_scan(&ok).is_ok());
        let mut bad = vec![
            frame_end_sealed(3, 100, 0, 0, 0, 1),
            frame_end_sealed(3, 100, 0, 1, 0, 0),
            frame_end_sealed(3, 100, 1, 0, 0, 0),
        ];
        for bit in 14..32u32 {
            bad.push(frame_end_sealed(3, 100, 0, 0, 1 << bit, 0));
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad.push(frame_end_sealed(3, 100, 0, 0, 1 << bit, 0));
        }
        for seal in bad {
            let mut s = d18_record(1, 100);
            s.extend(d95_record(2, 100, 10));
            s.extend(seal);
            assert!(run_scan(&s).is_err());
        }
        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut s = d18_record(1, 100);
            s.extend(d95_record(2, 100, 10));
            s.extend(record(
                3,
                100,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0; len],
            ));
            assert!(run_scan(&s).is_err());
        }
        for status in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = d18_record(1, 100);
            s.extend(d95_record(2, 100, 10));
            s.extend(frame_end_sealed(3, 100, 0, 0, status, 0));
            assert!(run_scan(&s).is_ok());
        }
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let mut ok = perf(1, 100, 20, 0);
        ok.extend(perf(2, 100, D95, 0));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID, 0));
        ok.extend(d18_record(4, 100));
        ok.extend(d95_record(5, 100, 10));
        ok.extend(frame_end_sealed(6, 100, 2, 2, 0, 0));
        assert!(run_scan(&ok).is_ok());
        let mut duplicate = perf(1, 100, 20, 0);
        duplicate.extend(perf(2, 100, 20, 0));
        duplicate.extend(d18_record(3, 100));
        duplicate.extend(d95_record(4, 100, 10));
        duplicate.extend(frame_end_sealed(5, 100, 1, 1, 0, 0));
        assert!(run_scan(&duplicate).is_err());
        let mut interleaved = d18_record(1, 100);
        interleaved.extend(d95_record(2, 100, 10));
        interleaved.extend(perf(3, 999, 20, 0));
        interleaved.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(run_scan(&interleaved).is_err());
        let mut conflict = perf(1, 100, 20, 0);
        conflict.extend(perf(2, 101, 21, 0));
        conflict.extend(d18_record(3, 100));
        conflict.extend(d95_record(4, 100, 10));
        conflict.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(run_scan(&conflict).is_err());
        for (emitted, scheduled) in [(1, 1), (3, 3), (2, 3), (3, 2)] {
            let mut s = perf(1, 100, 20, 0);
            s.extend(perf(2, 100, D95, 0));
            s.extend(d18_record(3, 100));
            s.extend(d95_record(4, 100, 10));
            s.extend(frame_end_sealed(5, 100, emitted, scheduled, 0, 0));
            assert!(run_scan(&s).is_err());
        }
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        for (perf_hour, data_hour) in [(100, 101), (101, 100), (100, 200), (200, 100)] {
            let mut s = perf(1, perf_hour, 20, 0);
            s.extend(d18_record(2, data_hour));
            s.extend(d95_record(3, data_hour, 10));
            s.extend(frame_end_sealed(4, data_hour, 1, 1, 0, 0));
            assert!(run_scan(&s).is_err());
        }
        let mut agreeing = perf(1, 100, 20, 0);
        agreeing.extend(d18_record(2, 100));
        agreeing.extend(d95_record(3, 100, 10));
        agreeing.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(run_scan(&agreeing).is_ok());
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(
            scan_stream(&decapitated, &test_schema(), StreamIdentityMode::IdentityV1).is_err()
        );
        assert!(scan_stream(&decapitated, &test_schema(), StreamIdentityMode::Legacy).is_ok());
        let mut identity = record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        );
        identity.extend(good_stream());
        assert!(scan_stream(&identity, &test_schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a DATA record leaves that hour open and ends the
        // stream on a record that is not a FRAME_END; that would produce a
        // plausible shorter number and must be refused.
        let full = good_stream();
        for records in [1usize, 2, 4, 5] {
            let cut = boundary(&full, records);
            assert!(
                run_scan(&full[..cut]).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream.
        for (records, expected_hours) in [(3usize, 1usize), (6, 2)] {
            let cut = boundary(&full, records);
            let scan = run_scan(&full[..cut]).expect("closed stream is well-formed");
            assert_eq!(scan.hours.len(), expected_hours, "cut after {records}");
        }
    }

    #[test]
    fn a_stream_ending_mid_record_is_rejected() {
        let full = good_stream();
        let boundaries: Vec<usize> = (0..=6).map(|n| boundary(&full, n)).collect();
        for cut in 1..full.len() {
            if boundaries.contains(&cut) {
                continue;
            }
            assert!(run_scan(&full[..cut]).is_err(), "mid-record cut at {cut}");
        }
        assert!(run_scan(&[]).is_err(), "an empty stream is not a measurement");
    }

    #[test]
    fn a_header_the_canonical_parser_rejects_is_rejected_here() {
        for records in 0..6 {
            let at = boundary(&good_stream(), records);
            for (offset, patch) in [
                (0usize, vec![b'X']),              // magic
                (4, 2u16.to_le_bytes().to_vec()),  // unsupported version
                (6, 33u16.to_le_bytes().to_vec()), // unsupported header length
            ] {
                let mut s = good_stream();
                s[at + offset..at + offset + patch.len()].copy_from_slice(&patch);
                assert!(
                    run_scan(&s).is_err(),
                    "record {records} patched at +{offset} must be refused"
                );
            }
        }
    }

    #[test]
    fn a_sequence_discontinuity_is_rejected() {
        for records in 0..6 {
            let at = boundary(&good_stream(), records);
            for wrong in [0u64, 99] {
                let mut s = good_stream();
                s[at + 8..at + 16].copy_from_slice(&wrong.to_le_bytes());
                assert!(run_scan(&s).is_err(), "record {records} sequence {wrong}");
            }
        }
    }

    #[test]
    fn a_frame_end_that_does_not_close_its_open_hour_is_rejected() {
        let mut s = d18_record(1, 100);
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 999));
        assert!(run_scan(&s).is_err());

        let s = frame_end(1, 100);
        assert!(run_scan(&s).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = d18_record(1, 101);
        s.extend(d95_record(2, 101, 10));
        s.extend(frame_end(3, 101));
        s.extend(d18_record(4, 100));
        s.extend(d95_record(5, 100, 20));
        s.extend(frame_end(6, 100));
        assert!(run_scan(&s).is_err());
    }

    #[test]
    fn a_stream_with_no_d95_records_is_refused_not_reported_as_zero() {
        let mut s = d18_record(1, 100);
        s.extend(frame_end(2, 100));
        assert!(run_scan(&s).is_err());
    }

    #[test]
    fn an_hour_that_carries_no_d95_record_is_counted_not_measured() {
        // Historically this hour was counted and reported; the completeness
        // sweep made it a REFUSED STREAM instead (the `bench_real_d65.rs`
        // bound): every closed hour of a d95-scheduled capture carries at least
        // one d95 DATA record, so an hour with none is a partially-lost or
        // relabelled d95 lane, not a quiet zero.
        let mut s = d18_record(1, 100);
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        s.extend(d18_record(4, 101));
        s.extend(frame_end(5, 101));
        let err = match run_scan(&s) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d95 record must refuse the stream"),
        };
        assert!(
            err.contains("without any d95 DATA record"),
            "the refusal names the completeness rule: {err}"
        );
    }

    #[test]
    fn a_partially_missing_d18_lane_refuses_the_stream() {
        // The d18 half of the completeness bound: hour 101 carries a d95 record
        // but no d18 record at all. Structurally well formed — only the
        // completeness check can see it.
        let mut s = d18_record(1, 100);
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        s.extend(d95_record(4, 101, 11));
        s.extend(frame_end(5, 101));
        let err = match run_scan(&s) {
            Err(err) => err,
            Ok(_) => panic!("a partially-missing d18 lane must refuse the stream"),
        };
        assert!(err.contains("d18"), "the refusal names the d18 bound: {err}");
    }

    #[test]
    fn an_unassigned_d18_status_bit_is_reported_and_still_committed() {
        // Round-1 finding. The recipe's "reject an unassigned status bit before
        // classifying" rule protects a domain's OWN measurement; applied to the
        // d18 dependency it makes the model stricter than the recorder, which
        // tests the failure mask alone and then commits the roster
        // (`world_fold.rs:13922`). Excluding those hours would exclude hours the
        // recorder measures. The bits are reported instead.
        for bit in 14..32u32 {
            let fold = d18_roster(&d18_payload(1 << bit, &[(368, "D04")], None))
                .unwrap_or_else(|e| panic!("bit {bit} must not be refused: {e}"));
            assert_eq!(
                fold.roster,
                Some(BTreeSet::from([368])),
                "bit {bit} is not a failure bit, so the recorder commits the roster"
            );
            assert_eq!(fold.unassigned_status_bits, 1 << bit, "bit {bit} reported");
        }
        // The reviewer's own failing input, end to end: the d95 hour behind such
        // a d18 record must be MEASURED, not excluded.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(1 << 14, &[(1, "GER"), (368, "D04")], None),
        ));
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        let scan = run_scan(&s).expect("well-formed");
        assert_eq!(scan.hours.len(), 1, "the hour is measured, as the recorder does");
        assert_eq!(scan.excluded.total(), 0);
        assert_eq!(scan.d18_unassigned_status_records, 1);

        // A clean roster is committed and reports nothing; a failure status still
        // commits nothing at all.
        let clean = d18_roster(&d18_payload(0, &[(368, "D04")], None)).expect("clean");
        assert_eq!(clean.roster, Some(BTreeSet::from([368])));
        assert_eq!(clean.unassigned_status_bits, 0);
        let failed = d18_roster(&d18_payload(
            COMPACT_STATUS_STATE_UNAVAILABLE,
            &[(368, "D04")],
            None,
        ))
        .expect("a failure status is not a refusal");
        assert_eq!(failed.roster, None);

        assert!(failed.body_deviation.is_none(), "a well-formed body");

        // This block used to assert the defect: that a MALFORMED failure-status
        // payload is an `Err`, "because production's payload authority is
        // status-independent". Two things were conflated, and the d96 port's
        // round-3 review separated them (thread
        // 019fa548-5e33-7061-9236-5c7151f92098). The authority's
        // status-independence decides what this harness must REPORT; the FOLD's
        // own order — status immediately after the header, `Ok` without reading
        // the body (`world_fold.rs:14158`-`:14164`) — decides how the record is
        // CLASSIFIED. So the body is still walked, and a body the authority would
        // reject is still surfaced, but as a named certification deviation rather
        // than as a reclassification into "refused".
        let mut malformed = d18_payload(COMPACT_STATUS_STATE_UNAVAILABLE, &[(368, "D04")], None);
        let len = malformed.len();
        malformed.truncate(len - 1);
        let fold = d18_roster(&malformed)
            .expect("the fold never reads this body, so it commits nothing and returns Ok");
        assert_eq!(fold.roster, None, "a failure status commits nothing");
        assert!(
            fold.body_deviation.is_some(),
            "the authority would reject the body: reported, not reclassified"
        );
    }

    #[test]
    fn a_d18_record_whose_changed_rows_exceeds_row_count_refuses_the_stream() {
        // Ported from `bench_real_d96.rs`, Codex round-1 finding
        // 019fa530-c013-74a2-b717-31f8e89e8639. `fold_country_identity` never reads
        // the word, so this mirror folded such a record normally and its hours were
        // measured — while production's status-independent d18 authority rejects it
        // (`domain_data_authority.rs:1029`-`:1033`) and the stream is not certified.
        // Fixed by property, not by one input: the whole (row_count, changed_rows,
        // status) space is swept below, and the authority's own predicate is applied
        // fail-closed ahead of the fold mirror.
        let mut bad = d18_payload(0, &[(368, "D04")], None);
        bad[12..16].copy_from_slice(&2u32.to_le_bytes()); // changed_rows = 2, row_count = 1
        let mut s = Vec::new();
        s.extend(record(1, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &bad));
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        let err = match run_scan(&s) {
            Err(err) => err,
            Ok(scan) => panic!(
                "changed_rows > row_count must refuse the stream, but {} hour(s) measured",
                scan.hours.len()
            ),
        };
        assert!(
            err.contains("changed_rows 2 > row_count 1") && err.contains("d18 payload at byte"),
            "{err}"
        );

        // The property, over the whole space: `changed_rows <= row_count` is the
        // rule on any row count, and a failure status does not exempt it (the
        // authority is status-independent).
        for status in [0u32, COMPACT_STATUS_STATE_UNAVAILABLE] {
            for row_count in 0..4usize {
                let tags: Vec<(u32, &str)> = (0..row_count)
                    .map(|index| (index as u32 + 1, ["GER", "ENG", "FRA"][index % 3]))
                    .collect();
                for changed_rows in 0..6u32 {
                    let mut payload = d18_payload(status, &tags, None);
                    payload[12..16].copy_from_slice(&changed_rows.to_le_bytes());
                    assert_eq!(
                        check_d18_payload_authority(&payload).is_ok(),
                        changed_rows as usize <= row_count,
                        "status {status:#x}, row_count {row_count}, changed_rows {changed_rows}"
                    );
                }
            }
        }
        // A payload too short to carry the word is NOT this helper's business: the
        // fold refuses it and the recorder tolerates that refusal, so it must fall
        // through to `d18_roster` rather than refusing the stream (d96 round-2
        // finding, thread 019fa53d-daf5-7810-a470-aeeb710e8166).
        for len in 0..20usize {
            assert!(
                check_d18_payload_authority(&vec![0u8; len]).is_ok(),
                "a {len}-byte payload is the fold mirror's business, not the authority's"
            );
        }

        // And the well-formed case still measures.
        let mut good = d18_payload(0, &[(368, "D04")], None);
        good[12..16].copy_from_slice(&1u32.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(1, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &good));
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        let scan = run_scan(&s).expect("changed_rows == row_count is legal");
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_failure_status_d18_record_is_classified_by_the_fold_not_by_its_body() {
        // Ported from `bench_real_d96.rs`, Codex round-3 finding
        // 019fa548-5e33-7061-9236-5c7151f92098, whose exact input this carries: a
        // 20-byte d18 payload claiming `row_count = 1` with no row body at all,
        // under a failure status. `fold_country_identity` checks the status
        // immediately after the header and returns `Ok` WITHOUT reading the body
        // (`world_fold.rs:14158`), so the record is a failure record — not a
        // refused one. This bin's mirror walked the rows first and reported it as
        // `d18_refused_records`.
        let mut headless = vec![0u8; 20];
        headless[8..12].copy_from_slice(&1u32.to_le_bytes()); // row_count = 1
        headless[16..20].copy_from_slice(&COMPACT_STATUS_STATE_UNAVAILABLE.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(1, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &headless));
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        let scan = run_scan(&s).expect("the recorder accepts this stream");
        assert_eq!(scan.d18_failure_records, 1, "the fold's classification");
        assert_eq!(scan.d18_refused_records, 0, "not a refused payload");
        // …and the certification consequence is still reported, because the payload
        // authority DOES walk that body (the recipe's d49 rule).
        assert_eq!(scan.d18_failure_records_with_unwalkable_body, 1);
        assert!(scan.d18_first_failure_body_deviation.is_some());
        // The roster committed nothing, so the hour is excluded, not measured.
        assert_eq!(scan.hours.len(), 0);

        // A truncated HEADER is still a refused payload, because the fold's own
        // `need(payload, 20, ...)` precedes the status read.
        assert!(d18_roster(&[0u8; 19]).is_err());
    }

    #[test]
    fn the_shipping_split_counts_only_the_records_the_arm_contains() {
        // Round-1 finding: charging framing for every d95 record in the stream
        // underflows once an hour is excluded, because an excluded hour's records
        // are in neither arm. Two excluded hours, then one measured hour.
        let mut s = Vec::new();
        // Hour 100 and 101: a d95 record whose hour's d18 record is a
        // failure-status one, committing no roster -> excluded.
        s.extend(d18_failure_record(1, 100));
        s.extend(d95_record(2, 100, 10));
        s.extend(frame_end(3, 100));
        s.extend(d18_failure_record(4, 101));
        s.extend(d95_record(5, 101, 20));
        s.extend(frame_end(6, 101));
        // Hour 102: measured.
        s.extend(d18_record(7, 102));
        s.extend(d95_record(8, 102, 30));
        s.extend(frame_end(9, 102));
        let scan = run_scan(&s).expect("well-formed");
        assert_eq!(scan.excluded.no_roster_this_hour, 2);
        assert_eq!(scan.d95_records, 3, "three records seen");
        assert_eq!(scan.hours.len(), 1, "one measured hour");

        let ship_records: usize = scan.hours.iter().map(|h| h.ship_spans.len()).sum();
        assert_eq!(ship_records, 1, "only the measured hour's record is charged");
        let ship_bytes: usize = scan
            .hours
            .iter()
            .flat_map(|h| h.ship_spans.iter())
            .map(|span| span.len())
            .sum();
        let (payload, framing) = shipping_split(ship_bytes, ship_records);
        assert_eq!(framing, COMPACT_RECORD_HEADER_LEN);
        assert_eq!(payload, ship_bytes - COMPACT_RECORD_HEADER_LEN);
        assert_eq!(payload + framing, ship_bytes, "the split is exhaustive");
        // The defective form, pinned: counting every d95 record would have
        // subtracted more framing than the arm holds.
        assert!(
            scan.d95_records * COMPACT_RECORD_HEADER_LEN > ship_bytes,
            "this stream is the underflow case the split must survive"
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
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(b"H4CR");
            }
            let _ = run_scan(&bytes);
            let _ = d18_roster(&bytes);
        }
        // And corruption of a genuinely well-formed stream, byte by byte.
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = run_scan(&s);
        }
    }
}
