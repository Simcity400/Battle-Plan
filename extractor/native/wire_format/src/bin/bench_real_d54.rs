//! Measure the wire format against the shipping compact stream on REAL d54
//! `army_battleplans_depth_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d54-scheduled capture
//! (`runs/batch3/deepwar_a`). The stream's own d54 DATA records are the shipping
//! format's cost for the domain: a 60-byte header, a fixed 88-byte row per
//! changed order, three 8-byte child tables and the `OGS1` orders-group census,
//! exactly what the recorder stores today. This tool replays those records into
//! per-hour states — the same fold `world_fold::fold_army_battleplans_depth`
//! performs — and re-encodes every hour with this crate's grammar: a BASELINE at
//! each 48-hour block start (and after every capture gap) and
//! field-granular stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** Header, row, child-table,
//! extension and frame decoding all go through `wire_format::d54`'s public
//! boundary — including `d54::State::fold_payload`, because one d54 record is
//! not one d54 state: the grammar is baseline-plus-delta over the order table and
//! replace-or-clear over the census, so an hour's state is a function of the
//! stream up to that hour. H4CR framing, record semantics, sequence continuity
//! and every status predicate go through `compact_contract`'s own parser,
//! validator and classifiers. A harness that reimplements either drifts from the
//! recorder without anything failing.
//!
//! **The whole stream is validated before any artifact is written.** A stream that
//! is truncated, spliced or corrupt would otherwise yield a smaller but entirely
//! plausible measurement — a silently wrong number, which is worse than a crash.
//! [`scan_stream`] therefore enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`
//!   (magic, version, header length, payload bound);
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`
//!   (sequence continuity from the stream's first record, record-type and
//!   record-flag semantics, FRAME_END shape);
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`), its DOMAIN_PERF hour binding
//!   (`:328`-`:365`) and the DATA arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal validation (`:46`-`:89`) and its distinct non-probe PERF
//!   domain-count cross-check (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. This crate cannot validate other domains' payloads, but it
//! replays d54's full status-independent payload authority.
//!
//! **d54's authority is a single one, and it runs first.**
//! `decode_compact_army_battleplans_depth_live_data_payload`
//! (`domain_data_authority.rs:8317`-`:8325`) runs `battleplans54::decode_depth_payload`
//! on every DOMAIN_DATA record regardless of status, and the fold's own first
//! statement is that same call (`world_fold.rs:21762`-`:21765`), before it reads
//! any status word. `d54::split_payload` **is** that function, so this harness
//! calls it on every d54 record before it classifies the record's status, and a
//! violation refuses the stream. There is no d49-style certifier/fold split here:
//! both authorities are the same predicate and both are fatal.
//!
//! **Three exclusion paths, all from the fold, in the fold's own order.**
//! `fold_army_battleplans_depth` decodes first, then returns without folding on a
//! hard failure status (`:21776`), then on a delta-before-baseline (`:21792`),
//! then — after its second row decode — on the expected-partial tolerance
//! (`:21813`). d54 IS one of the four domains in
//! `COMPACT_EXPECTED_PARTIAL_TOLERANCES` (`compact_contract/src/lib.rs:197`), so
//! unlike d65 that branch is live rather than dead, and
//! [`the_tolerance_entry_still_exists`] pins it so its removal fails the run
//! instead of silently changing what is measured. All three leave the accumulator
//! retained and the domain-hour unknown, so all three are *excluded hours*, never
//! stream rejections. The predicates are `compact_contract`'s own — never a
//! hand-copied mask literal, even though the fold itself writes one
//! (`:21774`), and [`assert_status_predicates_agree`] pins that the fold's mask
//! and the shared predicate classify identically over the whole v1 status space.
//! Hour uniqueness is checked over EVERY d54 DATA record and BEFORE any exclusion,
//! so a success + failure pair inside one hour cannot slip through.
//!
//! **d27 and d28 are dependencies of the COLLECTOR, not writers of the fold's
//! accumulator, and this harness proves the difference.** The survey calls them
//! hard dependencies and they are: `read_compact_army_battleplans_depth_live`
//! builds every d54 row out of d27's order read and authenticates each row's owner
//! against d28's command rows or the `OGS1` census
//! (`collector/src/attach_log.rs:46402`-`:46620`). But inside `world_fold.rs`
//! **exactly one function writes `army_battleplans_depth` and
//! `army_orders_group_manager`**, and it is d54's own fold — so the recipe's
//! "enumerate every other writer of your accumulator" question has the answer
//! *none*, and no d27/d28 accumulator mirror is needed or built.
//!
//! What the dependency chain does produce is d54's status word, so that is
//! measured per hour rather than assumed: this harness reads d27's and d28's own
//! status words through their ports' `peek_header` (**key columns only** — no d27
//! or d28 record, schema or fold is built here) and cross-tabulates them against
//! d54's. **On `deepwar_a` that cross-tab falsifies the survey**, which says d54
//! "clears only when d27/d28 stop reporting partials": d27 and d28 report a
//! tolerated partial on **0 of 514 hours**, while d54 is withheld on 241. The
//! collector has four ways to raise d54's PARTIAL and only the first two are the
//! dependencies' — an inherited d27/d28 tolerated partial (`:46454`-`:46482`), a
//! hard dependency failure (`:46484`-`:46508`), an `OGS1` census rejection or
//! order/army disagreement (`:46521`-`:46534`, `:46655`-`:46669`), and d54's OWN
//! extraction omissions, `skipped_rows != 0` at `:46759` — and it is the fourth
//! that fires here. The one-way implications are all that can be asserted, so
//! the residue is counted and printed instead.
//!
//! The DATA payloads of every domain other than 54 pass UNVALIDATED beyond header
//! sanity, the stream validator and the hour-boundary rules — production validates
//! every domain's payload (`recorder_host/src/stream_fold.rs:593`); this harness owns only d54's
//! grammar. That unvalidated remainder is explicitly BOUNDED: every closed game
//! hour must carry exactly one d54 DATA record of any status, and the d27 and d28
//! lanes must each be all-or-nothing — exactly one record per closed hour when
//! scheduled, none at all when not — so a d54 record relabelled to another domain
//! id, or dropped, refuses the run instead of silently shrinking it.
//!
//! **The transcription check.** `d54.rs` hand-transcribes a codec that lives in a
//! lane crate, which the library half of `wire_format` may not depend on (the
//! recipe's dependency ruling gives it `compact_contract` and nothing else), so
//! the d67/d70 dual-decode cross-check is not available: there is no second
//! decoder to run. Two checks stand in for it, and both are in this bin because
//! this bin is the only half that may reach the shipping contract at all:
//! [`assert_status_predicates_agree`] pins every restated status constant against
//! `compact_contract`'s, and every one of the capture's d54 payloads must
//! reproduce its own byte length exactly from the transcribed table strides
//! (`length_is_reproduced_by_the_transcribed_strides`), so a wrong stride fails on
//! the first record rather than at review. That the strides are not machine-checked
//! against the lane crate itself is a recorded recipe gap, not a claim of proof.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d54.bin   — the d54 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface twice — cadence-independent and along the
//! delta path — because one number cannot answer both questions, and the
//! payload-level path-geometry evidence the ordinal decision in `d54.rs` rests on.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    expected_partial_tolerance_applies, parse_compact_record_header,
    COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
    COMPACT_STATUS_PARTIAL_DATA, COMPACT_STATUS_SOURCE_UNAVAILABLE,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, Key, KeyedDelta,
    KeyedState,
};
use wire_format::{d27, d28, d54};

const D27: u16 = 27;
const D28: u16 = 28;
const D54: u16 = 54;
const BLOCK_HOURS: usize = 48;

