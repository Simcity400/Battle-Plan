//! Measure the wire format against the shipping compact stream on REAL d67
//! `country_dynamic_modifiers_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d67-scheduled capture
//! (`runs/batch3/deepwar_b2`). The stream's own d67 DATA records are the shipping
//! format's cost for the domain: a 104-byte `DMI2` header, a fixed 28-byte row
//! per instance, the concatenated modifier-id text and the concatenated `i64`
//! value block, full-replace every hour, exactly what the recorder stores today.
//! This tool replays those records into the per-hour state
//! `world_fold::fold_country_dynamic_modifier_instances` builds and re-encodes
//! every hour with this crate's grammar: a BASELINE at each 48-hour block start,
//! field-granular stored-minimal DELTAs between.
//!
//! **Two passes, because the symbol table is what pass 1 builds.** §5 requires
//! symbol ids to be file-global and forbids in-stream interning — "it must be
//! fully read before any seek" — so the `modifier_id` vocabulary is collected
//! over the whole validated stream first and frozen into a `d67::Symbols` that
//! pass 2 only reads (recipe step 9; d29 is the precedent). Pass 1 still exercises
//! the *identical* fold path rather than a mirror of it: it calls
//! `d67::fold_payload` with a **record-local** symbol table holding exactly that
//! record's own ids. That proves pass 2 can never fail a lookup pass 1 did not
//! make — the global table is the union of the record-local ones, so every symbol
//! pass 2 resolves was resolved in pass 1 — and it means every rejection path in
//! the fold (roster admission, zero-removal admission, duplicate identity,
//! ordinal overflow) belongs to the pass that accepts or refuses the stream.
//! The record-local table is injective over that record's ids exactly as the
//! global one is, so it cannot introduce or hide a key collision.
//!
//! **The d18 roster is modelled, not assumed.** d67's fold refuses a DMI2 frame
//! unless the current hour's d18 `country_identity` roster has been folded
//! (`world_fold.rs:9869`-`:9874`) and refuses any instance whose owner is not in
//! it (`:9878`-`:9883`). This harness therefore tracks the roster exactly as
//! `WorldFold` does: a d18 DATA record with a non-failure status replaces it
//! (`:14148`-`:14156`), a failure-status one leaves it unrefreshed (`:13924`-`:13931`),
//! and every FRAME_END clears the "known this hour" flag (`:3002`). It keeps only
//! the column the admission needs — each row's `tag_db_id` — but it **validates
//! the whole d18 payload under production's own authority**, `CIM1` extension
//! included (`domain_data_authority.rs:1012`-`:1283`), because a roster this
//! harness accepted while production refused the record would be a measurement of
//! a stream that does not exist; see [`parse_d18_roster`].
//!
//! **Nothing here re-derives a contract it can call.** Payload decoding and the
//! frame fold go through `wire_format::d67`'s public boundary; H4CR framing,
//! record semantics and sequence continuity go through `compact_contract`'s own
//! parser and stateful validator; failure classification goes through
//! `compact_contract`'s own predicates rather than a copied bit mask.
//!
//! **And the one contract d67 has to duplicate is machine-checked, not trusted.**
//! d67 is not admitted to the production-codec dependency surface
//! (`Cargo.toml`, `d22.rs:470`), so `d67::decode_payload` transcribes the `DMI2`
//! grammar with per-constant citations. This binary *does*
//! depend on `compact_contract`, so it checks the transcription two ways:
//! [`check_authority_payload`] decodes **every** d67 DATA record with both
//! decoders and requires identical frames, and the unit test
//! `the_two_decoders_agree_on_every_fuzzed_payload` requires identical
//! accept/reject verdicts over a byte-level fuzz sweep. Every transcribed
//! constant is asserted equal to its original in
//! `every_transcribed_constant_equals_the_shipping_one`.
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
//! - that tracker's DOMAIN_PERF hour binding (`:328`-`:365`);
//! - the FRAME_END seal fields, `validate_frame_end_seal_fields` (`:46`-`:89`),
//!   re-implemented as [`check_frame_end_seal`], plus the seal's domain-count
//!   cross-check against the distinct non-probe PERF domains observed in the
//!   hour (`:411`-`:419`);
//! - the EOF rules of `eof_check` (`:459`): no residual bytes, no hour left open,
//!   at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of `verify_stream_logical_boundary` (`:537`).
//! The one production check outside it is a crate-boundary limitation rather than
//! a choice: production calls `validate_domain_data_payload_at` on EVERY
//! DOMAIN_DATA record (`:593`), decoding each domain's payload grammar through
//! `recorder_host::domain_data_authority`. `wire_format` cannot depend on
//! `recorder_host`, so the only DOMAIN_DATA payloads this harness validates are
//! d67's — which it does, fully and fail-closed, on every record whatever its
//! status, exactly as that authority does
//! (`domain_data_authority.rs:12509`-`:12568`) — and d18's, under that same
//! authority's d18 arm (`:1012`-`:1283`). This capture's other DATA domains are
//! 68, 94 and 95; their payloads are checked by the recorder that produced the
//! run, not here, so a hand-made stream carrying a corrupt d68/d94/d95 payload
//! would measure here and not there. That unvalidated remainder is explicitly
//! BOUNDED: every closed game hour must carry exactly one d67 DATA record of
//! any status, and the d18 lane must be all-or-nothing — exactly one record
//! per closed hour when scheduled (the completeness checks in [`scan_stream`]),
//! so a d67 or d18 record relabelled to another domain id — or dropped —
//! refuses the run instead of silently shrinking it or silently starving an
//! excluded hour's roster refresh.
//!
//! **The three outcomes are named.** For every non-folding path in
//! `fold_country_dynamic_modifiers`:
//!
//! | path | `world_fold.rs` | outcome here |
//! |---|---|---|
//! | record flag clear → legacy v1 grammar | `:9680`, `:9696` | **stream rejection** — a second grammar this port does not implement (see `d67`'s module docs). 0 of 1033 records. |
//! | duplicate DMI2 frame in one hour | `:9844`-`:9847` | **stream rejection** |
//! | `DMI2` magic missing under the flag | `:9684`-`:9686` | **stream rejection** |
//! | codec `Err` (any structural rule) | `:9848`-`:9849` | **stream rejection** |
//! | producer authority not exact-a729 | `:9850`-`:9852` | **stream rejection** |
//! | failure status | `:9853`-`:9861` | **excluded hour** — the accumulator is retained and the hour carries no d67 data |
//! | no valid frame hour | `:9867`-`:9868` | **stream rejection** |
//! | no current-hour d18 roster | `:9869`-`:9874` | **stream rejection** |
//! | an instance owner outside the roster | `:9878`-`:9883` | **stream rejection** |
//! | a synchronous zero-removal event | `:9894`-`:9917` | **stream rejection** — §9 events are stop-reported, not silently dropped. 0 of 1033 records. |
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d67.bin   — the d67 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the shipping arm's byte split, the real churn surface both
//! cadence-independently and along the delta path, and the symbol-id assignment
//! band (`d67::Symbols`).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_is_failure,
    compact_status_unknown_flags_v1, parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN,
    COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, check_minimal, decode_baseline, decode_delta, diff, encode_baseline, encode_delta,
    KeyedDelta, KeyedState,
};
use wire_format::d67;
use wire_format::Schema;

const D67: u16 = 67;
const D18: u16 = 18;
const BLOCK_HOURS: usize = 48;

/// d18's payload header: `elapsed_wall_ms:u64`, `row_count:u32`,
/// `changed_rows:u32`, `status_flags:u32` (`world_fold.rs:13919`-`:13922`).
const D18_HEADER_LEN: usize = 20;

/// One usable d67 hour: its clocks, the d18 roster in force when its record was
/// folded, and its byte spans in the source stream.
#[derive(Debug)]
struct D67Hour {
    game_hour: i32,
    frame_hour: i64,
    roster: d67::CountryRoster,
    ship_span: Range<usize>,
    payload_span: Range<usize>,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded failure-status hour or a non-contiguous hour step — which is
    /// what forces the encoder to open a fresh baseline (`opens_baseline`).
    follows_previous: bool,
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences:
/// the 48-hour block cadence (every block opens with its own baseline, §1.5's
/// closing paragraph), and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap
/// affecting the state part forces the next state observation to be a
/// `BASELINE`." Every hour this domain did not successfully sample is such a
/// gap — the excluded failure-status hour, and any non-contiguous hour step —
/// and d67's cadence is hourly, so a measured hour that does not follow the
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
    changed_rows: usize,
    tombstones: usize,
}

