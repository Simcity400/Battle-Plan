//! The `variant_upgrades` grammar, shared by **d39 `air_variant_upgrades`** and
//! **d40 `stockpile_variant_upgrades`**.
//!
//! Derived from `extractor/native/recorder_host/src/world_fold.rs:15505`-`:15584`
//! (`fold_variant_upgrades`), its payload constants at `:993`-`:995`, its domain
//! ids at `:644`-`:645`, its dispatch at `:4704`-`:4705`, its accumulator element
//! at `:23893`-`:23907`, and the collector read and payload writer that produce
//! the rows (`extractor/native/collector/src/attach_log.rs:54023`-`:54196`,
//! `:57566`-`:57630`, `:70563`-`:70609`). Neither domain has a dossier
//! (`documents/domains/UNDOCUMENTED_DOMAINS.md` lists 39 and 40), so the fold and
//! the collector read are the whole basis.
//!
//! # One grammar, two domains
//!
//! `world_fold.rs:4704` and `:4705` dispatch **the same function** for both
//! domains; the only divergences inside it are the accumulator it writes
//! (`:15575`-`:15579`) and the name in its error strings (`:15506`-`:15510`).
//! The payload grammar is therefore byte-identical, and duplicating it into two
//! modules would give the fan-out two copies of one authority to keep in step.
//!
//! §11 assigns field ids **per domain**, so the two domains still need two
//! `Schema`s with two `domain` values. This module holds the whole grammar once,
//! parameterized by a [`Domain`] descriptor — a plain `(id, name)` value, not a
//! type parameter — and `crate::d39` / `crate::d40` are thin identity modules
//! that bind that descriptor and re-export the recipe's fixed boundary
//! (`record_from_*_row`, `key_from_*_row`, `split_payload`, `fold_payload`) at
//! each domain's own name. Field ids are numerically identical in both domains
//! because the fields are identical; that is not id reuse under §11, whose rule
//! is per-domain and forbids reusing an id for a *different* field.
//!
//! # Payload grammar
//!
//! (`world_fold.rs:15511`-`:15574`, writer `attach_log.rs:70573`-`:70608`.) A
//! 44-byte header, then `row_count` fixed 16-byte variant rows, then
//! `emitted_upgrade_rows` fixed 8-byte upgrade entries:
//!
//! | offset | width | header field |
//! |---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms` |
//! | `+8`  | u32 | `row_count` (`:15516`) |
//! | `+12` | u32 | `changed_rows` |
//! | `+16` | u32 | `status_flags` (`:15517`) |
//! | `+20` | u32 | `observed_variant_rows` |
//! | `+24` | u32 | `observed_upgrade_rows` |
//! | `+28` | u32 | `emitted_upgrade_rows` (`:15518`) |
//! | `+32` | u32 | `invalid_variant_rows` |
//! | `+36` | u32 | reserved, must be zero (`:15519`, `:15521`) |
//! | `+40` | u32 | reserved, must be zero (`:15520`, `:15521`) |
//!
//! | row | offset | width | field | scale | gate |
//! |---|---|---|---|---|---|
//! | variant | `+0`  | u32 | `variant_id`     | raw | none (key; zero rejected `:15542`) |
//! | variant | `+4`  | u32 | `flags`          | —   | **must be exactly 0** (`:15546`) |
//! | variant | `+8`  | u32 | `upgrade_first`  | —   | transport (`:15549`) |
//! | variant | `+12` | u32 | `upgrade_count`  | —   | transport (`:15550`) |
//! | upgrade | `+0`  | u32 | `upgrade_index`  | raw | none (key component, `:15571`) |
//! | upgrade | `+4`  | u32 | `level`          | raw | none (`:15572`) |
//!
//! **No scale factor anywhere.** All four state-bearing words are integers the
//! fold emits raw: `VariantUpgradeState::to_json` (`world_fold.rs:23900`-`:23905`)
//! passes `upgrade_index` and `level` through `json!` unchanged, and the two row
//! words that are not transport are the key. The 32768-vs-100000 class of error
//! has no surface in this domain; the offset test in place of the scale test is
//! `every_field_is_emitted_raw_at_its_own_offset`.
//!
//! **No strings, no cross-domain dependency, no delete row, no clear.**
//!
//! # Two collections
//!
//! * `variants` — keyed by `variant_id`, and carrying **no value fields at all**.
//! * `upgrades` — keyed by `(variant_id, ordinal)`, carrying `upgrade_index` and
//!   `level`.
//!
//! The `(upgrade_first, upgrade_count)` pair is **transport, never state**
//! (`:15549`-`:15558`, `:15580`-`:15583`): it says where this frame happened to
//! put a variant's entries in the shared array, so two frames carrying the same
//! world with the arrays laid out differently would differ in every index. The
//! fold itself keeps only the *slice* (`upgrades[first..end].to_vec()`), which is
//! why the entries flatten into a second keyed collection and those two words do
//! not survive.
//!
//! **The child key is the entry's ordinal in its variant's list, not its
//! `upgrade_index` word.** What the fold stores per variant is a
//! **`Vec<VariantUpgradeState>`** (`:15582`), and a `Vec` is a map from position
//! to element — so keying by position is what makes this collection reproduce the
//! fold's own container, element for element, for **every** payload the fold
//! accepts:
//!
//! * two entries in one range carrying the same `upgrade_index` are two rows, as
//!   they are two `Vec` elements — keying by `upgrade_index` would collapse them;
//! * a range whose indices are not ascending keeps the fold's order, because the
//!   keyed set sorts by ordinal — keying by `upgrade_index` would silently
//!   re-sort it.
//!
//! Neither payload is one the collector writes (see below), and neither is one
//! the fold rejects; a mapping that lost either would be losing data on input the
//! recorder folds. The ordinal is state and not transport for the same reason
//! `(first, count)` is transport and it is not: it indexes the **variant's own**
//! list, so it is unchanged by how a frame packs the shared array.
//!
//! `upgrade_index` therefore rides as an ordinary value field. In every row a
//! real capture contains it equals the ordinal, because the collector writes its
//! own loop counter into it (`attach_log.rs:57624`) — but the FOLD enforces
//! nothing of the kind, and the recipe's derived-witness rule is about invariants
//! the fold holds, not about coincidences of one writer. Dropping it would make
//! this schema unable to carry a payload the recorder accepts. Recorded in the
//! deliverable's open questions.
//!
//! **A key-only collection is deliberate, not a placeholder.** A variant row's
//! only non-transport words are its key and a flag word the fold requires to be
//! zero, so `variants` has an empty record — which encodes to zero bytes
//! (`bitmap.rs:56`, `record.rs:127`-`:217`) and costs only its key. Dropping the
//! collection and inferring variants from `upgrades` keys would lose every
//! variant whose upgrade list is empty, and that is not a corner: in the
//! reference capture **all 204 d39 variants have an empty upgrade list**, as do
//! 1111 of d40's 1204 — so without this collection d39 would encode nothing at
//! all across 146 hours of real data.
//!
//! # Four states: none
//!
//! Every field is a real measured value in every row. The row's `flags` word must
//! be **exactly** zero (`world_fold.rs:15546`) — there is no gate bit, and in
//! particular no delete bit — so no field can be `Absent`, `Null` or `Unknown`,
//! and the schema declares no `.optional()`, `.nullable()` or
//! `.unknown_capable()` capability. That is the recipe's step-8 case 3, recorded
//! here with its reason; `no_state_capability_is_declared` is the test that pins
//! it. Uncaptured is an **hour-level** state in these domains, never a per-field
//! one: a read that fails returns no rows and badges the record
//! `COMPACT_STATUS_SOURCE_UNAVAILABLE` (`attach_log.rs:54030`-`:54041`,
//! `:54064`-`:54075`, `:54109`), and a variant whose object or upgrade container
//! fails validation is dropped from the payload and counted in
//! `invalid_variant_rows` with `COMPACT_STATUS_PARTIAL_DATA`
//! (`:54150`-`:54153`, `:54169`-`:54186`) — both are failure statuses for these
//! domains (no expected-partial tolerance is registered for 39 or 40,
//! `compact_contract/src/lib.rs:138`-`:158`), so the fold folds nothing and
//! retains the accumulator (`world_fold.rs:15524`).
//!
//! **Zero is a value.** `level == 0` is a real upgrade slot at level zero — 43 of
//! the 228 d40 entries in the reference capture — and `upgrade_count == 0` is a
//! real variant with no upgrade slots, not a missing list.
//!
//! # Pure upsert: no baseline, no delta, no delete
//!
//! `fold_variant_upgrades` has **one** mode. It never clears its accumulator and
//! it never reads `COMPACT_STATUS_DELTA_DATA`; every record's rows are upserted
//! with `target.insert(variant_id, …)` (`:15580`-`:15583`). Consequences that the
//! rest of this module is built on:
//!
//! * There is no delta-before-baseline exclusion path — d22's and d23's second
//!   exclusion does not exist here, because there is no baseline to be before.
//!   The **only** path on which this fold returns without folding is a failure
//!   status (`:15524`), and it sits **after** the reserved-word check (`:15521`)
//!   and **before** the payload-length check (`:15531`). [`parse_header`] and
//!   [`split_payload`] are split at exactly that seam so a caller can reproduce
//!   the order: a failure-status record with a nonzero reserved word is a
//!   *rejected* frame, while a failure-status record whose length does not match
//!   its counts is *excluded and never length-checked*, which is what the
//!   recorder does.
//! * A variant key never leaves the `variants` collection, so that collection
//!   never produces a wire tombstone.
//! * A variant's upgrade list is carried **whole** by the row that owns it, so
//!   re-stating a variant with a shorter list **does** remove its stale children:
//!   the `upgrades` collection can produce wire tombstones even though the
//!   shipping grammar has no delete row. See [`fold_payload`].
//!
//! # Mirroring the fold
//!
//! The fold is the authority, so this module reproduces its permissiveness
//! exactly and refuses **nothing** it accepts: a `variant_id` repeated inside one
//! payload is last-writer-wins rather than a rejected frame (`:15580`, a plain
//! `insert` per row in payload order); two variants may address overlapping
//! slices of the entry array; an entry no row addresses is still decoded and
//! checked (`:15567`-`:15574`); `upgrade_index` is accepted over the whole `u32`
//! range with no zero check and no bound, and `level` likewise even though the
//! collector masks it to `0..=255` (`attach_log.rs:57626`); a variant range may
//! repeat an `upgrade_index` or carry its indices in any order, both of which
//! survive because the child key is the ordinal; and `changed_rows` is surfaced
//! but not checked, because the fold never reads it.
//!
//! The only rejections are the fold's own, at the fold's own positions: the
//! 44-byte minimum, the two reserved words, the exact payload length, a zero
//! `variant_id`, a nonzero `flags` word, a range outside the entry array, and the
//! exact-coverage rule.
//!
//! **No derived field (§6).** Nothing here changes deterministically with the
//! clock: a variant's upgrade levels move when a player edits a design. The
//! mappers still take `frame_tick` so every domain's boundary has one shape.

