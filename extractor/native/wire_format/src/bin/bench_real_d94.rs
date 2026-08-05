//! Measure the wire format against the shipping compact stream on REAL d94
//! `pov_province_visibility_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a capture that schedules **both**
//! d94 and d18 (`runs/batch3/deepwar_b2`). The stream's own d94 DATA records are
//! the shipping format's cost for the domain: a 24-byte FOW1 header plus the
//! complete engine province-level byte vector, re-sent in full every game hour.
//! This tool replays those records into per-hour keyed states — the same fold
//! `world_fold::fold_pov_province_visibility` performs, gates included — and
//! re-encodes every hour with this crate's grammar: a BASELINE at each 48-hour
//! block start, field-granular stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** The FOW1 payload grammar,
//! the key, the record and the frame-level fold go through `wire_format::d94`'s
//! four public boundaries; H4CR framing, record semantics and sequence continuity
//! go through `compact_contract`'s own parser and stateful validator. A harness
//! that reimplements either drifts from the recorder without anything failing.
//! Because d94's payload grammar lives in `compact_contract::pov_visibility`
//! rather than in the fold, and the reference library deliberately does not
//! depend on `compact_contract`, [`check_codec_agreement`] additionally asserts
//! that `PovVisibilitySnapshot::decode` and `d94::split_payload` agree — on every
//! record of the capture and on a swept synthetic space in the tests. That is
//! what keeps the restatement in `d94.rs` from drifting silently.
//!
//! **The d18 dependency is MODELLED, not assumed** (recipe step 9's first
//! treatment). `fold_pov_province_visibility` reads two facts out of the d18
//! `country_identity` accumulator and folds nothing without them
//! (`world_fold.rs:10512`-`:10524`), so [`D18Mirror`] reproduces the outcome of
//! `fold_country_identity` (`:13918`-`:14156`) hour by hour, in stream order:
//! the freshness flag it sets (`:14153`) and the `tag_db_id` key column of the
//! roster it installs (`:14146`). It reads only what those two gates need — the
//! tag strings, cosmetic tags, claim/core memberships and major flags are walked
//! for length and validity and then discarded. `country_identity_known_this_hour`
//! is cleared at every hour boundary (`:3002`) while `tag_by_db_id` **persists**
//! (`:2999`-`:3001`), and the mirror keeps them separate for the same reason the
//! recorder does.
//!
//! **The three outcomes of recipe step 9, named for this domain.** d94's fold has
//! four non-folding paths and they are not all the same kind of thing:
//!
//! - a payload the FOW1 codec refuses (magic, version, reserved, length, or any
//!   of `validate`'s three range rules) is a **stream rejection**. The recorder's
//!   second, status-independent authority runs that same decode on every
//!   DOMAIN_DATA record (`recorder_host/src/domain_data_authority.rs:16737`,
//!   dispatched `:17141`) and its `parse_errors` become the `Err` that
//!   `validate_domain_data_payload_at` reports at seal time
//!   (`recorder_host/src/stream_fold.rs:593`-`:611`);
//! - the **stale-roster** gate (`world_fold.rs:10512`), the **POV-not-in-roster**
//!   gate (`:10518`) and the **non-increasing capture_sequence** gate (`:10525`)
//!   are each an **excluded hour**: the dispatcher catches the `Err` and marks
//!   the domain-hour unknown (`:4917`-`:4925`), the stream stays valid, and d94
//!   publishes nothing for that hour.
//!
//! **The status decision is explicit, because this fold never reads a status
//! word** (recipe step 9: "a fold that never reads status at all forces an
//! explicit, recorded refuse-or-exclude decision; never choose silently"). The
//! FOW1 payload carries no status field at all, and `fold_pov_province_visibility`
//! ignores `record_flags`, so **no d94 DATA record is ever excluded for its
//! status** — this harness mirrors the fold and measures every record the codec
//! accepts. The domain's status word exists only in its DOMAIN_PERF record
//! (`status_flags` at payload offset 32). That word is therefore not a
//! classifier here; it is an **assert-inert capture check**, in the sense recipe
//! step 9 gives the term: [`check_perf_status`] fails the whole run, louder than
//! the recorder would, if any d94 PERF carries a failure bit or an unassigned v1
//! bit — because either would mean the measured hours are not the regime this
//! number claims to describe. It never fires on the reference capture, where all
//! 1,033 d94 PERF records read `0x000`.
//!
//! **The whole stream is validated before any artifact is written.** A stream
//! that is truncated, spliced, or corrupt would otherwise yield a smaller but
//! entirely plausible measurement. `scan_stream` therefore enforces,
//! fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`;
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`;
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`) and that tracker's DOMAIN_PERF hour
//!   binding (`:328`-`:365`);
//! - the FRAME_END seal fields, `validate_frame_end_seal_fields` (`:46`-`:89`),
//!   re-implemented here as [`check_frame_end_seal`], plus the seal's
//!   domain-count cross-check against the distinct non-probe PERF domains
//!   actually observed in the hour (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`).
//!
//! That is the same documented **subset** of the production boundary contract
//! `bench_real_d63.rs` implements, and it carries the same crate-boundary
//! limitation: production also runs `validate_domain_data_payload_at` on every
//! DOMAIN_DATA record of every domain, and `wire_format` cannot depend on
//! `recorder_host`. The two payload grammars this harness *can* validate are
//! d94's — fully, on every record — and d18's, which it must model anyway.
//! Every other domain's DATA payloads pass UNVALIDATED beyond header sanity,
//! the stream validator and the hour-boundary rules. That unvalidated
//! remainder is explicitly BOUNDED: every closed game hour must carry at least
//! one d94 DATA record (d94 tolerates several per hour, so "exactly one" is
//! not its grammar), and the d18 lane must be all-or-nothing — exactly one
//! record per closed hour when scheduled (the completeness checks in
//! [`scan_stream`]) — so a d94 or d18 record relabelled to another domain id,
//! or dropped, refuses the run instead of silently shrinking it or silently
//! converting hours into no-roster exclusions.
//!
//! **Two passes, because the state is large.** 13,414 province slots an hour over
//! 1,033 hours is 13.9M records; holding a snapshot per hour is not possible.
//! Pass 1 validates the whole stream and retains only record spans plus each
//! hour's roster verdict; pass 2 re-folds through the same `d94` boundary while
//! encoding, holding two hours of state at a time.
//!
//! **Three arms, because two of them differ only in re-baseline policy.** The
//! recipe's own warning — the block ratio measures re-baseline policy, not
//! grammar — has its sharpest instance here: this domain's baseline is 4.1 bytes
//! per row where the shipping payload is 1 byte per row (a dense positional
//! array spends nothing on identity; a keyed set spells every key out), while its
//! delta is ~26 bytes for a whole hour. So the arm's size is almost entirely a
//! function of how often it re-baselines, and reporting one cadence would report
//! a policy as a grammar result. `wire_new.bin` re-baselines every 48 hours, as
//! every other bin in this family does; `wire_single_baseline.bin` baselines
//! ONCE, which is what the shipping lanes actually do per campaign. Both are
//! reader-replayed hour by hour; neither is the "real" answer on its own.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d94.bin              — the d94 DATA records verbatim (32-byte H4CR
//!                               header + payload), in stream order
//!   wire_new.bin              — wire-format bodies, BASELINE every 48 h
//!   wire_single_baseline.bin  — wire-format bodies, ONE baseline for the run
//!   *.offsets                 — hour-boundary byte offsets, one per line
//!
//! It also prints the real churn surface, twice: cadence-independent (changed
//! rows over measured-hour transitions) and along the delta path (changed rows
//! and tombstones over the hours that actually emitted a delta).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::ops::Range;

use compact_contract::pov_visibility::PovVisibilitySnapshot;
use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_is_failure, compact_status_unknown_flags_v1, parse_compact_record_header,
    COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d94;

const D94: u16 = 94;
const D18: u16 = 18;
const BLOCK_HOURS: usize = 48;

/// The DOMAIN_PERF payload word the collector writes the domain's status into
/// (`collector/src/attach_log.rs:60031`-`:60104`).
const PERF_STATUS_OFFSET: usize = 32;

/// Every globally-defined compact status occupies one of bits 0..=14
/// (`recorder_host/src/stream_fold.rs:43`).
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;

/// The reader-context probe domain, excluded from the seal's domain accounting
/// (`recorder_host/src/stream_fold.rs:330`).
const PROBE_DOMAIN_ID: u16 = 250;

/// One measurable d94 hour: its game hour, the byte span of its DATA record in
/// the source stream (the shipping arm is those bytes verbatim, header included),
/// and the span of the record's payload, which pass 2 re-folds.
struct D94Hour {
    game_hour: i32,
    ship_span: Range<usize>,
    payload_span: Range<usize>,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded hour (any of the fold's three gates) or a non-contiguous hour
    /// step — which is what forces both wire arms to open a fresh baseline
    /// (`WireArm::push_hour`).
    follows_previous: bool,
}

/// Whether a wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences:
/// the arm's block cadence (every block opens with its own baseline, §1.5's
/// closing paragraph), and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap
/// affecting the state part forces the next state observation to be a
/// `BASELINE`." Every hour this domain did not successfully sample is such a
/// gap — each excluded-hour gate, and any non-contiguous hour step — and d94's
/// cadence is hourly, so a measured hour that does not follow the previous
/// measured one by exactly one is preceded by one. Neither a delta nor a
/// retained reader state can be justified across it. The rule applies to BOTH
/// arms, including the single-baseline policy arm: block cadence is a policy,
/// the gap-forced baseline is grammar.
fn opens_baseline(hours_seen: usize, block_hours: usize, follows_previous: bool) -> bool {
    hours_seen % block_hours == 0 || !follows_previous
}

struct StreamScan {
    hours: Vec<D94Hour>,
    /// d94 hours the fold refuses on one of its three gates. Excluded from both
    /// arms, because the recorder publishes nothing for them.
    excluded_hours: Vec<(i32, String)>,
    /// Measured d94 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// d94 DATA records seen, of any outcome.
    d94_records: usize,
    /// Hours carrying more than one d94 DATA record.
    multi_record_hours: usize,
    /// The d94 DOMAIN_PERF status words observed, for the report.
    perf_status: BTreeMap<u32, usize>,
    /// Distinct POV countries the capture's d94 records carry.
    pov_countries: BTreeMap<u32, usize>,
    /// Distinct province-vector lengths the capture's d94 records carry.
    slot_counts: BTreeMap<u32, usize>,
    /// Hours in which a d18 record was folded successfully.
    d18_measured_hours: usize,
    /// The distinct roster sizes the d18 mirror installed.
    d18_roster_sizes: BTreeSet<usize>,
    /// d18 records whose failure status left the roster stale
    /// (`world_fold.rs:13922`), by status word.
    d18_failure_status: BTreeMap<u32, usize>,
}

// --- The d18 dependency, modelled ---------------------------------------------

/// The outcome of one d18 `country_identity` payload, as
/// `fold_country_identity` produces it (`world_fold.rs:13918`-`:14156`).
enum D18Outcome {
    /// A clean record: the roster is replaced and the hour is "known".
    Measured(BTreeSet<u32>),
    /// A failure status: the fold returns `Ok` **without touching** the roster
    /// and without setting the freshness flag (`:13922`-`:13928`), so the hour
    /// stays "not known" and the previous roster persists.
    FailureStatus(u32),
}

/// The two facts d94 needs out of d18, tracked across the stream exactly as the
/// recorder tracks them.
///
/// `tag_db_ids` PERSISTS across hours — d18 is full-replace and the map is
/// overwritten only by a successful fold (`world_fold.rs:14146`) — while
/// `known_this_hour` is cleared at every hour boundary (`:3002`).
#[derive(Default)]
struct D18Mirror {
    tag_db_ids: BTreeSet<u32>,
    known_this_hour: bool,
}

impl D18Mirror {
    /// `world_fold.rs:3002` and `:2999`-`:3001`.
    fn begin_hour(&mut self) {
        self.known_this_hour = false;
    }

