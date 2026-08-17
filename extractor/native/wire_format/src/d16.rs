//! d16 `decisions_live` — per-country decision-status counters.
//!
//! Derived from `world_fold.rs:19023`-`:19081`
//! (`fold_decisions`), the collector write that produces the payload
// @world-fold-authority domain=16 functions=fold_decisions lines=19023-19081 sha256=be53b53fb85f6abfa59eca3721f7d8b65ef511ef04d038a280f0379815af269c closure=28182abd9cdaf1a5d82d9f8a07bb2f340e0fb9b76090ab2d2126371df84d5a06
//! (`extractor/native/collector/src/attach_log.rs:69581`-`:69648`), and the read
//! that produces the counters (`attach_log.rs:83141`-`:83235`).
//! `extractor/documents/domains/d16_decisions_live.md` carries real field-level
//! knowledge and is used below, with one recorded disagreement (see
//! "The dossier's preservation claim is false on this path").
//!
//! **This domain is a SPLIT deliverable.** The `countries` collection is ported
//! here. The payload's second table — the `timed_live` rows, an ordered
//! occurrence list carrying a string-valued `decision` token — is a
//! **stop condition** and is parsed for validation and byte accounting only, never
//! mapped onto a `Record`. See "Why `timed_live` stops" below.
//!
//! ## Payload grammar (`world_fold.rs::fold_decisions`, written at `attach_log.rs:69593`-`:69647`)
//!
//! A 24-byte header:
//!
//! | offset | width | field | written |
//! |---|---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms` | `attach_log.rs:69594` |
//! | `+8`  | u32 | `country_count`   | `:69595`, read `world_fold.rs::fold_decisions` |
//! | `+12` | u32 | `changed_rows`    | `:69596` — the SAME word as `country_count` |
//! | `+16` | u32 | `status_flags`    | `:69597` — **never read by the fold** |
//! | `+20` | u32 | `timed_count`     | `:69598`, read `world_fold.rs::fold_decisions` |
//!
//! then `country_count` fixed 32-byte country rows, then `timed_count`
//! variable-length timed rows. The fold requires the cursor to land exactly on
//! `payload.len()` (`world_fold.rs::fold_decisions`), so the two tables together account for every byte.
//!
//! ### Country row — 32 bytes, of which the fold reads 20
//!
//! | offset | width | fold reads it? | field | scale | gate |
//! |---|---|---|---|---|---|
//! | `+0`  | u32 | yes (`world_fold.rs::fold_decisions`) | `tag_db_id`                | raw | none (key) |
//! | `+4`  | u32 | reads and **discards** (`world_fold.rs::fold_decisions`, binding `_flags`) | `flags` | — | — |
//! | `+8`  | u32 | yes (`world_fold.rs::fold_decisions`) | `active_timed`             | raw | none |
//! | `+12` | u32 | yes (`world_fold.rs::fold_decisions`) | `active_targeted`          | raw | none |
//! | `+16` | u32 | yes (`world_fold.rs::fold_decisions`) | `active_targeted_timed`    | raw | none |
//! | `+20` | u32 | **no**         | `decisions_taken_bucket`   | raw | — |
//! | `+24` | u32 | **no**         | `decision_to_re_enable`    | raw | — |
//! | `+28` | u32 | **no**         | `timed_row_count`          | raw | — |
//!
//! **No scales anywhere.** Every word is a raw vector size the collector read out
//! of `CDecisionStatus` (`attach_log.rs:83209`-`:83212`) and the fold emits each
//! one unmodified (`world_fold.rs:13819`-`:13821`). There is no `FIXED_SCALE`, no
//! `/32768`, and no field this domain divides — the 32768-vs-100000 failure class
//! cannot arise, and the offset test in the test module pins that as a claim
//! rather than an assumption.
//!
//! The last three columns are the "12 unread bytes". They are **not** ported:
//! the recipe's precedence rule makes `fold_decisions` the authority on what this
//! domain carries, and it carries what the fold folds. Their names and meanings
//! are recorded above (recovered from the writer at `attach_log.rs:69610`-`:69634`)
//! so the blind spot is documented rather than merely observed, and they are
//! listed in the deliverable's open questions.
//!
//! ## The `flags` word at `+4` — semantics recovered, deliberately not ported
//!
//! `attach_log.rs:69601`-`:69607` writes exactly two bits:
//!
//! - `1<<0` — `CountryDecisionsRow::status_present.is_some()` (`:69602`)
//! - `1<<1` — `CountryDecisionsRow::timed_live.is_some()` (`:69605`)
//!
//! The fold binds the word to `_flags` and drops it (`world_fold.rs::fold_decisions`), so
//! under the recipe's precedence rule neither bit is part of this domain's data.
//! Both bits are recorded here because one of them carries information nothing
//! else on the wire does — see the next two sections — and both are open
//! questions in the deliverable, not silently discarded facts.
//!
//! ## Four-state semantics: every ported field is a plain value, and zero is real
//!
//! §3's four states are decided per field. All three counters land in the same
//! place, and the reasoning is unusual enough to be worth stating in full.
//!
//! **The fold gates nothing.** Unlike `fold_country_core_scalars`, `fold_decisions`
//! reads all three counters unconditionally (`world_fold.rs::fold_decisions`) and
//! emits every one as a JSON number. It never emits `Null` for them, under any
//! input. Step 2's first authority therefore says "value", always.
//!
//! **The collector conflates a real zero with a failed read, and the payload
//! cannot separate them.** `decision_status_present` (`attach_log.rs:83193`) reads
//! six vector sizes and inserts a key only when `decision_vector_size` returned
//! `Some(size)` **and** `size > 0` (`:83209`-`:83212`). `decision_vector_size`
//! returns `None` on unreadable memory, a `size > cap` inversion, a capacity past
//! `MAX_DECISION_STATUS_LIST`, or a data pointer that is not a heap object
//! (`:83222`-`:83234`) — but it returns `Some(0)` for a genuinely empty vector
//! (`:83227`), which the `size > 0` test then drops just the same. The writer
//! then packs `.unwrap_or(0)` for every missing key (`:69621`-`:69622`). So a
//! packed `0` means "this vector was empty" **or** "this vector could not be read",
//! and no bit anywhere in the row distinguishes them: `flags` bit `1<<0` is the
//! OR over all six keys, so it is set whenever any one of the six was readable and
//! non-zero, which says nothing about this particular counter.
//!
//! That ambiguity is real and it is **upstream of this boundary**. The recipe's
//! scope puts collector changes out of bounds, and §3's own rule is explicit that
//! **zero** is "a real measured value, never collapsed" into an empty state.
//! Mapping `0` to `Unknown` would assert "not captured" over the 58.50% of counter
//! cells that read zero in the reference capture (3,954 of 6,759) — the
//! overwhelming majority of which are countries that genuinely have no active
//! decisions of that kind — and would destroy a real signal to express a doubt.
//! So all three fields are plain values with **no declared capability**: no
//! `.optional()`, no `.nullable()`, no `.unknown_capable()`. The mapper can emit
//! exactly one thing, a value, which is exactly what the schema declares.
//!
//! The tie-break rule ("choose Unknown") does not apply: it arbitrates Null vs
//! Unknown for a *gated* field, and nothing here is gated. The conflation is
//! recorded as the deliverable's first open question, with the evidence that
//! would settle it (a collector change that packs the six sizes as
//! `Option<u32>`-shaped presence bits, which is §13 work, not schema work).
//!
//! ## The dossier's preservation claim is false on this path
//!
//! `domains/d16_decisions_live.md` states: *"Fold semantics preserve measured-empty
//! `timed_live=[]` separately from an unmeasured or failed country."* On the
//! compact path that is not what happens. `fold_decisions` writes
//! `obj.insert("timed_live", json!([]))` for **every** country row
//! (`world_fold.rs::fold_decisions`) before any timed row is appended, and the only bit that
//! could tell the two apart — `flags` `1<<1`, written from
//! `timed_live.is_some()` at `attach_log.rs:69605` — is the word the fold discards
//! at `:13810`. A country whose timed vector was unreadable and a country whose
//! timed vector was empty fold to the identical `"timed_live": []`.
//!
//! This is not hypothetical: the reference capture
//! (`runs/batch1/deepwar_t1/collector_stream.bin`) carries 2,253 country rows with
//! exactly two distinct flag words, `3` (2,252 rows) and `1` (1 row). That one row
//! is a country whose timed vector could not be read, and the fold recorded it as
//! measured-empty. The recipe's rule is that the fold wins and the disagreement is
//! reported, so it is reported: the dossier describes an intent the compact
//! payload's fold does not implement. This costs the ported collection nothing —
//! `timed_live` is not in it — and it is an open question for the stop-report.
//!
//! ## Why `timed_live` stops
//!
//! The second table's rows are 12 fixed bytes (`tag_db_id`, `days` i32, `state`
//! i32) plus a length-prefixed UTF-8 `decision` token
//! (`world_fold.rs::fold_decisions`, written at `attach_log.rs:69654`-`:69659`).
//! **The shape blocker is closed; the token blocker remains fatal.**
//!
//! *Shape — settled, not open.* The fold `push`es each row onto the country's
//! array (`world_fold.rs::fold_decisions`). It does not upsert, does not dedup and
//! does not sort, so a
//! payload with one country row for tag 7 and two identical timed rows
//! `(7, 30, 0, "x")` folds to two entries in source order, and any shape with a
//! key would collapse them into one. That is §7.3's third shape word for word —
//! "meaningful physical order, duplicates legitimate, no unique key" — beside d56
//! and d64, and §7.4 states outright that ordered occurrence lists have **no key**,
//! so no key question arises for this table at all. `Shape` declares the variant
//! (`Shape::OrderedOccurrenceList`, `schema.rs:163`), and `body.rs` now implements
//! its full-replacement baseline and delta grammar. The former shape blocker is
//! therefore closed.
//!
//! *The token is what still stops the collection.* Every shape must carry `decision`
//! as a value, and §4 has nothing to carry a string in: `Kind` is
//! `Unsigned | Signed | Monotone | Enum | Boolean | ContentId | Continuous`
//! (`schema.rs:12`-`:20`). Dropping it is data loss — it is the row's only
//! distinguishing content, since `(tag, days, state)` repeats across different
//! decisions — and hashing it into a `ContentId` would invent the manifest-id
//! id unless this mapper receives the file-global §5 resolver. Until that
//! resolver is wired here, the collection remains stopped regardless of its implemented
//! shape.
//!
//! The full stop-report is the deliverable's, not this file's; this module parses
//! the table because the fold's trailing-byte rule
//! (`world_fold.rs::fold_decisions`) makes the whole
//! payload invalid if it is malformed, and because the deliverable reports its
//! byte share.
//!
//! ## Shape: upsert, never replace
//!
//! `fold_decisions` commits with `self.country_mut(tag).insert("decisions", ..)`
//! per tag (`:13847`-`:13850`). A country that is **absent** from a payload keeps
//! whatever it was folded last — the fold clears nothing. That makes one d16
//! record smaller than one d16 state, so [`Accumulator`] is the fourth public
//! boundary the recipe calls for, and it is what the measurement harness folds
//! through. The reference capture makes the distinction load-bearing rather than
//! theoretical: its first record carries all 356 countries and every later record
//! carries 0–6, so a per-record reading of "the state" would be wrong in 902 of
//! 903 hours.
//!
//! **No derived field (§6).** A decision countdown is not carried here at all (it
//! lives in the stopped `timed_live` table), and the three counters move on player
//! and AI action, not on the clock. The mapper still takes `frame_tick` so every
//! domain's boundary has one shape.
//!
//! ## Mirroring the fold's permissiveness
//!
//! `fold_decisions` is at the permissive end of the fold family. It does **not**
//! reject a zero `tag_db_id` (contrast `world_fold.rs:9279`), does **not** reject
//! undefined `flags` bits (contrast `:9283`), applies **no** range check to any
//! counter, does **not** read `status_flags` at all, does **not** check
//! `changed_rows` against `country_count`, and treats a `tag_db_id` repeated
//! inside one frame as last-writer-wins rather than a rejected frame
//! (`per_tag.insert`, `world_fold.rs::fold_decisions`). Every one of those is mirrored here rather than
//! tightened, because tightening would reject payloads the recorder folds — the
//! data-loss direction — and each is pinned by a test.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 16;
/// The ported collection. There is deliberately no collection id for the stopped
/// `timed_live` table — not because its shape is undecided (the fold settles it as
/// `Shape::OrderedOccurrenceList`; see "Why `timed_live` stops") but because that
/// shape has no implementation to declare a collection against: every encoder and
/// decoder in `body.rs` hard-rejects a non-`KeyedSet` shape, so a collection id
/// here would name something nothing can encode. Field ids are append-only (§11)
/// and collection ids follow the same discipline, so the id is left unclaimed
/// until §7.3 lands rather than spent on a collection that cannot be written.
pub const COUNTRIES: u32 = 1;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
pub mod field {
    pub const TAG_DB_ID: u32 = 1;
    pub const ACTIVE_TIMED: u32 = 2;
    pub const ACTIVE_TARGETED: u32 = 3;
    pub const ACTIVE_TARGETED_TIMED: u32 = 4;
}