/// One usable d54 hour: its game hour, the folded state at hour close, and the
/// byte span of the record that produced it (the shipping arm is those bytes
/// verbatim, header included).
#[derive(Debug)]
struct D54Hour {
    game_hour: i32,
    states: Vec<KeyedState>,
    ship_span: Range<usize>,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded hour (failure status, delta-before-baseline, tolerated partial)
    /// or a non-contiguous hour step — which is what forces the encoder to open
    /// a fresh baseline (`opens_baseline`).
    follows_previous: bool,
}

/// The shape of the rows the capture actually carried, so the coverage caveats in
/// the deliverable are measured rather than assumed.
#[derive(Debug, Default)]
struct RowStats {
    records: usize,
    order_rows: usize,
    /// The two header words the batch-3 survey's "rows/h" column could mean, so
    /// the correction is measured rather than argued.
    min_row_count: u32,
    max_row_count: u32,
    min_observed_army_rows: u32,
    max_observed_army_rows: u32,
    min_observed_order_rows: u32,
    max_observed_order_rows: u32,
    deleted_rows: usize,
    live_rows: usize,
    /// Live rows carried by a record the fold FOLDS, versus one it withholds.
    live_rows_folded: usize,
    live_rows_withheld: usize,
    /// Four-state coverage of the domain's one optional field.
    target_declared_absent: usize,
    target_declared_unknown: usize,
    target_declared_value: usize,
    /// Witness-bit coverage.
    non_type5_geometry_blocked: usize,
    execution_blocked: usize,
    /// Child-table coverage.
    orders_with_targets: usize,
    orders_with_path: usize,
    orders_with_assigned: usize,
    target_child_rows: usize,
    path_child_rows: usize,
    assigned_child_rows: usize,
    /// Path shape: polylines that repeat a province, and polylines whose ids
    /// happen to ascend. Both are the ordinal decision's evidence.
    paths_with_a_repeated_province: usize,
    paths_ascending: usize,
    /// Order type histogram over live rows.
    order_types: BTreeMap<i32, usize>,
    /// `OGS1` coverage.
    records_with_extension: usize,
    legacy_records: usize,
    /// Legacy records that the fold nonetheless FOLDS — the only ones on which
    /// the clear-the-census rule is exercised for real.
    legacy_records_folded: usize,
    census_army_rows: usize,
    census_army_group_ids: usize,
    /// Census `execution_type` values above the labelled UI subset (`> 2`),
    /// which is why that field is `Unsigned` and not an `Enum`.
    census_execution_type_above_labelled: usize,
}

/// Payload-level path-geometry evidence: how a polyline evolves between two
/// observations of the SAME order key, over every record in the stream whether
/// the fold folds it or not.
///
/// It has to be payload level. The fold withholds 241 of the capture's 514
/// records and 610 of its 616 live order rows sit in those, so fold-level
/// evidence about path evolution does not exist on this capture — a fact this
/// harness reports rather than papers over.
#[derive(Debug, Default)]
struct PathEvidence {
    observations: usize,
    repeats: usize,
    unchanged: usize,
    changed: usize,
    /// Of the changes: pure reorders of an identical multiset. This is d11's
    /// `convoy_priority_reservations` stop condition; a non-zero count here would
    /// mean a keyed set stores two different polylines identically.
    pure_reorders: usize,
    shrank: usize,
    grew: usize,
    prefix_stable: usize,
    suffix_stable: usize,
    /// Churn under the chosen ordinal key versus under a (measured impossible)
    /// province-id key.
    ordinal_positions_differing: usize,
    ordinal_positions_total: usize,
    province_key_id_set_changes: usize,
    /// Distinct order keys, and how many were serialized more than once.
    distinct_keys: usize,
    keys_seen_more_than_once: usize,
    /// Repeat observations on which `slot_index` moved. The fold's own comment
    /// calls it "a mutable attribute on the value" (`world_fold.rs:23874`);
    /// this is the measurement that says whether keying on it would have churned.
    slot_index_changed: usize,
}

/// The per-hour dependency cross-tab. d27 and d28 do not write this domain's
/// accumulator; what they produce is its status word.
#[derive(Debug, Default)]
struct DependencyCoverage {
    d27_records: usize,
    d28_records: usize,
    /// d54 hours by their own classification, split by what the dependencies said.
    withheld_with_a_dependency_partial: usize,
    withheld_without_a_dependency_partial: usize,
    failure_with_a_dependency_failure: usize,
    failure_without_a_dependency_failure: usize,
    /// d54 delta frames whose d27 record was also a delta — the collector
    /// inherits exactly that bit (`attach_log.rs:46442`).
    delta_inherited_from_d27: usize,
    delta_not_inherited_from_d27: usize,
    /// Hours in which d27 or d28 was itself an expected partial.
    d27_tolerated_partial: usize,
    d28_tolerated_partial: usize,
    d27_hard_failure: usize,
    d28_hard_failure: usize,
    /// Hours carrying a d54 record but missing a d27 or d28 one.
    hours_missing_a_dependency_record: usize,
}

#[derive(Debug)]
struct StreamScan {
    hours: Vec<D54Hour>,
    /// d54 records excluded by each of the fold's three non-folding paths.
    failure_records: usize,
    delta_before_baseline_records: usize,
    tolerated_partial_records: usize,
    /// Measured d54 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    stats: RowStats,
    path_evidence: PathEvidence,
    dependencies: DependencyCoverage,
    /// Hours whose folded state carried at least one order row.
    hours_with_an_order: usize,
    max_orders: usize,
}

// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
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

/// The transcribed table strides must reproduce this payload's own length
/// exactly. `d54.rs` hand-transcribes a lane-crate codec that `wire_format`'s
/// library half may not depend on, so there is no second decoder to cross-check
/// against; this is the empirical stand-in, run on every real record.
fn length_is_reproduced_by_the_transcribed_strides(
    split: &d54::Split<'_>,
    payload_len: usize,
) -> Result<(), String> {
    let legacy_len = d54::HEADER_LEN
        + split.header.row_count as usize * d54::ROW_LEN
        + split.header.target_count as usize * d54::TARGET_ROW_LEN
        + split.header.path_count as usize * d54::PATH_GEOMETRY_ROW_LEN
        + split.header.assigned_division_count as usize * d54::ASSIGNED_DIVISION_ROW_LEN;
    let expected = match &split.extension {
        None => legacy_len,
        Some(extension) => {
            legacy_len
                + d54::EXTENSION_HEADER_LEN
                + extension.armies.len() * d54::EXTENSION_ROW_LEN
                + extension.army_group_ids.len() * 4
        }
    };
    if expected != payload_len {
        return Err(format!(
            "the transcribed d54 table strides reproduce {expected} bytes but the record \
             carries {payload_len}"
        ));
    }
    Ok(())
}

