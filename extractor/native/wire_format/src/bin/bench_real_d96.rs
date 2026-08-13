//! Measure the wire format against the shipping compact stream on REAL d96
//! `country_preferred_tactic_weight_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a capture scheduling d96, d92 and
//! d18 together (`runs/batch3/deepwar_c`). The stream's own d96 DATA records are
//! the shipping format's cost for the domain: a 276-byte PTW1 header plus, per
//! *selected* country, 40 fixed bytes and a u16-length-prefixed tactic token — a
//! full snapshot every hour, exactly what the recorder stores today. This tool
//! replays those records into the state
//! `world_fold::fold_country_preferred_tactic_weights` builds and re-encodes
//! every hour with this crate's grammar: a BASELINE at each 48-hour block start
//! and at every §1.5 state gap, field-granular stored-minimal DELTAs between.
//!
//! **d96 is FULL REPLACE with no cross-hour accumulator at all**
//! (`tactic_weight_fold.rs:22`-`:25`, `:74`; `world_fold.rs:3102`): the hour's
//! snapshot is cleared at every hour boundary, so one record is one state. The
//! fold still gets its own public `State`/`fold_payload` boundary in
//! `wire_format::d96`, because the frame walk is grammar — two collections, a
//! variable-length row walk, two cross-domain gates and a cross-domain identity
//! join — and grammar has exactly one home.
//!
//! **Two passes, because the symbol table is what pass 1 builds.** §5 requires
//! symbol ids to be file-global and forbids in-stream interning — "it must be
//! fully read before any seek" — so the `tactic_id` vocabulary is collected over
//! the whole validated stream first and frozen into a `Symbols` that pass 2 only
//! reads. The table is the one d92 uses (`d96::Symbols` IS `d92::Symbols`),
//! because the fold's join requires the two domains' tokens to be equal
//! string-for-string; the vocabulary is collected from **both** domains'
//! decodable records, so neither can need a symbol the other did not contribute.
//!
//! **Pass 1 cannot call the record mapper**, since the table it would resolve
//! against does not exist yet. It instead applies the fold's own acceptance rules
//! over key-only mirrors of the two dependencies, plus `d96::split_payload` —
//! which is the entire PTW1 decode, see below. That is enough to decide which
//! hours are measured AND to *prove* pass 2 can never need a symbol pass 1 did
//! not see: both passes walk exactly the same spans through exactly the same
//! `split_payload`, and pass 1 enters every token of every decodable record into
//! the vocabulary. `the_symbol_table_covers_every_token_in_the_stream` tests it.
//!
//! # The two modelled dependencies
//!
//! `fold_country_preferred_tactic_weights` runs three gates before it decodes
//! anything (`world_fold.rs:10542`-`:10560`): a proven D18 record must have
//! landed this hour, a **clean D92 snapshot** must exist for this hour, and its
//! `source_sequence` must be available. Both dependencies are therefore
//! **modelled**, not asserted-inert, and this harness folds both itself.
//!
//! - **D18** is a key-only mirror of `fold_country_identity`
//!   (`world_fold.rs:13918`-`:14158`), re-derived from that function rather than
//!   copied from a sibling bin: the 20-byte header and its plain-failure early
//!   return, the variable row walk with its zero-id/empty-tag/duplicate
//!   rejections and the wave-4 cosmetic marker, the optional CIM1 extension in
//!   full (its three status words, safety bounds, status/count contract, ordered
//!   state rows, roster-validated members, membership totals and complete-major
//!   rule), the exact-length check, the full roster replace and `KNOWN := true`.
//!   The roster map PERSISTS across hours (`:2999`) while the KNOWN flag resets
//!   (`:3002`). d96 needs it twice over — for its own first gate, and because
//!   D92's containment gate is defined against it.
//! - **D92** is folded through this crate's own port, `wire_format::d92`, rather
//!   than through a second reading of PTC1: `d92::split_payload` is the whole
//!   decode and the mirror applies the fold's own gates around it
//!   (`world_fold.rs:10477`-`:10540`) — `handle_compact_failure` first, then the
//!   this-hour D18 flag, then the **containment** gate (D18 ⊆ D92, extras kept).
//!   Only a clean fold publishes `country_preferred_tactics_this_hour` and its
//!   sequence, and the recorder clears both at every hour boundary
//!   (`:3099`-`:3101`), so the mirror does too.
//!
//! **The post-fix roster semantics are what is measured.** D92 carries the
//! complete gamestate roster (439 tags on `deepwar_c`) while D18 publishes the
//! filtered identity cache (356), and the 2026-07-27 fix made D92's gate
//! containment and pointed d96's roster witness at D92's row count rather than
//! D18's. This harness reproduces exactly that: the extras are kept in the D92
//! map, `d96::check_join` validates every one of them, and the run reports how
//! many of them are *selected* — the tag-286 case whose weight row dropping the
//! extras would have left unverified.
//!
//! # The outcomes, by name
//!
//! Every path on which the fold does not fold is classified, and d96's are not
//! d92's:
//!
//! 1. **Stream rejection.** A PTW1 (or PTC1) payload that does not decode.
//!    Production's status-independent payload authority decodes every d96 record
//!    whatever its status and whatever the cross-domain gates say
//!    (`domain_data_authority.rs:13246`-`:13262`, dispatched at `:17130`), and a
//!    parse error is recorded against the stream rather than skipped. So a
//!    malformed payload invalidates the run rather than skipping an hour — which
//!    is why this harness decodes FIRST even though the fold decodes LAST. That
//!    is the authority's order, not the fold's, and the two are kept apart on
//!    purpose. Framing, boundary, seal, EOF and unassigned-status-bit violations
//!    are stream rejections too.
//! 2. **Excluded hour — no proven D18 record this hour** (`world_fold.rs:10543`).
//! 3. **Excluded hour — no clean D92 snapshot this hour**
//!    (`:10550`-`:10560`), reported with D92's own cause: D92 failure-status, D92
//!    with no D18 roster, D92 not covering the D18 roster, or no D92 record ahead
//!    of the d96 record in the hour at all.
//! 4. **Excluded hour — d96's own failure status.** `handle_compact_failure`
//!    (`:10568`) through `compact_contract`'s own tolerance-aware predicate
//!    rather than a copied bit mask. Position matters and is the fold's: after
//!    the decode, after both gates, before the join.
//! 5. **Excluded hour — the PTW1/D92 join.** A roster-count or source-sequence
//!    disagreement, or a selected-tactic identity mismatch
//!    (`tactic_weight_fold.rs:45`-`:73`). Both return `Err` from the fold, and the
//!    recorder's d96 `Err` arm (`world_fold.rs:4918`-`:4925`) marks the domain
//!    unknown for the hour and continues — so these are excluded hours, NOT
//!    stream rejections. They are counted and the first one is printed in full,
//!    because a join failure would silently shrink the measurement.
//!
//! Hour uniqueness is enforced over EVERY d96 DATA record, of every status,
//! BEFORE any exclusion, or a success+failure pair in one hour slips through.
//!
//! # No other writer touches the accumulator
//!
//! `country_preferred_tactic_weights_this_hour` is written in exactly three
//! places, all of them this domain's: `begin_hour` at the hour boundary
//! (`world_fold.rs:3102`), `mark_unavailable` on the domain's `Err` arm
//! (`:4919`) and inside the fold's gates (`:10545`, `:10575`), and
//! `fold_payload` itself (`:10567`). No other domain prunes or patches it — d30's
//! orphan prune (d32's case) has no analogue here — so there is nothing to model
//! or to assert inert. The two dependencies are modelled above.
//!
//! # What else is validated
//!
//! Nothing here re-derives a contract it can call. Row and payload decoding go
//! through `wire_format::d96`'s and `wire_format::d92`'s public boundaries; H4CR
//! framing, record semantics and sequence continuity go through
//! `compact_contract`'s own parser and stateful validator; failure classification
//! goes through `compact_contract`'s own predicates, and
//! [`assert_status_predicates_agree`] proves the library's restated
//! `d96::FAILURE_STATUS_MASK` equals it for every status word. The whole stream is
//! validated before any artifact is written, so a truncated or spliced stream
//! cannot yield a smaller but entirely plausible number. `scan_stream` enforces,
//! fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`;
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`;
//! - the hour-boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`), its DOMAIN_PERF hour binding (`:328`-`:365`) and the
//!   DATA arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal-field validation (`:46`-`:89`, `:401`-`:409`) and the seal's
//!   domain-count cross-check against distinct non-probe PERF domains
//!   (`:411`-`:419`);
//! - the EOF rules of `eof_check` (`:459`);
//! - the whole PTW1 grammar on every d96 record, and the whole PTC1 grammar on
//!   every d92 record, via the two ports' `split_payload`.
//!
//! That is a documented **subset** of
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`), which
//! `wire_format` must not depend on. The DATA payloads of every domain other than
//! 96, 92 and 18 pass UNVALIDATED beyond header sanity, the stream validator and
//! the hour-boundary rules — production validates every domain's payload
//! (`recorder_host/src/stream_fold.rs:593`). That unvalidated remainder is explicitly BOUNDED:
//! every closed game hour must carry exactly one d96 DATA record of any status,
//! and the d92 and d18 lanes must each be all-or-nothing — exactly one record per
//! closed hour when scheduled — so a d96, d92 or d18 record relabelled to another
//! domain id, or dropped, refuses the run instead of silently shrinking it or
//! silently converting hours into dependency exclusions.
//!
//! # Properties this run reports rather than relies on
//!
//! - **`source_sequence == hour_index_from_raw(game_hour_raw) + 1`**, the
//!   collector's own computation (`attach_log.rs:33175`-`:33178`), which would
//!   make it a §6 derived field. The fold enforces nothing of the kind, so
//!   `d96.rs` carries it as a plain value and this harness MEASURES the identity.
//! - **`modifier_key` stability per tactic.** The grammar requires only a
//!   non-zero key (`compact_contract/src/tactic_weight.rs:314`), so a frame whose key differs from the
//!   previous hour's for the same tactic is a frame the fold folds. Whether the
//!   mapping is in fact constant on this capture is measured and printed — a
//!   property of one stream, never a licence to drop a field.
//! - **`modifier_metadata_flags` values.** The codec constrains one bit; the
//!   census observed every live key at flags 3. The distinct values seen are
//!   printed.
//! - **The accumulator's null rate** — how many rows carry an entry and how many
//!   are the `Null` state — because that is the coverage of the four-state
//!   decision this port turns on.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d96.bin   — the d96 DATA records verbatim (32-byte H4CR header +
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
    compact_status_unknown_flags_v1, expected_partial_tolerance, lane_local_status_mask,
    parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d92::{self, TacticRow};
use wire_format::d96::{self, Preferred, Symbols};

const D96: u16 = 96;
const D92: u16 = 92;
const D18: u16 = 18;
const BLOCK_HOURS: usize = 48;
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
const PROBE_DOMAIN_ID: u16 = 250;

/// The library half restates `COMPACT_STATUS_FAILURE_MASK` because it takes no
/// dependency on `compact_contract` (`d96.rs:FAILURE_STATUS_MASK`). This bin
/// does, so it proves the copy has not drifted — for every status word, not a
/// sample — plus the two facts the restatement rests on: d96 has no lane-local
/// status bit and no expected-partial tolerance. d70's pattern
/// (`bench_real_d70.rs:307`).
fn assert_status_predicates_agree() {
    assert_eq!(
        lane_local_status_mask(D96),
        0,
        "d96 gained a lane-local status bit; d96::FAILURE_STATUS_MASK must be revisited"
    );
    assert!(
        expected_partial_tolerance(D96).is_none(),
        "d96 gained an expected-partial tolerance; d96::FAILURE_STATUS_MASK must be revisited"
    );
    assert_eq!(
        d96::FAILURE_STATUS_MASK, COMPACT_STATUS_FAILURE_MASK,
        "the library's failure mask has drifted from compact_contract's"
    );
    // The two predicates must partition the u32 the same way. 2^32 words is too
    // many, so sweep every single bit, which is what they are defined over.
    for bit in 0..32u32 {
        let word = 1u32 << bit;
        assert_eq!(
            compact_status_failure_flags_after_expected_partial_tolerance(D96, word) != 0,
            d96::FAILURE_STATUS_MASK & word != 0,
            "status bit {bit}: the library's failure predicate has drifted"
        );
    }
}

// --- FRAME_END seal (`recorder_host/src/stream_fold.rs:46`-`:89`). -----------------------------

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
/// Holds exactly what the two gates read: the roster's KEY set, and whether a
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
    /// and therefore excludes that hour's d92 record, and with it d96's.
    model_failures: usize,
    first_model_failure: Option<String>,
    /// Failure-status D18 records whose BODY the payload authority would reject.
    ///
    /// The fold never looks at the body of such a record (`:14158` returns before
    /// the row walk), so its classification is "failure record" and this harness
    /// takes it — but `decode_compact_country_identity_data_payload` walks the
    /// rows whatever the status (`domain_data_authority.rs:1042`), so production
    /// would not certify the stream. The recipe's d49 rule: keep the fold's
    /// classification, report the certification consequence as a named deviation.
    /// 0 on the measured capture.
    failure_records_with_unwalkable_body: usize,
    first_failure_body_deviation: Option<String>,
}

/// The D18 payload authority's status-independent checks that the FOLD does not
/// make — applied fail-closed, as a stream rejection.
///
/// `validate_domain_data_payload_at` (`recorder_host/src/stream_fold.rs:593`) runs
/// `decode_compact_country_identity_data_payload`
/// (`domain_data_authority.rs:1013`) on EVERY d18 DOMAIN_DATA record whatever its
/// status, and a parse error clears the stream's decodable verdict. Its checks are
/// the same walk [`D18Mirror::fold`] performs — row truncation, cosmetic markers,
/// zero/empty/duplicate tags, the whole CIM1 extension, trailing bytes — with
/// **one exception**: `changed_rows > row_count` at offset `+12`
/// (`domain_data_authority.rs:1027`-`:1032`), a word `fold_country_identity`
/// never reads at all.
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
///   would measure a stream production does not certify — d65's rule
///   ("the same defect class is enforced at different layers"). It refuses the
///   stream.
///
/// Round-1 review finding (thread 019fa530-c013-74a2-b717-31f8e89e8639): without
/// this, a one-hour stream with `row_count = 1, changed_rows = 2` and otherwise
/// valid records was measured.
///
/// **This function applies ONLY the checks the fold does not make**, which is the
/// whole of its licence to refuse a stream. Round 2 (thread
/// 019fa53d-daf5-7810-a470-aeeb710e8166) caught it exceeding that: a payload
/// shorter than 20 bytes was refused here, but the FOLD refuses it too
/// (`world_fold.rs:13919`-`:13921`) and the recorder tolerates that refusal, so
/// the hour is excluded and every other hour still measures. Refusing the stream
/// contradicted this harness's own documented treatment. A short payload
/// therefore returns `Ok` from here and falls through to the mirror, which fails
/// it into `model_failures` — reported, with production's certification verdict
/// noted, exactly like any other unparseable d18 payload.
fn check_d18_payload_authority(payload: &[u8]) -> Result<(), String> {
    // Not a check: the authority's 20-byte minimum (`domain_data_authority.rs:1018`)
    // is also the FOLD's (`world_fold.rs:13919`), so it is the mirror's business.
    // The length is read here only to know whether the word below exists at all.
    if payload.len() < 20 {
        return Ok(());
    }
    let row_count = u32_at(payload, 8) as usize;
    let changed_rows = u32_at(payload, 12) as usize;
    // `:1027`-`:1032`. The one rule the fold never applies, because it never reads
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
    /// generic `Err` arm (`world_fold.rs:5016`) leaves the real accumulator. The
    /// caller reports the reason rather than refusing the stream.
    fn fold(&mut self, payload: &[u8]) -> Result<(), String> {
        // `world_fold.rs:14155` — the fold's own `need(payload, 20, ...)`.
        if payload.len() < 20 {
            return Err("country_identity header truncated".into());
        }
        let status_flags = u32_at(payload, 16);
        // `:14158`-`:14164`. The status check comes FIRST, immediately after the
        // header: the fold notes the failure and returns `Ok` **without reading
        // the body at all**, leaving the roster and the KNOWN flag unchanged.
        // Round-3 review finding (thread 019fa548-5e33-7061-9236-5c7151f92098):
        // walking the rows first misclassified a failure-status record with an
        // unwalkable body as an unparseable one, contradicting the fold's own
        // non-folding-path classification.
        //
        // The payload AUTHORITY does walk that body whatever the status
        // (`domain_data_authority.rs:1042`), so a body it would reject is a
        // certification deviation — counted and reported here, per the recipe's
        // d49 rule, without changing the classification the fold gives the hour.
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
                    let cosmetic = read_string(payload, &mut cursor, "country_identity cosmetic")?;
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
        // The optional CIM1 extension, in full.
        if cursor < payload.len() {
            self.walk_cim1(payload, &mut cursor, &fresh)?;
        }
        // The fold's exact-length check.
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
            let claim_count = read_u32(payload, cursor, "country_identity CIM1 state row")? as usize;
            let core_count = read_u32(payload, cursor, "country_identity CIM1 state row")? as usize;
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
                return Err(format!("country_identity CIM1 major row {index} is invalid"));
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

// --- The D92 mirror, through this crate's own d92 port. ----------------------

/// Why an hour has no clean D92 snapshot. Reported apart because each one is a
/// different finding about the capture.
#[derive(Clone, Copy, Debug, Default)]
struct D92Causes {
    /// D92 records `handle_compact_failure` refused (`world_fold.rs:10481`).
    failure_records: usize,
    /// D92 records with no proven D18 record that hour (`:10489`).
    no_roster_records: usize,
    /// D92 records whose tag set does not COVER the D18 roster (`:10519`).
    roster_uncovered_records: usize,
}

/// `country_preferred_tactics_this_hour` and its source sequence
/// (`world_fold.rs:1842`-`:1844`), as the current hour's state.
///
/// The payload SPAN is retained rather than the row map: pass 2 re-decodes it
/// through the same `d92::split_payload`, so the two passes cannot disagree and
/// the harness never holds 439 rows × 896 hours of strings.
#[derive(Clone, Debug)]
struct PreferredHour {
    payload_span: Range<usize>,
    rows: usize,
    source_sequence: u64,
    /// D92 tags D18 did not publish — kept in state by the post-fix gate.
    extra_tags: usize,
    /// …of which these are SELECTED, so d96 carries a weight row for them. This
    /// is the tag-286 case the fix exists to keep verifiable.
    selected_extra_tags: usize,
}

#[derive(Default)]
struct D92Mirror {
    /// The current hour's clean snapshot, cleared at every hour boundary
    /// (`world_fold.rs:3099`-`:3101`).
    current: Option<PreferredHour>,
    causes: D92Causes,
    records: usize,
    first_roster_mismatch: Option<String>,
}

impl D92Mirror {
    /// `world_fold.rs:3099`-`:3101` — unlike D18's roster, the D92 snapshot does
    /// NOT persist across hours.
    fn open_hour(&mut self) {
        self.current = None;
    }

    /// Fold one already-decoded D92 record, in the fold's own order
    /// (`world_fold.rs:10477`-`:10540`): `handle_compact_failure`, then the
    /// this-hour D18 flag, then the CONTAINMENT gate.
    fn fold(
        &mut self,
        header: &d92::PayloadHeader,
        rows: &[TacticRow],
        payload_span: Range<usize>,
        d18: &D18Mirror,
        game_hour: i32,
    ) {
        self.records += 1;
        if compact_status_failure_flags_after_expected_partial_tolerance(D92, header.status_flags)
            != 0
        {
            self.causes.failure_records += 1;
            self.current = None;
            return;
        }
        if !d18.known_this_hour {
            self.causes.no_roster_records += 1;
            self.current = None;
            return;
        }
        let observed: BTreeSet<u32> = rows.iter().map(|row| row.tag_db_id).collect();
        if d18.roster.difference(&observed).next().is_some() {
            self.causes.roster_uncovered_records += 1;
            if self.first_roster_mismatch.is_none() {
                let only_d18: Vec<u32> = d18.roster.difference(&observed).copied().take(8).collect();
                self.first_roster_mismatch = Some(format!(
                    "hour {game_hour}: d92 carries {} tags, D18 roster has {}; {} only in D18 \
                     (first {only_d18:?})",
                    observed.len(),
                    d18.roster.len(),
                    d18.roster.difference(&observed).count(),
                ));
            }
            self.current = None;
            return;
        }
        let extra_tags = observed.difference(&d18.roster).count();
        let selected_extra_tags = rows
            .iter()
            .filter(|row| row.is_selected() && !d18.roster.contains(&row.tag_db_id))
            .count();
        self.current = Some(PreferredHour {
            payload_span,
            rows: rows.len(),
            source_sequence: header.source_sequence,
            extra_tags,
            selected_extra_tags,
        });
    }
}

/// Rebuild D92's row map from a retained payload span, through the same public
/// boundary pass 1 used. Pass 2's only view of the dependency.
fn preferred_map(stream: &[u8], span: Range<usize>) -> Result<(BTreeMap<u32, TacticRow>, u64), String> {
    let (header, rows) = d92::split_payload(&stream[span])?;
    let map: BTreeMap<u32, TacticRow> = rows.into_iter().map(|row| (row.tag_db_id, row)).collect();
    Ok((map, header.source_sequence))
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

/// One measured d96 hour: its game hour, its byte span in the source stream (the
/// shipping arm is those bytes verbatim, header included), the payload span pass
/// 2 re-folds from, and the D92 snapshot the gates were satisfied against.
struct D96Hour {
    game_hour: i32,
    ship_span: Range<usize>,
    payload_span: Range<usize>,
    preferred: PreferredHour,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded hour or a non-contiguous hour step — which is what forces the
    /// encoder to open a fresh baseline (`opens_baseline`).
    follows_previous: bool,
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences: the
/// 48-hour block cadence (every block opens with its own baseline, §1.5's closing
/// paragraph), and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the
/// state part forces the next state observation to be a `BASELINE`." Every hour
/// this domain did not successfully sample is such a gap — each excluded-hour
/// reason, and any non-contiguous hour step — and d96's cadence is hourly, so a
/// measured hour that does not follow the previous measured one by exactly one is
/// preceded by one. Neither a delta nor a retained reader state can be justified
/// across it.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
}

/// One wire-arm hour: the §1.5/block baseline decision, the encode, and the
/// per-hour reader replay over both collections. Factored out of `main` so the
/// tests execute the PRODUCTION encode path over a scanned stream.
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
    // BASELINE at each block start — and at every §1.5 gap, which forces a fresh
    // baseline and restarts the delta chain (`opens_baseline`).
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
    // reconstructed prior, exactly as a conforming reader would, and require the
    // result to be stored-equal to this hour's state. This is what makes the size
    // claim a claim about decodable bytes. An hour that emitted nothing is still
    // checked: the reader's retained state must already equal the fold's, which is
    // precisely what "nothing changed" asserts.
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
    /// d96 records whose payload status marks the domain-hour failed.
    failure_records: usize,
    /// Records refused because no proven D18 record landed that hour.
    no_roster_records: usize,
    /// Records refused because the hour has no clean D92 snapshot.
    no_preferred_records: usize,
    /// …of which the hour carried no D92 record at all ahead of the d96 one.
    no_preferred_record_at_all: usize,
    /// Records refused by the PTW1/D92 join.
    join_mismatch_records: usize,
    /// Measured hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Measured records carrying no rows at all.
    rowless_records: usize,
    /// Rows the fold applied, across every measured record.
    rows_folded: usize,
    /// …of which these carry an accumulator entry (the rest map to `Null`).
    rows_with_accumulator_entry: usize,
    /// …and of these, the ones whose entry value is a real measured zero.
    rows_with_zero_accumulator_entry: usize,
    /// Measured records whose `source_sequence` is NOT `hour_index + 1`.
    source_sequence_violations: usize,
    /// Measured hours in which D92 carried a SELECTED tag D18 filtered out.
    hours_with_selected_extra_tags: usize,
    /// Their total across measured hours.
    selected_extra_rows: usize,
    /// Weight rows naming a tag D92 does not carry. The join iterates D92's map
    /// and so does not refuse these, and neither does the fold — the port mirrors
    /// that permissiveness (`d96::check_join`), and this counts what the capture
    /// actually contains rather than assuming the producer cannot emit one.
    rows_for_tags_absent_from_d92: usize,
}

struct StreamScan {
    hours: Vec<D96Hour>,
    counts: ScanCounts,
    /// The `tactic_id` vocabulary of the whole stream, frozen before any record
    /// is mapped (§5). Collected from d96 AND d92, which share it.
    symbols: Symbols,
    /// Every `(tactic_database_id, modifier_key)` pair observed, for the
    /// stability report. Reported, never relied on.
    id_to_modifier_key: BTreeMap<i32, BTreeSet<u32>>,
    /// Every `(tactic_database_id, tactic_id)` pair observed in d96.
    id_to_token: BTreeMap<i32, BTreeSet<String>>,
    metadata_flags_seen: BTreeSet<u32>,
    country_factors_seen: BTreeSet<i64>,
    /// d92 DATA records seen, of any status — the denominator the causes below
    /// are counted against. Carried rather than re-derived from the measured
    /// hours, which would be wrong whenever an hour is excluded for a d96-only
    /// reason (its d92 record folded cleanly and is not in any cause bucket).
    d92_records: usize,
    d92_causes: D92Causes,
    d92_first_roster_mismatch: Option<String>,
    d18_failure_records: usize,
    d18_proven_records: usize,
    d18_model_failures: usize,
    d18_first_model_failure: Option<String>,
    d18_failure_records_with_unwalkable_body: usize,
    d18_first_failure_body_deviation: Option<String>,
    first_join_mismatch: Option<String>,
}

/// Validate the whole stream against the canonical contract and fold every usable
/// d96 DATA record. Every rejection path is an `Err`, never a silently shorter
/// measurement and never a panic.
fn scan_stream(stream: &[u8], identity_mode: StreamIdentityMode) -> Result<StreamScan, String> {
    // Identity is a caller-stated protocol, never inferred from bytes:
    // `recorder_host/src/stream_fold.rs:220`, `stream_validator.rs:328`.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D96Hour> = Vec::new();
    let mut counts = ScanCounts::default();
    // COMPLETENESS accounting: every d96, d92 and d18 DATA record of ANY status,
    // against every closed game hour — see the checks after the EOF rules. The
    // dependency lanes are tracked PER HOUR too: aggregate counts alone would
    // accept a compensating duplicate (two records in one hour, none in another)
    // whose totals still match.
    let mut d96_records = 0usize;
    let mut d18_records = 0usize;
    let mut closed_hours = 0usize;
    let mut d18_covered_hours = 0usize;
    let mut d92_covered_hours = 0usize;
    let mut hour_saw_d18 = false;
    let mut hour_saw_d92 = false;
    let mut vocabulary: BTreeSet<String> = BTreeSet::new();
    let mut id_to_modifier_key: BTreeMap<i32, BTreeSet<u32>> = BTreeMap::new();
    let mut id_to_token: BTreeMap<i32, BTreeSet<String>> = BTreeMap::new();
    let mut metadata_flags_seen: BTreeSet<u32> = BTreeSet::new();
    let mut country_factors_seen: BTreeSet<i64> = BTreeSet::new();
    let mut d18 = D18Mirror::default();
    let mut d92m = D92Mirror::default();
    let mut first_join_mismatch: Option<String> = None;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d96 DATA record of ANY status — the uniqueness rule.
    let mut last_d96_hour: Option<i32> = None;
    let mut last_d92_hour: Option<i32> = None;
    // Hour of the last d96 record that entered the measurement — the contiguity
    // report, which a skipped excluded hour legitimately breaks.
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
            // `eof_check`: a stream ending mid-record is not a short stream, it is
            // a corrupt one.
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
                    // A new game hour: the D18 KNOWN flag resets here
                    // (`world_fold.rs:3002`) and the D92 snapshot is dropped
                    // outright (`:3099`-`:3101`).
                    d18.open_hour();
                    d92m.open_hour();
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
                if hour_saw_d92 {
                    d92_covered_hours += 1;
                }
                hour_saw_d18 = false;
                hour_saw_d92 = false;
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        // D18, then D92, then d96 — the recorder's own record order, and the
        // reason the same-hour gates can be satisfied at all. Each is folded as it
        // is met, in stream order.
        if header.domain_id == D18 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d18_records += 1;
            hour_saw_d18 = true;
            // The authority's status-independent rule the fold does not make. A
            // stream rejection, not a tolerated mirror failure — see
            // [`check_d18_payload_authority`] for why the two halves differ.
            check_d18_payload_authority(payload)
                .map_err(|e| format!("d18 payload at byte {pos}: {e}"))?;
            if let Err(reason) = d18.fold(payload) {
                // MIRROR THE RECORDER, do not out-strict it. D18 has no special
                // `Err` arm in `fold_domain_data`, so it lands on the generic one
                // — `Err(err) => self.note(...)` (`world_fold.rs:5016`) — which
                // records a note and continues. The roster map is left unchanged
                // and `country_identity_known_this_hour` stays clear, so the
                // hour's D92 record takes its "no proven D18 record" path and the
                // d96 record behind it is an EXCLUDED HOUR. Never silent: the
                // reason is retained and printed.
                d18.model_failures += 1;
                if d18.first_model_failure.is_none() {
                    d18.first_model_failure =
                        Some(format!("hour {} at byte {pos}: {reason}", header.game_hour_raw));
                }
            }
        }

        if header.domain_id == D92 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            hour_saw_d92 = true;
            // The whole PTC1 decode, first and unconditionally: production's
            // authority decodes every d92 record whatever its status
            // (`domain_data_authority.rs:13195`-`:13212`). A payload that does not
            // decode is a REJECTED STREAM, not an excluded hour.
            let (ptc_header, ptc_rows) = d92::split_payload(payload)
                .map_err(|e| format!("d92 payload at byte {pos}: {e}"))?;
            if let Some(previous) = last_d92_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d92 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d92_hour = Some(header.game_hour_raw);
            let unknown_status = compact_status_unknown_flags_v1(ptc_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d92 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }
            // §5: the vocabulary is the union of both domains' tokens, because the
            // two share one symbol table and the join requires their tokens equal.
            for row in &ptc_rows {
                if let Some(token) = row.tactic_id.as_deref() {
                    if !vocabulary.contains(token) {
                        vocabulary.insert(token.to_string());
                    }
                }
            }
            d92m.fold(
                &ptc_header,
                &ptc_rows,
                payload_start..record_end,
                &d18,
                header.game_hour_raw,
            );
        }

        if header.domain_id == D96 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d96_records += 1;
            // The whole PTW1 decode, first and unconditionally — the authority's
            // order, not the fold's (see the module docs).
            let (ptw_header, rows) = d96::split_payload(payload)
                .map_err(|e| format!("d96 payload at byte {pos}: {e}"))?;

            // One d96 DATA record per game hour, checked over EVERY d96 record and
            // BEFORE any exclusion: gating this on the record being usable would
            // let a success/failure pair through the rule entirely.
            if let Some(previous) = last_d96_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d96 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d96_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. The failure predicate is a classifier
            // over the *assigned* v1 status bits
            // (`status_flags_base.rs:40`-`:44`); an unassigned bit has no defined
            // classification and is not evidence of a future informational status,
            // so a record carrying one refuses the stream rather than falling
            // through to "success".
            let unknown_status = compact_status_unknown_flags_v1(ptw_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d96 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }
            // The library's restated mask must agree with production's predicate on
            // every record the capture actually carries, not only in the sweep.
            let production_failure = compact_status_failure_flags_after_expected_partial_tolerance(
                D96,
                ptw_header.status_flags,
            ) != 0;
            if production_failure != ptw_header.is_failure() {
                return Err(format!(
                    "d96 DATA at byte {pos}: status {:#x} classifies as failure=\
                     {production_failure} under compact_contract but the library boundary \
                     decided otherwise",
                    ptw_header.status_flags
                ));
            }

            // The vocabulary, before any classification: a token is a token
            // whatever the hour's fate, and §5 forbids growing the table later.
            for row in &rows {
                if !vocabulary.contains(&row.tactic_id) {
                    vocabulary.insert(row.tactic_id.clone());
                }
            }

            // The exclusions, in the FOLD's order (`world_fold.rs:10542`-`:10570`):
            // D18 gate, D92 gate, then failure status, then the join.
            if !d18.known_this_hour {
                counts.no_roster_records += 1;
                pos = record_end;
                continue;
            }
            let Some(preferred_hour) = d92m.current.clone() else {
                counts.no_preferred_records += 1;
                if !hour_saw_d92 {
                    counts.no_preferred_record_at_all += 1;
                }
                pos = record_end;
                continue;
            };
            if production_failure {
                counts.failure_records += 1;
                pos = record_end;
                continue;
            }
            // The PTW1/D92 join, through the port's own boundary.
            let (map, sequence) = preferred_map(stream, preferred_hour.payload_span.clone())?;
            if let Err(reason) = d96::check_join(
                &ptw_header,
                &rows,
                Preferred {
                    rows: &map,
                    source_sequence: sequence,
                },
            ) {
                counts.join_mismatch_records += 1;
                if first_join_mismatch.is_none() {
                    first_join_mismatch =
                        Some(format!("hour {} at byte {pos}: {reason}", header.game_hour_raw));
                }
                pos = record_end;
                continue;
            }

            // Measured. Everything below is reporting.
            let follows_previous =
                last_measured_hour.is_some_and(|previous| header.game_hour_raw == previous + 1);
            if last_measured_hour.is_some() && !follows_previous {
                counts.non_contiguous += 1;
            }
            last_measured_hour = Some(header.game_hour_raw);

            if rows.is_empty() {
                counts.rowless_records += 1;
            }
            counts.rows_for_tags_absent_from_d92 += rows
                .iter()
                .filter(|row| !map.contains_key(&row.tag_db_id))
                .count();
            country_factors_seen.insert(ptw_header.country_factor_raw);
            for row in &rows {
                counts.rows_folded += 1;
                if row.accumulator_entry_present {
                    counts.rows_with_accumulator_entry += 1;
                    if row.accumulator_raw == 0 {
                        counts.rows_with_zero_accumulator_entry += 1;
                    }
                }
                id_to_modifier_key
                    .entry(row.tactic_database_id)
                    .or_default()
                    .insert(row.modifier_key);
                id_to_token
                    .entry(row.tactic_database_id)
                    .or_default()
                    .insert(row.tactic_id.clone());
                metadata_flags_seen.insert(row.modifier_metadata_flags);
            }
            if preferred_hour.selected_extra_tags != 0 {
                counts.hours_with_selected_extra_tags += 1;
                counts.selected_extra_rows += preferred_hour.selected_extra_tags;
            }

            // Reported, not relied on: `source_sequence == hour_index + 1`.
            let expected_sequence =
                compact_contract::calendar::hour_index_from_raw(header.game_hour_raw)
                    .and_then(|hour| u64::try_from(hour).ok())
                    .map(|hour| hour.saturating_add(1));
            if expected_sequence != Some(ptw_header.source_sequence) {
                counts.source_sequence_violations += 1;
            }

            hours.push(D96Hour {
                game_hour: header.game_hour_raw,
                ship_span: pos..record_end,
                payload_span: payload_start..record_end,
                preferred: preferred_hour,
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
        // cannot distinguish a stream without d96 from one whose every hour a
        // dependency refused, and those are different findings.
        return Err(format!(
            "stream carries no usable d96 DATA records: {d96_records} d96 DATA record(s) seen, \
             of which {} failure-status, {} with no proven D18 record that hour, {} with no \
             clean D92 snapshot ({} of those hours carried no D92 record at all), {} refused by \
             the PTW1/D92 join. D92 itself contributed {} records, of which {} failure-status, \
             {} with no D18 roster and {} not covering it. D18 contributed {} proven, {} \
             failure-status and {} unparseable records. First D92 roster mismatch: {}. First \
             join mismatch: {}",
            counts.failure_records,
            counts.no_roster_records,
            counts.no_preferred_records,
            counts.no_preferred_record_at_all,
            counts.join_mismatch_records,
            d92m.records,
            d92m.causes.failure_records,
            d92m.causes.no_roster_records,
            d92m.causes.roster_uncovered_records,
            d18.proven_records,
            d18.failure_records,
            d18.model_failures,
            d92m.first_roster_mismatch.as_deref().unwrap_or("none"),
            first_join_mismatch.as_deref().unwrap_or("none"),
        ));
    }

    // COMPLETENESS: every closed game hour must carry exactly one d96 DATA record
    // of any status, and each dependency lane must be all-or-nothing — exactly one
    // record per closed hour when scheduled, none at all when not. This bounds
    // what this harness canNOT validate: production runs
    // `validate_domain_data_payload_at` on EVERY domain's DATA payload
    // (`recorder_host/src/stream_fold.rs:593`), while this harness owns only d96's, d92's and d18's
    // grammars, so a record of any of the three whose header was relabelled — or
    // lost outright — would otherwise be silently ignored: a lost d96 record
    // shrinks the run, and a lost d92 or d18 record silently turns its hour's d96
    // record into a dependency exclusion. A per-capture completeness assertion,
    // NOT a grammar rule (`bench_real_d65.rs` precedent).
    if d96_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d96_records} d96 DATA \
             record(s); every closed hour of a d96-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of unmodelled domains to see where \
             the difference went"
        ));
    }
    // Both an aggregate and a PER-HOUR coverage check for each dependency, because
    // aggregate counts alone would accept a compensating duplicate — two records
    // in one hour and none in another — whose totals still match.
    for (name, records, covered) in [
        ("d18", d18_records, d18_covered_hours),
        ("d92", d92m.records, d92_covered_hours),
    ] {
        if records != 0 && (records != closed_hours || covered != closed_hours) {
            return Err(format!(
                "stream closes {closed_hours} game hour(s) but carries {records} {name} DATA \
                 record(s) covering {covered} hour(s); a capture scheduling {name} carries \
                 exactly one per closed hour, and a partially-missing {name} lane would \
                 silently convert its hours' d96 records into dependency exclusions"
            ));
        }
    }

    // §5: the table is complete before any record is mapped, and never grows
    // afterwards. Pass 2 walks exactly these spans with exactly this
    // `split_payload`, so it cannot need a symbol this pass did not see.
    let symbols = Symbols::from_vocabulary(vocabulary);

    Ok(StreamScan {
        hours,
        counts,
        symbols,
        id_to_modifier_key,
        id_to_token,
        metadata_flags_seen,
        country_factors_seen,
        d92_records: d92m.records,
        d92_causes: d92m.causes,
        d92_first_roster_mismatch: d92m.first_roster_mismatch.clone(),
        d18_failure_records: d18.failure_records,
        d18_proven_records: d18.proven_records,
        d18_model_failures: d18.model_failures,
        d18_first_model_failure: d18.first_model_failure.clone(),
        d18_failure_records_with_unwalkable_body: d18.failure_records_with_unwalkable_body,
        d18_first_failure_body_deviation: d18.first_failure_body_deviation.clone(),
        first_join_mismatch,
    })
}

fn main() {
    const USAGE: &str = "usage: bench_real_d96 <collector_stream.bin> <out dir> [legacy|identity-v1]";
    assert_status_predicates_agree();
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d96::schema();
    let countries = &schema.collections[d96::COUNTRIES_INDEX];
    let frame = &schema.collections[d96::FRAME_INDEX];

    // ---- Pass 1: validate the WHOLE stream and classify its d96 records. ----
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

    let mut acc = d96::State::new();
    let mut reader: Option<Vec<KeyedState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut rows_total = 0usize;
    let mut preferred_rows_total = 0usize;
    let mut extra_tags_total = 0usize;

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

        // The dependency, rebuilt through the same public boundary pass 1 used,
        // and checked against what pass 1 recorded.
        let (map, sequence) = preferred_map(&stream, hour.preferred.payload_span.clone())
            .unwrap_or_else(|e| panic!("game hour {}: D92 re-decode failed: {e}", hour.game_hour));
        assert_eq!(
            (map.len(), sequence),
            (hour.preferred.rows, hour.preferred.source_sequence),
            "game hour {}: pass 2 re-decoded a different D92 snapshot",
            hour.game_hour
        );
        preferred_rows_total += map.len();
        extra_tags_total += hour.preferred.extra_tags;

        // The hour's state, through the SAME public boundary the unit tests
        // exercise, under exactly the dependency pass 1 validated.
        let prior: Vec<KeyedState> = acc.as_slice().to_vec();
        let outcome = d96::fold_payload(
            &schema,
            &mut acc,
            &stream[hour.payload_span.clone()],
            true,
            Some(Preferred {
                rows: &map,
                source_sequence: sequence,
            }),
            0,
            &scan.symbols,
        )
        .unwrap_or_else(|e| panic!("game hour {}: pass 2 fold failed: {e}", hour.game_hour));
        assert!(
            matches!(outcome, d96::FoldOutcome::Folded { .. }),
            "game hour {}: pass 1 measured an hour the fold did not fold",
            hour.game_hour
        );
        let state = acc.as_slice();
        rows_total += acc.countries().len();

        // Cadence-independent churn, before the block decision.
        if hours_seen > 0 {
            let d = diff(
                countries,
                &prior[d96::COUNTRIES_INDEX],
                &state[d96::COUNTRIES_INDEX],
            );
            ci_changed += d.changed.len();
            ci_tombstones += d.tombstones.len();
            ci_transitions += 1;
            let f = diff(frame, &prior[d96::FRAME_INDEX], &state[d96::FRAME_INDEX]);
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
            dp_changed += outcome.changed_by_collection[d96::COUNTRIES_INDEX];
            dp_tombstones += outcome.tombstones_by_collection[d96::COUNTRIES_INDEX];
            delta_bytes += outcome.bytes;
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d96.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d96.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    let ambiguous_keys: Vec<i32> = scan
        .id_to_modifier_key
        .iter()
        .filter(|(_, keys)| keys.len() > 1)
        .map(|(id, _)| *id)
        .collect();
    let ambiguous_tokens: Vec<i32> = scan
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
        "excluded hours       {} failure-status, {} no-D18-this-hour, {} no-clean-D92 ({} with \
         no D92 record at all), {} PTW1/D92 join mismatch{}",
        scan.counts.failure_records,
        scan.counts.no_roster_records,
        scan.counts.no_preferred_records,
        scan.counts.no_preferred_record_at_all,
        scan.counts.join_mismatch_records,
        scan.first_join_mismatch
            .as_deref()
            .map(|first| format!(" (first: {first})"))
            .unwrap_or_default()
    );
    println!(
        "d92 modelled         {} records: {} failure-status, {} no-D18-roster, {} \
         D18-not-covered{}",
        scan.d92_records,
        scan.d92_causes.failure_records,
        scan.d92_causes.no_roster_records,
        scan.d92_causes.roster_uncovered_records,
        scan.d92_first_roster_mismatch
            .as_deref()
            .map(|first| format!(" (first: {first})"))
            .unwrap_or_default()
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
        "d92 roster           {:.1} rows/hour joined, {:.1} of them tags D18 filtered out; \
         {} measured hour(s) carried a SELECTED extra tag ({} row(s) total)",
        preferred_rows_total as f64 / hours_seen.max(1) as f64,
        extra_tags_total as f64 / hours_seen.max(1) as f64,
        scan.counts.hours_with_selected_extra_tags,
        scan.counts.selected_extra_rows
    );
    println!(
        "shipping rows        {} folded, {} rowless records, {} naming a tag D92 does not \
         carry (mirrored, not refused)",
        scan.counts.rows_folded,
        scan.counts.rowless_records,
        scan.counts.rows_for_tags_absent_from_d92
    );
    println!(
        "accumulator entries  {} of {} rows carry one ({:.2}%), {} of those are a measured \
         zero; the other {} rows map to Null",
        scan.counts.rows_with_accumulator_entry,
        scan.counts.rows_folded,
        100.0 * scan.counts.rows_with_accumulator_entry as f64
            / scan.counts.rows_folded.max(1) as f64,
        scan.counts.rows_with_zero_accumulator_entry,
        scan.counts.rows_folded - scan.counts.rows_with_accumulator_entry
    );
    println!(
        "tactic symbols       {} (§5 vocabulary over d96 AND d92, frozen before mapping)",
        scan.symbols.len()
    );
    println!(
        "id/token stability   {} distinct database ids, {} ever named more than one token {:?}",
        scan.id_to_token.len(),
        ambiguous_tokens.len(),
        ambiguous_tokens
    );
    println!(
        "modifier_key         {} distinct database ids, {} ever named more than one key {:?}",
        scan.id_to_modifier_key.len(),
        ambiguous_keys.len(),
        ambiguous_keys
    );
    println!(
        "metadata flags       distinct values {:?}",
        scan.metadata_flags_seen
    );
    println!(
        "country_factor_raw   distinct values {:?}",
        scan.country_factors_seen
    );
    println!(
        "source_sequence      {} of {hours_seen} measured hours violate `hour_index + 1`",
        scan.counts.source_sequence_violations
    );
    println!(
        "weights avg          {:.1} rows/hour",
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
    println!("frame singleton      changed on {frame_changed} of {ci_transitions} transitions");
    println!(
        "ship_d96.bin         {} bytes ({framing_bytes} of them H4CR record framing, {:.1}%, \
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
    /// row a `tag_db_id`, a length-prefixed tag and an absent-cosmetic marker. No
    /// CIM1 trailer, which the fold treats as legal (`world_fold.rs:13986`).
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

    fn frame_end(out: &mut Vec<u8>, sequence: u64, hour: i32) {
        push_record(
            out,
            sequence,
            hour,
            1000,
            COMPACT_RECORD_TYPE_FRAME_END,
            &vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        );
    }

    /// A PTC1 payload (`bench_real_d92.rs`'s builder, since the grammar is d92's).
    fn ptc_payload(rows: &[(u32, i32, &str)], source_sequence: u64, status_flags: u32) -> Vec<u8> {
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
        p.extend_from_slice(&status_flags.to_le_bytes());
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

    /// A PTW1 payload. `rows` is `(tag, database id, token, Option<accumulator>)`
    /// — `None` meaning the accumulator holds no entry for the key.
    fn ptw_payload(
        rows: &[(u32, i32, &str, Option<i64>)],
        observed_country_count: u32,
        source_sequence: u64,
        country_factor_raw: i64,
        status_flags: u32,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        for (tag, id, token, accumulator) in rows {
            let (flags, raw) = match accumulator {
                Some(value) => (d96::FLAG_ACCUMULATOR_ENTRY_PRESENT, *value),
                None => (0u16, 0i64),
            };
            body.extend_from_slice(&tag.to_le_bytes());
            body.extend_from_slice(&id.to_le_bytes());
            body.extend_from_slice(&7u32.to_le_bytes()); // modifier_key
            body.extend_from_slice(&3u32.to_le_bytes()); // metadata flags
            body.extend_from_slice(&0i32.to_le_bytes()); // metadata gate
            body.extend_from_slice(&flags.to_le_bytes());
            body.extend_from_slice(&(token.len() as u16).to_le_bytes());
            body.extend_from_slice(&raw.to_le_bytes());
            body.extend_from_slice(&(country_factor_raw + raw).to_le_bytes());
            body.extend_from_slice(token.as_bytes());
        }
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // read_time_us
        p.extend_from_slice(&observed_country_count.to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&d96::MAGIC);
        p.extend_from_slice(&d96::VERSION.to_le_bytes());
        p.extend_from_slice(&(d96::HEADER_LEN as u16).to_le_bytes());
        p.extend_from_slice(&source_sequence.to_le_bytes());
        p.extend_from_slice(&(d96::FIXED_SCALE as u32).to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved
        p.extend_from_slice(&country_factor_raw.to_le_bytes());
        for (_, receipt, _) in d96::RECEIPTS {
            p.extend_from_slice(receipt);
        }
        assert_eq!(p.len(), d96::HEADER_LEN);
        p.extend_from_slice(&body);
        p
    }

    /// A whole legacy-identity stream of one-hour frames, in the recorder's own
    /// record order: D18, then D92, then d96, then a zero-count FRAME_END seal.
    fn stream_of(frames: &[(i32, Vec<u8>, Vec<u8>, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut sequence = 1u64;
        for (hour, d18, d92_payload, d96_payload) in frames {
            for (domain, payload) in [(D18, d18), (D92, d92_payload), (D96, d96_payload)] {
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
            frame_end(&mut out, sequence, *hour);
            sequence += 1;
        }
        out
    }

    /// The `deepwar_c` shape in miniature: D18 publishes two tags, D92 carries a
    /// third it filtered out, and that third tag is SELECTED — the tag-286 case.
    fn deepwar_frame(hour: i32, sequence: u64) -> (i32, Vec<u8>, Vec<u8>, Vec<u8>) {
        (
            hour,
            d18_payload(&[(1, "AAA"), (2, "BBB")], 0),
            ptc_payload(
                &[(1, 0, ""), (2, 5, "shock"), (286, 6, "blitzkrieg")],
                sequence,
                0,
            ),
            ptw_payload(
                &[
                    (2, 5, "shock", Some(40_000)),
                    (286, 6, "blitzkrieg", None),
                ],
                3,
                sequence,
                -25_000,
                0,
            ),
        )
    }

    #[test]
    fn the_library_failure_mask_agrees_with_production() {
        assert_status_predicates_agree();
    }

    /// The claim pass 1 rests on: a vocabulary built from the stream resolves
    /// EVERY row of every measured payload through `record_from_row`. Both passes
    /// walk the same `split_payload`, so pass 2 can never need a symbol pass 1 did
    /// not see — which is what makes §5's "no in-stream interning" a property of
    /// this harness rather than a hope.
    #[test]
    fn the_symbol_table_covers_every_token_in_the_stream() {
        let s = stream_of(&[deepwar_frame(100, 1), deepwar_frame(101, 2)]);
        let scan = scan_stream(&s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.symbols.len(), 2, "two distinct tokens, deduplicated");
        let schema = d96::schema();
        let countries = &schema.collections[d96::COUNTRIES_INDEX];
        for hour in &scan.hours {
            let (header, rows) =
                d96::split_payload(&s[hour.payload_span.clone()]).expect("decodes");
            for row in &rows {
                d96::record_from_row(
                    countries,
                    row,
                    0,
                    &scan.symbols,
                    header.country_factor_raw,
                )
                .expect("every token resolves against the pass-1 vocabulary");
            }
        }
    }

    #[test]
    fn a_selected_tag_d18_filtered_out_is_measured_and_joined() {
        // The post-fix path, end to end: the extra tag is kept in D92's state, the
        // join validates its weight row, and the row is encoded like any other.
        let s = stream_of(&[deepwar_frame(100, 1)]);
        let scan = scan_stream(&s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.counts.rows_folded, 2);
        assert_eq!(scan.counts.rows_with_accumulator_entry, 1);
        assert_eq!(scan.counts.hours_with_selected_extra_tags, 1);
        assert_eq!(scan.counts.selected_extra_rows, 1);
        assert_eq!(scan.hours[0].preferred.rows, 3, "D92's WHOLE row set");
        assert_eq!(scan.hours[0].preferred.extra_tags, 1);

        // And the fold accepts it under exactly that dependency.
        let schema = d96::schema();
        let mut acc = d96::State::new();
        let (map, sequence) =
            preferred_map(&s, scan.hours[0].preferred.payload_span.clone()).expect("re-decodes");
        d96::fold_payload(
            &schema,
            &mut acc,
            &s[scan.hours[0].payload_span.clone()],
            true,
            Some(Preferred {
                rows: &map,
                source_sequence: sequence,
            }),
            0,
            &scan.symbols,
        )
        .expect("folds");
        assert_eq!(acc.len(), 2);
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

        // A failure-status record leaves both untouched (`:13922`-`:13929`).
        d18.fold(&d18_payload(&[(9, "ZZZ")], COMPACT_STATUS_FAILURE_MASK))
            .expect("returns Ok without folding");
        assert!(!d18.known_this_hour);
        assert_eq!(d18.roster, [1u32, 2].into_iter().collect::<BTreeSet<_>>());
        assert_eq!(d18.failure_records, 1);

        // An unparseable payload is all-or-nothing and never disturbs the mirror.
        let good = d18_payload(&[(3, "CCC")], 0);
        for cut in 20..good.len() {
            assert!(d18.fold(&good[..cut]).is_err());
            assert_eq!(d18.roster.len(), 2, "no partial roster at cut {cut}");
        }
    }

    #[test]
    fn the_d92_mirror_reproduces_the_containment_gate_and_does_not_persist() {
        // Containment, not equality: a D92 superset folds and keeps its extras, a
        // D92 record missing a D18 tag does not — and neither survives the hour
        // boundary (`world_fold.rs:3099`-`:3101`).
        let mut d18 = D18Mirror::default();
        d18.open_hour();
        d18.fold(&d18_payload(&[(1, "AAA"), (2, "BBB")], 0))
            .expect("folds");
        let mut d92m = D92Mirror::default();
        d92m.open_hour();

        let superset = ptc_payload(&[(1, 0, ""), (2, 5, "shock"), (286, 6, "blitzkrieg")], 1, 0);
        let (h, rows) = d92::split_payload(&superset).expect("decodes");
        d92m.fold(&h, &rows, 0..superset.len(), &d18, 100);
        let current = d92m.current.clone().expect("a superset folds");
        assert_eq!(current.rows, 3);
        assert_eq!(current.extra_tags, 1);
        assert_eq!(current.selected_extra_tags, 1);

        // The snapshot does NOT persist into the next hour.
        d92m.open_hour();
        assert!(d92m.current.is_none());

        // A D18 tag with no D92 row.
        let gapped = ptc_payload(&[(1, 0, ""), (286, 6, "blitzkrieg")], 2, 0);
        let (h, rows) = d92::split_payload(&gapped).expect("decodes");
        d92m.fold(&h, &rows, 0..gapped.len(), &d18, 101);
        assert!(d92m.current.is_none());
        assert_eq!(d92m.causes.roster_uncovered_records, 1);
        assert!(d92m
            .first_roster_mismatch
            .as_deref()
            .expect("recorded")
            .contains("1 only in D18"));

        // A failure-status D92 record, and one landing in an hour with no D18.
        d92m.open_hour();
        let failed = ptc_payload(&[], 0, COMPACT_STATUS_FAILURE_MASK);
        let (h, rows) = d92::split_payload(&failed).expect("decodes");
        d92m.fold(&h, &rows, 0..failed.len(), &d18, 102);
        assert!(d92m.current.is_none());
        assert_eq!(d92m.causes.failure_records, 1);

        d18.open_hour();
        d92m.open_hour();
        let (h, rows) = d92::split_payload(&superset).expect("decodes");
        d92m.fold(&h, &rows, 0..superset.len(), &d18, 103);
        assert!(d92m.current.is_none());
        assert_eq!(d92m.causes.no_roster_records, 1);
    }

    #[test]
    fn an_hour_without_a_clean_d92_snapshot_is_excluded_and_counted() {
        // Three ways the dependency fails, each a separate excluded hour with its
        // own reported cause. The gate `scan_stream` applies is its own copy of
        // the fold's order, so this drives it end to end.
        let d18 = d18_payload(&[(1, "AAA"), (2, "BBB")], 0);
        let good92 = |seq| ptc_payload(&[(1, 0, ""), (2, 5, "shock")], seq, 0);
        let good96 = |seq| {
            ptw_payload(&[(2, 5, "shock", Some(40_000))], 2, seq, -25_000, 0)
        };
        // Hour 101: D92 carries a failure status.
        // Hour 102: D92 does not cover the D18 roster.
        let s = stream_of(&[
            (100, d18.clone(), good92(1), good96(1)),
            (
                101,
                d18.clone(),
                ptc_payload(&[], 0, COMPACT_STATUS_FAILURE_MASK),
                good96(2),
            ),
            (
                102,
                d18.clone(),
                ptc_payload(&[(1, 0, "")], 3, 0),
                good96(3),
            ),
            (103, d18.clone(), good92(4), good96(4)),
        ]);
        let scan = scan_stream(&s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2, "only the two clean-D92 hours measure");
        assert_eq!(scan.counts.no_preferred_records, 2);
        assert_eq!(scan.counts.no_preferred_record_at_all, 0);
        assert_eq!(scan.d92_causes.failure_records, 1);
        assert_eq!(scan.d92_causes.roster_uncovered_records, 1);
        assert_eq!(scan.counts.non_contiguous, 1);
    }

    #[test]
    fn a_d96_record_ahead_of_its_d92_record_is_an_excluded_hour() {
        // The dependency is same-hour AND ordered: the recorder folds in stream
        // order, so a d96 record that arrives first sees no snapshot at all. That
        // is an excluded hour, reported apart from a D92 record that failed.
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let p92 = ptc_payload(&[(1, 5, "shock")], 1, 0);
        let p96 = ptw_payload(&[(1, 5, "shock", None)], 1, 1, 0, 0);
        let mut s = Vec::new();
        push_record(&mut s, 1, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(&mut s, 2, 100, D96, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p96);
        push_record(&mut s, 3, 100, D92, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p92);
        frame_end(&mut s, 4, 100);
        let reason = match scan_stream(&s, StreamIdentityMode::Legacy) {
            Err(reason) => reason,
            Ok(scan) => panic!("no hour is measurable, but {} were", scan.hours.len()),
        };
        assert!(
            reason.contains("no clean D92 snapshot") && reason.contains("carried no D92 record"),
            "{reason}"
        );
    }

    #[test]
    fn a_join_mismatch_is_an_excluded_hour_and_never_silent() {
        // A PTW1 whose roster witness disagrees with D92's row count — the exact
        // shape the 2026-07-27 gate fix turns on, with D18's filtered count
        // substituted for D92's.
        let d18 = d18_payload(&[(1, "AAA"), (2, "BBB")], 0);
        let p92 = ptc_payload(&[(1, 0, ""), (2, 5, "shock"), (286, 6, "blitzkrieg")], 1, 0);
        let wrong_witness = ptw_payload(
            &[(2, 5, "shock", None), (286, 6, "blitzkrieg", None)],
            2, // D18's count, not D92's 3
            1,
            0,
            0,
        );
        let reason = match scan_stream(
            &stream_of(&[(100, d18, p92, wrong_witness)]),
            StreamIdentityMode::Legacy,
        ) {
            Err(reason) => reason,
            Ok(scan) => panic!("no hour is measurable, but {} were", scan.hours.len()),
        };
        assert!(
            reason.contains("refused by the PTW1/D92 join")
                && reason.contains("roster count or source-sequence mismatch"),
            "{reason}"
        );
    }

    #[test]
    fn a_failure_status_d96_record_is_an_excluded_hour() {
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let p92 = |seq| ptc_payload(&[(1, 5, "shock")], seq, 0);
        let good = |seq| ptw_payload(&[(1, 5, "shock", None)], 1, seq, 0, 0);
        let failed = ptw_payload(&[], 0, 0, 0, COMPACT_STATUS_FAILURE_MASK);
        let s = stream_of(&[
            (100, d18.clone(), p92(1), good(1)),
            (101, d18.clone(), p92(2), failed),
            (102, d18, p92(3), good(3)),
        ]);
        let scan = scan_stream(&s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.counts.failure_records, 1);
        assert_eq!(scan.counts.non_contiguous, 1);
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces the
        // next state observation to be a `BASELINE`." An excluded d96 hour is such
        // a gap — the next measured hour must clear `follows_previous`, and
        // `opens_baseline` must open a baseline on it wherever it lands in a block.
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let p92 = |seq| ptc_payload(&[(1, 5, "shock")], seq, 0);
        let good = |seq, acc| ptw_payload(&[(1, 5, "shock", acc)], 1, seq, 0, 0);
        let s = stream_of(&[
            (100, d18.clone(), p92(1), good(1, Some(1))),
            (
                101,
                d18.clone(),
                p92(2),
                ptw_payload(&[], 0, 0, 0, COMPACT_STATUS_FAILURE_MASK),
            ),
            (102, d18, p92(3), good(3, Some(2))),
        ]);
        let scan = scan_stream(&s, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.counts.failure_records, 1);
        assert_eq!(scan.counts.non_contiguous, 1);
        assert!(!scan.hours[0].follows_previous, "the first hour follows none");
        assert!(
            !scan.hours[1].follows_previous,
            "hour 102 does not follow hour 100"
        );

        // ...and that is what makes the encoder open a baseline for it, even
        // though it is only the second measured hour of the block — through the
        // PRODUCTION encode path, not just the decision function.
        let schema = d96::schema();
        let mut acc = d96::State::new();
        let mut priors = Vec::new();
        let mut states = Vec::new();
        for hour in &scan.hours {
            priors.push(acc.as_slice().to_vec());
            let (map, sequence) =
                preferred_map(&s, hour.preferred.payload_span.clone()).expect("re-decodes");
            d96::fold_payload(
                &schema,
                &mut acc,
                &s[hour.payload_span.clone()],
                true,
                Some(Preferred {
                    rows: &map,
                    source_sequence: sequence,
                }),
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
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let scan = scan_stream(
            &stream_of(&[
                (100, d18.clone(), p92(1), good(1, None)),
                (101, d18, p92(2), good(2, Some(5))),
            ]),
            StreamIdentityMode::Legacy,
        )
        .expect("well-formed");
        assert_eq!(scan.counts.non_contiguous, 0);
        assert!(scan.hours[1].follows_previous);
        assert!(!opens_baseline(1, scan.hours[1].follows_previous));
    }

    #[test]
    fn the_cross_domain_completeness_bounds_refuse_a_gappy_stream() {
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let p92 = |seq| ptc_payload(&[(1, 5, "shock")], seq, 0);
        let p96 = |seq| ptw_payload(&[(1, 5, "shock", None)], 1, seq, 0, 0);

        // A closed hour with no d96 record at all.
        let mut s = Vec::new();
        push_record(&mut s, 1, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(&mut s, 2, 100, D92, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p92(1));
        push_record(&mut s, 3, 100, D96, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p96(1));
        frame_end(&mut s, 4, 100);
        push_record(&mut s, 5, 101, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(&mut s, 6, 101, D92, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p92(2));
        push_record(&mut s, 7, 101, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]);
        frame_end(&mut s, 8, 101);
        let err = match scan_stream(&s, StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d96 record must refuse the stream"),
        };
        assert!(err.contains("every closed hour"), "{err}");

        // A partially-missing d92 lane: hour 101 carries d18 and d96 but no d92.
        let mut s = Vec::new();
        push_record(&mut s, 1, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(&mut s, 2, 100, D92, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p92(1));
        push_record(&mut s, 3, 100, D96, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p96(1));
        frame_end(&mut s, 4, 100);
        push_record(&mut s, 5, 101, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(&mut s, 6, 101, D96, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p96(2));
        frame_end(&mut s, 7, 101);
        let err = match scan_stream(&s, StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a partially-missing d92 lane must refuse the stream"),
        };
        assert!(err.contains("d92"), "the refusal names the d92 bound: {err}");

        // A COMPENSATING duplicate — two d18 records in hour 100, none in hour 101
        // — leaves the aggregate count equal to the closed hours and is caught only
        // by the per-hour coverage check.
        let mut s = Vec::new();
        push_record(&mut s, 1, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(&mut s, 2, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(&mut s, 3, 100, D92, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p92(1));
        push_record(&mut s, 4, 100, D96, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p96(1));
        frame_end(&mut s, 5, 100);
        push_record(&mut s, 6, 101, D92, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p92(2));
        push_record(&mut s, 7, 101, D96, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p96(2));
        frame_end(&mut s, 8, 101);
        let err = match scan_stream(&s, StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a compensating d18 duplicate must refuse the stream"),
        };
        assert!(err.contains("covering"), "the refusal shows the coverage: {err}");
    }

    #[test]
    fn a_malformed_payload_of_either_domain_refuses_the_stream() {
        // Production's authority decodes every d96 and d92 record whatever its
        // status and whatever the gates say, so a payload that does not decode
        // invalidates the run rather than skipping an hour.
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let p92 = ptc_payload(&[(1, 5, "shock")], 1, 0);
        let p96 = ptw_payload(&[(1, 5, "shock", None)], 1, 1, 0, 0);

        let refusal = |frames: &[(i32, Vec<u8>, Vec<u8>, Vec<u8>)], what: &str| -> String {
            match scan_stream(&stream_of(frames), StreamIdentityMode::Legacy) {
                Err(reason) => reason,
                Ok(scan) => panic!("{what}, but {} hour(s) measured", scan.hours.len()),
            }
        };

        let mut truncated96 = p96.clone();
        truncated96.truncate(d96::HEADER_LEN - 1);
        let err = refusal(
            &[(100, d18.clone(), p92.clone(), truncated96)],
            "a malformed PTW1 must refuse the stream",
        );
        assert!(err.contains("d96 payload at byte"), "{err}");

        let mut truncated92 = p92.clone();
        truncated92.truncate(d92::HEADER_LEN - 1);
        let err = refusal(
            &[(100, d18.clone(), truncated92, p96.clone())],
            "a malformed PTC1 must refuse the stream",
        );
        assert!(err.contains("d92 payload at byte"), "{err}");

        // An unassigned status bit is refused BEFORE classification. The payload
        // is otherwise well-formed and CLEAN — a bit outside the assigned v1
        // vocabulary is not a failure bit, so `validate_snapshot` takes the clean
        // branch and the record reaches the harness's classifier, which is where
        // the rule has to fire.
        let unassigned = ptw_payload(&[], 1, 1, 0, 1 << 16);
        let err = refusal(
            &[(100, d18, p92, unassigned)],
            "an unassigned status bit must refuse the stream",
        );
        assert!(err.contains("unassigned v1 status bits"), "{err}");
    }

    #[test]
    fn a_d18_record_whose_changed_rows_exceeds_row_count_refuses_the_stream() {
        // Codex round-1 finding (thread 019fa530-c013-74a2-b717-31f8e89e8639).
        // `fold_country_identity` never reads the word, so the mirror folded such a
        // record normally and the hour was measured — while production's
        // status-independent d18 authority rejects it
        // (`domain_data_authority.rs:1027`-`:1032`) and the stream is not
        // certified. Fixed by property, not by the reported input: the whole
        // (row_count, changed_rows) space is swept below, and the authority's own
        // predicate is applied fail-closed ahead of the mirror.
        let p92 = |seq| ptc_payload(&[(1, 5, "shock")], seq, 0);
        let p96 = |seq| ptw_payload(&[(1, 5, "shock", None)], 1, seq, 0, 0);

        // The reviewer's exact input: one valid row, `changed_rows = 2`, clean
        // status, valid d92 and d96 records.
        let reviewer = d18_payload_with_changed_rows(&[(1, "AAA")], 0, 2);
        let err = match scan_stream(
            &stream_of(&[(100, reviewer, p92(1), p96(1))]),
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
        // fall through to the mirror rather than refusing the stream (round-2
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
            &stream_of(&[(100, good, p92(1), p96(1))]),
            StreamIdentityMode::Legacy,
        )
        .expect("changed_rows == row_count is legal");
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn an_unparseable_d18_payload_excludes_its_hour_and_keeps_the_rest() {
        // Round-2 review finding (thread 019fa53d-daf5-7810-a470-aeeb710e8166),
        // with the reviewer's exact input: hour 101 carries a 19-byte d18 payload.
        // The recorder's d18 fold returns `Err`, the generic error arm notes it and
        // continues, d18 stays unproven for that hour, so d92 takes its no-roster
        // path and d96 is an excluded hour — while hour 100 still measures. A
        // stream rejection here would refuse a stream the recorder accepts.
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let p92 = |seq| ptc_payload(&[(1, 5, "shock")], seq, 0);
        let p96 = |seq| ptw_payload(&[(1, 5, "shock", None)], 1, seq, 0, 0);
        let scan = scan_stream(
            &stream_of(&[
                (100, d18.clone(), p92(1), p96(1)),
                (101, vec![0u8; 19], p92(2), p96(2)),
                (102, d18, p92(3), p96(3)),
            ]),
            StreamIdentityMode::Legacy,
        )
        .expect("the recorder accepts this stream, so the harness must too");
        assert_eq!(scan.hours.len(), 2, "hours 100 and 102 measure");
        assert_eq!(scan.d18_model_failures, 1, "counted, never silent");
        assert!(scan.d18_first_model_failure.is_some(), "and reported");
        // The exclusion is attributed in the FOLD's order: d96's own D18 gate
        // fires first (`world_fold.rs:10543`), so the hour is a no-D18 exclusion
        // rather than a no-clean-D92 one, even though its d92 record was refused
        // for the same underlying reason.
        assert_eq!(scan.counts.no_roster_records, 1);
        assert_eq!(scan.counts.no_preferred_records, 0);
        assert_eq!(scan.d92_causes.no_roster_records, 1);
        assert_eq!(scan.counts.non_contiguous, 1, "so hour 102 follows a gap");
        assert!(!scan.hours[1].follows_previous);
    }

    #[test]
    fn a_failure_status_d18_record_is_classified_by_the_fold_not_by_its_body() {
        // Round-3 review finding (thread 019fa548-5e33-7061-9236-5c7151f92098),
        // with the reviewer's exact input: a 20-byte d18 payload claiming
        // `row_count = 1` with no row body at all, under a failure status.
        // `fold_country_identity` checks the status immediately after the header
        // and returns `Ok` WITHOUT reading the body (`world_fold.rs:14158`), so
        // the hour is a failure record — not an unparseable one. Walking the rows
        // first reported `0 failure-status / 1 unparseable` for it.
        let p92 = |seq| ptc_payload(&[(1, 5, "shock")], seq, 0);
        let p96 = |seq| ptw_payload(&[(1, 5, "shock", None)], 1, seq, 0, 0);
        let mut headless = vec![0u8; 20];
        headless[8..12].copy_from_slice(&1u32.to_le_bytes()); // row_count = 1
        headless[16..20].copy_from_slice(&1u32.to_le_bytes()); // STATE_UNAVAILABLE
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let scan = scan_stream(
            &stream_of(&[
                (100, d18.clone(), p92(1), p96(1)),
                (101, headless.clone(), p92(2), p96(2)),
                (102, d18, p92(3), p96(3)),
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
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.counts.no_roster_records, 1);

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

    #[test]
    fn a_weight_row_for_a_tag_d92_does_not_carry_is_measured_and_counted() {
        // The join iterates D92's map, so such a row is neither refused nor
        // silently dropped — the port mirrors the fold's permissiveness and the
        // harness COUNTS it rather than assuming the producer cannot emit one.
        let d18 = d18_payload(&[(1, "AAA")], 0);
        // D92: tag 1 selected, tags 2 and 3 with no tactic. d96: tag 1's row plus a
        // row for tag 999, which D92 never mentions. `rows.len() = 2 <= 3`.
        let p92 = ptc_payload(&[(1, 5, "shock"), (2, 0, ""), (3, 0, "")], 1, 0);
        let p96 = ptw_payload(
            &[(1, 5, "shock", None), (999, 6, "blitzkrieg", None)],
            3,
            1,
            0,
            0,
        );
        let scan = scan_stream(
            &stream_of(&[(100, d18, p92, p96)]),
            StreamIdentityMode::Legacy,
        )
        .expect("the fold accepts this, so the harness must too");
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.counts.rows_folded, 2);
        assert_eq!(
            scan.counts.rows_for_tags_absent_from_d92, 1,
            "counted, not assumed away"
        );
    }

    #[test]
    fn two_d96_records_in_one_hour_refuse_the_stream() {
        // Hour uniqueness is enforced over EVERY d96 record of any status, before
        // any exclusion, or a success+failure pair in one hour slips through.
        let d18 = d18_payload(&[(1, "AAA")], 0);
        let p92 = ptc_payload(&[(1, 5, "shock")], 1, 0);
        let p96 = ptw_payload(&[(1, 5, "shock", None)], 1, 1, 0, 0);
        let failed = ptw_payload(&[], 0, 0, 0, COMPACT_STATUS_FAILURE_MASK);
        let mut s = Vec::new();
        push_record(&mut s, 1, 100, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d18);
        push_record(&mut s, 2, 100, D92, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p92);
        push_record(&mut s, 3, 100, D96, COMPACT_RECORD_TYPE_DOMAIN_DATA, &failed);
        push_record(&mut s, 4, 100, D96, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p96);
        frame_end(&mut s, 5, 100);
        let err = match scan_stream(&s, StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("two d96 records in one hour must refuse the stream"),
        };
        assert!(err.contains("after hour"), "{err}");
    }
}
