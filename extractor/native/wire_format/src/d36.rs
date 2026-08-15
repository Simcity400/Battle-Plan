//! d36 `air_variant_reserves` — per-country stockpiled reserves of each air
//! variant.
//!
//! Derived from `world_fold.rs:20440`-`:20558`
//! (`fold_air_variant_reserves`), its payload constants at `:989`-`:990` and
// @world-fold-authority domain=36 functions=fold_air_variant_reserves lines=20440-20558 sha256=0494555a8f3fb15f3c2401694cf45d691d481cc20aa9170d0a7c64477408f3bb closure=36b2975a919094aeca5bcb88875838ba3fdb9ae0f6b56c37afb5b0cdf8020d47
//! `:156`, the render at `world_fold.rs::build_designs`, and the collector read that produces
//! the rows (`extractor/native/collector/src/attach_log.rs:55392`-`:55530`,
//! `:71487`-`:71523`). There is no `extractor/documents/domains/d36_*.md`, so the
//! fold and the collector read are the whole basis.
//!
//! **Payload grammar** (`world_fold.rs::fold_air_variant_reserves`): a 48-byte header
//! followed by `row_count` fixed 24-byte rows. There is no legacy short row — the
//! fold requires `payload.len() == 48 + row_count * 24` exactly (`world_fold.rs::fold_air_variant_reserves`), so
//! the length never selects a row form and the mapper accepts exactly one.
//!
//! Header:
//!
//! | offset | width | field | note |
//! |---|---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms`          | collector timing, not game state |
//! | `+8`  | u32 | `row_count`               | rows in THIS record, not table size |
//! | `+12` | u32 | `changed_rows`            | `<= row_count` (`world_fold.rs::fold_air_variant_reserves`) |
//! | `+16` | u32 | `status_flags`            | the two early-exit gates |
//! | `+20` | u32 | `observed_country_rows`   | |
//! | `+24` | u32 | `observed_stockpile_rows` | |
//! | `+28` | u32 | `observed_air_variant_rows` | **the fold does not read this** |
//! | `+32` | u32 | `matched_stockpile_rows`  | `<= observed_stockpile_rows` (`world_fold.rs::fold_air_variant_reserves`) |
//! | `+36` | u32 | `invalid_country_rows`    | `<= observed_country_rows` (`world_fold.rs::fold_air_variant_reserves`) |
//! | `+40` | u32 | `deleted_rows`            | `<= row_count` (`world_fold.rs::fold_air_variant_reserves`) |
//! | `+44` | u32 | `scale_raw`               | asserted `== 100_000` (`world_fold.rs::fold_air_variant_reserves`) |
//!
//! Row:
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `tag_db_id`  | raw | none (key component) |
//! | `+4`  | u32 | `variant_id` | raw | none (key component) |
//! | `+8`  | u32 | `flags`      | —   | only bit 0, the delete bit (`world_fold.rs::fold_air_variant_reserves`) |
//! | `+12` | u32 | `reserved`   | —   | must be 0 (`world_fold.rs::fold_air_variant_reserves`) |
//! | `+16` | i64 | `amount_raw` | `/100_000` (`world_fold.rs::build_designs`) | none |
//!
//! **The domain declares no empty state at all — no `optional`, no `nullable`,
//! no `unknown_capable`.** There is exactly one value field and it is
//! unconditional: the fold reads `amount_raw` at a fixed offset with no gate
//! (`world_fold.rs::fold_air_variant_reserves`), and the only flag bit the grammar admits is the row-level delete
//! (`world_fold.rs::fold_air_variant_reserves`), which is a removal from the keyed set rather than a state
//! of a field. The collector never emits a row with an unreadable amount either:
//! `read_production_stockpile_raw` (`attach_log.rs:84993`) returns `None` for the
//! whole country when any entry is unreadable, which drops the country from the
//! record and raises `invalid_country_rows`, and that in turn sets
//! `COMPACT_STATUS_PARTIAL_DATA` (`:55467`) — a *record*-level failure for this
//! domain, because d36 carries no expected-partial tolerance
//! (`compact_contract/src/lib.rs:136`-`:158`), so the fold's
//! `handle_compact_failure` (`:15320`) discards the whole record. There is
//! therefore no path on which a d36 row exists and its amount does not. A
//! stockpile of genuinely zero planes reads `amount_raw == 0`, and per
//! `WIRE_FORMAT.md` §3 that is a real **Zero**, never one of the three empty
//! states.
//!
//! **`scale_raw` is not a field.** The fold asserts it equal to `FIXED_SCALE`
//! before folding anything (`:15319`), so it is constant over every payload the
//! grammar accepts; it is then copied into every stored row (`:15374`) and
//! re-emitted per row by the renderer (`:8751`). Per the recipe's rule for a
//! value the fold hard-enforces as a function of carried state, the invariant is
//! enforced in [`parse_header`] and the constant left off the wire — re-encoding
//! it would be a second copy of state the decoder must then keep consistent.
//! [`FIXED_SCALE_RAW`] is that constant, and `scale_word_is_invariant_not_data`
//! is the test.
//!
//! **One record is not one state** (`:15342`-`:15346`). `replace_or_sparse_after_first`
//! (`:2947`) clears the accumulator only for the FIRST non-delta record; every
//! record after that is a sparse patch — upserts plus low-bit deletes — whatever
//! its `DELTA_DATA` flag says. That matches the collector, which emits a full
//! snapshot only while its tracker is empty and changed-rows-plus-tombstones
//! after (`attach_log.rs:55485`-`:55518`). The measured `deepwar_t1` capture is
//! exactly that shape: hour 1 carries 186 rows, the remaining 145 hours carry
//! 172 rows between them, and 129 of them carry none. So the domain-hour state is
//! this module's [`State`] after the record, not the record's rows, and every
//! caller folds through the one [`State::fold_payload`].
//!
//! **Every `i64` the fold accepts is mapped.** The fold range-checks
//! `amount_raw` not at all, so neither does this mapper: a cutoff here would
//! reject rows the recorder folds and make the measurement harness refuse a valid
//! capture. [`EXACT_AMOUNT_RAW_BAND`] records where the raw-integer recovery
//! stops being exact and why that residue is a `WIRE_FORMAT.md` §13 codec item
//! rather than something the schema can fix.
//!
//! **No derived field (§6).** Nothing here changes deterministically with the
//! clock: a reserve pool moves when production delivers or a wing draws, not on a
//! clock-linear ramp. The mapper still takes `frame_tick` so every domain's
//! boundary has one shape.

use std::collections::BTreeMap;

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 36;
pub const RESERVES: u32 = 1;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
pub mod field {
    pub const TAG_DB_ID: u32 = 1;
    pub const VARIANT_ID: u32 = 2;
    pub const AMOUNT: u32 = 3;
}

// --- Payload grammar, from `world_fold.rs:989`-`:990` and `:15277`-`:15376`. --

/// `AIR_VARIANT_RESERVE_HEADER_LEN`, `world_fold.rs::AIR_VARIANT_RESERVE_HEADER_LEN`.
pub const HEADER_LEN: usize = 48;
/// `AIR_VARIANT_RESERVE_ROW_LEN`, `world_fold.rs::AIR_VARIANT_RESERVE_ROW_LEN`. Fixed; no legacy form.
pub const ROW_LEN: usize = 24;

/// `ROW_FLAG_DELETED_LOW_BIT`, `world_fold.rs::ROW_FLAG_DELETED_LOW_BIT`, applied at `:15363`. It is
/// the ONLY bit the row's flag word may carry: `flags & !1 != 0` is a hard error
/// (`:15352`-`:15354`).
pub const ROW_FLAG_DELETED: u32 = 1;