/// Accumulate the row-shape counters and the path evidence from one
/// already-validated d54 payload. Reads only through `d54`'s public boundary, so
/// it cannot disagree with the mapper about what a row says.
fn tally(
    split: &d54::Split<'_>,
    folded: bool,
    stats: &mut RowStats,
    evidence: &mut PathEvidence,
    last_path: &mut BTreeMap<Key, (Vec<u32>, u32)>,
) -> Result<(), String> {
    if stats.records == 0 {
        stats.min_row_count = u32::MAX;
        stats.min_observed_army_rows = u32::MAX;
        stats.min_observed_order_rows = u32::MAX;
    }
    stats.records += 1;
    stats.min_row_count = stats.min_row_count.min(split.header.row_count);
    stats.max_row_count = stats.max_row_count.max(split.header.row_count);
    stats.min_observed_army_rows = stats
        .min_observed_army_rows
        .min(split.header.observed_army_rows);
    stats.max_observed_army_rows = stats
        .max_observed_army_rows
        .max(split.header.observed_army_rows);
    stats.min_observed_order_rows = stats
        .min_observed_order_rows
        .min(split.header.observed_order_rows);
    stats.max_observed_order_rows = stats
        .max_observed_order_rows
        .max(split.header.observed_order_rows);
    match &split.extension {
        None => {
            stats.legacy_records += 1;
            if folded {
                stats.legacy_records_folded += 1;
            }
        }
        Some(extension) => {
            stats.records_with_extension += 1;
            stats.census_army_rows += extension.armies.len();
            stats.census_army_group_ids += extension.army_group_ids.len();
            for army in &extension.armies {
                if army.execution_type > 2 {
                    stats.census_execution_type_above_labelled += 1;
                }
            }
        }
    }

    for index in 0..split.header.row_count as usize {
        let bytes = split.order_row(index);
        let row = d54::parse_order_row(bytes)?;
        stats.order_rows += 1;
        if row.deleted() {
            stats.deleted_rows += 1;
            continue;
        }
        stats.live_rows += 1;
        if folded {
            stats.live_rows_folded += 1;
        } else {
            stats.live_rows_withheld += 1;
        }
        *stats.order_types.entry(row.order_type).or_default() += 1;
        match row.target_declared_count {
            Some(_) => stats.target_declared_value += 1,
            None if row.target_status == d54::STATUS_UNSUPPORTED => {
                stats.target_declared_absent += 1
            }
            None => stats.target_declared_unknown += 1,
        }
        if row.flags & d54::FLAG_NON_TYPE5_GEOMETRY_BLOCKED != 0 {
            stats.non_type5_geometry_blocked += 1;
        }
        if row.flags & d54::FLAG_EXECUTION_BLOCKED != 0 {
            stats.execution_blocked += 1;
        }
        if row.has_targets() {
            stats.orders_with_targets += 1;
            stats.target_child_rows += row.child_count as usize;
        }
        if row.assigned_count != 0 {
            stats.orders_with_assigned += 1;
            stats.assigned_child_rows += row.assigned_count as usize;
        }
        if !row.has_path() {
            continue;
        }
        stats.orders_with_path += 1;
        stats.path_child_rows += row.child_count as usize;
        let mut path = Vec::with_capacity(row.child_count as usize);
        for ordinal in 0..row.child_count {
            path.push(d54::parse_child_row(
                split.path_row((row.child_first + ordinal) as usize),
                d54::MAX_PROVINCE_ID,
                "path",
            )?);
        }
        let unique: BTreeSet<u32> = path.iter().copied().collect();
        if unique.len() != path.len() {
            stats.paths_with_a_repeated_province += 1;
        }
        if path.windows(2).all(|w| w[0] <= w[1]) {
            stats.paths_ascending += 1;
        }

        let order_key = d54::order_key(
            row.owner_kind,
            row.owner_id,
            row.engine_order_instance_id,
            row.generation,
        );
        evidence.observations += 1;
        if let Some((previous, previous_slot)) = last_path.get(&order_key) {
            evidence.repeats += 1;
            if *previous_slot != row.slot_index {
                evidence.slot_index_changed += 1;
            }
            if *previous == path {
                evidence.unchanged += 1;
            } else {
                evidence.changed += 1;
                if path.len() < previous.len() {
                    evidence.shrank += 1;
                } else if path.len() > previous.len() {
                    evidence.grew += 1;
                }
                let span = path.len().max(previous.len());
                evidence.ordinal_positions_total += span;
                for position in 0..span {
                    if path.get(position) != previous.get(position) {
                        evidence.ordinal_positions_differing += 1;
                    }
                }
                let now: BTreeSet<u32> = path.iter().copied().collect();
                let before: BTreeSet<u32> = previous.iter().copied().collect();
                evidence.province_key_id_set_changes += now.symmetric_difference(&before).count();
                let mut sorted_now = path.clone();
                let mut sorted_before = previous.clone();
                sorted_now.sort_unstable();
                sorted_before.sort_unstable();
                if sorted_now == sorted_before {
                    evidence.pure_reorders += 1;
                }
                let overlap = path.len().min(previous.len());
                if overlap > 0 && path[..overlap] == previous[..overlap] {
                    evidence.prefix_stable += 1;
                }
                if overlap > 0
                    && path[path.len() - overlap..] == previous[previous.len() - overlap..]
                {
                    evidence.suffix_stable += 1;
                }
            }
        }
        last_path.insert(order_key, (path, row.slot_index));
    }
    Ok(())
}

/// Classify one status word, rejecting an unassigned bit BEFORE classifying.
///
/// The failure predicate and its partial-tolerance sibling classify the
/// *assigned* v1 status vocabulary and are not validity checks
/// (`compact_contract/src/status_flags_base.rs:40`-`:44`); an unassigned bit has
/// no defined classification and is not evidence of a future informational
/// status, so a record carrying one refuses the stream rather than falling
/// through to "success".
fn classify(domain: u16, status_flags: u32, at: &str) -> Result<(bool, bool), String> {
    let unknown = compact_status_unknown_flags_v1(status_flags);
    if unknown != 0 {
        return Err(format!(
            "{at}: status word {status_flags:#x} carries unassigned v1 bits {unknown:#x}; \
             an unassigned bit has no defined classification"
        ));
    }
    let failure =
        compact_status_failure_flags_after_expected_partial_tolerance(domain, status_flags) != 0;
    let tolerated = expected_partial_tolerance_applies(domain, status_flags).is_some();
    Ok((failure, tolerated))
}