// --- Payload grammar, from `world_fold.rs::fold_decisions`. --------------------

/// `world_fold.rs::fold_decisions` — `need(payload, 24, "decisions header")`.
pub const HEADER_LEN: usize = 24;
/// `world_fold.rs::fold_decisions` — `need(payload, cursor + 32)` then `cursor += 32`.
pub const COUNTRY_ROW_LEN: usize = 32;
/// The fixed part of a timed row, before its length-prefixed string
/// (`world_fold.rs::fold_decisions`).
pub const TIMED_ROW_FIXED_LEN: usize = 12;
/// The `u16` length prefix `read_string` consumes (`world_fold.rs::read_string`).
pub const STRING_LEN_PREFIX: usize = 2;

/// `attach_log.rs:69602`-`:69604` — the country's decision-status counters were
/// readable and at least one was non-zero. Read into `_flags` and discarded by the
/// fold (`world_fold.rs::fold_decisions`); recorded, not ported. See the module docs.
pub const FLAG_STATUS_PRESENT: u32 = 1 << 0;
/// `attach_log.rs:69605`-`:69607` — the country's active-timed vector was read
/// successfully (possibly to an empty list). Discarded by the fold; this is the
/// bit whose loss makes the dossier's preservation claim false.
pub const FLAG_TIMED_LIVE_READ: u32 = 1 << 1;