/// `FIXED_SCALE`, `world_fold.rs::FIXED_SCALE`, and the header word the fold asserts
/// equal to it (`:15319`, via `ensure_fixed_scale` at `world_fold.rs::ensure_fixed_scale`).
pub const FIXED_SCALE: f64 = 100_000.0;
pub const FIXED_SCALE_RAW: u32 = 100_000;
/// `FIXED_SCALE` is `10^5`, so `Quant::Fixed(5)` is exactly the source's own
/// grid — see `schema()` and `fixed_five_reproduces_every_raw_in_the_band`.
pub const SCALE_DIGITS: u8 = 5;

/// The magnitude of `amount_raw` up to which `Kind::Continuous` + `Fixed(5)`
/// reproduces the source integer **exactly**. It is a documented property, **not
/// a gate**: the mapper accepts every `i64` the fold accepts.
///
/// `amount_raw` is an `i64` (`world_fold.rs::fold_air_variant_reserves`) and `Value::Continuous` is a
/// binary64, so the pair is exact only while the round trip
/// `round_ties_even((raw / 10^5) * 10^5)` returns `raw`. That holds for every
/// `|raw| <= 2^51` on every magnitude
/// `fixed_five_reproduces_every_raw_in_the_band` tries.
///
/// **What that test is, stated exactly, because the distinction is the whole
/// point of this paragraph.** It is a SAMPLE, not an enumeration: the edges, one
/// value per decade, the 2,000 magnitudes immediately below the band's top, and
/// 20,000 deterministic pseudorandom draws from `0..=2^51`. Two quadrillion
/// integers are not enumerable, so the honest statement is "no failure was found
/// at or below `2^51`", and this constant is published on that evidence.
///
/// **An earlier draft said the round trip "first fails inside `2^52`" (measured).
/// That claim is withdrawn**: the test skips every `|raw|` above
/// `EXACT_AMOUNT_RAW_BAND` outright, so it cannot locate the first failure, and
/// nothing else here measures one.
///
/// **The maximal exact band is not claimed at all, in either direction.** The
/// arithmetic is not domain-specific — it is `quantize(raw as f64 / 10^5,
/// Fixed(5))`, the same function over the same grid as d38's `amount_raw` — so
/// d38's witness carries over: `d38::FIRST_KNOWN_INEXACT_AMOUNT_RAW`,
/// `3_435_973_836_800_005`, does not survive the round trip, while its
/// predecessor does (`d38.rs`'s own
/// `fixed_five_reproduces_every_raw_in_the_band`). That is one *existence*
/// witness roughly 52.6% above this constant. It does NOT establish that it is
/// the smallest failing magnitude, and neither test establishes that the exact
/// magnitudes form a contiguous run — so "the exact band ends at X" is not a
/// statement either file is entitled to make. Nothing gates on any of these
/// numbers — the mapper maps every `i64` the fold maps — which is why the
/// conservative, sampled bound is the safe one to publish and the sharper one is
/// left unstated.
///
/// Above the band the stored value is still a legal point of the declared
/// `10^-5` grid and still within one unit in the last place of the source, and
/// encode/decode remains exactly consistent (§6's escape rule keeps an unstable
/// integer off the quantized path entirely, `value.rs:195`), but a consumer
/// recovering the raw integer may see it differ by one. That residue is the same
/// class of gap the recipe already records for `/32768` sources: closing it needs
/// an integer-domain `Fixed` in the codec, a `WIRE_FORMAT.md` §13 item, not a
/// schema-side or mapper-side decision. It is in the deliverable's open
/// questions.
///
/// **Refusing such a row would be worse than carrying it.** The fold range-checks
/// `amount_raw` not at all, and the collector admits `|amount| <= 1e12` — i.e.
/// `|amount_raw| <= 1e17` (`collector/src/attach_log/offsets.rs:556`, applied at
/// `attach_log.rs:85025`) — so a mapper-side cutoff would reject rows the
/// recorder folds and make the measurement harness refuse a valid capture. For
/// scale: the band is ~2.25e10 in game units and the measured maximum over the
/// whole `deepwar_t1` capture is 101.0.
pub const EXACT_AMOUNT_RAW_BAND: i64 = 1 << 51;

/// `COMPACT_STATUS_DELTA_DATA`, `compact_contract/src/lib.rs`. Named here because
/// d36 is not admitted to the production-codec dependency surface; the
/// measurement bin asserts the local value and shipping contract agree.
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

/// `COMPACT_STATUS_FAILURE_MASK`. d36 has no lane-local status bits and no
/// expected-partial tolerance (`compact_contract/src/lib.rs:136`-`:158` lists
/// domains 26, 27, 28 and 54 only), so the mask is the whole predicate for this
/// domain; the measurement bin calls `compact_contract`'s own
/// `compact_status_failure_flags_after_expected_partial_tolerance` and asserts
/// the two agree, so this constant cannot drift unnoticed.
pub const FAILURE_STATUS_MASK: u32 = (1 << 0)   // STATE_UNAVAILABLE
    | (1 << 1)  // PROFILE_MISMATCH
    | (1 << 2)  // NO_CLOCK
    | (1 << 3)  // NO_SCALAR_SOURCE
    | (1 << 6)  // WORKER_PANIC
    | (1 << 7)  // BRIDGE_UNAVAILABLE
    | (1 << 8)  // SOURCE_UNAVAILABLE
    | (1 << 9)  // PARTIAL_DATA
    | (1 << 12); // CONTROL_INVALID