/// Validate the whole stream against the canonical contract and fold its usable
/// d54 DATA records into per-hour states. Every rejection path is an `Err`, never
/// a silently shorter measurement and never a panic — see the module docs for what
/// is enforced and what the recorder's own validator additionally checks.
fn scan_stream(
    schema: &wire_format::Schema,
    stream: &[u8],
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut scan = StreamScan {
        hours: Vec::new(),
        failure_records: 0,
        delta_before_baseline_records: 0,
        tolerated_partial_records: 0,
        non_contiguous: 0,
        stats: RowStats::default(),
        path_evidence: PathEvidence::default(),
        dependencies: DependencyCoverage::default(),
        hours_with_an_order: 0,
        max_orders: 0,
    };

    let mut d54_records = 0usize;
    let mut closed_hours = 0usize;
    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last DATA record of ANY status, per domain — the uniqueness rule.
    let mut last_d54_hour: Option<i32> = None;
    let mut last_d27_hour: Option<i32> = None;
    let mut last_d28_hour: Option<i32> = None;
    // Hour of the last d54 record that entered the measurement — the contiguity
    // report, which a skipped excluded hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // The dependency status words seen in the hour currently open. d27 and d28
    // are emitted before d54 in the frame, but nothing in the format promises
    // that, so the cross-tab is settled at hour close.
    let mut hour_d27_status: Option<u32> = None;
    let mut hour_d28_status: Option<u32> = None;
    let mut hour_d54_status: Option<u32> = None;

    let mut state = d54::State::new();
    let mut pending_hour: Option<D54Hour> = None;
    // Last observed polyline per order key, across EVERY record — see
    // `PathEvidence`.
    let mut last_path: BTreeMap<Key, (Vec<u32>, u32)> = BTreeMap::new();
    let mut distinct_order_keys: BTreeMap<Key, usize> = BTreeMap::new();

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
                                "DATA hour conflict at byte {pos}: pending PERF hour {pending}, \
                                 then DATA hour {}",
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
                            "non-monotonic FRAME_END at byte {pos}: closed hour {last_closed}, \
                             then {}",
                            header.game_hour_raw
                        ));
                    }
                }

                // The dependency cross-tab, settled once the hour is complete.
                if let Some(d54_status) = hour_d54_status.take() {
                    tally_dependencies(
                        &mut scan.dependencies,
                        d54_status,
                        hour_d27_status,
                        hour_d28_status,
                        header.game_hour_raw,
                    )?;
                }
                hour_d27_status = None;
                hour_d28_status = None;

                // The hour's state is taken at hour close, after every fold of the
                // hour.
                if let Some(mut hour) = pending_hour.take() {
                    hour.states = state.collections();
                    if !hour.states[0].is_empty() {
                        scan.hours_with_an_order += 1;
                    }
                    scan.max_orders = scan.max_orders.max(hour.states[0].len());
                    scan.hours.push(hour);
                }

                open_hour = None;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
                last_closed_hour = Some(header.game_hour_raw);
                closed_hours += 1;
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        // --- The dependency lanes: KEY COLUMNS ONLY -------------------------
        //
        // Neither writes this domain's accumulator, so neither is modelled: only
        // the status word each produced is read, through its own port's header
        // boundary rather than a byte offset re-derived here.
        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            && (header.domain_id == D27 || header.domain_id == D28)
        {
            let (last, slot) = if header.domain_id == D27 {
                (&mut last_d27_hour, &mut hour_d27_status)
            } else {
                (&mut last_d28_hour, &mut hour_d28_status)
            };
            if let Some(previous) = *last {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d{} DATA at byte {pos} carries hour {} after hour {previous}",
                        header.domain_id, header.game_hour_raw
                    ));
                }
            }
            *last = Some(header.game_hour_raw);
            let status = if header.domain_id == D27 {
                scan.dependencies.d27_records += 1;
                d27::peek_header(payload)
                    .map_err(|e| format!("d27 payload at byte {pos}: {e}"))?
                    .status_flags
            } else {
                scan.dependencies.d28_records += 1;
                d28::peek_header(payload)
                    .map_err(|e| format!("d28 payload at byte {pos}: {e}"))?
                    .status_flags
            };
            // Validity before classification on the dependency words too.
            classify(header.domain_id, status, &format!("d{} at byte {pos}", header.domain_id))?;
            *slot = Some(status);
        }

        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA && header.domain_id == D54 {
            d54_records += 1;
            // One d54 DATA record per game hour, checked over EVERY d54 record and
            // BEFORE any exclusion: gating this on the record being usable would
            // let a success + failure pair through the rule entirely, and two
            // records in one hour would fold twice into a state measured once.
            if let Some(previous) = last_d54_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d54 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d54_hour = Some(header.game_hour_raw);

            let peeked = d54::peek_header(payload)
                .map_err(|e| format!("d54 payload at byte {pos}: {e}"))?;
            hour_d54_status = Some(peeked.status_flags);
            let (failure, tolerated) =
                classify(D54, peeked.status_flags, &format!("d54 at byte {pos}"))?;

            // Production's status-independent payload authority, replayed in full
            // and on every record — a malformed payload is a stream rejection even
            // under a failure status, because `decode_depth_payload` runs before
            // any status is read (`world_fold.rs:21762`).
            let split = d54::split_payload(payload)
                .map_err(|e| format!("d54 payload at byte {pos}: {e}"))?;
            length_is_reproduced_by_the_transcribed_strides(&split, payload.len())
                .map_err(|e| format!("d54 payload at byte {pos}: {e}"))?;
            for index in 0..split.header.row_count as usize {
                let row = d54::parse_order_row(split.order_row(index))
                    .map_err(|e| format!("d54 payload at byte {pos}: {e}"))?;
                *distinct_order_keys
                    .entry(d54::order_key(
                        row.owner_kind,
                        row.owner_id,
                        row.engine_order_instance_id,
                        row.generation,
                    ))
                    .or_default() += 1;
            }

            let outcome = state
                .fold_payload(schema, payload, failure, tolerated)
                .map_err(|e| format!("d54 payload at byte {pos}: {e}"))?;
            tally(
                &split,
                outcome == d54::FoldOutcome::Applied,
                &mut scan.stats,
                &mut scan.path_evidence,
                &mut last_path,
            )
            .map_err(|e| format!("d54 payload at byte {pos}: {e}"))?;

            match outcome {
                d54::FoldOutcome::FailureStatus => {
                    scan.failure_records += 1;
                    pos = record_end;
                    continue;
                }
                d54::FoldOutcome::DeltaBeforeBaseline => {
                    scan.delta_before_baseline_records += 1;
                    pos = record_end;
                    continue;
                }
                d54::FoldOutcome::ToleratedPartialWithheld => {
                    scan.tolerated_partial_records += 1;
                    pos = record_end;
                    continue;
                }
                d54::FoldOutcome::Applied => {}
            }

            let follows_previous = last_measured_hour
                .is_some_and(|previous| header.game_hour_raw == previous.saturating_add(1));
            if last_measured_hour.is_some() && !follows_previous {
                scan.non_contiguous += 1;
            }
            last_measured_hour = Some(header.game_hour_raw);
            // The state itself is filled in at FRAME_END, at hour close.
            pending_hour = Some(D54Hour {
                game_hour: header.game_hour_raw,
                states: Vec::new(),
                ship_span: pos..record_end,
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
    if scan.hours.is_empty() {
        return Err("stream carries no usable d54 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d54 DATA
    // record of any status, and each dependency lane must be all-or-nothing.
    // This bounds what this harness canNOT validate: production runs
    // `validate_domain_data_payload_at` on EVERY domain's DATA payload
    // (`recorder_host/src/stream_fold.rs:593`), while this harness owns only d54's grammar, so a
    // d54 record whose header was relabelled — or lost outright — would
    // otherwise be silently ignored, shrinking the run. A per-capture
    // completeness assertion, NOT a grammar rule (`bench_real_d65.rs`
    // precedent).
    if d54_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d54_records} d54 DATA \
             record(s); every closed hour of a d54-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of unmodelled domains to see where \
             the difference went"
        ));
    }
    for (name, count) in [
        ("d27", scan.dependencies.d27_records),
        ("d28", scan.dependencies.d28_records),
    ] {
        if count != 0 && count != closed_hours {
            return Err(format!(
                "stream closes {closed_hours} game hour(s) but carries {count} {name} DATA \
                 record(s); d54 is auto-pulled with both dependencies, so a partially-missing \
                 {name} lane would silently hide what produced d54's status word"
            ));
        }
    }

    scan.path_evidence.distinct_keys = distinct_order_keys.len();
    scan.path_evidence.keys_seen_more_than_once =
        distinct_order_keys.values().filter(|n| **n > 1).count();
    Ok(scan)
}