/// Byte offsets of the three words the fold reads past the key
/// (`world_fold.rs::fold_decisions`), in row order. Used by the mapper and by the
/// offset test, so a transposition cannot pass one and fail the other.
pub const COUNTER_OFFSETS: [(u32, usize); 3] = [
    (field::ACTIVE_TIMED, 8),
    (field::ACTIVE_TARGETED, 12),
    (field::ACTIVE_TARGETED_TIMED, 16),
];

/// The three words at `+20`, `+24` and `+28` that `fold_decisions` never reads,
/// with the names the writer gives them (`attach_log.rs:69614`-`:69634`). Present
/// as documentation and as the subject of a test that pins the mapper's
/// indifference to them; deliberately not mapped onto any field.
pub const UNREAD_TAIL_OFFSETS: [(&str, usize); 3] = [
    ("decisions_taken_bucket", 20),
    ("decision_to_re_enable", 24),
    ("timed_row_count", 28),
];

pub fn schema() -> Schema {
    use field as f;
    let fields = vec![
        // Key: the country's `tag_db_id` (`world_fold.rs::fold_decisions`). ContentId
        // because a country tag is content-defined (§5) — a manifest must resolve
        // it for it to keep its meaning under another mod set, which is the
        // deciding question for ContentId-vs-Unsigned. d63's `tag_db_id` and
        // d20's `owner` are classed the same way. This is a ContentId **key**,
        // the heavier §13 retrofit class: key components sort and delta-encode by
        // raw value, so a manifest remap would reorder keys and change the byte
        // layout of every baseline. Flagged separately in the open questions.
        Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
        // Sizes of live decision vectors: they rise when a decision is activated
        // and fall when one completes or is cancelled, so `Unsigned`, not
        // `Monotone` — §4 makes Monotone a hint for counters that *usually* rise,
        // and these do not. Not `ContentId`: a vector length is not an id and no
        // manifest resolves it. Not `Continuous`: the fold emits each as an
        // integer with no divisor (`world_fold.rs::fold_decisions`).
        //
        // No capability is declared on any of the three. The fold gates none of
        // them and emits a value under every input, and the collector's
        // zero-vs-unreadable conflation is not expressible from the payload —
        // see the four-state section of the module docs. A declared capability
        // the mapper could never emit would cost real bits for nothing.
        Field::new(f::ACTIVE_TIMED, Kind::Unsigned),
        Field::new(f::ACTIVE_TARGETED, Kind::Unsigned),
        Field::new(f::ACTIVE_TARGETED_TIMED, Kind::Unsigned),
    ];

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: COUNTRIES,
            // `tag_db_id` is a stable identity that persists across hours, and
            // only a keyed set has a delta grammar (§7.3). The fold's per-tag
            // upsert (`world_fold.rs::fold_decisions`) is a keyed-set update, not
            // a replacement: see [`Accumulator`].
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 24-byte payload header (`world_fold.rs::fold_decisions`, written at
/// `attach_log.rs:69593`-`:69598`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub country_count: u32,
    /// `attach_log.rs:69596` packs the same word as `country_count`, and
    /// `fold_decisions` never reads it. Surfaced for reporting; see
    /// [`split_payload`] for why it is not enforced.
    pub changed_rows: u32,
    /// `attach_log.rs:69597`. **`fold_decisions` never reads this word**, so this
    /// module cannot act on it either — the exclusion decision belongs to the
    /// caller and is recorded in the measurement harness.
    pub status_flags: u32,
    pub timed_count: u32,
}

/// Split a d16 DATA payload into its header, its country rows and the raw timed
/// region, applying the structural checks `fold_decisions` applies
/// (`world_fold.rs::fold_decisions`).
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness that
/// can drift from the recorder without anything failing.
///
/// **The timed region is returned unvalidated.** Its length is not a function of
/// the header — every row carries a variable-length string — so the fold's
/// trailing-byte rule (`world_fold.rs::fold_decisions`) can only be checked by walking it. That walk is
/// [`parse_timed_rows`], and a caller that wants the fold's acceptance decision
/// must run it; [`Accumulator::fold_payload`] does.
///
/// **No `changed_rows == country_count` check.** d16 *is* full-replace per tag,
/// and the writer does pack the two words identically (`attach_log.rs:69595`-
/// `:69596`), but `fold_decisions` never reads `+12` at all. Enforcing an
/// invariant the fold does not state would reject payloads the recorder folds,
/// which is the data-loss direction; the harness counts mismatches and reports
/// them instead (the reference capture has 0 in 903 records).
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8], &[u8]), String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "decisions payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        country_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        timed_count: u32_at(payload, 20),
    };
    // Checked arithmetic: `country_count` is attacker-shaped input from a stream
    // on disk, and `country_count * COUNTRY_ROW_LEN` overflows `usize` for a large
    // enough word. The fold's `need(payload, cursor + 32)` per row reaches the
    // same rejection one row at a time; doing it in one multiplication must not
    // introduce a wrap the per-row form cannot have.
    let countries_len = (header.country_count as usize)
        .checked_mul(COUNTRY_ROW_LEN)
        .ok_or_else(|| {
            format!(
                "decisions country_count {} overflows",
                header.country_count
            )
        })?;
    let countries_end = HEADER_LEN
        .checked_add(countries_len)
        .ok_or_else(|| "decisions payload length overflows".to_string())?;
    if payload.len() < countries_end {
        return Err(format!(
            "decisions payload {} shorter than {countries_end} for {} country row(s)",
            payload.len(),
            header.country_count
        ));
    }
    Ok((
        header,
        &payload[HEADER_LEN..countries_end],
        &payload[countries_end..],
    ))
}

/// One row of the stopped `timed_live` table, borrowed from the payload
/// (`world_fold.rs::fold_decisions`).
///
/// This is **not** a `Record` and there is no mapper for it. It exists so the
/// fold's structural rules can be enforced and so the deliverable can account for
/// the table's bytes; see "Why `timed_live` stops" in the module docs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimedRow<'a> {
    pub tag_db_id: u32,
    pub days: i32,
    pub state: i32,
    pub decision: &'a str,
}