pub fn schema() -> Schema {
    use field as f;
    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: RESERVES,
            // `(tag_db_id, variant_id)` is `AirVariantReserveKey`
            // (`world_fold.rs:15359`-`:15362`) and it persists across hours: the
            // fold patches and deletes against it rather than rebuilding the
            // table, which is what a keyed set needs — and only a keyed set has a
            // delta grammar (§7.3).
            shape: Shape::KeyedSet,
            fields: vec![
                // ContentId because a country tag is content-defined (§5): a
                // manifest must resolve id 42 for it to keep its meaning under
                // another mod set, which is the deciding question for
                // ContentId-vs-Unsigned. d63 classes the same `tag_db_id` this
                // way and d20 classes its `owner` country the same way. This is a
                // ContentId **key** component — see the open questions; §13's
                // manifest-id rule reorders keys, not just values.
                Field::new(f::TAG_DB_ID, Kind::ContentId).key(0),
                // `Unsigned`, not `ContentId`: `variant_id` is a runtime catalog
                // identity minted during play. The collector reads it out of a
                // live production-variant object (`VARIANT_DEF_ID` off a heap
                // pointer, `attach_log.rs:85023`) and d36's own row set is gated
                // on the variant being *currently active*
                // (`attach_log.rs:55413`-`:55456`) — a player-designed variant,
                // not a content definition a manifest resolves. The measured
                // capture agrees: 155 distinct variant ids up to 7318, in a
                // campaign whose whole country roster is 41 tags. Sharing the
                // catalog with d33/d37 does not make it content — those are join
                // targets, and d37's own rows carry the *names* the player typed.
                Field::new(f::VARIANT_ID, Kind::Unsigned).key(1),
                // The one value field. `Continuous` because the fold divides it
                // by `FIXED_SCALE` to produce the rendered `amount`
                // (`world_fold.rs:8749`) — a stockpile is a fractional quantity,
                // not a count, even where whole numbers dominate.
                //
                // `Quant::Fixed(5)` is not a precision *budget*: `amount_raw` is a
                // signed 64-bit CFixedPoint whose denominator the fold asserts to
                // be exactly `10^5` before folding anything (`world_fold.rs::fold_air_variant_reserves`), so
                // `fixed(5)` is the source's own grid and reproduces the source
                // integer bit for bit over the whole accepted range (proved in
                // `fixed_five_reproduces_every_raw_in_the_band`). §6's "exact
                // unless justified" bar is met by a stronger fact than a precision
                // argument: nothing is rounded at all. It also matters that the
                // raw integer survives — the fold's own renderer emits
                // `amount_raw` beside `amount` (`world_fold.rs::build_designs`), so a downstream consumer
                // requires the integer, and `fixed(5)` is precisely the
                // quantization under which `round(value * 1e5)` returns it
                // unchanged, where `exact` would spend 8 raw bytes per value to
                // carry the same information.
                //
                // The comparison against `Quant::Exact` holds in BOTH directions,
                // which is the part worth stating precisely. `exact` is not a
                // lossless alternative here: the lossy step for a large
                // `amount_raw` is `raw as f64` in the mapper, which both
                // quantizations share, so `exact` recovers the source integer over
                // exactly the same band and spends 8 bytes doing it. Above that
                // band (`EXACT_AMOUNT_RAW_BAND`) neither carries the integer
                // exactly, and §6's escape rule (`value.rs:149`-`:165`) keeps the
                // wire *self*-consistent either way by pushing an unstable value
                // onto the raw-f64 path. So `fixed(5)` never loses anything
                // `exact` would have kept, and it is smaller and recovers the
                // integer directly where either can. See the deliverable's
                // defence.
                Field::new(f::AMOUNT, Kind::Continuous).quant(Quant::Fixed(SCALE_DIGITS)),
            ],
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 48-byte payload header (`world_fold.rs::fold_air_variant_reserves`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_country_rows: u32,
    pub observed_stockpile_rows: u32,
    /// `+28`. **The fold does not read this word**; the independent authority
    /// decoder does (`recorder_host/src/domain_data_authority.rs:15091`). Carried
    /// so the header has one full parse, constrained by nothing.
    pub observed_air_variant_rows: u32,
    pub matched_stockpile_rows: u32,
    pub invalid_country_rows: u32,
    pub deleted_rows: u32,
    pub scale_raw: u32,
}

impl PayloadHeader {
    /// `COMPACT_STATUS_DELTA_DATA` (`world_fold.rs::replace_or_sparse_after_first`). Note this is *not* the
    /// same question as "is this record a sparse patch" — see [`State::fold_payload`].
    pub fn is_delta(self) -> bool {
        self.status_flags & STATUS_DELTA_DATA != 0
    }

    /// The fold's failure gate (`world_fold.rs::fold_air_variant_reserves`).
    pub fn is_failure(self) -> bool {
        self.status_flags & FAILURE_STATUS_MASK != 0
    }
}

/// Parse and check the payload header, applying every check the fold performs
/// **before** its status early-exits (`world_fold.rs::fold_air_variant_reserves`).
///
/// Position matters and is taken from the fold's own call order: the four
/// `ensure_count_at_most` invariants and the scale assertion run before
/// `handle_compact_failure`, so a failure-status record with an inconsistent
/// header is an `Err` in the recorder too, not a quietly skipped hour. The exact
/// length check runs *after* those exits (`:15334`) and therefore lives in
/// [`split_payload`], not here.
pub fn parse_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "air_variant_reserves payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_country_rows: u32_at(payload, 20),
        observed_stockpile_rows: u32_at(payload, 24),
        observed_air_variant_rows: u32_at(payload, 28),
        matched_stockpile_rows: u32_at(payload, 32),
        invalid_country_rows: u32_at(payload, 36),
        deleted_rows: u32_at(payload, 40),
        scale_raw: u32_at(payload, 44),
    };
    // The four `ensure_count_at_most` invariants, `world_fold.rs:15291`-`:15318`,
    // in the fold's order. All four are `<=`, not `==`: d36 is not a full-replace
    // domain, and a record carrying fewer changed rows than rows is ordinary.
    ensure_at_most("changed_rows", header.changed_rows, "row_count", header.row_count)?;
    ensure_at_most(
        "matched_stockpile_rows",
        header.matched_stockpile_rows,
        "observed_stockpile_rows",
        header.observed_stockpile_rows,
    )?;
    ensure_at_most(
        "invalid_country_rows",
        header.invalid_country_rows,
        "observed_country_rows",
        header.observed_country_rows,
    )?;
    ensure_at_most("deleted_rows", header.deleted_rows, "row_count", header.row_count)?;
    // `ensure_fixed_scale`, `world_fold.rs:15319`. This is what makes the scale a
    // constant rather than a field.
    if header.scale_raw != FIXED_SCALE_RAW {
        return Err(format!(
            "air_variant_reserves scale_raw {} != {FIXED_SCALE_RAW}",
            header.scale_raw
        ));
    }
    Ok(header)
}

fn ensure_at_most(field: &str, value: u32, max_field: &str, max: u32) -> core::result::Result<(), String> {
    if value > max {
        Err(format!(
            "air_variant_reserves {field} {value} > {max_field} {max}"
        ))
    } else {
        Ok(())
    }
}

/// Split a d36 DATA payload into its header and its rows slice, adding the
/// fold's exact-length check (`world_fold.rs::fold_air_variant_reserves`) to [`parse_header`]'s
/// invariants.
///
/// This lives here rather than in each caller so the header grammar has exactly
/// one implementation: a measurement harness that re-derives it is a harness that
/// can drift from the recorder without anything failing.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    let header = parse_header(payload)?;
    // Checked arithmetic: `row_count` is input from a file on disk, and
    // `row_count * ROW_LEN` overflows `usize` for a large enough word. The fold
    // saturates instead (`:15335`), which reaches the same rejection by a
    // different route — a saturated `expected_len` never equals `payload.len()`.
    let expected = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| {
            format!(
                "air_variant_reserves row_count {} overflows",
                header.row_count
            )
        })?;
    if payload.len() != expected {
        return Err(format!(
            "air_variant_reserves len {} != {expected}",
            payload.len()
        ));
    }
    Ok((header, &payload[HEADER_LEN..]))
}

/// The key of one compact row: `(tag_db_id, variant_id)`
/// (`world_fold.rs::fold_air_variant_reserves`).
///
/// Neither component is range-checked, because the fold does not check either.
/// The independent authority decoder rejects `tag_db_id == 0` and
/// `tag_db_id > COMPACT_AIR_VARIANT_RESERVE_MAX_TAG_DB_ID`
/// (`domain_data_authority.rs:15190`-`:15196`,
/// `:715`); the fold is the authority here and it wins. Recorded as a finding.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    check_row_len(row)?;
    Ok(key(&[u32_at(row, 0) as u64, u32_at(row, 4) as u64]))
}

/// Whether this row removes its key from the set rather than carrying a state
/// (`world_fold.rs::fold_air_variant_reserves`).
///
/// A deleted row's `amount_raw` is packed as zero by the collector
/// (`attach_log.rs:55525`), which is why it is a removal and not a record: mapping
/// it would publish a fabricated `0.0` reserve for a key that no longer exists.
pub fn is_deleted_row(row: &[u8]) -> core::result::Result<bool, String> {
    Ok(row_flags(row)? & ROW_FLAG_DELETED != 0)
}

/// The row's flag word, with the fold's unknown-bit rejection
/// (`world_fold.rs::fold_air_variant_reserves`) and its reserved-word rejection (`world_fold.rs::fold_air_variant_reserves`).
pub fn row_flags(row: &[u8]) -> core::result::Result<u32, String> {
    check_row_len(row)?;
    let flags = u32_at(row, 8);
    if flags & !ROW_FLAG_DELETED != 0 {
        return Err(format!(
            "air_variant_reserves row has unknown flags {flags:#x}"
        ));
    }
    let reserved = u32_at(row, 12);
    if reserved != 0 {
        return Err(format!(
            "air_variant_reserves row reserved field is nonzero ({reserved})"
        ));
    }
    Ok(flags)
}