use core::ops::Range;

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

/// Which of the two domains a call is for.
///
/// `id` is the compact domain id (`world_fold.rs:644`-`:645`) and becomes
/// `Schema::domain`; `name` is the fold's own name for the domain in its
/// diagnostics (`:15506`-`:15510`), carried so that an error from this module
/// reads the way the recorder's would. `crate::d39` and `crate::d40` each bind
/// one of these; nothing else constructs one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Domain {
    pub id: u32,
    pub name: &'static str,
}

/// Collection ids. Both are `Shape::KeyedSet`.
pub const VARIANTS: u32 = 1;
pub const UPGRADES: u32 = 2;

/// Index of each collection in [`schema`]`().collections` and in the slices the
/// body encoders take. The order is the schema's and never changes.
pub const VARIANTS_INDEX: usize = 0;
pub const UPGRADES_INDEX: usize = 1;
pub const COLLECTION_COUNT: usize = 2;

/// Field ids are stable and append-only; §11 forbids reusing an id for a
/// *different* field. The two domains have independent id spaces and identical
/// fields, so they use the same numbers for the same concepts — `variant_id` is
/// deliberately one id across both collections and both domains.
pub mod field {
    pub const VARIANT_ID: u32 = 1;
    /// The entry's position in its own variant's list — the index of the fold's
    /// `Vec` (`world_fold.rs:15582`), which is what gives the child rows an
    /// identity that reproduces that `Vec` exactly. See the module docs.
    pub const UPGRADE_ORDINAL: u32 = 2;
    /// The `upgrade_index` word the payload carries at entry `+0`
    /// (`world_fold.rs:15571`). A value, not a key.
    pub const UPGRADE_INDEX: u32 = 3;
    pub const LEVEL: u32 = 4;
}

// --- Payload grammar, from `world_fold.rs:993`-`:995` and `:15511`-`:15574`. --

/// `VARIANT_UPGRADE_HEADER_LEN`, `world_fold.rs:993`.
pub const HEADER_LEN: usize = 44;
/// `VARIANT_UPGRADE_ROW_LEN`, `world_fold.rs:994`. Fixed; there is no legacy form.
pub const ROW_LEN: usize = 16;
/// `VARIANT_UPGRADE_ENTRY_ROW_LEN`, `world_fold.rs:995`.
pub const ENTRY_ROW_LEN: usize = 8;

/// The schema for one of the two domains.
///
/// Both schemas are the same grammar under two `domain` values: §11's canonical
/// bytes cover the collections and their fields, not the domain id, so
/// `d39::schema().schema_hash() == d40::schema().schema_hash()` — the identity
/// `the_two_domains_are_one_grammar` asserts.
pub fn schema(domain: Domain) -> Schema {
    use field as f;

    // `variants`: the roster of variants this domain has ever carried.
    //
    // No value fields. A variant row's four words are its key, a flag word the
    // fold requires to be zero (`world_fold.rs:15546`), and the `(first, count)`
    // transport pair — so the row's whole non-key content is "this variant
    // exists". The record encodes to zero bytes; the collection costs its keys
    // and nothing else. See the module docs for why it is not dropped.
    let variants = Collection {
        collection_id: VARIANTS,
        // `variant_id` is a stable identity that persists across hours (it is the
        // fold's own map key, `world_fold.rs:15582`), which is what a keyed set
        // needs — and only a keyed set has a delta grammar (§7.3), which this
        // domain needs because the shipping stream re-states a variant only when
        // it changes.
        shape: Shape::KeyedSet,
        fields: vec![Field::new(f::VARIANT_ID, Kind::Unsigned).key(0)],
    };

    // `upgrades`: one row per `(variant_id, ordinal)` — the entry's position in
    // its own variant's list — carrying the two words the entry holds.
    //
    // The ordinal is the key rather than the payload's `upgrade_index` word
    // because the fold stores a **`Vec`** (`world_fold.rs:15582`), and a `Vec` is
    // exactly a map from position to element. Keying by position therefore makes
    // this collection reproduce the fold's own container for **every** payload
    // the fold accepts: two entries carrying the same `upgrade_index` are two
    // rows, and the fold's order survives because the keyed set sorts by ordinal.
    // Keying by `upgrade_index` instead would silently collapse the first case
    // and silently re-sort the second, and both are payloads the fold folds.
    //
    // The ordinal is state, not transport: it indexes the **variant's own** list
    // (`upgrades[first..end]`), so it is unchanged by how a frame happens to pack
    // the shared entry array — unlike `(upgrade_first, upgrade_count)`, which is
    // exactly that packing and is therefore not carried at all.
    let upgrades = Collection {
        collection_id: UPGRADES,
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::VARIANT_ID, Kind::Unsigned).key(0),
            Field::new(f::UPGRADE_ORDINAL, Kind::Unsigned).key(1),
            // The payload word at entry `+0`, emitted raw by the fold
            // (`world_fold.rs:15571`, `:23903`). The collector happens to write
            // its own loop counter here (`attach_log.rs:57624`), which makes it
            // equal to the ordinal in every row a real capture contains — but the
            // FOLD enforces nothing of the kind, and the recipe's
            // derived-witness rule ("a value the fold hard-enforces as a function
            // of carried state is not a field") is about invariants the fold
            // holds, not coincidences of one writer. Carrying it keeps this
            // schema able to represent every payload the recorder accepts. See
            // the deliverable's open questions.
            Field::new(f::UPGRADE_INDEX, Kind::Unsigned),
            // Raw `u32`, never scaled (`world_fold.rs:23904`). `Unsigned` and not
            // `Monotone`: a level is a property of a design, not a counter, and a
            // redesign lowers it — the monotone hint would be a claim about the
            // data rather than merely an unhelpful one. Not `Continuous`, so §6's
            // quantization has no surface here at all.
            Field::new(f::LEVEL, Kind::Unsigned),
        ],
    };

    Schema {
        domain: domain.id,
        // Hourly, like every other compact lane: one DATA record per game hour
        // (146 records over 146 consecutive hours in the reference capture, for
        // each of the two domains).
        cadence_log2: 0,
        collections: vec![variants, upgrades],
        event_part: None,
    }
}

// --- The payload header ------------------------------------------------------

/// The 44-byte payload header (`world_fold.rs:15511`-`:15520`, written at
/// `attach_log.rs:70583`-`:70592`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    /// How many variant rows the payload carries (`world_fold.rs:15516`).
    pub row_count: u32,
    /// How many rows the collector *saw* change. The fold never reads this
    /// field, so nothing here constrains it; the writer happens to set it equal
    /// to `row_count` (`attach_log.rs:54187`, `:70585`), and asserting an
    /// equality the recorder does not enforce would reject streams the recorder
    /// accepts. Surfaced for reporting.
    pub changed_rows: u32,
    pub status_flags: u32,
    /// Variants the collector observed this hour, changed or not
    /// (`attach_log.rs:54140`). Diagnostic; the fold never reads it.
    pub observed_variant_rows: u32,
    /// Upgrade entries observed across all observed variants. Diagnostic.
    pub observed_upgrade_rows: u32,
    /// How many upgrade entries the payload carries (`world_fold.rs:15518`).
    pub emitted_upgrade_rows: u32,
    /// Variants dropped by the read because their object or upgrade container
    /// failed validation (`attach_log.rs:54151`). Diagnostic: a nonzero value
    /// also sets `COMPACT_STATUS_PARTIAL_DATA`, which is a failure status for
    /// these domains, so the fold excludes such an hour outright.
    pub invalid_variant_rows: u32,
}

/// The payload's two fixed-stride arrays, already bounded by the exact
/// total-length check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tables<'a> {
    pub variants: &'a [u8],
    pub upgrades: &'a [u8],
}

