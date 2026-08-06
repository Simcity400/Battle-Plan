//! d35 `air_variant_stats` — the per-variant aircraft stat catalog.
//!
//! Derived from `world_fold.rs:17238`-`:17301`
//! (`fold_air_variant_stats`), its payload constants at `:987`-`:988`, the state
// @world-fold-authority domain=35 functions=fold_air_variant_stats lines=17238-17301 sha256=d39b1dfa684718401b65209d22f943d7593c95d8c65c0b7f669764c571f07a50
//! struct and render at `:23835`-`:23889`, and the collector read and payload
//! writer that produce the rows
//! (`extractor/native/collector/src/attach_log.rs:52997`-`:53067`,
//! `:57497`-`:57563` and `:70484`-`:70520`).
//! `domains/UNDOCUMENTED_DOMAINS.md` lists d35 with no dossier; the fold and the
//! collector read are the whole basis, which the recipe treats as sufficient.
//!
//! **Payload grammar** (`world_fold.rs::fold_air_variant_stats`, writer at
//! `attach_log.rs:70492`-`:70500`): a 36-byte header — `elapsed_wall_ms` u64 at
//! `+0`, `row_count` u32 at `+8`, `changed_rows` u32 at `+12`, `status_flags` u32
//! at `+16`, `observed_variant_rows` u32 at `+20`, `invalid_variant_rows` u32 at
//! `+24`, `fixed_scale_raw` u32 at `+28`, `range_scale_raw` u32 at `+32` —
//! followed by `row_count` fixed 64-byte rows. There is no legacy short row: the
//! fold requires `payload.len() == 36 + row_count * 64` exactly (`:15230`), so
//! the length never selects a row form and the mapper accepts exactly one.
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `variant_id`               | raw               | none (key; zero rejected, `:15241`) |
//! | `+4`  | u32 | `flags`                    | raw               | none (read and stored, never validated, `:15252`) |
//! | `+8`  | u32 | `equipment_token_raw`      | raw               | none |
//! | `+12` | u32 | `equipment_type_token_raw` | raw               | none |
//! | `+16` | u32 | `interface_type_raw`       | raw               | none |
//! | `+20` | i32 | `speed_cache`              | `/fixed_scale_raw`| none |
//! | `+24` | i32 | `range`                    | **`/range_scale_raw`** | none |
//! | `+28` | i32 | `air_attack`               | `/fixed_scale_raw`| none |
//! | `+32` | i32 | `agility`                  | `/fixed_scale_raw`| none |
//! | `+36` | i32 | `max_speed`                | `/fixed_scale_raw`| none |
//! | `+40` | i32 | `reliability`              | `/fixed_scale_raw`| none |
//! | `+44` | i32 | `air_defence`              | `/fixed_scale_raw`| none |
//! | `+48` | i32 | `thrust`                   | `/fixed_scale_raw`| none |
//! | `+52` | i32 | `fuel_consumption`         | `/fixed_scale_raw`| none |
//! | `+56` | i32 | `build_cost_ic`            | `/fixed_scale_raw`| none |
//! | `+60` | u32 | `reserved`                 | —                 | hard-enforced zero (`:15246`) |
//!
//! ## The two scales, and why neither is a constant of this module
//!
//! d35 is the first fan-out domain with **two different denominators, both
//! carried in the payload header and NEITHER asserted against `FIXED_SCALE`**.
//! `to_json` divides `range_raw` by `range_scale_raw` and the other nine stats by
//! `fixed_scale_raw` (`world_fold.rs:23860`-`:23870`), and unlike d32/d36/d38 the
//! fold never calls `ensure_fixed_scale` — the only check is that both words are
//! non-zero (`:15220`-`:15222`). So the header is the authority for both, and
//! this module hard-codes neither: [`RowScales`] is a mapper input, not a
//! constant. The capture the measurement runs against carries
//! `(fixed, range) = (100_000, 1_000)` on all 146 hours, which is exactly the
//! trap — a module that "confirmed" the measured pair and then wrote it into a
//! `const` would be correct on this capture and silently wrong by 100× on
//! `range` for any other, and every round-trip test would still pass because the
//! error would be in the mapping, not the codec.
//!
//! The same fact has a second consequence that a full-replace domain would not
//! have: **the fold copies both header words into every row's stored state**
//! (`world_fold.rs::fold_air_variant_stats`), and d35 never clears its accumulator. A row
//! inserted while the header said `1_000` keeps `1_000` even after a later header
//! says something else, because nothing re-states it. The scales are therefore
//! per-row state, not a per-frame constant, and they are carried as per-row
//! fields here for the same reason. Carrying them also keeps the raw fixed-point
//! word recoverable — `round(value * scale)` is exact for every `i32` over any
//! non-zero `u32` denominator, since the two roundings contribute at most
//! `2^31 * 2^-52 ≈ 5e-7` of absolute error, far under the half-ulp needed to
//! reach an adjacent integer (pinned by `the_raw_word_is_recoverable_from_the_stored_value`).
//!
//! ## Pure upsert: no delete grammar, no clear, no delta rejection
//!
//! `fold_air_variant_stats` guards `handle_compact_failure` and then inserts
//! (`:15223`-`:15271`). It calls neither `replace_or_sparse_after_first` nor
//! `reject_delta_before_baseline`, it never reads `COMPACT_STATUS_DELTA_DATA`,
//! it has no delete bit, and it never clears the map. d22's shipping layer at
//! least clears on a BASELINE; d35's has no baseline at all. Three consequences,
//! all of them load-bearing:
//!
//! 1. **A key never leaves the accumulator.** [`Accumulator::variants`] is
//!    monotone in its key set, so `body::diff` between two consecutive hours can
//!    never produce a tombstone. That is a theorem about this domain, not an
//!    observation about one capture, and it is asserted every hour by the
//!    measurement bin.
//! 2. **One record is not one state.** The collector emits only rows that
//!    changed since its own last frame (`attach_log.rs:52773`-`:52777` (pre-9160a8a0 lines; see git history)), so most
//!    hours carry a *rowless* payload and the hour's state is the accumulator,
//!    not the record. This is the recipe's fourth public boundary: [`Accumulator`]
//!    and [`Accumulator::fold_payload`] live here so the measurement bin, the
//!    tests and any future caller share one replay of the shipping grammar.
//! 3. **A variant whose collector read fails is silently stale, not removed.**
//!    A row that fails `read_compact_air_variant_stats_row` (`:57497`) is counted
//!    into `invalid_variant_rows` and omitted; with no delete row, the
//!    accumulator keeps that variant's previous values. That is a property of the
//!    shipping grammar, faithfully reproduced, and it is why `invalid_variant_rows`
//!    is exposed on [`PayloadHeader`] rather than dropped.
//!
//! **`changed_rows` is not asserted equal to `row_count`.** The writer sets both
//! from the same `rows.len()` (`attach_log.rs:70494`-`:70495`) and they are equal
//! on every record of the capture, but the fold does not read `changed_rows` at
//! all. Asserting a producer identity the authority does not state is how a
//! harness starts rejecting streams the recorder accepts — it was a round-1
//! finding on d10. Both words are exposed; neither is enforced.
//!
//! ## Four states: this schema declares none of them
//!
//! Every one of the sixteen value fields is present, non-null and known on every
//! row the grammar admits. The fold has **no flag gate anywhere** — `flags` is
//! read into the state and never tested (`:15252`), and no field's meaning
//! depends on it. There is no legacy row, no optional trailer, and no per-field
//! read that can fail its way into the payload: the collector's per-field reads
//! are `?`-propagated out of `read_compact_air_variant_stats_row`
//! (`attach_log.rs:57511`-`:57532`), so a single unreadable word drops the whole
//! *row*, and a row that is absent from the payload is not a row with an empty
//! field. Uncaptured is a per-row event in this domain, never a per-field one.
//!
//! So no field declares `.optional()`, `.nullable()` or `.unknown_capable()`, and
//! `zero` is a real measured value throughout — `air_attack` reads `0` on 59 of
//! the capture's 204 variants (unarmed transports and trainers), `air_defence`
//! and `thrust` on 4 each, `fuel_consumption` on 1, and `interface_type_raw` on
//! 4. Every one of those is data, not an empty state.
//!
//! ## `flags` is carried; `reserved` is not
//!
//! These two words look alike and are opposite cases. `reserved` at `+60` is
//! **hard-enforced zero by the fold** (`:15245`-`:15248`), so it is a value the
//! fold enforces as a function of nothing at all: the invariant is checked in
//! [`record_from_row`] and the word is left off the wire, exactly as d62's
//! witness-flags word and d24's derived weather bits were. `flags` at `+4` is
//! **stored and never validated**. It is written as a literal `0u32` by today's
//! writer (`attach_log.rs:70503`) and measures constant `0` across all 204 rows,
//! but the fold accepts any value, so this module carries it as an opaque
//! `Unsigned` witness. Rejecting a non-zero `flags` would make this mapper refuse
//! rows the recorder folds without complaint; inventing a gate for it would
//! invent semantics no authority states. Neither is done.
//!
//! ## No `ContentId` in this domain
//!
//! `variant_id` is a **runtime instance identity**: the collector reads it out of
//! live `CDivisionEquipmentVariant` heap objects
//! (`DIVISION_EQUIPMENT_VARIANT_OFF_ID`, `attach_log.rs:57505`), variants are
//! designed during play, and variant 3421 is a different variant next campaign.
//! No content manifest resolves it — the d33/d37 catalogs that name variants are
//! themselves capture data. §5's deciding question answers "no", so it is
//! `Unsigned`, and d35 therefore carries no `ContentId` **key**, which is the
//! heavier §13 retrofit class.
//!
//! The three token words answer the same question the same way, and the evidence
//! is unusually direct: to turn `equipment_token_raw` into a name at all, the
//! collector **scans the live process heap** for token-record objects
//! (`refresh_compact_air_variant_token_dictionary`, `attach_log.rs:53822`-`:53852`),
//! maintaining an `ambiguous_tokens` set for the ones it cannot pin down. A
//! number that can only be resolved by walking the heap of the process that
//! produced it is a process-local handle, not a reference a manifest resolves;
//! the *string* it points at is mod content, the handle is not.
//! `equipment_type_token_raw` (35 distinct values, 1…368) and `interface_type_raw`
//! (21 distinct, 0…27) are engine discriminators of the same family as d20's
//! `div_type`. All three are `Unsigned`.
//!
//! `interface_type_raw` is deliberately **not** an `Enum`: the fold validates no
//! cardinality, so any `enum_card(n)` this module invented would reject codes at
//! or above `n` (§4) — turning data the recorder accepts into a refused row.
//!
//! **A d35 port yields unnamed variants.** The human-readable `display_name` and
//! `variant_name` live in d33 `air_variant_defs` and d37
//! `stockpile_variant_defs`, which are stop conditions (length-prefixed UTF-8
//! strings, and `WIRE_FORMAT.md` has no string kind). This domain carries the
//! numbers and the identity to join on; it does not carry the names, and nothing
//! here can supply them.
//!
//! **No derived field (§6).** Nothing in d35 changes deterministically with the
//! clock: a variant's stats move when its owner researches or refits, on player
//! and AI action. The mapper still takes `frame_tick` so every domain's boundary
//! has one shape.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 35;
pub const VARIANTS: u32 = 1;