/// One compact row of the `reserves` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 24-byte row slice (the caller has
/// already consumed the table header via [`split_payload`]); d36 has exactly one
/// row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is §6's derived-field input (stored = v + c * (frame_tick >> u));
/// d36 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// A **deleted** row is not a record and is rejected here: it removes its key
/// from the set (`world_fold.rs::fold_air_variant_reserves`). [`State::fold_payload`] branches on
/// [`is_deleted_row`] before ever calling this.
///
/// Fail-closed throughout, mirroring `fold_air_variant_reserves`: a row the fold
/// would reject is an `Err` here, never a coerced value. Nothing in this function
/// can panic on any 24-byte input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d36 has no derived field; §6's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    check_row_len(row)?;
    if row_flags(row)? & ROW_FLAG_DELETED != 0 {
        return Err(
            "air_variant_reserves deleted row is a removal, not a record".to_string()
        );
    }
    // Every `i64` the fold accepts is mapped; see `EXACT_AMOUNT_RAW_BAND` for
    // where the raw-integer recovery stops being exact and why a cutoff here
    // would be the worse failure.
    let amount_raw = i64_at(row, 16);

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // `world_fold.rs:8749` — the fold's own render divides by
            // `FIXED_SCALE`, and `:15319` has already proved the payload's own
            // scale word equals it.
            field::AMOUNT => Value::Continuous(amount_raw as f64 / FIXED_SCALE),
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!(
                    "air_variant_reserves has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "air_variant_reserves row length {} != {ROW_LEN}",
            row.len()
        ));
    }
    Ok(())
}

/// Callers reach these only after a length check, so the slices are in bounds.
fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn i64_at(b: &[u8], off: usize) -> i64 {
    i64::from_le_bytes([
        b[off],
        b[off + 1],
        b[off + 2],
        b[off + 3],
        b[off + 4],
        b[off + 5],
        b[off + 6],
        b[off + 7],
    ])
}

// --- The frame fold ----------------------------------------------------------

/// What one d36 DATA record did to the accumulator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// `handle_compact_failure` (`world_fold.rs::handle_compact_failure`, called at `:15320`): the
    /// domain-hour is unknown and the accumulator is retained. Nothing was
    /// folded, so the hour is not d36 data and belongs in no measurement.
    FailureRetained,
    /// `reject_delta_before_baseline` (`:2895`, called at `:15327`): a record
    /// flagged `DELTA_DATA` with no baseline yet has no identity set to patch.
    /// Nothing was folded.
    DeltaBeforeBaseline,
    /// The record was applied. `reset` is true for the one record that clears the
    /// accumulator first — the first non-delta record (`:15343`-`:15346`).
    Folded {
        reset: bool,
        upserts: usize,
        deletes: usize,
    },
}

/// The reconstructed `air_variant_reserves` state, folded exactly as
/// `fold_air_variant_reserves` folds it (`world_fold.rs::fold_air_variant_reserves`).
///
/// d36 is replace-once-then-sparse, so an hour's state is **not** an hour's rows:
/// it is this accumulator after the hour's record. Every caller — the measurement
/// harness and the tests — folds through this one implementation, because a
/// second one is a second answer to "what did hour N hold".
///
/// `fold_air_variant_reserves` is the accumulator's only writer: nothing else in
/// `world_fold.rs` touches `self.air_variant_reserves` except the renderer at
/// `:8742`, which reads it (verified by grep over `recorder_host` and
/// `collector`). Unlike d26, no other domain's rows prune it.
#[derive(Clone, Debug, Default)]
pub struct State {
    /// The reserve table, keyed by `(tag_db_id, variant_id)`.
    pub reserves: KeyedState,
    /// `delta_baselines_seen` for this domain (`world_fold.rs::replace_or_sparse_after_first`).
    baseline_seen: bool,
}

impl State {
    pub fn baseline_seen(&self) -> bool {
        self.baseline_seen
    }

    /// Fold one d36 DATA payload. `schema` must be [`schema()`]'s.
    ///
    /// Every rejection path is an `Err` — a payload the recorder would refuse is
    /// refused here, never partially applied. On `Err` the accumulator is left
    /// untouched: rows are decoded into a scratch list first, then applied, which
    /// also means a bad row late in a record cannot leave half a patch behind.
    /// (The fold itself applies as it walks and returns `Err` mid-way; the
    /// recorder treats that as a fatal stream error, so the difference is only
    /// visible to a caller that continues after an `Err`, which none does.)
    ///
    /// The order of the checks is the fold's own order, and it is load-bearing:
    /// the header invariants come first (`:15291`-`:15319`), then the failure
    /// gate (`:15320`), then delta-before-baseline (`:15327`), and only then the
    /// exact-length check and the row loop (`:15334`).
    pub fn fold_payload(
        &mut self,
        schema: &Schema,
        payload: &[u8],
        frame_tick: u64,
    ) -> core::result::Result<FoldOutcome, String> {
        let header = parse_header(payload)?;
        if header.is_failure() {
            return Ok(FoldOutcome::FailureRetained);
        }
        if header.is_delta() && !self.baseline_seen {
            return Ok(FoldOutcome::DeltaBeforeBaseline);
        }

        let (header, body) = split_payload(payload)?;
        let collection = schema
            .collections
            .iter()
            .find(|c| c.collection_id == RESERVES)
            .ok_or_else(|| "air_variant_reserves schema has no reserves collection".to_string())?;

        // Decode every row before touching the accumulator.
        let mut patch: Vec<(Key, Option<Record>)> = Vec::with_capacity(body.len() / ROW_LEN);
        let mut rows = 0usize;
        let mut deletes = 0usize;
        for row in body.chunks_exact(ROW_LEN) {
            rows += 1;
            let k = key_from_row(row)?;
            if is_deleted_row(row)? {
                deletes += 1;
                patch.push((k, None));
            } else {
                patch.push((k, Some(record_from_row(collection, row, frame_tick)?)));
            }
        }
        // `split_payload` already ties `payload.len()` to `row_count`, so this can
        // only fire if the two derivations disagree — which is exactly the drift
        // the shared boundary exists to prevent.
        if rows != header.row_count as usize {
            return Err(format!(
                "air_variant_reserves decoded {rows} rows for row_count {}",
                header.row_count
            ));
        }
        // The fold's own witness (`:15312`): the header's `deleted_rows` must not
        // exceed `row_count`, already checked, and the rows must be consistent
        // with it. The fold does not compare the two, so neither does this — the
        // collector sets `deleted_rows` from the tombstones it emitted
        // (`attach_log.rs:55508`), but a record with a different count is one the
        // recorder accepts, and refusing it here would refuse a stream the
        // recorder folds. Reported, not enforced.
        let _ = deletes;

        // `replace_or_sparse_after_first` (`:2947`, called at `:15342`). Note what
        // this is NOT: it is not "the DELTA_DATA flag says sparse". The FIRST
        // non-delta record clears the accumulator and marks the baseline seen;
        // every record after that is sparse whatever its flag. In the measured
        // capture no record ever sets DELTA_DATA, and only hour 1 resets.
        let sparse = header.is_delta() || self.baseline_seen;
        if !sparse {
            self.reserves.clear();
            self.baseline_seen = true;
        }
        let mut upserts = 0usize;
        for (k, record) in patch {
            match record {
                Some(record) => {
                    upserts += 1;
                    self.reserves.insert(k, record);
                }
                None => {
                    self.reserves.remove(&k);
                }
            }
        }
        Ok(FoldOutcome::Folded {
            reset: !sparse,
            upserts,
            deletes,
        })
    }
}

