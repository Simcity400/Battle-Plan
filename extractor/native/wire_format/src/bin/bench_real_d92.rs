//! Measure the wire format against the shipping compact stream on REAL d92
//! `country_preferred_tactic_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d92-scheduled capture
//! (`runs/batch3/deepwar_c`). The stream's own d92 DATA records are the shipping
//! format's cost for the domain: a 164-byte PTC1 header plus, per country, 12
//! fixed bytes and a u16-length-prefixed tactic token — a full snapshot every
//! hour, exactly what the recorder stores today. This tool replays those records
//! into the state `world_fold::fold_country_preferred_tactics` builds and
//! re-encodes every hour with this crate's grammar: a BASELINE at each 48-hour
//! block start, field-granular stored-minimal DELTAs between.
//!
//! **d92 is FULL REPLACE** (`world_fold.rs:10360`-`:10366`): each record carries
//! the whole table and the prior state is discarded, so one record is one state
//! and there is no accumulator to carry across hours. The fold still gets its own
//! public `State`/`fold_payload` boundary in `wire_format::d92`, because the
//! frame walk is grammar (two collections, a variable-length row walk and a
//! modelled cross-domain gate) and grammar has exactly one home.
//!
//! **Two passes, because the symbol table is what pass 1 builds.** §5 requires
//! symbol ids to be file-global and forbids in-stream interning — "it must be
//! fully read before any seek" — so the `tactic_id` vocabulary is collected over
//! the whole validated stream first and frozen into a `d92::Symbols` that pass 2
//! only reads. A domain with content-defined text needs this whatever its size.
//!
//! **Pass 1 cannot call the record mapper**, since the table it would resolve
//! against does not exist yet. It instead applies the fold's own acceptance rules
//! over a **key-only mirror**: `d92::split_payload` (which is the entire PTC1
//! decode, see below), the D18 roster-containment gate over a key-only mirror of
//! the D18 roster, and nothing else. That is enough to decide which hours are
//! measured AND to *prove* pass 2 can never need a symbol pass 1 did not see:
//! both passes walk exactly the same spans through exactly the same
//! `split_payload`, and pass 1 enters every token of every measured hour into the
//! vocabulary. `the_symbol_table_covers_every_token_in_the_stream` tests it.
//!
//! # The modelled D18 dependency
//!
//! `fold_country_preferred_tactics` refuses the hour unless a **proven D18 record
//! landed this same hour** (`world_fold.rs:10320`) and the observed tag set
//! **COVERS** the current D18 roster (`:10351`) — containment, not equality: d92
//! reads the complete gamestate country table while D18 publishes the filtered
//! identity cache, so d92 is a superset and only a MISSING D18 tag is a loss. D18
//! is therefore **modelled**, not asserted-inert: this harness folds D18 itself,
//! as a key-only mirror of `fold_country_identity` (`:13918`-`:14158`), which is
//! all the gate needs. The mirror reproduces, in the fold's own order:
//!
//! - the 20-byte header (`:14155`) and its `compact_status_is_failure` early
//!   return (`:14158`-`:14164`) — note this is the PLAIN failure predicate, not
//!   the tolerance-aware one d92 itself uses; the fold consults it IMMEDIATELY
//!   after the header and returns without reading the body at all, so on that
//!   path the roster map is left unchanged, the this-hour KNOWN flag stays clear,
//!   and the record is a FAILURE record however unwalkable its body is;
//! - the variable row walk: `tag_db_id` u32, a u16-length-prefixed UTF-8 tag, and
//!   the wave-4 cosmetic marker `0`/`1`/`2` with its optional string
//!   (`:13930`-`:13985`), including the zero-id, empty-tag and duplicate-id
//!   rejections;
//! - the optional CIM1 extension in full (`:13990`-`:14140`) — its header, its
//!   three independent status words, its safety bounds, its status/count
//!   contract, its strictly-ordered state rows and roster-validated members, its
//!   membership totals and its complete-major-roster rule. The mirror keeps no
//!   claims, cores or majors, but it must WALK them, because the walk is what
//!   decides whether the fold reaches `:14156` and sets the KNOWN flag at all;
//! - the exact-length check (`:14142`) and the full replace of the roster
//!   (`:14150`), then KNOWN := true (`:14156`);
//! - and `country_identity_known_this_hour := false` at every hour boundary
//!   (`:3002`), while the roster map itself deliberately PERSISTS (`:2999`).
//!
//! A D18 payload this mirror cannot parse is **tolerated exactly as the recorder
//! tolerates it, and reported**. D18 has no special `Err` arm in
//! `fold_domain_data`, so it lands on the generic one — `Err(err) =>
//! self.note(...)` (`world_fold.rs:5016`) — which notes and continues: the
//! roster map is left unchanged and the KNOWN flag stays clear, so that hour's
//! d92 record takes its "no proven D18 record" path and is an excluded hour.
//! Refusing the stream instead would make this harness stricter than the
//! recorder, which is its own defect. The mirror's fold is all-or-nothing, so an
//! `Err` cannot leave a half-applied roster behind; the count and the first
//! reason are printed, because every occurrence changes WHICH d92 hours are
//! measured and a reader of the measurement must be able to see that. It fires 0
//! times on the measured capture.
//!
//! **The D18 payload AUTHORITY is a second, status-independent layer, and only
//! the checks the fold does not make live there.**
//! `decode_compact_country_identity_data_payload`
//! (`domain_data_authority.rs:1013`) runs on every d18 DOMAIN_DATA record whatever
//! its status, and its complaint clears the stream's decodable verdict. Its walk is
//! the mirror's walk with exactly one addition — `changed_rows > row_count`
//! (`:1029`-`:1033`), a header word `fold_country_identity` never reads — so
//! [`check_d18_payload_authority`] applies that one rule fail-closed (d65's rule:
//! the fold has no behaviour to preserve) while every other authority refusal keeps
//! the fold's classification and is reported as a named deviation instead (d49's
//! rule). Two of those deviations are counted: an unparseable payload the recorder
//! tolerates, and a failure-status record whose body the authority would reject but
//! the fold never reads. Both are 0 on the measured capture. Ported from
//! `bench_real_d96.rs`, whose Codex rounds 1-3 established all three rulings.
//!
//! # The three outcomes, by name
//!
//! Every path on which the fold does not fold is classified, and d92's are not
//! d63's:
//!
//! 1. **Stream rejection.** A PTC1 payload that does not decode. The fold calls
//!    `preferred_tactic::decode_payload` **before** it reads the status word
//!    (`world_fold.rs:10309`), and production's own status-independent authority
//!    decodes every d92 record whatever its status
//!    (`domain_data_authority.rs:13195`-`:13212`) and clears the stream's
//!    `decodable` verdict on a parse error (`main.rs:16628`). So a malformed
//!    payload invalidates the run rather than skipping an hour — d83's shape, not
//!    d63's, and the reason [`d92::split_payload`] carries the WHOLE decode and
//!    there is no cheaper header-only exit to preserve. Framing, boundary, seal,
//!    EOF and unassigned-status-bit violations are stream rejections too, as is a
//!    d18 record whose `changed_rows` exceeds its `row_count`
//!    ([`check_d18_payload_authority`]).
//! 2. **Excluded hour — failure status.** `handle_compact_failure`
//!    (`world_fold.rs:10311`), through `compact_contract`'s own tolerance-aware
//!    predicate rather than a copied bit mask. Position matters and is the fold's:
//!    AFTER the decode, BEFORE the roster gates.
//! 3. **Excluded hour — the D18 gates.** No proven D18 record this hour
//!    (`:10320`), or a roster that does not cover it (`:10351`). Both return
//!    `Err` from the fold, and the recorder's d92 `Err` arm
//!    (`:4897`-`:4906`) marks the domain unknown for the hour and continues — so
//!    these are excluded hours, NOT stream rejections. Both are counted and
//!    reported separately.
//!
//! Hour uniqueness is enforced over EVERY d92 DATA record, of every status,
//! BEFORE any exclusion, or a success+failure pair in one hour slips through.
//!
//! # What else is validated
//!
//! Nothing here re-derives a contract it can call. Row and payload decoding go
//! through `wire_format::d92`'s public boundary; H4CR framing, record semantics
//! and sequence continuity go through `compact_contract`'s own parser and
//! stateful validator; failure classification goes through `compact_contract`'s
//! own predicates rather than a copied mask. The whole stream is validated before
//! any artifact is written, so a truncated or spliced stream cannot yield a
//! smaller but entirely plausible number. `scan_stream` enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`;
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`;
//! - the hour-boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`stream_fold.rs:277`), its DOMAIN_PERF hour binding (`:328`-`:365`) and the
//!   DATA arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal-field validation (`:46`-`:89`, `:401`-`:409`) and the seal's
//!   domain-count cross-check against distinct non-probe PERF domains
//!   (`:411`-`:419`);
//! - the EOF rules of `eof_check` (`:459`);
//! - and the whole PTC1 grammar, on every d92 record, via `d92::split_payload`.
//!
//! That is a documented **subset** of `recorder_host::stream_fold::verify_stream_logical_boundary`
//! (`:537`), which `wire_format` must not depend on. The DATA payloads of every
//! domain other than 92 and 18 pass UNVALIDATED beyond header sanity, the
//! stream validator and the hour-boundary rules — production validates every
//! domain's payload (`stream_fold.rs:593`); this harness owns only d92's
//! grammar and the d18 mirror. That unvalidated remainder is explicitly
//! BOUNDED: every closed game hour must carry exactly one d92 DATA record of
//! any status, and the d18 lane must be all-or-nothing — exactly one record
//! per closed hour when scheduled (the completeness checks in [`scan_stream`]),
//! so a d92 or d18 record relabelled to another domain id — or dropped —
//! refuses the run instead of silently shrinking it or silently converting
//! hours into no-roster exclusions.
//!
//! # Two properties this run reports rather than relies on
//!
//! - **`source_sequence == hour_index_from_raw(game_hour_raw) + 1`.** The
//!   collector computes it that way (`attach_log.rs:33335`), which would make it
//!   a §6 derived field costing zero in every delta. The fold enforces nothing of
//!   the kind, so `d92.rs` carries it as a plain value and this harness MEASURES
//!   the identity instead. Any violation is counted and printed, never assumed
//!   away.
//! - **Global `tactic_database_id` ↔ `tactic_id` stability.** The codec requires
//!   the mapping to be injective within ONE payload
//!   (`preferred_tactic.rs:203`-`:209`) and nothing extends that across payloads,
//!   which is why `d92.rs` carries both fields. Whether the mapping happens to be
//!   globally constant on THIS capture is measured and printed — a property of
//!   one stream, never a licence to drop a field.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d92.bin   — the d92 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_is_failure,
    compact_status_unknown_flags_v1, parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d92::{self, Symbols};

const D92: u16 = 92;
const D18: u16 = 18;
const BLOCK_HOURS: usize = 48;
const FRAME_END_KNOWN_STATUS_MASK: u32 = (1 << 14) - 1;
const PROBE_DOMAIN_ID: u16 = 250;

// --- FRAME_END seal (`stream_fold.rs:46`-`:89`). -----------------------------

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

// --- The D18 key-only roster mirror. -----------------------------------------

/// A key-only mirror of `fold_country_identity` (`world_fold.rs:13918`).
///
/// Holds exactly what d92's gate reads: the roster's KEY set, and whether a
/// proven D18 record landed this hour. Tags, cosmetic tags, claims, cores and
/// majors are walked (the walk decides whether the fold succeeds at all) and then
/// discarded.
#[derive(Default)]
struct D18Mirror {
    /// `tag_by_db_id`'s key set. Persists across hours (`world_fold.rs:2999`).
    roster: BTreeSet<u32>,
    /// `country_identity_known_this_hour` (`:1934`-`:1944`). Reset each hour.
    known_this_hour: bool,
    /// D18 records whose status made the fold return early (`:13924`).
    failure_records: usize,
    /// Hours in which a proven D18 record landed.
    proven_records: usize,
    /// D18 records this mirror could not parse. The recorder tolerates these
    /// (generic `Err` arm, `world_fold.rs:5016`) and so does this harness, but
    /// they are counted and printed: each one leaves D18 unproven for its hour
    /// and therefore excludes that hour's d92 record.
    model_failures: usize,
    first_model_failure: Option<String>,
    /// Failure-status D18 records whose BODY the payload authority would reject.
    ///
    /// The fold never looks at the body of such a record (`world_fold.rs:14158`
    /// returns before the row walk), so its classification is "failure record"
    /// and this harness takes it — but
    /// `decode_compact_country_identity_data_payload` walks the rows whatever the
    /// status (`domain_data_authority.rs:1042`), so production would not certify
    /// the stream. The recipe's d49 rule: keep the fold's classification, report
    /// the certification consequence as a named deviation. 0 on the measured
    /// capture.
    failure_records_with_unwalkable_body: usize,
    first_failure_body_deviation: Option<String>,
}

/// The D18 payload authority's status-independent checks that the FOLD does not
/// make — applied fail-closed, as a stream rejection.
///
/// `validate_domain_data_payload_at` (`stream_fold.rs:593`) runs
/// `decode_compact_country_identity_data_payload`
/// (`domain_data_authority.rs:1013`) on EVERY d18 DOMAIN_DATA record whatever its
/// status, and a parse error clears the stream's decodable verdict. Its checks are
/// the same walk [`D18Mirror::walk_body`] performs — row truncation, cosmetic
/// markers, zero/empty/duplicate tags, the whole CIM1 extension, trailing bytes —
/// with **one exception**: `changed_rows > row_count` at offset `+12`
/// (`domain_data_authority.rs:1029`-`:1033`), a word `fold_country_identity`
/// never reads at all (it reads `row_count` at `+8` and `status_flags` at `+16`
/// and nothing else from the header, `world_fold.rs:14156`-`:14157`).
///
/// That layering decides the treatment, and the two halves are NOT the same:
///
/// - Where the fold HAS a behaviour, the harness keeps the fold's classification
///   and reports the certification consequence as a named deviation (the recipe's
///   d49 rule). An unparseable d18 payload is that case: the recorder notes it and
///   continues, leaving d18 unproven for the hour, so the harness tolerates it and
///   counts it — while production's certifier would refuse the stream. 0
///   occurrences on the measured capture; a non-zero count is a deviation the
///   reader must weigh, not an accepted stream.
/// - Where the fold has NO behaviour, there is no classification to preserve and
///   the authority predicate applies fail-closed. `changed_rows` is that case:
///   the fold folds such a record normally and adopts its roster, so measuring it
///   would measure a stream production does not certify — d65's rule ("the same
///   defect class is enforced at different layers"). It refuses the stream.
///
/// Ported from `bench_real_d96.rs`, where the rule was Codex round-1 finding
/// `019fa530-c013-74a2-b717-31f8e89e8639` and its over-reach was round-2 finding
/// `019fa53d-daf5-7810-a470-aeeb710e8166`. **This function applies ONLY the checks
/// the fold does not make**, which is the whole of its licence to refuse a stream:
/// a payload shorter than 20 bytes is the FOLD's refusal too
/// (`world_fold.rs:14155`), the recorder tolerates that refusal, and the hour is
/// merely excluded — so a short payload returns `Ok` here and falls through to the
/// mirror, which fails it into the tolerated-and-reported `model_failures` bucket.
fn check_d18_payload_authority(payload: &[u8]) -> Result<(), String> {
    // Not a check: the authority's 20-byte minimum (`domain_data_authority.rs:1018`)
    // is also the FOLD's (`world_fold.rs:14155`), so it is the mirror's business.
    // The length is read here only to know whether the word below exists at all.
    if payload.len() < 20 {
        return Ok(());
    }
    let row_count = u32_at(payload, 8) as usize;
    let changed_rows = u32_at(payload, 12) as usize;
    // `:1029`-`:1033`. The one rule the fold never applies, because it never reads
    // the word.
    if changed_rows > row_count {
        return Err(format!(
            "country_identity data changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    Ok(())
}

impl D18Mirror {
    /// `:3002` — the KNOWN flag is per-frame; the roster map itself persists.
    fn open_hour(&mut self) {
        self.known_this_hour = false;
    }

    /// Fold one D18 DATA payload.
    ///
    /// **All-or-nothing**, like `fold_country_identity`: every rejection returns
    /// before `self.roster` is replaced and before the KNOWN flag is set, so an
    /// `Err` leaves this mirror exactly as it was — which is what the recorder's
    /// generic `Err` arm (`world_fold.rs:5016`) leaves the real accumulator.
    /// The caller reports the reason rather than refusing the stream.
    fn fold(&mut self, payload: &[u8]) -> Result<(), String> {
        // `world_fold.rs:14155` — the fold's own `need(payload, 20, ...)`.
        if payload.len() < 20 {
            return Err("country_identity header truncated".into());
        }
        let status_flags = u32_at(payload, 16);
        // `:14158`-`:14164`. The status check comes FIRST, immediately after the
        // header: the fold notes the failure and returns `Ok` **without reading
        // the body at all**, leaving the roster and the KNOWN flag unchanged.
        // This is the PLAIN failure predicate, which is what the recorder's d18
        // fold uses — not d92's tolerance-aware one. Codex round-3 finding on the
        // d96 port (thread 019fa548-5e33-7061-9236-5c7151f92098): walking the rows
        // first misclassified a failure-status record with an unwalkable body as
        // an unparseable one, contradicting the fold's own non-folding-path
        // classification.
        //
        // The payload AUTHORITY does walk that body whatever the status
        // (`domain_data_authority.rs:1042`), so a body it would reject is a
        // certification deviation — counted and reported here, per the recipe's
        // d49 rule, without changing the classification the fold gives the hour.
        // That is what the ordering claim this comment replaces was really about:
        // what the harness must REPORT, never how it classifies the fold's own
        // outcome.
        if compact_status_is_failure(status_flags) {
            let deviation = self.walk_body(payload).err();
            self.failure_records += 1;
            if let Some(reason) = deviation {
                self.failure_records_with_unwalkable_body += 1;
                if self.first_failure_body_deviation.is_none() {
                    self.first_failure_body_deviation = Some(reason);
                }
            }
            return Ok(());
        }
        let fresh = self.walk_body(payload)?;
        // `:14150` full replace, `:14156` KNOWN := true.
        self.roster = fresh;
        self.known_this_hour = true;
        self.proven_records += 1;
        Ok(())
    }

    /// The body walk itself: rows, the optional CIM1 extension and the
    /// exact-length check, returning the roster key set it proves.
    ///
    /// Separated from [`D18Mirror::fold`] because the two callers need it for
    /// different reasons — the clean path to ADOPT the roster, the failure path
    /// only to see whether the payload authority would have accepted the body.
    /// Non-mutating, so neither can leave a half-applied roster behind.
    fn walk_body(&self, payload: &[u8]) -> Result<BTreeSet<u32>, String> {
        let row_count = u32_at(payload, 8) as usize;
        let mut cursor = 20usize;
        let mut fresh: BTreeSet<u32> = BTreeSet::new();
        for _ in 0..row_count {
            let tag_db_id = read_u32(payload, &mut cursor, "country_identity row tag_db_id")?;
            let tag = read_string(payload, &mut cursor, "country_identity tag")?;
            // `:13936`-`:13947`.
            if tag_db_id == 0 {
                return Err("country_identity row with zero tag_db_id".into());
            }
            if tag.is_empty() {
                return Err(format!("country_identity row {tag_db_id} with empty tag"));
            }
            if !fresh.insert(tag_db_id) {
                return Err(format!("country_identity duplicates tag_db_id {tag_db_id}"));
            }
            // `:13951`-`:13979`: the wave-4 cosmetic marker.
            let marker = read_u8(payload, &mut cursor, "country_identity cosmetic marker")?;
            match marker {
                0 | 1 => {}
                2 => {
                    let cosmetic =
                        read_string(payload, &mut cursor, "country_identity cosmetic")?;
                    if cosmetic.is_empty() {
                        return Err(format!(
                            "country_identity row {tag_db_id} present cosmetic marker with \
                             empty string"
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
        // `:13986`-`:14140`: the optional CIM1 extension.
        if cursor < payload.len() {
            self.walk_cim1(payload, &mut cursor, &fresh)?;
        }
        // `:14142`-`:14147`.
        if cursor != payload.len() {
            return Err(format!(
                "country_identity len {} != consumed {cursor}",
                payload.len()
            ));
        }
        Ok(fresh)
    }

    /// `world_fold.rs:13986`-`:14140`, walked but not retained.
    fn walk_cim1(
        &self,
        payload: &[u8],
        cursor: &mut usize,
        fresh: &BTreeSet<u32>,
    ) -> Result<(), String> {
        const CIM1_HEADER_LEN: usize = 40;
        let base = *cursor;
        if base
            .checked_add(CIM1_HEADER_LEN)
            .is_none_or(|end| end > payload.len())
        {
            return Err("country_identity CIM1 header truncated".into());
        }
        // `:13991`-`:13997`.
        if &payload[base..base + 4] != b"CIM1"
            || u16_at(payload, base + 4) != 1
            || u16_at(payload, base + 6) as usize != CIM1_HEADER_LEN
            || u32_at(payload, base + 36) != 0
        {
            return Err("country_identity CIM1 header is invalid".into());
        }
        let claims_status = u32_at(payload, base + 8);
        let cores_status = u32_at(payload, base + 12);
        let major_status = u32_at(payload, base + 16);
        let state_count = u32_at(payload, base + 20) as usize;
        let expected_claim_memberships = u32_at(payload, base + 24) as usize;
        let expected_core_memberships = u32_at(payload, base + 28) as usize;
        let major_count = u32_at(payload, base + 32) as usize;
        let claims_available = !compact_status_is_failure(claims_status);
        let cores_available = !compact_status_is_failure(cores_status);
        let major_available = !compact_status_is_failure(major_status);
        // `:14003`-`:14009`.
        if state_count > 100_000
            || expected_claim_memberships > 1_000_000
            || expected_core_memberships > 1_000_000
            || major_count > fresh.len()
        {
            return Err("country_identity CIM1 count exceeds safety bound".into());
        }
        // `:14010`-`:14017`.
        if (!claims_available && expected_claim_memberships != 0)
            || (!cores_available && expected_core_memberships != 0)
            || ((!claims_available && !cores_available) && state_count != 0)
            || (!major_available && major_count != 0)
            || (major_available && major_count != fresh.len())
        {
            return Err("country_identity CIM1 status/count contract is invalid".into());
        }
        *cursor = base + CIM1_HEADER_LEN;
        let mut claim_memberships = 0usize;
        let mut core_memberships = 0usize;
        let mut previous_state_id = 0u32;
        // `:14035`-`:14108`.
        for index in 0..state_count {
            let state_id = read_u32(payload, cursor, "country_identity CIM1 state row")?;
            let claim_count =
                read_u32(payload, cursor, "country_identity CIM1 state row")? as usize;
            let core_count =
                read_u32(payload, cursor, "country_identity CIM1 state row")? as usize;
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
            for (count, what) in [(claim_count, "claim"), (core_count, "core")] {
                let mut previous_tag_id = 0u32;
                for member_index in 0..count {
                    let tag_db_id =
                        read_u32(payload, cursor, "country_identity CIM1 state members")?;
                    if tag_db_id == 0
                        || (member_index != 0 && tag_db_id <= previous_tag_id)
                        || !fresh.contains(&tag_db_id)
                    {
                        return Err(format!(
                            "country_identity CIM1 state {state_id} {what} member is invalid"
                        ));
                    }
                    previous_tag_id = tag_db_id;
                }
            }
            claim_memberships += claim_count;
            core_memberships += core_count;
        }
        // `:14106`-`:14111`.
        if claim_memberships != expected_claim_memberships
            || core_memberships != expected_core_memberships
        {
            return Err("country_identity CIM1 membership totals do not match header".into());
        }
        // `:14113`-`:14134`.
        let mut majors: BTreeSet<u32> = BTreeSet::new();
        let mut previous_major_id = 0u32;
        for index in 0..major_count {
            let start = *cursor;
            if start.checked_add(8).is_none_or(|end| end > payload.len()) {
                return Err("country_identity CIM1 major row truncated".into());
            }
            let tag_db_id = u32_at(payload, start);
            let marker = payload[start + 4];
            if tag_db_id == 0
                || (index != 0 && tag_db_id <= previous_major_id)
                || marker > 1
                || payload[start + 5..start + 8] != [0, 0, 0]
                || !fresh.contains(&tag_db_id)
            {
                return Err(format!(
                    "country_identity CIM1 major row {index} is invalid"
                ));
            }
            previous_major_id = tag_db_id;
            majors.insert(tag_db_id);
            *cursor = start + 8;
        }
        // `:14135`-`:14139`.
        if major_available && majors != *fresh {
            return Err("country_identity CIM1 major roster is incomplete".into());
        }
        Ok(())
    }
}

// --- Bounds-checked readers, mirroring the fold's `need`/`read_string`. -------

fn read_u8(payload: &[u8], cursor: &mut usize, label: &str) -> Result<u8, String> {
    let at = *cursor;
    if at >= payload.len() {
        return Err(format!("{label}: truncated"));
    }
    *cursor = at + 1;
    Ok(payload[at])
}

fn read_u32(payload: &[u8], cursor: &mut usize, label: &str) -> Result<u32, String> {
    let at = *cursor;
    let end = at.checked_add(4).ok_or_else(|| format!("{label}: overflow"))?;
    if end > payload.len() {
        return Err(format!("{label}: truncated"));
    }
    *cursor = end;
    Ok(u32_at(payload, at))
}

/// `world_fold.rs:30256`-`:30266`.
fn read_string(payload: &[u8], cursor: &mut usize, label: &str) -> Result<String, String> {
    let at = *cursor;
    let len_end = at.checked_add(2).ok_or_else(|| format!("{label}: overflow"))?;
    if len_end > payload.len() {
        return Err(format!("{label}: truncated"));
    }
    let len = u16_at(payload, at) as usize;
    let end = len_end
        .checked_add(len)
        .ok_or_else(|| format!("{label}: overflow"))?;
    if end > payload.len() {
        return Err(format!("{label}: truncated"));
    }
    *cursor = end;
    core::str::from_utf8(&payload[len_end..end])
        .map(str::to_string)
        .map_err(|e| format!("{label}: invalid utf-8: {e}"))
}

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().expect("2 bytes"))
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().expect("4 bytes"))
}

// --- The scan. ---------------------------------------------------------------

/// One measured d92 hour: its game hour, its byte span in the source stream (the
/// shipping arm is those bytes verbatim, header included), the payload span pass
/// 2 re-folds from, and the D18 roster the gate was satisfied against.
///
/// The roster is RECORDED rather than recomputed, so pass 2 folds under exactly
/// the roster pass 1 validated.
struct D92Hour {
    game_hour: i32,
    ship_span: Range<usize>,
    payload_span: Range<usize>,
    roster: Vec<u32>,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded hour (failure-status, no-roster, roster-uncovered) or a
    /// non-contiguous hour step — which is what forces the encoder to open a
    /// fresh baseline (`opens_baseline`).
    follows_previous: bool,
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences:
/// the 48-hour block cadence (every block opens with its own baseline, §1.5's
/// closing paragraph), and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap
/// affecting the state part forces the next state observation to be a
/// `BASELINE`." Every hour this domain did not successfully sample is such a
/// gap — each excluded-hour reason, and any non-contiguous hour step — and
/// d92's cadence is hourly, so a measured hour that does not follow the
/// previous measured one by exactly one is preceded by one. Neither a delta
/// nor a retained reader state can be justified across it.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
}

/// One wire-arm hour: the §1.5/block baseline decision, the encode, and the
/// per-hour reader replay over both collections. Factored out of `main` so the
/// tests execute the PRODUCTION encode path over a scanned stream: a mutant
/// that reverts the baseline decision here to block-position-only fails the
/// gap test (a Codex round-1 finding — the earlier tests exercised the
/// decision function but not this callsite).
struct WireHourOutcome {
    opened_baseline: bool,
    /// The baseline sits outside a scheduled block start — §1.5 gap-forced.
    gap_forced: bool,
    emitted: bool,
    bytes: usize,
    changed_by_collection: Vec<usize>,
    tombstones_by_collection: Vec<usize>,
}

#[allow(clippy::too_many_arguments)]
fn encode_wire_hour(
    schema: &wire_format::Schema,
    hours_seen: usize,
    follows_previous: bool,
    game_hour: i32,
    prior: &[KeyedState],
    state: &[KeyedState],
    reader: &mut Option<Vec<KeyedState>>,
    wire: &mut Vec<u8>,
) -> WireHourOutcome {
    let hour_start = wire.len();
    // BASELINE at each block start — and at every §1.5 gap, which forces a
    // fresh baseline and restarts the delta chain (`opens_baseline`).
    let block_start = opens_baseline(hours_seen, follows_previous);
    let gap_forced = block_start && hours_seen % BLOCK_HOURS != 0;
    let mut emitted = true;
    let mut changed_by_collection = vec![0usize; schema.collections.len()];
    let mut tombstones_by_collection = vec![0usize; schema.collections.len()];
    if block_start {
        encode_baseline(schema, state, wire).expect("baseline encodes");
    } else {
        let deltas: Vec<Option<_>> = schema
            .collections
            .iter()
            .enumerate()
            .map(|(index, collection)| {
                let d = diff(collection, &prior[index], &state[index]);
                if d.is_empty() {
                    None
                } else {
                    Some(d)
                }
            })
            .collect();
        if deltas.iter().all(Option::is_none) {
            emitted = false;
        } else {
            for (index, delta) in deltas.iter().enumerate() {
                if let Some(d) = delta {
                    changed_by_collection[index] = d.changed.len();
                    tombstones_by_collection[index] = d.tombstones.len();
                }
            }
            encode_delta(schema, prior, &deltas, wire).expect("delta encodes");
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
            *reader = Some(states);
        } else {
            let reader_prior = reader.take().expect("a delta follows a baseline");
            let (deltas, consumed) =
                decode_delta(schema, &reader_prior, hour_bytes, 0).expect("delta decodes");
            assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
            let mut next = reader_prior;
            for (index, delta) in deltas.iter().enumerate() {
                if let Some(delta) = delta {
                    apply(&mut next[index], delta);
                }
            }
            *reader = Some(next);
        }
    }
    let replayed = reader.as_ref().expect("the first hour is a baseline");
    for (index, collection) in schema.collections.iter().enumerate() {
        assert_eq!(
            replayed[index].len(),
            state[index].len(),
            "game hour {game_hour}: collection {} row count",
            collection.collection_id
        );
        let residual = diff(collection, &replayed[index], &state[index]);
        assert!(
            residual.is_empty(),
            "game hour {game_hour}: collection {} reader state is not stored-equal to the fold",
            collection.collection_id
        );
    }
    WireHourOutcome {
        opened_baseline: block_start,
        gap_forced,
        emitted,
        bytes,
        changed_by_collection,
        tombstones_by_collection,
    }
}

#[derive(Default)]
struct ScanCounts {
    /// d92 records whose payload status marks the domain-hour failed.
    failure_records: usize,
    /// Records refused because no proven D18 record landed that hour.
    no_roster_records: usize,
    /// Records refused because the tag set did not COVER the D18 roster: at
    /// least one D18 tag had no d92 row.
    roster_uncovered_records: usize,
    /// Measured hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Measured records carrying no rows at all.
    rowless_records: usize,
    /// Rows the fold applied, across every measured record.
    rows_folded: usize,
    /// Rows carrying a selected tactic (the rest are the null sentinel).
    selected_rows: usize,
    /// Measured records whose `source_sequence` is NOT `hour_index + 1`.
    source_sequence_violations: usize,
}

struct StreamScan {
    hours: Vec<D92Hour>,
    counts: ScanCounts,
    /// The `tactic_id` vocabulary of the whole stream, frozen before any record
    /// is mapped (§5).
    symbols: Symbols,
    /// Every `(tactic_database_id, tactic_id)` pair observed, for the global
    /// stability report. Reported, never relied on.
    id_to_token: BTreeMap<i32, BTreeSet<String>>,
    d18_failure_records: usize,
    d18_proven_records: usize,
    d18_model_failures: usize,
    d18_first_model_failure: Option<String>,
    d18_failure_records_with_unwalkable_body: usize,
    d18_first_failure_body_deviation: Option<String>,
}

/// Validate the whole stream against the canonical contract and fold every
/// usable d92 DATA record. Every rejection path is an `Err`, never a silently
/// shorter measurement and never a panic.
fn scan_stream(stream: &[u8], identity_mode: StreamIdentityMode) -> Result<StreamScan, String> {
    // Identity is a caller-stated protocol, never inferred from bytes:
    // `stream_fold.rs:220`, `stream_validator.rs:328`.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D92Hour> = Vec::new();
    let mut counts = ScanCounts::default();
    // COMPLETENESS accounting: every d92 and d18 DATA record of ANY status,
    // against every closed game hour — see the checks after the EOF rules. The
    // d18 lane is also tracked PER HOUR: aggregate counts alone would accept a
    // compensating duplicate (two d18 records in one hour, none in another)
    // whose totals still match (a Codex round-1 finding).
    let mut d92_records = 0usize;
    let mut d18_records = 0usize;
    let mut closed_hours = 0usize;
    let mut d18_covered_hours = 0usize;
    let mut hour_saw_d18 = false;
    let mut vocabulary: BTreeSet<String> = BTreeSet::new();
    let mut id_to_token: BTreeMap<i32, BTreeSet<String>> = BTreeMap::new();
    let mut d18 = D18Mirror::default();

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d92 DATA record of ANY status — the uniqueness rule.
    let mut last_d92_hour: Option<i32> = None;
    // Hour of the last d92 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;
    let mut first_roster_mismatch: Option<String> = None;

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
                    // A new game hour: the D18 KNOWN flag resets here, exactly as
                    // `world_fold.rs:3002` resets it at the hour boundary.
                    d18.open_hour();
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
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        // D18 is folded BEFORE d92 is considered, in stream order — which is the
        // recorder's order and the reason the same-hour gate can be satisfied at
        // all.
        if header.domain_id == D18 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d18_records += 1;
            hour_saw_d18 = true;
            // Production's status-independent payload authority, fail-closed and
            // BEFORE the mirror: its complaint refuses the stream at seal time, so
            // a record it rejects is not an hour to skip. Only the checks the fold
            // does not make live there — see [`check_d18_payload_authority`] for
            // why the two halves differ.
            check_d18_payload_authority(payload)
                .map_err(|e| format!("d18 payload at byte {pos}: {e}"))?;
            if let Err(reason) = d18.fold(payload) {
                // MIRROR THE RECORDER, do not out-strict it. D18 has no special
                // `Err` arm in `fold_domain_data`, so it lands on the generic one
                // — `Err(err) => self.note(...)` (`world_fold.rs:5016`) — which
                // records a note and continues. The roster map is left unchanged
                // and `country_identity_known_this_hour` stays clear, so the
                // hour's d92 record then takes its "no proven D18 record" path
                // and is an EXCLUDED HOUR. Returning `Err` here would refuse a
                // stream the recorder accepts, which is its own defect (round-1
                // review finding).
                //
                // It is still never silent: the reason is retained and printed,
                // and any occurrence changes which d92 hours are measured, which
                // a reader of this measurement must be able to see.
                d18.model_failures += 1;
                if d18.first_model_failure.is_none() {
                    d18.first_model_failure = Some(format!(
                        "hour {} at byte {pos}: {reason}",
                        header.game_hour_raw
                    ));
                }
            }
        }

        if header.domain_id == D92 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d92_records += 1;
            // The whole PTC1 decode, first and unconditionally: the fold decodes
            // before it reads the status word (`world_fold.rs:10309`) and
            // production's authority decodes every record whatever its status
            // (`domain_data_authority.rs:13203`). A payload that does not decode
            // is a REJECTED STREAM, not an excluded hour.
            let (ptc_header, rows) = d92::split_payload(payload)
                .map_err(|e| format!("d92 payload at byte {pos}: {e}"))?;

            // One d92 DATA record per game hour, checked over EVERY d92 record
            // and BEFORE any exclusion: gating this on the record being usable
            // would let a success/failure pair through the rule entirely.
            if let Some(previous) = last_d92_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d92 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d92_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. The failure predicate is a
            // classifier over the *assigned* v1 status bits
            // (`status_flags_base.rs:21`-`:24`); an unassigned bit has no defined
            // classification and is not evidence of a future informational
            // status, so a record carrying one refuses the stream rather than
            // falling through to "success".
            let unknown_status = compact_status_unknown_flags_v1(ptc_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d92 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // Exclusion 1: `handle_compact_failure` (`world_fold.rs:10311`),
            // through `compact_contract`'s own tolerance-aware predicate rather
            // than a copied bit mask. Its POSITION is the fold's: after the
            // decode, before the roster gates.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D92,
                ptc_header.status_flags,
            ) != 0
            {
                counts.failure_records += 1;
                pos = record_end;
                continue;
            }

            // Exclusion 2: no proven D18 record this hour (`:10322`).
            if !d18.known_this_hour {
                counts.no_roster_records += 1;
                pos = record_end;
                continue;
            }
            // Exclusion 3: the roster-CONTAINMENT gate (`:10351`). A d92 tag D18
            // filtered out is the normal shape and is FOLDED; only a D18 tag
            // with no d92 row refuses the hour.
            let observed: BTreeSet<u32> = rows.iter().map(|row| row.tag_db_id).collect();
            if d18.roster.difference(&observed).next().is_some() {
                counts.roster_uncovered_records += 1;
                if first_roster_mismatch.is_none() {
                    let only_d92: Vec<u32> =
                        observed.difference(&d18.roster).copied().take(8).collect();
                    let only_d18: Vec<u32> =
                        d18.roster.difference(&observed).copied().take(8).collect();
                    first_roster_mismatch = Some(format!(
                        "hour {}: d92 carries {} tags, D18 roster has {}; \
                         {} only in d92 (first {:?}), {} only in D18 (first {:?})",
                        header.game_hour_raw,
                        observed.len(),
                        d18.roster.len(),
                        observed.difference(&d18.roster).count(),
                        only_d92,
                        d18.roster.difference(&observed).count(),
                        only_d18,
                    ));
                }
                pos = record_end;
                continue;
            }

            // Measured. Everything below is reporting, plus the §5 vocabulary.
            let follows_previous =
                last_measured_hour.is_some_and(|previous| header.game_hour_raw == previous + 1);
            if last_measured_hour.is_some() && !follows_previous {
                counts.non_contiguous += 1;
            }
            last_measured_hour = Some(header.game_hour_raw);

            if rows.is_empty() {
                counts.rowless_records += 1;
            }
            for row in &rows {
                counts.rows_folded += 1;
                if let Some(token) = row.tactic_id.as_deref() {
                    counts.selected_rows += 1;
                    if !vocabulary.contains(token) {
                        vocabulary.insert(token.to_string());
                    }
                    id_to_token
                        .entry(row.tactic_database_id)
                        .or_default()
                        .insert(token.to_string());
                }
            }

            // Reported, not relied on: `source_sequence == hour_index + 1`.
            let expected_sequence = compact_contract::calendar::hour_index_from_raw(
                header.game_hour_raw,
            )
            .and_then(|hour| u64::try_from(hour).ok())
            .map(|hour| hour.saturating_add(1));
            if expected_sequence != Some(ptc_header.source_sequence) {
                counts.source_sequence_violations += 1;
            }

            hours.push(D92Hour {
                game_hour: header.game_hour_raw,
                ship_span: pos..record_end,
                payload_span: payload_start..record_end,
                roster: d18.roster.iter().copied().collect(),
                follows_previous,
            });
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
        // Say WHICH exclusion emptied the measurement: "no usable records" alone
        // cannot distinguish a stream without d92 from one whose every hour the
        // D18 gate refused, and those are different findings.
        return Err(format!(
            "stream carries no usable d92 DATA records: {} d92 DATA records seen, of which \
             {} failure-status, {} with no proven D18 record that hour, {} with a tag set \
             that does not cover the D18 roster; D18 itself contributed {} proven, {} \
             failure-status and {} unparseable records. First mismatch: {}",
            counts.failure_records
                + counts.no_roster_records
                + counts.roster_uncovered_records,
            counts.failure_records,
            counts.no_roster_records,
            counts.roster_uncovered_records,
            d18.proven_records,
            d18.failure_records,
            d18.model_failures,
            first_roster_mismatch.as_deref().unwrap_or("none"),
        ));
    }

    // COMPLETENESS: every closed game hour must carry exactly one d92 DATA
    // record of any status, and the d18 lane must be all-or-nothing — exactly
    // one record per closed hour when scheduled, none at all when not. This
    // bounds what this harness canNOT validate: production runs
    // `validate_domain_data_payload_at` on EVERY domain's DATA payload
    // (`stream_fold.rs:593`), while this harness owns only d92's and d18's
    // grammars, so a record of either domain whose header was relabelled — or
    // lost outright — would otherwise be silently ignored: a lost d92 record
    // shrinks the run, and a lost d18 record silently turns its hour's d92
    // record into a no-roster exclusion. A per-capture completeness assertion,
    // NOT a grammar rule (`bench_real_d65.rs` precedent).
    if d92_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d92_records} d92 DATA \
             record(s); every closed hour of a d92-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of unmodelled domains to see where \
             the difference went"
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
             silently convert its hours' d92 records into no-roster exclusions"
        ));
    }

    // §5: the table is complete before any record is mapped, and never grows
    // afterwards. Pass 2 walks exactly these spans with exactly this
    // `split_payload`, so it cannot need a symbol this pass did not see.
    let symbols = Symbols::from_vocabulary(vocabulary);

    let d18_failure_records = d18.failure_records;
    let d18_proven_records = d18.proven_records;
    let d18_model_failures = d18.model_failures;
    let d18_first_model_failure = d18.first_model_failure.clone();
    let d18_failure_records_with_unwalkable_body = d18.failure_records_with_unwalkable_body;
    let d18_first_failure_body_deviation = d18.first_failure_body_deviation.clone();
    Ok(StreamScan {
        hours,
        counts,
        symbols,
        id_to_token,
        d18_failure_records,
        d18_proven_records,
        d18_model_failures,
        d18_first_model_failure,
        d18_failure_records_with_unwalkable_body,
        d18_first_failure_body_deviation,
    })
}

fn main() {
    const USAGE: &str =
        "usage: bench_real_d92 <collector_stream.bin> <out dir> [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d92::schema();
    let countries = &schema.collections[d92::COUNTRIES_INDEX];
    let frame = &schema.collections[d92::FRAME_INDEX];

    // ---- Pass 1: validate the WHOLE stream and classify its d92 records. ----
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&stream, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: re-fold and encode both arms. ------------------------------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut acc = d92::State::new();
    let mut reader: Option<Vec<KeyedState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut rows_total = 0usize;

    // Churn, reported TWICE. One number cannot answer both questions: the
    // cadence-independent figure is taken over every measured-hour transition,
    // BEFORE the block decision, so a change landing on a block boundary still
    // counts; the delta-path figure is taken over the hours that actually emitted
    // a delta. Dividing by the wrong denominator is a known review finding.
    let mut ci_changed = 0usize;
    let mut ci_tombstones = 0usize;
    let mut ci_transitions = 0usize;
    let mut dp_changed = 0usize;
    let mut dp_tombstones = 0usize;
    let mut frame_changed = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        // The hour's state, through the SAME public boundary the unit tests
        // exercise, under exactly the roster pass 1 validated.
        let prior: Vec<KeyedState> = acc.as_slice().to_vec();
        let roster: BTreeSet<u32> = hour.roster.iter().copied().collect();
        d92::fold_payload(
            &schema,
            &mut acc,
            &stream[hour.payload_span.clone()],
            &roster,
            true,
            0,
            &scan.symbols,
        )
        .unwrap_or_else(|e| panic!("game hour {}: pass 2 fold failed: {e}", hour.game_hour));
        let state = acc.as_slice();
        rows_total += acc.countries().len();

        // Cadence-independent churn, before the block decision.
        if hours_seen > 0 {
            let d = diff(countries, &prior[d92::COUNTRIES_INDEX], &state[d92::COUNTRIES_INDEX]);
            ci_changed += d.changed.len();
            ci_tombstones += d.tombstones.len();
            ci_transitions += 1;
            let f = diff(frame, &prior[d92::FRAME_INDEX], &state[d92::FRAME_INDEX]);
            frame_changed += f.changed.len();
        }

        // The wire-arm hour — baseline decision, encode and reader replay —
        // through the SAME function the tests execute (`encode_wire_hour`).
        let outcome = encode_wire_hour(
            &schema,
            hours_seen,
            hour.follows_previous,
            hour.game_hour,
            &prior,
            state,
            &mut reader,
            &mut wire,
        );
        if outcome.gap_forced {
            gap_forced_baselines += 1;
        }
        if outcome.opened_baseline {
            baselines += 1;
            baseline_bytes += outcome.bytes;
        } else if outcome.emitted {
            dp_changed += outcome.changed_by_collection[d92::COUNTRIES_INDEX];
            dp_tombstones += outcome.tombstones_by_collection[d92::COUNTRIES_INDEX];
            delta_bytes += outcome.bytes;
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d92.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d92.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    let ambiguous_ids: Vec<i32> = scan
        .id_to_token
        .iter()
        .filter(|(_, tokens)| tokens.len() > 1)
        .map(|(id, _)| *id)
        .collect();
    let framing_bytes = scan.hours.len() * COMPACT_RECORD_HEADER_LEN;

    println!(
        "hours                {hours_seen} measured ({} non-contiguous steps)",
        scan.counts.non_contiguous
    );
    println!(
        "excluded hours       {} failure-status, {} no-D18-this-hour, {} D18-not-covered",
        scan.counts.failure_records,
        scan.counts.no_roster_records,
        scan.counts.roster_uncovered_records
    );
    println!(
        "d18 modelled         {} proven records, {} failure-status records, {} unparseable by \
         this mirror{}",
        scan.d18_proven_records,
        scan.d18_failure_records,
        scan.d18_model_failures,
        scan.d18_first_model_failure
            .as_deref()
            .map(|first| format!(" (first: {first})"))
            .unwrap_or_default()
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
        "shipping rows        {} folded ({} selected, {} null sentinel), {} rowless records",
        scan.counts.rows_folded,
        scan.counts.selected_rows,
        scan.counts.rows_folded - scan.counts.selected_rows,
        scan.counts.rowless_records
    );
    println!(
        "tactic symbols       {} (§5 vocabulary, frozen before mapping)",
        scan.symbols.len()
    );
    println!(
        "id/token stability   {} distinct database ids, {} of them ever named more than one \
         token {:?}",
        scan.id_to_token.len(),
        ambiguous_ids.len(),
        ambiguous_ids
    );
    println!(
        "source_sequence      {} of {hours_seen} measured hours violate \
         `hour_index + 1`",
        scan.counts.source_sequence_violations
    );
    println!(
        "countries avg        {:.1} rows/hour",
        rows_total as f64 / hours_seen.max(1) as f64
    );
    println!(
        "baselines            {baselines} ({} at a {BLOCK_HOURS}-h block start, \
         {gap_forced_baselines} forced by a WIRE_FORMAT \u{a7}1.5 state gap)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "churn cadence-indep  {:.2} changed rows/hour, {:.3} tombstones/hour over {} \
         measured-hour transitions",
        ci_changed as f64 / ci_transitions.max(1) as f64,
        ci_tombstones as f64 / ci_transitions.max(1) as f64,
        ci_transitions
    );
    println!(
        "churn delta-path     {:.2} changed rows/hour, {:.3} tombstones/hour over {} \
         delta-emitting hours",
        dp_changed as f64 / delta_hours.max(1) as f64,
        dp_tombstones as f64 / delta_hours.max(1) as f64,
        delta_hours
    );
    println!(
        "frame singleton      changed on {frame_changed} of {ci_transitions} transitions"
    );
    println!(
        "ship_d92.bin         {} bytes ({framing_bytes} of them H4CR record framing, {:.1}%, \
         which no per-collection port can claim)",
        ship.len(),
        100.0 * framing_bytes as f64 / ship.len().max(1) as f64
    );
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len().max(1) as f64
    );
    println!(
        "wire raw split       {baseline_bytes} baseline + {delta_bytes} delta \
         ({:.1}% / {:.1}%)",
        100.0 * baseline_bytes as f64 / wire.len().max(1) as f64,
        100.0 * delta_bytes as f64 / wire.len().max(1) as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A D18 payload in the fold's own legacy shape: a 20-byte header, then per
    /// row a `tag_db_id`, a length-prefixed tag and an absent-cosmetic marker.
    /// No CIM1 trailer, which the fold treats as legal (`world_fold.rs:13986`).
    fn d18_payload(tags: &[(u32, &str)], status_flags: u32) -> Vec<u8> {
        let mut p = vec![0u8; 20];
        p[8..12].copy_from_slice(&(tags.len() as u32).to_le_bytes());
        p[16..20].copy_from_slice(&status_flags.to_le_bytes());
        for (id, tag) in tags {
            p.extend_from_slice(&id.to_le_bytes());
            p.extend_from_slice(&(tag.len() as u16).to_le_bytes());
            p.extend_from_slice(tag.as_bytes());
            p.push(0); // cosmetic marker: absent
        }
        p
    }

    /// A D18 payload with `changed_rows` supplied independently of `row_count`,
    /// for the authority rule the fold never reads.
    fn d18_payload_with_changed_rows(
        tags: &[(u32, &str)],
        status_flags: u32,
        changed_rows: u32,
    ) -> Vec<u8> {
        let mut p = d18_payload(tags, status_flags);
        p[12..16].copy_from_slice(&changed_rows.to_le_bytes());
        p
    }

    /// One canonical H4CR record.
    fn push_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) {
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes()); // version
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // record flags
        out.extend_from_slice(payload);
    }

    /// A whole legacy-identity stream of one-hour frames: D18 then d92 then a
    /// zero-count FRAME_END seal, which is the recorder's own record order.
    fn stream_of(frames: &[(i32, Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut sequence = 1u64;
        for (hour, d18, d92_payload) in frames {
            for (domain, payload) in [(D18, d18), (D92, d92_payload)] {
                push_record(
                    &mut out,
                    sequence,
                    *hour,
                    domain,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    payload,
                );
                sequence += 1;
            }
            push_record(
                &mut out,
                sequence,
                *hour,
                1000,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
            );
            sequence += 1;
        }
        out
    }

    fn ptc_payload(rows: &[(u32, i32, &str)], source_sequence: u64) -> Vec<u8> {
        let mut body = Vec::new();
        let mut selected = 0u32;
        for (tag, id, token) in rows {
            let flags: u16 = if *id > 0 {
                selected += 1;
                d92::FLAG_SELECTED | d92::FLAG_TOKEN_PRESENT
            } else {
                0
            };
            let token = if *id > 0 { *token } else { "" };
            body.extend_from_slice(&tag.to_le_bytes());
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(&flags.to_le_bytes());
            body.extend_from_slice(&(token.len() as u16).to_le_bytes());
            body.extend_from_slice(token.as_bytes());
        }
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // read_time_us
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&selected.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // status_flags
        p.extend_from_slice(&d92::MAGIC);
        p.extend_from_slice(&d92::VERSION.to_le_bytes());
        p.extend_from_slice(&(d92::HEADER_LEN as u16).to_le_bytes());
        p.extend_from_slice(&source_sequence.to_le_bytes());
        p.extend_from_slice(&d92::EXECUTABLE_SHA256);
        p.extend_from_slice(&d92::CONSUMER_EVIDENCE_SHA256);
        p.extend_from_slice(&d92::WRITER_EVIDENCE_SHA256);
        p.extend_from_slice(&d92::IDENTITY_EVIDENCE_SHA256);
        p.extend_from_slice(&body);
        p
    }

    /// The claim pass 1 rests on: a vocabulary built from a payload through
    /// `tokens_in_payload` resolves EVERY row of that payload through
    /// `record_from_row`. Both walk the same `split_payload`, so pass 2 can never
    /// need a symbol pass 1 did not see — which is what makes §5's "no in-stream
    /// interning" a property of this harness rather than a hope.
    #[test]
    fn the_symbol_table_covers_every_token_in_the_stream() {
        let payload = ptc_payload(
            &[
                (1, 5, "shock"),
                (2, 0, ""),
                (3, 6, "blitzkrieg"),
                (4, 5, "shock"),
            ],
            1,
        );
        let symbols = Symbols::from_vocabulary(d92::tokens_in_payload(&payload).expect("walks"));
        let schema = d92::schema();
        let countries = &schema.collections[d92::COUNTRIES_INDEX];
        let (_, rows) = d92::split_payload(&payload).expect("decodes");
        for row in &rows {
            d92::record_from_row(countries, row, 0, &symbols)
                .expect("every token resolves against the pass-1 vocabulary");
        }
        assert_eq!(symbols.len(), 2, "two distinct tokens, deduplicated");
    }

    #[test]
    fn the_d18_mirror_tracks_the_roster_and_the_this_hour_flag() {
        let mut d18 = D18Mirror::default();
        d18.open_hour();
        assert!(!d18.known_this_hour, "a new hour has not measured yet");
        d18.fold(&d18_payload(&[(1, "AAA"), (2, "BBB")], 0))
            .expect("folds");
        assert!(d18.known_this_hour);
        assert_eq!(d18.roster, [1u32, 2].into_iter().collect::<BTreeSet<_>>());

        // A new hour clears the KNOWN flag but PERSISTS the roster map
        // (`world_fold.rs:2999`-`:3002`).
        d18.open_hour();
        assert!(!d18.known_this_hour);
        assert_eq!(d18.roster.len(), 2, "the roster map persists across hours");
    }

    #[test]
    fn a_failure_status_d18_record_leaves_the_roster_untouched() {
        // `world_fold.rs:14158`-`:14164`: the PLAIN failure predicate, an early
        // return, and the map left exactly as it was.
        let mut d18 = D18Mirror::default();
        d18.open_hour();
        d18.fold(&d18_payload(&[(1, "AAA")], 0)).expect("folds");
        d18.open_hour();
        d18.fold(&d18_payload(&[(9, "ZZZ")], COMPACT_STATUS_FAILURE_MASK))
            .expect("returns Ok without folding");
        assert!(!d18.known_this_hour, "no proven record this hour");
        assert_eq!(
            d18.roster,
            [1u32].into_iter().collect::<BTreeSet<_>>(),
            "the failed record's rows are not adopted"
        );
        assert_eq!(d18.failure_records, 1);
    }

    #[test]
    fn a_malformed_d18_payload_errors_without_disturbing_the_mirror() {
        // The mirror's fold is all-or-nothing, exactly like `fold_country_identity`:
        // an `Err` must leave the roster and the KNOWN flag as they were, because
        // that is what the recorder's generic `Err` arm leaves behind
        // (`world_fold.rs:5016`). The CALLER tolerates the error and reports it
        // rather than refusing the stream — the `d18.fold(payload)` call site in
        // `scan_stream`, which counts it into `model_failures` and continues.
        let mut d18 = D18Mirror::default();
        d18.open_hour();
        d18.fold(&d18_payload(&[(1, "AAA")], 0)).expect("folds");
        let roster_before = d18.roster.clone();

        let good = d18_payload(&[(2, "BBB")], 0);
        for cut in 20..good.len() {
            assert!(
                d18.fold(&good[..cut]).is_err(),
                "a {cut}-byte D18 payload must be refused"
            );
            assert_eq!(d18.roster, roster_before, "no partial roster at cut {cut}");
        }
        // A trailing byte the fold's exact-length check rejects (`:14142`). This
        // is the round-1 reviewer's own input: it is read as a truncated CIM1
        // extension, and the recorder tolerates it.
        let mut long = d18_payload(&[(2, "BBB")], 0);
        long.push(0);
        assert!(d18.fold(&long).is_err());
        assert_eq!(d18.roster, roster_before, "a trailing byte adopts nothing");
        assert!(d18.known_this_hour, "the earlier proven record still stands");

        // A bare 20-byte header plus one trailing zero byte — the reviewer's
        // minimal case, on a mirror that has never folded anything.
        let mut fresh = D18Mirror::default();
        fresh.open_hour();
        let mut bare = vec![0u8; 20];
        bare.push(0);
        assert!(fresh.fold(&bare).is_err());
        assert!(fresh.roster.is_empty());
        assert!(!fresh.known_this_hour, "D18 stays unproven for the hour");
    }

    #[test]
    fn a_d18_record_whose_changed_rows_exceeds_row_count_refuses_the_stream() {
        // Ported from `bench_real_d96.rs`, where this was Codex round-1 finding
        // 019fa530-c013-74a2-b717-31f8e89e8639 and the same gap was then flagged
        // here for a sweep. `fold_country_identity` never reads the word, so the
        // mirror folded such a record normally and the hour was measured — while
        // production's status-independent d18 authority rejects it
        // (`domain_data_authority.rs:1029`-`:1033`) and the stream is not
        // certified. Fixed by property, not by the reported input: the whole
        // (row_count, changed_rows, status) space is swept below, and the
        // authority's own predicate is applied fail-closed ahead of the mirror.
        let p92 = |seq| ptc_payload(&[(1, 5, "shock")], seq);

        // One valid row, `changed_rows = 2`, clean status, a valid d92 record.
        let reviewer = d18_payload_with_changed_rows(&[(1, "AAA")], 0, 2);
        let err = match scan_stream(
            &stream_of(&[(100, reviewer, p92(1))]),
            StreamIdentityMode::Legacy,
        ) {
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
        // rule, on any row count, and a failure status does not exempt it (the
        // authority is status-independent).
        for status in [0u32, COMPACT_STATUS_FAILURE_MASK] {
            for row_count in 0..4u32 {
                let tags: Vec<(u32, &str)> = (0..row_count)
                    .map(|index| (index + 1, ["AAA", "BBB", "CCC"][index as usize % 3]))
                    .collect();
                for changed_rows in 0..6u32 {
                    let payload = d18_payload_with_changed_rows(&tags, status, changed_rows);
                    assert_eq!(
                        check_d18_payload_authority(&payload).is_ok(),
                        changed_rows <= row_count,
                        "status {status:#x}, row_count {row_count}, changed_rows {changed_rows}"
                    );
                }
            }
        }
        // A payload too short to carry the word is NOT this helper's business:
        // the fold refuses it and the recorder tolerates that refusal, so it must
        // fall through to the mirror rather than refusing the stream (d96 round-2
        // finding, thread 019fa53d-daf5-7810-a470-aeeb710e8166). Swept over every
        // length the word could straddle.
        for len in 0..20usize {
            assert!(
                check_d18_payload_authority(&vec![0u8; len]).is_ok(),
                "a {len}-byte payload is the mirror's business, not the authority's"
            );
        }

        // And the well-formed case still measures.
        let good = d18_payload_with_changed_rows(&[(1, "AAA")], 0, 1);
        let scan = scan_stream(
            &stream_of(&[(100, good, p92(1))]),
            StreamIdentityMode::Legacy,
        )
        .expect("changed_rows == row_count is legal");
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_failure_status_d18_record_is_classified_by_the_fold_not_by_its_body() {
        // Ported from `bench_real_d96.rs`, Codex round-3 finding
        // 019fa548-5e33-7061-9236-5c7151f92098, whose exact input this carries: a
        // 20-byte d18 payload claiming `row_count = 1` with no row body at all,
        // under a failure status. `fold_country_identity` checks the status
        // immediately after the header and returns `Ok` WITHOUT reading the body
        // (`world_fold.rs:14158`), so the hour is a failure record — not an
        // unparseable one. This bin's mirror walked the rows first and reported
        // `0 failure-status / 1 unparseable` for it.
        let p92 = |seq| ptc_payload(&[(1, 5, "shock")], seq);
        let mut headless = vec![0u8; 20];
        headless[8..12].copy_from_slice(&1u32.to_le_bytes()); // row_count = 1
        headless[16..20].copy_from_slice(&1u32.to_le_bytes()); // STATE_UNAVAILABLE
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let scan = scan_stream(
            &stream_of(&[
                (100, d18.clone(), p92(1)),
                (101, headless.clone(), p92(2)),
                (102, d18, p92(3)),
            ]),
            StreamIdentityMode::Legacy,
        )
        .expect("the recorder accepts this stream");
        assert_eq!(scan.d18_failure_records, 1, "the fold's classification");
        assert_eq!(scan.d18_model_failures, 0, "not an unparseable record");
        // …and the certification consequence is still reported, because the
        // payload authority DOES walk that body (the recipe's d49 rule).
        assert_eq!(scan.d18_failure_records_with_unwalkable_body, 1);
        assert!(scan.d18_first_failure_body_deviation.is_some());
        // The roster is untouched, so hour 101 is excluded and 102 follows a gap.
        assert_eq!(scan.hours.len(), 2, "hours 100 and 102 measure");
        assert_eq!(scan.counts.no_roster_records, 1);
        assert_eq!(scan.counts.roster_uncovered_records, 0);
        assert_eq!(scan.counts.non_contiguous, 1, "so hour 102 follows a gap");
        assert!(!scan.hours[1].follows_previous);

        // A WELL-FORMED failure-status record is a failure record with no
        // deviation at all — the two must not be conflated.
        let clean_failure = d18_payload(&[(1, "AAA")], COMPACT_STATUS_FAILURE_MASK);
        let mut mirror = D18Mirror::default();
        mirror.open_hour();
        mirror.fold(&clean_failure).expect("returns Ok");
        assert_eq!(mirror.failure_records, 1);
        assert_eq!(mirror.failure_records_with_unwalkable_body, 0);
        assert!(!mirror.known_this_hour);
        assert!(mirror.roster.is_empty(), "the failed record adopts nothing");

        // And a truncated HEADER is still an unparseable record, because the
        // fold's own `need(payload, 20, ...)` precedes the status read.
        let mut short = D18Mirror::default();
        short.open_hour();
        assert!(short.fold(&[0u8; 19]).is_err());
        assert_eq!(short.failure_records, 0);
    }

    /// The measured production shape, pinned. On `runs/batch3/deepwar_c` the D18
    /// roster is the collector's FILTERED `country_identity.rows` cache
    /// (356/hour) while d92 reads `domain51_read_complete_country_roster`
    /// (439/hour). d92 is a strict SUPERSET in every one of the 896 hours — 83
    /// extra ids, none ever missing — so the gate is containment
    /// (`world_fold.rs:10351`): a superset folds and keeps every row; a d92
    /// record that misses a D18 tag is refused, because that tag is one the
    /// projection can name and would otherwise publish as unavailable.
    #[test]
    fn a_superset_of_the_d18_roster_folds_and_a_gap_in_it_does_not() {
        let schema = d92::schema();
        let mut acc = d92::State::new();
        let payload = ptc_payload(&[(1, 0, ""), (2, 0, ""), (3, 0, "")], 1);
        let roster: BTreeSet<u32> = [1u32, 2].into_iter().collect();
        d92::fold_payload(
            &schema,
            &mut acc,
            &payload,
            &roster,
            true,
            0,
            &Symbols::empty(),
        )
        .expect("a superset covers the D18 roster");
        assert_eq!(
            acc.countries().len(),
            3,
            "the tag D18 filtered out is kept, not dropped"
        );

        let gapped = ptc_payload(&[(1, 0, ""), (3, 0, "")], 1);
        let roster: BTreeSet<u32> = [1u32, 2].into_iter().collect();
        let err = d92::fold_payload(
            &schema,
            &mut acc,
            &gapped,
            &roster,
            true,
            0,
            &Symbols::empty(),
        )
        .expect_err("a D18 tag with no d92 row is a loss");
        assert!(err.contains("does not cover current D18 roster"), "{err}");
        assert!(
            err.contains("1 of 2 D18 tags missing") && err.contains("d92 carries 2"),
            "the counts are reported: {err}"
        );
    }

    /// The gate `scan_stream` applies is its OWN copy of the fold's rule
    /// (`:815`-`:840`), not a call into `d92::fold_payload`, so the tests above
    /// cannot see it drift. This one drives `scan_stream` end to end over a
    /// synthetic stream carrying the deepwar_c shape: an hour where d92 is a
    /// strict superset of D18 must be MEASURED, and an hour where one D18 tag
    /// has no d92 row must be EXCLUDED and counted.
    #[test]
    fn scan_stream_measures_a_superset_hour_and_excludes_an_uncovered_one() {
        let d18 = d18_payload(&[(1, "AAA"), (2, "BBB")], 0);
        // Hour 1: d92 carries a third tag D18 filtered out, and it is selected —
        // deepwar_c's tag 286 in miniature.
        let superset = ptc_payload(&[(1, 0, ""), (2, 5, "shock"), (3, 6, "blitzkrieg")], 1);
        // Hour 2: the same complete roster minus D18's tag 2.
        let uncovered = ptc_payload(&[(1, 0, ""), (3, 6, "blitzkrieg")], 2);

        let scan = scan_stream(
            &stream_of(&[
                (100, d18.clone(), superset),
                (101, d18.clone(), uncovered),
            ]),
            StreamIdentityMode::Legacy,
        )
        .expect("the superset hour keeps the measurement non-empty");
        assert_eq!(scan.hours.len(), 1, "exactly the superset hour is measured");
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(
            scan.counts.rows_folded, 3,
            "all three rows are folded, including the tag D18 filtered out"
        );
        assert_eq!(scan.counts.selected_rows, 2);
        assert_eq!(
            scan.counts.roster_uncovered_records, 1,
            "the hour that misses a D18 tag is excluded and counted"
        );
        assert_eq!(scan.counts.failure_records, 0);
        assert_eq!(scan.counts.no_roster_records, 0);
        assert_eq!(scan.d18_proven_records, 2);

        // With only the uncovered hour, there is nothing to measure at all, and
        // the harness must say WHICH exclusion emptied it rather than write an
        // artifact.
        let reason = match scan_stream(
            &stream_of(&[(100, d18, ptc_payload(&[(1, 0, "")], 1))]),
            StreamIdentityMode::Legacy,
        ) {
            Err(reason) => reason,
            Ok(scan) => panic!("no hour is measurable, but {} were", scan.hours.len()),
        };
        assert!(
            reason.contains("that does not cover the D18 roster") && reason.contains("1 only in D18"),
            "{reason}"
        );
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." An excluded d92 hour
        // (here: a roster-uncovered record) is such a gap — the next measured
        // hour must clear `follows_previous`, and `opens_baseline` must open a
        // baseline on it wherever it lands in a block. A delta would relate two
        // states across unobserved time.
        let d18 = d18_payload(&[(1, "AAA"), (2, "BBB")], 0);
        let full = |seq| ptc_payload(&[(1, 0, ""), (2, 5, "shock")], seq);
        let uncovered = ptc_payload(&[(1, 0, "")], 2);
        let s = stream_of(&[
            (100, d18.clone(), full(1)),
            (101, d18.clone(), uncovered), // excluded: does not cover D18
            (102, d18.clone(), full(3)),
        ]);
        let scan = scan_stream(&s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.counts.roster_uncovered_records, 1);
        assert_eq!(scan.counts.non_contiguous, 1);
        assert!(!scan.hours[0].follows_previous, "the first hour follows none");
        assert!(
            !scan.hours[1].follows_previous,
            "hour 102 does not follow hour 100"
        );
        // ...and that is what makes the encoder open a baseline for it, even
        // though it is only the second measured hour of the block — through the
        // PRODUCTION encode path, not just the decision function.
        assert!(opens_baseline(1, scan.hours[1].follows_previous));
        let schema = d92::schema();
        let mut acc = d92::State::new();
        let mut priors = Vec::new();
        let mut states = Vec::new();
        for hour in &scan.hours {
            priors.push(acc.as_slice().to_vec());
            let roster: BTreeSet<u32> = hour.roster.iter().copied().collect();
            d92::fold_payload(
                &schema,
                &mut acc,
                &s[hour.payload_span.clone()],
                &roster,
                true,
                0,
                &scan.symbols,
            )
            .expect("pass 1 accepted this payload");
            states.push(acc.as_slice().to_vec());
        }
        let mut wire = Vec::new();
        let mut reader: Option<Vec<KeyedState>> = None;
        let o0 = encode_wire_hour(
            &schema,
            0,
            scan.hours[0].follows_previous,
            scan.hours[0].game_hour,
            &priors[0],
            &states[0],
            &mut reader,
            &mut wire,
        );
        assert!(o0.opened_baseline && !o0.gap_forced);
        let o1 = encode_wire_hour(
            &schema,
            1,
            scan.hours[1].follows_previous,
            scan.hours[1].game_hour,
            &priors[1],
            &states[1],
            &mut reader,
            &mut wire,
        );
        assert!(
            o1.opened_baseline && o1.gap_forced,
            "the post-gap hour must be a gap-forced baseline, not a delta"
        );

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

        // A contiguous run is unaffected: only the block boundary baselines.
        let scan = scan_stream(
            &stream_of(&[(100, d18.clone(), full(1)), (101, d18, full(2))]),
            StreamIdentityMode::Legacy,
        )
        .expect("well-formed");
        assert_eq!(scan.counts.non_contiguous, 0);
        assert!(scan.hours[1].follows_previous);
        assert!(!opens_baseline(1, scan.hours[1].follows_previous));
    }

    #[test]
    fn a_closed_hour_without_a_d92_record_refuses_the_stream() {
        // The cross-domain bound: hour 101 carries the d18 record and another
        // domain's DATA record but no d92 record — structurally well formed, so
        // only the completeness check can see it.
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let full = ptc_payload(&[(1, 0, "")], 1);
        let mut s = Vec::new();
        push_record(&mut s, 1, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(&mut s, 2, 100, D92, COMPACT_RECORD_TYPE_DOMAIN_DATA, &full);
        push_record(
            &mut s,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        );
        push_record(&mut s, 4, 101, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(&mut s, 5, 101, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]);
        push_record(
            &mut s,
            6,
            101,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        );
        let err = match scan_stream(&s, StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d92 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );

        // And a partially-missing d18 lane is refused too: hour 101 carries a
        // d92 record but no d18 record at all.
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let mut s = Vec::new();
        push_record(&mut s, 1, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(
            &mut s,
            2,
            100,
            D92,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &ptc_payload(&[(1, 0, "")], 1),
        );
        push_record(
            &mut s,
            3,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        );
        push_record(
            &mut s,
            4,
            101,
            D92,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &ptc_payload(&[(1, 0, "")], 2),
        );
        push_record(
            &mut s,
            5,
            101,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        );
        let err = match scan_stream(&s, StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a partially-missing d18 lane must refuse the stream"),
        };
        assert!(err.contains("d18"), "the refusal names the d18 bound: {err}");

        // A COMPENSATING duplicate — two d18 records in hour 100, none in hour
        // 101 — leaves the aggregate count equal to the closed hours and is
        // caught only by the per-hour coverage check (Codex round-1 finding).
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let mut s = Vec::new();
        push_record(&mut s, 1, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(&mut s, 2, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(
            &mut s,
            3,
            100,
            D92,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &ptc_payload(&[(1, 0, "")], 1),
        );
        push_record(
            &mut s,
            4,
            100,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        );
        push_record(
            &mut s,
            5,
            101,
            D92,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &ptc_payload(&[(1, 0, "")], 2),
        );
        push_record(
            &mut s,
            6,
            101,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        );
        let err = match scan_stream(&s, StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a compensating d18 duplicate must refuse the stream"),
        };
        assert!(err.contains("covering"), "the refusal shows the coverage: {err}");
    }

    #[test]
    fn a_failure_status_d18_record_is_still_row_walked_but_only_to_report() {
        // This test used to assert the defect: that a failure-status d18 record
        // with a malformed body is an `Err` from the mirror, because "production's
        // payload authority is status-independent". Two separate things were
        // conflated, and the d96 port's round-3 review separated them
        // (thread 019fa548-5e33-7061-9236-5c7151f92098). The authority's
        // status-independence decides what the harness must REPORT; the FOLD's own
        // order — status immediately after the header, `Ok` without reading the
        // body (`world_fold.rs:14158`-`:14164`) — decides how the record is
        // CLASSIFIED. So the body is still walked, and a body the authority would
        // reject is still surfaced, but as a named certification deviation rather
        // than as a reclassification into "unparseable".
        let mut d18 = D18Mirror::default();
        d18.open_hour();
        // Malformed rows under a failure status: a FAILURE record, with the
        // deviation counted.
        let mut malformed = d18_payload(&[(1, "AAA")], 1 << 0);
        let len = malformed.len();
        malformed.truncate(len - 1);
        d18.fold(&malformed)
            .expect("the fold never reads this body, so it is a failure record");
        assert_eq!(d18.failure_records, 1);
        assert_eq!(
            d18.failure_records_with_unwalkable_body, 1,
            "the authority would reject the body: reported, not reclassified"
        );
        assert!(d18.first_failure_body_deviation.is_some());
        // Well-formed failure-status: a failure record with NO deviation, and the
        // roster still untouched.
        assert!(d18.fold(&d18_payload(&[(1, "AAA")], 1 << 0)).is_ok());
        assert_eq!(d18.failure_records, 2);
        assert_eq!(d18.failure_records_with_unwalkable_body, 1);
        assert!(!d18.known_this_hour);
        assert!(d18.roster.is_empty());
    }
}