#[allow(clippy::too_many_arguments)]
fn encode_wire_hour(
    schema: &Schema,
    hours_seen: usize,
    follows_previous: bool,
    game_hour: i32,
    prior: Option<&d67::State>,
    state: &d67::State,
    reader: &mut Vec<KeyedState>,
    wire: &mut Vec<u8>,
    symbol_occurrences: &mut BTreeMap<u64, usize>,
) -> WireHourOutcome {
    let hour_start = wire.len();
    // BASELINE at each block start — and at every §1.5 gap, which forces a
    // fresh baseline and restarts the delta chain (`opens_baseline`).
    let block_start = opens_baseline(hours_seen, follows_previous);
    let gap_forced = block_start && hours_seen % BLOCK_HOURS != 0;
    let mut emitted = true;
    let mut changed_rows = 0usize;
    let mut tombstones = 0usize;
    if block_start {
        encode_baseline(schema, &state.as_slice(), wire).expect("baseline encodes");
        for i in 0..d67::COLLECTION_COUNT {
            for k in state.get(i).keys() {
                *symbol_occurrences.entry(k.0[1] as u64).or_default() += 1;
            }
        }
    } else {
        let prev = prior.expect("a non-block-start hour has a prior");
        let deltas: Vec<Option<KeyedDelta>> = (0..d67::COLLECTION_COUNT)
            .map(|i| {
                let d = diff(&schema.collections[i], prev.get(i), state.get(i));
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
                for row in &delta.changed {
                    *symbol_occurrences.entry(row.key.0[1] as u64).or_default() += 1;
                }
                for k in &delta.tombstones {
                    *symbol_occurrences.entry(k.0[1] as u64).or_default() += 1;
                }
                // §7.1 minimality, checked rather than assumed.
                check_minimal(&schema.collections[i], prev.get(i), state.get(i), delta)
                    .unwrap_or_else(|e| panic!("hour {game_hour}: {e:?}"));
            }
            encode_delta(schema, &prev.as_slice(), &deltas, wire).expect("delta encodes");
        }
    }
    let bytes = wire.len() - hour_start;

    // Reader replay: decode the bytes just written against the reader's OWN
    // reconstructed prior, exactly as a conforming reader would, and require
    // the result to be stored-equal to the fold, every hour.
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
    for i in 0..d67::COLLECTION_COUNT {
        assert_eq!(
            reader[i].len(),
            state.get(i).len(),
            "game hour {game_hour}: collection {} row count",
            schema.collections[i].collection_id
        );
        assert!(
            diff(&schema.collections[i], &reader[i], state.get(i)).is_empty(),
            "game hour {game_hour}: collection {} is not stored-equal to the fold",
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

#[derive(Debug, Default)]
struct ScanCounts {
    /// d67 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d67 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// d18 records that carried a failure status and so did not refresh the
    /// roster (`world_fold.rs:13924`-`:13931`).
    d18_failure_records: usize,
    instances_max: usize,
    values_max: usize,
    /// The shipping arm's byte split, over the measured records only.
    ship_record_framing: usize,
    ship_payload_header: usize,
    ship_instance_rows: usize,
    ship_id_blob: usize,
    ship_value_block: usize,
    /// The stop-reported §9 slice: zero-removal rows and the bytes they own.
    stopped_zero_removal_rows: usize,
    stopped_zero_removal_bytes: usize,
}

#[derive(Debug)]
struct StreamScan {
    hours: Vec<D67Hour>,
    vocabulary: BTreeSet<String>,
    counts: ScanCounts,
}

/// Every globally-defined compact status occupies one of bits 0..=14. FRAME_END
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
/// depend on `recorder_host`. Returns the seal's
/// `(emitted_domains, scheduled_domains)` for production's cross-check
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

/// Production's **second** authority on a d67 DOMAIN_DATA record —
/// `validate_domain_data_payload_at` →
/// `decode_compact_country_dynamic_modifiers_data_payload`
/// (`recorder_host/src/domain_data_authority.rs:12509`-`:12568`). It runs on
/// every record whatever its status, and any error it reports makes the record
/// `malformed_payload_records`, which `strict_stream_certification_verdict`
/// refuses. For a schema-v2 record that authority is exactly
/// `compact_contract::dynamic_modifiers::decode_payload` plus the magic check
/// (`:12516`-`:12523`), so this calls it directly.
///
/// It then requires `d67::decode_payload` — the transcription retained because
/// d67 is not admitted to the production-codec dependency surface — to have
/// produced the identical frame. That equality is the whole reason the duplication is
/// safe, and it is checked on every real record rather than argued.
fn check_authority_payload(
    frame: &d67::Frame,
    payload: &[u8],
    record_flags: u32,
    at_byte: usize,
) -> Result<(), String> {
    if !compact_contract::dynamic_modifiers::is_v2_payload(payload) {
        return Err(format!(
            "d67 payload at byte {at_byte}: schema-v2 record omitted DMI2 magic"
        ));
    }
    let shipping = compact_contract::dynamic_modifiers::decode_payload(payload)
        .map_err(|e| format!("d67 payload at byte {at_byte}: DMI2 invalid: {e:?}"))?;
    // `world_fold.rs:9850`-`:9852`, and the authority's own gate.
    if !shipping.authority.is_exact_a729() {
        return Err(format!(
            "d67 payload at byte {at_byte}: DMI2 producer authority rejected"
        ));
    }
    if record_flags & COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2 == 0 {
        return Err(format!(
            "d67 payload at byte {at_byte}: DMI2 payload without the schema-v2 record flag"
        ));
    }
    if !frames_agree(frame, &shipping) {
        return Err(format!(
            "d67 payload at byte {at_byte}: the transcribed decoder and \
             compact_contract::dynamic_modifiers disagree"
        ));
    }
    Ok(())
}

/// Whole-frame equality between the transcription and the shipping codec.
fn frames_agree(
    ours: &d67::Frame,
    theirs: &compact_contract::dynamic_modifiers::DynamicModifierFrame,
) -> bool {
    if ours.header.elapsed_wall_ms != theirs.elapsed_wall_ms
        || ours.header.status_flags != theirs.status_flags
        || ours.header.instance_count as usize != theirs.instances.len()
        || ours.header.zero_removal_count as usize != theirs.zero_removals.len()
        || ours.instances.len() != theirs.instances.len()
        || ours.zero_removals.len() != theirs.zero_removals.len()
    {
        return false;
    }
    let instances_match = ours.instances.iter().zip(&theirs.instances).all(|(a, b)| {
        a.owner_tag_db_id == b.owner_tag_db_id
            && a.modifier_id == b.modifier_id
            && a.enabled == b.enabled
            && a.values_raw == b.values_raw
            && a.remaining_days == b.remaining_days
    });
    let events_match = ours
        .zero_removals
        .iter()
        .zip(&theirs.zero_removals)
        .all(|(a, b)| {
            a.owner_tag_db_id == b.owner_tag_db_id
                && a.modifier_id == b.modifier_id
                && a.enabled == b.enabled
                && a.values_raw == b.values_raw
                && a.observed_game_hour_raw == b.observed_game_hour_raw
        });
    instances_match && events_match
}

/// The d18 `country_identity` roster — the input d67's admission reads — parsed
/// under **production's own d18 payload authority**.
///
/// Grammar, `world_fold.rs:13918`-`:13971`: a 20-byte header
/// (`elapsed_wall_ms`, `row_count`, `changed_rows`, `status_flags`), then
/// `row_count` variable rows of `tag_db_id:u32`, a u16-length-prefixed UTF-8 tag
/// (`read_string`, `:30256`) and a 1-byte cosmetic marker — `0` absent, `1` null,
/// `2` followed by another length-prefixed string — then the optional `CIM1`
/// claims/cores/dynamic-major extension.
///
/// **Every rule here is one of production's two authorities on a d18 record**,
/// because the roster is an input to d67's admission and a stream this harness
/// accepted while production refused it would be a measurement of a stream that
/// does not exist:
///
/// - `decode_compact_country_identity_data_payload`
///   (`recorder_host/src/domain_data_authority.rs:1012`-`:1283`), which runs on
///   every DOMAIN_DATA record whatever its status and whose complaint makes the
///   record `malformed_payload_records`. It contributes `changed_rows <=
///   row_count` (`:1029`), the row walk, and the whole `CIM1` grammar down to its
///   trailing-byte rule (`:1251`).
/// - `fold_country_identity` (`world_fold.rs:13918`), whose own rules on this
///   payload are a subset of the above.
///
/// The authority accumulates several complaints before returning while this
/// returns on the first, which is the same accept/reject verdict: a record with
/// one complaint is refused either way.
///
/// **Nothing allocates on a declared count.** `row_count` is an attacker-shaped
/// `u32` from a file, so the row set grows as rows are walked rather than being
/// reserved up front — a `Vec::with_capacity(row_count)` here was a round-1
/// review finding (a 20-byte payload declaring `u32::MAX` rows asked for 16 GiB).
fn parse_d18_roster(payload: &[u8], at_byte: usize) -> Result<d67::CountryRoster, String> {
    if payload.len() < D18_HEADER_LEN {
        return Err(format!(
            "d18 payload at byte {at_byte}: {} byte(s) is shorter than its {D18_HEADER_LEN}-byte \
             header",
            payload.len()
        ));
    }
    let row_count = u32::from_le_bytes(payload[8..12].try_into().expect("4 bytes")) as usize;
    let changed_rows = u32::from_le_bytes(payload[12..16].try_into().expect("4 bytes")) as usize;
    // `domain_data_authority.rs:1029`.
    if changed_rows > row_count {
        return Err(format!(
            "d18 payload at byte {at_byte}: changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    // Grows with the rows actually walked; never reserved from `row_count`.
    let mut ids: BTreeSet<u32> = BTreeSet::new();
    let mut cursor = D18_HEADER_LEN;
    let read_string = |cursor: &mut usize, what: &str| -> Result<usize, String> {
        if *cursor + 2 > payload.len() {
            return Err(format!("d18 payload at byte {at_byte}: {what} length prefix truncated"));
        }
        let len = u16::from_le_bytes(payload[*cursor..*cursor + 2].try_into().expect("2")) as usize;
        *cursor += 2;
        if *cursor + len > payload.len() {
            return Err(format!("d18 payload at byte {at_byte}: {what} bytes truncated"));
        }
        std::str::from_utf8(&payload[*cursor..*cursor + len])
            .map_err(|_| format!("d18 payload at byte {at_byte}: {what} is not utf8"))?;
        *cursor += len;
        Ok(len)
    };
    for row in 0..row_count {
        if cursor + 4 > payload.len() {
            return Err(format!("d18 payload at byte {at_byte}: row {row} tag_db_id truncated"));
        }
        let tag_db_id =
            u32::from_le_bytes(payload[cursor..cursor + 4].try_into().expect("4 bytes"));
        cursor += 4;
        let tag_len = read_string(&mut cursor, "tag")?;
        // `world_fold.rs:13939`-`:13944`.
        if tag_db_id == 0 {
            return Err(format!("d18 payload at byte {at_byte}: row {row} has a zero tag_db_id"));
        }
        if tag_len == 0 {
            return Err(format!("d18 payload at byte {at_byte}: row {row} has an empty tag"));
        }
        if cursor >= payload.len() {
            return Err(format!(
                "d18 payload at byte {at_byte}: row {row} cosmetic marker truncated"
            ));
        }
        let marker = payload[cursor];
        cursor += 1;
        match marker {
            0 | 1 => {}
            2 => {
                let len = read_string(&mut cursor, "cosmetic tag")?;
                if len == 0 {
                    return Err(format!(
                        "d18 payload at byte {at_byte}: row {row} present cosmetic marker with an \
                         empty string"
                    ));
                }
            }
            other => {
                return Err(format!(
                    "d18 payload at byte {at_byte}: row {row} invalid cosmetic marker {other}"
                ))
            }
        }
        // `world_fold.rs:13945`-`:13949`, `domain_data_authority.rs:1105`-`:1108`:
        // a duplicated identity is a rejected frame.
        if !ids.insert(tag_db_id) {
            return Err(format!(
                "d18 payload at byte {at_byte}: row {row} duplicates tag_db_id {tag_db_id}"
            ));
        }
    }
    // The optional `CIM1` claims/cores/dynamic-major extension, under the
    // authority's own rules (`domain_data_authority.rs:1134`-`:1252`, mirroring
    // `world_fold.rs:14002`-`:14147`). d67 reads none of it, but production
    // refuses a record whose tail is malformed, so a harness that waved it
    // through would measure a stream the recorder rejects.
    if cursor != payload.len() {
        cursor = parse_d18_cim1(payload, cursor, &ids, at_byte)?;
    }
    // `domain_data_authority.rs:1277`-`:1282`.
    if cursor != payload.len() {
        return Err(format!(
            "d18 payload at byte {at_byte}: {} trailing byte(s) after {row_count} row(s)",
            payload.len() - cursor
        ));
    }
    Ok(d67::CountryRoster::from_ids(ids))
}

/// The `CIM1` extension, exactly as production's d18 authority validates it
/// (`domain_data_authority.rs:1134`-`:1252`). Returns the cursor after the
/// extension. Total: no payload of any length or content panics, and nothing
/// allocates on a declared count.
fn parse_d18_cim1(
    payload: &[u8],
    start: usize,
    tag_ids: &BTreeSet<u32>,
    at_byte: usize,
) -> Result<usize, String> {
    const CIM1_HEADER_LEN: usize = 40;
    let bad = |what: &str| format!("d18 payload at byte {at_byte}: CIM1 {what}");
    let u16_at = |at: usize| u16::from_le_bytes(payload[at..at + 2].try_into().expect("2 bytes"));
    let u32_at = |at: usize| u32::from_le_bytes(payload[at..at + 4].try_into().expect("4 bytes"));

    if payload.len().saturating_sub(start) < CIM1_HEADER_LEN
        || &payload[start..start + 4] != b"CIM1"
        || u16_at(start + 4) != 1
        || usize::from(u16_at(start + 6)) != CIM1_HEADER_LEN
        || u32_at(start + 36) != 0
    {
        return Err(bad("header is invalid"));
    }
    let claims_status = u32_at(start + 8);
    let cores_status = u32_at(start + 12);
    let major_status = u32_at(start + 16);
    let states = u32_at(start + 20) as usize;
    let expected_claims = u32_at(start + 24) as usize;
    let expected_cores = u32_at(start + 28) as usize;
    let majors = u32_at(start + 32) as usize;
    let claims_available = !compact_status_is_failure(claims_status);
    let cores_available = !compact_status_is_failure(cores_status);
    let major_available = !compact_status_is_failure(major_status);
    if states > 100_000
        || expected_claims > 1_000_000
        || expected_cores > 1_000_000
        || majors > tag_ids.len()
        || (!claims_available && expected_claims != 0)
        || (!cores_available && expected_cores != 0)
        || (!claims_available && !cores_available && states != 0)
        || (!major_available && majors != 0)
        || (major_available && majors != tag_ids.len())
    {
        return Err(bad("status/count contract is invalid"));
    }

    let mut at = start + CIM1_HEADER_LEN;
    let mut claim_total = 0usize;
    let mut core_total = 0usize;
    let mut previous_state = 0u32;
    for state_index in 0..states {
        if payload.len().saturating_sub(at) < 12 {
            return Err(bad(&format!("state row {state_index} is truncated")));
        }
        let state_id = u32_at(at);
        let claim_count = u32_at(at + 4) as usize;
        let core_count = u32_at(at + 8) as usize;
        at += 12;
        if state_id == 0 || (state_index != 0 && state_id <= previous_state) {
            return Err(bad(&format!("state row {state_index} is not strictly ordered")));
        }
        previous_state = state_id;
        if (!claims_available && claim_count != 0) || (!cores_available && core_count != 0) {
            return Err(bad(&format!("unavailable state row {state_id} carries members")));
        }
        let member_bytes = claim_count
            .checked_add(core_count)
            .and_then(|count| count.checked_mul(4))
            .ok_or_else(|| bad("state member size overflow"))?;
        if payload.len().saturating_sub(at) < member_bytes {
            return Err(bad(&format!("state row {state_id} members are truncated")));
        }
        for (label, count, total) in [
            ("claim", claim_count, &mut claim_total),
            ("core", core_count, &mut core_total),
        ] {
            let mut previous_tag = 0u32;
            for member_index in 0..count {
                let tag = u32_at(at);
                at += 4;
                if tag == 0
                    || (member_index != 0 && tag <= previous_tag)
                    || !tag_ids.contains(&tag)
                {
                    return Err(bad(&format!("state {state_id} {label} member is invalid")));
                }
                previous_tag = tag;
                *total += 1;
            }
        }
    }
    if claim_total != expected_claims || core_total != expected_cores {
        return Err(bad("membership totals do not match the header"));
    }

    let mut major_ids: BTreeSet<u32> = BTreeSet::new();
    let mut previous_major = 0u32;
    for major_index in 0..majors {
        if payload.len().saturating_sub(at) < 8 {
            return Err(bad(&format!("major row {major_index} is truncated")));
        }
        let tag = u32_at(at);
        let marker = payload[at + 4];
        if tag == 0
            || (major_index != 0 && tag <= previous_major)
            || marker > 1
            || payload[at + 5..at + 8] != [0, 0, 0]
            || !tag_ids.contains(&tag)
        {
            return Err(bad(&format!("major row {major_index} is invalid")));
        }
        previous_major = tag;
        major_ids.insert(tag);
        at += 8;
    }
    if major_available && &major_ids != tag_ids {
        return Err(bad("major roster is incomplete"));
    }
    if at != payload.len() {
        return Err(bad(&format!("has {} trailing byte(s)", payload.len() - at)));
    }
    Ok(at)
}

/// Validate the whole stream against the canonical contract, model the d18
/// roster, fold every usable d67 DATA record and collect the symbol vocabulary.
/// Every rejection path is an `Err`, never a silently shorter measurement and
/// never a panic.
fn scan_stream(
    stream: &[u8],
    schema: &Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // The identity protocol is an INPUT, never sniffed
    // (`recorder_host/src/stream_fold.rs:220`, `stream_validator.rs:328`): an `IdentityV1` stream
    // whose mandatory CID1 record has been spliced off is byte-indistinguishable
    // from a legacy stream, so a harness that sniffs measures a decapitated
    // stream production refuses.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D67Hour> = Vec::new();
    let mut vocabulary: BTreeSet<String> = BTreeSet::new();
    let mut counts = ScanCounts::default();
    // COMPLETENESS accounting: every d67 and d18 DATA record of ANY status,
    // against every closed game hour — see the checks after the EOF rules. The
    // d18 lane is also tracked PER HOUR: aggregate counts alone would accept a
    // compensating duplicate (two d18 records in one hour, none in another)
    // whose totals still match (a Codex round-1 finding).
    let mut d67_records = 0usize;
    let mut d18_records = 0usize;
    let mut closed_hours = 0usize;
    let mut d18_covered_hours = 0usize;
    let mut hour_saw_d18 = false;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d67 DATA record of ANY status — the uniqueness rule.
    let mut last_d67_hour: Option<i32> = None;
    // Hour of the last d67 record that entered the measurement.
    let mut last_measured_hour: Option<i32> = None;
    // `WorldFold::tag_by_db_id` plus `country_identity_known_this_hour`
    // (`world_fold.rs:1944`, `:3002`, `:14156`). The roster itself persists
    // across hours; only the KNOWN flag resets, which is exactly what d67 reads.
    let mut roster_this_hour: Option<d67::CountryRoster> = None;

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
                // `world_fold.rs:3002`: the KNOWN flag is per hour.
                roster_this_hour = None;
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D18 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d18_records += 1;
            hour_saw_d18 = true;
            if payload.len() < D18_HEADER_LEN {
                return Err(format!(
                    "d18 DATA at byte {pos}: {} byte(s) is shorter than its header",
                    payload.len()
                ));
            }
            let status_flags =
                u32::from_le_bytes(payload[16..20].try_into().expect("4 bytes"));
            // Validity before classification, the same rule d67 gets below.
            let unknown_status = compact_status_unknown_flags_v1(status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d18 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}",
                    header.game_hour_raw
                ));
            }
            // The payload authority runs on EVERY d18 DATA record whatever its
            // status — `validate_domain_data_payload_at`
            // (`recorder_host/src/stream_fold.rs:593`) is called before the fold
            // and `decode_compact_country_identity_data_payload` never looks at
            // the status word to decide whether to walk the rows. Validating only
            // the non-failure records would accept a stream production refuses
            // (round-2 review finding: a `row_count = 0`, `changed_rows = 1`
            // payload under `COMPACT_STATUS_PARTIAL_DATA`).
            let roster = parse_d18_roster(payload, pos)?;
            // `world_fold.rs:13924`-`:13931`: only the ROSTER UPDATE is
            // status-gated. A failure status leaves the map unchanged and,
            // because the KNOWN flag was reset at the hour boundary, leaves this
            // hour without a roster.
            if compact_status_is_failure(status_flags) {
                counts.d18_failure_records += 1;
            } else {
                roster_this_hour = Some(roster);
            }
        }

        if header.domain_id == D67 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d67_records += 1;
            // One d67 DATA record per game hour, over EVERY d67 record and
            // BEFORE any status-based exclusion — the fold enforces it directly
            // (`world_fold.rs:9844`-`:9847`), and gating it on the record being
            // usable would let a success/failure pair through the rule entirely.
            if let Some(previous) = last_d67_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d67 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d67_hour = Some(header.game_hour_raw);

            // The fold's order: the record flag selects the grammar (`:9680`),
            // the payload decodes in full (`:9848`), the producer authority is
            // checked (`:9850`), and only THEN is the status word consulted
            // (`:9853`). So the decode comes first here too, and a malformed
            // payload is a refused stream whatever its status.
            let schema_v2 = header.record_flags & COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2 != 0;
            let frame = d67::decode_payload(payload, schema_v2)
                .map_err(|e| format!("d67 payload at byte {pos}: {e}"))?;

            // Production's second authority, on every record whatever its
            // status — and the transcription equality check.
            check_authority_payload(&frame, payload, header.record_flags, pos)?;

            // VALIDITY BEFORE CLASSIFICATION (`compact_contract/src/lib.rs:121`-
            // `:123`): bits 15..=31 are unassigned and have no defined
            // classification, so a record carrying one refuses the stream rather
            // than falling through to "not a failure".
            let unknown_status = compact_status_unknown_flags_v1(frame.header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d67 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // `handle_compact_failure` (`world_fold.rs:9853`) retains the
            // accumulator and folds nothing. The predicate is `compact_contract`'s
            // own, not a copied bit mask.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D67,
                frame.header.status_flags,
            ) != 0
            {
                counts.failure_records += 1;
            } else {
                let follows_previous =
                    last_measured_hour.is_some_and(|previous| header.game_hour_raw == previous + 1);
                if last_measured_hour.is_some() && !follows_previous {
                    counts.non_contiguous += 1;
                }
                last_measured_hour = Some(header.game_hour_raw);

                // Pass 1 folds through the IDENTICAL boundary, with a
                // record-local symbol table — see the module docs for why that
                // proves pass 2 can never fail a lookup.
                let local = d67::Symbols::from_vocabulary(
                    frame
                        .instances
                        .iter()
                        .map(|instance| instance.modifier_id.clone()),
                );
                let frame_hour = d67::hour_index_from_raw(header.game_hour_raw);
                let state = d67::fold_payload(
                    schema,
                    &frame,
                    frame_hour,
                    roster_this_hour.as_ref(),
                    &local,
                    0,
                )
                .map_err(|e| format!("d67 payload at byte {pos}: {e}"))?;
                let frame_hour = frame_hour.expect("fold_payload refuses a missing frame hour");
                counts.instances_max = counts.instances_max.max(state.instances.len());
                counts.values_max = counts.values_max.max(state.values.len());

                for instance in &frame.instances {
                    vocabulary.insert(instance.modifier_id.clone());
                }

                // The shipping arm's byte split for this record.
                let instance_rows = frame.instances.len();
                let event_rows = frame.zero_removals.len();
                let instance_id_bytes: usize = frame
                    .instances
                    .iter()
                    .map(|instance| instance.modifier_id.len())
                    .sum();
                let instance_value_bytes: usize = frame
                    .instances
                    .iter()
                    .map(|instance| instance.values_raw.len() * 8)
                    .sum();
                let event_bytes: usize = frame
                    .zero_removals
                    .iter()
                    .map(|event| {
                        d67::ROW_LEN + event.modifier_id.len() + event.values_raw.len() * 8
                    })
                    .sum();
                counts.ship_record_framing += COMPACT_RECORD_HEADER_LEN;
                counts.ship_payload_header += d67::HEADER_LEN;
                counts.ship_instance_rows += instance_rows * d67::ROW_LEN;
                counts.ship_id_blob += instance_id_bytes;
                counts.ship_value_block += instance_value_bytes;
                counts.stopped_zero_removal_rows += event_rows;
                counts.stopped_zero_removal_bytes += event_bytes;

                hours.push(D67Hour {
                    game_hour: header.game_hour_raw,
                    frame_hour,
                    roster: roster_this_hour
                        .clone()
                        .expect("fold_payload refuses a missing roster"),
                    ship_span: pos..record_end,
                    payload_span: payload_start..record_end,
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
        return Err("stream carries no usable d67 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d67 DATA
    // record of any status, and the d18 lane must be all-or-nothing — exactly
    // one record per closed hour when scheduled, none at all when not. This
    // bounds what this harness canNOT validate: production runs
    // `validate_domain_data_payload_at` on EVERY domain's DATA payload
    // (`recorder_host/src/stream_fold.rs:593`), while this harness owns only d67's and d18's
    // grammars, so a record of either domain whose header was relabelled — or
    // lost outright — would otherwise be silently ignored: a lost d67 record
    // shrinks the run, and a d18 record lost in a failure-excluded d67 hour
    // would go unnoticed (a measured hour already refuses on a missing
    // roster). A per-capture completeness assertion, NOT a grammar rule
    // (`bench_real_d65.rs` precedent).
    if d67_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d67_records} d67 DATA \
             record(s); every closed hour of a d67-scheduled capture must carry exactly one, \
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
             silently leave excluded d67 hours without the roster refresh production saw"
        ));
    }

    Ok(StreamScan {
        hours,
        vocabulary,
        counts,
    })
}

/// Re-fold one measured hour through the identical boundary pass 1 used. Pass 1
/// already accepted every one of these records, so a failure here is a bug in
/// this harness, not a property of the stream — hence the panics.
fn fold_hour(schema: &Schema, hour: &D67Hour, payload: &[u8], symbols: &d67::Symbols) -> d67::State {
    let frame = d67::decode_payload(payload, true).expect("pass 1 accepted this payload");
    d67::fold_payload(
        schema,
        &frame,
        Some(hour.frame_hour),
        Some(&hour.roster),
        symbols,
        0,
    )
    .expect("pass 1 folded this frame")
}

/// The number of bytes a varint id occupies (`varint.rs`), for the symbol-id
/// assignment band.
fn varint_len(value: u64) -> usize {
    let mut out = Vec::new();
    wire_format::varint::write_varint(&mut out, value);
    out.len()
}

fn main() {
    const USAGE: &str = "usage: bench_real_d67 <collector_stream.bin> <out dir> \
                         [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    // The identity protocol is stated, never sniffed — see `scan_stream`.
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d67::schema();

    // ---- Pass 1: validate the WHOLE stream, model the roster, fold every d67
    // record and build the symbol vocabulary. No artifact is created before this
    // succeeds. -------------------------------------------------------------
    let scan = match scan_stream(&stream, &schema, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // §5: the table is frozen from the whole content set before any record is
    // mapped, and never grown by the mapper.
    let symbols = d67::Symbols::from_vocabulary(scan.vocabulary.iter().cloned());

    // ---- Pass 2: re-fold the same records through the same code path and
    // encode both arms. -------------------------------------------------------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut prior: Option<d67::State> = None;
    let mut reader: Vec<KeyedState> = vec![KeyedState::new(); d67::COLLECTION_COUNT];
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut rows_total = 0usize;
    // Delta-path churn: changed rows and tombstones over the hours that emitted
    // a delta.
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    // Cadence-independent churn: the same counts over every measured-hour
    // transition, taken BEFORE the block decision, so a change landing on a
    // block boundary still counts (recipe step 9).
    let mut free_changed_total = 0usize;
    let mut free_tombstones_total = 0usize;
    let mut transitions = 0usize;
    // Symbol-id occurrences over the rows this arm actually encodes.
    let mut symbol_occurrences: BTreeMap<u64, usize> = BTreeMap::new();

    for hour in &scan.hours {
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        let state = fold_hour(&schema, hour, &stream[hour.payload_span.clone()], &symbols);
        rows_total += state.rows();

        // Cadence-independent churn, before the block decision.
        if let Some(prev) = &prior {
            transitions += 1;
            for i in 0..d67::COLLECTION_COUNT {
                let d = diff(&schema.collections[i], prev.get(i), state.get(i));
                free_changed_total += d.changed.len();
                free_tombstones_total += d.tombstones.len();
            }
        }

        // The wire-arm hour — baseline decision, encode and reader replay —
        // through the SAME function the tests execute (`encode_wire_hour`).
        let outcome = encode_wire_hour(
            &schema,
            hours_seen,
            hour.follows_previous,
            hour.game_hour,
            prior.as_ref(),
            &state,
            &mut reader,
            &mut wire,
            &mut symbol_occurrences,
        );
        if outcome.gap_forced {
            gap_forced_baselines += 1;
        }
        if outcome.opened_baseline {
            baselines += 1;
            baseline_bytes += outcome.bytes;
        } else if outcome.emitted {
            changed_rows_total += outcome.changed_rows;
            tombstones_total += outcome.tombstones;
            delta_bytes += outcome.bytes;
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        prior = Some(state);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d67.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d67.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    // The symbol-id assignment band. The stand-in is the sorted index; any other
    // assignment over THIS vocabulary permutes which symbols fall past the
    // one-byte varint boundary, so the honest claim is the measured cost plus
    // the extremes over all assignments.
    let stand_in_bytes: usize = symbol_occurrences
        .iter()
        .map(|(id, count)| varint_len(*id) * count)
        .collect::<Vec<_>>()
        .iter()
        .sum();
    let mut occurrences: Vec<usize> = symbol_occurrences.values().copied().collect();
    occurrences.sort_unstable();
    let ids: Vec<u64> = (0..occurrences.len() as u64).collect();
    let lengths: Vec<usize> = ids.iter().map(|id| varint_len(*id)).collect();
    // Cheapest: the most frequent symbols take the shortest ids. Dearest: the
    // reverse. `occurrences` is ascending and `lengths` is non-decreasing.
    let best: usize = occurrences
        .iter()
        .rev()
        .zip(&lengths)
        .map(|(count, len)| count * len)
        .sum();
    let worst: usize = occurrences
        .iter()
        .zip(&lengths)
        .map(|(count, len)| count * len)
        .sum();

    let delta_hours = hours_seen - baselines - empty_hours;
    let ship_len = ship.len();
    let counts = &scan.counts;
    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        counts.non_contiguous
    );
    println!(
        "failure-status hours {} (excluded from both arms); d18 failure records {}",
        counts.failure_records, counts.d18_failure_records
    );
    println!(
        "rows avg             {:.1} over both collections (max {} instances, {} value slots)",
        rows_total as f64 / hours_seen as f64,
        counts.instances_max,
        counts.values_max
    );
    println!("symbols              {} modifier ids", symbols.len());
    println!(
        "baselines            {baselines} ({} at a {BLOCK_HOURS}-h block start, \
         {gap_forced_baselines} forced by a WIRE_FORMAT \u{a7}1.5 state gap)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "churn cadence-free   {:.3} changed rows/hour, {:.3} tombstones/hour over \
         {transitions} measured-hour transitions",
        free_changed_total as f64 / transitions.max(1) as f64,
        free_tombstones_total as f64 / transitions.max(1) as f64
    );
    println!(
        "churn delta-path     {:.2} changed rows/hour, {:.3} tombstones/hour over \
         {delta_hours} emitted-delta hours",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d67.bin         {ship_len} bytes");
    println!(
        "  H4CR framing       {:>9} B ({:.1}%)  — per-record, no per-collection port claims it",
        counts.ship_record_framing,
        100.0 * counts.ship_record_framing as f64 / ship_len as f64
    );
    println!(
        "  DMI2 header        {:>9} B ({:.1}%)  — 104 B/record, incl. the 64-byte authority receipt",
        counts.ship_payload_header,
        100.0 * counts.ship_payload_header as f64 / ship_len as f64
    );
    println!(
        "  instance rows      {:>9} B ({:.1}%)  — claimed by the wire arm",
        counts.ship_instance_rows,
        100.0 * counts.ship_instance_rows as f64 / ship_len as f64
    );
    println!(
        "  modifier id text   {:>9} B ({:.1}%)  — claimed, as §5 symbol ids",
        counts.ship_id_blob,
        100.0 * counts.ship_id_blob as f64 / ship_len as f64
    );
    println!(
        "  value block        {:>9} B ({:.1}%)  — claimed by the values collection",
        counts.ship_value_block,
        100.0 * counts.ship_value_block as f64 / ship_len as f64
    );
    println!(
        "  §9 zero-removals   {:>9} B ({:.1}%)  — STOPPED, claimed by nobody ({} rows)",
        counts.stopped_zero_removal_bytes,
        100.0 * counts.stopped_zero_removal_bytes as f64 / ship_len as f64,
        counts.stopped_zero_removal_rows
    );
    println!(
        "wire_new.bin         {} bytes ({:.2}%)  baseline {} B, delta {} B",
        wire.len(),
        100.0 * wire.len() as f64 / ship_len as f64,
        baseline_bytes,
        delta_bytes
    );
    println!(
        "symbol-id bytes      {stand_in_bytes} under the sorted stand-in; band over all \
         assignments of this {}-symbol vocabulary: {best}..{worst} B ({:.2}%..{:.2}% of the wire \
         arm)",
        symbols.len(),
        100.0 * best as f64 / wire.len() as f64,
        100.0 * worst as f64 / wire.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_STATUS_PARTIAL_DATA};

    fn test_schema() -> Schema {
        d67::schema()
    }

    /// Build one H4CR record the way the recorder frames it
    /// (`compact_contract/src/record_header.rs:60`).
    fn record(
        sequence: u64,
        game_hour: i32,
        domain: u16,
        kind: u16,
        record_flags: u32,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&record_flags.to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn frame_end(sequence: u64, game_hour: i32) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        )
    }

    /// A one-country d18 payload carrying `tag_db_id`, its tag, and an absent
    /// cosmetic marker.
    fn d18_payload(status_flags: u32, tag_db_id: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&1u32.to_le_bytes()); // row_count
        payload.extend_from_slice(&1u32.to_le_bytes()); // changed_rows
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&tag_db_id.to_le_bytes());
        payload.extend_from_slice(&3u16.to_le_bytes());
        payload.extend_from_slice(b"GER");
        payload.push(0); // cosmetic marker: absent
        payload
    }

    /// A one-instance `DMI2` payload, packed the way the shipping encoder packs
    /// one (`compact_contract/src/dynamic_modifiers.rs:313`).
    fn d67_payload(status_flags: u32, owner: u32, modifier: &str, values: &[i64]) -> Vec<u8> {
        use compact_contract::dynamic_modifiers as dmi;
        dmi::encode_payload(&dmi::DynamicModifierFrame {
            elapsed_wall_ms: 0,
            status_flags,
            authority: dmi::DynamicModifierAuthority::exact_a729(),
            instances: vec![dmi::DynamicModifierInstance {
                owner_tag_db_id: owner,
                modifier_id: modifier.to_string(),
                enabled: true,
                values_raw: values.to_vec(),
                remaining_days: None,
            }],
            zero_removals: Vec::new(),
        })
        .expect("encodes")
    }

    const HOUR_A: i32 = 60_832_272;
    const HOUR_B: i32 = 60_832_273;

    /// Two closed hours, each carrying a d18 roster and one d67 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        let mut sequence = 1u64;
        for (hour, value) in [(HOUR_A, 10i64), (HOUR_B, 11)] {
            s.extend(record(
                sequence,
                hour,
                D18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &d18_payload(0, 7),
            ));
            sequence += 1;
            s.extend(record(
                sequence,
                hour,
                D67,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                &d67_payload(0, 7, "alpha", &[value, 20]),
            ));
            sequence += 1;
            s.extend(frame_end(sequence, hour));
            sequence += 1;
        }
        s
    }

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        scan_stream(stream, &test_schema(), StreamIdentityMode::Legacy)
    }

    /// The byte offset of the Nth record boundary.
    fn boundary(stream: &[u8], records: usize) -> usize {
        let mut pos = 0usize;
        for _ in 0..records {
            let h = parse_compact_record_header(&stream[pos..]).expect("framed");
            pos += COMPACT_RECORD_HEADER_LEN + h.payload_len;
        }
        pos
    }

    #[test]
    fn a_well_formed_stream_yields_its_d67_hours() {
        let scan = scan(&good_stream()).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, HOUR_A);
        assert_eq!(scan.hours[1].game_hour, HOUR_B);
        assert_eq!(scan.hours[0].frame_hour, 17_032_273);
        assert_eq!(scan.counts.failure_records, 0);
        assert_eq!(scan.counts.non_contiguous, 0);
        assert_eq!(scan.vocabulary.len(), 1);
        assert!(scan.hours[0].roster.contains(7));
        assert_eq!(scan.counts.stopped_zero_removal_rows, 0);
        // The byte split adds up to the shipping arm exactly.
        let c = &scan.counts;
        let total = c.ship_record_framing
            + c.ship_payload_header
            + c.ship_instance_rows
            + c.ship_id_blob
            + c.ship_value_block
            + c.stopped_zero_removal_bytes;
        let ship: usize = scan.hours.iter().map(|h| h.ship_span.len()).sum();
        assert_eq!(total, ship, "the byte split must partition the shipping arm");
    }

    #[test]
    fn every_transcribed_constant_equals_the_shipping_one() {
        use compact_contract::dynamic_modifiers as dmi;
        assert_eq!(d67::MAGIC, dmi::MAGIC);
        assert_eq!(d67::SCHEMA_VERSION, dmi::SCHEMA_VERSION);
        assert_eq!(d67::HEADER_LEN, dmi::HEADER_LEN);
        assert_eq!(d67::ROW_LEN, dmi::ROW_LEN);
        assert_eq!(d67::VALUE_SCALE, dmi::VALUE_SCALE);
        assert_eq!(d67::MAX_INSTANCES, dmi::MAX_INSTANCES);
        assert_eq!(d67::MAX_VALUES_PER_INSTANCE, dmi::MAX_VALUES_PER_INSTANCE);
        assert_eq!(d67::MAX_MODIFIER_ID_LEN, dmi::MAX_MODIFIER_ID_LEN);
        assert_eq!(d67::AUTHORITY_KIND_EXACT_A729, dmi::AUTHORITY_KIND_EXACT_A729);
        assert_eq!(d67::A729_EXECUTABLE_SHA256, dmi::A729_EXECUTABLE_SHA256);
        assert_eq!(
            d67::STATIC_AUTHORITY_RECEIPT_SHA256,
            dmi::STATIC_AUTHORITY_RECEIPT_SHA256
        );
        assert_eq!(d67::DOMAIN as u16, dmi::DOMAIN_ID);
        assert_eq!(
            d67::MAX_OWNER_TAG_DB_ID,
            compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID
        );
        assert_eq!(
            d67::MAX_PAYLOAD_LEN,
            compact_contract::COMPACT_MAX_PAYLOAD_LEN as usize
        );
        assert_eq!(
            d67::DATE_EPOCH_CONST,
            compact_contract::calendar::DATE_EPOCH_CONST
        );
        // The calendar predicate, across its whole boundary neighbourhood.
        for stored in [
            i32::MIN,
            -1,
            0,
            d67::RAW_DATE_MIN - 2,
            d67::RAW_DATE_MIN - 1,
            d67::RAW_DATE_MIN,
            d67::RAW_DATE_MIN + 1,
            HOUR_A,
            d67::RAW_DATE_MAX - 1,
            d67::RAW_DATE_MAX,
            d67::RAW_DATE_MAX + 1,
            i32::MAX,
        ] {
            assert_eq!(
                d67::hour_index_from_raw(stored),
                compact_contract::calendar::hour_index_from_raw(stored),
                "calendar disagreement at {stored}"
            );
        }
    }

    #[test]
    fn the_two_decoders_agree_on_every_fuzzed_payload() {
        // The transcription is only safe if it accepts and rejects exactly what
        // the shipping codec does. This sweeps random bytes and, far more
        // usefully, single-byte mutations of well-formed payloads — where an
        // off-by-one in a window check would land.
        use compact_contract::dynamic_modifiers as dmi;
        let good = d67_payload(0, 7, "alpha", &[10, -20, 0]);
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut agreed_accept = 0usize;
        for case in 0..20_000 {
            let bytes = if case % 2 == 0 {
                let len = (next() as usize) % 260;
                let mut bytes = vec![0u8; len];
                for byte in &mut bytes {
                    *byte = next() as u8;
                }
                bytes
            } else {
                let mut mutated = good.clone();
                let at = (next() as usize) % mutated.len();
                mutated[at] ^= 1u8 << (next() % 8);
                mutated
            };
            let ours = d67::decode_payload(&bytes, true);
            let theirs = dmi::decode_payload(&bytes);
            match (&ours, &theirs) {
                (Ok(a), Ok(b)) => {
                    assert!(
                        frames_agree(a, b),
                        "case {case}: both accepted, frames differ"
                    );
                    agreed_accept += 1;
                }
                (Err(_), Err(_)) => {}
                _ => panic!(
                    "case {case}: verdict mismatch (ours ok = {}, theirs ok = {}) on {} bytes",
                    ours.is_ok(),
                    theirs.is_ok(),
                    bytes.len()
                ),
            }
        }
        assert!(
            agreed_accept > 0,
            "the sweep must include payloads both decoders accept"
        );
        // The authority hash is the one rule the shipping DECODER enforces and a
        // naive transcription might not: check it explicitly in both directions.
        let mut forged = good.clone();
        forged[40] ^= 0xff;
        assert!(d67::decode_payload(&forged, true).is_err());
        assert!(dmi::decode_payload(&forged).is_err());
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper classifies
        // the v1 vocabulary and is not a validity predicate, so an unassigned bit
        // must be rejected BEFORE classification — on d67's own records and on
        // the d18 records this harness classifies to model the roster.
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            let status = 1u32 << bit;
            for extra in [0u32, COMPACT_STATUS_PARTIAL_DATA] {
                let mut s = Vec::new();
                s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &d18_payload(0, 7)));
                s.extend(record(
                    2,
                    HOUR_A,
                    D67,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                    &d67_payload(status | extra, 7, "alpha", &[1]),
                ));
                s.extend(frame_end(3, HOUR_A));
                assert!(scan(&s).is_err(), "d67 status bit {bit} (+{extra:#x})");

                let mut s = Vec::new();
                s.extend(record(
                    1,
                    HOUR_A,
                    D18,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    0,
                    &d18_payload(status | extra, 7),
                ));
                s.extend(frame_end(2, HOUR_A));
                assert!(scan(&s).is_err(), "d18 status bit {bit} (+{extra:#x})");
            }
        }
        // Informational bits are still accepted: the check must reject
        // unassigned bits and nothing else.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &d18_payload(0, 7)));
            s.extend(record(
                2,
                HOUR_A,
                D67,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                &d67_payload(informational, 7, "alpha", &[1]),
            ));
            s.extend(frame_end(3, HOUR_A));
            let scan = scan(&s).unwrap_or_else(|e| panic!("bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
        }
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." A failure-status d67
        // hour is excluded, so the next measured hour does not follow the
        // previous measured one, must clear `follows_previous`, and
        // `opens_baseline` must open a baseline on it wherever it lands in a
        // block. A delta would relate two states across unobserved time.
        const HOUR_C: i32 = HOUR_B + 1;
        let mut s = Vec::new();
        let mut sequence = 1u64;
        for (hour, status, value) in [
            (HOUR_A, 0u32, 10i64),
            (HOUR_B, COMPACT_STATUS_PARTIAL_DATA, 11), // excluded d67 hour
            (HOUR_C, 0, 12),
        ] {
            s.extend(record(
                sequence,
                hour,
                D18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &d18_payload(0, 7),
            ));
            sequence += 1;
            s.extend(record(
                sequence,
                hour,
                D67,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                &d67_payload(status, 7, "alpha", &[value, 20]),
            ));
            sequence += 1;
            s.extend(frame_end(sequence, hour));
            sequence += 1;
        }
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.hours.len(), 2);
        assert_eq!(scanned.counts.failure_records, 1);
        assert_eq!(scanned.counts.non_contiguous, 1);
        assert!(
            !scanned.hours[0].follows_previous,
            "the first hour follows none"
        );
        assert!(
            !scanned.hours[1].follows_previous,
            "the hour after the excluded one does not follow the first"
        );
        // ...and that is what makes the encoder open a baseline for it, even
        // though it is only the second measured hour of the block — through the
        // PRODUCTION encode path, not just the decision function.
        assert!(opens_baseline(1, scanned.hours[1].follows_previous));
        let schema = test_schema();
        let symbols = d67::Symbols::from_vocabulary(scanned.vocabulary.iter().cloned());
        let state0 = fold_hour(
            &schema,
            &scanned.hours[0],
            &s[scanned.hours[0].payload_span.clone()],
            &symbols,
        );
        let state1 = fold_hour(
            &schema,
            &scanned.hours[1],
            &s[scanned.hours[1].payload_span.clone()],
            &symbols,
        );
        let mut wire = Vec::new();
        let mut reader: Vec<KeyedState> = vec![KeyedState::new(); d67::COLLECTION_COUNT];
        let mut occurrences = BTreeMap::new();
        let o0 = encode_wire_hour(
            &schema,
            0,
            scanned.hours[0].follows_previous,
            scanned.hours[0].game_hour,
            None,
            &state0,
            &mut reader,
            &mut wire,
            &mut occurrences,
        );
        assert!(o0.opened_baseline && !o0.gap_forced);
        let o1 = encode_wire_hour(
            &schema,
            1,
            scanned.hours[1].follows_previous,
            scanned.hours[1].game_hour,
            Some(&state0),
            &state1,
            &mut reader,
            &mut wire,
            &mut occurrences,
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
        let scanned = scan(&good_stream()).expect("well-formed");
        assert_eq!(scanned.counts.non_contiguous, 0);
        assert!(scanned.hours[1].follows_previous);
        assert!(!opens_baseline(1, scanned.hours[1].follows_previous));
    }

    #[test]
    fn a_closed_hour_without_a_d67_record_refuses_the_stream() {
        // The cross-domain bound: hour B carries only the d18 record and
        // another domain's DATA record — structurally well formed, so only the
        // completeness check can see the missing d67 record.
        let mut s = Vec::new();
        s.extend(record(
            1,
            HOUR_A,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &d18_payload(0, 7),
        ));
        s.extend(record(
            2,
            HOUR_A,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(0, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(3, HOUR_A));
        s.extend(record(
            4,
            HOUR_B,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &d18_payload(0, 7),
        ));
        s.extend(record(5, HOUR_B, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &[0u8; 8]));
        s.extend(frame_end(6, HOUR_B));
        let err = match scan(&s) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d67 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );

        // And a partially-missing d18 lane is refused too: hour B carries a
        // failure-status d67 record (so the missing roster is not caught by the
        // fold) and no d18 record at all.
        let mut s = Vec::new();
        s.extend(record(
            1,
            HOUR_A,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &d18_payload(0, 7),
        ));
        s.extend(record(
            2,
            HOUR_A,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(0, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(3, HOUR_A));
        s.extend(record(
            4,
            HOUR_B,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(COMPACT_STATUS_PARTIAL_DATA, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(5, HOUR_B));
        let err = match scan(&s) {
            Err(err) => err,
            Ok(_) => panic!("a partially-missing d18 lane must refuse the stream"),
        };
        assert!(err.contains("d18"), "the refusal names the d18 bound: {err}");

        // A COMPENSATING duplicate — two d18 records in hour A, none in hour B
        // — leaves the aggregate count equal to the closed hours and is caught
        // only by the per-hour coverage check (Codex round-1 finding). Hour B's
        // d67 record is a failure-status one, so the missing roster is not
        // caught by the fold either.
        let mut s = Vec::new();
        s.extend(record(
            1,
            HOUR_A,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &d18_payload(0, 7),
        ));
        s.extend(record(
            2,
            HOUR_A,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &d18_payload(0, 7),
        ));
        s.extend(record(
            3,
            HOUR_A,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(0, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(4, HOUR_A));
        s.extend(record(
            5,
            HOUR_B,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(COMPACT_STATUS_PARTIAL_DATA, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(6, HOUR_B));
        let err = match scan(&s) {
            Err(err) => err,
            Ok(_) => panic!("a compensating d18 duplicate must refuse the stream"),
        };
        assert!(err.contains("covering"), "the refusal shows the coverage: {err}");
    }

    #[test]
    fn a_failure_status_hour_is_excluded_not_refused() {
        let mut s = Vec::new();
        s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &d18_payload(0, 7)));
        s.extend(record(
            2,
            HOUR_A,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(COMPACT_STATUS_PARTIAL_DATA, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(3, HOUR_A));
        // One hour, and it is excluded — not a stream rejection, and not a
        // measured hour either.
        let mut full = s.clone();
        let mut sequence = 4u64;
        full.extend(record(sequence, HOUR_B, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &d18_payload(0, 7)));
        sequence += 1;
        full.extend(record(
            sequence,
            HOUR_B,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(0, 7, "alpha", &[1]),
        ));
        sequence += 1;
        full.extend(frame_end(sequence, HOUR_B));
        let scan = scan(&full).expect("accepted");
        assert_eq!(scan.hours.len(), 1, "the failure hour carries no data");
        assert_eq!(scan.counts.failure_records, 1);
        assert_eq!(scan.hours[0].game_hour, HOUR_B);
    }

    #[test]
    fn a_d67_record_without_a_current_hour_roster_refuses_the_stream() {
        // `world_fold.rs:9869`-`:9874`. Three ways the roster can be missing,
        // and all three are stream rejections rather than skipped hours.
        //
        // 1. No d18 record at all this hour.
        let mut s = Vec::new();
        s.extend(record(
            1,
            HOUR_A,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(0, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(2, HOUR_A));
        let err = scan(&s).expect_err("refused");
        assert!(err.contains("country_identity roster"), "{err}");

        // 2. A d18 record whose status marks it failed, which leaves the map
        //    unrefreshed and the hour without a roster.
        let mut s = Vec::new();
        s.extend(record(
            1,
            HOUR_A,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &d18_payload(COMPACT_STATUS_PARTIAL_DATA, 7),
        ));
        s.extend(record(
            2,
            HOUR_A,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(0, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(3, HOUR_A));
        assert!(scan(&s).is_err());

        // 3. A roster that does not carry the instance's owner (`:9878`).
        let mut s = Vec::new();
        s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &d18_payload(0, 9)));
        s.extend(record(
            2,
            HOUR_A,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(0, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(3, HOUR_A));
        let err = scan(&s).expect_err("refused");
        assert!(err.contains("no current country_identity mapping"), "{err}");

        // And the roster is per hour: last hour's d18 does not license this
        // hour's d67 (`world_fold.rs:3002`).
        let mut s = Vec::new();
        s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &d18_payload(0, 7)));
        s.extend(frame_end(2, HOUR_A));
        s.extend(record(
            3,
            HOUR_B,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(0, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(4, HOUR_B));
        let err = scan(&s).expect_err("refused");
        assert!(err.contains("country_identity roster"), "{err}");
    }

    #[test]
    fn a_legacy_v1_record_refuses_the_stream() {
        // The record flag is the discriminator (`world_fold.rs:9680`). A cleared
        // flag selects a grammar this port does not implement, so the run is
        // invalid rather than silently short.
        let mut s = Vec::new();
        s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &d18_payload(0, 7)));
        s.extend(record(
            2,
            HOUR_A,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &d67_payload(0, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(3, HOUR_A));
        let err = scan(&s).expect_err("refused");
        assert!(err.contains("legacy v1"), "{err}");
    }

    #[test]
    fn a_second_d67_record_in_one_hour_refuses_the_stream() {
        // `world_fold.rs:9844`-`:9847`, enforced over EVERY d67 record and before
        // any status exclusion, so a success/failure pair cannot slip through.
        for (first_status, second_status) in [(0u32, 0u32), (COMPACT_STATUS_PARTIAL_DATA, 0), (0, COMPACT_STATUS_PARTIAL_DATA)] {
            let mut s = Vec::new();
            s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &d18_payload(0, 7)));
            s.extend(record(
                2,
                HOUR_A,
                D67,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                &d67_payload(first_status, 7, "alpha", &[1]),
            ));
            s.extend(record(
                3,
                HOUR_A,
                D67,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                &d67_payload(second_status, 7, "beta", &[1]),
            ));
            s.extend(frame_end(4, HOUR_A));
            assert!(
                scan(&s).is_err(),
                "two d67 records in one hour ({first_status:#x}, {second_status:#x})"
            );
        }
    }

    #[test]
    fn a_zero_removal_event_stops_the_run_rather_than_being_dropped() {
        use compact_contract::dynamic_modifiers as dmi;
        let payload = dmi::encode_payload(&dmi::DynamicModifierFrame {
            elapsed_wall_ms: 0,
            status_flags: 0,
            authority: dmi::DynamicModifierAuthority::exact_a729(),
            instances: vec![dmi::DynamicModifierInstance {
                owner_tag_db_id: 7,
                modifier_id: "alpha".into(),
                enabled: true,
                values_raw: vec![1],
                remaining_days: None,
            }],
            zero_removals: vec![dmi::DynamicModifierZeroRemoval {
                owner_tag_db_id: 7,
                modifier_id: "beta".into(),
                enabled: true,
                values_raw: vec![0],
                observed_game_hour_raw: HOUR_A,
            }],
        })
        .expect("encodes");
        let mut s = Vec::new();
        s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &d18_payload(0, 7)));
        s.extend(record(
            2,
            HOUR_A,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &payload,
        ));
        s.extend(frame_end(3, HOUR_A));
        let err = scan(&s).expect_err("stop-reported");
        assert!(err.contains("zero-removal"), "{err}");
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        // Records 1, 2, 4 and 5 are the DATA records: a cut after one of them
        // leaves the hour open and ends the stream on a non-FRAME_END.
        for records in [1usize, 2, 4, 5] {
            let cut = boundary(&full, records);
            assert!(
                scan(&full[..cut]).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream.
        for (records, expected_hours) in [(3usize, 1usize), (6, 2)] {
            let cut = boundary(&full, records);
            let scan = scan(&full[..cut]).expect("closed stream is well-formed");
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
            assert!(scan(&full[..cut]).is_err(), "mid-record cut at {cut}");
        }
        assert!(scan(&[]).is_err(), "an empty stream is not a measurement");
    }

    #[test]
    fn a_header_the_canonical_parser_rejects_is_rejected_here() {
        for records in 0..6 {
            let at = boundary(&good_stream(), records);
            for (offset, patch) in [
                (0usize, vec![b'X']),
                (4, 2u16.to_le_bytes().to_vec()),
                (6, 33u16.to_le_bytes().to_vec()),
            ] {
                let mut s = good_stream();
                s[at + offset..at + offset + patch.len()].copy_from_slice(&patch);
                assert!(
                    scan(&s).is_err(),
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
                assert!(
                    scan(&s).is_err(),
                    "record {records} sequence {wrong}"
                );
            }
        }
    }

    #[test]
    fn a_corrupt_d18_roster_refuses_the_stream() {
        // The roster is an input to d67's admission, so a payload this harness
        // cannot walk exactly is a refused stream, never a guessed roster.
        let good = d18_payload(0, 7);
        for len in 0..good.len() {
            let mut s = Vec::new();
            s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &good[..len]));
            s.extend(frame_end(2, HOUR_A));
            assert!(scan(&s).is_err(), "a {len}-byte d18 payload must be refused");
        }
        // A trailing region that is not a CIM1 extension.
        let mut trailing = good.clone();
        trailing.extend_from_slice(b"XXXX");
        let mut s = Vec::new();
        s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &trailing));
        s.extend(frame_end(2, HOUR_A));
        assert!(scan(&s).is_err());
        // A zero tag_db_id, and an invalid cosmetic marker.
        let mut zero_id = good.clone();
        zero_id[20..24].copy_from_slice(&0u32.to_le_bytes());
        let mut bad_marker = good.clone();
        let last = bad_marker.len() - 1;
        bad_marker[last] = 3;
        for payload in [zero_id, bad_marker] {
            let mut s = Vec::new();
            s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &payload));
            s.extend(frame_end(2, HOUR_A));
            assert!(scan(&s).is_err());
        }
    }

    #[test]
    fn the_d18_payload_is_validated_under_productions_own_authority() {
        // Round-1 review findings, swept over the property rather than the
        // reported input: the roster parse must (a) never allocate on a declared
        // count, and (b) refuse exactly what
        // `decode_compact_country_identity_data_payload` refuses.
        let good = d18_payload(0, 7);

        // (a) A 20-byte payload declaring u32::MAX rows. Before the fix this
        // reserved ~16 GiB; it must be a plain Err, and every other count word
        // must be equally harmless.
        for (offset, word) in [(8usize, u32::MAX), (12, u32::MAX)] {
            let mut header_only = good[..D18_HEADER_LEN].to_vec();
            header_only[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
            assert!(parse_d18_roster(&header_only, 0).is_err(), "count at +{offset}");
        }

        // (b) `changed_rows > row_count` (`domain_data_authority.rs:1029`) — the
        // fold never reads that word, so only the second authority catches it.
        let mut changed = good.clone();
        changed[12..16].copy_from_slice(&2u32.to_le_bytes());
        assert!(parse_d18_roster(&changed, 0).is_err());
        let mut s = Vec::new();
        s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &changed));
        s.extend(frame_end(2, HOUR_A));
        assert!(scan(&s).is_err(), "a stream production refuses must not measure here");

        // The same rules apply to a FAILURE-status d18 record: production's
        // payload authority runs before the fold and is status-independent
        // (`recorder_host/src/stream_fold.rs:593`), so validating only the records that refresh the
        // roster would accept a stream the recorder refuses (round-2 review
        // finding, with the reviewer's own input).
        for status in [0u32, COMPACT_STATUS_PARTIAL_DATA] {
            let mut rowless = good[..D18_HEADER_LEN].to_vec();
            rowless[8..12].copy_from_slice(&0u32.to_le_bytes()); // row_count
            rowless[12..16].copy_from_slice(&1u32.to_le_bytes()); // changed_rows
            rowless[16..20].copy_from_slice(&status.to_le_bytes());
            let mut s = Vec::new();
            s.extend(record(1, HOUR_A, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &rowless));
            s.extend(frame_end(2, HOUR_A));
            // Followed by a healthy hour, so the stream would otherwise measure.
            s.extend(record(3, HOUR_B, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &good));
            s.extend(record(
                4,
                HOUR_B,
                D67,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                &d67_payload(0, 7, "alpha", &[1]),
            ));
            s.extend(frame_end(5, HOUR_B));
            assert!(
                scan(&s).is_err(),
                "a malformed d18 payload under status {status:#x} must refuse the stream"
            );
        }
        // And a well-formed failure-status d18 record is still only excluded from
        // refreshing the roster, not a rejection. Hour A carries a
        // failure-status d67 record too, so the stream stays complete — every
        // closed hour carries exactly one record of each modelled domain.
        let mut s = Vec::new();
        s.extend(record(
            1,
            HOUR_A,
            D18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &d18_payload(COMPACT_STATUS_PARTIAL_DATA, 7),
        ));
        s.extend(record(
            2,
            HOUR_A,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(COMPACT_STATUS_PARTIAL_DATA, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(3, HOUR_A));
        s.extend(record(4, HOUR_B, D18, COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &good));
        s.extend(record(
            5,
            HOUR_B,
            D67,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &d67_payload(0, 7, "alpha", &[1]),
        ));
        s.extend(frame_end(6, HOUR_B));
        let scanned = scan(&s).expect("a clean failure-status d18 record is not a rejection");
        assert_eq!(scanned.counts.d18_failure_records, 1);
        assert_eq!(scanned.hours.len(), 1);

        // A well-formed CIM1 tail is accepted, and every one-word corruption of
        // it is refused.
        let cim1 = {
            let mut p = good.clone();
            p.extend_from_slice(b"CIM1");
            p.extend_from_slice(&1u16.to_le_bytes()); // version
            p.extend_from_slice(&40u16.to_le_bytes()); // header len
            p.extend_from_slice(&0u32.to_le_bytes()); // claims status
            p.extend_from_slice(&0u32.to_le_bytes()); // cores status
            p.extend_from_slice(&0u32.to_le_bytes()); // major status
            p.extend_from_slice(&1u32.to_le_bytes()); // state rows
            p.extend_from_slice(&1u32.to_le_bytes()); // expected claims
            p.extend_from_slice(&0u32.to_le_bytes()); // expected cores
            p.extend_from_slice(&1u32.to_le_bytes()); // majors (== roster size)
            p.extend_from_slice(&0u32.to_le_bytes()); // reserved
            p.extend_from_slice(&5u32.to_le_bytes()); // state_id
            p.extend_from_slice(&1u32.to_le_bytes()); // claim count
            p.extend_from_slice(&0u32.to_le_bytes()); // core count
            p.extend_from_slice(&7u32.to_le_bytes()); // claim member
            p.extend_from_slice(&7u32.to_le_bytes()); // major tag
            p.extend_from_slice(&[1, 0, 0, 0]); // marker + reserved
            p
        };
        let roster = parse_d18_roster(&cim1, 0).expect("a well-formed CIM1 tail is accepted");
        assert!(roster.contains(7));
        let tail = good.len();
        for (offset, patch, what) in [
            (tail, b"CIM2".to_vec(), "magic"),
            (tail + 4, 2u16.to_le_bytes().to_vec(), "version"),
            (tail + 6, 41u16.to_le_bytes().to_vec(), "header length"),
            (tail + 36, 1u32.to_le_bytes().to_vec(), "reserved"),
            (tail + 20, 2u32.to_le_bytes().to_vec(), "state count"),
            (tail + 24, 2u32.to_le_bytes().to_vec(), "expected claims"),
            (tail + 32, 0u32.to_le_bytes().to_vec(), "major count"),
            (tail + 40, 0u32.to_le_bytes().to_vec(), "state id"),
            (tail + 52, 9u32.to_le_bytes().to_vec(), "claim member not in the roster"),
            (tail + 56, 9u32.to_le_bytes().to_vec(), "major tag not in the roster"),
            (tail + 60, vec![2u8], "major marker"),
            (tail + 61, vec![1u8], "major row reserved"),
        ] {
            let mut bad = cim1.clone();
            bad[offset..offset + patch.len()].copy_from_slice(&patch);
            assert!(
                parse_d18_roster(&bad, 0).is_err(),
                "a patched CIM1 {what} must be refused"
            );
        }
        // Every truncation of the extension, and a trailing byte. (A cut at
        // exactly `good.len()` is the unextended payload, which is legal.)
        for len in good.len() + 1..cim1.len() {
            assert!(parse_d18_roster(&cim1[..len], 0).is_err(), "CIM1 cut at {len}");
        }
        let mut trailing = cim1.clone();
        trailing.push(0);
        assert!(parse_d18_roster(&trailing, 0).is_err());
    }

    #[test]
    fn the_d18_roster_parser_never_panics_on_arbitrary_bytes() {
        let good = d18_payload(0, 7);
        let mut state = 0x1234_5678_9ABC_DEF1u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let len = (next() as usize) % 120;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                *byte = next() as u8;
            }
            let _ = parse_d18_roster(&bytes, 0);
            let mut mutated = good.clone();
            let at = (next() as usize) % mutated.len();
            mutated[at] = next() as u8;
            let _ = parse_d18_roster(&mutated, 0);
        }
    }

    #[test]
    fn the_frame_end_seal_is_enforced() {
        let full = good_stream();
        let at = boundary(&full, 2); // the first FRAME_END
        // reserved != 0, emitted != scheduled, an unknown status bit, a fatal
        // status bit — each a refused stream in production (`:402`-`:409`).
        for (offset, patch) in [
            (COMPACT_RECORD_HEADER_LEN + 28, 1u32.to_le_bytes().to_vec()),
            (COMPACT_RECORD_HEADER_LEN + 16, 1u32.to_le_bytes().to_vec()),
            (COMPACT_RECORD_HEADER_LEN + 24, (1u32 << 16).to_le_bytes().to_vec()),
            (
                COMPACT_RECORD_HEADER_LEN + 24,
                COMPACT_STATUS_PARTIAL_DATA.to_le_bytes().to_vec(),
            ),
        ] {
            let mut s = full.clone();
            s[at + offset..at + offset + patch.len()].copy_from_slice(&patch);
            assert!(scan(&s).is_err(), "seal patched at +{offset}");
        }
    }
}