/// Walk the timed region exactly as `fold_decisions` does
/// (`world_fold.rs::fold_decisions`), including its trailing-byte rule.
///
/// The fold reads `timed_count` rows and then requires the cursor to land on
/// `payload.len()`; a region with bytes left over, or one that runs short, is a
/// malformed payload and the whole frame is rejected. `read_string`
/// (`world_fold.rs::read_string`) also rejects invalid UTF-8, which is mirrored.
///
/// Total over arbitrary bytes: every read is bounds-checked, and each iteration
/// consumes at least `TIMED_ROW_FIXED_LEN + STRING_LEN_PREFIX` bytes, so a
/// `timed_count` of `u32::MAX` terminates in an error rather than spinning.
pub fn parse_timed_rows(
    region: &[u8],
    timed_count: u32,
) -> core::result::Result<Vec<TimedRow<'_>>, String> {
    let mut rows = Vec::new();
    let mut cursor = 0usize;
    for index in 0..timed_count {
        // `world_fold.rs::fold_decisions` — `need(payload, cursor + 12, "decisions timed row")`.
        if region.len() < cursor + TIMED_ROW_FIXED_LEN {
            return Err(format!(
                "decisions timed row {index}: {} byte(s) left, needs {TIMED_ROW_FIXED_LEN}",
                region.len() - cursor
            ));
        }
        let tag_db_id = u32_at(region, cursor);
        let days = i32_at(region, cursor + 4);
        let state = i32_at(region, cursor + 8);
        cursor += TIMED_ROW_FIXED_LEN;
        // `read_string`, `world_fold.rs::read_string`.
        if region.len() < cursor + STRING_LEN_PREFIX {
            return Err(format!("decisions timed row {index}: truncated string length"));
        }
        let len = u16::from_le_bytes([region[cursor], region[cursor + 1]]) as usize;
        cursor += STRING_LEN_PREFIX;
        if region.len() < cursor + len {
            return Err(format!(
                "decisions timed row {index}: string of {len} byte(s) runs past the payload"
            ));
        }
        let decision = core::str::from_utf8(&region[cursor..cursor + len])
            .map_err(|e| format!("decisions timed row {index}: invalid utf-8: {e}"))?;
        cursor += len;
        rows.push(TimedRow { tag_db_id, days, state, decision });
    }
    // `world_fold.rs::fold_decisions` — the cursor must land exactly on the end.
    if cursor != region.len() {
        return Err(format!(
            "decisions trailing bytes: consumed {cursor} of {} in the timed region",
            region.len()
        ));
    }
    Ok(rows)
}

/// The key of one country row: its `tag_db_id` (`world_fold.rs::fold_decisions`).
///
/// Zero is a legal key. `fold_decisions` applies no zero check — unlike
/// `fold_country_core_scalars` (`world_fold.rs::fold_country_core_scalars`) — so a zero `tag_db_id` becomes the
/// country entry `0` rather than a rejected frame, and this mirrors that.
pub fn key_from_country_row(row: &[u8]) -> core::result::Result<Key, String> {
    check_country_row_len(row)?;
    Ok(key(&[u32_at(row, 0) as u64]))
}

/// One compact row of the `countries` table, as the collector emits it today
/// (`attach_log.rs:69608`-`:69634`), onto its stored Record. `row` is the raw
/// 32-byte row slice (the caller has already consumed the payload header via
/// [`split_payload`]); d16 has exactly one country row form, so length selects
/// nothing and a wrong length is rejected. `frame_tick` is §6's derived-field
/// input (stored = v + c * (frame_tick >> u)); d16 declares no derived field, so
/// it is taken and ignored — every domain's mapper has the same shape.
///
/// Nothing in this function can panic on any 32-byte input, and no 32-byte input
/// is rejected: `fold_decisions` reads every one of these three words
/// unconditionally, applies no range check and applies no flag gate
/// (`world_fold.rs::fold_decisions`), so a mapper that refused a row here would
/// drop data the recorder keeps. The row's `flags` word and its three unread tail
/// words are ignored exactly as the fold ignores them.
pub fn record_from_country_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d16 has no derived field; §6's transform input is unused. Named rather than
    // `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    check_country_row_len(row)?;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let Some((_, offset)) = COUNTER_OFFSETS
            .iter()
            .copied()
            .find(|(id, _)| *id == f.field_id)
        else {
            // A collection this mapper does not know how to fill. Filling it with
            // a placeholder would publish invented data under a field id that
            // means something else.
            return Err(format!(
                "decisions countries has no mapping for field id {}",
                f.field_id
            ));
        };
        // Raw, unscaled, ungated — `world_fold.rs::fold_decisions`. A zero here is
        // a measured value (§3), not an empty state; see the module docs for why
        // the collector's zero-vs-unreadable conflation is not expressed as
        // `Unknown`.
        values.push(Value::Unsigned(u32_at(row, offset) as u64));
    }
    Ok(Record::new(values))
}

// --- The frame-level fold ----------------------------------------------------

/// What one payload did to the accumulator, for the harness's reporting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FoldOutcome {
    /// Country rows in the payload (`header.country_count`).
    pub country_rows: usize,
    /// Country rows whose `tag_db_id` was already claimed earlier in the SAME
    /// payload. The fold's `per_tag.insert` (`world_fold.rs::fold_decisions`) makes those
    /// last-writer-wins rather than a rejected frame, which is mirrored; the
    /// count is reported because it is a property worth noticing, not an error.
    pub duplicate_country_tags: usize,
    /// Rows in the stopped `timed_live` table, parsed for validation only.
    pub timed_rows: usize,
    /// Timed rows whose `tag_db_id` matched no country row in this payload. The
    /// fold drops them silently (`world_fold.rs::fold_decisions`); counted here so the
    /// silence is at least measured.
    pub timed_rows_dropped: usize,
}