/// Read the header and apply **only** the checks the fold applies *before* it
/// consults the status word: the 44-byte minimum (`world_fold.rs:15511`) and the
/// two reserved words (`:15521`).
///
/// This is split from [`split_payload`] because the fold's order is load-bearing.
/// `handle_compact_failure` (`:15524`) returns before the payload-length check
/// (`:15531`), so a failure-status record whose length does not match its counts
/// is *excluded*, not rejected — while a failure-status record with a nonzero
/// reserved word **is** rejected. A caller that length-checks every record before
/// excluding failures refuses streams the recorder folds; one that excludes
/// failures before checking the reserved words accepts frames the recorder
/// refuses. Call this on every DATA record, then exclude on status, then call
/// [`split_payload`].
pub fn parse_header(domain: Domain, payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "{} payload {} shorter than the {HEADER_LEN}-byte header",
            domain.name,
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_variant_rows: u32_at(payload, 20),
        observed_upgrade_rows: u32_at(payload, 24),
        emitted_upgrade_rows: u32_at(payload, 28),
        invalid_variant_rows: u32_at(payload, 32),
    };
    // `world_fold.rs:15521` — both words, and either one alone is fatal.
    if u32_at(payload, 36) != 0 || u32_at(payload, 40) != 0 {
        return Err(format!("{} header reserved field is nonzero", domain.name));
    }
    Ok(header)
}

/// Split a DATA payload into its header and its two arrays, applying every
/// structural check the fold applies **after** the status word
/// (`world_fold.rs:15527`-`:15566`): the exact payload length, every variant
/// row's own words, every declared range against the entry array, and the exact
/// coverage rule `declared_upgrades == emitted_upgrade_rows`.
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness that
/// can drift from the recorder without anything failing.
///
/// Callers that must reproduce the fold's exclusion order call [`parse_header`]
/// first and skip this for a failure-status record — see [`parse_header`].
pub fn split_payload(
    domain: Domain,
    payload: &[u8],
) -> core::result::Result<(PayloadHeader, Tables<'_>), String> {
    let header = parse_header(domain, payload)?;
    // Checked arithmetic: both counts are input from a file on disk, and
    // `count * stride` overflows `usize` for a large enough word. The fold uses
    // `saturating_*` (`:15527`-`:15530`), which turns an overflowing product into
    // a length mismatch rather than a wrapped, plausible one; checked reports the
    // overflow as itself. Both refuse the payload.
    let variants_len = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .ok_or_else(|| format!("{} row_count {} overflows", domain.name, header.row_count))?;
    let upgrades_len = (header.emitted_upgrade_rows as usize)
        .checked_mul(ENTRY_ROW_LEN)
        .ok_or_else(|| {
            format!(
                "{} emitted_upgrade_rows {} overflows",
                domain.name, header.emitted_upgrade_rows
            )
        })?;
    let expected = HEADER_LEN
        .checked_add(variants_len)
        .and_then(|len| len.checked_add(upgrades_len))
        .ok_or_else(|| format!("{} payload length overflows", domain.name))?;
    if payload.len() != expected {
        return Err(format!(
            "{} len {} != {expected}",
            domain.name,
            payload.len()
        ));
    }
    let rows_end = HEADER_LEN + variants_len;
    let tables = Tables {
        variants: &payload[HEADER_LEN..rows_end],
        upgrades: &payload[rows_end..],
    };

    // `world_fold.rs:15537`-`:15566`: every row is parsed and range-checked, and
    // the declared ranges must cover the entry array exactly, before anything is
    // folded. A payload that fails either is not this grammar.
    let mut declared: usize = 0;
    for row in tables.variants.chunks_exact(ROW_LEN) {
        let (_, range) = variant_row(domain, &header, row)?;
        declared = declared
            .checked_add(range.len())
            .ok_or_else(|| format!("{} declared upgrade rows overflow", domain.name))?;
    }
    if declared != header.emitted_upgrade_rows as usize {
        return Err(format!(
            "{} emitted_upgrade_rows {} != declared {declared}",
            domain.name, header.emitted_upgrade_rows
        ));
    }
    Ok((header, tables))
}

// --- Row decoding ------------------------------------------------------------

/// One variant row's `variant_id` and the entry range it addresses, with the
/// fold's own per-row checks (`world_fold.rs:15541`-`:15558`).
///
/// The range is transport, never state — see the module docs. It is public
/// because the harness, the tests and [`fold_payload`] must all read it the same
/// way.
pub fn variant_row(
    domain: Domain,
    header: &PayloadHeader,
    row: &[u8],
) -> core::result::Result<(u32, Range<usize>), String> {
    let variant_id = variant_id_of_row(domain, row)?;
    let first = u32_at(row, 8) as usize;
    let count = u32_at(row, 12) as usize;
    // `world_fold.rs:15551`-`:15553`.
    let end = first
        .checked_add(count)
        .ok_or_else(|| format!("{} upgrade range overflow", domain.name))?;
    // `world_fold.rs:15554`-`:15558`.
    if end > header.emitted_upgrade_rows as usize {
        return Err(format!(
            "{} upgrade range {first}+{count} exceeds {}",
            domain.name, header.emitted_upgrade_rows
        ));
    }
    Ok((variant_id, first..end))
}

/// A variant row's `variant_id`, with the fold's zero check
/// (`world_fold.rs:15542`) and its exact-zero flags check (`:15546`).
///
/// The flags check belongs here rather than only in [`record_from_variant_row`]
/// because it is what makes the row *this grammar*: a nonzero flag word means an
/// unrecognised row form, and nothing in it — including its key — can be trusted.
pub fn variant_id_of_row(domain: Domain, row: &[u8]) -> core::result::Result<u32, String> {
    check_len(domain, row, ROW_LEN, "variant row")?;
    let variant_id = u32_at(row, 0);
    if variant_id == 0 {
        return Err(format!("{} row has zero variant_id", domain.name));
    }
    // `world_fold.rs:15545`-`:15548`. Exactly zero: there is no gate bit and no
    // delete bit in this grammar, so every other bit pattern is an unknown row
    // form rather than a row with a feature this mapper can ignore.
    let flags = u32_at(row, 4);
    if flags != 0 {
        return Err(format!(
            "{} row {variant_id} has unknown flags {flags:#x}",
            domain.name
        ));
    }
    Ok(variant_id)
}

/// An upgrade entry's `upgrade_index` (`world_fold.rs:15571`).
///
/// The fold applies no check to it at all — no zero rejection, no bound — so
/// none is applied here.
pub fn upgrade_index_of_row(domain: Domain, entry: &[u8]) -> core::result::Result<u32, String> {
    check_len(domain, entry, ENTRY_ROW_LEN, "upgrade entry")?;
    Ok(u32_at(entry, 0))
}

/// The key of one variant row: its `variant_id` (`world_fold.rs:15582`).
pub fn key_from_variant_row(domain: Domain, row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[variant_id_of_row(domain, row)? as u64]))
}

/// The key of one flattened upgrade row: `(variant_id, ordinal)`.
///
/// Neither half is read from the entry's own bytes: `variant_id` belongs to the
/// owning variant row and `ordinal` is the entry's position inside that row's
/// range — the index of the fold's `Vec` (`world_fold.rs:15582`). That is what
/// makes the key total over every payload the fold accepts; see [`schema`].
///
/// Infallible by construction, and shared rather than re-derived so the harness,
/// the tests and [`fold_payload`] cannot disagree about what a child's key is.
pub fn key_from_upgrade_row(variant_id: u32, ordinal: u32) -> Key {
    key(&[variant_id as u64, ordinal as u64])
}

// --- The boundary mapping ----------------------------------------------------

/// One compact row of the `variants` table, as the collector emits it today, onto
/// its stored Record. `row` is the raw 16-byte row slice (the caller has already
/// consumed the payload header via [`split_payload`]); there is exactly one
/// variant row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is §6's derived-field input (stored = v + c * (frame_tick >> u));
/// this domain declares no derived field, so it is taken and ignored — every
/// domain's mapper has the same shape.
///
/// The record is **empty**: every non-key word in a variant row is either the
/// flag word the fold pins to zero or the `(first, count)` transport pair. The
/// row is still fully validated, because a row the fold would reject must not
/// become a key here.
///
/// Nothing in this function can panic on any input.
pub fn record_from_variant_row(
    domain: Domain,
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // No derived field; §6's transform input is unused. Named rather than
    // `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    let _ = variant_id_of_row(domain, row)?;
    // `variants` is the collection with **no** value fields, so the field-id loop
    // every other domain's mapper relies on cannot tell it from another
    // collection on its own: an unrecognised collection with no value fields
    // would silently produce a correct-looking empty record. The collection id is
    // checked first for exactly that reason — see [`record_from_upgrade_row`],
    // where the same trap runs the other way.
    check_collection(domain, collection, VARIANTS, "variants")?;
    // And any value field at all still means this is not `variants`, whatever its
    // id claims. Filling one with a placeholder would publish invented data under
    // a field id that means something else.
    if let Some(f) = collection.value_fields().next() {
        return Err(no_mapping(domain, "variants", f.field_id));
    }
    Ok(Record::new(Vec::new()))
}

/// One 8-byte upgrade entry onto its stored `upgrades` Record.
///
/// **Raw, never scaled** (`world_fold.rs:23904`). See
/// [`record_from_variant_row`] for the shared contract.
pub fn record_from_upgrade_row(
    domain: Domain,
    collection: &Collection,
    entry: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    let _ = upgrade_index_of_row(domain, entry)?;
    // Handed the sibling `variants` collection, the field-id loop below would
    // iterate zero times and return an EMPTY record rather than an error — a
    // record of the wrong arity, published as if it were an upgrade row. A
    // collection with no value fields is the one shape that trap has, and this
    // domain has such a collection, so the id is checked before the loop.
    check_collection(domain, collection, UPGRADES, "upgrades")?;
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        values.push(match f.field_id {
            // `world_fold.rs:15571` reads the word at `+0` and `:23903` emits it
            // unchanged. No zero check and no bound: the fold applies neither.
            field::UPGRADE_INDEX => Value::Unsigned(u32_at(entry, 0) as u64),
            // `world_fold.rs:15572` reads the word at `+4` and `:23904` emits it
            // unchanged. Zero is a real level, not an empty state.
            field::LEVEL => Value::Unsigned(u32_at(entry, 4) as u64),
            other => return Err(no_mapping(domain, "upgrades", other)),
        });
    }
    Ok(Record::new(values))
}