/// Field ids are stable and append-only; §11 forbids reuse for a different field.
pub mod field {
    pub const VARIANT_ID: u32 = 1;
    pub const FLAGS: u32 = 2;
    pub const EQUIPMENT_TOKEN: u32 = 3;
    pub const EQUIPMENT_TYPE_TOKEN: u32 = 4;
    pub const INTERFACE_TYPE: u32 = 5;
    pub const SPEED_CACHE: u32 = 6;
    pub const RANGE: u32 = 7;
    pub const AIR_ATTACK: u32 = 8;
    pub const AGILITY: u32 = 9;
    pub const MAX_SPEED: u32 = 10;
    pub const RELIABILITY: u32 = 11;
    pub const AIR_DEFENCE: u32 = 12;
    pub const THRUST: u32 = 13;
    pub const FUEL_CONSUMPTION: u32 = 14;
    pub const BUILD_COST_IC: u32 = 15;
    pub const FIXED_SCALE_RAW: u32 = 16;
    pub const RANGE_SCALE_RAW: u32 = 17;
}

// --- Payload grammar, from `world_fold.rs:987`-`:988` and `:15211`-`:15271`. --

/// `AIR_VARIANT_STATS_HEADER_LEN`, `world_fold.rs::AIR_VARIANT_STATS_HEADER_LEN`; `need(payload, 36, ...)`
/// at `:15211`.
pub const HEADER_LEN: usize = 36;
/// `AIR_VARIANT_STATS_ROW_LEN`, `world_fold.rs::AIR_VARIANT_STATS_ROW_LEN`. Fixed; no legacy form.
pub const ROW_LEN: usize = 64;

/// Which header word divides a stat. The fold picks per field, not per name
/// (`world_fold.rs:23860`-`:23870`) — `range` is the single odd one out, and this
/// enum exists so the choice is data the tests can enumerate rather than nine
/// copies of one expression and one copy of another.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScaleChoice {
    /// `/ fixed_scale_raw` — nine of the ten stats.
    Fixed,
    /// `/ range_scale_raw` — `range` alone (`world_fold.rs:23862`).
    Range,
}

/// The ten i32 stats, in row order, with their byte offsets and their scale.
/// Taken from the fold's row reads (`world_fold.rs::fold_air_variant_stats`) paired with
/// the render's divisions (`:23860`-`:23870`).
pub const STATS: [(u32, usize, ScaleChoice); 10] = [
    (field::SPEED_CACHE, 20, ScaleChoice::Fixed),
    (field::RANGE, 24, ScaleChoice::Range),
    (field::AIR_ATTACK, 28, ScaleChoice::Fixed),
    (field::AGILITY, 32, ScaleChoice::Fixed),
    (field::MAX_SPEED, 36, ScaleChoice::Fixed),
    (field::RELIABILITY, 40, ScaleChoice::Fixed),
    (field::AIR_DEFENCE, 44, ScaleChoice::Fixed),
    (field::THRUST, 48, ScaleChoice::Fixed),
    (field::FUEL_CONSUMPTION, 52, ScaleChoice::Fixed),
    (field::BUILD_COST_IC, 56, ScaleChoice::Fixed),
];