/// The per-hour dependency attribution. Reported, never assumed: the collector
/// raises d54's PARTIAL from a d27 or d28 tolerated partial
/// (`attach_log.rs:46454`-`:46482`) and its SOURCE_UNAVAILABLE from a hard
/// dependency failure (`:46484`-`:46508`) — but ALSO from an orders-group manager
/// rejection (`:46521`-`:46534`), which leaves no trace in any other domain's
/// status word. So the implication runs one way only and the residue is counted
/// rather than asserted away.
fn tally_dependencies(
    coverage: &mut DependencyCoverage,
    d54_status: u32,
    d27_status: Option<u32>,
    d28_status: Option<u32>,
    game_hour: i32,
) -> Result<(), String> {
    if coverage.d27_records != 0 && d27_status.is_none() {
        coverage.hours_missing_a_dependency_record += 1;
        return Err(format!(
            "hour {game_hour} carries a d54 record but no d27 record, although the capture \
             schedules d27"
        ));
    }
    if coverage.d28_records != 0 && d28_status.is_none() {
        coverage.hours_missing_a_dependency_record += 1;
        return Err(format!(
            "hour {game_hour} carries a d54 record but no d28 record, although the capture \
             schedules d28"
        ));
    }
    let d27_partial = d27_status
        .is_some_and(|s| expected_partial_tolerance_applies(D27, s).is_some());
    let d28_partial = d28_status
        .is_some_and(|s| expected_partial_tolerance_applies(D28, s).is_some());
    let d27_failure = d27_status.is_some_and(|s| {
        compact_status_failure_flags_after_expected_partial_tolerance(D27, s) != 0
    });
    let d28_failure = d28_status.is_some_and(|s| {
        compact_status_failure_flags_after_expected_partial_tolerance(D28, s) != 0
    });
    if d27_partial {
        coverage.d27_tolerated_partial += 1;
    }
    if d28_partial {
        coverage.d28_tolerated_partial += 1;
    }
    if d27_failure {
        coverage.d27_hard_failure += 1;
    }
    if d28_failure {
        coverage.d28_hard_failure += 1;
    }

    let d54_hard = d54_status & COMPACT_STATUS_SOURCE_UNAVAILABLE != 0;
    let d54_partial_only =
        !d54_hard && d54_status & COMPACT_STATUS_PARTIAL_DATA != 0;
    if d54_partial_only {
        if d27_partial || d28_partial {
            coverage.withheld_with_a_dependency_partial += 1;
        } else {
            coverage.withheld_without_a_dependency_partial += 1;
        }
    }
    if d54_hard {
        if d27_failure || d28_failure {
            coverage.failure_with_a_dependency_failure += 1;
        } else {
            coverage.failure_without_a_dependency_failure += 1;
        }
    }
    if d54_status & d54::STATUS_DELTA_DATA != 0 {
        if d27_status.is_some_and(|s| s & d54::STATUS_DELTA_DATA != 0) {
            coverage.delta_inherited_from_d27 += 1;
        } else {
            coverage.delta_not_inherited_from_d27 += 1;
        }
    }
    Ok(())
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons: the 48-hour block cadence and a capture gap.
/// State after a gap to resume from a
/// complete baseline. Every hour this
/// domain did not successfully sample is such a gap — each of the fold's three
/// excluded-hour paths, and any non-contiguous hour step — and d54's cadence is
/// hourly, so a measured hour that does not follow the previous measured one by
/// exactly one is preceded by one. State may have moved and returned while nobody
/// was looking, so neither a delta nor a retained reader state can be justified
/// across it. **d54 needs this badly**: 248 of its 266 measured hours do not
/// follow the previous measured one.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
}

struct WireHourOutcome {
    gap_forced: bool,
    emitted: bool,
    changed_by_collection: Vec<usize>,
    tombstones_by_collection: Vec<usize>,
}