    fn observe(&mut self, outcome: D18Outcome) {
        match outcome {
            D18Outcome::Measured(ids) => {
                self.tag_db_ids = ids;
                self.known_this_hour = true;
            }
            D18Outcome::FailureStatus(_) => {}
        }
    }

    fn roster(&self) -> d94::Roster {
        if self.known_this_hour {
            d94::Roster::measured_this_hour(self.tag_db_ids.clone())
        } else {
            d94::Roster::carried_over(self.tag_db_ids.clone())
        }
    }
}

fn u16_at(b: &[u8], off: usize) -> Result<u16, String> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| format!("country_identity payload is short at +{off}"))
}

fn u32_at(b: &[u8], off: usize) -> Result<u32, String> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| format!("country_identity payload is short at +{off}"))
}

/// Consume one length-prefixed string, exactly as `read_string`
/// (`world_fold.rs:30256`-`:30266`): a u16 length, then that many bytes, which
/// must be valid UTF-8. The bytes themselves are discarded — d94 needs the roster
/// key column, not the tags.
fn skip_string(payload: &[u8], cursor: &mut usize, label: &str) -> Result<usize, String> {
    let len = u16_at(payload, *cursor)? as usize;
    *cursor += 2;
    let end = cursor
        .checked_add(len)
        .ok_or_else(|| format!("{label}: length overflow"))?;
    let bytes = payload
        .get(*cursor..end)
        .ok_or_else(|| format!("{label}: payload is short"))?;
    std::str::from_utf8(bytes).map_err(|e| format!("{label}: invalid utf-8: {e}"))?;
    *cursor = end;
    Ok(len)
}

/// The d18 payload header the two authorities below share
/// (`world_fold.rs:13919`-`:13921`,
/// `recorder_host/src/domain_data_authority.rs:1013`-`:1027`): `row_count` at
/// `+8`, `changed_rows` at `+12`, `status_flags` at `+16`, after a 20-byte
/// minimum both of them enforce.
fn country_identity_header(payload: &[u8]) -> Result<(usize, usize, u32), String> {
    if payload.len() < 20 {
        return Err(format!(
            "country_identity payload {} shorter than its 20-byte header",
            payload.len()
        ));
    }
    Ok((
        u32_at(payload, 8)? as usize,
        u32_at(payload, 12)? as usize,
        u32_at(payload, 16)?,
    ))
}