/// d16's accumulated per-country state — the fourth public boundary the recipe
/// calls for, because one d16 record is not one d16 state.
///
/// `fold_decisions` commits each payload with `self.country_mut(tag).insert(..)`
/// (`world_fold.rs::fold_decisions`): it upserts the tags the payload carries and
/// **clears nothing**, so a country that stops appearing keeps its last folded
/// counters indefinitely. There is no baseline/delta distinction to model — the
/// fold never reads `status_flags` (and so never sees `COMPACT_STATUS_DELTA_DATA`)
/// and has no clear-on-baseline path at all. Every payload is an upsert.
///
/// This lives beside the schema rather than in a harness so the measurement bin,
/// the tests and any future caller share one implementation.
#[derive(Clone, Debug, Default)]
pub struct Accumulator {
    /// The `countries` collection's current content, keyed by `tag_db_id`.
    pub countries: KeyedState,
}

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one d16 DATA payload, exactly as `fold_decisions` does.
    ///
    /// The caller owns the failure-status decision: `fold_decisions` never reads
    /// `status_flags` (`world_fold.rs::fold_decisions` contains no read of `+16`),
    /// so there is no fold behaviour here to mirror and this function applies
    /// every payload it is given. The measurement harness records its own
    /// refuse-or-exclude choice explicitly.
    ///
    /// On `Err` the accumulator is left exactly as it was. That mirrors the fold,
    /// whose every error path — the header check (`:13802`), the per-row `need`
    /// (`:13808`, `:13828`), `read_string` (`:13833`) and the trailing-byte check
    /// (`:13841`) — precedes its first world mutation at `:13847`.
    pub fn fold_payload(
        &mut self,
        countries_collection: &Collection,
        payload: &[u8],
    ) -> core::result::Result<FoldOutcome, String> {
        let (header, country_rows, timed_region) = split_payload(payload)?;

        // Build everything before mutating anything, so a structural error leaves
        // the accumulator where the fold would have left it.
        let mut staged: std::collections::BTreeMap<Key, Record> = std::collections::BTreeMap::new();
        let mut tags: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        let mut duplicate_country_tags = 0usize;
        let mut rows = 0usize;
        for row in country_rows.chunks_exact(COUNTRY_ROW_LEN) {
            rows += 1;
            let k = key_from_country_row(row)?;
            let record = record_from_country_row(countries_collection, row, 0)?;
            // `per_tag.insert`, `world_fold.rs::fold_decisions`: last writer wins.
            if staged.insert(k, record).is_some() {
                duplicate_country_tags += 1;
            }
            tags.insert(u32_at(row, 0));
        }
        // `chunks_exact` cannot under-run: `split_payload` sized the slice from
        // `country_count`. Asserting it anyway keeps the two in one contract.
        if rows != header.country_count as usize {
            return Err(format!(
                "decisions country row count {rows} != header {}",
                header.country_count
            ));
        }

        // Parse the stopped table. Not mapped, but the fold's trailing-byte rule
        // (`:13841`) rejects the WHOLE payload when it is malformed, so accepting
        // a frame without this walk would accept frames the recorder refuses.
        let timed = parse_timed_rows(timed_region, header.timed_count)?;
        // `world_fold.rs::fold_decisions` — a timed row whose tag matched no country row in
        // this payload is dropped without a diagnostic.
        let timed_rows_dropped = timed
            .iter()
            .filter(|r| !tags.contains(&r.tag_db_id))
            .count();

        // `world_fold.rs::fold_decisions` — upsert per tag, clear nothing.
        for (k, record) in staged {
            self.countries.insert(k, record);
        }

        Ok(FoldOutcome {
            country_rows: rows,
            duplicate_country_tags,
            timed_rows: timed.len(),
            timed_rows_dropped,
        })
    }
}

fn check_country_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != COUNTRY_ROW_LEN {
        return Err(format!(
            "decisions country row length {} != {COUNTRY_ROW_LEN}",
            row.len()
        ));
    }
    Ok(())
}