/// The two header denominators, as one value a caller cannot build wrong.
///
/// `fold_air_variant_stats` rejects the whole payload when either word is zero
/// (`world_fold.rs::fold_air_variant_stats`), and it does so **before** the status guard,
/// so the check applies to failure-status records too. Making that the only
/// constructor means no code path in this crate can divide by a denominator the
/// fold would have refused, including the paths that legitimately parse a
/// record they will then exclude.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RowScales {
    fixed_scale_raw: u32,
    range_scale_raw: u32,
}

impl RowScales {
    pub fn new(fixed_scale_raw: u32, range_scale_raw: u32) -> core::result::Result<Self, String> {
        if fixed_scale_raw == 0 || range_scale_raw == 0 {
            return Err("air_variant_stats scale fields must be nonzero".into());
        }
        Ok(Self { fixed_scale_raw, range_scale_raw })
    }

    pub fn fixed_scale_raw(self) -> u32 {
        self.fixed_scale_raw
    }

    pub fn range_scale_raw(self) -> u32 {
        self.range_scale_raw
    }

    /// The denominator for one stat. Both are non-zero by construction.
    pub fn divisor(self, choice: ScaleChoice) -> f64 {
        match choice {
            ScaleChoice::Fixed => self.fixed_scale_raw as f64,
            ScaleChoice::Range => self.range_scale_raw as f64,
        }
    }
}

pub fn schema() -> Schema {
    use field as f;
    let mut fields = vec![
        // Key: `variant_id` (`world_fold.rs::fold_air_variant_stats`). `Unsigned`, not
        // `ContentId` — a runtime instance identity, see the module docs.
        Field::new(f::VARIANT_ID, Kind::Unsigned).key(0),
        // Read and stored, never validated (`:15252`). An opaque witness word.
        Field::new(f::FLAGS, Kind::Unsigned),
        // Process-local token handles and an engine discriminator, all three
        // `Unsigned` — see the module docs for why none is a `ContentId` and why
        // `interface_type_raw` is not an `Enum`.
        Field::new(f::EQUIPMENT_TOKEN, Kind::Unsigned),
        Field::new(f::EQUIPMENT_TYPE_TOKEN, Kind::Unsigned),
        Field::new(f::INTERFACE_TYPE, Kind::Unsigned),
    ];

    // The ten stats. `Continuous` because the fold divides every one of them
    // (`world_fold.rs:23860`-`:23870`), and `Signed` under the hood because the
    // fold reads them with `read_i32` (`:15254`-`:15264`) — the collector's own
    // filter happens to admit only `0..=1_000_000_000`
    // (`attach_log.rs:57634`), but the fold enforces no sign and neither does
    // this module.
    //
    // `Quant::Exact` for all ten, and this is the one quantization decision in
    // the domain that is not a default: the denominator is a header word, not a
    // schema constant. A `Fixed(k)` declaration is a claim that the source lives
    // on a `10^-k` grid, and d35's source lives on a grid the payload chooses at
    // runtime — `Fixed(5)`/`Fixed(3)` would be exactly right for this capture's
    // `(100_000, 1_000)` and silently lossy for any other legal pair. §4's rule
    // decides it outright: `Exact` for anything you are unsure about, because
    // under-quantizing costs bytes and over-quantizing is unrecoverable.
    for (id, _, _) in STATS {
        fields.push(Field::new(id, Kind::Continuous).quant(Quant::Exact));
    }

    // The two denominators, carried per row because the fold stores them per row
    // and d35 never clears (`world_fold.rs::fold_air_variant_stats`). They are what makes
    // the raw fixed-point word recoverable from the stored value.
    fields.push(Field::new(f::FIXED_SCALE_RAW, Kind::Unsigned));
    fields.push(Field::new(f::RANGE_SCALE_RAW, Kind::Unsigned));

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: VARIANTS,
            // `variant_id` is a stable identity that persists across hours, which
            // is what a keyed set needs — and only a keyed set has a delta
            // grammar (§7.3). The shipping layer is incremental upsert with no
            // delete, so the key set only grows; see [`Accumulator`].
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 36-byte payload header (`world_fold.rs::fold_air_variant_stats`, writer at
/// `attach_log.rs:70493`-`:70500`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    /// Exposed, never enforced against `row_count` — see the module docs.
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_variant_rows: u32,
    /// Variants the collector saw but could not read (`attach_log.rs:53030`).
    /// With no delete grammar these variants keep their previous values.
    pub invalid_variant_rows: u32,
    pub scales: RowScales,
}

/// Parse the header and apply **only the checks the fold performs before its
/// status guard**: the 36-byte minimum (`world_fold.rs::fold_air_variant_stats`) and both scale
/// words being non-zero (`world_fold.rs::fold_air_variant_stats`).
///
/// The split from [`split_payload`] is not stylistic. `fold_air_variant_stats`
/// returns early on a failure status at `world_fold.rs::fold_air_variant_stats`, **before** the exact
/// length check at `world_fold.rs::fold_air_variant_stats`. A failure-status record whose length does
/// not match its `row_count` is therefore a record the recorder accepts and folds
/// nothing from, and a caller that ran the length check first would refuse a
/// stream the recorder reads happily. Callers that must classify a record by
/// status before deciding whether it is data call this; callers that have already
/// decided the record is data call [`split_payload`].
pub fn parse_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "air_variant_stats payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    Ok(PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_variant_rows: u32_at(payload, 20),
        invalid_variant_rows: u32_at(payload, 24),
        scales: RowScales::new(u32_at(payload, 28), u32_at(payload, 32))?,
    })
}

/// Split a d35 DATA payload into its header and its rows slice, applying every
/// structural check `fold_air_variant_stats` applies to a record it will fold
/// (`world_fold.rs::fold_air_variant_stats`).
///
/// A zero-row payload is well-formed and common: the collector emits only rows
/// that changed since its own last frame, so 145 of the capture's 146 records
/// are exactly 36 bytes with `row_count == 0`. The fold's loop simply does not
/// run (`world_fold.rs::fold_air_variant_stats`), and this returns an empty rows slice rather than an error.
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    let header = parse_header(payload)?;
    // Checked arithmetic: `row_count` is a word from a file on disk, and
    // `row_count * ROW_LEN` overflows `usize` for a large enough value. The fold
    // saturates here (`:15231`-`:15232`); saturating would turn an absurd count
    // into a plausible small length, so this rejects instead.
    let expected = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| format!("air_variant_stats row_count {} overflows", header.row_count))?;
    if payload.len() != expected {
        return Err(format!(
            "air_variant_stats len {} != {expected}",
            payload.len()
        ));
    }
    Ok((header, &payload[HEADER_LEN..]))
}

/// The key of one compact row: its `variant_id` (`world_fold.rs::fold_air_variant_stats`).
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    Ok(key(&[variant_id_of_row(row)? as u64]))
}

/// `variant_id` with the fold's zero check (`world_fold.rs::fold_air_variant_stats`).
pub fn variant_id_of_row(row: &[u8]) -> core::result::Result<u32, String> {
    check_row_len(row)?;
    let variant_id = u32_at(row, 0);
    if variant_id == 0 {
        return Err("air_variant_stats row has zero variant_id".into());
    }
    Ok(variant_id)
}