/// A mapper fills exactly one collection, and says so before it reads a field.
fn check_collection(
    domain: Domain,
    collection: &Collection,
    expected: u32,
    what: &str,
) -> core::result::Result<(), String> {
    if collection.collection_id != expected {
        return Err(format!(
            "{} {what} mapper was handed collection {} (expected {expected})",
            domain.name, collection.collection_id
        ));
    }
    Ok(())
}

fn no_mapping(domain: Domain, collection: &str, field_id: u32) -> String {
    format!(
        "{} {collection} has no mapping for field id {field_id}",
        domain.name
    )
}

// --- The domain fold ---------------------------------------------------------

/// The accumulated state of both collections, in [`schema`] order.
///
/// A single record is not a state: the shipping stream re-states a variant only
/// when the collector sees it change (`attach_log.rs:54154`-`:54159`), so most
/// hours carry no rows at all and the hour's state is everything upserted since
/// the stream began. This lives beside the mapper for the same reason
/// [`split_payload`] does — the measurement harness, the tests and any future
/// caller must share one implementation of the accumulation rules, because a
/// re-derived fold drifts from the recorder without anything failing.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    collections: Vec<KeyedState>,
}

/// **Written out rather than derived.** A derived `Default` builds an *empty*
/// collection list, and every accessor indexes the two collections, so that value
/// would be a panic waiting for its first caller. `Default` is `new`.
impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            collections: vec![KeyedState::new(); COLLECTION_COUNT],
        }
    }

    /// The two collections in schema order — the shape `encode_baseline`,
    /// `encode_delta` and `decode_delta` take.
    pub fn as_slice(&self) -> &[KeyedState] {
        &self.collections
    }

    /// One collection by its schema index. `Option`, not an index: the argument
    /// comes from a caller rather than from this module, and a slice index would
    /// unwind on a number the recorder would simply not have.
    pub fn get(&self, index: usize) -> Option<&KeyedState> {
        self.collections.get(index)
    }

    pub fn variants(&self) -> &KeyedState {
        &self.collections[VARIANTS_INDEX]
    }

    pub fn upgrades(&self) -> &KeyedState {
        &self.collections[UPGRADES_INDEX]
    }

    pub fn rows(&self) -> usize {
        self.collections.iter().map(KeyedState::len).sum()
    }
}

/// Fold one DATA payload into `acc`, exactly as `fold_variant_upgrades` folds it
/// (`world_fold.rs:15567`-`:15583`).
///
/// **One mode.** There is no baseline/delta split to pass in and no clear: every
/// record upserts. A **failure-status** record folds nothing (`:15524`) and is
/// the caller's to exclude before calling this — see [`parse_header`] for why the
/// exclusion sits where it does.
///
/// **A variant's list is replaced whole.** `target.insert(variant_id, slice)`
/// (`:15582`) overwrites the previous `Vec` rather than merging into it, so a
/// re-stated variant carrying fewer entries *drops* the entries it no longer
/// carries. In the keyed-set projection that is a removal of the stale
/// `(variant_id, upgrade_index)` rows, and the only way this domain ever produces
/// a wire tombstone.
///
/// **Atomic.** Every row and every entry is decoded and checked before anything
/// is written, so a payload this function rejects leaves `acc` exactly as it
/// found it — the frame-level all-or-nothing the fold has by returning `Err`
/// before it mutates.
///
/// **Rows apply in payload order, last one winning.** The fold has no duplicate
/// rejection for `variant_id` (`:15580`-`:15583` is a plain per-row `insert`), so
/// a repeated variant resolves the way its map resolves it.
///
/// **Nothing is refused that the fold folds.** Children are keyed by their
/// position in the variant's range, so a `upgrade_index` repeated inside one
/// range is two rows and a range whose indices are out of order keeps its order —
/// both are payloads the fold accepts, and both reconstruct to its `Vec`
/// element for element.
pub fn fold_payload(
    domain: Domain,
    schema: &Schema,
    acc: &mut State,
    payload: &[u8],
    frame_tick: u64,
) -> core::result::Result<(), String> {
    if schema.collections.len() != COLLECTION_COUNT {
        return Err(format!(
            "{} schema has {} collections, expected {COLLECTION_COUNT}",
            domain.name,
            schema.collections.len()
        ));
    }
    // An accumulator that is not this domain's shape is a caller error, and every
    // line below indexes the two collections. Diagnosed, never a panic.
    if acc.collections.len() != COLLECTION_COUNT {
        return Err(format!(
            "{} accumulator has {} collections, expected {COLLECTION_COUNT}",
            domain.name,
            acc.collections.len()
        ));
    }
    let variants_c = &schema.collections[VARIANTS_INDEX];
    let upgrades_c = &schema.collections[UPGRADES_INDEX];
    let (header, tables) = split_payload(domain, payload)?;

    // ---- Decode and check everything first. ----
    //
    // `world_fold.rs:15567`-`:15574` decodes the whole entry array before it
    // applies a single range, so an entry no row addresses is still decoded.
    // Here every entry is length-checked and mapped for the same reason: the
    // arrays are the frame's, not the addressed slice's.
    let mut entries: Vec<Record> = Vec::with_capacity(header.emitted_upgrade_rows as usize);
    for entry in tables.upgrades.chunks_exact(ENTRY_ROW_LEN) {
        entries.push(record_from_upgrade_row(domain, upgrades_c, entry, frame_tick)?);
    }

    struct DecodedVariant {
        key: Key,
        variant_id: u32,
        record: Record,
        children: Vec<(Key, Record)>,
    }
    let mut decoded: Vec<DecodedVariant> = Vec::with_capacity(header.row_count as usize);
    for row in tables.variants.chunks_exact(ROW_LEN) {
        let (variant_id, range) = variant_row(domain, &header, row)?;
        let slice = entries.get(range.clone()).ok_or_else(|| {
            // Unreachable via `split_payload`, which range-checks against the same
            // count the array was decoded from; a `?` rather than an index so a
            // future caller pairing a range with the wrong array gets an error.
            format!(
                "{} range {}..{} is outside its {} entry table",
                domain.name,
                range.start,
                range.end,
                entries.len()
            )
        })?;
        // The ordinal is the entry's position in THIS variant's range, so it
        // restarts at 0 for every variant and is distinct within one — which is
        // what makes the child key total, with no duplicate case to refuse. The
        // range's length is bounded by `emitted_upgrade_rows`, a `u32`, so the
        // ordinal always fits one.
        let children: Vec<(Key, Record)> = slice
            .iter()
            .enumerate()
            .map(|(ordinal, record)| {
                (
                    key_from_upgrade_row(variant_id, ordinal as u32),
                    record.clone(),
                )
            })
            .collect();
        decoded.push(DecodedVariant {
            // Through the shared extractor, never a second hand-rolled copy of
            // "the parent key is the variant id" — the two would coincide today
            // and drift the first time either moves.
            key: key_from_variant_row(domain, row)?,
            variant_id,
            record: record_from_variant_row(domain, variants_c, row, frame_tick)?,
            children,
        });
    }

    // ---- Nothing below can fail, so `acc` is never left half-written. ----
    for variant in decoded {
        replace_children(
            &mut acc.collections[UPGRADES_INDEX],
            variant.variant_id,
            variant.children,
        );
        acc.collections[VARIANTS_INDEX].insert(variant.key, variant.record);
    }
    Ok(())
}

/// Replace every `(variant_id, *)` row with `entries`, mirroring
/// `target.insert(variant_id, …)` (`world_fold.rs:15582`): the variant's list is
/// carried whole by the row that owns it, so a row that carries the list carries
/// all of it and the previous list is gone.
fn replace_children(state: &mut KeyedState, variant_id: u32, entries: Vec<(Key, Record)>) {
    // `Key` orders lexicographically over its components, so this half-open range
    // is exactly this variant's children: the low bound is its own first child's
    // key, taken from the shared derivation rather than rebuilt, and the high
    // bound is the next variant's, which no child key can reach. `id + 1` cannot
    // overflow `i128`.
    let low = key_from_upgrade_row(variant_id, 0);
    let high = Key(vec![variant_id as i128 + 1, 0]);
    let stale: Vec<Key> = state.range(low..high).map(|(k, _)| k.clone()).collect();
    for key in stale {
        state.remove(&key);
    }
    for (key, record) in entries {
        state.insert(key, record);
    }
}

fn check_len(
    domain: Domain,
    slice: &[u8],
    expected: usize,
    what: &str,
) -> core::result::Result<(), String> {
    if slice.len() != expected {
        return Err(format!(
            "{} {what} length {} != {expected}",
            domain.name,
            slice.len()
        ));
    }
    Ok(())
}