/// Production's **second, status-independent authority** for a d18 DOMAIN_DATA
/// record: `decode_compact_country_identity_data_payload`
/// (`recorder_host/src/domain_data_authority.rs:1013`-`:1284`). Its `parse_errors`
/// become the `Err` that `validate_domain_data_payload_at` reports at seal time
/// (`recorder_host/src/stream_fold.rs:593`-`:611`), so a complaint here is a
/// **stream rejection**, not an hour to skip.
///
/// It differs from the fold in exactly two ways, and both were round-2 review
/// findings — a stream production refuses used to be measured here as healthy:
///
/// 1. **It checks `changed_rows <= row_count`** (`:1029`-`:1033`). The fold never
///    reads that word at all, so nothing else in this harness would catch it.
/// 2. **It is status-independent**: unlike the fold, which returns early on a
///    failure status without walking the body (`world_fold.rs:13922`), this
///    authority walks every row and the whole CIM1 extension whatever the status
///    says. A failure-status d18 record with a malformed body is therefore a
///    refused stream, even though the fold reads nothing from it.
///
/// Everything else it checks is the fold's own rule set, so both callers share
/// one walk ([`walk_country_identity_body`]) rather than carrying two copies that
/// can drift.
fn check_d18_authority_payload(payload: &[u8]) -> Result<(), String> {
    let (row_count, changed_rows, _status_flags) = country_identity_header(payload)?;
    // `domain_data_authority.rs:1029`-`:1033`.
    if changed_rows > row_count {
        return Err(format!(
            "country_identity changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    // Status-independent, unlike the fold: the authority walks the body whatever
    // the status word says.
    walk_country_identity_body(payload, row_count).map(|_| ())
}

/// Reproduce `fold_country_identity`'s outcome for one payload
/// (`world_fold.rs:13918`-`:14156`).
///
/// **A payload this mirror cannot parse fails the run** rather than being modelled
/// as a not-known hour. The mirror cannot tell "the recorder would refuse this
/// too" from "this mirror is wrong", and guessing would silently change which
/// hours the measurement contains. In practice the authority above has already
/// refused any such payload by the time this runs. It never fires on the
/// reference capture, where all 1,033 d18 records parse.
fn fold_country_identity(payload: &[u8]) -> Result<D18Outcome, String> {
    let (row_count, _changed_rows, status_flags) = country_identity_header(payload)?;
    // `:13922`-`:13928`. The production path (not `boundary_validation`) returns
    // Ok and leaves the map alone — WITHOUT walking the body, which is why the
    // authority above has to walk it separately.
    if compact_status_is_failure(status_flags) {
        return Ok(D18Outcome::FailureStatus(status_flags));
    }
    Ok(D18Outcome::Measured(walk_country_identity_body(
        payload, row_count,
    )?))
}

/// The d18 body walk both authorities run: `row_count` variable-length rows, then
/// the optional CIM1 extension, then the no-trailing-bytes rule.
///
/// **What this reads and what it only walks.** d94 needs the `tag_db_id` key
/// column; everything else — tag strings, cosmetic markers and their strings, the
/// whole CIM1 claims/cores/major extension — is walked because the
/// `cursor != payload.len()` rule (`world_fold.rs:14140`,
/// `domain_data_authority.rs:1279`) makes the *length* of every one of them
/// load-bearing, and validated because a refusal of any of them is what decides
/// known-vs-not-known on one side and stream-rejection on the other. The values
/// are then dropped.
fn walk_country_identity_body(
    payload: &[u8],
    row_count: usize,
) -> Result<BTreeSet<u32>, String> {
    let mut cursor = 20usize;
    let mut fresh: BTreeSet<u32> = BTreeSet::new();
    for _ in 0..row_count {
        let tag_db_id = u32_at(payload, cursor)?;
        cursor += 4;
        skip_string(payload, &mut cursor, "country_identity tag")
            .and_then(|len| {
                // `:13943` — an empty tag is a refused payload.
                if len == 0 {
                    Err("country_identity row with empty tag".to_string())
                } else {
                    Ok(())
                }
            })?;
        // `:13940` — a zero tag_db_id is a refused payload.
        if tag_db_id == 0 {
            return Err("country_identity row with zero tag_db_id".into());
        }
        // `:13946` — a duplicate is a refused payload, never last-writer-wins.
        if !fresh.insert(tag_db_id) {
            return Err(format!("country_identity duplicates tag_db_id {tag_db_id}"));
        }
        // The wave-4 cosmetic marker, `:13954`-`:13976`.
        let marker = *payload
            .get(cursor)
            .ok_or("country_identity cosmetic marker is past the payload")?;
        cursor += 1;
        match marker {
            0 | 1 => {}
            2 => {
                let len = skip_string(payload, &mut cursor, "country_identity cosmetic")?;
                if len == 0 {
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

    // The optional CIM1 extension, `:13982`-`:14139`. It is present on every
    // record of the reference capture, so walking it is not optional: without it
    // the trailing-bytes rule below would refuse every hour and the measurement
    // would be empty.
    if cursor < payload.len() {
        const CIM1_HEADER_LEN: usize = 40;
        if payload.len() < cursor + CIM1_HEADER_LEN
            || &payload[cursor..cursor + 4] != b"CIM1"
            || u16_at(payload, cursor + 4)? != 1
            || u16_at(payload, cursor + 6)? as usize != CIM1_HEADER_LEN
            || u32_at(payload, cursor + 36)? != 0
        {
            return Err("country_identity CIM1 header is invalid".into());
        }
        let claims_status = u32_at(payload, cursor + 8)?;
        let cores_status = u32_at(payload, cursor + 12)?;
        let major_status = u32_at(payload, cursor + 16)?;
        let state_count = u32_at(payload, cursor + 20)? as usize;
        let expected_claim_memberships = u32_at(payload, cursor + 24)? as usize;
        let expected_core_memberships = u32_at(payload, cursor + 28)? as usize;
        let major_count = u32_at(payload, cursor + 32)? as usize;
        let claims_available = !compact_status_is_failure(claims_status);
        let cores_available = !compact_status_is_failure(cores_status);
        let major_available = !compact_status_is_failure(major_status);
        // `:14002`-`:14009`.
        if state_count > 100_000
            || expected_claim_memberships > 1_000_000
            || expected_core_memberships > 1_000_000
            || major_count > fresh.len()
        {
            return Err("country_identity CIM1 count exceeds safety bound".into());
        }
        // `:14010`-`:14018`.
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
            let state_id = u32_at(payload, cursor)?;
            let claim_count = u32_at(payload, cursor + 4)? as usize;
            let core_count = u32_at(payload, cursor + 8)? as usize;
            cursor += 12;
            // `:14042`-`:14047`.
            if state_id == 0 || (index != 0 && state_id <= previous_state_id) {
                return Err(format!(
                    "country_identity CIM1 state row {index} is not strictly ordered"
                ));
            }
            previous_state_id = state_id;
            // `:14049`-`:14056`.
            if (!claims_available && claim_count != 0) || (!cores_available && core_count != 0) {
                return Err(format!(
                    "country_identity CIM1 unavailable state row {state_id} carries members"
                ));
            }
            // `:14057`-`:14062` — checked, because these are file words.
            let row_bytes = claim_count
                .checked_add(core_count)
                .and_then(|count| count.checked_mul(4))
                .ok_or("country_identity CIM1 state row size overflow")?;
            if payload.len() < cursor + row_bytes {
                return Err("country_identity CIM1 state members are past the payload".into());
            }
            for (member_index, count) in [(0usize, claim_count), (1, core_count)] {
                let mut previous_tag_id = 0u32;
                for i in 0..count {
                    let tag_db_id = u32_at(payload, cursor)?;
                    cursor += 4;
                    // `:14066`-`:14075` and `:14086`-`:14095`, the same predicate.
                    if tag_db_id == 0
                        || (i != 0 && tag_db_id <= previous_tag_id)
                        || !fresh.contains(&tag_db_id)
                    {
                        return Err(format!(
                            "country_identity CIM1 state {state_id} member is invalid"
                        ));
                    }
                    previous_tag_id = tag_db_id;
                }
                if member_index == 0 {
                    claim_memberships += count;
                } else {
                    core_memberships += count;
                }
            }
        }
        // `:14104`-`:14108`.
        if claim_memberships != expected_claim_memberships
            || core_memberships != expected_core_memberships
        {
            return Err("country_identity CIM1 membership totals do not match header".into());
        }
        // The major rows, `:14111`-`:14131`.
        let mut previous_major_id = 0u32;
        let mut majors: BTreeSet<u32> = BTreeSet::new();
        for index in 0..major_count {
            let tag_db_id = u32_at(payload, cursor)?;
            let marker = *payload
                .get(cursor + 4)
                .ok_or("country_identity CIM1 major row is past the payload")?;
            let padding = payload
                .get(cursor + 5..cursor + 8)
                .ok_or("country_identity CIM1 major row is past the payload")?;
            if tag_db_id == 0
                || (index != 0 && tag_db_id <= previous_major_id)
                || marker > 1
                || padding != [0, 0, 0]
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
        // `:14133`-`:14137` — the major roster must be the whole roster.
        if major_available && majors != fresh {
            return Err("country_identity CIM1 major roster is incomplete".into());
        }
    }
    // `:14140`-`:14145`.
    if cursor != payload.len() {
        return Err(format!(
            "country_identity len {} != consumed {cursor}",
            payload.len()
        ));
    }
    Ok(fresh)
}

// --- Stream-level checks ------------------------------------------------------

/// The FRAME_END seal check, re-implemented from
/// `recorder_host::stream_fold::validate_frame_end_seal_fields` (`:46`-`:89`)
/// because `wire_format` must not depend on `recorder_host`.
///
/// Returns the seal's `(emitted_domains, scheduled_domains)` so the caller can
/// run production's cross-check against the PERF domains it actually observed
/// (`:411`-`:419`). Total: no payload of any length panics.
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

/// The d94 DOMAIN_PERF status word — an assert-inert capture check, not a
/// classifier. See the module docs for why the distinction is load-bearing:
/// d94's fold never reads any status, so nothing is excluded for one, and this
/// exists to make a capture in a different regime fail loudly rather than be
/// measured as though it were this one.
///
/// A PERF payload too short to carry the word is a malformed record and is
/// refused rather than skipped.
fn check_perf_status(payload: &[u8], at_byte: usize) -> Result<u32, String> {
    let status_flags = payload
        .get(PERF_STATUS_OFFSET..PERF_STATUS_OFFSET + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| {
            format!(
                "d94 DOMAIN_PERF at byte {at_byte}: payload {} is too short to carry its \
                 status word at +{PERF_STATUS_OFFSET}",
                payload.len()
            )
        })?;
    // Validity before classification (`compact_contract/src/lib.rs:121`-`:123`):
    // an unassigned bit has no defined classification and is not evidence of a
    // future informational status.
    let unknown = compact_status_unknown_flags_v1(status_flags);
    if unknown != 0 {
        return Err(format!(
            "d94 DOMAIN_PERF at byte {at_byte} carries unassigned v1 status bits {unknown:#x}; \
             an unassigned bit has no defined classification"
        ));
    }
    if compact_status_is_failure(status_flags) {
        return Err(format!(
            "d94 DOMAIN_PERF at byte {at_byte} reports failure status {status_flags:#x}. \
             The fold reads no status word, so this harness cannot exclude the hour without \
             diverging from the recorder — and it will not measure a regime it has not \
             examined. Re-run the measurement against a capture whose d94 lane is clean, or \
             extend this harness once the semantics of a failed d94 hour are established."
        ));
    }
    Ok(status_flags)
}

/// Assert that this crate's restatement of the FOW1 grammar and the shipping
/// codec agree on a payload.
///
/// `d94::split_payload` exists because the reference library must not depend on
/// `compact_contract`; this check is what stops the two from drifting. It
/// compares the whole decoded content — every header word and the entire byte
/// vector — and it also requires the two to agree about REFUSAL, in both
/// directions, so neither can be quietly more permissive than the other.
fn check_codec_agreement(payload: &[u8], at_byte: usize) -> Result<(), String> {
    match (d94::split_payload(payload), PovVisibilitySnapshot::decode(payload)) {
        (Ok((header, body)), Ok(snapshot)) => {
            if header.capture_sequence != snapshot.capture_sequence
                || header.pov_country_id != snapshot.pov_country_id
                || header.count as usize != snapshot.province_levels.len()
                || body != snapshot.province_levels.as_slice()
            {
                Err(format!(
                    "d94 payload at byte {at_byte}: wire_format::d94 and \
                     compact_contract::pov_visibility disagree about its content"
                ))
            } else {
                Ok(())
            }
        }
        (Err(_), Err(_)) => Ok(()),
        (Ok(_), Err(codec)) => Err(format!(
            "d94 payload at byte {at_byte}: wire_format::d94 accepts a payload the shipping \
             codec refuses ({codec})"
        )),
        (Err(ours), Ok(_)) => Err(format!(
            "d94 payload at byte {at_byte}: wire_format::d94 refuses a payload the shipping \
             codec accepts ({ours})"
        )),
    }
}

/// Validate the whole stream against the canonical contract, model d18, and
/// classify every d94 DATA record. Every rejection path is an `Err`, never a
/// silently shorter measurement and never a panic.
fn scan_stream(stream: &[u8], identity_mode: StreamIdentityMode) -> Result<StreamScan, String> {
    // The identity protocol is an INPUT, never sniffed: an `IdentityV1` stream
    // whose mandatory CID1 record has been spliced off is byte-indistinguishable
    // from a legacy stream, so a sniffing harness measures a decapitated stream
    // that production refuses (`compact_contract/src/stream_validator.rs:328`,
    // `recorder_host/src/stream_fold.rs:220`).
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut scan = StreamScan {
        hours: Vec::new(),
        excluded_hours: Vec::new(),
        non_contiguous: 0,
        d94_records: 0,
        multi_record_hours: 0,
        perf_status: BTreeMap::new(),
        pov_countries: BTreeMap::new(),
        slot_counts: BTreeMap::new(),
        d18_measured_hours: 0,
        d18_roster_sizes: BTreeSet::new(),
        d18_failure_status: BTreeMap::new(),
    };

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // The DOMAIN_PERF hour-binding state, `BoundaryTracker`'s own
    // (`stream_fold.rs:328`-`:365`, reset at `:449`-`:451`).
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    let mut last_measured_hour: Option<i32> = None;

    // The modelled d18 accumulator, and this hour's d94 accumulator. Both are
    // per-hour objects in the recorder, so both are reset at the same place the
    // recorder resets them.
    let mut d18 = D18Mirror::default();
    let mut d94_state = d94::State::default();
    // The hour the two accumulators currently describe, and that hour's d94
    // bookkeeping.
    let mut accumulating_hour: Option<i32> = None;
    let mut hour_d94_records = 0usize;
    // COMPLETENESS accounting: closed game hours, hours in which at least one
    // d94 DATA record landed, and d18 DATA records of any outcome — see the
    // checks after the EOF rules. The d18 lane is also tracked PER HOUR:
    // aggregate counts alone would accept a compensating duplicate (two d18
    // records in one hour, none in another) whose totals still match (a Codex
    // round-1 finding).
    let mut closed_hours = 0usize;
    let mut d94_covered_hours = 0usize;
    let mut d18_records = 0usize;
    let mut d18_covered_hours = 0usize;
    let mut hour_saw_d18 = false;
    let mut hour_span: Option<(Range<usize>, Range<usize>)> = None;
    let mut hour_excluded_reason: Option<String> = None;

    // Close out an accumulating hour: either it folded a snapshot (measured) or
    // it did not (excluded, with the reason the fold gave).
    macro_rules! close_accumulating_hour {
        ($scan:expr) => {
            if let Some(hour) = accumulating_hour.take() {
                if d94_state.seen() {
                    d94_covered_hours += 1;
                    if d94_state.accepted().is_some() {
                        let (ship_span, payload_span) = hour_span
                            .clone()
                            .ok_or_else(|| format!("hour {hour}: accepted d94 with no span"))?;
                        let follows_previous =
                            last_measured_hour.is_some_and(|previous| hour == previous + 1);
                        if last_measured_hour.is_some() && !follows_previous {
                            $scan.non_contiguous += 1;
                        }
                        last_measured_hour = Some(hour);
                        $scan.hours.push(D94Hour {
                            game_hour: hour,
                            ship_span,
                            payload_span,
                            follows_previous,
                        });
                    } else {
                        $scan.excluded_hours.push((
                            hour,
                            hour_excluded_reason
                                .clone()
                                .unwrap_or_else(|| "d94 folded nothing".to_string()),
                        ));
                    }
                }
                d94_state.begin_hour();
                d18.begin_hour();
                hour_d94_records = 0;
                hour_span = None;
                hour_excluded_reason = None;
            }
        };
    }

    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|e| format!("record at byte {pos}: {e}"))?;
        let payload_start = pos + COMPACT_RECORD_HEADER_LEN;
        let record_end = payload_start
            .checked_add(header.payload_len)
            .ok_or_else(|| format!("record at byte {pos}: length overflow"))?;
        if record_end > stream.len() {
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
                if header.domain_id == D94 {
                    let status = check_perf_status(payload, pos)?;
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
                    // `stream_fold.rs:378`-`:386`, and it comes BEFORE the
                    // monotonicity check, as it does there.
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
                closed_hours += 1;
                if hour_saw_d18 {
                    d18_covered_hours += 1;
                }
                hour_saw_d18 = false;
                // `world_fold::finish_hour` — the accumulators are snapshotted
                // and reset here, so this is where the hour's verdict is taken.
                close_accumulating_hour!(scan);
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            // A DATA record for a new hour starts the accumulating hour. The
            // boundary checks above already guarantee hours do not interleave.
            if accumulating_hour != Some(header.game_hour_raw) {
                close_accumulating_hour!(scan);
                accumulating_hour = Some(header.game_hour_raw);
            }
            if header.domain_id == D18 {
                d18_records += 1;
                hour_saw_d18 = true;
                // Production's second, status-independent authority runs on EVERY
                // d18 DOMAIN_DATA record and its complaint refuses the stream at
                // seal time. It must come BEFORE the fold outcome is used, or a
                // stream the recorder refuses gets measured here as healthy —
                // which is exactly what the `changed_rows` and failure-status-body
                // gaps were.
                check_d18_authority_payload(payload).map_err(|e| {
                    format!(
                        "d18 payload at byte {pos} (hour {}): {e}. Production's \
                         status-independent payload authority refuses this record, and its \
                         complaint refuses the whole stream at seal time.",
                        header.game_hour_raw
                    )
                })?;
                let outcome = fold_country_identity(payload).map_err(|e| {
                    format!(
                        "d18 payload at byte {pos} (hour {}): {e}. The d18 roster is a hard \
                         input to every d94 hour, so a payload this mirror cannot reproduce \
                         fails the run rather than silently changing which hours are measured.",
                        header.game_hour_raw
                    )
                })?;
                match &outcome {
                    D18Outcome::Measured(ids) => {
                        scan.d18_measured_hours += 1;
                        scan.d18_roster_sizes.insert(ids.len());
                    }
                    D18Outcome::FailureStatus(status) => {
                        *scan.d18_failure_status.entry(*status).or_insert(0) += 1;
                    }
                }
                d18.observe(outcome);
            } else if header.domain_id == D94 {
                scan.d94_records += 1;
                hour_d94_records += 1;
                if hour_d94_records == 2 {
                    scan.multi_record_hours += 1;
                }
                // Production's second, status-independent authority is exactly
                // this decode (`domain_data_authority.rs:16737`), and its
                // complaint refuses the stream at seal time — so a payload the
                // codec rejects is a stream rejection here, before anything else
                // looks at the record.
                check_codec_agreement(payload, pos)?;
                if let Ok((fow_header, _)) = d94::split_payload(payload) {
                    *scan.pov_countries.entry(fow_header.pov_country_id).or_insert(0) += 1;
                    *scan.slot_counts.entry(fow_header.count).or_insert(0) += 1;
                }
                // Fold exactly as `fold_pov_province_visibility` does, with the
                // hour's modelled d18 roster. An `Err` from the codec is a stream
                // rejection (already caught above and re-checked here, because
                // the fold's own call order puts the decode first); an `Err` from
                // one of the three gates is an excluded hour.
                match d94_state.fold_payload(payload, &d18.roster()) {
                    Ok(()) => {
                        hour_span = Some((pos..record_end, payload_start..record_end));
                        hour_excluded_reason = None;
                    }
                    Err(reason) if reason.contains("FOW1 invalid") => {
                        return Err(format!("d94 payload at byte {pos}: {reason}"));
                    }
                    Err(reason) => {
                        // Every gate refusal RETRACTS the hour (`d94.rs`'s error
                        // contract), so a later bad record erases an earlier good
                        // one and the span must go with it.
                        hour_span = None;
                        hour_excluded_reason = Some(reason);
                    }
                }
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
    // Every hour is closed by its FRAME_END, and the EOF rules above require the
    // stream's last record to be one — so nothing can be left accumulating here.
    // Stated as a check rather than argued: an hour that reached EOF without its
    // seal would otherwise be dropped from the measurement in silence.
    if accumulating_hour.is_some() {
        return Err(format!(
            "game hour {:?} is still accumulating at EOF",
            accumulating_hour
        ));
    }
    if scan.hours.is_empty() {
        return Err("stream carries no usable d94 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry at least one d94 DATA
    // record (d94 tolerates several per hour — `multi_record_hours` — so
    // "exactly one" is not this domain's grammar), and the d18 lane must be
    // all-or-nothing — exactly one record per closed hour when scheduled, none
    // at all when not. This bounds what this harness canNOT validate:
    // production runs `validate_domain_data_payload_at` on EVERY domain's DATA
    // payload (`stream_fold.rs:593`), while this harness owns only d94's and
    // d18's grammars, so a record of either domain whose header was relabelled
    // — or lost outright — would otherwise be silently ignored: a lost d94
    // record silently drops its hour from the measurement, and a lost d18
    // record silently converts its hour's d94 record into a no-roster
    // exclusion. A per-capture completeness assertion, NOT a grammar rule
    // (`bench_real_d65.rs` precedent).
    if d94_covered_hours != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but only {d94_covered_hours} carry a \
             d94 DATA record; every closed hour of a d94-scheduled capture must carry at \
             least one, and this harness cannot validate the payloads of unmodelled domains \
             to see where the difference went"
        ));
    }
    // Both an aggregate and a PER-HOUR coverage check, because aggregate counts
    // alone would accept a compensating duplicate — two d18 records in one hour
    // and none in another — whose totals still match (Codex round-1 finding).
    if d18_records != 0 && (d18_records != closed_hours || d18_covered_hours != closed_hours) {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d18_records} d18 DATA \
             record(s) covering {d18_covered_hours} hour(s); a capture scheduling d18 \
             carries exactly one per closed hour, and a partially-missing d18 lane would \
             silently convert its hours' d94 records into no-roster exclusions"
        ));
    }

    Ok(scan)
}

/// One encoding arm: a BASELINE every `block_hours` measured hours and a
/// stored-minimal DELTA between, with the reader replay that turns the size into
/// a claim about decodable bytes.
///
/// Both arms of this measurement go through this one implementation and differ
/// only in `block_hours`, which is the point: for d94 the arm's size is almost
/// entirely a function of re-baseline cadence, so two arms that shared no code
/// would be comparing implementations rather than policies.
struct WireArm {
    block_hours: usize,
    bytes: Vec<u8>,
    offsets: Vec<usize>,
    reader: Option<KeyedState>,
    hours: usize,
    baselines: usize,
    /// Baselines the §1.5 gap rule forced outside a scheduled block start.
    gap_forced_baselines: usize,
    baseline_bytes: usize,
    empty_hours: usize,
    changed_rows: usize,
    tombstones: usize,
}

impl WireArm {
    fn new(block_hours: usize) -> Self {
        Self {
            block_hours,
            bytes: Vec::new(),
            offsets: vec![0usize],
            reader: None,
            hours: 0,
            baselines: 0,
            gap_forced_baselines: 0,
            baseline_bytes: 0,
            empty_hours: 0,
            changed_rows: 0,
            tombstones: 0,
        }
    }

    fn push_hour(
        &mut self,
        schema: &wire_format::Schema,
        collection: &wire_format::Collection,
        prior: Option<&KeyedState>,
        state: &KeyedState,
        game_hour: i32,
        follows_previous: bool,
    ) {
        let hour_start = self.bytes.len();
        // BASELINE at each block start — and at every §1.5 gap, which forces a
        // fresh baseline and restarts the delta chain (`opens_baseline`).
        let block_start = opens_baseline(self.hours, self.block_hours, follows_previous);
        if block_start && self.hours % self.block_hours != 0 {
            self.gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if block_start || prior.is_none() {
            encode_baseline(schema, std::slice::from_ref(state), &mut self.bytes)
                .expect("baseline encodes");
            self.baselines += 1;
            self.baseline_bytes += self.bytes.len() - hour_start;
        } else {
            let prev = prior.expect("checked above");
            let d = diff(collection, prev, state);
            if d.is_empty() {
                self.empty_hours += 1;
                emitted = false;
            } else {
                self.changed_rows += d.changed.len();
                self.tombstones += d.tombstones.len();
                encode_delta(schema, std::slice::from_ref(prev), &[Some(d)], &mut self.bytes)
                    .expect("delta encodes");
            }
        }
        self.offsets.push(self.bytes.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require
        // the result to be stored-equal to this hour's state. An hour that
        // emitted nothing is still checked: the reader's retained state must
        // already equal the fold's, which is precisely what "nothing changed"
        // asserts.
        if emitted {
            let hour_bytes = &self.bytes[hour_start..];
            if block_start || self.reader.is_none() {
                let (states, consumed) =
                    decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                self.reader = Some(states.into_iter().next().expect("one collection"));
            } else {
                let reader_prior = self.reader.take().expect("checked above");
                let (deltas, consumed) =
                    decode_delta(schema, std::slice::from_ref(&reader_prior), hour_bytes, 0)
                        .expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                let mut next = reader_prior;
                apply(&mut next, deltas[0].as_ref().expect("one collection"));
                self.reader = Some(next);
            }
        }
        let replayed = self.reader.as_ref().expect("a baseline opened the arm");
        assert_eq!(replayed.len(), state.len(), "game hour {game_hour}: row count");
        let residual = diff(collection, replayed, state);
        assert!(
            residual.is_empty(),
            "game hour {game_hour}: reader state is not stored-equal to the fold"
        );
        self.hours += 1;
    }

    fn delta_hours(&self) -> usize {
        self.hours - self.baselines - self.empty_hours
    }

    fn delta_bytes(&self) -> usize {
        self.bytes.len() - self.baseline_bytes
    }

    fn write(&self, out_dir: &str, stem: &str) {
        let mut f =
            std::fs::File::create(std::path::Path::new(out_dir).join(format!("{stem}.bin")))
                .expect("create arm");
        f.write_all(&self.bytes).expect("write arm");
        let mut f =
            std::fs::File::create(std::path::Path::new(out_dir).join(format!("{stem}.offsets")))
                .expect("create offsets");
        for o in &self.offsets {
            writeln!(f, "{o}").expect("write offsets");
        }
    }

    fn report(&self, label: &str, ship_bytes: usize) {
        println!(
            "{label:<24} {} bytes ({:.2}% of shipping)  {} baseline(s) ({} \u{a7}1.5 \
             gap-forced) {} B, {} delta hour(s) {} B, {} empty hour(s)",
            self.bytes.len(),
            100.0 * self.bytes.len() as f64 / ship_bytes.max(1) as f64,
            self.baselines,
            self.gap_forced_baselines,
            self.baseline_bytes,
            self.delta_hours(),
            self.delta_bytes(),
            self.empty_hours
        );
    }
}

/// Re-fold one measured hour's payload into its keyed state, through the same
/// `d94` boundary pass 1 used. Pass 2 calls this instead of retaining 13.9M
/// records across the whole capture.
fn refold_hour(
    stream: &[u8],
    hour: &D94Hour,
    collection: &wire_format::Collection,
    roster: &d94::Roster,
) -> KeyedState {
    let payload = &stream[hour.payload_span.clone()];
    let mut state = d94::State::default();
    state
        .fold_payload(payload, roster)
        .expect("pass 1 accepted this payload with this roster");
    state
        .keyed_state(collection)
        .expect("pass 1 accepted this payload")
        .expect("pass 1 accepted this payload")
}

fn main() {
    const USAGE: &str = "usage: bench_real_d94 <collector_stream.bin> <out dir> \
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
    let schema = d94::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream, model d18, classify every d94
    // record. No artifact is created before this succeeds. ---------------------
    let scan = match scan_stream(&stream, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // Pass 2 re-folds each measured hour's payload. Pass 1 already proved every
    // one of them folds with its own hour's roster, and a roster gate can only
    // make a fold FAIL, never change the snapshot it produces, so pass 2 replays
    // with a roster that admits the hour it already accepted. This is stated
    // rather than assumed: the assertion in `refold_hour` fails loudly if pass 1
    // and pass 2 ever disagree.
    let mut pass2_roster_ids: BTreeSet<u32> = BTreeSet::new();
    for pov in scan.pov_countries.keys() {
        pass2_roster_ids.insert(*pov);
    }
    let pass2_roster = d94::Roster::measured_this_hour(pass2_roster_ids);

    // ---- Pass 2: encode all three arms. -------------------------------------
    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    // The 48-hour arm every other bin in this family writes, and the
    // one-baseline-per-run arm that isolates grammar from re-baseline policy.
    let mut blocked = WireArm::new(BLOCK_HOURS);
    let mut single = WireArm::new(usize::MAX);

    let mut prior: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    let mut slots_total = 0usize;
    // Cadence-independent churn: over every measured-hour transition, taken
    // before the block decision, so a change landing on a block boundary still
    // counts.
    let mut transition_changed_rows = 0usize;
    let mut transition_tombstones = 0usize;
    let mut transitions = 0usize;
    let mut unchanged_transitions = 0usize;
    let mut max_transition_changed = 0usize;

    for hour in &scan.hours {
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        let state = refold_hour(&stream, hour, collection, &pass2_roster);
        slots_total += state.len();

        // Cadence-independent churn, before the block decision.
        if let Some(prev) = prior.as_ref() {
            let d = diff(collection, prev, &state);
            transitions += 1;
            transition_changed_rows += d.changed.len();
            transition_tombstones += d.tombstones.len();
            if d.is_empty() {
                unchanged_transitions += 1;
            }
            max_transition_changed = max_transition_changed.max(d.changed.len());
        }

        blocked.push_hour(
            &schema,
            collection,
            prior.as_ref(),
            &state,
            hour.game_hour,
            hour.follows_previous,
        );
        single.push_hour(
            &schema,
            collection,
            prior.as_ref(),
            &state,
            hour.game_hour,
            hour.follows_previous,
        );

        prior = Some(state);
        hours_seen += 1;
    }

    {
        let mut f =
            std::fs::File::create(std::path::Path::new(&out_dir).join("ship_d94.bin")).unwrap();
        f.write_all(&ship).unwrap();
        let mut f =
            std::fs::File::create(std::path::Path::new(&out_dir).join("ship_d94.offsets")).unwrap();
        for o in &ship_offsets {
            writeln!(f, "{o}").unwrap();
        }
    }
    blocked.write(&out_dir, "wire_new");
    single.write(&out_dir, "wire_single_baseline");

    println!(
        "hours                {hours_seen} measured ({} non-contiguous steps), {} excluded",
        scan.non_contiguous,
        scan.excluded_hours.len()
    );
    println!(
        "d94 records          {} ({} hour(s) carried more than one)",
        scan.d94_records, scan.multi_record_hours
    );
    println!("d94 PERF status      {:?}", scan.perf_status);
    println!(
        "d18 dependency       {} hour(s) measured a roster; roster sizes {:?}; \
         failure-status records {:?}",
        scan.d18_measured_hours, scan.d18_roster_sizes, scan.d18_failure_status
    );
    println!("POV countries        {:?}", scan.pov_countries);
    println!("province slots       {:?}", scan.slot_counts);
    println!(
        "slots avg            {:.1}",
        slots_total as f64 / hours_seen.max(1) as f64
    );
    println!(
        "churn (cadence-independent) {:.2} changed rows/hour, {:.3} tombstones/hour over {} \
         measured-hour transitions ({} unchanged, max {} changed)",
        transition_changed_rows as f64 / transitions.max(1) as f64,
        transition_tombstones as f64 / transitions.max(1) as f64,
        transitions,
        unchanged_transitions,
        max_transition_changed
    );
    println!(
        "churn (delta path, 48 h)    {:.2} changed rows/hour, {:.3} tombstones/hour over {} \
         emitted-delta hours",
        blocked.changed_rows as f64 / blocked.delta_hours().max(1) as f64,
        blocked.tombstones as f64 / blocked.delta_hours().max(1) as f64,
        blocked.delta_hours()
    );
    println!("ship_d94.bin             {} bytes", ship.len());
    blocked.report("wire_new.bin (48 h)", ship.len());
    single.report("wire_single_baseline.bin", ship.len());
    println!(
        "per-hour cost        shipping {:.1} B/h; wire delta path {:.1} B/delta-hour; \
         wire baseline {:.0} B each ({:.2} B/row against the shipping payload's 1.00 B/row)",
        ship.len() as f64 / hours_seen.max(1) as f64,
        blocked.delta_bytes() as f64 / blocked.delta_hours().max(1) as f64,
        blocked.baseline_bytes as f64 / blocked.baselines.max(1) as f64,
        blocked.baseline_bytes as f64
            / (blocked.baselines.max(1) as f64 * (slots_total as f64 / hours_seen.max(1) as f64))
    );
    println!(
        "shipping byte split  {} B d94 payload claimed by the wire arm, {} B H4CR framing \
         no per-collection port can claim ({:.2}%), 0 B stop-reported",
        ship.len() - hours_seen * COMPACT_RECORD_HEADER_LEN,
        hours_seen * COMPACT_RECORD_HEADER_LEN,
        100.0 * (hours_seen * COMPACT_RECORD_HEADER_LEN) as f64 / ship.len() as f64
    );
    for (hour, reason) in &scan.excluded_hours {
        println!("excluded hour {hour}: {reason}");
    }
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
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// A FRAME_END with explicit seal fields, so a test can perturb exactly one.
    /// Every seal in these streams is built here: an all-zero payload would be a
    /// legal seal only for an hour with no DOMAIN_PERF records at all, and most
    /// of these streams have them.
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

    fn perf(sequence: u64, game_hour: i32, domain: u16, status: u32) -> Vec<u8> {
        let mut payload = vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN];
        payload[PERF_STATUS_OFFSET..PERF_STATUS_OFFSET + 4]
            .copy_from_slice(&status.to_le_bytes());
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &payload,
        )
    }

    /// A FOW1 payload, packed as `PovVisibilitySnapshot::encode` packs one.
    fn fow(capture_sequence: u64, pov: u32, levels: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"FOW1");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&capture_sequence.to_le_bytes());
        out.extend_from_slice(&pov.to_le_bytes());
        out.extend_from_slice(&(levels.len() as u32).to_le_bytes());
        out.extend_from_slice(levels);
        out
    }

    /// A d18 `country_identity` payload carrying `tags` and no CIM1 extension.
    fn d18_payload(status_flags: u32, tags: &[(u32, &str)]) -> Vec<u8> {
        d18_payload_counts(status_flags, tags, tags.len() as u32, tags.len() as u32)
    }

    /// The same, with `row_count` and `changed_rows` written independently, so a
    /// test can perturb exactly one of the two header words.
    fn d18_payload_counts(
        status_flags: u32,
        tags: &[(u32, &str)],
        row_count: u32,
        changed_rows: u32,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        for (tag_db_id, tag) in tags {
            body.extend_from_slice(&tag_db_id.to_le_bytes());
            body.extend_from_slice(&(tag.len() as u16).to_le_bytes());
            body.extend_from_slice(tag.as_bytes());
            body.push(0); // cosmetic marker: absent
        }
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&row_count.to_le_bytes());
        payload.extend_from_slice(&changed_rows.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&body);
        payload
    }

    /// The same, with a CIM1 extension whose three families are all available and
    /// whose state list is empty — the smallest well-formed extension.
    fn d18_payload_with_cim1(tags: &[(u32, &str)]) -> Vec<u8> {
        let mut payload = d18_payload(0, tags);
        payload.extend_from_slice(b"CIM1");
        payload.extend_from_slice(&1u16.to_le_bytes()); // version
        payload.extend_from_slice(&40u16.to_le_bytes()); // header len
        payload.extend_from_slice(&0u32.to_le_bytes()); // claims status
        payload.extend_from_slice(&0u32.to_le_bytes()); // cores status
        payload.extend_from_slice(&0u32.to_le_bytes()); // major status
        payload.extend_from_slice(&0u32.to_le_bytes()); // state count
        payload.extend_from_slice(&0u32.to_le_bytes()); // expected claim memberships
        payload.extend_from_slice(&0u32.to_le_bytes()); // expected core memberships
        payload.extend_from_slice(&(tags.len() as u32).to_le_bytes()); // major count
        payload.extend_from_slice(&0u32.to_le_bytes()); // reserved
        for (tag_db_id, _) in tags {
            payload.extend_from_slice(&tag_db_id.to_le_bytes());
            payload.push(1); // major
            payload.extend_from_slice(&[0, 0, 0]);
        }
        payload
    }

    const ROSTER: [(u32, &str); 2] = [(7, "GER"), (368, "SOV")];

    /// One healthy hour: d18 then d94, PERF for both, a sealed FRAME_END.
    fn hour(sequence: &mut u64, game_hour: i32, capture_sequence: u64, levels: &[u8]) -> Vec<u8> {
        let mut s = Vec::new();
        let mut next = || {
            *sequence += 1;
            *sequence
        };
        s.extend(perf(next(), game_hour, D18, 0));
        s.extend(perf(next(), game_hour, D94, 0));
        s.extend(record(
            next(),
            game_hour,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(0, &ROSTER),
        ));
        s.extend(record(
            next(),
            game_hour,
            D94,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &fow(capture_sequence, 368, levels),
        ));
        s.extend(frame_end_sealed(next(), game_hour, 2, 2, 0, 0));
        s
    }

    fn good_stream() -> Vec<u8> {
        let mut sequence = 0u64;
        let mut s = Vec::new();
        s.extend(hour(&mut sequence, 100, 1, &[1, 2, 3]));
        s.extend(hour(&mut sequence, 101, 2, &[1, 2, 4]));
        s
    }

    fn boundary(stream: &[u8], records: usize) -> usize {
        let mut pos = 0usize;
        for _ in 0..records {
            let h = parse_compact_record_header(&stream[pos..]).expect("framed");
            pos += COMPACT_RECORD_HEADER_LEN + h.payload_len;
        }
        pos
    }

    #[test]
    fn a_well_formed_stream_yields_its_d94_hours() {
        let scan = scan_stream(&good_stream(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.excluded_hours.len(), 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.d94_records, 2);
        assert_eq!(scan.d18_measured_hours, 2);
        assert_eq!(scan.perf_status.get(&0), Some(&2));
        assert_eq!(scan.pov_countries.get(&368), Some(&2));
        assert_eq!(scan.slot_counts.get(&3), Some(&2));
    }

    #[test]
    fn a_cim1_extension_is_walked_rather_than_refused_as_trailing_bytes() {
        // Every d18 record of the reference capture carries one, so a mirror that
        // stopped after the variable rows would refuse all 1,033 hours on the
        // fold's `cursor != payload.len()` rule and measure nothing.
        let outcome = fold_country_identity(&d18_payload_with_cim1(&ROSTER)).expect("parses");
        match outcome {
            D18Outcome::Measured(ids) => {
                assert_eq!(ids, [7u32, 368].into_iter().collect::<BTreeSet<_>>())
            }
            D18Outcome::FailureStatus(_) => panic!("a clean record is not a failure"),
        }
        // And a stream built from those records measures its hours.
        let mut s = Vec::new();
        s.extend(perf(1, 100, D18, 0));
        s.extend(perf(2, 100, D94, 0));
        s.extend(record(
            3,
            100,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload_with_cim1(&ROSTER),
        ));
        s.extend(record(
            4,
            100,
            D94,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &fow(1, 368, &[1, 2, 3]),
        ));
        s.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        let scan = scan_stream(&s, StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn an_hour_without_a_current_hour_d18_record_is_excluded_not_measured() {
        // Historically this stream — hour 101 with a d94 record and NO d18
        // record — proved the freshness gate (`world_fold.rs:10512`): the
        // roster persists, so the second hour's d94 record still names a
        // country the map contains, and is refused anyway. The completeness
        // sweep made this shape a REFUSED STREAM instead: a capture that
        // schedules d18 carries exactly one record per closed hour, so a d18
        // record missing outright is a partially-lost or relabelled d18 lane,
        // not an hour to exclude quietly. The freshness-gate distinction stays
        // covered by `a_d18_failure_status_leaves_the_hour_unknown_and_excludes_it`,
        // whose d18 record is PRESENT but status-failed — the only way
        // production reaches the gate on a complete capture.
        let mut s = Vec::new();
        s.extend(hour(&mut 0u64, 100, 1, &[1, 2, 3]));
        // Hour 101: d94 but no d18.
        s.extend(perf(6, 101, D94, 0));
        s.extend(record(
            7,
            101,
            D94,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &fow(2, 368, &[1, 2, 4]),
        ));
        s.extend(frame_end_sealed(8, 101, 1, 1, 0, 0));
        let err = match scan_stream(&s, StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a partially-missing d18 lane must refuse the stream"),
        };
        assert!(err.contains("d18"), "the refusal names the d18 bound: {err}");

        // A COMPENSATING duplicate — two d18 records in hour 100, none in hour
        // 101 — leaves the aggregate count equal to the closed hours and is
        // caught only by the per-hour coverage check (Codex round-1 finding).
        // Hour 101's d94 record is excluded by the freshness gate, so the fold
        // cannot catch the missing roster either.
        let mut s = Vec::new();
        s.extend(perf(1, 100, D18, 0));
        s.extend(perf(2, 100, D94, 0));
        s.extend(record(
            3,
            100,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(0, &ROSTER),
        ));
        s.extend(record(
            4,
            100,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(0, &ROSTER),
        ));
        s.extend(record(
            5,
            100,
            D94,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &fow(1, 368, &[1, 2, 3]),
        ));
        s.extend(frame_end_sealed(6, 100, 2, 2, 0, 0));
        s.extend(perf(7, 101, D94, 0));
        s.extend(record(
            8,
            101,
            D94,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &fow(2, 368, &[1, 2, 4]),
        ));
        s.extend(frame_end_sealed(9, 101, 1, 1, 0, 0));
        let err = match scan_stream(&s, StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a compensating d18 duplicate must refuse the stream"),
        };
        assert!(err.contains("covering"), "the refusal shows the coverage: {err}");
    }

    #[test]
    fn a_d18_failure_status_leaves_the_hour_unknown_and_excludes_it() {
        // `world_fold.rs:13922`-`:13928`: the fold returns Ok, does NOT touch the
        // roster and does NOT set the freshness flag — so d94 folds nothing even
        // though a d18 record was present. Every failure bit, one at a time.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, D18, 0));
            s.extend(perf(2, 100, D94, 0));
            s.extend(record(
                3,
                100,
                D18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d18_payload(1 << bit, &ROSTER),
            ));
            s.extend(record(
                4,
                100,
                D94,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &fow(1, 368, &[1, 2, 3]),
            ));
            s.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
            // A healthy hour behind it, so the run has a measurement at all —
            // a stream whose every d94 hour is excluded is refused outright
            // (`a_stream_with_no_d94_records_is_refused_not_reported_as_zero`).
            s.extend(hour(&mut 5u64, 101, 2, &[1, 2, 3]));
            let scan = scan_stream(&s, StreamIdentityMode::Legacy);
            let scan = scan.unwrap_or_else(|e| panic!("bit {bit}: the stream is well-formed: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
            assert_eq!(scan.excluded_hours.len(), 1, "bit {bit}");
            assert_eq!(scan.excluded_hours[0].0, 100, "bit {bit}");
            assert_eq!(scan.d18_measured_hours, 1, "bit {bit}");
            assert_eq!(scan.d18_failure_status.get(&(1 << bit)), Some(&1), "bit {bit}");
        }
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." Hour 101 carries a
        // failure-status d18 record, so its d94 record is excluded by the
        // freshness gate — the next measured hour must clear
        // `follows_previous`, and `opens_baseline` must open a baseline on it
        // wherever it lands in a block, in BOTH arms: block cadence is policy,
        // the gap-forced baseline is grammar.
        let mut sequence = 0u64;
        let mut s = Vec::new();
        s.extend(hour(&mut sequence, 100, 1, &[1, 2, 3]));
        let next = |sequence: &mut u64| {
            *sequence += 1;
            *sequence
        };
        s.extend(perf(next(&mut sequence), 101, D18, 0));
        s.extend(perf(next(&mut sequence), 101, D94, 0));
        s.extend(record(
            next(&mut sequence),
            101,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(1 << 0, &ROSTER), // failure status -> freshness gate
        ));
        s.extend(record(
            next(&mut sequence),
            101,
            D94,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &fow(2, 368, &[1, 2, 4]),
        ));
        s.extend(frame_end_sealed(next(&mut sequence), 101, 2, 2, 0, 0));
        s.extend(hour(&mut sequence, 102, 3, &[1, 2, 5]));
        let scan = scan_stream(&s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.excluded_hours.len(), 1);
        assert_eq!(scan.non_contiguous, 1);
        assert!(!scan.hours[0].follows_previous, "the first hour follows none");
        assert!(
            !scan.hours[1].follows_previous,
            "hour 102 does not follow hour 100"
        );
        // ...and that is what makes both encoders open a baseline for it, even
        // though it is only the second measured hour of the block — including
        // the single-baseline arm, whose block cadence alone would never
        // baseline again.
        assert!(opens_baseline(1, 48, scan.hours[1].follows_previous));
        assert!(opens_baseline(1, usize::MAX, scan.hours[1].follows_previous));
        // Through the PRODUCTION encode path (`WireArm::push_hour`), not just
        // the decision function: the post-gap hour must be a gap-forced
        // baseline in both arms.
        let schema = wire_format::d94::schema();
        let collection = &schema.collections[0];
        let roster = d94::Roster::measured_this_hour(ROSTER.iter().map(|(id, _)| *id).collect());
        let state0 = refold_hour(&s, &scan.hours[0], collection, &roster);
        let state1 = refold_hour(&s, &scan.hours[1], collection, &roster);
        for block_hours in [48usize, usize::MAX] {
            let mut arm = WireArm::new(block_hours);
            arm.push_hour(
                &schema,
                collection,
                None,
                &state0,
                scan.hours[0].game_hour,
                scan.hours[0].follows_previous,
            );
            assert_eq!(arm.baselines, 1);
            assert_eq!(arm.gap_forced_baselines, 0);
            arm.push_hour(
                &schema,
                collection,
                Some(&state0),
                &state1,
                scan.hours[1].game_hour,
                scan.hours[1].follows_previous,
            );
            assert_eq!(
                arm.baselines, 2,
                "block {block_hours}: the post-gap hour must be a baseline, not a delta"
            );
            assert_eq!(arm.gap_forced_baselines, 1);
        }

        // The decision function over its whole input space: a block boundary
        // always opens one, and so does any gap, at every position in a block.
        for hours_seen in 0..96 {
            assert!(
                opens_baseline(hours_seen, 48, false),
                "a gap at position {hours_seen} must open a baseline"
            );
            assert_eq!(
                opens_baseline(hours_seen, 48, true),
                hours_seen % 48 == 0,
                "contiguous position {hours_seen}"
            );
        }

        // A contiguous run is unaffected: only the block boundary baselines.
        let scan = scan_stream(&good_stream(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.non_contiguous, 0);
        assert!(scan.hours[1].follows_previous);
        assert!(!opens_baseline(1, 48, scan.hours[1].follows_previous));
    }

    #[test]
    fn a_closed_hour_without_a_d94_record_refuses_the_stream() {
        // The cross-domain bound: hour 101 carries the d18 record and another
        // domain's DATA record but no d94 record — structurally well formed, so
        // only the completeness check can see it.
        let mut sequence = 0u64;
        let mut s = Vec::new();
        s.extend(hour(&mut sequence, 100, 1, &[1, 2, 3]));
        let next = |sequence: &mut u64| {
            *sequence += 1;
            *sequence
        };
        s.extend(perf(next(&mut sequence), 101, D18, 0));
        s.extend(record(
            next(&mut sequence),
            101,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(0, &ROSTER),
        ));
        s.extend(record(
            next(&mut sequence),
            101,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end_sealed(next(&mut sequence), 101, 1, 1, 0, 0));
        let err = match scan_stream(&s, StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d94 record must refuse the stream"),
        };
        assert!(
            err.contains("at least one"),
            "the refusal names the completeness rule: {err}"
        );
    }

    #[test]
    fn a_pov_country_outside_the_roster_excludes_the_hour() {
        // `world_fold.rs:10518`-`:10524`.
        let mut s = Vec::new();
        s.extend(perf(1, 100, D18, 0));
        s.extend(perf(2, 100, D94, 0));
        s.extend(record(
            3,
            100,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(0, &[(7, "GER")]),
        ));
        s.extend(record(
            4,
            100,
            D94,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &fow(1, 368, &[1, 2, 3]),
        ));
        s.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        // A healthy hour behind it — see the note in the d18-failure test.
        s.extend(hour(&mut 5u64, 101, 2, &[1, 2, 3]));
        let scan = scan_stream(&s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 101);
        assert_eq!(scan.excluded_hours.len(), 1);
        assert_eq!(scan.excluded_hours[0].0, 100);
        assert!(scan.excluded_hours[0].1.contains("absent from current D18 roster"));
    }

    #[test]
    fn a_second_d94_record_in_one_hour_follows_the_folds_sequence_rule() {
        // `world_fold.rs:10525`-`:10532`. Unlike the full-replace domains whose
        // harnesses reject a second record outright, d94's fold explicitly
        // supports several per hour: a strictly greater sequence REPLACES, and
        // anything else RETRACTS the hour entirely.
        let base = |capture_second: u64| {
            let mut s = Vec::new();
            s.extend(perf(1, 100, D18, 0));
            s.extend(perf(2, 100, D94, 0));
            s.extend(record(
                3,
                100,
                D18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d18_payload(0, &ROSTER),
            ));
            s.extend(record(
                4,
                100,
                D94,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &fow(10, 368, &[1, 2, 3]),
            ));
            s.extend(record(
                5,
                100,
                D94,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &fow(capture_second, 368, &[4, 5, 6]),
            ));
            s.extend(frame_end_sealed(6, 100, 2, 2, 0, 0));
            // A healthy hour behind it — see the note in the d18-failure test.
            s.extend(hour(&mut 6u64, 101, 20, &[7, 7, 7]));
            s
        };
        // A greater sequence: the hour is measured, and it is the SECOND record's
        // bytes that are measured.
        let scan = scan_stream(&base(11), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.multi_record_hours, 1);
        assert_eq!(scan.d94_records, 3);
        let payload = &base(11)[scan.hours[0].payload_span.clone()];
        let (header, body) = d94::split_payload(payload).expect("splits");
        assert_eq!(body, &[4u8, 5, 6], "the later record is the hour's state");
        assert_eq!(header.capture_sequence, 11);
        // Equal and lower both retract the hour: the earlier good record does not
        // stand, so hour 100 is excluded and only hour 101 is measured.
        for capture_second in [10u64, 9, 1] {
            let scan =
                scan_stream(&base(capture_second), StreamIdentityMode::Legacy).expect("well-formed");
            assert_eq!(scan.hours.len(), 1, "sequence {capture_second}");
            assert_eq!(scan.hours[0].game_hour, 101, "sequence {capture_second}");
            assert_eq!(scan.excluded_hours.len(), 1, "sequence {capture_second}");
            assert_eq!(scan.excluded_hours[0].0, 100, "sequence {capture_second}");
            assert!(scan.excluded_hours[0]
                .1
                .contains("not strictly increasing"));
        }
    }

    #[test]
    fn a_malformed_fow1_payload_refuses_the_stream_rather_than_excluding_the_hour() {
        // The second, status-independent authority runs the same decode on every
        // DOMAIN_DATA record and its complaint refuses the stream at seal time
        // (`domain_data_authority.rs:16737`, `stream_fold.rs:593`). Skipping the
        // hour instead would produce a plausible number for a stream the recorder
        // rejects. Every way the codec can refuse, one at a time.
        let good = fow(1, 368, &[1, 2, 3]);
        let mut bad: Vec<(String, Vec<u8>)> = Vec::new();
        for i in 0..4 {
            let mut p = good.clone();
            p[i] ^= 0xFF;
            bad.push((format!("magic byte {i}"), p));
        }
        let mut version = good.clone();
        version[4..6].copy_from_slice(&2u16.to_le_bytes());
        bad.push(("version 2".to_string(), version));
        let mut reserved = good.clone();
        reserved[6..8].copy_from_slice(&1u16.to_le_bytes());
        bad.push(("reserved nonzero".to_string(), reserved));
        let mut zero_sequence = good.clone();
        zero_sequence[8..16].copy_from_slice(&0u64.to_le_bytes());
        bad.push(("zero capture sequence".to_string(), zero_sequence));
        for pov in [0u32, d94::MAX_POV_COUNTRY_ID + 1] {
            let mut p = good.clone();
            p[16..20].copy_from_slice(&pov.to_le_bytes());
            bad.push((format!("pov {pov}"), p));
        }
        let mut truncated = good.clone();
        truncated.pop();
        bad.push(("truncated".to_string(), truncated));
        let mut trailing = good.clone();
        trailing.push(0);
        bad.push(("trailing byte".to_string(), trailing));
        bad.push(("empty vector".to_string(), fow(1, 368, &[])));
        bad.push(("header only".to_string(), good[..24.min(good.len())].to_vec()));
        bad.push(("empty payload".to_string(), Vec::new()));

        for (label, payload) in bad {
            let mut s = Vec::new();
            s.extend(perf(1, 100, D18, 0));
            s.extend(perf(2, 100, D94, 0));
            s.extend(record(
                3,
                100,
                D18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d18_payload(0, &ROSTER),
            ));
            s.extend(record(
                4,
                100,
                D94,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
            assert!(
                scan_stream(&s, StreamIdentityMode::Legacy).is_err(),
                "{label} must refuse the stream, not be skipped"
            );
        }
    }

    #[test]
    fn the_two_codecs_must_agree_about_acceptance_and_content() {
        // `d94::split_payload` restates a grammar the reference library may not
        // import. This is the check that keeps the restatement honest, swept over
        // a synthetic space rather than the capture alone: every single-byte
        // corruption of a well-formed payload, plus a length sweep, plus random
        // bytes. Any disagreement in either direction is an error.
        let good = fow(1, 368, &[0, 1, 2, 3, 255]);
        check_codec_agreement(&good, 0).expect("a good payload agrees");
        for i in 0..good.len() {
            for patch in [0x01u8, 0x7F, 0x80, 0xFF] {
                let mut p = good.clone();
                p[i] ^= patch;
                check_codec_agreement(&p, 0)
                    .unwrap_or_else(|e| panic!("byte {i} ^ {patch:#x}: {e}"));
            }
        }
        for len in 0..good.len() + 4 {
            let mut p = good.clone();
            p.resize(len, 0);
            check_codec_agreement(&p, 0).unwrap_or_else(|e| panic!("len {len}: {e}"));
        }
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..2_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let len = (seed as usize) % 64;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                *byte = seed as u8;
            }
            if len >= 4 && seed & 1 == 0 {
                bytes[..4].copy_from_slice(b"FOW1");
            }
            check_codec_agreement(&bytes, 0).expect("codecs agree on arbitrary bytes");
        }
    }

    #[test]
    fn a_failure_or_unassigned_d94_perf_status_fails_the_run() {
        // The assert-inert capture check. d94's fold reads no status word, so
        // this cannot be an exclusion — it is a refusal to measure a regime the
        // harness has not examined, and it must fire louder than the recorder
        // would rather than quietly.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, D18, 0));
            s.extend(perf(2, 100, D94, 1 << bit));
            s.extend(record(
                3,
                100,
                D18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d18_payload(0, &ROSTER),
            ));
            s.extend(record(
                4,
                100,
                D94,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &fow(1, 368, &[1, 2, 3]),
            ));
            s.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
            assert!(
                scan_stream(&s, StreamIdentityMode::Legacy).is_err(),
                "d94 PERF failure bit {bit}"
            );
        }
        // Every unassigned v1 bit, 15..=31, alone and beside an informational one.
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            for extra in [0u32, 1 << 10] {
                let mut s = Vec::new();
                s.extend(perf(1, 100, D94, (1 << bit) | extra));
                s.extend(record(
                    2,
                    100,
                    D18,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d18_payload(0, &ROSTER),
                ));
                s.extend(record(
                    3,
                    100,
                    D94,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &fow(1, 368, &[1, 2, 3]),
                ));
                s.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
                assert!(
                    scan_stream(&s, StreamIdentityMode::Legacy).is_err(),
                    "d94 PERF unassigned bit {bit} (+{extra:#x})"
                );
            }
        }
        // An informational bit is NOT a failure: the check must reject the fatal
        // and the unassigned, and nothing else.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, D94, informational));
            s.extend(record(
                2,
                100,
                D18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d18_payload(0, &ROSTER),
            ));
            s.extend(record(
                3,
                100,
                D94,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &fow(1, 368, &[1, 2, 3]),
            ));
            s.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
            let scan = scan_stream(&s, StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "informational {informational:#x}");
        }
        // A PERF payload too short to carry the word is refused, not skipped.
        let mut short = Vec::new();
        short.extend(record(
            1,
            100,
            D94,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; PERF_STATUS_OFFSET],
        ));
        short.extend(record(
            2,
            100,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(0, &ROSTER),
        ));
        short.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&short, StreamIdentityMode::Legacy).is_err());
    }

    /// One hour carrying an explicit d18 payload, a clean d94 record and a sealed
    /// FRAME_END — the shape the d18 authority tests perturb.
    fn hour_with_d18(d18: &[u8]) -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(perf(1, 100, D18, 0));
        s.extend(perf(2, 100, D94, 0));
        s.extend(record(3, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, d18));
        s.extend(record(
            4,
            100,
            D94,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &fow(1, 368, &[1, 2, 3]),
        ));
        s.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        s
    }

    #[test]
    fn a_d18_changed_rows_above_row_count_refuses_the_stream() {
        // Round-2 review finding. The d18 FOLD never reads `changed_rows`, so
        // nothing in the fold mirror catches this — but production's
        // status-independent payload authority does
        // (`domain_data_authority.rs:1029`-`:1033`) and its complaint refuses the
        // stream at seal time (`stream_fold.rs:593`-`:611`). Before this check,
        // such a stream was measured here as a healthy hour: a measured-hour vs
        // stream-rejection misclassification.
        //
        // The reviewer's exact input first — row_count=1, changed_rows=2, clean
        // status, one valid roster row for country 368.
        let reported = d18_payload_counts(0, &[(368, "SOV")], 1, 2);
        assert!(
            scan_stream(&hour_with_d18(&reported), StreamIdentityMode::Legacy).is_err(),
            "row_count=1 changed_rows=2 must refuse the stream"
        );
        // Then the whole property, not just that input: every way `changed_rows`
        // can exceed `row_count`, at several row counts, and under both a clean
        // and a failure status (the authority is status-independent).
        for status in [0u32, 1, 1 << 8] {
            for (row_count, changed_rows) in
                [(0u32, 1u32), (1, 2), (1, u32::MAX), (2, 3), (2, u32::MAX), (0, u32::MAX)]
            {
                let tags: Vec<(u32, &str)> = if row_count == 0 {
                    Vec::new()
                } else if row_count == 1 {
                    vec![(368, "SOV")]
                } else {
                    vec![(7, "GER"), (368, "SOV")]
                };
                let payload = d18_payload_counts(status, &tags, row_count, changed_rows);
                assert!(
                    scan_stream(&hour_with_d18(&payload), StreamIdentityMode::Legacy).is_err(),
                    "status {status:#x}, row_count {row_count}, changed_rows {changed_rows}"
                );
                assert!(check_d18_authority_payload(&payload).is_err());
            }
        }
        // And the bound is not off by one in the other direction: equal and lower
        // are both accepted, and the hour is measured.
        for (row_count, changed_rows) in [(2u32, 2u32), (2, 1), (2, 0)] {
            let payload =
                d18_payload_counts(0, &[(7, "GER"), (368, "SOV")], row_count, changed_rows);
            check_d18_authority_payload(&payload)
                .unwrap_or_else(|e| panic!("row_count {row_count} changed_rows {changed_rows}: {e}"));
            let scan = scan_stream(&hour_with_d18(&payload), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("row_count {row_count} changed_rows {changed_rows}: {e}"));
            assert_eq!(scan.hours.len(), 1);
        }
    }

    #[test]
    fn a_failure_status_d18_record_is_still_checked_by_the_payload_authority() {
        // The second half of the round-2 finding, and the same property one layer
        // over: the d18 FOLD returns early on a failure status without walking the
        // body (`world_fold.rs:13922`), but the payload authority walks every row
        // and the whole CIM1 extension WHATEVER the status says. So a
        // failure-status record with a malformed body is a refused stream, even
        // though the fold reads nothing from it. Excluding it before checking the
        // payload let a stream production refuses produce a plausible measurement.
        let malformed: Vec<(&str, Vec<u8>)> = vec![
            ("zero tag_db_id", d18_payload(0, &[(0, "GER")])),
            ("empty tag", d18_payload(0, &[(7, "")])),
            (
                "duplicate tag_db_id",
                d18_payload(0, &[(7, "GER"), (7, "SOV")]),
            ),
            ("changed_rows > row_count", d18_payload_counts(0, &[(7, "GER")], 1, 2)),
        ];
        for (label, clean_body) in &malformed {
            for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
                // Re-stamp the same malformed body with a failure status.
                let mut payload = clean_body.clone();
                payload[16..20].copy_from_slice(&(1u32 << bit).to_le_bytes());
                assert!(
                    check_d18_authority_payload(&payload).is_err(),
                    "{label} under failure bit {bit} must be refused by the authority"
                );
                // And the fold, by itself, is happy to skip it — which is exactly
                // why the authority has to run.
                assert!(
                    matches!(
                        fold_country_identity(&payload),
                        Ok(D18Outcome::FailureStatus(_))
                    ),
                    "{label} under failure bit {bit}: the fold skips the body"
                );
                let mut s = hour_with_d18(&payload);
                s.extend(hour(&mut 5u64, 101, 2, &[1, 2, 3]));
                assert!(
                    scan_stream(&s, StreamIdentityMode::Legacy).is_err(),
                    "{label} under failure bit {bit} must refuse the stream"
                );
            }
        }
        // A WELL-FORMED failure-status record is still just an excluded hour: the
        // authority must reject bad payloads and nothing else.
        let mut s = hour_with_d18(&d18_payload(1, &ROSTER));
        s.extend(hour(&mut 5u64, 101, 2, &[1, 2, 3]));
        let scan = scan_stream(&s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 101);
        assert_eq!(scan.excluded_hours.len(), 1);
    }

    #[test]
    fn a_d18_payload_the_mirror_cannot_reproduce_fails_the_run() {
        // The modelled dependency's honesty check: the mirror cannot tell "the
        // recorder would refuse this too" from "this mirror is wrong", so it
        // refuses rather than silently changing which hours are measured. Swept
        // over the fold's own row-level refusals.
        let mut bad: Vec<(String, Vec<u8>)> = Vec::new();
        bad.push(("zero tag_db_id".to_string(), d18_payload(0, &[(0, "GER")])));
        bad.push(("empty tag".to_string(), d18_payload(0, &[(7, "")])));
        bad.push((
            "duplicate tag_db_id".to_string(),
            d18_payload(0, &[(7, "GER"), (7, "SOV")]),
        ));
        let mut bad_marker = d18_payload(0, &[(7, "GER")]);
        *bad_marker.last_mut().unwrap() = 3;
        bad.push(("invalid cosmetic marker".to_string(), bad_marker));
        let mut trailing = d18_payload(0, &ROSTER);
        trailing.push(0);
        bad.push(("trailing byte that is not CIM1".to_string(), trailing));
        let mut truncated = d18_payload(0, &ROSTER);
        truncated.pop();
        bad.push(("truncated row".to_string(), truncated));
        bad.push(("short header".to_string(), vec![0u8; 19]));
        // A CIM1 whose major roster is not the whole roster (`:14133`).
        let mut short_majors = d18_payload(0, &ROSTER);
        short_majors.extend_from_slice(b"CIM1");
        short_majors.extend_from_slice(&1u16.to_le_bytes());
        short_majors.extend_from_slice(&40u16.to_le_bytes());
        for word in [0u32, 0, 0, 0, 0, 0, 1, 0] {
            short_majors.extend_from_slice(&word.to_le_bytes());
        }
        short_majors.extend_from_slice(&7u32.to_le_bytes());
        short_majors.push(1);
        short_majors.extend_from_slice(&[0, 0, 0]);
        bad.push(("CIM1 major roster incomplete".to_string(), short_majors));

        for (label, payload) in bad {
            assert!(
                fold_country_identity(&payload).is_err(),
                "{label} must be refused by the mirror"
            );
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(record(
                2,
                100,
                D94,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &fow(1, 368, &[1, 2, 3]),
            ));
            s.extend(frame_end_sealed(3, 100, 0, 0, 0, 0));
            assert!(
                scan_stream(&s, StreamIdentityMode::Legacy).is_err(),
                "{label} must fail the run"
            );
        }
    }

    #[test]
    fn the_d18_roster_persists_across_hours_but_its_freshness_does_not() {
        // `world_fold.rs:2999`-`:3002`. The mirror must keep the two apart: a
        // populated roster plus a stale freshness flag is a REFUSAL, and the same
        // roster with a fresh flag folds.
        let mut mirror = D18Mirror::default();
        mirror.observe(
            fold_country_identity(&d18_payload(0, &ROSTER)).expect("parses"),
        );
        assert!(mirror.roster().is_known_this_hour());
        assert!(mirror.roster().contains(368));
        mirror.begin_hour();
        assert!(!mirror.roster().is_known_this_hour());
        assert!(
            mirror.roster().contains(368),
            "the map persists even though the hour is not known"
        );
        assert_eq!(mirror.roster().len(), 2);
        // A failure-status record in the new hour changes neither fact.
        mirror.observe(
            fold_country_identity(&d18_payload(1, &[(1, "ENG")])).expect("parses"),
        );
        assert!(!mirror.roster().is_known_this_hour());
        assert!(mirror.roster().contains(368));
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        // A cut after any record other than a FRAME_END leaves an hour open.
        for records in [1usize, 2, 3, 4, 6, 7, 8, 9] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream.
        for (records, expected_hours) in [(5usize, 1usize), (10, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], StreamIdentityMode::Legacy)
                .expect("closed stream is well-formed");
            assert_eq!(scan.hours.len(), expected_hours, "cut after {records}");
        }
    }

    #[test]
    fn a_stream_ending_mid_record_is_rejected() {
        let full = good_stream();
        let boundaries: Vec<usize> = (0..=10).map(|n| boundary(&full, n)).collect();
        for cut in 1..full.len() {
            if boundaries.contains(&cut) {
                continue;
            }
            assert!(
                scan_stream(&full[..cut], StreamIdentityMode::Legacy).is_err(),
                "mid-record cut at {cut}"
            );
        }
        assert!(
            scan_stream(&[], StreamIdentityMode::Legacy).is_err(),
            "an empty stream is not a measurement"
        );
    }

    #[test]
    fn a_header_the_canonical_parser_rejects_is_rejected_here() {
        for records in 0..10 {
            let at = boundary(&good_stream(), records);
            for (offset, patch) in [
                (0usize, vec![b'X']),
                (4, 2u16.to_le_bytes().to_vec()),
                (6, 33u16.to_le_bytes().to_vec()),
            ] {
                let mut s = good_stream();
                s[at + offset..at + offset + patch.len()].copy_from_slice(&patch);
                assert!(
                    scan_stream(&s, StreamIdentityMode::Legacy).is_err(),
                    "record {records} patched at +{offset} must be refused"
                );
            }
        }
    }

    #[test]
    fn a_sequence_discontinuity_is_rejected() {
        for records in 0..10 {
            let at = boundary(&good_stream(), records);
            for wrong in [0u64, 99] {
                let mut s = good_stream();
                s[at + 8..at + 16].copy_from_slice(&wrong.to_le_bytes());
                assert!(
                    scan_stream(&s, StreamIdentityMode::Legacy).is_err(),
                    "record {records} sequence {wrong}"
                );
            }
        }
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        // `validate_frame_end_seal_fields` (`stream_fold.rs:46`-`:89`), swept.
        let prefix = |seal: Vec<u8>| {
            let mut s = Vec::new();
            s.extend(perf(1, 100, D18, 0));
            s.extend(perf(2, 100, D94, 0));
            s.extend(record(
                3,
                100,
                D18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d18_payload(0, &ROSTER),
            ));
            s.extend(record(
                4,
                100,
                D94,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &fow(1, 368, &[1, 2, 3]),
            ));
            s.extend(seal);
            s
        };
        assert_eq!(
            scan_stream(&prefix(frame_end_sealed(5, 100, 2, 2, 0, 0)), StreamIdentityMode::Legacy)
                .expect("a sealed stream is accepted")
                .hours
                .len(),
            1
        );
        let mut bad: Vec<(String, Vec<u8>)> = vec![
            ("reserved nonzero".to_string(), frame_end_sealed(5, 100, 2, 2, 0, 1)),
            ("emitted != scheduled".to_string(), frame_end_sealed(5, 100, 2, 3, 0, 0)),
            ("counts disagree with observed PERF".to_string(), frame_end_sealed(5, 100, 1, 1, 0, 0)),
        ];
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            bad.push((
                format!("unknown seal status bit {bit}"),
                frame_end_sealed(5, 100, 2, 2, 1 << bit, 0),
            ));
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad.push((
                format!("fatal seal status bit {bit}"),
                frame_end_sealed(5, 100, 2, 2, 1 << bit, 0),
            ));
        }
        for (label, seal) in bad {
            assert!(
                scan_stream(&prefix(seal), StreamIdentityMode::Legacy).is_err(),
                "{label} must refuse the stream"
            );
        }
        // A seal payload of the wrong length, at every length.
        for len in [0usize, 1, 16, 31, 33, 64] {
            let seal = record(
                5,
                100,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0u8; len],
            );
            assert!(
                scan_stream(&prefix(seal), StreamIdentityMode::Legacy).is_err(),
                "a {len}-byte FRAME_END payload is not a seal"
            );
        }
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        // `stream_fold.rs:328`-`:365`, swept over the whole predicate.
        let data18 = |seq, hour| {
            record(
                seq,
                hour,
                D18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d18_payload(0, &ROSTER),
            )
        };
        let data94 = |seq, hour, capture| {
            record(
                seq,
                hour,
                D94,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &fow(capture, 368, &[1, 2, 3]),
            )
        };
        // The probe domain may repeat without counting as a duplicate.
        let mut probe_twice = Vec::new();
        probe_twice.extend(perf(1, 100, PROBE_DOMAIN_ID, 0));
        probe_twice.extend(perf(2, 100, PROBE_DOMAIN_ID, 0));
        probe_twice.extend(data18(3, 100));
        probe_twice.extend(data94(4, 100, 1));
        probe_twice.extend(frame_end_sealed(5, 100, 0, 0, 0, 0));
        assert!(scan_stream(&probe_twice, StreamIdentityMode::Legacy).is_ok());

        // Duplicate non-probe PERF domain inside one hour.
        let mut dup = Vec::new();
        dup.extend(perf(1, 100, D18, 0));
        dup.extend(perf(2, 100, D18, 0));
        dup.extend(data18(3, 100));
        dup.extend(data94(4, 100, 1));
        dup.extend(frame_end_sealed(5, 100, 1, 1, 0, 0));
        assert!(scan_stream(&dup, StreamIdentityMode::Legacy).is_err());

        // A PERF for an hour other than the open one.
        let mut interleaved = Vec::new();
        interleaved.extend(data18(1, 100));
        interleaved.extend(perf(2, 999, D18, 0));
        interleaved.extend(data94(3, 100, 1));
        interleaved.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&interleaved, StreamIdentityMode::Legacy).is_err());

        // A PERF for an hour already closed.
        let mut stale = Vec::new();
        stale.extend(data18(1, 100));
        stale.extend(data94(2, 100, 1));
        stale.extend(frame_end_sealed(3, 100, 0, 0, 0, 0));
        stale.extend(perf(4, 100, D18, 0));
        stale.extend(data18(5, 101));
        stale.extend(data94(6, 101, 2));
        stale.extend(frame_end_sealed(7, 101, 1, 1, 0, 0));
        assert!(scan_stream(&stale, StreamIdentityMode::Legacy).is_err());

        // Two different pending PERF hours before any DATA record opens one.
        let mut conflict = Vec::new();
        conflict.extend(perf(1, 100, D18, 0));
        conflict.extend(perf(2, 101, D94, 0));
        conflict.extend(data18(3, 100));
        conflict.extend(data94(4, 100, 1));
        conflict.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&conflict, StreamIdentityMode::Legacy).is_err());

        // A DATA record must open the hour the pending PERF records bound
        // (`stream_fold.rs:378`-`:386`), in both directions.
        for (perf_hour, data_hour) in [(100i32, 101i32), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, D18, 0));
            s.extend(data18(2, data_hour));
            s.extend(data94(3, data_hour, 1));
            s.extend(frame_end_sealed(4, data_hour, 1, 1, 0, 0));
            assert!(
                scan_stream(&s, StreamIdentityMode::Legacy).is_err(),
                "pending PERF hour {perf_hour} then DATA hour {data_hour}"
            );
        }
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(hour(&mut 0u64, 101, 1, &[1, 2, 3]));
        s.extend(hour(&mut 5u64, 100, 2, &[1, 2, 4]));
        assert!(scan_stream(&s, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_stream_with_no_d94_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(perf(1, 100, D18, 0));
        s.extend(record(
            2,
            100,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d18_payload(0, &ROSTER),
        ));
        s.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&s, StreamIdentityMode::Legacy).is_err());
        // And a stream whose every d94 hour is excluded is equally not a
        // measurement: it must not report zero bytes as a result.
        let mut all_excluded = Vec::new();
        all_excluded.extend(perf(1, 100, D94, 0));
        all_excluded.extend(record(
            2,
            100,
            D94,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &fow(1, 368, &[1, 2, 3]),
        ));
        all_excluded.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&all_excluded, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
        let decapitated = good_stream();
        assert!(
            scan_stream(&decapitated, StreamIdentityMode::IdentityV1).is_err(),
            "an identity-mode stream whose CID1 record is missing must be refused"
        );
        assert_eq!(
            scan_stream(&decapitated, StreamIdentityMode::Legacy)
                .expect("a genuine legacy stream")
                .hours
                .len(),
            2
        );
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
            scan_stream(&identity_stream, StreamIdentityMode::Legacy).is_err(),
            "a CID1 record under the legacy claim must be refused"
        );
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the harness reads a file from disk and must
        // return a diagnosis, never unwind. This covers the d18 mirror too, which
        // walks variable-length rows and a nested extension off the same bytes.
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
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
            let _ = scan_stream(&bytes, StreamIdentityMode::Legacy);
        }
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s, StreamIdentityMode::Legacy);
        }
        // And the d18 mirror alone, over arbitrary bytes: it is the piece with
        // the deepest nesting and the most cursor arithmetic.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..4_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let len = (seed as usize) % 200;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                *byte = seed as u8;
            }
            let _ = fold_country_identity(&bytes);
        }
        // Including payloads whose row_count is enormous but whose body is not.
        for row_count in [1u32, 1_000, u32::MAX] {
            let mut p = d18_payload(0, &ROSTER);
            p[8..12].copy_from_slice(&row_count.to_le_bytes());
            let _ = fold_country_identity(&p);
        }
    }
}