/// One compact row of the `variants` table, as the collector emits it today,
/// onto its stored Record. `row` is the raw 64-byte row slice (the caller has
/// already consumed the table header via [`split_payload`]); d35 has exactly one
/// row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is §6's derived-field input (stored = v + c * (frame_tick >> u));
/// d35 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// **`scales` is a fourth parameter, and it has to be.** The recipe's fixed
/// signature assumes a domain whose scale factors are constants of its own
/// module; d35's are words in the payload header that the fold does not assert
/// against anything (`world_fold.rs::fold_air_variant_stats`, divided at `:23860`-`:23870`).
/// The recipe's own rule for this case is "where an invariant is mode-dependent
/// or header-relative, take the mode or header value as a parameter instead of
/// hard-coding one frame shape"; here it is a *value* rather than an invariant,
/// and the same reasoning applies with more force. Pass
/// [`PayloadHeader::scales`] from the row's own record.
///
/// Fail-closed throughout, mirroring `fold_air_variant_stats`: a row that the
/// fold would reject is an `Err` here, never a coerced value. Nothing in this
/// function can panic on any 64-byte input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
    scales: RowScales,
) -> core::result::Result<Record, String> {
    // d35 has no derived field; §6's transform input is unused. Named rather
    // than `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    check_row_len(row)?;
    // `variant_id` is the key and so is not carried in the record body (§3), but
    // its validity is still part of this row's validity.
    let _ = variant_id_of_row(row)?;

    // `world_fold.rs::fold_air_variant_stats`: a non-zero `reserved` means the row is not
    // this grammar. The word is enforced and then left off the wire — re-encoding
    // a constant the fold already guarantees would be a second copy of state a
    // decoder must keep consistent.
    let reserved = u32_at(row, 60);
    if reserved != 0 {
        return Err(format!(
            "air_variant_stats row {} reserved field is nonzero ({reserved})",
            u32_at(row, 0)
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // `flags` is stored unvalidated (`:15252`); any value is data.
            field::FLAGS => Value::Unsigned(u32_at(row, 4) as u64),
            field::EQUIPMENT_TOKEN => Value::Unsigned(u32_at(row, 8) as u64),
            field::EQUIPMENT_TYPE_TOKEN => Value::Unsigned(u32_at(row, 12) as u64),
            field::INTERFACE_TYPE => Value::Unsigned(u32_at(row, 16) as u64),
            field::FIXED_SCALE_RAW => Value::Unsigned(scales.fixed_scale_raw() as u64),
            field::RANGE_SCALE_RAW => Value::Unsigned(scales.range_scale_raw() as u64),
            other => {
                let Some((_, offset, choice)) =
                    STATS.iter().copied().find(|(id, _, _)| *id == other)
                else {
                    // A collection this mapper does not know how to fill. Filling
                    // it with a placeholder would publish invented data under a
                    // field id that means something else.
                    return Err(format!(
                        "air_variant_stats has no mapping for field id {other}"
                    ));
                };
                Value::Continuous(i32_at(row, offset) as f64 / scales.divisor(choice))
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The running state (the recipe's fourth boundary) ------------------------

/// The running d35 state, replaying `fold_air_variant_stats`'s upsert grammar.
///
/// d35 needs this and d63 did not: the collector emits only rows that changed
/// since its own last frame (`attach_log.rs:52773`-`:52777` (pre-9160a8a0 lines; see git history)), so one record is
/// not one state and the hour's content lives here rather than in the record.
///
/// The whole grammar is "insert every row" — there is no baseline/delta
/// distinction to model, because the fold never reads `COMPACT_STATUS_DELTA_DATA`
/// and never clears. That is why [`fold_payload`](Accumulator::fold_payload)
/// takes no `is_delta` parameter where d22's does.
#[derive(Clone, Debug, Default)]
pub struct Accumulator {
    /// The `variants` collection's current content, keyed by `variant_id`.
    pub variants: KeyedState,
}

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold one d35 DATA payload, exactly as `fold_air_variant_stats` does from
    /// `world_fold.rs::fold_air_variant_stats` onward.
    ///
    /// The caller must already have excluded failure-status records: the fold
    /// returns at `world_fold.rs::fold_air_variant_stats` before reaching any of this, retaining the
    /// accumulator. Classify with [`parse_header`], not with this.
    ///
    /// **Rows are inserted as they are read, and a row error leaves the rows
    /// before it applied.** That is not an oversight — the fold's loop validates
    /// and inserts one row at a time (`world_fold.rs::fold_air_variant_stats`), so its accumulator is
    /// partially mutated on a mid-payload rejection too, and diverging here would
    /// mean a caller that continues past an error holds a different state than
    /// the recorder does. d22's `fold_payload` can promise all-or-nothing only
    /// because `fold_map_control`'s own errors all precede its first mutation;
    /// d35's do not. Pinned by `a_row_error_leaves_the_earlier_rows_applied`.
    ///
    /// A `variant_id` repeated inside one payload is **last-writer-wins**, not a
    /// rejected frame: the fold's `.insert` (`world_fold.rs::fold_air_variant_stats`) overwrites silently, where
    /// d63's fold rejects the duplicate.
    pub fn fold_payload(
        &mut self,
        collection: &Collection,
        payload: &[u8],
    ) -> core::result::Result<PayloadHeader, String> {
        let (header, rows) = split_payload(payload)?;
        for row in rows.chunks_exact(ROW_LEN) {
            let k = key_from_row(row)?;
            let record = record_from_row(collection, row, 0, header.scales)?;
            self.variants.insert(k, record);
        }
        Ok(header)
    }
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "air_variant_stats row length {} != {ROW_LEN}",
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
    use crate::body::diff;
    use crate::TickGrid;

    /// The capture's measured pair (`runs/batch2/deepwar_t1`, all 146 hours).
    const CAPTURE_FIXED_SCALE: u32 = 100_000;
    const CAPTURE_RANGE_SCALE: u32 = 1_000;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn collection() -> Collection {
        schema().collections[0].clone()
    }

    fn capture_scales() -> RowScales {
        RowScales::new(CAPTURE_FIXED_SCALE, CAPTURE_RANGE_SCALE).expect("both non-zero")
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    /// A raw 64-byte row, built the way the collector packs one
    /// (`attach_log.rs:70502`-`:70517`).
    fn raw_row(variant_id: u32, flags: u32, tokens: [u32; 3], stats: [i32; 10]) -> Vec<u8> {
        raw_row_with_reserved(variant_id, flags, tokens, stats, 0)
    }

    fn raw_row_with_reserved(
        variant_id: u32,
        flags: u32,
        tokens: [u32; 3],
        stats: [i32; 10],
        reserved: u32,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        row.extend_from_slice(&variant_id.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        for v in tokens {
            row.extend_from_slice(&v.to_le_bytes());
        }
        for v in stats {
            row.extend_from_slice(&v.to_le_bytes());
        }
        row.extend_from_slice(&reserved.to_le_bytes());
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    /// A row shaped like a real one from the capture: a variant whose stats sit
    /// in the measured ranges, with `range` deliberately distinguishable under
    /// the two scales (350_000 reads 350.0 at /1000 and 3.5 at /100000).
    fn full_row() -> Vec<u8> {
        raw_row(
            3421,
            0,
            [20_201, 17, 4],
            [
                45_000_000, // speed_cache  -> 450.0
                350_000,    // range        -> 350.0  (range scale!)
                1_200_000,  // air_attack   -> 12.0
                3_500_000,  // agility      -> 35.0
                55_000_000, // max_speed    -> 550.0
                80_000,     // reliability  -> 0.8
                1_000_000,  // air_defence  -> 10.0
                2_400_000,  // thrust       -> 24.0
                50_000,     // fuel_consump -> 0.5
                1_950_000,  // build_cost   -> 19.5
            ],
        )
    }

    fn payload(scales: (u32, u32), status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&204u32.to_le_bytes()); // observed_variant_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // invalid_variant_rows
        p.extend_from_slice(&scales.0.to_le_bytes());
        p.extend_from_slice(&scales.1.to_le_bytes());
        for row in rows {
            p.extend_from_slice(row);
        }
        p
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d35 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_variant() {
        let c = collection();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 1, "the fold keys rows by variant_id alone");
        assert_eq!(keys[0].field_id, field::VARIANT_ID);
        assert_eq!(
            keys[0].kind,
            Kind::Unsigned,
            "a variant id is a runtime instance identity, not mod-set content: no \
             manifest resolves it, so it is not a ContentId key"
        );
        assert!(keys[0].kind.legal_as_key());
        assert_eq!(
            c.fields.len(),
            17,
            "1 key + flags + 3 tokens + 10 stats + 2 scale words"
        );
    }

    #[test]
    fn no_field_is_a_content_id() {
        // The §13 manifest retrofit sweeps for `ContentId`; d35 must show up in
        // it with nothing, and that has to be an assertion rather than a claim.
        // Every id-like word here is either a runtime instance identity or a
        // process-local token handle — see the module docs.
        let c = collection();
        assert!(
            c.fields.iter().all(|f| f.kind != Kind::ContentId),
            "d35 carries no ContentId, so no field of it changes when §5's \
             manifest-id assignment rule lands"
        );
    }

    #[test]
    fn the_schema_declares_no_empty_state_at_all() {
        // Step 8 case 3 wants a raw-bytes case per declared capability. d35
        // declares NONE: the fold has no flag gate anywhere (`flags` at +4 is
        // read and never tested, `:15252`), there is no legacy row, and a
        // collector read that fails drops the whole row rather than emptying a
        // field. So there is no Absent, Null or Unknown case to build, and this
        // test is the record of that rather than an invented one. Every value
        // field carries a value on every row the grammar admits — including a
        // real zero, which `a_real_zero_is_a_value_not_an_empty_state` covers.
        let c = collection();
        assert_eq!(c.optional_count(), 0, "nothing here is ever inapplicable");
        assert!(c.value_fields().all(|f| !f.nullable));
        assert!(c.value_fields().all(|f| !f.unknown_capable));

        // And the mapper agrees with the declaration: over the whole row space
        // it can reach, no value is ever one of the three empty states.
        for stats in [[0i32; 10], [1; 10], [-1; 10], [i32::MAX; 10], [i32::MIN; 10]] {
            for flags in [0u32, 1, u32::MAX] {
                let r = record_from_row(
                    &c,
                    &raw_row(1, flags, [0, 0, 0], stats),
                    0,
                    capture_scales(),
                )
                .expect("maps");
                assert!(
                    r.values.iter().all(|v| v.carries_value()),
                    "no row of this grammar produces an empty state"
                );
            }
        }
    }

    #[test]
    fn no_field_is_derived() {
        // §6 derived fields are clock-linear integers; nothing in d35 changes
        // deterministically with the clock, so `frame_tick` never enters a value.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
        let c = collection();
        let a = record_from_row(&c, &full_row(), 0, capture_scales()).expect("maps");
        let b = record_from_row(&c, &full_row(), 9_999_999, capture_scales()).expect("maps");
        assert_eq!(a, b, "frame_tick cannot reach any value");
    }

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_row(&c, &full_row(), 0, capture_scales()).expect("maps");
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
    fn an_extreme_row_roundtrips_byte_for_byte() {
        // The fold enforces no sign and no magnitude on the ten stats, so the
        // round-trip claim has to cover the whole i32 range, not the collector's
        // narrower `0..=1_000_000_000` filter (`attach_log.rs:57634`).
        let c = collection();
        for stats in [[i32::MIN; 10], [i32::MAX; 10], [0; 10]] {
            let row = raw_row(u32::MAX, u32::MAX, [u32::MAX; 3], stats);
            let record = record_from_row(&c, &row, 0, capture_scales()).expect("maps");
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
    fn every_field_maps_from_its_own_offset_at_its_own_scale() {
        // Step 8 case 6, and the test the whole domain exists to get right: a
        // distinct value at every offset, so a transposed pair fails, and a
        // `range` value that reads differently under each of the two scales, so
        // a single-scale mapper fails.
        let c = collection();
        let stats: [i32; 10] = [
            45_000_000, 350_000, 1_200_000, 3_500_000, 55_000_000, 80_000, 1_000_000, 2_400_000,
            50_000, 1_950_000,
        ];
        let r = record_from_row(
            &c,
            &raw_row(3421, 7, [20_201, 17, 4], stats),
            0,
            capture_scales(),
        )
        .expect("maps");

        assert_eq!(r.values[index_of(&c, field::FLAGS)], Value::Unsigned(7));
        assert_eq!(
            r.values[index_of(&c, field::EQUIPMENT_TOKEN)],
            Value::Unsigned(20_201)
        );
        assert_eq!(
            r.values[index_of(&c, field::EQUIPMENT_TYPE_TOKEN)],
            Value::Unsigned(17)
        );
        assert_eq!(
            r.values[index_of(&c, field::INTERFACE_TYPE)],
            Value::Unsigned(4)
        );
        assert_eq!(
            r.values[index_of(&c, field::FIXED_SCALE_RAW)],
            Value::Unsigned(CAPTURE_FIXED_SCALE as u64)
        );
        assert_eq!(
            r.values[index_of(&c, field::RANGE_SCALE_RAW)],
            Value::Unsigned(CAPTURE_RANGE_SCALE as u64)
        );

        let expected: [(u32, f64); 10] = [
            (field::SPEED_CACHE, 450.0),
            // 350_000 / 1_000, NOT / 100_000. Read with the fixed scale this
            // would be 3.5 — a 100x error that every round-trip test passes.
            (field::RANGE, 350.0),
            (field::AIR_ATTACK, 12.0),
            (field::AGILITY, 35.0),
            (field::MAX_SPEED, 550.0),
            (field::RELIABILITY, 0.8),
            (field::AIR_DEFENCE, 10.0),
            (field::THRUST, 24.0),
            (field::FUEL_CONSUMPTION, 0.5),
            (field::BUILD_COST_IC, 19.5),
        ];
        for (id, want) in expected {
            assert_eq!(
                r.values[index_of(&c, id)],
                Value::Continuous(want),
                "field {id} must read {want}"
            );
        }
    }

    #[test]
    fn the_range_field_is_the_only_one_on_the_range_scale() {
        // The scale assignment stated as a property over the whole stat set,
        // driven by making the two denominators differ by a factor the naked eye
        // catches: with (1, 1000), a fixed-scaled stat reads its raw value and
        // `range` reads a thousandth of it.
        let c = collection();
        let scales = RowScales::new(1, 1_000).expect("non-zero");
        let r = record_from_row(&c, &raw_row(1, 0, [0; 3], [7_000; 10]), 0, scales).expect("maps");
        for (id, _, choice) in STATS {
            let want = match choice {
                ScaleChoice::Fixed => 7_000.0,
                ScaleChoice::Range => 7.0,
            };
            assert_eq!(
                r.values[index_of(&c, id)],
                Value::Continuous(want),
                "field {id} is on the {choice:?} scale"
            );
        }
        assert_eq!(
            STATS
                .iter()
                .filter(|(_, _, ch)| *ch == ScaleChoice::Range)
                .map(|(id, _, _)| *id)
                .collect::<Vec<_>>(),
            vec![field::RANGE],
            "exactly one stat divides by range_scale_raw (`world_fold.rs:23788`)"
        );
    }

    #[test]
    fn the_scales_come_from_the_header_and_are_never_hard_coded() {
        // The domain's headline hazard: neither word is asserted against
        // `FIXED_SCALE` by the fold, so the mapper must track whatever the header
        // says. A module that had baked in the capture's (100_000, 1_000) would
        // pass every other test in this file and fail this one.
        let c = collection();
        let row = raw_row(1, 0, [0; 3], [64; 10]);
        for (fixed, range) in [
            (100_000u32, 1_000u32), // the capture
            (1, 1),
            (2, 4),
            (32_768, 32_768), // a power-of-two denominator is legal too
            (u32::MAX, u32::MAX),
        ] {
            let scales = RowScales::new(fixed, range).expect("non-zero");
            let r = record_from_row(&c, &row, 0, scales).expect("maps");
            assert_eq!(
                r.values[index_of(&c, field::SPEED_CACHE)],
                Value::Continuous(64.0 / fixed as f64),
                "speed_cache under fixed scale {fixed}"
            );
            assert_eq!(
                r.values[index_of(&c, field::RANGE)],
                Value::Continuous(64.0 / range as f64),
                "range under range scale {range}"
            );
            // And the header words themselves are carried, so a reader can tell
            // which grid the value came off.
            assert_eq!(
                r.values[index_of(&c, field::FIXED_SCALE_RAW)],
                Value::Unsigned(fixed as u64)
            );
            assert_eq!(
                r.values[index_of(&c, field::RANGE_SCALE_RAW)],
                Value::Unsigned(range as u64)
            );
        }
    }

    #[test]
    fn a_zero_scale_word_cannot_be_constructed_or_parsed() {
        // `world_fold.rs::fold_air_variant_stats` rejects the payload; making `RowScales`
        // the only way in means no division by zero exists anywhere in the crate.
        assert!(RowScales::new(0, 1_000).is_err());
        assert!(RowScales::new(100_000, 0).is_err());
        assert!(RowScales::new(0, 0).is_err());
        assert!(RowScales::new(1, 1).is_ok());

        for (fixed, range) in [(0u32, 1_000u32), (100_000, 0), (0, 0)] {
            let p = payload((fixed, range), 0, &[full_row()]);
            assert!(parse_header(&p).is_err(), "scales ({fixed}, {range})");
            assert!(split_payload(&p).is_err(), "scales ({fixed}, {range})");
        }
    }

    #[test]
    fn the_raw_word_is_recoverable_from_the_stored_value() {
        // The claim the module docs make about carrying the scale words:
        // `round(value * scale)` returns the raw i32 exactly, for any i32 over
        // any non-zero u32. If this ever failed, carrying the scales would not
        // buy back the raw and the domain would need `Signed` raws instead.
        let c = collection();
        let raws: [i32; 10] = [
            i32::MIN,
            -1_000_000_001,
            -1,
            0,
            1,
            80_000,
            999_900_000,
            1_000_000_000,
            2_147_483_646,
            i32::MAX,
        ];
        for (fixed, range) in [
            (100_000u32, 1_000u32),
            (1, 1),
            (3, 7),
            (32_768, 65_536),
            (u32::MAX, u32::MAX - 1),
        ] {
            let scales = RowScales::new(fixed, range).expect("non-zero");
            let r = record_from_row(&c, &raw_row(1, 0, [0; 3], raws), 0, scales).expect("maps");
            for (i, (id, _, choice)) in STATS.into_iter().enumerate() {
                let Value::Continuous(v) = r.values[index_of(&c, id)] else {
                    panic!("field {id} is continuous");
                };
                let recovered = (v * scales.divisor(choice)).round();
                assert_eq!(
                    recovered, raws[i] as f64,
                    "raw {} at scale {:?} did not survive the round trip",
                    raws[i], choice
                );
            }
        }
    }

    #[test]
    fn a_real_zero_is_a_value_not_an_empty_state() {
        // 59 of the capture's 204 variants read `air_attack == 0` (transports and
        // trainers), 4 read `air_defence == 0` and `thrust == 0`, 1 reads
        // `fuel_consumption == 0`, 4 read `interface_type_raw == 0`, and every one
        // of the 204 reads `flags == 0`. §3 keeps zero out of the three empty
        // states, so every one of those is `Unsigned(0)` or `Continuous(0.0)`.
        let c = collection();
        let r = record_from_row(&c, &raw_row(836, 0, [20_165, 1, 0], [0; 10]), 0, capture_scales())
            .expect("maps");
        assert_eq!(r.values[index_of(&c, field::FLAGS)], Value::Unsigned(0));
        assert_eq!(
            r.values[index_of(&c, field::INTERFACE_TYPE)],
            Value::Unsigned(0)
        );
        for (id, _, _) in STATS {
            assert_eq!(
                r.values[index_of(&c, id)],
                Value::Continuous(0.0),
                "stat {id} zero is a measured value"
            );
        }
    }

    #[test]
    fn flags_is_carried_opaquely_and_gates_nothing() {
        // The writer packs a literal `0u32` (`attach_log.rs:70503`) and the fold
        // stores whatever is there without a test (`:15252`). So: every value is
        // accepted, every value is carried, and no other field's value moves with
        // it. Rejecting a non-zero would refuse rows the recorder folds.
        let c = collection();
        let baseline = record_from_row(&c, &full_row(), 0, capture_scales()).expect("maps");
        for flags in [0u32, 1, 0x8000_0000, u32::MAX] {
            let mut row = full_row();
            row[4..8].copy_from_slice(&flags.to_le_bytes());
            let r = record_from_row(&c, &row, 0, capture_scales()).expect("every flags word maps");
            assert_eq!(
                r.values[index_of(&c, field::FLAGS)],
                Value::Unsigned(flags as u64)
            );
            for f in c.value_fields().filter(|f| f.field_id != field::FLAGS) {
                assert_eq!(
                    r.values[index_of(&c, f.field_id)],
                    baseline.values[index_of(&c, f.field_id)],
                    "field {} must not move with flags {flags:#x}",
                    f.field_id
                );
            }
        }
    }

    #[test]
    fn reserved_is_enforced_and_not_carried() {
        // The opposite case to `flags`: hard-enforced zero by the fold
        // (`:15245`-`:15248`), so it is an invariant rather than a field.
        let c = collection();
        assert!(
            !c.fields.iter().any(|f| f.field_id == 18),
            "no field id was spent on `reserved`"
        );
        assert_eq!(c.fields.len(), 17);
        for reserved in [1u32, 0x8000_0000, u32::MAX] {
            let row = raw_row_with_reserved(1, 0, [0; 3], [0; 10], reserved);
            assert!(
                record_from_row(&c, &row, 0, capture_scales()).is_err(),
                "reserved {reserved:#x} must be rejected as the fold rejects it"
            );
        }
        assert!(record_from_row(
            &c,
            &raw_row_with_reserved(1, 0, [0; 3], [0; 10], 0),
            0,
            capture_scales()
        )
        .is_ok());
    }

    #[test]
    fn no_continuous_field_is_quantized() {
        // Step 8 case 4 does not apply: d35 declares no quantized field, so there
        // is no all-`Exact` variant to measure against. The reason is not the
        // usual "no recorded precision" — it is that the denominator is a header
        // word. `Fixed(k)` declares that the source lives on a `10^-k` grid;
        // d35's source lives on whichever grid the payload's `fixed_scale_raw`
        // and `range_scale_raw` name at runtime, and the schema is fixed while
        // the header is not. `Fixed(5)`/`Fixed(3)` would be exact for this
        // capture's (100_000, 1_000) and silently lossy for any other legal pair,
        // which is §4's unrecoverable direction.
        let c = collection();
        let continuous: Vec<u32> = c
            .value_fields()
            .filter(|f| matches!(f.kind, Kind::Continuous))
            .map(|f| f.field_id)
            .collect();
        assert_eq!(continuous.len(), 10, "the ten scaled stats");
        for f in c.value_fields() {
            if matches!(f.kind, Kind::Continuous) {
                assert_eq!(f.quant, Quant::Exact, "field {} must stay exact", f.field_id);
            }
        }
    }

    #[test]
    fn the_mapper_rejects_every_row_the_fold_rejects() {
        let c = collection();
        // Wrong length, both directions, including the empty slice.
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 32, 128] {
            let row = vec![0u8; len];
            assert!(
                record_from_row(&c, &row, 0, capture_scales()).is_err(),
                "length {len} must be rejected, not read"
            );
        }
        // Zero variant_id (`world_fold.rs::fold_air_variant_stats`).
        assert!(record_from_row(
            &c,
            &raw_row(0, 0, [1, 1, 1], [1; 10]),
            0,
            capture_scales()
        )
        .is_err());
        // Non-zero reserved, covered exhaustively by bit position.
        for bit in 0..32u32 {
            let row = raw_row_with_reserved(1, 0, [0; 3], [0; 10], 1 << bit);
            assert!(
                record_from_row(&c, &row, 0, capture_scales()).is_err(),
                "reserved bit {bit} must be rejected"
            );
        }
    }

    #[test]
    fn the_mapper_accepts_every_row_the_fold_accepts() {
        // The other direction, and the one that matters more: the fold's only
        // per-row rejections are a zero `variant_id` and a non-zero `reserved`.
        // Everything else — any `flags`, any token, any stat of either sign — is
        // data. A mapper that tightened this would silently drop real rows.
        let c = collection();
        for variant_id in [1u32, 836, 7_318, u32::MAX] {
            for flags in [0u32, 1, u32::MAX] {
                for stats in [[0i32; 10], [i32::MIN; 10], [i32::MAX; 10], [-1; 10]] {
                    let row = raw_row(variant_id, flags, [0, u32::MAX, 27], stats);
                    assert!(
                        record_from_row(&c, &row, 0, capture_scales()).is_ok(),
                        "variant {variant_id} flags {flags:#x} must map"
                    );
                }
            }
        }
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a
        // record full of placeholders under this domain's field ids.
        let mut c = collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&c, &full_row(), 0, capture_scales()).is_err());
    }

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let rows = vec![full_row()];
        let p = payload((CAPTURE_FIXED_SCALE, CAPTURE_RANGE_SCALE), 0, &rows);
        let (header, body) = split_payload(&p).expect("well-formed payload");
        assert_eq!(header.row_count, 1);
        assert_eq!(header.changed_rows, 1);
        assert_eq!(header.observed_variant_rows, 204);
        assert_eq!(header.scales, capture_scales());
        assert_eq!(body, &rows[0][..]);

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
            assert!(parse_header(&p[..len]).is_err(), "len {len}");
        }
        // Length that does not match the row count.
        let mut truncated = p.clone();
        truncated.pop();
        assert!(split_payload(&truncated).is_err());
        // A row_count whose byte length overflows `usize` must be rejected, not
        // saturated into a plausible small length as the fold's own arithmetic
        // would (`world_fold.rs::fold_air_variant_stats`).
        let mut huge = p.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(split_payload(&huge).is_err());
    }

    #[test]
    fn a_rowless_payload_is_well_formed_not_an_error() {
        // 145 of the capture's 146 records are exactly this: the collector had
        // nothing to re-state, so it emitted a bare header. The fold's row loop
        // does not run (`:15239`) and the accumulator is retained untouched.
        let c = collection();
        let p = payload((CAPTURE_FIXED_SCALE, CAPTURE_RANGE_SCALE), 0, &[]);
        assert_eq!(p.len(), HEADER_LEN);
        let (header, body) = split_payload(&p).expect("a bare header is a valid payload");
        assert_eq!(header.row_count, 0);
        assert!(body.is_empty());

        let mut acc = Accumulator::new();
        acc.fold_payload(&c, &payload((100_000, 1_000), 0, &[full_row()]))
            .expect("folds");
        let before = acc.variants.clone();
        acc.fold_payload(&c, &p).expect("folds a rowless payload");
        assert_eq!(acc.variants, before, "a rowless payload changes nothing");
    }

    #[test]
    fn split_payload_does_not_run_the_checks_the_fold_defers_past_its_status_guard() {
        // `fold_air_variant_stats` returns on a failure status at `:15223`-`:15229`,
        // BEFORE the exact-length check at `:15230`. So a failure-status record
        // with an inconsistent length is a record the recorder accepts. A caller
        // that classified by status must be able to read the header without the
        // length check refusing first — that is what `parse_header` is for, and
        // a harness that used `split_payload` for classification would reject
        // streams the recorder reads.
        let mut p = payload((CAPTURE_FIXED_SCALE, CAPTURE_RANGE_SCALE), 1, &[]);
        p[8..12].copy_from_slice(&9u32.to_le_bytes()); // row_count lies
        let header = parse_header(&p).expect("the header alone is readable");
        assert_eq!(header.status_flags, 1);
        assert_eq!(header.row_count, 9);
        assert!(
            split_payload(&p).is_err(),
            "the length check still exists, it just runs later"
        );
        // But the scale check does NOT run later: it precedes the status guard,
        // so it applies to a failure record too.
        let bad = payload((0, 1_000), 1, &[]);
        assert!(parse_header(&bad).is_err());
    }

    #[test]
    fn changed_rows_is_never_asserted_against_row_count() {
        // The writer sets both from one `rows.len()` (`attach_log.rs:70494`), and
        // they are equal on all 146 records of the capture — but the fold never
        // reads `changed_rows`. Enforcing a producer identity no authority states
        // is how a harness starts refusing streams the recorder accepts (a
        // round-1 finding on d10). Both are exposed; neither is enforced.
        let mut p = payload((CAPTURE_FIXED_SCALE, CAPTURE_RANGE_SCALE), 0, &[full_row()]);
        for changed in [0u32, 1, 7, u32::MAX] {
            p[12..16].copy_from_slice(&changed.to_le_bytes());
            let (header, _) = split_payload(&p).expect("changed_rows is data, not a constraint");
            assert_eq!(header.changed_rows, changed);
            assert_eq!(header.row_count, 1);
        }
    }

    #[test]
    fn the_key_is_the_variant_id() {
        assert_eq!(key_from_row(&full_row()).expect("keys"), key(&[3421]));
        assert!(key_from_row(&raw_row(0, 0, [0; 3], [0; 10])).is_err());
        assert!(key_from_row(&[]).is_err());
        assert_eq!(
            key_from_row(&raw_row(u32::MAX, 0, [0; 3], [0; 10])).expect("keys"),
            key(&[u32::MAX as u64])
        );
    }

    #[test]
    fn the_accumulator_upserts_and_never_removes_a_key() {
        // The whole d35 grammar in one test: rows upsert, absent keys persist
        // untouched, and nothing in any payload can remove one.
        let c = collection();
        let mut acc = Accumulator::new();
        let a = raw_row(1, 0, [1, 1, 1], [1_000_000; 10]);
        let b = raw_row(2, 0, [2, 2, 2], [2_000_000; 10]);
        acc.fold_payload(&c, &payload((100_000, 1_000), 0, &[a.clone(), b.clone()]))
            .expect("folds");
        assert_eq!(acc.variants.len(), 2);

        // A payload re-stating only variant 1 must leave variant 2 exactly as it
        // was: there is no clear and no delete.
        let a2 = raw_row(1, 0, [9, 9, 9], [3_000_000; 10]);
        let before_two = acc.variants[&key(&[2])].clone();
        acc.fold_payload(&c, &payload((100_000, 1_000), 0, &[a2]))
            .expect("folds");
        assert_eq!(acc.variants.len(), 2, "no key ever leaves");
        assert_eq!(acc.variants[&key(&[2])], before_two);
        assert_eq!(
            acc.variants[&key(&[1])].values[index_of(&c, field::EQUIPMENT_TOKEN)],
            Value::Unsigned(9),
            "variant 1 was upserted"
        );

        // And an empty payload leaves both.
        acc.fold_payload(&c, &payload((100_000, 1_000), 0, &[]))
            .expect("folds");
        assert_eq!(acc.variants.len(), 2);
    }

    #[test]
    fn the_key_set_is_monotone_so_a_delta_can_never_tombstone() {
        // The theorem the measurement bin asserts every hour, stated here over
        // the grammar rather than over one capture: with no delete row and no
        // clear, the accumulator's key set only grows, so `diff` between two
        // consecutive folded states has no tombstones. If the fold ever gains a
        // delete grammar this test is what fails.
        let c = collection();
        let mut acc = Accumulator::new();
        let mut prior: Option<KeyedState> = None;
        let payloads = [
            vec![raw_row(1, 0, [1, 1, 1], [1; 10])],
            vec![],
            vec![raw_row(2, 0, [2, 2, 2], [2; 10]), raw_row(3, 0, [3, 3, 3], [3; 10])],
            vec![raw_row(1, 0, [4, 4, 4], [4; 10])],
            vec![],
        ];
        for p in payloads {
            acc.fold_payload(&c, &payload((100_000, 1_000), 0, &p))
                .expect("folds");
            if let Some(prev) = &prior {
                let d = diff(&c, prev, &acc.variants);
                assert!(
                    d.tombstones.is_empty(),
                    "an upsert-only accumulator cannot lose a key"
                );
            }
            prior = Some(acc.variants.clone());
        }
        assert_eq!(acc.variants.len(), 3);
    }

    #[test]
    fn a_repeated_variant_id_in_one_payload_is_last_writer_wins() {
        // `world_fold.rs::fold_air_variant_stats` inserts without checking, so a repeat overwrites
        // rather than rejecting the frame — the opposite of d63, whose fold
        // rejects a duplicate `tag_db_id`. Mirroring the fold is the rule.
        let c = collection();
        let mut acc = Accumulator::new();
        let first = raw_row(5, 0, [1, 1, 1], [1_000_000; 10]);
        let second = raw_row(5, 0, [2, 2, 2], [2_000_000; 10]);
        acc.fold_payload(&c, &payload((100_000, 1_000), 0, &[first, second]))
            .expect("a duplicate key is not a rejected frame");
        assert_eq!(acc.variants.len(), 1);
        assert_eq!(
            acc.variants[&key(&[5])].values[index_of(&c, field::EQUIPMENT_TOKEN)],
            Value::Unsigned(2),
            "the last row in the payload wins"
        );
    }

    #[test]
    fn a_row_error_leaves_the_earlier_rows_applied() {
        // Faithful to `world_fold.rs::fold_air_variant_stats`, which validates and inserts
        // one row at a time and so leaves its own accumulator partially mutated
        // on a mid-payload rejection. Diverging would mean a caller that
        // continues past an error holds a state the recorder does not.
        let c = collection();
        let mut acc = Accumulator::new();
        let good = raw_row(1, 0, [1, 1, 1], [1; 10]);
        let bad = raw_row(0, 0, [2, 2, 2], [2; 10]); // zero variant_id
        assert!(acc
            .fold_payload(&c, &payload((100_000, 1_000), 0, &[good, bad]))
            .is_err());
        assert_eq!(acc.variants.len(), 1, "the row before the error was applied");
        assert!(acc.variants.contains_key(&key(&[1])));
    }

    #[test]
    fn a_row_keeps_the_scale_of_the_payload_that_carried_it() {
        // The consequence of header-carried scales meeting an accumulator that
        // never clears: a row inserted under one pair keeps that pair even after
        // a later header names another, because nothing re-states it. This is
        // why the scale words are per-row fields and not a per-frame constant.
        let c = collection();
        let mut acc = Accumulator::new();
        acc.fold_payload(
            &c,
            &payload((100_000, 1_000), 0, &[raw_row(1, 0, [0; 3], [500_000; 10])]),
        )
        .expect("folds");
        acc.fold_payload(
            &c,
            &payload((1_000, 100_000), 0, &[raw_row(2, 0, [0; 3], [500_000; 10])]),
        )
        .expect("folds");

        assert_eq!(
            acc.variants[&key(&[1])].values[index_of(&c, field::SPEED_CACHE)],
            Value::Continuous(5.0),
            "variant 1 stays on the 100_000 grid it arrived on"
        );
        assert_eq!(
            acc.variants[&key(&[2])].values[index_of(&c, field::SPEED_CACHE)],
            Value::Continuous(500.0)
        );
        assert_eq!(
            acc.variants[&key(&[1])].values[index_of(&c, field::FIXED_SCALE_RAW)],
            Value::Unsigned(100_000)
        );
        assert_eq!(
            acc.variants[&key(&[2])].values[index_of(&c, field::FIXED_SCALE_RAW)],
            Value::Unsigned(1_000)
        );
    }

    #[test]
    fn fold_payload_never_panics_on_arbitrary_bytes() {
        // The public boundary reads bytes that came off a disk; it must return a
        // diagnosis, never unwind.
        let c = collection();
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..4_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 300;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            let mut acc = Accumulator::new();
            let _ = acc.fold_payload(&c, &bytes);
            let _ = parse_header(&bytes);
            let _ = split_payload(&bytes);
            let _ = key_from_row(&bytes);
            let _ = record_from_row(&c, &bytes, 0, capture_scales());
        }
    }
}