/// One wire-arm hour: the frame-run contract/block baseline decision, the encode, and the
/// per-hour reader replay over every collection. Factored out of `main` so the
/// tests execute the PRODUCTION encode path over a scanned stream.
fn encode_wire_hour(
    schema: &wire_format::Schema,
    hours_seen: usize,
    follows_previous: bool,
    game_hour: i32,
    prior: Option<&[KeyedState]>,
    states: &[KeyedState],
    reader: &mut Option<Vec<KeyedState>>,
    wire: &mut Vec<u8>,
) -> WireHourOutcome {
    let hour_start = wire.len();
    let block_start = opens_baseline(hours_seen, follows_previous);
    let gap_forced = block_start && hours_seen % BLOCK_HOURS != 0;
    let mut emitted = true;
    let mut changed_by_collection = vec![0usize; schema.collections.len()];
    let mut tombstones_by_collection = vec![0usize; schema.collections.len()];
    let opened_baseline = block_start || prior.is_none();
    if opened_baseline {
        encode_baseline(schema, states, wire).expect("baseline encodes");
    } else {
        let previous = prior.expect("prior exists");
        let deltas: Vec<Option<KeyedDelta>> = schema
            .collections
            .iter()
            .enumerate()
            .map(|(i, collection)| {
                let d = diff(collection, &previous[i], &states[i]);
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
            for (i, delta) in deltas.iter().enumerate() {
                if let Some(d) = delta {
                    changed_by_collection[i] = d.changed.len();
                    tombstones_by_collection[i] = d.tombstones.len();
                }
            }
            encode_delta(schema, previous, &deltas, wire).expect("delta encodes");
        }
    }

    // Reader replay: decode the bytes just written against the reader's OWN
    // reconstructed prior, exactly as a conforming reader would, and require the
    // result to be stored-equal to this hour's state. An hour that emitted
    // nothing is still checked: the reader's retained state must already equal
    // the fold's, which is precisely what "nothing changed" asserts.
    if emitted {
        let hour_bytes = &wire[hour_start..];
        if opened_baseline || reader.is_none() {
            let (decoded, consumed) =
                decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
            assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
            *reader = Some(decoded);
        } else {
            let mut reader_state = reader.take().expect("reader exists");
            let (deltas, consumed) =
                decode_delta(schema, &reader_state, hour_bytes, 0).expect("delta decodes");
            assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
            for (i, delta) in deltas.iter().enumerate() {
                if let Some(d) = delta {
                    apply(&mut reader_state[i], d);
                }
            }
            *reader = Some(reader_state);
        }
    }
    let replayed = reader.as_ref().expect("reader exists");
    for (i, collection) in schema.collections.iter().enumerate() {
        assert_eq!(
            replayed[i].len(),
            states[i].len(),
            "game hour {game_hour}: collection {} row count",
            collection.collection_id
        );
        assert!(
            diff(collection, &replayed[i], &states[i]).is_empty(),
            "game hour {game_hour}: collection {} reader state is not stored-equal to the fold",
            collection.collection_id
        );
    }
    WireHourOutcome {
        gap_forced,
        emitted,
        changed_by_collection,
        tombstones_by_collection,
    }
}

#[derive(Debug, Default)]
struct Measurement {
    ship: Vec<u8>,
    ship_offsets: Vec<usize>,
    wire: Vec<u8>,
    wire_offsets: Vec<usize>,
    hours_seen: usize,
    baselines: usize,
    /// Baselines the gap handling forced outside a scheduled block start.
    gap_forced_baselines: usize,
    baseline_bytes: usize,
    delta_bytes: usize,
    delta_hours: usize,
    empty_hours: usize,
    state_rows_total: usize,
    /// Delta-path churn, per collection.
    delta_changed: Vec<usize>,
    delta_tombstones: Vec<usize>,
    /// Cadence-independent churn, per collection: changed rows over MEASURED-HOUR
    /// transitions, taken before the block decision, so a change landing on a
    /// block boundary still counts.
    real_changed: Vec<usize>,
    real_tombstones: Vec<usize>,
    real_changed_hours: usize,
}

fn measure(
    schema: &wire_format::Schema,
    stream: &[u8],
    hours: &[D54Hour],
    block_hours: usize,
) -> Measurement {
    let n = schema.collections.len();
    let mut m = Measurement {
        ship_offsets: vec![0usize],
        wire_offsets: vec![0usize],
        delta_changed: vec![0; n],
        delta_tombstones: vec![0; n],
        real_changed: vec![0; n],
        real_tombstones: vec![0; n],
        ..Measurement::default()
    };
    let _ = block_hours;
    let mut prior: Option<&[KeyedState]> = None;
    let mut reader: Option<Vec<KeyedState>> = None;

    for hour in hours {
        m.ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        m.ship_offsets.push(m.ship.len());

        let states: &[KeyedState] = &hour.states;
        m.state_rows_total += states.iter().map(KeyedState::len).sum::<usize>();

        // Cadence-independent churn, taken BEFORE the block decision so a change
        // on a block boundary is counted like any other. The gap rule decides the
        // ENCODING, not what changed, so this counts a transition between every
        // pair of consecutive MEASURED states — across an excluded hour included.
        if let Some(previous) = prior {
            let mut changed_anything = false;
            for (i, collection) in schema.collections.iter().enumerate() {
                let d = diff(collection, &previous[i], &states[i]);
                if !d.is_empty() {
                    changed_anything = true;
                    m.real_changed[i] += d.changed.len();
                    m.real_tombstones[i] += d.tombstones.len();
                }
            }
            if changed_anything {
                m.real_changed_hours += 1;
            }
        }

        let hour_start = m.wire.len();
        let outcome = encode_wire_hour(
            schema,
            m.hours_seen,
            hour.follows_previous,
            hour.game_hour,
            prior,
            states,
            &mut reader,
            &mut m.wire,
        );
        let bytes = m.wire.len() - hour_start;
        if opens_baseline(m.hours_seen, hour.follows_previous) || prior.is_none() {
            m.baselines += 1;
            m.baseline_bytes += bytes;
            if outcome.gap_forced {
                m.gap_forced_baselines += 1;
            }
        } else if outcome.emitted {
            m.delta_hours += 1;
            m.delta_bytes += bytes;
            for i in 0..n {
                m.delta_changed[i] += outcome.changed_by_collection[i];
                m.delta_tombstones[i] += outcome.tombstones_by_collection[i];
            }
        } else {
            m.empty_hours += 1;
        }
        m.wire_offsets.push(m.wire.len());

        prior = Some(&hour.states);
        m.hours_seen += 1;
    }
    m
}

/// Pin every status constant `d54.rs` restates against the shipping contract, and
/// pin that the fold's own hand-written mask classifies identically to the shared
/// predicate over the whole v1 status space.
fn assert_status_predicates_agree() {
    assert_eq!(
        d54::STATUS_DELTA_DATA,
        compact_contract::COMPACT_STATUS_DELTA_DATA,
        "d54's restated delta bit must equal the shipping contract's"
    );
    // d54 IS in the tolerance table, so unlike d65 its tolerance branch is live.
    // If a future edit removed the entry, the branch would go dead and 241 of the
    // capture's hours would silently start folding — this fails the run first.
    let tolerance = compact_contract::expected_partial_tolerance(D54)
        .expect("domain 54 is one of the four expected-partial tolerance domains");
    assert_eq!(tolerance.tolerated_flags, COMPACT_STATUS_PARTIAL_DATA);

    // `fold_army_battleplans_depth` writes its own mask
    // (`world_fold.rs:21774`: `COMPACT_STATUS_FAILURE_MASK & !COMPACT_STATUS_PARTIAL_DATA`)
    // rather than calling the shared predicate. The harness uses the predicate,
    // as the recipe requires; this proves the two agree on every assigned status
    // word, so following the predicate is following the fold.
    let fold_mask = COMPACT_STATUS_FAILURE_MASK & !COMPACT_STATUS_PARTIAL_DATA;
    for status in 0u32..(1 << 16) {
        if compact_status_unknown_flags_v1(status) != 0 {
            continue;
        }
        let by_fold = status & fold_mask != 0;
        let by_predicate =
            compact_status_failure_flags_after_expected_partial_tolerance(D54, status) != 0;
        assert_eq!(
            by_fold, by_predicate,
            "status {status:#x} classifies differently under the fold's mask and the \
             shared predicate"
        );
    }
}

fn main() {
    const USAGE: &str = "usage: bench_real_d54 <collector_stream.bin> <out dir> \
                         [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    assert_status_predicates_agree();

    let stream = std::fs::read(&input).expect("read stream");
    let schema = d54::schema();
    schema.validate(0).expect("d54 schema is well formed");

    // ---- Pass 1: validate the WHOLE stream and fold its d54 DATA records. ----
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&schema, &stream, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads stream content again. --
    let m = measure(&schema, &stream, &scan.hours, BLOCK_HOURS);

    for (name, bytes) in [("ship_d54.bin", &m.ship), ("wire_new.bin", &m.wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d54.offsets", &m.ship_offsets),
        ("wire_new.offsets", &m.wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let s = &scan.stats;
    let e = &scan.path_evidence;
    let d = &scan.dependencies;
    let names = [
        "orders",
        "target_states",
        "path_geometry",
        "assigned_divisions",
        "orders_group_manager",
        "orders_group_armies",
        "orders_group_army_groups",
    ];

    println!(
        "hours                {} measured of {} d54 records ({} non-contiguous steps)",
        m.hours_seen, s.records, scan.non_contiguous
    );
    println!(
        "excluded hours       {} failure-status, {} tolerated-partial (withheld), \
         {} delta-before-baseline",
        scan.failure_records, scan.tolerated_partial_records, scan.delta_before_baseline_records
    );
    println!(
        "shipping rows        {} order rows: {} tombstones, {} live ({} in folded records, \
         {} in withheld ones)",
        s.order_rows, s.deleted_rows, s.live_rows, s.live_rows_folded, s.live_rows_withheld
    );
    println!("order types (live)   {:?}", s.order_types);
    println!(
        "header row words     row_count {}..={} (the emitted order rows), observed_army_rows \
         {}..={} (the OGS1 census width, which is what the DOMAIN_PERF row_count carries), \
         observed_order_rows {}..={}",
        s.min_row_count,
        s.max_row_count,
        s.min_observed_army_rows,
        s.max_observed_army_rows,
        s.min_observed_order_rows,
        s.max_observed_order_rows
    );
    println!(
        "child tables         path {} rows over {} orders; target {} over {}; assigned {} over {}",
        s.path_child_rows,
        s.orders_with_path,
        s.target_child_rows,
        s.orders_with_targets,
        s.assigned_child_rows,
        s.orders_with_assigned
    );
    println!(
        "target_declared_count coverage: {} absent, {} unknown, {} value",
        s.target_declared_absent, s.target_declared_unknown, s.target_declared_value
    );
    println!(
        "witness bits         non_type5_geometry_blocked {} rows, execution_blocked {} rows",
        s.non_type5_geometry_blocked, s.execution_blocked
    );
    println!(
        "OGS1 census          {} records carry one ({} army rows, {} army-group ids); \
         {} legacy records, {} of which the fold folds; {} army rows carry an unlabelled \
         execution_type",
        s.records_with_extension,
        s.census_army_rows,
        s.census_army_group_ids,
        s.legacy_records,
        s.legacy_records_folded,
        s.census_execution_type_above_labelled
    );
    println!(
        "ORDINAL EVIDENCE     {} path observations; {} distinct order keys in the stream \
         (tombstones included), {} of them serialized more than once; {} repeat observations \
         of a PATH-CARRYING key: {} unchanged, {} changed",
        e.observations, e.distinct_keys, e.keys_seen_more_than_once, e.repeats, e.unchanged,
        e.changed
    );
    println!(
        "  path shape         {} polylines repeat a province (a province-id key would collapse \
         them); {} of {} are ascending",
        s.paths_with_a_repeated_province, s.paths_ascending, s.orders_with_path
    );
    println!(
        "  path changes       {} shrank, {} grew, {} kept their prefix, {} kept their suffix, \
         {} are PURE REORDERS (d11's stop condition; a non-zero count means a keyed set stores \
         two different polylines identically)",
        e.shrank, e.grew, e.prefix_stable, e.suffix_stable, e.pure_reorders
    );
    println!(
        "  churn model        ordinal key {} of {} positions differ; province-id key would have \
         emitted {} id-set changes",
        e.ordinal_positions_differing, e.ordinal_positions_total, e.province_key_id_set_changes
    );
    println!(
        "  key choice         slot_index moved on {} of {} repeats, which is why the fold keeps \
         it OFF the key (`world_fold.rs:23874`)",
        e.slot_index_changed, e.repeats
    );
    println!(
        "DEPENDENCY COVERAGE  {} d27 records, {} d28 records, one per closed hour",
        d.d27_records, d.d28_records
    );
    println!(
        "  dependency status  d27 tolerated-partial on {} hours, hard failure on {}; \
         d28 tolerated-partial on {}, hard failure on {}",
        d.d27_tolerated_partial, d.d27_hard_failure, d.d28_tolerated_partial, d.d28_hard_failure
    );
    println!(
        "  d54 attribution    withheld hours: {} with a dependency partial, {} without; \
         source-unavailable hours: {} with a dependency failure, {} without (an orders-group \
         manager rejection leaves no trace in another domain's status word)",
        d.withheld_with_a_dependency_partial,
        d.withheld_without_a_dependency_partial,
        d.failure_with_a_dependency_failure,
        d.failure_without_a_dependency_failure
    );
    println!(
        "  delta inheritance  {} d54 delta frames whose d27 record was also a delta, {} not",
        d.delta_inherited_from_d27, d.delta_not_inherited_from_d27
    );
    println!(
        "state size           {:.1} rows/hour across all seven collections; orders peak {}, \
         and only {} of {} measured hours carry an order at all",
        m.state_rows_total as f64 / m.hours_seen.max(1) as f64,
        scan.max_orders,
        scan.hours_with_an_order,
        m.hours_seen
    );
    println!(
        "baselines            {} ({} at a {BLOCK_HOURS}-h block start, {} forced by a \
 state gap)",
        m.baselines,
        m.baselines - m.gap_forced_baselines,
        m.gap_forced_baselines
    );
    println!(
        "delta hours          {}   empty hours {}",
        m.delta_hours, m.empty_hours
    );

    let transitions = m.hours_seen.saturating_sub(1);
    let real_changed: usize = m.real_changed.iter().sum();
    let real_tombstones: usize = m.real_tombstones.iter().sum();
    let delta_changed: usize = m.delta_changed.iter().sum();
    let delta_tombstones: usize = m.delta_tombstones.iter().sum();
    println!(
        "real churn           {real_changed} changed rows and {real_tombstones} tombstone(s) \
         over {transitions} measured-hour transitions ({} of which changed anything); \
         {:.4} rows/h, {:.4} tombstones/h (cadence-independent, stored precision)",
        m.real_changed_hours,
        real_changed as f64 / transitions.max(1) as f64,
        real_tombstones as f64 / transitions.max(1) as f64
    );
    println!(
        "delta-path churn     {delta_changed} changed rows and {delta_tombstones} tombstone(s) \
         over {} emitted-delta hours; {:.4} rows/h, {:.4} tombstones/h ({} changed row(s) fell \
         on a baseline instead)",
        m.delta_hours,
        delta_changed as f64 / m.delta_hours.max(1) as f64,
        delta_tombstones as f64 / m.delta_hours.max(1) as f64,
        real_changed.saturating_sub(delta_changed)
    );
    for (i, name) in names.iter().enumerate() {
        println!(
            "  churn {name:<24} real {} changed / {} tombstones; delta-path {} / {}",
            m.real_changed[i], m.real_tombstones[i], m.delta_changed[i], m.delta_tombstones[i]
        );
    }
    println!("ship_d54.bin         {} bytes", m.ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.2}%)",
        m.wire.len(),
        100.0 * m.wire.len() as f64 / m.ship.len() as f64
    );
    println!(
        "wire raw split       baseline {} bytes in {} baselines, delta {} bytes in {} deltas",
        m.baseline_bytes, m.baselines, m.delta_bytes, m.delta_hours
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{
        COMPACT_DOMAIN_FRAME, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_PARTIAL_DATA,
    };

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
        let mut payload = vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        // One non-probe DOMAIN_PERF domain per hour in these fixtures.
        payload[16..20].copy_from_slice(&1u32.to_le_bytes());
        payload[20..24].copy_from_slice(&1u32.to_le_bytes());
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    fn perf(sequence: u64, game_hour: i32, domain: u16) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        )
    }

    /// A d54 payload: `orders` live order rows carrying `paths` between them, and
    /// a v2 census of `armies` armies.
    fn payload(status_flags: u32, paths: &[&[u32]], armies: u32) -> Vec<u8> {
        let mut header = Vec::new();
        let path_total: usize = paths.iter().map(|p| p.len()).sum();
        header.extend_from_slice(&0u64.to_le_bytes());
        header.extend_from_slice(&(paths.len() as u32).to_le_bytes());
        header.extend_from_slice(&(paths.len() as u32).to_le_bytes());
        header.extend_from_slice(&status_flags.to_le_bytes());
        header.extend_from_slice(&armies.to_le_bytes());
        header.extend_from_slice(&(paths.len() as u32).to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes()); // target_count
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&(path_total as u32).to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes()); // assigned_division_count
        header.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(header.len(), d54::HEADER_LEN);

        let mut cursor = 0u32;
        for (index, path) in paths.iter().enumerate() {
            let mut row = Vec::new();
            row.extend_from_slice(&(1_000u32 + index as u32).to_le_bytes()); // owner_id
            row.extend_from_slice(&(7u64).to_le_bytes()); // generation
            for word in [
                d54::FLAG_ACTIVE_VALUE | d54::FLAG_PATH_GEOMETRY,
                0u32,                             // slot_index
                d54::ORDER_TYPE_LINE as u32,      // order_type
                cursor,                           // child_first
                path.len() as u32,                // child_count
                0,                                // target_declared
                d54::STATUS_UNSUPPORTED,
                d54::STATUS_EXACT,
                d54::STATUS_EXACT,
                d54::STATUS_EXACT,
                d54::STATUS_EXACT,
                d54::STATUS_UNKNOWN,
                d54::STATUS_UNKNOWN,
            ] {
                row.extend_from_slice(&word.to_le_bytes());
            }
            row.extend_from_slice(&(500u32 + index as u32).to_le_bytes()); // instance
            row.extend_from_slice(&0u32.to_le_bytes()); // assigned_first
            row.extend_from_slice(&0u32.to_le_bytes()); // assigned_count
            row.extend_from_slice(&1u32.to_le_bytes()); // active
            row.extend_from_slice(&d54::STATUS_EXACT.to_le_bytes());
            row.extend_from_slice(&d54::ORDER_OWNER_KIND_ARMY.to_le_bytes());
            assert_eq!(row.len(), d54::ROW_LEN);
            header.extend_from_slice(&row);
            cursor += path.len() as u32;
        }
        for path in paths {
            for province in path.iter() {
                header.extend_from_slice(&province.to_le_bytes());
                header.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        // OGS1 v2 census.
        header.extend_from_slice(&d54::EXTENSION_MAGIC);
        header.extend_from_slice(&d54::EXTENSION_VERSION_V2.to_le_bytes());
        header.extend_from_slice(&(d54::EXTENSION_HEADER_LEN as u16).to_le_bytes());
        header.extend_from_slice(&armies.to_le_bytes());
        header.extend_from_slice(&armies.to_le_bytes()); // registered_entry_count
        header.extend_from_slice(&0u32.to_le_bytes()); // army_group_count
        header.extend_from_slice(&d54::EXTENSION_FLAG_COMPLETE_MANAGER.to_le_bytes());
        for index in 0..armies {
            for word in [index + 1, 0u32, 0u32, 0u32] {
                header.extend_from_slice(&word.to_le_bytes());
            }
        }
        header
    }

    /// A stream of `statuses.len()` consecutive hours, each carrying one d54
    /// record.
    fn stream_of(hours: &[(u32, Vec<Vec<u32>>)]) -> Vec<u8> {
        let mut out = Vec::new();
        // H4CR sequences are 1-based (`compact_contract`'s validator starts at 1).
        let mut sequence = 1u64;
        for (index, (status, paths)) in hours.iter().enumerate() {
            let hour = 100 + index as i32;
            let borrowed: Vec<&[u32]> = paths.iter().map(|p| p.as_slice()).collect();
            out.extend_from_slice(&perf(sequence, hour, D54));
            sequence += 1;
            out.extend_from_slice(&record(
                sequence,
                hour,
                D54,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload(*status, &borrowed, 2),
            ));
            sequence += 1;
            out.extend_from_slice(&frame_end(sequence, hour));
            sequence += 1;
        }
        out
    }

    #[test]
    fn the_status_predicates_agree_with_the_shipping_contract() {
        assert_status_predicates_agree();
    }

    #[test]
    fn the_tolerance_entry_still_exists() {
        // d54 IS listed, so its tolerance branch is LIVE — the opposite of d65,
        // whose branch is dead because the table does not list it. If a future
        // edit removed the entry, 241 of this capture's hours would silently
        // start folding; this test fails first.
        let tolerance = compact_contract::expected_partial_tolerance(D54)
            .expect("domain 54 is a tolerance domain");
        assert_eq!(tolerance.tolerated_flags, COMPACT_STATUS_PARTIAL_DATA);
        assert!(
            expected_partial_tolerance_applies(D54, COMPACT_STATUS_PARTIAL_DATA).is_some(),
            "a lone PARTIAL bit is tolerated for d54"
        );
        assert!(
            expected_partial_tolerance_applies(
                D54,
                COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE
            )
            .is_none(),
            "PARTIAL beside a hard bit is not a tolerated partial"
        );
    }

    #[test]
    fn a_clean_stream_scans_and_measures() {
        let schema = d54::schema();
        let stream = stream_of(&[
            (0, vec![vec![10, 20, 30]]),
            (COMPACT_STATUS_DELTA_DATA, vec![vec![10, 25, 30]]),
        ]);
        let scan = scan_stream(&schema, &stream, StreamIdentityMode::Legacy).expect("scans");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.path_evidence.changed, 1);
        assert_eq!(scan.path_evidence.pure_reorders, 0);
        let m = measure(&schema, &stream, &scan.hours, BLOCK_HOURS);
        assert_eq!(m.hours_seen, 2);
        assert_eq!(m.baselines, 1, "the second hour follows the first, so it deltas");
        assert_eq!(m.delta_hours, 1);
        assert!(m.wire.len() < m.ship.len());
    }

    #[test]
    fn a_withheld_hour_forces_the_next_state_to_be_a_baseline() {
        // Without the complete-baseline-after-gap rule, the wire arm under-reports its
        // own size and over-claims its replay verification: state may have moved
        // and returned across the withheld hour, and where the two states happen
        // to be equal a delta arm emits nothing at all.
        let schema = d54::schema();
        let stream = stream_of(&[
            (0, vec![vec![10, 20, 30]]),
            (COMPACT_STATUS_PARTIAL_DATA, vec![vec![99]]),
            (0, vec![vec![10, 20, 30]]),
        ]);
        let scan = scan_stream(&schema, &stream, StreamIdentityMode::Legacy).expect("scans");
        assert_eq!(scan.tolerated_partial_records, 1);
        assert_eq!(scan.hours.len(), 2);
        assert!(!scan.hours[1].follows_previous);
        let m = measure(&schema, &stream, &scan.hours, BLOCK_HOURS);
        assert_eq!(m.baselines, 2, "the gap forces a second baseline");
        assert_eq!(m.gap_forced_baselines, 1);
        assert_eq!(m.delta_hours, 0);
        assert_eq!(m.empty_hours, 0);
    }

    #[test]
    fn the_gap_rule_does_not_touch_the_churn_counters() {
        // d50's regression: the gap rule decides the ENCODING, not what changed.
        // A transition across a withheld hour is still a transition.
        let schema = d54::schema();
        let stream = stream_of(&[
            (0, vec![vec![10, 20, 30]]),
            (COMPACT_STATUS_PARTIAL_DATA, vec![vec![99]]),
            (0, vec![vec![10, 40, 30]]),
        ]);
        let scan = scan_stream(&schema, &stream, StreamIdentityMode::Legacy).expect("scans");
        let m = measure(&schema, &stream, &scan.hours, BLOCK_HOURS);
        assert_eq!(
            m.real_changed[2], 1,
            "one waypoint moved across the withheld hour and it is counted"
        );
        assert_eq!(m.delta_changed[2], 0, "but no delta was emitted for it");
    }

    #[test]
    fn a_withheld_frame_is_still_validated_in_full() {
        // The recorder validates a tolerated-partial frame's every row before it
        // withholds it, so a malformed one is a corrupt stream there too — an
        // `Err` from `scan_stream`, never a quietly skipped hour.
        let schema = d54::schema();
        let mut stream = stream_of(&[(0, vec![vec![10]]), (COMPACT_STATUS_PARTIAL_DATA, vec![vec![10]])]);
        // Corrupt the second record's path child id to zero.
        let needle = 10u32.to_le_bytes();
        let last = stream
            .windows(4)
            .enumerate()
            .filter(|(_, w)| *w == needle)
            .map(|(i, _)| i)
            .next_back()
            .expect("a path child row");
        stream[last..last + 4].copy_from_slice(&0u32.to_le_bytes());
        assert!(scan_stream(&schema, &stream, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        let schema = d54::schema();
        let stream = stream_of(&[(1 << 20, vec![vec![10]])]);
        let error = scan_stream(&schema, &stream, StreamIdentityMode::Legacy)
            .expect_err("an unassigned status bit refuses the stream");
        assert!(error.contains("unassigned"), "{error}");
    }

    #[test]
    fn two_d54_records_in_one_hour_refuse_the_stream() {
        let schema = d54::schema();
        let mut stream = Vec::new();
        stream.extend_from_slice(&perf(1, 100, D54));
        stream.extend_from_slice(&record(
            2,
            100,
            D54,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[&[10]], 2),
        ));
        // A second record in the same hour, this one a failure — the pair the
        // uniqueness rule exists to catch.
        stream.extend_from_slice(&record(
            3,
            100,
            D54,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA, &[], 2),
        ));
        stream.extend_from_slice(&frame_end(4, 100));
        assert!(scan_stream(&schema, &stream, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_relabelled_d54_record_refuses_the_stream() {
        // The completeness bound: every closed hour must carry exactly one d54
        // record, because this harness cannot validate an unmodelled domain's
        // payload to see where a missing one went.
        let schema = d54::schema();
        let mut stream = Vec::new();
        stream.extend_from_slice(&perf(1, 100, D54));
        stream.extend_from_slice(&record(
            2,
            100,
            D54,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[&[10]], 2),
        ));
        stream.extend_from_slice(&frame_end(3, 100));
        stream.extend_from_slice(&perf(4, 101, D54));
        stream.extend_from_slice(&frame_end(5, 101));
        assert!(scan_stream(&schema, &stream, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_truncated_stream_refuses_rather_than_measuring_a_prefix() {
        let schema = d54::schema();
        let full = stream_of(&[(0, vec![vec![10, 20]])]);
        for cut in 1..full.len() {
            let scanned = scan_stream(&schema, &full[..cut], StreamIdentityMode::Legacy);
            assert!(
                scanned.is_err(),
                "a stream cut at byte {cut} must be refused, not measured"
            );
        }
    }

    #[test]
    fn the_transcribed_strides_are_checked_against_every_record() {
        // The stand-in for the d67/d70 dual-decode: a wrong table stride cannot
        // reproduce a real record's own length.
        let bytes = payload(0, &[&[10, 20, 30]], 2);
        let split = d54::split_payload(&bytes).expect("payload splits");
        length_is_reproduced_by_the_transcribed_strides(&split, bytes.len())
            .expect("the transcribed strides reproduce the length");
        assert!(
            length_is_reproduced_by_the_transcribed_strides(&split, bytes.len() + 1).is_err()
        );
    }
}