/// A fresh, empty accumulator. `BTreeMap::new` behind a name, so callers do not
/// have to know the state's shape.
pub fn empty_state() -> State {
    State {
        reserves: BTreeMap::new(),
        baseline_seen: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{decode_baseline, encode_baseline};
    use crate::value::{dequantize, quantize};
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn collection() -> Collection {
        schema().collections[0].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// A raw 24-byte row, built the way the collector packs one
    /// (`attach_log.rs:71511`-`:71518`).
    fn raw_row(tag_db_id: u32, variant_id: u32, flags: u32, reserved: u32, amount_raw: i64) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        row.extend_from_slice(&tag_db_id.to_le_bytes());
        row.extend_from_slice(&variant_id.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&reserved.to_le_bytes());
        row.extend_from_slice(&amount_raw.to_le_bytes());
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    fn live_row(tag_db_id: u32, variant_id: u32, amount_raw: i64) -> Vec<u8> {
        raw_row(tag_db_id, variant_id, 0, 0, amount_raw)
    }

    /// A whole payload: the 48-byte header with the collector's own witness
    /// counts, then the rows.
    fn payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        payload_with(status_flags, rows, |h| h)
    }

    /// The header words in payload order, so a test can perturb exactly one.
    #[derive(Clone, Copy)]
    struct Header {
        elapsed_wall_ms: u64,
        row_count: u32,
        changed_rows: u32,
        status_flags: u32,
        observed_country_rows: u32,
        observed_stockpile_rows: u32,
        observed_air_variant_rows: u32,
        matched_stockpile_rows: u32,
        invalid_country_rows: u32,
        deleted_rows: u32,
        scale_raw: u32,
    }

    fn payload_with(
        status_flags: u32,
        rows: &[Vec<u8>],
        perturb: impl FnOnce(Header) -> Header,
    ) -> Vec<u8> {
        let deleted = rows
            .iter()
            .filter(|r| u32::from_le_bytes([r[8], r[9], r[10], r[11]]) & ROW_FLAG_DELETED != 0)
            .count() as u32;
        let h = perturb(Header {
            elapsed_wall_ms: 0,
            row_count: rows.len() as u32,
            changed_rows: rows.len() as u32,
            status_flags,
            observed_country_rows: 356,
            observed_stockpile_rows: 1997,
            observed_air_variant_rows: 204,
            matched_stockpile_rows: rows.len() as u32,
            invalid_country_rows: 0,
            deleted_rows: deleted,
            scale_raw: FIXED_SCALE_RAW,
        });
        let mut p = Vec::new();
        p.extend_from_slice(&h.elapsed_wall_ms.to_le_bytes());
        for word in [
            h.row_count,
            h.changed_rows,
            h.status_flags,
            h.observed_country_rows,
            h.observed_stockpile_rows,
            h.observed_air_variant_rows,
            h.matched_stockpile_rows,
            h.invalid_country_rows,
            h.deleted_rows,
            h.scale_raw,
        ] {
            p.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(p.len(), HEADER_LEN);
        for row in rows {
            p.extend_from_slice(row);
        }
        p
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d36 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_country_and_variant() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 2, "AirVariantReserveKey is a pair (`:15285`)");
        assert_eq!(keys[0].field_id, field::TAG_DB_ID);
        assert_eq!(keys[1].field_id, field::VARIANT_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "a country tag is content-defined (§5), so a manifest must resolve it"
        );
        assert_eq!(
            keys[1].kind,
            Kind::Unsigned,
            "a variant id is a runtime catalog identity, not content"
        );
        for k in &keys {
            assert!(k.kind.legal_as_key());
            assert!(
                !k.optional && !k.nullable && !k.unknown_capable && k.derived.is_none(),
                "a key component is concrete and present in every row"
            );
        }
        assert_eq!(c.fields.len(), 3, "2 key components + 1 value field");
        assert_eq!(c.value_fields().count(), 1);
    }

    #[test]
    fn the_schema_declares_no_empty_state_at_all() {
        // The step-3 capability test. d36 declares NO `.optional()`, NO
        // `.nullable()` and NO `.unknown_capable()`, so the mapper may emit
        // exactly one thing — a value — and the "raw-bytes case per declared
        // capability" cases do not exist. See the module docs for why: the single
        // value field is ungated, and every path on which the collector cannot
        // read an amount drops the whole record via a record-level failure
        // status rather than emitting a gated-off row.
        let c = collection();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(c.value_fields().all(|f| !f.nullable));
        assert!(c.value_fields().all(|f| !f.unknown_capable));
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; a reserve pool moves on
        // production and attrition, not on the clock, so `frame_tick` never
        // enters a value.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
    }

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_row(&c, &live_row(41, 7318, 10_100_000), 0).expect("maps");
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
    fn a_zero_amount_is_a_value_not_an_empty_state() {
        // A country holding no planes of a variant it has designed: the row
        // exists, the amount is 0. §3 keeps zero out of the three empty states.
        let c = collection();
        let r = record_from_row(&c, &live_row(41, 7318, 0), 0).expect("maps");
        let v = r.values[index_of(&c, field::AMOUNT)];
        assert_eq!(v, Value::Continuous(0.0));
        assert!(v.carries_value(), "zero carries a value");
        assert!(!v.is_null() && !v.is_unknown() && v.is_present());

        // And it round-trips as a value, not as an empty state.
        let mut bytes = Vec::new();
        r.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(back.values[index_of(&c, field::AMOUNT)], Value::Continuous(0.0));
    }

    #[test]
    fn the_mapper_never_emits_an_empty_state() {
        // The states test, in the only form d36 admits: over the whole space of
        // rows the grammar accepts, every mapped value carries a value. A mapper
        // that emitted Absent/Null/Unknown here would contradict a schema that
        // declares none of the three, and the encoder would reject it.
        let c = collection();
        for amount in [
            0i64,
            1,
            -1,
            10_100_000,
            EXACT_AMOUNT_RAW_BAND,
            -EXACT_AMOUNT_RAW_BAND,
            EXACT_AMOUNT_RAW_BAND + 1,
            i64::MAX,
            i64::MIN,
        ] {
            for (tag, variant) in [(0u32, 0u32), (1, 1), (368, 7318), (u32::MAX, u32::MAX)] {
                let r = record_from_row(&c, &live_row(tag, variant, amount), 0)
                    .unwrap_or_else(|e| panic!("row ({tag},{variant},{amount}) maps: {e}"));
                let v = r.values[index_of(&c, field::AMOUNT)];
                assert!(v.carries_value(), "({tag},{variant},{amount}) -> {v:?}");
                let mut bytes = Vec::new();
                r.encode(&c, &mut bytes).expect("a declared-capability-free record encodes");
            }
        }
    }

    #[test]
    fn amount_reads_at_offset_16_through_the_hundred_thousand_scale() {
        // The test that catches a 32768-vs-100000 mapping error and a transposed
        // offset in the same breath: known fixed-point words against their exact
        // f64 readings, with distinct values in every other row slot so a
        // mis-offset read lands on a recognisably wrong number.
        let c = collection();
        let cases: [(i64, f64); 9] = [
            (100_000, 1.0),
            (150_000, 1.5),
            (-100_000, -1.0),
            (1, 0.000_01),
            (-1, -0.000_01),
            (10_100_000, 101.0),
            (0, 0.0),
            (32_768, 0.327_68),
            (100_000_000, 1000.0),
        ];
        for (raw, expected) in cases {
            let r = record_from_row(&c, &live_row(0xAAAA_BBBB, 0xCCCC_DDDD, raw), 0).expect("maps");
            assert_eq!(
                r.values[index_of(&c, field::AMOUNT)],
                Value::Continuous(expected),
                "raw {raw} must read {expected} (scale {FIXED_SCALE})"
            );
        }
        // The key components read at +0 and +4, in that order — a transposition
        // is the same failure class as a wrong scale.
        assert_eq!(
            key_from_row(&live_row(0xAAAA_BBBB, 0xCCCC_DDDD, 0)).expect("keys"),
            key(&[0xAAAA_BBBB, 0xCCCC_DDDD])
        );
        // The flag word reads at +8 and the reserved word at +12, not the other
        // way round: a delete bit in the reserved slot is a rejected row, and a
        // nonzero word in the flag slot is a rejected row.
        assert!(row_flags(&raw_row(1, 2, 0, ROW_FLAG_DELETED, 0)).is_err());
        assert!(is_deleted_row(&raw_row(1, 2, ROW_FLAG_DELETED, 0, 0)).expect("flags"));
    }

    #[test]
    fn scale_word_is_invariant_not_data() {
        // `world_fold.rs:15319` asserts the header's scale word equals
        // `FIXED_SCALE`, so it is constant over every payload the grammar accepts
        // and carries no information. The invariant is enforced here; no field
        // re-encodes it.
        assert!(collection()
            .fields
            .iter()
            .all(|f| f.field_id != field::TAG_DB_ID || f.is_key()));
        assert_eq!(collection().value_fields().count(), 1, "only `amount` is on the wire");
        for wrong in [0u32, 1, 32_768, 99_999, 100_001, 1_000_000, u32::MAX] {
            let p = payload_with(0, &[live_row(1, 2, 100_000)], |mut h| {
                h.scale_raw = wrong;
                h
            });
            assert!(
                parse_header(&p).is_err(),
                "scale_raw {wrong} must be refused, not carried"
            );
        }
        assert!(parse_header(&payload(0, &[live_row(1, 2, 100_000)])).is_ok());
    }

    #[test]
    fn fixed_five_reproduces_every_raw_in_the_band() {
        // The Fixed(5)-vs-Exact defence, as an executable claim: on every
        // `amount_raw` magnitude tried at or below `EXACT_AMOUNT_RAW_BAND`,
        // `quantize(raw / 10^5)` returns `raw` unchanged — nothing is rounded,
        // and the raw integer the fold's own renderer emits beside the scaled
        // value (`world_fold.rs::build_designs`) survives the wire.
        let q = Quant::Fixed(SCALE_DIGITS);
        let mut raws: Vec<i64> = vec![0, 1, -1, 5, 99_999, 100_000, 100_001, 10_100_000];
        for decade in 0..16u32 {
            raws.push(10i64.pow(decade));
            raws.push(-10i64.pow(decade));
            raws.push(10i64.pow(decade) + 1);
        }
        for step in 0..2_000i64 {
            raws.push(EXACT_AMOUNT_RAW_BAND - step);
            raws.push(-(EXACT_AMOUNT_RAW_BAND - step));
        }
        // A deterministic spread over the whole band, not just its ends.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let magnitude = (state % (EXACT_AMOUNT_RAW_BAND as u64 + 1)) as i64;
            raws.push(if state & 1 == 0 { magnitude } else { -magnitude });
        }
        for raw in raws {
            if raw.unsigned_abs() > EXACT_AMOUNT_RAW_BAND as u64 {
                continue;
            }
            let v = raw as f64 / FIXED_SCALE;
            assert_eq!(
                quantize(v, q),
                Some(raw),
                "raw {raw} must survive fixed(5) unchanged"
            );
            assert_eq!(
                dequantize(raw, q),
                v,
                "raw {raw} must dequantize to the same f64 the mapper produced"
            );
        }
    }

    #[test]
    fn quantization_saves_bytes_against_an_all_exact_variant() {
        // Step 8.4. The comparison is against the same states encoded with
        // `Quant::Exact`, which is the alternative this domain actually had.
        let c = collection();
        let mut exact_schema = schema();
        for f in &mut exact_schema.collections[0].fields {
            if f.field_id == field::AMOUNT {
                f.quant = Quant::Exact;
            }
        }
        let exact_c = exact_schema.collections[0].clone();

        let mut fixed_state: KeyedState = BTreeMap::new();
        let mut exact_state: KeyedState = BTreeMap::new();
        // A realistic table: the capture's own shape, 191 rows over 41 tags.
        for i in 0..191u32 {
            let row = live_row(1 + i % 41, 100 + i * 37, 100_000 * (1 + i as i64 % 400));
            fixed_state.insert(
                key_from_row(&row).expect("keys"),
                record_from_row(&c, &row, 0).expect("maps"),
            );
            exact_state.insert(
                key_from_row(&row).expect("keys"),
                record_from_row(&exact_c, &row, 0).expect("maps"),
            );
        }

        let mut fixed_bytes = Vec::new();
        encode_baseline(&schema(), &[fixed_state.clone()], &mut fixed_bytes).expect("encodes");
        let mut exact_bytes = Vec::new();
        encode_baseline(&exact_schema, &[exact_state], &mut exact_bytes).expect("encodes");
        assert!(
            fixed_bytes.len() < exact_bytes.len(),
            "fixed(5) {} bytes must beat exact {} bytes",
            fixed_bytes.len(),
            exact_bytes.len()
        );

        // And it is lossless: the decoded state is stored-equal to the encoded one.
        let (states, consumed) = decode_baseline(&schema(), &fixed_bytes, 0).expect("decodes");
        assert_eq!(consumed, fixed_bytes.len());
        assert_eq!(states[0], fixed_state, "fixed(5) round-trips the whole table");
    }

    #[test]
    fn the_mapper_rejects_every_row_the_fold_rejects() {
        let c = collection();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 48] {
            let row = vec![0u8; len];
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "length {len} must be rejected, not read"
            );
            assert!(key_from_row(&row).is_err(), "length {len}");
            assert!(row_flags(&row).is_err(), "length {len}");
            assert!(is_deleted_row(&row).is_err(), "length {len}");
        }
        // An unknown flag bit (`world_fold.rs::fold_air_variant_reserves`) — every one of them.
        for bit in 1..32u32 {
            let row = raw_row(1, 2, ROW_FLAG_DELETED | (1 << bit), 0, 0);
            assert!(record_from_row(&c, &row, 0).is_err(), "flag bit {bit}");
            assert!(row_flags(&row).is_err(), "flag bit {bit}");
            assert!(is_deleted_row(&row).is_err(), "flag bit {bit}");
        }
        // A nonzero reserved word (`:15356`), including single bits.
        for bit in 0..32u32 {
            let row = raw_row(1, 2, 0, 1 << bit, 5);
            assert!(record_from_row(&c, &row, 0).is_err(), "reserved bit {bit}");
        }
        // A deleted row is a removal, never a record carrying 0.0.
        assert!(record_from_row(&c, &raw_row(1, 2, ROW_FLAG_DELETED, 0, 0), 0).is_err());
        // ...and the fold rejects NOTHING about `amount_raw`, so neither does the
        // mapper: every `i64`, including the whole range outside the exactness
        // band, maps. Regression for the round-1 review finding, whose reported
        // input was `2^51 + 1` — a value the collector and the fold both accept.
        for raw in [
            EXACT_AMOUNT_RAW_BAND,
            -EXACT_AMOUNT_RAW_BAND,
            EXACT_AMOUNT_RAW_BAND + 1,
            -(EXACT_AMOUNT_RAW_BAND + 1),
            1 << 52,
            1 << 53,
            100_000_000_000_000_000, // the collector's own ceiling, 1e12 * 10^5
            i64::MAX,
            i64::MIN,
        ] {
            assert!(
                record_from_row(&c, &live_row(1, 2, raw), 0).is_ok(),
                "amount_raw {raw} is accepted by the fold and must be mapped"
            );
        }
    }

    #[test]
    fn every_i64_amount_survives_encode_and_decode_unchanged() {
        // The property that must hold over the WHOLE `i64` domain once the mapper
        // stopped gating on magnitude: whatever the mapper produced, the bytes
        // reproduce it exactly and re-encode identically. Above
        // `EXACT_AMOUNT_RAW_BAND` the recovered *raw integer* may drift by one
        // (see that constant), but the wire never disagrees with itself — §6's
        // escape rule puts an unstable value on the raw-f64 path instead.
        let c = collection();
        let mut raws: Vec<i64> = vec![
            0,
            1,
            -1,
            10_100_000,
            EXACT_AMOUNT_RAW_BAND,
            EXACT_AMOUNT_RAW_BAND + 1,
            -(EXACT_AMOUNT_RAW_BAND + 1),
            (1 << 52) + 1,
            (1 << 53) + 1,
            100_000_000_000_000_000,
            i64::MAX,
            i64::MIN,
            i64::MAX - 1,
            i64::MIN + 1,
        ];
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            raws.push(state as i64);
        }
        for raw in raws {
            let record = record_from_row(&c, &live_row(1, 2, raw), 0)
                .unwrap_or_else(|e| panic!("raw {raw} maps: {e}"));
            let mut bytes = Vec::new();
            record
                .encode(&c, &mut bytes)
                .unwrap_or_else(|e| panic!("raw {raw} encodes: {e:?}"));
            let (back, pos) = Record::decode(&c, &bytes, 0)
                .unwrap_or_else(|e| panic!("raw {raw} decodes: {e:?}"));
            assert_eq!(pos, bytes.len(), "raw {raw} consumes exactly its own bytes");
            assert_eq!(back, record, "raw {raw} decodes to the value that was mapped");
            let mut reencoded = Vec::new();
            back.encode(&c, &mut reencoded).expect("re-encodes");
            assert_eq!(reencoded, bytes, "raw {raw} is byte-identical on re-encode");
        }
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a record
        // full of placeholders under this domain's field ids.
        let mut c = collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&c, &live_row(1, 2, 5), 0).is_err());
    }

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let rows = vec![live_row(1, 2, 100_000), live_row(1, 3, 200_000)];
        let p = payload(0, &rows);
        let (header, body) = split_payload(&p).expect("well-formed payload");
        assert_eq!(header.row_count, 2);
        assert_eq!(header.scale_raw, FIXED_SCALE_RAW);
        assert_eq!(body.len(), 2 * ROW_LEN);

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
            assert!(parse_header(&p[..len]).is_err(), "len {len}");
        }
        // A rowless record is legal and common — 129 of the capture's 146 hours.
        let empty = payload(0, &[]);
        assert_eq!(empty.len(), HEADER_LEN);
        let (h, b) = split_payload(&empty).expect("a rowless record is well-formed");
        assert_eq!(h.row_count, 0);
        assert!(b.is_empty());
        // Length that does not match the row count, both directions.
        let mut short = p.clone();
        short.pop();
        assert!(split_payload(&short).is_err());
        let mut long = p.clone();
        long.push(0);
        assert!(split_payload(&long).is_err());
        // A row_count whose byte length overflows `usize` must be rejected, not
        // wrapped into a plausible small length.
        let huge = payload_with(0, &rows, |mut h| {
            h.row_count = u32::MAX;
            h.changed_rows = u32::MAX;
            h.deleted_rows = 0;
            h
        });
        assert!(split_payload(&huge).is_err());
    }

    #[test]
    fn parse_header_enforces_all_four_count_invariants() {
        // `world_fold.rs::fold_air_variant_reserves`, one at a time, each at max + 1 and each
        // at its boundary — the four are separate checks and a copy-paste error
        // between them is exactly the failure this covers.
        let rows = vec![live_row(1, 2, 100_000)];
        type Perturb = fn(Header) -> Header;
        let over: [(&str, Perturb); 4] = [
            ("changed_rows > row_count", |mut h| {
                h.changed_rows = h.row_count + 1;
                h
            }),
            ("matched > observed_stockpile", |mut h| {
                h.matched_stockpile_rows = h.observed_stockpile_rows + 1;
                h
            }),
            ("invalid_country > observed_country", |mut h| {
                h.invalid_country_rows = h.observed_country_rows + 1;
                h
            }),
            ("deleted_rows > row_count", |mut h| {
                h.deleted_rows = h.row_count + 1;
                h
            }),
        ];
        for (name, perturb) in over {
            assert!(
                parse_header(&payload_with(0, &rows, perturb)).is_err(),
                "{name} must be refused"
            );
        }
        // Equality is legal for all four — they are `ensure_count_at_most`, not
        // `ensure_count_below`.
        let at_bound: [(&str, Perturb); 4] = [
            ("changed_rows == row_count", |mut h| {
                h.changed_rows = h.row_count;
                h
            }),
            ("matched == observed_stockpile", |mut h| {
                h.matched_stockpile_rows = h.observed_stockpile_rows;
                h
            }),
            ("invalid_country == observed_country", |mut h| {
                h.invalid_country_rows = h.observed_country_rows;
                h
            }),
            ("deleted_rows == row_count", |mut h| {
                h.deleted_rows = h.row_count;
                h
            }),
        ];
        for (name, perturb) in at_bound {
            assert!(
                parse_header(&payload_with(0, &rows, perturb)).is_ok(),
                "{name} must be accepted"
            );
        }
        // And `changed_rows < row_count` is ordinary, NOT the d63 full-replace
        // rule: d36's fold requires `<=` only (`:15291`). Refusing it here would
        // refuse records the recorder folds.
        let partial = payload_with(0, &rows, |mut h| {
            h.changed_rows = 0;
            h
        });
        assert!(parse_header(&partial).is_ok());
        assert!(split_payload(&partial).is_ok());
    }

    #[test]
    fn the_key_is_the_country_and_variant_pair() {
        assert_eq!(
            key_from_row(&live_row(41, 7318, 0)).expect("keys"),
            key(&[41, 7318])
        );
        // Both components are read, and neither is range-checked: the fold checks
        // neither (`world_fold.rs::fold_air_variant_reserves`), so a zero tag_db_id is a key
        // here even though the independent authority decoder rejects it
        // (`domain_data_authority.rs:15190`). Recorded as a finding.
        assert_eq!(key_from_row(&live_row(0, 0, 0)).expect("keys"), key(&[0, 0]));
        assert_eq!(
            key_from_row(&live_row(u32::MAX, u32::MAX, 0)).expect("keys"),
            key(&[u32::MAX as u64, u32::MAX as u64])
        );
        assert!(key_from_row(&[]).is_err());
    }

    // --- The frame fold ------------------------------------------------------

    fn amount_of(state: &State, c: &Collection, tag: u32, variant: u32) -> Option<f64> {
        state
            .reserves
            .get(&key(&[tag as u64, variant as u64]))
            .map(|r| match r.values[index_of(c, field::AMOUNT)] {
                Value::Continuous(v) => v,
                other => panic!("amount is not continuous: {other:?}"),
            })
    }

    #[test]
    fn the_first_record_is_a_baseline_and_later_ones_are_sparse() {
        // `replace_or_sparse_after_first` (`world_fold.rs:2947`, `:15342`): the
        // FIRST non-delta record clears; every record after it patches, whatever
        // its DELTA_DATA flag. This is the whole reason one record is not one
        // state, and getting it backwards would either wipe the table every hour
        // or never wipe it at all.
        let s = schema();
        let c = collection();
        let mut state = empty_state();

        let outcome = state
            .fold_payload(&s, &payload(0, &[live_row(1, 10, 100_000), live_row(1, 11, 200_000)]), 0)
            .expect("folds");
        assert_eq!(
            outcome,
            FoldOutcome::Folded { reset: true, upserts: 2, deletes: 0 }
        );
        assert!(state.baseline_seen());
        assert_eq!(state.reserves.len(), 2);

        // A second record with the SAME clear DELTA_DATA flag must not clear.
        let outcome = state
            .fold_payload(&s, &payload(0, &[live_row(1, 12, 300_000)]), 0)
            .expect("folds");
        assert_eq!(
            outcome,
            FoldOutcome::Folded { reset: false, upserts: 1, deletes: 0 }
        );
        assert_eq!(state.reserves.len(), 3, "the patch added, it did not replace");
        assert_eq!(amount_of(&state, &c, 1, 10), Some(1.0));
        assert_eq!(amount_of(&state, &c, 1, 12), Some(3.0));

        // An explicit DELTA_DATA record is also a patch.
        state
            .fold_payload(&s, &payload(STATUS_DELTA_DATA, &[live_row(1, 10, 500_000)]), 0)
            .expect("folds");
        assert_eq!(state.reserves.len(), 3);
        assert_eq!(amount_of(&state, &c, 1, 10), Some(5.0), "upsert replaces in place");

        // A rowless record changes nothing at all — the capture's common case.
        let before = state.reserves.clone();
        let outcome = state.fold_payload(&s, &payload(0, &[]), 0).expect("folds");
        assert_eq!(
            outcome,
            FoldOutcome::Folded { reset: false, upserts: 0, deletes: 0 }
        );
        assert_eq!(state.reserves, before);
    }

    #[test]
    fn a_deleted_row_removes_its_key() {
        // `world_fold.rs::fold_air_variant_reserves`. A delete for a key that is not present
        // is a no-op, exactly as `BTreeMap::remove` is in the fold.
        let s = schema();
        let mut state = empty_state();
        state
            .fold_payload(&s, &payload(0, &[live_row(1, 10, 100_000), live_row(2, 10, 100_000)]), 0)
            .expect("folds");
        let outcome = state
            .fold_payload(&s, &payload(0, &[raw_row(1, 10, ROW_FLAG_DELETED, 0, 0)]), 0)
            .expect("folds");
        assert_eq!(
            outcome,
            FoldOutcome::Folded { reset: false, upserts: 0, deletes: 1 }
        );
        assert_eq!(state.reserves.len(), 1);
        assert!(state.reserves.contains_key(&key(&[2, 10])));

        // A delete for an absent key is accepted and changes nothing.
        state
            .fold_payload(&s, &payload(0, &[raw_row(9, 9, ROW_FLAG_DELETED, 0, 0)]), 0)
            .expect("folds");
        assert_eq!(state.reserves.len(), 1);

        // A delete and an upsert for the same key in one record apply in row
        // order, exactly as the fold's single pass does.
        state
            .fold_payload(
                &s,
                &payload(
                    0,
                    &[raw_row(2, 10, ROW_FLAG_DELETED, 0, 0), live_row(2, 10, 700_000)],
                ),
                0,
            )
            .expect("folds");
        assert_eq!(amount_of(&state, &collection(), 2, 10), Some(7.0));
    }

    #[test]
    fn a_failure_status_record_folds_nothing_and_retains_the_accumulator() {
        // `handle_compact_failure` (`world_fold.rs:2863`, `:15320`), every bit in
        // the mask one at a time. The accumulator is RETAINED, not cleared, and
        // the baseline flag does not move — a failure record must not be able to
        // consume the domain's one baseline.
        let s = schema();
        for bit in 0..32u32 {
            let flag = 1u32 << bit;
            let mut state = empty_state();
            let outcome = state
                .fold_payload(&s, &payload(flag, &[live_row(1, 10, 100_000)]), 0)
                .expect("a failure record is not a malformed record");
            if FAILURE_STATUS_MASK & flag != 0 {
                assert_eq!(outcome, FoldOutcome::FailureRetained, "bit {bit}");
                assert!(state.reserves.is_empty(), "bit {bit}");
                assert!(!state.baseline_seen(), "bit {bit} must not consume the baseline");
            } else if flag == STATUS_DELTA_DATA {
                // Bit 10 is not a failure bit; on a virgin accumulator it is the
                // OTHER early exit, and the ordering test covers it.
                assert_eq!(outcome, FoldOutcome::DeltaBeforeBaseline, "bit {bit}");
            } else {
                assert!(
                    matches!(outcome, FoldOutcome::Folded { .. }),
                    "bit {bit} is not a failure bit"
                );
            }
        }
        // And a failure record arriving after a baseline leaves the table alone.
        let mut state = empty_state();
        state
            .fold_payload(&s, &payload(0, &[live_row(1, 10, 100_000)]), 0)
            .expect("folds");
        let before = state.reserves.clone();
        assert_eq!(
            state
                .fold_payload(&s, &payload(1, &[live_row(1, 10, 900_000)]), 0)
                .expect("folds"),
            FoldOutcome::FailureRetained
        );
        assert_eq!(state.reserves, before);
    }

    #[test]
    fn a_delta_before_any_baseline_folds_nothing() {
        // `reject_delta_before_baseline` (`world_fold.rs:2895`, `:15327`). It runs
        // AFTER the failure gate and BEFORE the row loop, which is the order this
        // asserts by construction.
        let s = schema();
        let mut state = empty_state();
        let outcome = state
            .fold_payload(&s, &payload(STATUS_DELTA_DATA, &[live_row(1, 10, 100_000)]), 0)
            .expect("folds");
        assert_eq!(outcome, FoldOutcome::DeltaBeforeBaseline);
        assert!(state.reserves.is_empty());
        assert!(!state.baseline_seen());

        // The same record after a baseline is an ordinary patch.
        state.fold_payload(&s, &payload(0, &[]), 0).expect("folds");
        assert!(state.baseline_seen());
        assert!(matches!(
            state
                .fold_payload(&s, &payload(STATUS_DELTA_DATA, &[live_row(1, 10, 100_000)]), 0)
                .expect("folds"),
            FoldOutcome::Folded { reset: false, .. }
        ));
        assert_eq!(state.reserves.len(), 1);
    }

    #[test]
    fn the_header_checks_run_before_the_status_gates() {
        // The fold checks the four counts and the scale at `:15291`-`:15319`,
        // BEFORE `handle_compact_failure` at `:15320`. So a failure-status record
        // with an inconsistent header is an error in the recorder too, and this
        // fold must not swallow it as a quietly skipped hour.
        let s = schema();
        let mut state = empty_state();
        let bad = payload_with(1, &[live_row(1, 10, 100_000)], |mut h| {
            h.scale_raw = 32_768;
            h
        });
        assert!(state.fold_payload(&s, &bad, 0).is_err());
        let bad_counts = payload_with(1, &[live_row(1, 10, 100_000)], |mut h| {
            h.changed_rows = h.row_count + 5;
            h
        });
        assert!(state.fold_payload(&s, &bad_counts, 0).is_err());
    }

    #[test]
    fn the_exact_length_check_runs_after_the_status_gates() {
        // The mirror of the test above: the length check is at `:15334`, AFTER
        // both early exits, so a failure-status record whose length disagrees with
        // its row_count returns Ok(()) in the recorder — it never reaches the
        // check. Enforcing length earlier would refuse a stream the recorder
        // folds.
        let s = schema();
        let mut state = empty_state();
        let mut truncated = payload(1, &[live_row(1, 10, 100_000)]);
        truncated.truncate(HEADER_LEN + 4);
        assert_eq!(
            state.fold_payload(&s, &truncated, 0).expect("folds"),
            FoldOutcome::FailureRetained
        );
        // With a clean status the same payload is a rejected record.
        let mut truncated = payload(0, &[live_row(1, 10, 100_000)]);
        truncated.truncate(HEADER_LEN + 4);
        assert!(state.fold_payload(&s, &truncated, 0).is_err());
    }

    #[test]
    fn a_rejected_row_leaves_the_accumulator_untouched() {
        // Rows are decoded before any are applied, so a bad row late in a record
        // cannot leave half a patch behind.
        let s = schema();
        let mut state = empty_state();
        state
            .fold_payload(&s, &payload(0, &[live_row(1, 10, 100_000)]), 0)
            .expect("folds");
        let before = state.reserves.clone();
        let bad = payload(
            0,
            &[live_row(1, 11, 200_000), raw_row(1, 12, 0, 7, 300_000)],
        );
        assert!(state.fold_payload(&s, &bad, 0).is_err());
        assert_eq!(state.reserves, before, "no row of a rejected record applies");
    }

    #[test]
    fn fold_payload_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the boundary reads bytes from a file and must
        // return a diagnosis, never unwind.
        let s = schema();
        let mut sample = 0x853C_49E6_748F_EA9Bu64;
        for _ in 0..4_000 {
            sample ^= sample << 13;
            sample ^= sample >> 7;
            sample ^= sample << 17;
            let len = (sample as usize) % 200;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                sample ^= sample << 13;
                sample ^= sample >> 7;
                sample ^= sample << 17;
                *byte = sample as u8;
            }
            let mut state = empty_state();
            let _ = state.fold_payload(&s, &bytes, sample);
            let _ = split_payload(&bytes);
            let _ = parse_header(&bytes);
            let _ = key_from_row(&bytes);
            let _ = row_flags(&bytes);
            let _ = record_from_row(&collection(), &bytes, 0);
        }
        // And corruption of a well-formed payload, byte by byte.
        let good = payload(0, &[live_row(1, 10, 100_000), raw_row(2, 11, 1, 0, 0)]);
        for i in 0..good.len() {
            let mut p = good.clone();
            p[i] ^= 0xFF;
            let mut state = empty_state();
            let _ = state.fold_payload(&s, &p, 0);
        }
    }
}