/// Callers reach this only after a length check, so the slice is in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta};
    use crate::schema::Quant;
    use crate::TickGrid;

    /// Every test runs over **both** domains. The grammar is shared, so a test
    /// that ran over one would leave the other's binding unexercised — and the
    /// binding is the only thing that differs.
    fn domains() -> [Domain; 2] {
        [crate::d39::AIR_VARIANT_UPGRADES, crate::d40::STOCKPILE_VARIANT_UPGRADES]
    }

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn variants_c(domain: Domain) -> Collection {
        schema(domain).collections[VARIANTS_INDEX].clone()
    }

    fn upgrades_c(domain: Domain) -> Collection {
        schema(domain).collections[UPGRADES_INDEX].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// A raw 16-byte variant row, packed the way the collector packs one
    /// (`attach_log.rs:70596`-`:70601`).
    fn variant_row_bytes(variant_id: u32, flags: u32, first: u32, count: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        for v in [variant_id, flags, first, count] {
            row.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    /// A raw 8-byte upgrade entry (`attach_log.rs:70604`-`:70607`).
    fn entry_bytes(upgrade_index: u32, level: u32) -> Vec<u8> {
        let mut e = Vec::with_capacity(ENTRY_ROW_LEN);
        e.extend_from_slice(&upgrade_index.to_le_bytes());
        e.extend_from_slice(&level.to_le_bytes());
        assert_eq!(e.len(), ENTRY_ROW_LEN);
        e
    }

    /// A whole DATA payload, with the header words the writer writes.
    fn payload(status_flags: u32, rows: &[Vec<u8>], entries: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        out.extend_from_slice(&status_flags.to_le_bytes());
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // observed_variant_rows
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes()); // observed_upgrade_rows
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes()); // emitted_upgrade_rows
        out.extend_from_slice(&0u32.to_le_bytes()); // invalid_variant_rows
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved1
        assert_eq!(out.len(), HEADER_LEN);
        for row in rows {
            out.extend_from_slice(row);
        }
        for e in entries {
            out.extend_from_slice(e);
        }
        out
    }

    /// One variant with a dense upgrade list, the shape the collector emits.
    fn one_variant_payload(variant_id: u32, levels: &[u32]) -> Vec<u8> {
        let entries: Vec<Vec<u8>> = levels
            .iter()
            .enumerate()
            .map(|(i, lv)| entry_bytes(i as u32, *lv))
            .collect();
        payload(
            0,
            &[variant_row_bytes(variant_id, 0, 0, levels.len() as u32)],
            &entries,
        )
    }

    fn fold(domain: Domain, acc: &mut State, bytes: &[u8]) {
        fold_payload(domain, &schema(domain), acc, bytes, 0).expect("folds")
    }

    // --- Schema ------------------------------------------------------------

    #[test]
    fn the_schema_is_valid() {
        for domain in domains() {
            schema(domain)
                .validate(grid().log2())
                .unwrap_or_else(|e| panic!("{} schema is well-formed: {e:?}", domain.name));
            assert_eq!(schema(domain).domain, domain.id);
        }
    }

    #[test]
    fn the_two_domains_are_one_grammar() {
        // §11's canonical bytes cover the collections and their fields, not the
        // domain id, so the shared fold's byte-identical grammar shows up as
        // identical schema hashes. If a future edit gives one domain a field the
        // other lacks, this is what fails.
        let [air, stockpile] = domains();
        assert_ne!(air.id, stockpile.id);
        assert_eq!(
            schema(air).canonical_bytes(),
            schema(stockpile).canonical_bytes()
        );
        assert_eq!(schema(air).schema_hash(), schema(stockpile).schema_hash());
    }

    #[test]
    fn it_is_two_keyed_sets_keyed_by_variant_and_by_variant_and_index() {
        for domain in domains() {
            let v = variants_c(domain);
            assert_eq!(v.collection_id, VARIANTS);
            assert_eq!(v.shape, Shape::KeyedSet, "only a keyed set has deltas");
            let keys = v.key_fields();
            assert_eq!(keys.len(), 1, "variants are keyed by variant_id alone");
            assert_eq!(keys[0].field_id, field::VARIANT_ID);
            assert_eq!(
                keys[0].kind,
                Kind::Unsigned,
                "a variant id is a runtime instance identity, not content (§5)"
            );
            assert!(keys[0].kind.legal_as_key());
            assert_eq!(v.fields.len(), 1, "the key and nothing else");
            assert_eq!(
                v.value_fields().count(),
                0,
                "a variant row's non-key words are a zero flag word and transport"
            );

            let u = upgrades_c(domain);
            assert_eq!(u.collection_id, UPGRADES);
            assert_eq!(u.shape, Shape::KeyedSet);
            let keys = u.key_fields();
            assert_eq!(keys.len(), 2, "an upgrade is (variant_id, ordinal)");
            assert_eq!(keys[0].field_id, field::VARIANT_ID);
            assert_eq!(
                keys[1].field_id,
                field::UPGRADE_ORDINAL,
                "the key is the position in the fold's Vec, not the index word"
            );
            for k in &keys {
                assert_eq!(k.kind, Kind::Unsigned);
                assert!(k.kind.legal_as_key());
            }
            assert_eq!(u.fields.len(), 4, "2 key components + upgrade_index + level");
            let values: Vec<u32> = u.value_fields().map(|f| f.field_id).collect();
            assert_eq!(values, vec![field::UPGRADE_INDEX, field::LEVEL]);

            assert_eq!(schema(domain).collections.len(), COLLECTION_COUNT);
        }
    }

    #[test]
    fn no_state_capability_is_declared() {
        // The recipe's step-8 case 3, recorded with its reason. The row's `flags`
        // word must be EXACTLY zero (`world_fold.rs:15546`), so there is no gate
        // and no delete bit — no field can be Absent, Null or Unknown, and every
        // declared capability would cost real bits for a state the mapper can
        // never emit. Uncaptured is an hour-level state here (a failure status,
        // `:15524`), never a per-field one.
        for domain in domains() {
            for c in [variants_c(domain), upgrades_c(domain)] {
                assert_eq!(c.optional_count(), 0, "collection {}", c.collection_id);
                assert!(c.value_fields().all(|f| !f.nullable));
                assert!(c.value_fields().all(|f| !f.unknown_capable));
            }
        }
    }

    #[test]
    fn no_field_is_continuous_so_no_quantization_applies() {
        // Step 4's "quantization saves bytes" test does not apply: there is no
        // continuous field at all. Every state-bearing word is an integer the
        // fold emits raw (`world_fold.rs:23900`-`:23905`) — no scale, no measured
        // quantity, nothing a viewer draws or interpolates. Step 8's scale test
        // does not apply for the same reason, and
        // `every_field_is_emitted_raw_at_its_own_offset` pins what it would have.
        for domain in domains() {
            for c in [variants_c(domain), upgrades_c(domain)] {
                for f in c.fields.iter() {
                    assert!(!matches!(f.kind, Kind::Continuous), "field {}", f.field_id);
                    assert_eq!(f.quant, Quant::None, "field {}", f.field_id);
                }
            }
        }
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; a variant's upgrades move
        // when a player edits a design, so `frame_tick` never enters a value.
        for domain in domains() {
            for c in [variants_c(domain), upgrades_c(domain)] {
                assert!(c.fields.iter().all(|f| f.derived.is_none()));
            }
        }
    }

    // --- The mapper --------------------------------------------------------

    #[test]
    fn a_variant_row_maps_to_an_empty_record_that_costs_no_bytes() {
        for domain in domains() {
            let c = variants_c(domain);
            let record =
                record_from_variant_row(domain, &c, &variant_row_bytes(7, 0, 0, 0), 0)
                    .expect("maps");
            assert!(record.values.is_empty(), "the row's whole content is its key");
            let mut bytes = Vec::new();
            record.encode(&c, &mut bytes).expect("encodes");
            assert!(
                bytes.is_empty(),
                "a record with no value fields writes no bitmaps and no values"
            );
            let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
            assert_eq!(pos, 0, "consumes exactly its own zero bytes");
            assert_eq!(back, record);
            let mut reencoded = Vec::new();
            back.encode(&c, &mut reencoded).expect("re-encodes");
            assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
        }
    }

    #[test]
    fn an_upgrade_row_roundtrips_byte_for_byte() {
        for domain in domains() {
            let c = upgrades_c(domain);
            let record = record_from_upgrade_row(domain, &c, &entry_bytes(3, 13), 0).expect("maps");
            assert_eq!(record.values[index_of(&c, field::LEVEL)], Value::Unsigned(13));
            assert_eq!(
                record.values[index_of(&c, field::UPGRADE_INDEX)],
                Value::Unsigned(3)
            );
            let mut bytes = Vec::new();
            record.encode(&c, &mut bytes).expect("encodes");
            let (back, pos) = Record::decode(&c, &bytes, 0).expect("decodes");
            assert_eq!(pos, bytes.len(), "consumes exactly its own bytes");
            assert_eq!(back, record, "decode reproduces the record");
            let mut reencoded = Vec::new();
            back.encode(&c, &mut reencoded).expect("re-encodes");
            assert_eq!(reencoded, bytes, "decode/encode is byte-identical");
        }
    }

    #[test]
    fn a_real_zero_level_is_a_value_not_an_empty_state() {
        // 43 of the 228 d40 entries in the reference capture are level 0: a real
        // upgrade slot that has not been upgraded. §3 keeps zero out of the three
        // empty states, and this schema has no empty state to collapse it into.
        for domain in domains() {
            let c = upgrades_c(domain);
            let r = record_from_upgrade_row(domain, &c, &entry_bytes(0, 0), 0).expect("maps");
            assert_eq!(r.values[index_of(&c, field::LEVEL)], Value::Unsigned(0));
            assert_eq!(
                r.values[index_of(&c, field::UPGRADE_INDEX)],
                Value::Unsigned(0),
                "upgrade slot 0 is a real slot, not an empty state"
            );
        }
    }

    #[test]
    fn every_field_is_emitted_raw_at_its_own_offset() {
        // The domain has no scale factor, so step 8's scale test becomes the claim
        // it would otherwise verify: each word is read at its own offset and
        // mapped bit for bit, across the whole u32 range including the extremes. A
        // stray division or a transposed pair of adjacent words fails here.
        for domain in domains() {
            let c = upgrades_c(domain);
            for v in [0u32, 1, 255, 32_768, 100_000, 1_000_000_000, u32::MAX] {
                let index = v.wrapping_sub(1);
                let entry = entry_bytes(index, v);
                let r = record_from_upgrade_row(domain, &c, &entry, 0).expect("maps");
                assert_eq!(
                    r.values[index_of(&c, field::LEVEL)],
                    Value::Unsigned(v as u64),
                    "level is read at +4, raw"
                );
                assert_eq!(
                    r.values[index_of(&c, field::UPGRADE_INDEX)],
                    Value::Unsigned(index as u64),
                    "upgrade_index is read at +0, raw, and not swapped with level"
                );
                assert_eq!(
                    upgrade_index_of_row(domain, &entry).expect("reads"),
                    index,
                    "upgrade_index is read at +0"
                );
            }
            // And the child key is built from the parent and the ordinal, with
            // neither half read from the entry's own bytes.
            for (variant_id, ordinal) in [(1u32, 0u32), (7318, 3), (u32::MAX, u32::MAX)] {
                assert_eq!(
                    key_from_upgrade_row(variant_id, ordinal),
                    key(&[variant_id as u64, ordinal as u64])
                );
            }
            // And the variant row's four words: the key at +0, and the transport
            // pair at +8/+12 read in that order and not the other way round.
            let header = parse_header(domain, &payload(0, &[], &[])).expect("header");
            let mut h = header;
            h.emitted_upgrade_rows = 40;
            for id in [1u32, 7, 7318, 7363, u32::MAX] {
                let row = variant_row_bytes(id, 0, 10, 30);
                assert_eq!(
                    key_from_variant_row(domain, &row).expect("keys"),
                    key(&[id as u64])
                );
                assert_eq!(
                    variant_row(domain, &h, &row).expect("ranges"),
                    (id, 10..40),
                    "first is at +8 and count at +12"
                );
            }
        }
    }

    #[test]
    fn the_mapper_rejects_every_row_the_fold_rejects() {
        for domain in domains() {
            let v = variants_c(domain);
            let u = upgrades_c(domain);
            // Wrong length, both directions, including the empty slice.
            for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 48] {
                let row = vec![0u8; len];
                assert!(
                    record_from_variant_row(domain, &v, &row, 0).is_err(),
                    "variant row length {len} must be rejected, not read"
                );
                assert!(key_from_variant_row(domain, &row).is_err(), "length {len}");
            }
            for len in [0usize, 1, ENTRY_ROW_LEN - 1, ENTRY_ROW_LEN + 1, 16] {
                let entry = vec![0u8; len];
                assert!(
                    record_from_upgrade_row(domain, &u, &entry, 0).is_err(),
                    "upgrade entry length {len} must be rejected, not read"
                );
                assert!(upgrade_index_of_row(domain, &entry).is_err());
            }
            // Zero variant_id (`world_fold.rs:15542`).
            assert!(record_from_variant_row(domain, &v, &variant_row_bytes(0, 0, 0, 0), 0).is_err());
            assert!(key_from_variant_row(domain, &variant_row_bytes(0, 0, 0, 0)).is_err());
            // A nonzero flag word (`:15546`) — EVERY bit, one at a time, because
            // the fold compares against zero rather than masking known bits.
            for bit in 0..32u32 {
                let row = variant_row_bytes(1, 1 << bit, 0, 0);
                assert!(
                    record_from_variant_row(domain, &v, &row, 0).is_err(),
                    "flag bit {bit} is not zero and must be rejected"
                );
                assert!(key_from_variant_row(domain, &row).is_err());
            }
            // An upgrade entry has no content the fold rejects: no zero check on
            // `upgrade_index`, no bound on `level`. Adding either would refuse a
            // frame the recorder folds.
            assert!(record_from_upgrade_row(domain, &u, &entry_bytes(0, 0), 0).is_ok());
            assert!(record_from_upgrade_row(domain, &u, &entry_bytes(u32::MAX, u32::MAX), 0).is_ok());
        }
    }

    #[test]
    fn the_mappers_refuse_a_collection_they_cannot_fill() {
        // Including each other's: the two collections share a payload but not a
        // field-id space, so crossing them must be an error rather than a record
        // of the wrong arity. This matters more than usual for `variants`, whose
        // correct record is empty — a mapper that returned an empty record for
        // any collection would silently produce arity-0 rows for `upgrades`.
        for domain in domains() {
            let mut v = variants_c(domain);
            assert!(record_from_variant_row(domain, &upgrades_c(domain), &variant_row_bytes(1, 0, 0, 0), 0).is_err());
            assert!(record_from_upgrade_row(domain, &variants_c(domain), &entry_bytes(0, 1), 0).is_err());
            v.fields.push(Field::new(999, Kind::Unsigned));
            assert!(record_from_variant_row(domain, &v, &variant_row_bytes(1, 0, 0, 0), 0).is_err());
            let mut u = upgrades_c(domain);
            u.fields.push(Field::new(999, Kind::Unsigned));
            assert!(record_from_upgrade_row(domain, &u, &entry_bytes(0, 1), 0).is_err());
        }
    }

    // --- The payload header ------------------------------------------------

    #[test]
    fn parse_header_reads_every_word_and_checks_only_the_reserved_pair() {
        for domain in domains() {
            let p = one_variant_payload(7, &[1, 2]);
            let h = parse_header(domain, &p).expect("well-formed");
            assert_eq!(h.row_count, 1);
            assert_eq!(h.changed_rows, 1);
            assert_eq!(h.status_flags, 0);
            assert_eq!(h.observed_variant_rows, 1);
            assert_eq!(h.observed_upgrade_rows, 2);
            assert_eq!(h.emitted_upgrade_rows, 2);
            assert_eq!(h.invalid_variant_rows, 0);

            // Short of the header.
            for len in 0..HEADER_LEN {
                assert!(parse_header(domain, &p[..len]).is_err(), "len {len}");
            }
            // Either reserved word, any bit (`world_fold.rs:15521`).
            for offset in [36usize, 40] {
                for bit in [0u32, 7, 31] {
                    let mut bad = p.clone();
                    bad[offset..offset + 4].copy_from_slice(&(1u32 << bit).to_le_bytes());
                    assert!(
                        parse_header(domain, &bad).is_err(),
                        "reserved at +{offset} bit {bit}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_header_parse_and_the_length_check_are_split_at_the_folds_status_seam() {
        // The ordering property, and the reason `parse_header` exists.
        // `handle_compact_failure` (`world_fold.rs:15524`) returns Ok BEFORE the
        // payload-length check (`:15531`), so a record the recorder excludes on
        // status is never length-checked. A harness that called `split_payload`
        // on every record would refuse a stream the recorder folds; one that
        // excluded on status before reading the reserved words would accept a
        // frame the recorder refuses.
        for domain in domains() {
            let mut truncated = one_variant_payload(7, &[1, 2]);
            truncated.pop();
            assert!(
                parse_header(domain, &truncated).is_ok(),
                "the header is still readable, so the status word is still readable"
            );
            assert!(
                split_payload(domain, &truncated).is_err(),
                "the length check is the part that runs after the status check"
            );
            // And the reserved check is on the pre-status side: it fires even on a
            // payload whose length is also wrong.
            truncated[36..40].copy_from_slice(&1u32.to_le_bytes());
            assert!(parse_header(domain, &truncated).is_err());
        }
    }

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        for domain in domains() {
            let good = one_variant_payload(7, &[5, 6, 7]);
            let (header, tables) = split_payload(domain, &good).expect("well-formed");
            assert_eq!(header.row_count, 1);
            assert_eq!(header.emitted_upgrade_rows, 3);
            assert_eq!(tables.variants.len(), ROW_LEN);
            assert_eq!(tables.upgrades.len(), 3 * ENTRY_ROW_LEN);

            // A header-only payload with both counts zero is well-formed, and it
            // is what 141 of d40's 146 hours and 145 of d39's carry.
            let empty = payload(0, &[], &[]);
            assert_eq!(empty.len(), HEADER_LEN);
            let (h, t) = split_payload(domain, &empty).expect("an empty record is well-formed");
            assert_eq!((h.row_count, h.emitted_upgrade_rows), (0, 0));
            assert!(t.variants.is_empty() && t.upgrades.is_empty());

            // Any length that does not match the two counts, in both directions.
            let mut short = good.clone();
            short.pop();
            assert!(split_payload(domain, &short).is_err());
            let mut long = good.clone();
            long.push(0);
            assert!(split_payload(domain, &long).is_err());

            // Counts whose byte length overflows `usize` must be rejected, not
            // wrapped into a plausible small length. The fold saturates
            // (`:15527`-`:15530`), which also refuses, by a different route.
            for offset in [8usize, 28] {
                let mut huge = good.clone();
                huge[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
                assert!(split_payload(domain, &huge).is_err(), "count at +{offset}");
            }
            // And a count that is merely far too large for the payload.
            let mut wrong = good.clone();
            wrong[8..12].copy_from_slice(&1000u32.to_le_bytes());
            assert!(split_payload(domain, &wrong).is_err());

            // A range that leaves the entry array (`:15554`).
            let over = payload(
                0,
                &[variant_row_bytes(7, 0, 0, 4)],
                &[entry_bytes(0, 1), entry_bytes(1, 2), entry_bytes(2, 3)],
            );
            assert!(split_payload(domain, &over).is_err());
            // A range whose end overflows `usize` (`:15551`).
            let overflow = payload(
                0,
                &[variant_row_bytes(7, 0, u32::MAX, u32::MAX)],
                &[entry_bytes(0, 1)],
            );
            assert!(split_payload(domain, &overflow).is_err());
            // Coverage: declared != emitted (`:15562`), in both directions. The
            // payload length is kept consistent so it is the coverage rule and not
            // the length that fires.
            let under = payload(
                0,
                &[variant_row_bytes(7, 0, 0, 1)],
                &[entry_bytes(0, 1), entry_bytes(1, 2)],
            );
            assert!(split_payload(domain, &under).is_err(), "1 declared, 2 emitted");
            let twice = payload(
                0,
                &[variant_row_bytes(7, 0, 0, 2), variant_row_bytes(8, 0, 0, 2)],
                &[entry_bytes(0, 1), entry_bytes(1, 2)],
            );
            assert!(
                split_payload(domain, &twice).is_err(),
                "two variants sharing one slice declare 4 against 2 emitted"
            );
            // A row the fold rejects makes the whole payload invalid, even though
            // the payload's length and coverage are right.
            let zero_id = payload(0, &[variant_row_bytes(0, 0, 0, 0)], &[]);
            assert!(split_payload(domain, &zero_id).is_err());
            let flagged = payload(0, &[variant_row_bytes(1, 1, 0, 0)], &[]);
            assert!(split_payload(domain, &flagged).is_err());
        }
    }

    // --- The fold ----------------------------------------------------------

    #[test]
    fn a_record_upserts_and_never_clears() {
        // The whole shipping grammar: `world_fold.rs:15580`-`:15583` inserts the
        // rows it carries and touches nothing else. There is no baseline, so a
        // record carrying no rows leaves the accumulated catalog standing — which
        // is what 141 of d40's 146 hours do.
        for domain in domains() {
            let mut acc = State::new();
            fold(domain, &mut acc, &one_variant_payload(7, &[1, 2]));
            assert_eq!(acc.variants().len(), 1);
            assert_eq!(acc.upgrades().len(), 2);

            fold(domain, &mut acc, &payload(0, &[], &[]));
            assert_eq!(acc.variants().len(), 1, "a rowless record clears nothing");
            assert_eq!(acc.upgrades().len(), 2);

            fold(domain, &mut acc, &one_variant_payload(9, &[3]));
            assert_eq!(acc.variants().len(), 2, "a later record adds");
            assert_eq!(acc.upgrades().len(), 3);
            assert!(acc.variants().contains_key(&key(&[7])));
            let u = upgrades_c(domain);
            assert_eq!(
                acc.upgrades()[&key(&[7, 1])].values[index_of(&u, field::LEVEL)],
                Value::Unsigned(2),
                "the untouched variant keeps its children"
            );
        }
    }

    #[test]
    fn restating_a_variant_replaces_its_whole_list() {
        // `target.insert(variant_id, slice)` (`world_fold.rs:15582`) overwrites
        // the `Vec`, so a shorter list drops the entries it no longer carries.
        // This is the only removal in the whole domain, and the only source of a
        // wire tombstone.
        for domain in domains() {
            let u = upgrades_c(domain);
            let mut acc = State::new();
            fold(domain, &mut acc, &one_variant_payload(7, &[1, 2, 3, 4]));
            assert_eq!(acc.upgrades().len(), 4);

            fold(domain, &mut acc, &one_variant_payload(7, &[9, 8]));
            assert_eq!(acc.upgrades().len(), 2, "indexes 2 and 3 are gone");
            assert_eq!(
                acc.upgrades()[&key(&[7, 0])].values[index_of(&u, field::LEVEL)],
                Value::Unsigned(9)
            );
            assert_eq!(
                acc.upgrades()[&key(&[7, 1])].values[index_of(&u, field::LEVEL)],
                Value::Unsigned(8)
            );
            assert!(!acc.upgrades().contains_key(&key(&[7, 2])));
            assert_eq!(acc.variants().len(), 1, "the variant itself never leaves");

            // And an empty list empties the child set without removing the parent.
            fold(domain, &mut acc, &one_variant_payload(7, &[]));
            assert!(acc.upgrades().is_empty());
            assert_eq!(acc.variants().len(), 1);
        }
    }

    #[test]
    fn one_variants_replacement_does_not_touch_another() {
        // The `[id, 0] .. [id + 1, 0]` range in `replace_children` is exactly this
        // variant's children: a neighbouring id must survive untouched, including
        // the adjacent ones on both sides.
        for domain in domains() {
            let mut acc = State::new();
            fold(
                domain,
                &mut acc,
                &payload(
                    0,
                    &[
                        variant_row_bytes(6, 0, 0, 2),
                        variant_row_bytes(7, 0, 2, 2),
                        variant_row_bytes(8, 0, 4, 2),
                    ],
                    &[
                        entry_bytes(0, 60),
                        entry_bytes(1, 61),
                        entry_bytes(0, 70),
                        entry_bytes(1, 71),
                        entry_bytes(0, 80),
                        entry_bytes(1, 81),
                    ],
                ),
            );
            assert_eq!(acc.upgrades().len(), 6);
            fold(domain, &mut acc, &one_variant_payload(7, &[99]));
            assert_eq!(acc.upgrades().len(), 5);
            for (variant, index) in [(6u32, 0u64), (6, 1), (8, 0), (8, 1), (7, 0)] {
                assert!(
                    acc.upgrades().contains_key(&key(&[variant as u64, index])),
                    "({variant}, {index}) must survive"
                );
            }
            assert!(!acc.upgrades().contains_key(&key(&[7, 1])));
        }
    }

    #[test]
    fn a_variant_repeated_in_one_payload_is_last_writer_wins() {
        // `world_fold.rs:15580`-`:15583` is a plain per-row `insert` in payload
        // order, with no duplicate rejection. Refusing such a frame would refuse
        // one the shipping recorder folds.
        for domain in domains() {
            let u = upgrades_c(domain);
            let mut acc = State::new();
            fold(
                domain,
                &mut acc,
                &payload(
                    0,
                    &[variant_row_bytes(7, 0, 0, 2), variant_row_bytes(7, 0, 2, 1)],
                    &[entry_bytes(0, 1), entry_bytes(1, 2), entry_bytes(0, 42)],
                ),
            );
            assert_eq!(acc.variants().len(), 1);
            assert_eq!(acc.upgrades().len(), 1, "the last row's list wins whole");
            assert_eq!(
                acc.upgrades()[&key(&[7, 0])].values[index_of(&u, field::LEVEL)],
                Value::Unsigned(42)
            );
        }
    }

    #[test]
    fn two_variants_may_share_one_slice_of_the_entry_array() {
        // The fold range-checks each row independently and copies the slice
        // (`:15582`), so overlapping ranges are legal and each variant gets its
        // own copy. The collector never packs one, but the recorder folds it.
        for domain in domains() {
            let u = upgrades_c(domain);
            let mut acc = State::new();
            fold(
                domain,
                &mut acc,
                &payload(
                    0,
                    &[variant_row_bytes(7, 0, 0, 1), variant_row_bytes(8, 0, 0, 1)],
                    // Coverage counts declarations, so two rows declaring one
                    // entry each need two emitted entries; the second is
                    // addressed by nobody, which the fold also permits.
                    &[entry_bytes(0, 11), entry_bytes(0, 22)],
                ),
            );
            assert_eq!(acc.upgrades().len(), 2);
            for variant in [7u64, 8] {
                assert_eq!(
                    acc.upgrades()[&key(&[variant, 0])].values[index_of(&u, field::LEVEL)],
                    Value::Unsigned(11),
                    "both variants read entry 0"
                );
            }
        }
    }

    #[test]
    fn the_child_rows_reproduce_the_folds_vec_element_for_element() {
        // The property the ordinal key exists for: for EVERY payload the fold
        // accepts, reading a variant's rows in key order gives back exactly the
        // `Vec` the fold stored (`world_fold.rs:15582`). The two payloads that
        // would break an `upgrade_index`-keyed set are the interesting ones, and
        // the fold folds both.
        for domain in domains() {
            let u = upgrades_c(domain);
            let idx = index_of(&u, field::UPGRADE_INDEX);
            let lvl = index_of(&u, field::LEVEL);

            // (1) A repeated `upgrade_index` inside one range is two `Vec`
            // elements, so it is two rows. Keying by the index word would have
            // collapsed them and lost one.
            let mut acc = State::new();
            let dup = payload(
                0,
                &[variant_row_bytes(7, 0, 0, 2)],
                &[entry_bytes(3, 1), entry_bytes(3, 2)],
            );
            assert!(split_payload(domain, &dup).is_ok(), "the fold folds this");
            fold(domain, &mut acc, &dup);
            assert_eq!(acc.upgrades().len(), 2, "both entries survive");
            let rows: Vec<(u64, u64)> = acc
                .upgrades()
                .values()
                .map(|r| match (r.values[idx], r.values[lvl]) {
                    (Value::Unsigned(i), Value::Unsigned(l)) => (i, l),
                    other => panic!("unexpected values {other:?}"),
                })
                .collect();
            assert_eq!(rows, vec![(3, 1), (3, 2)], "in the fold's own order");

            // (2) A range whose indices descend keeps the fold's order, because
            // the keyed set sorts by ordinal. Keying by the index word would have
            // silently re-sorted it to (2, ..), (5, ..).
            let mut acc = State::new();
            fold(
                domain,
                &mut acc,
                &payload(
                    0,
                    &[variant_row_bytes(7, 0, 0, 2)],
                    &[entry_bytes(5, 50), entry_bytes(2, 20)],
                ),
            );
            let rows: Vec<(u64, u64)> = acc
                .upgrades()
                .values()
                .map(|r| match (r.values[idx], r.values[lvl]) {
                    (Value::Unsigned(i), Value::Unsigned(l)) => (i, l),
                    other => panic!("unexpected values {other:?}"),
                })
                .collect();
            assert_eq!(rows, vec![(5, 50), (2, 20)], "payload order, not sorted");
            assert_eq!(
                acc.upgrades().keys().cloned().collect::<Vec<_>>(),
                vec![key(&[7, 0]), key(&[7, 1])],
                "the keys are ordinals"
            );

            // (3) The ordinal restarts per variant, so two variants sharing an
            // index are two independent rows and nothing collides.
            let mut acc = State::new();
            fold(
                domain,
                &mut acc,
                &payload(
                    0,
                    &[variant_row_bytes(7, 0, 0, 1), variant_row_bytes(8, 0, 1, 1)],
                    &[entry_bytes(3, 1), entry_bytes(3, 2)],
                ),
            );
            assert_eq!(acc.upgrades().len(), 2);
            assert_eq!(
                acc.upgrades()[&key(&[8, 0])].values[lvl],
                Value::Unsigned(2),
                "variant 8's only child is at ordinal 0, not ordinal 1"
            );
        }
    }

    #[test]
    fn a_structural_error_leaves_the_accumulator_untouched() {
        for domain in domains() {
            let mut acc = State::new();
            fold(domain, &mut acc, &one_variant_payload(7, &[1, 2]));
            let before = acc.clone();
            let mut broken = one_variant_payload(9, &[3]);
            broken.pop();
            assert!(fold_payload(domain, &schema(domain), &mut acc, &broken, 0).is_err());
            assert_eq!(acc, before, "a refused frame changes nothing");

            // Including a frame whose FIRST row is fine and whose second is not:
            // everything is decoded before anything is written.
            let half_bad = payload(
                0,
                &[variant_row_bytes(9, 0, 0, 0), variant_row_bytes(0, 0, 0, 0)],
                &[],
            );
            assert!(fold_payload(domain, &schema(domain), &mut acc, &half_bad, 0).is_err());
            assert_eq!(acc, before);
        }
    }

    #[test]
    fn the_fold_refuses_a_schema_or_accumulator_of_the_wrong_shape() {
        for domain in domains() {
            let mut wrong_schema = schema(domain);
            wrong_schema.collections.pop();
            let mut acc = State::new();
            assert!(
                fold_payload(domain, &wrong_schema, &mut acc, &payload(0, &[], &[]), 0).is_err()
            );
            let mut short = State::new();
            short.collections.pop();
            assert!(
                fold_payload(domain, &schema(domain), &mut short, &payload(0, &[], &[]), 0).is_err()
            );
        }
    }

    // --- The wire layer ----------------------------------------------------

    #[test]
    fn the_wire_layer_carries_what_the_shipping_layer_cannot() {
        // The end-to-end property: three hours of shipping records fold into three
        // full states, and each wire BASELINE/DELTA round-trips through the
        // reader's own reconstructed prior — including a tombstone, which the
        // shipping grammar has no row for and which this domain produces only by
        // re-stating a variant with a shorter list.
        for domain in domains() {
            let schema = schema(domain);
            let (v, u) = (variants_c(domain), upgrades_c(domain));
            let mut acc = State::new();

            fold(
                domain,
                &mut acc,
                &payload(
                    0,
                    &[variant_row_bytes(7, 0, 0, 2), variant_row_bytes(9, 0, 2, 0)],
                    &[entry_bytes(0, 1), entry_bytes(1, 2)],
                ),
            );
            let hour0: Vec<KeyedState> = acc.as_slice().to_vec();

            let mut wire = Vec::new();
            encode_baseline(&schema, &hour0, &mut wire).expect("baseline encodes");
            let (decoded, consumed) = decode_baseline(&schema, &wire, 0).expect("baseline decodes");
            assert_eq!(consumed, wire.len());
            assert_eq!(decoded.len(), COLLECTION_COUNT);
            let mut reader = decoded;
            assert_eq!(reader, hour0, "the reader reconstructs the fold's own state");
            assert_eq!(
                reader[UPGRADES_INDEX][&key(&[7, 1])].values[index_of(&u, field::LEVEL)],
                Value::Unsigned(2)
            );

            // Hour 1: variant 7 shrinks to one upgrade (a tombstone) and a new
            // variant 11 appears with none (a changed row with an empty record).
            fold(domain, &mut acc, &one_variant_payload(7, &[5]));
            fold(domain, &mut acc, &one_variant_payload(11, &[]));
            let hour1: Vec<KeyedState> = acc.as_slice().to_vec();

            let dv = diff(&v, &hour0[VARIANTS_INDEX], &hour1[VARIANTS_INDEX]);
            let du = diff(&u, &hour0[UPGRADES_INDEX], &hour1[UPGRADES_INDEX]);
            assert_eq!(dv.changed.len(), 1, "variant 11 is new");
            assert!(dv.tombstones.is_empty(), "a variant key never leaves");
            assert_eq!(du.changed.len(), 1, "(7, 0) moved from 1 to 5");
            assert_eq!(du.tombstones, vec![key(&[7, 1])]);

            let mut delta_bytes = Vec::new();
            encode_delta(&schema, &hour0, &[Some(dv), Some(du)], &mut delta_bytes)
                .expect("delta encodes");
            let (deltas, consumed) =
                decode_delta(&schema, &reader, &delta_bytes, 0).expect("delta decodes");
            assert_eq!(consumed, delta_bytes.len());
            for (i, d) in deltas.iter().enumerate() {
                if let Some(d) = d {
                    apply(&mut reader[i], d);
                }
            }
            assert_eq!(reader, hour1, "the reader's state is the fold's state");

            // Hour 2: nothing at all. The wire grammar's UNCHANGED, which
            // `encode_delta` refuses to fake.
            fold(domain, &mut acc, &payload(0, &[], &[]));
            let hour2: Vec<KeyedState> = acc.as_slice().to_vec();
            assert_eq!(hour2, hour1);
            assert!(diff(&v, &hour1[VARIANTS_INDEX], &hour2[VARIANTS_INDEX]).is_empty());
            assert!(diff(&u, &hour1[UPGRADES_INDEX], &hour2[UPGRADES_INDEX]).is_empty());
            assert!(encode_delta(&schema, &hour1, &[None, None], &mut Vec::new()).is_err());
        }
    }

    #[test]
    fn an_hour_that_changes_one_collection_only_encodes_one_contribution() {
        // The collection change bitmap is what makes the key-only `variants`
        // collection cheap: an hour that only changes a level contributes nothing
        // for it. That is the common case — d39 never carries an upgrade entry at
        // all, and d40's variants roster moves in only 5 of 146 hours.
        for domain in domains() {
            let schema = schema(domain);
            let (v, u) = (variants_c(domain), upgrades_c(domain));
            let mut acc = State::new();
            fold(domain, &mut acc, &one_variant_payload(7, &[1]));
            let hour0: Vec<KeyedState> = acc.as_slice().to_vec();
            fold(domain, &mut acc, &one_variant_payload(7, &[2]));
            let hour1: Vec<KeyedState> = acc.as_slice().to_vec();

            let dv = diff(&v, &hour0[VARIANTS_INDEX], &hour1[VARIANTS_INDEX]);
            let du = diff(&u, &hour0[UPGRADES_INDEX], &hour1[UPGRADES_INDEX]);
            assert!(dv.is_empty(), "the variant roster did not move");
            assert!(!du.is_empty());
            let mut bytes = Vec::new();
            encode_delta(&schema, &hour0, &[None, Some(du)], &mut bytes).expect("delta encodes");
            let (deltas, consumed) = decode_delta(&schema, &hour0, &bytes, 0).expect("decodes");
            assert_eq!(consumed, bytes.len());
            assert!(deltas[VARIANTS_INDEX].is_none(), "the variants bit is clear");
            assert!(deltas[UPGRADES_INDEX].is_some());
            let mut reader = hour0.clone();
            apply(&mut reader[UPGRADES_INDEX], deltas[UPGRADES_INDEX].as_ref().unwrap());
            assert_eq!(reader, hour1);
        }
    }

    #[test]
    fn a_variants_only_state_still_round_trips() {
        // d39's whole reference capture: 204 variants, not one upgrade entry. The
        // `upgrades` collection is empty in every hour, and the baseline must
        // still encode and decode it as an empty collection rather than omitting
        // it and desynchronising the reader.
        for domain in domains() {
            let schema = schema(domain);
            let mut acc = State::new();
            let rows: Vec<Vec<u8>> = (1..=5u32).map(|id| variant_row_bytes(id, 0, 0, 0)).collect();
            fold(domain, &mut acc, &payload(0, &rows, &[]));
            assert_eq!(acc.variants().len(), 5);
            assert!(acc.upgrades().is_empty());
            let state: Vec<KeyedState> = acc.as_slice().to_vec();
            let mut wire = Vec::new();
            encode_baseline(&schema, &state, &mut wire).expect("encodes");
            let (decoded, consumed) = decode_baseline(&schema, &wire, 0).expect("decodes");
            assert_eq!(consumed, wire.len());
            assert_eq!(decoded, state);
        }
    }
}