/// Callers reach these only after a length check, so the slices are in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn i32_at(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta};
    use crate::schema::Quant;
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn countries() -> Collection {
        schema().collections[0].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// A raw 32-byte country row, built the way the collector packs one
    /// (`attach_log.rs:69608`-`:69634`): tag, flags, the three ported counters,
    /// then the three words the fold never reads.
    fn country_row(tag: u32, flags: u32, counters: [u32; 3], tail: [u32; 3]) -> Vec<u8> {
        let mut row = Vec::with_capacity(COUNTRY_ROW_LEN);
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        for v in counters {
            row.extend_from_slice(&v.to_le_bytes());
        }
        for v in tail {
            row.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(row.len(), COUNTRY_ROW_LEN);
        row
    }

    fn full_row() -> Vec<u8> {
        country_row(
            7,
            FLAG_STATUS_PRESENT | FLAG_TIMED_LIVE_READ,
            [11, 22, 33],
            [44, 55, 66],
        )
    }

    /// A timed row exactly as `attach_log.rs:69640`-`:69643` packs one.
    fn timed_row(tag: u32, days: i32, state: i32, decision: &str) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&days.to_le_bytes());
        row.extend_from_slice(&state.to_le_bytes());
        row.extend_from_slice(&(decision.len() as u16).to_le_bytes());
        row.extend_from_slice(decision.as_bytes());
        row
    }

    fn payload(status_flags: u32, country_rows: &[Vec<u8>], timed_rows: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(country_rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&(country_rows.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(timed_rows.len() as u32).to_le_bytes());
        for r in country_rows {
            p.extend_from_slice(r);
        }
        for r in timed_rows {
            p.extend_from_slice(r);
        }
        p
    }

    // --- Step 8.1 ------------------------------------------------------------

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d16 schema is well-formed");
    }

    // --- Step 8.5 ------------------------------------------------------------

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_country() {
        let c = countries();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys rows by tag_db_id alone");
        assert_eq!(keys[0].field_id, field::TAG_DB_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a country tag is content-defined (§5), so a manifest must resolve it"
        );
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(c.fields.len(), 4, "1 key + 3 counters");
        for f in c.value_fields() {
            assert_eq!(f.kind, Kind::Unsigned, "field {} is a vector size", f.field_id);
        }
        assert_eq!(schema().collections.len(), 1, "timed_live is stopped, not ported");
    }

    // --- Step 8.3: the states the schema declares ----------------------------

    #[test]
    fn the_schema_declares_no_empty_state_and_the_mapper_emits_none() {
        // The capability-conditional cases of step 8.3 do not apply here, and this
        // test is the record of why rather than a silent omission. The schema
        // declares no `.optional()`, no `.nullable()` and no `.unknown_capable()`,
        // because `fold_decisions` reads all three counters unconditionally
        // (`world_fold.rs::fold_decisions`) — there is no gate whose clear state
        // could mean inapplicable, empty or uncaptured. So the mapper must emit a
        // value for every field on every 32-byte input, and it does, including on
        // an all-zero row and on a row with every bit set.
        let c = countries();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(c.value_fields().all(|f| !f.nullable), "no field is a relation");
        assert!(
            c.value_fields().all(|f| !f.unknown_capable),
            "no field can be uncaptured on this payload — see the module docs"
        );
        for row in [
            country_row(0, 0, [0; 3], [0; 3]),
            country_row(u32::MAX, u32::MAX, [u32::MAX; 3], [u32::MAX; 3]),
            full_row(),
        ] {
            let r = record_from_country_row(&c, &row, 0).expect("every 32-byte row maps");
            for v in &r.values {
                assert!(
                    !matches!(v, Value::Absent | Value::Null | Value::Unknown),
                    "the mapper may only emit values"
                );
            }
        }
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // The common case in the reference capture: 58.50% of counter cells read
        // zero, overwhelmingly countries with no active decisions of that kind.
        // §3 keeps zero out of the three empty states, and the collector's
        // zero-vs-unreadable conflation (`attach_log.rs:83209`-`:83212`) is not
        // expressible from the payload, so it must not be guessed at here.
        let c = countries();
        let r = record_from_country_row(&c, &country_row(1, 0, [0; 3], [0; 3]), 0).expect("maps");
        for (id, _) in COUNTER_OFFSETS {
            assert_eq!(
                r.values[index_of(&c, id)],
                Value::Unsigned(0),
                "counter {id} zero is a measured value"
            );
        }
    }

    // --- Step 8.6: every field's offset, and the absence of any scale --------

    #[test]
    fn every_counter_reads_its_own_offset_raw() {
        // Distinct values per offset, so a transposed pair fails — the unscaled
        // form of the 32768-vs-100000 test. `active_timed` is at `+8`,
        // `active_targeted` at `+12`, `active_targeted_timed` at `+16`
        // (`world_fold.rs::fold_decisions`).
        let c = countries();
        let r = record_from_country_row(&c, &country_row(9, 3, [100, 200, 300], [400, 500, 600]), 0)
            .expect("maps");
        assert_eq!(r.values[index_of(&c, field::ACTIVE_TIMED)], Value::Unsigned(100));
        assert_eq!(r.values[index_of(&c, field::ACTIVE_TARGETED)], Value::Unsigned(200));
        assert_eq!(
            r.values[index_of(&c, field::ACTIVE_TARGETED_TIMED)],
            Value::Unsigned(300)
        );

        // One offset at a time, everything else zero: an offset that read its
        // neighbour would show up as a zero where a value belongs.
        for (i, (id, offset)) in COUNTER_OFFSETS.into_iter().enumerate() {
            let mut counters = [0u32; 3];
            counters[i] = 4_242;
            let r = record_from_country_row(&c, &country_row(1, 0, counters, [7, 7, 7]), 0)
                .expect("maps");
            for (other, _) in COUNTER_OFFSETS {
                let expected = if other == id { 4_242 } else { 0 };
                assert_eq!(
                    r.values[index_of(&c, other)],
                    Value::Unsigned(expected),
                    "field {other} while {id} at +{offset} carries 4242"
                );
            }
        }

        // No scale anywhere: the fold divides nothing (`:13819`-`:13821`), so
        // values that would be mangled by a 32768 or 100000 divisor survive whole.
        for raw in [1u32, 32_768, 100_000, u32::MAX] {
            let r = record_from_country_row(&c, &country_row(1, 0, [raw, raw, raw], [0; 3]), 0)
                .expect("maps");
            for (id, _) in COUNTER_OFFSETS {
                assert_eq!(
                    r.values[index_of(&c, id)],
                    Value::Unsigned(raw as u64),
                    "counter {id} is emitted raw, never divided"
                );
            }
        }
    }

    #[test]
    fn the_unread_row_tail_and_the_flags_word_change_nothing() {
        // `world_fold.rs::fold_decisions` binds the flags word to `_flags` and drops it, and
        // nothing reads `+20`, `+24` or `+28`. Two rows differing only in those 16
        // bytes must map identically — the mapper must not have quietly adopted a
        // field the fold does not carry.
        let c = countries();
        let a = record_from_country_row(&c, &country_row(5, 0, [1, 2, 3], [0, 0, 0]), 0).unwrap();
        let b = record_from_country_row(
            &c,
            &country_row(5, u32::MAX, [1, 2, 3], [u32::MAX; 3]),
            0,
        )
        .unwrap();
        assert_eq!(a, b, "flags and the unread tail are not this domain's data");
        // And the offsets named in `UNREAD_TAIL_OFFSETS` are exactly the ones the
        // mapper never reads, one at a time.
        for (name, offset) in UNREAD_TAIL_OFFSETS {
            let mut row = country_row(5, 0, [1, 2, 3], [0, 0, 0]);
            row[offset..offset + 4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
            assert_eq!(
                record_from_country_row(&c, &row, 0).unwrap(),
                a,
                "{name} at +{offset} must not reach the record"
            );
        }
    }

    // --- Step 8.4 ------------------------------------------------------------

    #[test]
    fn no_field_is_quantized_because_none_is_continuous() {
        // Step 4's "quantization saves bytes" test does not apply: quantization is
        // declared on `Continuous` fields only, and d16 has none. All three
        // counters are vector sizes the fold emits as integers with no divisor
        // (`world_fold.rs::fold_decisions`), so there is nothing to draw rather
        // than interpolate and nothing to round.
        let c = countries();
        assert_eq!(
            c.value_fields()
                .filter(|f| matches!(f.kind, Kind::Continuous))
                .count(),
            0,
            "d16 has no continuous field"
        );
        for f in c.value_fields() {
            assert_eq!(f.quant, Quant::None, "field {} declares no quantizer", f.field_id);
        }
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; the counters move on player
        // and AI action, so `frame_tick` never enters a value. The one clock-linear
        // quantity this domain touches — a decision countdown — lives in the
        // stopped `timed_live` table and is not carried here.
        assert!(countries().fields.iter().all(|f| f.derived.is_none()));
        let c = countries();
        let a = record_from_country_row(&c, &full_row(), 0).unwrap();
        let b = record_from_country_row(&c, &full_row(), u64::MAX).unwrap();
        assert_eq!(a, b, "the tick is taken and ignored");
    }

    // --- Step 8.2 ------------------------------------------------------------

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let c = countries();
        let record = record_from_country_row(&c, &full_row(), 0).expect("maps");
        let mut bytes = Vec::new();
        record.encode(&c, &mut bytes).expect("encodes");
        let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
        assert_eq!(back, record, "decode reproduces the record");
        let mut reencoded = Vec::new();
        back.encode(&c, &mut reencoded).expect("re-encodes");
        assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
    }

    #[test]
    fn an_all_zero_and_an_all_max_row_roundtrip_byte_for_byte() {
        let c = countries();
        for row in [
            country_row(0, 0, [0; 3], [0; 3]),
            country_row(u32::MAX, u32::MAX, [u32::MAX; 3], [u32::MAX; 3]),
        ] {
            let record = record_from_country_row(&c, &row, 0).expect("maps");
            let mut bytes = Vec::new();
            record.encode(&c, &mut bytes).expect("encodes");
            let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
            assert_eq!(pos, bytes.len());
            assert_eq!(back, record);
            let mut reencoded = Vec::new();
            back.encode(&c, &mut reencoded).expect("re-encodes");
            assert_eq!(reencoded, bytes);
        }
    }

    #[test]
    fn a_baseline_and_a_delta_survive_the_reader() {
        // The whole grammar over this schema, not just one record: what the
        // measurement harness asserts every hour, in miniature.
        let s = schema();
        let c = countries();
        let mut prior: KeyedState = KeyedState::new();
        for (tag, counters) in [(7u32, [1u32, 2, 3]), (9, [0, 0, 0])] {
            let row = country_row(tag, 3, counters, [0; 3]);
            prior.insert(
                key_from_country_row(&row).unwrap(),
                record_from_country_row(&c, &row, 0).unwrap(),
            );
        }
        let mut next = prior.clone();
        let changed = country_row(7, 3, [5, 2, 3], [0; 3]);
        next.insert(
            key_from_country_row(&changed).unwrap(),
            record_from_country_row(&c, &changed, 0).unwrap(),
        );

        let mut base = Vec::new();
        encode_baseline(&s, std::slice::from_ref(&prior), &mut base).expect("baseline encodes");
        let (decoded, used) = decode_baseline(&s, &base, 0).expect("baseline decodes");
        assert_eq!(used, base.len());
        let mut reader = decoded.into_iter().next().unwrap();
        assert_eq!(reader, prior);

        let d = diff(&c, &prior, &next);
        assert!(!d.is_empty(), "one counter moved");
        let mut delta = Vec::new();
        encode_delta(&s, std::slice::from_ref(&prior), &[Some(d)], &mut delta)
            .expect("delta encodes");
        let (deltas, used) =
            decode_delta(&s, std::slice::from_ref(&reader), &delta, 0).expect("delta decodes");
        assert_eq!(used, delta.len());
        apply(&mut reader, deltas[0].as_ref().unwrap());
        assert_eq!(reader, next, "the reader reconstructs the fold's state");
    }

    // --- The mapper's own contract -------------------------------------------

    #[test]
    fn the_mapper_rejects_only_a_wrong_row_length() {
        let c = countries();
        for len in [0usize, 1, COUNTRY_ROW_LEN - 1, COUNTRY_ROW_LEN + 1, 48] {
            assert!(
                record_from_country_row(&c, &vec![0u8; len], 0).is_err(),
                "length {len} must be rejected, not read"
            );
            assert!(key_from_country_row(&vec![0u8; len]).is_err(), "length {len}");
        }
        // And no 32-byte row is rejected — the fold rejects none.
        // Every flag bit, every tag edge, every counter edge.
        for bit in 0..32u32 {
            assert!(record_from_country_row(&c, &country_row(1, 1 << bit, [0; 3], [0; 3]), 0).is_ok());
        }
        for tag in [0u32, 1, u32::MAX] {
            assert!(record_from_country_row(&c, &country_row(tag, 0, [0; 3], [0; 3]), 0).is_ok());
            assert!(key_from_country_row(&country_row(tag, 0, [0; 3], [0; 3])).is_ok());
        }
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a record
        // full of placeholders under this domain's field ids.
        let mut c = countries();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_country_row(&c, &full_row(), 0).is_err());
    }

    #[test]
    fn the_key_is_the_tag_db_id_and_zero_is_legal() {
        assert_eq!(key_from_country_row(&full_row()).expect("keys"), key(&[7]));
        // `fold_decisions` applies no zero check (contrast `world_fold.rs:9279`),
        // so a zero tag is a country entry, not a rejected frame.
        assert_eq!(
            key_from_country_row(&country_row(0, 0, [0; 3], [0; 3])).expect("keys"),
            key(&[0])
        );
    }

    // --- split_payload -------------------------------------------------------

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let row = full_row();
        let timed = timed_row(7, 30, 0, "GER_test_decision");
        let p = payload(0, &[row.clone()], &[timed.clone()]);
        let (header, countries_region, timed_region) = split_payload(&p).expect("well-formed");
        assert_eq!(header.country_count, 1);
        assert_eq!(header.changed_rows, 1);
        assert_eq!(header.status_flags, 0);
        assert_eq!(header.timed_count, 1);
        assert_eq!(countries_region, &row[..]);
        assert_eq!(timed_region, &timed[..]);

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
        }
        // Short of the country region.
        for len in HEADER_LEN..HEADER_LEN + COUNTRY_ROW_LEN {
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
        }
        // A country_count whose byte length overflows `usize` must be rejected,
        // not wrapped into a plausible small length.
        let mut huge = p.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());

        // `changed_rows != country_count` is NOT rejected: `fold_decisions` never
        // reads `+12`, and rejecting a payload the recorder folds would lose data.
        let mut mismatched = p.clone();
        mismatched[12..16].copy_from_slice(&99u32.to_le_bytes());
        let (h, _, _) = split_payload(&mismatched).expect("the fold accepts this");
        assert_eq!(h.changed_rows, 99, "surfaced for reporting, not enforced");

        // A failure status is likewise surfaced and not acted on here: the fold
        // never reads `+16`, so the exclusion decision belongs to the caller.
        let mut failed = p.clone();
        failed[16..20].copy_from_slice(&1u32.to_le_bytes());
        let (h, _, _) = split_payload(&failed).expect("the fold accepts this");
        assert_eq!(h.status_flags, 1);
    }

    #[test]
    fn split_payload_accepts_an_empty_payload_body() {
        // 53 of the reference capture's 903 records carry zero country rows; the
        // fold accepts them and folds nothing.
        let p = payload(0, &[], &[]);
        let (header, c, t) = split_payload(&p).expect("well-formed");
        assert_eq!(header.country_count, 0);
        assert!(c.is_empty() && t.is_empty());
    }

    // --- parse_timed_rows (the stopped table, validated only) ----------------

    #[test]
    fn parse_timed_rows_mirrors_read_string_and_the_trailing_check() {
        let rows = [
            timed_row(7, 30, 0, "GER_test_decision"),
            timed_row(9, -1, 2, ""),
        ];
        let mut region = Vec::new();
        for r in &rows {
            region.extend_from_slice(r);
        }
        let parsed = parse_timed_rows(&region, 2).expect("well-formed");
        assert_eq!(parsed[0], TimedRow { tag_db_id: 7, days: 30, state: 0, decision: "GER_test_decision" });
        assert_eq!(parsed[1], TimedRow { tag_db_id: 9, days: -1, state: 2, decision: "" });

        // Trailing bytes: `world_fold.rs::fold_decisions` rejects the whole payload.
        let mut extra = region.clone();
        extra.push(0);
        assert!(parse_timed_rows(&extra, 2).is_err(), "one trailing byte");
        // A count that under-reads the region leaves bytes over.
        assert!(parse_timed_rows(&region, 1).is_err());
        // A count that over-reads runs short.
        assert!(parse_timed_rows(&region, 3).is_err());
        // Every truncation.
        for len in 0..region.len() {
            assert!(parse_timed_rows(&region[..len], 2).is_err(), "cut at {len}");
        }
        // Invalid UTF-8 (`world_fold.rs::read_string`).
        let mut bad = Vec::new();
        bad.extend_from_slice(&7u32.to_le_bytes());
        bad.extend_from_slice(&0i32.to_le_bytes());
        bad.extend_from_slice(&0i32.to_le_bytes());
        bad.extend_from_slice(&2u16.to_le_bytes());
        bad.extend_from_slice(&[0xFF, 0xFE]);
        assert!(parse_timed_rows(&bad, 1).is_err(), "invalid utf-8");
        // A huge count terminates in an error rather than spinning.
        assert!(parse_timed_rows(&region, u32::MAX).is_err());
        assert!(parse_timed_rows(&[], 0).is_ok());
    }

    // --- The accumulator -----------------------------------------------------

    #[test]
    fn the_accumulator_upserts_and_never_clears() {
        // The defining property (`world_fold.rs::fold_decisions`): a country absent
        // from a later payload keeps its last folded counters. Getting this wrong
        // would silently drop 355 of 356 countries in the reference capture.
        let c = countries();
        let mut acc = Accumulator::new();
        acc.fold_payload(
            &c,
            &payload(
                0,
                &[
                    country_row(1, 3, [1, 0, 0], [0; 3]),
                    country_row(2, 3, [2, 0, 0], [0; 3]),
                ],
                &[],
            ),
        )
        .expect("folds");
        assert_eq!(acc.countries.len(), 2);

        // A payload carrying only country 1 must leave country 2 in place.
        acc.fold_payload(&c, &payload(0, &[country_row(1, 3, [9, 0, 0], [0; 3])], &[]))
            .expect("folds");
        assert_eq!(acc.countries.len(), 2, "country 2 is retained");
        assert_eq!(
            acc.countries[&key(&[1])].values[index_of(&c, field::ACTIVE_TIMED)],
            Value::Unsigned(9)
        );
        assert_eq!(
            acc.countries[&key(&[2])].values[index_of(&c, field::ACTIVE_TIMED)],
            Value::Unsigned(2)
        );

        // An empty payload changes nothing at all.
        let before = acc.countries.clone();
        let out = acc.fold_payload(&c, &payload(0, &[], &[])).expect("folds");
        assert_eq!(out, FoldOutcome::default());
        assert_eq!(acc.countries, before);
    }

    #[test]
    fn a_tag_repeated_inside_one_payload_is_last_writer_wins() {
        // `per_tag.insert`, `world_fold.rs::fold_decisions` — not a rejected frame.
        let c = countries();
        let mut acc = Accumulator::new();
        let out = acc
            .fold_payload(
                &c,
                &payload(
                    0,
                    &[
                        country_row(4, 3, [1, 1, 1], [0; 3]),
                        country_row(4, 3, [7, 7, 7], [0; 3]),
                    ],
                    &[],
                ),
            )
            .expect("folds");
        assert_eq!(out.country_rows, 2);
        assert_eq!(out.duplicate_country_tags, 1);
        assert_eq!(acc.countries.len(), 1);
        assert_eq!(
            acc.countries[&key(&[4])].values[index_of(&c, field::ACTIVE_TIMED)],
            Value::Unsigned(7),
            "the last row for a tag wins"
        );
    }

    #[test]
    fn identical_timed_rows_are_two_occurrences_not_one() {
        // The stop report's shape claim, pinned as behaviour. `fold_decisions`
        // `push`es (`world_fold.rs::fold_decisions`), so two byte-identical timed rows fold
        // to two entries in source order. Any keyed collection would collapse them
        // into one, which is why `Shape::OrderedOccurrenceList` is the collection's
        // real shape even though its separate string blocker keeps it unported.
        // Round-5 review finding: the report previously treated
        // `(tag_db_id, decision)` as a stable key without checking this.
        let region = {
            let mut r = Vec::new();
            r.extend_from_slice(&timed_row(7, 30, 0, "x"));
            r.extend_from_slice(&timed_row(7, 30, 0, "x"));
            r
        };
        let parsed = parse_timed_rows(&region, 2).expect("well-formed");
        assert_eq!(parsed.len(), 2, "both occurrences survive");
        assert_eq!(parsed[0], parsed[1], "and they are indistinguishable");

        let c = countries();
        let mut acc = Accumulator::new();
        let out = acc
            .fold_payload(
                &c,
                &payload(
                    0,
                    &[country_row(7, 3, [2, 0, 0], [0; 3])],
                    &[timed_row(7, 30, 0, "x"), timed_row(7, 30, 0, "x")],
                ),
            )
            .expect("folds");
        assert_eq!(out.timed_rows, 2, "the fold keeps both, and so does this");
        assert_eq!(out.timed_rows_dropped, 0);

        // Order is carried too: rows that differ only in position stay ordered.
        let ordered = {
            let mut r = Vec::new();
            r.extend_from_slice(&timed_row(7, 30, 0, "b"));
            r.extend_from_slice(&timed_row(7, 30, 0, "a"));
            r
        };
        let parsed = parse_timed_rows(&ordered, 2).expect("well-formed");
        assert_eq!(parsed[0].decision, "b", "source order, not sorted order");
        assert_eq!(parsed[1].decision, "a");
    }

    #[test]
    fn a_timed_row_with_no_matching_country_is_counted_not_rejected() {
        // `world_fold.rs::fold_decisions` drops it silently; the count makes the silence
        // visible without changing the fold's outcome.
        let c = countries();
        let mut acc = Accumulator::new();
        let out = acc
            .fold_payload(
                &c,
                &payload(
                    0,
                    &[country_row(4, 3, [1, 0, 0], [0; 3])],
                    &[timed_row(4, 5, 0, "a"), timed_row(99, 5, 0, "b")],
                ),
            )
            .expect("folds");
        assert_eq!(out.timed_rows, 2);
        assert_eq!(out.timed_rows_dropped, 1);
        assert_eq!(acc.countries.len(), 1);
    }

    #[test]
    fn a_malformed_payload_leaves_the_accumulator_untouched() {
        // Every fold error path precedes the world mutation at
        // `world_fold.rs::fold_decisions`, so a refused payload must not half-apply.
        let c = countries();
        let mut acc = Accumulator::new();
        acc.fold_payload(&c, &payload(0, &[country_row(1, 3, [1, 0, 0], [0; 3])], &[]))
            .expect("folds");
        let before = acc.countries.clone();

        let good = payload(
            0,
            &[country_row(2, 3, [5, 0, 0], [0; 3])],
            &[timed_row(2, 1, 0, "x")],
        );
        // Trailing byte — the country rows are valid, so this is precisely the
        // case where a careless implementation would apply them and then fail.
        let mut trailing = good.clone();
        trailing.push(0xFF);
        assert!(acc.fold_payload(&c, &trailing).is_err());
        assert_eq!(acc.countries, before, "no partial application");

        // Every truncation of a well-formed payload.
        for len in 0..good.len() {
            let _ = acc.fold_payload(&c, &good[..len]);
            assert_eq!(acc.countries, before, "truncation at {len} applied something");
        }
        // And the well-formed payload does apply.
        acc.fold_payload(&c, &good).expect("folds");
        assert_eq!(acc.countries.len(), 2);
    }

    #[test]
    fn the_accumulator_never_panics_on_arbitrary_bytes() {
        // `fold_payload` is public and reads bytes that came off a disk; it must
        // return a diagnosis, never unwind.
        let c = countries();
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..4_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 200;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            let mut acc = Accumulator::new();
            let _ = acc.fold_payload(&c, &bytes);
        }
        // And corruption of a genuinely well-formed payload, byte by byte.
        let good = payload(
            0,
            &[full_row(), country_row(2, 1, [0, 0, 0], [1, 2, 3])],
            &[timed_row(7, 30, 0, "GER_test_decision")],
        );
        for i in 0..good.len() {
            let mut p = good.clone();
            p[i] ^= 0xFF;
            let mut acc = Accumulator::new();
            let _ = acc.fold_payload(&c, &p);
        }
    }
}
