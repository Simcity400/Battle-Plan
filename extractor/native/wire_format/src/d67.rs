//! d67 `country_dynamic_modifiers_live` — the per-country active
//! dynamic-modifier **instance** set, schema-v2 (`DMI2`) grammar only.
//!
//! Derived from `world_fold.rs:13957`-`:14409`
//! (`fold_country_dynamic_modifiers` and the
//! `fold_country_dynamic_modifier_instances` it delegates to), the payload codec
// @world-fold-authority domain=67 functions=fold_country_dynamic_modifiers,fold_country_dynamic_modifier_instances lines=13957-14409 sha256=5b2eba3729d00a42e478e09dc13445adaa7b273bf1d61c9b1c00d38cb7b40a17 closure=44080462716952a9573b1e2910ea6595b22a1da3ffcb4be56465c2150d55ded9
//! `extractor/native/compact_contract/src/dynamic_modifiers.rs`, the collector
//! read at `extractor/native/collector/src/attach_log.rs:61148`-`:61371`, and the
//! entry decoder `extractor/native/lanes/country_dynamic_modifiers_live/timed_instance_exact.rs`.
//! Those four sources are the field-level basis.
//!
//! # Which grammar this module implements
//!
//! d67 carries **two** payload grammars and the discriminator is not in the
//! payload: it is the H4CR **record-header** flag
//! `COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2` (`world_fold.rs::fold_country_dynamic_modifiers`). Set → the
//! `DMI2` instance grammar (`world_fold.rs::fold_country_dynamic_modifiers`); clear → the legacy v1 id-set grammar
//! (`world_fold.rs::fold_country_dynamic_modifiers`), which carries only definition ids, no per-instance values,
//! no counter, and which the fold treats as a *different* history: folding one
//! invalidates the v2 lifecycle continuity outright (`world_fold.rs::fold_country_dynamic_modifiers`).
//!
//! **This port implements the v2 grammar only, and refuses a v1 record rather
//! than mapping it.** The reasons, in the recipe's terms:
//!
//! - v1 is not a shorter row of the same grammar (the d20 legacy-row case, which
//!   the recipe requires to be *mapped*). It is a second payload format with a
//!   different key space (country → id list), no `enabled`, no `values_raw` and
//!   no `remaining_days`; every value field of this schema would be unknown on
//!   it, and its own rows carry nothing this schema can key.
//! - The fold itself says the two are not interchangeable (`:9690`-`:9692`).
//! - The capture has **zero** v1 coverage: all 1033 d67 DATA records in
//!   `runs/batch3/deepwar_b2/collector_stream.bin` carry `record_flags`
//!   `0x80000000`. A v1 mapping would be written against no evidence at all.
//!
//! Refusal here is a **stream rejection**, never a silently skipped hour: see
//! [`decode_payload`]. The v1 grammar is reported as unported work, not modelled
//! as absent data.
//!
//! # Payload grammar (v2 `DMI2`)
//!
//! Transcribed from `compact_contract/src/dynamic_modifiers.rs:141`-`:310`
//! (`decode_payload`). **d67 is not admitted to the production-codec dependency
//! surface** (`Cargo.toml`; the same rule d22 states at `d22.rs:470` and d24 at
//! `d24.rs:999`), so the grammar is transcribed with
//! per-constant citations exactly as every other domain module transcribes its
//! fold's row layout. The duplication is not left on trust: `bench_real_d67`
//! *does* depend on `compact_contract` and asserts, on every one of the capture's
//! records and on a byte-level fuzz sweep, that [`decode_payload`] and
//! `compact_contract::dynamic_modifiers::decode_payload` accept and reject
//! exactly the same payloads and produce identical frames
//! (`bin/bench_real_d67.rs`, `the_two_decoders_agree_on_every_fuzzed_payload`).
//!
//! 104-byte header:
//!
//! | offset | width | field | note |
//! |---|---|---|---|
//! | `+0`  | 4×u8 | `MAGIC` = `DMI2` | `dynamic_modifiers.rs:23` |
//! | `+4`  | u16 | `schema_version` = 3 | `:24`, exact match required |
//! | `+6`  | u16 | `header_len` = 104 | `:25`, exact match required |
//! | `+8`  | u64 | `elapsed_wall_ms` | frame timing, not state |
//! | `+16` | u32 | `instance_count` | |
//! | `+20` | u32 | `id_blob_bytes` | |
//! | `+24` | u32 | `total_value_count` | |
//! | `+28` | u32 | `status_flags` | the failure/exclusion word |
//! | `+32` | u32 | `zero_removal_count` | event-continuity contract events — stop-reported, see below |
//! | `+36` | u32 | `authority_kind` = 1 | `:31` |
//! | `+40` | 32×u8 | `executable_sha256` | must equal a729, `:32` |
//! | `+72` | 32×u8 | `static_authority_receipt_sha256` | must equal, `:37` |
//!
//! Then `instance_count + zero_removal_count` fixed 28-byte rows — instances
//! first, zero-removal events after (`:235`) — then the concatenated id blob,
//! then the concatenated `i64` value vector:
//!
//! | offset | width | field | note |
//! |---|---|---|---|
//! | `+0`  | u32 | `owner_tag_db_id` | `1..=MAX_OWNER_TAG_DB_ID`, `:221`-`:224` |
//! | `+4`  | u32 | `id_offset` | must equal the running cursor, `:238` |
//! | `+8`  | u16 | `id_len` | `1..=128`, `:238` |
//! | `+10` | u16 | `flags` | bit0 `enabled`, bit1 `remaining_days present`, `:43`-`:45` |
//! | `+12` | u32 | `remaining_days_raw` | `<= i32::MAX` under the flag, `0` without it, `:255`-`:266` |
//! | `+16` | u32 | `value_offset` | must equal the running cursor, `:249` |
//! | `+20` | u32 | `value_count` | `<= 256`, `:249` |
//! | `+24` | u32 | `observed_game_hour_raw` | zero-removal rows only; `0` on an instance row, `:236` |
//!
//! **No field is scaled at this boundary, and that is the fold's own choice.**
//! The values are `i64` fixed-point over a `VALUE_SCALE` of `100_000`
//! (`dynamic_modifiers.rs:27`), and the fold emits them **raw**, carrying the
//! scale beside them as a constant (`world_fold.rs::fold_country_dynamic_modifier_instances`:
//! `"values_raw": instance.values_raw, "value_scale": dmi::VALUE_SCALE`). This is
//! the recipe's "fields emitted raw with no scale at all" case (d20's
//! `move_progress`), so [`field::VALUE_RAW`] is a `Signed` integer carrying the
//! engine's own word — lossless over the whole `i64` range, with no quantization
//! question and no binary64 round trip. `value_scale` itself is not a field: it
//! is a constant of the grammar, and re-encoding it would be a second copy of
//! state the decoder must keep consistent (recipe step 2).
//!
//! # Shape: two collections, and why the value vector is a child keyed set
//!
//! One instance carries a **variable-arity** `values_raw: Vec<i64>`. state-delta contract has no
//! variable-arity field, so the vector is either a child collection or a stop.
//! It is a child collection, keyed `(owner_tag_db_id, modifier_id,
//! value_ordinal)`, on the recipe's step-5 test: **the position must persist
//! across hours as the same thing**. It does, and the evidence is direct rather
//! than inferred —
//!
//! - The engine's slot layout is a property of the *definition*, not of the
//!   instance: `timed_instance_exact.rs` reads `value_begin`/`value_count` out of
//!   the entry and copies the vector verbatim (`:104`-`:118`); slot *i* is the
//!   *i*-th modifier effect of the definition named by `modifier_id`.
//! - Measured on the capture: across 1033 hours × 162 instances (167,346
//!   instance rows, 147 distinct `modifier_id`s), **no `modifier_id` is ever seen
//!   with two different arities**, and neither is any `(owner, modifier_id)`
//!   pair. Arity is a function of the modifier id alone — 0 for five ids, up to
//!   45 for `ITA_regio_esercito_3_dynamic_modifier`.
//!
//! That is the recipe's third ordinal case ("the rows have no identity of their
//! own, and the position IS one"), which puts the ordinal **in the key** — d11's
//! construction queue. A `Vec` is a map from position to element, so the ordinal
//! reproduces the container element for element, and an arity change is an
//! ordinary keyed-set add/tombstone rather than a re-slice. Nothing is refused:
//! the fold accepts any arity on any hour and so does this.
//!
//! `source_ordinal` is the other half of the ordinal question and it is a
//! **value field**, not a key component: the rows already have a fold-enforced
//! identity — `(owner_tag_db_id, modifier_id)`, unique by the decoder's own
//! `DuplicateIdentity` check (`dynamic_modifiers.rs:274`-`:281`) — and nothing
//! else ranks them, which is d11's production-lines case (`d11.rs:537`). The
//! fold carries it (`world_fold.rs::fold_country_dynamic_modifier_instances`, the `enumerate` index within
//! the owner's group in payload order) and it is **not** redundant with key
//! order: in 25,825 of the capture's 48,551 owner-hours the payload's
//! modifier-id sequence within one owner is *not* ascending, so dropping
//! `source_ordinal` would lose the emission order the fold publishes.
//!
//! # Four-state semantics
//!
//! `remaining_days` is the only field with an empty state, and it is
//! **Absent**, never Null and never Unknown:
//!
//! - The producer is exact. `read_timed_instance` maps the entry's `days_raw`
//!   word: `-1` → `None`, `>= 0` → `Some(n)`, anything else → `Err`
//!   (`timed_instance_exact.rs:96`-`:100`). `-1` is the engine's **permanent**
//!   sentinel, stated as such at `:53`-`:55`. `None` is therefore a measured
//!   fact about the modifier, not a failed read.
//! - It cannot be Unknown. Every read failure in this domain fails the **whole
//!   frame**: any `Err` from the entry decoder, any guard miss, any torn re-read
//!   sets `torn` and the collector clears every row and raises
//!   `COMPACT_STATUS_PARTIAL_DATA` (`attach_log.rs:61344`-`:61352`), which the
//!   fold turns into a retained-accumulator hour. There is no partial row and so
//!   no per-field "applicable, not captured" state anywhere in d67.
//! - Absent rather than Null, on the recipe's first question: a **permanent**
//!   modifier has no expiry countdown, so the concept does not apply to that row
//!   — d20's `move_target_province` on a division that is not moving, not d20's
//!   `owner` pointing at no country. The fold's JSON says `null` for it
//!   (`world_fold.rs::fold_country_dynamic_modifier_instances`, `Option<u32>` serialized), which per record-state contract decides
//!   value-vs-nothing and nothing else; the three-way choice is made here.
//!
//! Every other field is concrete in every row the grammar admits, so **no field
//! in either collection declares `.nullable()` or `.unknown_capable()`** — see
//! `the_schema_declares_exactly_the_states_the_mapper_emits`.
//!
//! # What this port does not carry
//!
//! - **`zero_removals` are event-continuity contract events and are stop-reported.** They are a
//!   happened-at-tick history — a synchronous zero observed inside the engine's
//!   erase path before the row is destroyed (`dynamic_modifiers.rs:60`-`:63`) —
//!   admitted only against the frame's own hour and against a lifecycle prior
//!   (`world_fold.rs::fold_country_dynamic_modifier_instances`). The event-continuity contract event codec is available, but this
//!   d67 event sub-stream remains independently unported and unevidenced.
//!   [`fold_payload`] therefore **refuses** a frame carrying one instead of
//!   dropping the events silently:
//!   dropping them would measure a shipping arm whose bytes include an event
//!   stream this arm never encoded. The capture carries **0** zero-removal rows
//!   in 1033 hours, so no measured hour is lost to the refusal.
//! - **`LifecycleTracker` transitions are not fields.** The fold derives them
//!   from consecutive observations of the carried state
//!   (`world_fold.rs::fold_country_dynamic_modifier_instances`, `dynamic_modifiers.rs:560`-`:628`), so they
//!   are the recipe's "value the fold hard-enforces as a function of carried
//!   state": a reader recomputes them from this schema's own instance sequence.
//!   The one input they need that state does not carry is the *continuity* flag,
//!   which is a stream-level gap signal. The gap contract codec exists, but d67's reader
//!   integration has not yet wired that continuity input — recorded as a
//!   caveat, not encoded.
//! - **`elapsed_wall_ms`** is frame timing, not state.
//!
//! # Hard d18 dependency
//!
//! The fold refuses a DMI2 frame outright unless the **current hour's** d18
//! `country_identity` roster has been folded (`world_fold.rs::fold_country_dynamic_modifier_instances`), and
//! then refuses it again if any instance's owner is missing from that roster
//! (`world_fold.rs::fold_country_dynamic_modifier_instances`). Both are modelled here rather than assumed: [`CountryRoster`]
//! is a typed input to [`record_from_instance_row`] and to [`fold_payload`], and
//! `None` reproduces "no roster this hour". The capture carries d18 on
//! 1033/1033 hours.
//!
//! # No derived field (derived-field contract)
//!
//! `remaining_days` is a *day* counter decremented on the owning country's daily
//! tick, whose phase is deliberately not encoded (`timed_instance_exact.rs:5`-`:9`),
//! so it is not a clock-linear function of the hourly frame tick and cannot be a
//! derived-field contract derived field. The mappers still take `frame_tick` so every domain's
//! boundary has one shape.

use std::collections::{BTreeMap, BTreeSet};

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 67;

/// The per-instance collection, keyed `(owner_tag_db_id, modifier_id)`.
pub const INSTANCES: u32 = 1;
/// The per-value-slot child collection, keyed
/// `(owner_tag_db_id, modifier_id, value_ordinal)`.
pub const VALUES: u32 = 2;

pub const INSTANCES_INDEX: usize = 0;
pub const VALUES_INDEX: usize = 1;
pub const COLLECTION_COUNT: usize = 2;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different
/// field. They are assigned per domain, so the two collections share the ids of
/// the concepts they share (d23's convention, `d23.rs:9`).
pub mod field {
    pub const OWNER_TAG_DB_ID: u32 = 1;
    pub const MODIFIER_ID: u32 = 2;
    pub const SOURCE_ORDINAL: u32 = 3;
    pub const ENABLED: u32 = 4;
    pub const REMAINING_DAYS: u32 = 5;
    pub const VALUE_ORDINAL: u32 = 6;
    pub const VALUE_RAW: u32 = 7;
}

// --- Payload grammar constants ----------------------------------------------
//
// Every one of these is transcribed from `compact_contract`, which this crate's
// library half does not depend on. `bench_real_d67`'s
// `every_transcribed_constant_equals_the_shipping_one` asserts each against its
// original, so a change there fails a test here rather than drifting.

/// `dynamic_modifiers.rs:23`.
pub const MAGIC: &[u8; 4] = b"DMI2";
/// `dynamic_modifiers.rs:24`. Any other version is refused (`:152`-`:154`).
pub const SCHEMA_VERSION: u16 = 3;
/// `dynamic_modifiers.rs:25`.
pub const HEADER_LEN: usize = 104;
/// `dynamic_modifiers.rs:26`.
pub const ROW_LEN: usize = 28;
/// `dynamic_modifiers.rs:27`. The denominator the raw `i64` values are stored
/// over. It is **not** applied here — the fold emits the raw word and carries
/// this constant beside it (`world_fold.rs::fold_country_dynamic_modifier_instances`).
pub const VALUE_SCALE: i64 = 100_000;
/// `dynamic_modifiers.rs:28`.
pub const MAX_INSTANCES: usize = 262_144;
/// `dynamic_modifiers.rs:29`.
pub const MAX_VALUES_PER_INSTANCE: usize = 256;
/// `dynamic_modifiers.rs:30`.
pub const MAX_MODIFIER_ID_LEN: usize = 128;
/// `dynamic_modifiers.rs:31`.
pub const AUTHORITY_KIND_EXACT_A729: u32 = 1;
/// `compact_contract/src/build_profile.rs`'s `MAX_COMPACT_OWNER_TAG_DB_ID`.
///
/// Was `2_048`, a guess at the vanilla-plus-DLC country count.
/// ENGINE-STRUCTURAL now: `id < *(int32*)(db + 0x0C)` with a signed-`int` size
/// gives `i32::MAX - 1`. Pinned by
/// `tests/wire_format_suite/producer_consumer_admission_agreement.rs`.
pub const MAX_OWNER_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;
/// `compact_contract/src/record_header.rs:10`.
pub const MAX_PAYLOAD_LEN: usize = 1 << 28;

/// `dynamic_modifiers.rs:32` — the exact-a729 executable hash. A frame carrying
/// any other producer authority is refused (`:172`-`:174`).
pub const A729_EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];
/// `dynamic_modifiers.rs:37` — the static-authority receipt hash.
pub const STATIC_AUTHORITY_RECEIPT_SHA256: [u8; 32] = [
    0xbd, 0xba, 0xc3, 0xcb, 0x11, 0x58, 0xc9, 0x71, 0xb4, 0x08, 0x58, 0x66, 0x11, 0xd6, 0xab, 0xa6,
    0x18, 0xec, 0xfe, 0x85, 0xb5, 0x26, 0x5e, 0x3b, 0x9e, 0x8c, 0x2c, 0x0b, 0xf5, 0x45, 0xe4, 0x15,
];

/// `dynamic_modifiers.rs:43`.
pub const FLAG_ENABLED: u16 = 1;
/// `dynamic_modifiers.rs:44`.
pub const FLAG_REMAINING_DAYS: u16 = 1 << 1;
/// `dynamic_modifiers.rs:45`. Any other bit is a hard error (`:228`-`:230`) — an
/// unrecognised flag word means the row is not this grammar.
pub const KNOWN_FLAGS: u16 = FLAG_ENABLED | FLAG_REMAINING_DAYS;

// --- content-identity contract symbols --------------------------------------------------------------

/// The `modifier_id` vocabulary, as a symbol table.
///
/// content-identity contract: "Country tags, equipment names, province and state ids are
/// content-defined. The stream carries manifest ids as varints, never text"
/// A dynamic-modifier definition id is a mod-set
/// content token — `GER_wunderwaffen_modifier` names whatever the loaded content
/// defines — so it travels as a `ContentId` resolved through a table the caller
/// supplies, and because `ContentId` is a legal key kind
/// (`schema.rs:29`-`:31`) the text-keyed collection becomes a `ContentId`-keyed
/// one with no new shape. **This is d29's rule and d29's implementation**
/// (`d29.rs:205`); see that module for the full argument.
///
/// The production assignment now lives in
/// `portable_content_manifest::symbol_manifest`; this domain-local mapper still
/// awaits the file-global resolver retrofit. [`Symbols::from_vocabulary`] uses a
/// measurement stand-in that is
/// deterministic, order-independent and derived only from the content set — the
/// 0-based index into the sorted, deduplicated list — and it is labelled a
/// stand-in, not an answer.
///
/// **The invariance claim is narrower here than in d29, and the difference is
/// measured.** d29's vocabulary is under 128 symbols, so every assignment costs
/// one varint byte and its measurement is assignment-invariant. d67's capture
/// vocabulary is **147** symbols: ids `0..=127` are one-byte varints and
/// `128..=146` are two-byte, so a different assignment moves which symbols pay
/// the second byte. The measurement therefore reports the stand-in's cost **and
/// the exact band** over all assignments of this vocabulary
/// (`bin/bench_real_d67.rs` prints the best- and worst-case symbol-id byte
/// totals). content-identity contract's ids are file-global, so an assignment over the whole content
/// set can widen the varint further; that is claimed nowhere.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Symbols {
    /// Sorted and deduplicated; the id of a symbol is its index.
    vocabulary: Vec<String>,
}

impl Symbols {
    /// A table with no symbols. Any row is rejected against it — fail-closed, so
    /// an unresolvable symbol is a diagnosed error and never a row with the
    /// modifier id quietly dropped.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build from every modifier id the content set can produce. The input is
    /// sorted and deduplicated here, so callers may pass it in any order and two
    /// callers over the same content set always mint the same ids.
    pub fn from_vocabulary<I, S>(symbols: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut vocabulary: Vec<String> = symbols.into_iter().map(Into::into).collect();
        vocabulary.sort();
        vocabulary.dedup();
        Self { vocabulary }
    }

    pub fn len(&self) -> usize {
        self.vocabulary.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vocabulary.is_empty()
    }

    /// The symbol id of `symbol`, or `None` if the content set does not contain
    /// it. `None` is an error at the boundary, never an `Unknown` value: the
    /// string WAS captured, so claiming it was not would be a false statement
    /// about the capture.
    pub fn id(&self, symbol: &str) -> Option<u64> {
        self.vocabulary
            .binary_search_by(|candidate| candidate.as_str().cmp(symbol))
            .ok()
            .map(|index| index as u64)
    }

    /// The symbol an id names, for a reader replaying the stream.
    pub fn symbol(&self, id: u64) -> Option<&str> {
        usize::try_from(id)
            .ok()
            .and_then(|index| self.vocabulary.get(index))
            .map(String::as_str)
    }
}

// --- The d18 roster, as a typed input ----------------------------------------

/// The current hour's d18 `country_identity` roster, reduced to the one column
/// d67's admission reads: the set of live `tag_db_id`s
/// (`world_fold.rs::fold_country_dynamic_modifier_instances`, `WorldFold::tag_by_db_id`).
///
/// It is an **input**, never re-derived: d67's fold hard-requires a
/// current-hour roster (`world_fold.rs::fold_country_dynamic_modifier_instances`) and a caller that guessed one would
/// admit instances the recorder refuses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryRoster {
    ids: BTreeSet<u32>,
}

impl CountryRoster {
    pub fn from_ids<I: IntoIterator<Item = u32>>(ids: I) -> Self {
        Self {
            ids: ids.into_iter().collect(),
        }
    }

    pub fn contains(&self, tag_db_id: u32) -> bool {
        self.ids.contains(&tag_db_id)
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

// --- Schema ------------------------------------------------------------------

pub fn schema() -> Schema {
    use field as f;

    let instances = Collection {
        collection_id: INSTANCES,
        // Full replace every frame (`world_fold.rs::fold_country_dynamic_modifier_instances` clears every
        // known country's list before writing the measured ones), but
        // `(owner_tag_db_id, modifier_id)` is a stable identity that persists
        // across hours — which is what a keyed set needs, and only a keyed set
        // has a delta grammar (collection-shape contract). Measured: 162 instances an hour for 1033
        // hours with 13 changed rows in total.
        shape: Shape::KeyedSet,
        fields: vec![
            // The owning country. `ContentId` because a country tag is
            // content-defined (content-identity contract) — the same identity d63 keys by
            // (`d63.rs:127`) and d20 classes as `ContentId`.
            Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).key(0),
            // The modifier definition id. Content-defined text → a `ContentId`
            // through [`Symbols`] (content-identity contract, step 3's rule; d29 is the worked
            // example).
            Field::new(f::MODIFIER_ID, Kind::ContentId).key(1),
            // The instance's position in its owner's emission order
            // (`world_fold.rs::fold_country_dynamic_modifier_instances`). A concrete required field beside the
            // fold-enforced key — d11's `line_ordinal` case (`d11.rs:537`).
            Field::new(f::SOURCE_ORDINAL, Kind::Unsigned),
            // `SDynamicModifierEntry+0x20`, exactly 0 or 1 or the read fails
            // (`timed_instance_exact.rs:91`-`:95`).
            Field::new(f::ENABLED, Kind::Boolean),
            // The engine's daily countdown. `Absent` — not Null, not Unknown —
            // when the entry carries the permanent `-1` sentinel; see the module
            // docs. `Unsigned` rather than `Monotone`: it counts *down*.
            Field::new(f::REMAINING_DAYS, Kind::Unsigned).optional(),
        ],
    };

    let values = Collection {
        collection_id: VALUES,
        // The instance's `values_raw` vector, one row per slot. A keyed set
        // because the slot index is the modifier definition's own value slot and
        // persists across hours — see the module docs for the measurement.
        shape: Shape::KeyedSet,
        fields: vec![
            Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).key(0),
            Field::new(f::MODIFIER_ID, Kind::ContentId).key(1),
            // The slot's position in the definition's value vector, generated at
            // this boundary as a dense ordinal (recipe step 5's third case).
            // `Unsigned`: it is an array index, not content.
            Field::new(f::VALUE_ORDINAL, Kind::Unsigned).key(2),
            // The engine's raw fixed-point word, emitted unscaled by the fold
            // (`world_fold.rs::fold_country_dynamic_modifier_instances`). `Signed` carries the whole `i64` range
            // losslessly; there is no quantization decision to make because the
            // field is not `Continuous`. Measured range on the capture:
            // -1,500,000 .. 20,000,000 over 65 distinct values.
            Field::new(f::VALUE_RAW, Kind::Signed),
        ],
    };

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour,
        // and the fold enforces at most one DMI2 frame per hour
        // (`world_fold.rs::fold_country_dynamic_modifier_instances`).
        cadence_log2: 0,
        collections: vec![instances, values],
        // event-continuity contract event bodies have no implementation in this crate (`lib.rs:13`) and
        // `zero_removals` are stop-reported — see the module docs.
        event_part: None,
    }
}

// --- The decoded payload -----------------------------------------------------

/// The 104-byte `DMI2` header (`dynamic_modifiers.rs:151`-`:174`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub instance_count: u32,
    pub id_blob_bytes: u32,
    pub total_value_count: u32,
    pub status_flags: u32,
    pub zero_removal_count: u32,
}

/// One current dynamic-modifier instance
/// (`dynamic_modifiers.rs:49`-`:58`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Instance {
    pub owner_tag_db_id: u32,
    pub modifier_id: String,
    pub enabled: bool,
    pub values_raw: Vec<i64>,
    /// `None` is the engine's exact `-1` permanent sentinel; `Some(0)` is a
    /// measured zero and is never collapsed into absence.
    pub remaining_days: Option<u32>,
}

/// One synchronous zero/removal **event** (`dynamic_modifiers.rs:64`-`:72`).
/// Decoded so the structural checks match the shipping decoder exactly, and then
/// stop-reported: [`fold_payload`] refuses a frame that carries any.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZeroRemoval {
    pub owner_tag_db_id: u32,
    pub modifier_id: String,
    pub enabled: bool,
    pub values_raw: Vec<i64>,
    pub observed_game_hour_raw: i32,
}

/// A whole decoded `DMI2` payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub header: PayloadHeader,
    pub instances: Vec<Instance>,
    pub zero_removals: Vec<ZeroRemoval>,
}

/// Decode one d67 DATA payload — this domain's analogue of the recipe's
/// `split_payload`.
///
/// The recipe's header splitter exists so a caller can *classify* a record
/// without paying for its body. d67's fold cannot be split that way: it decodes
/// the entire payload (`world_fold.rs::fold_country_dynamic_modifier_instances`) and checks the producer authority
/// (`world_fold.rs::fold_country_dynamic_modifier_instances`) **before** it consults the status word (`world_fold.rs::fold_country_dynamic_modifier_instances`), which is d83's
/// shape. So the structural boundary here is a full decode, there is
/// deliberately no header-only entry point, and a malformed payload is a refused
/// stream whatever its status.
///
/// `schema_v2` is the caller's reading of the H4CR record header's
/// `COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2` bit (`world_fold.rs::fold_country_dynamic_modifiers`). It is a
/// parameter rather than a bit tested here because that constant belongs to
/// `compact_contract`, which this crate's library half deliberately does not
/// depend on — d22's ruling (`d22.rs:470`). A clear flag selects the legacy v1
/// grammar, which this port does not implement, so it is an `Err`: see the
/// module docs for why v1 is refused rather than mapped.
///
/// Fail-closed and total: no byte slice of any length or content panics.
pub fn decode_payload(
    payload: &[u8],
    schema_v2: bool,
) -> core::result::Result<Frame, String> {
    if !schema_v2 {
        // `world_fold.rs::fold_country_dynamic_modifiers`. Not an hour to skip and not a row to
        // map: a grammar this port does not carry.
        return Err(
            "country_dynamic_modifiers record does not carry COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2; \
             the legacy v1 id-set grammar is not implemented by this port"
                .to_string(),
        );
    }
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "country_dynamic_modifiers DMI2 payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(format!(
            "country_dynamic_modifiers DMI2 payload {} exceeds the {MAX_PAYLOAD_LEN}-byte bound",
            payload.len()
        ));
    }
    // `world_fold.rs::fold_country_dynamic_modifiers` and `dynamic_modifiers.rs:136`: the magic is checked,
    // but it never *selects* the grammar — the record flag did that.
    if &payload[..4] != MAGIC.as_slice() {
        return Err("country_dynamic_modifiers schema-v2 record omitted DMI2 magic".to_string());
    }
    let version = u16_at(payload, 4);
    if version != SCHEMA_VERSION {
        return Err(format!(
            "country_dynamic_modifiers DMI2 schema version {version} != {SCHEMA_VERSION}"
        ));
    }
    let header_len = u16_at(payload, 6);
    if usize::from(header_len) != HEADER_LEN {
        return Err(format!(
            "country_dynamic_modifiers DMI2 header length {header_len} != {HEADER_LEN}"
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64_at(payload, 8),
        instance_count: u32_at(payload, 16),
        id_blob_bytes: u32_at(payload, 20),
        total_value_count: u32_at(payload, 24),
        status_flags: u32_at(payload, 28),
        zero_removal_count: u32_at(payload, 32),
    };
    let authority_kind = u32_at(payload, 36);
    if authority_kind != AUTHORITY_KIND_EXACT_A729 {
        return Err(format!(
            "country_dynamic_modifiers DMI2 authority kind {authority_kind} is not \
             {AUTHORITY_KIND_EXACT_A729}"
        ));
    }
    // `dynamic_modifiers.rs:168`-`:174`, and again at `world_fold.rs::fold_country_dynamic_modifier_instances`: any
    // other producer authority is refused outright.
    if payload[40..72] != A729_EXECUTABLE_SHA256 || payload[72..104] != STATIC_AUTHORITY_RECEIPT_SHA256
    {
        return Err("country_dynamic_modifiers DMI2 producer authority rejected".to_string());
    }

    let instance_count = header.instance_count as usize;
    let zero_removal_count = header.zero_removal_count as usize;
    if instance_count > MAX_INSTANCES {
        return Err(format!(
            "country_dynamic_modifiers DMI2 instance_count {instance_count} exceeds {MAX_INSTANCES}"
        ));
    }
    if zero_removal_count > MAX_INSTANCES {
        return Err(format!(
            "country_dynamic_modifiers DMI2 zero_removal_count {zero_removal_count} exceeds \
             {MAX_INSTANCES}"
        ));
    }
    let total_row_count = instance_count
        .checked_add(zero_removal_count)
        .ok_or_else(|| "country_dynamic_modifiers DMI2 row count overflow".to_string())?;
    if total_row_count > MAX_INSTANCES {
        return Err(format!(
            "country_dynamic_modifiers DMI2 total row count {total_row_count} exceeds \
             {MAX_INSTANCES}"
        ));
    }
    let id_blob_bytes = header.id_blob_bytes as usize;
    let total_value_count = header.total_value_count as usize;
    let maximum_values = total_row_count
        .checked_mul(MAX_VALUES_PER_INSTANCE)
        .ok_or_else(|| "country_dynamic_modifiers DMI2 value bound overflow".to_string())?;
    if total_value_count > maximum_values {
        return Err(format!(
            "country_dynamic_modifiers DMI2 total_value_count {total_value_count} exceeds \
             {maximum_values}"
        ));
    }
    // Checked throughout: these are attacker-shaped words from a file on disk.
    let rows_end = total_row_count
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| "country_dynamic_modifiers DMI2 row table overflow".to_string())?;
    let ids_end = rows_end
        .checked_add(id_blob_bytes)
        .ok_or_else(|| "country_dynamic_modifiers DMI2 id blob overflow".to_string())?;
    let values_bytes = total_value_count
        .checked_mul(8)
        .ok_or_else(|| "country_dynamic_modifiers DMI2 value block overflow".to_string())?;
    let expected_len = ids_end
        .checked_add(values_bytes)
        .ok_or_else(|| "country_dynamic_modifiers DMI2 payload length overflow".to_string())?;
    if payload.len() != expected_len {
        return Err(format!(
            "country_dynamic_modifiers DMI2 payload length {} != {expected_len}",
            payload.len()
        ));
    }

    let ids = &payload[rows_end..ids_end];
    let values = &payload[ids_end..];
    let mut expected_id_offset = 0usize;
    let mut expected_value_offset = 0usize;
    let mut current_identities: BTreeSet<(u32, &str)> = BTreeSet::new();
    let mut zero_identities: BTreeSet<(u32, &str)> = BTreeSet::new();
    let mut instances = Vec::with_capacity(instance_count);
    let mut zero_removals = Vec::with_capacity(zero_removal_count);
    for row in 0..total_row_count {
        let base = HEADER_LEN + row * ROW_LEN;
        let owner = u32_at(payload, base);
        // `dynamic_modifiers.rs:221`-`:224`.
        if owner == 0 || owner > MAX_OWNER_TAG_DB_ID {
            return Err(format!(
                "country_dynamic_modifiers DMI2 row {row} owner {owner} out of range"
            ));
        }
        let id_offset = u32_at(payload, base + 4) as usize;
        let id_len = usize::from(u16_at(payload, base + 8));
        let flags = u16_at(payload, base + 10);
        if flags & !KNOWN_FLAGS != 0 {
            return Err(format!(
                "country_dynamic_modifiers DMI2 row {row} has unknown flags {flags:#x}"
            ));
        }
        let remaining_raw = u32_at(payload, base + 12);
        let value_offset = u32_at(payload, base + 16) as usize;
        let value_count = u32_at(payload, base + 20) as usize;
        let event_hour_raw = u32_at(payload, base + 24);
        // `dynamic_modifiers.rs:235`: instance rows come first, zero-removal
        // events after; the partition is positional, not flagged.
        let is_zero_removal = row >= instance_count;
        // `:236`-`:237`: an instance row must zero-pack the event hour.
        if !is_zero_removal && event_hour_raw != 0 {
            return Err(format!(
                "country_dynamic_modifiers DMI2 instance row {row} carries an event hour"
            ));
        }
        // `:238`-`:240`: the id windows must be exactly contiguous.
        if id_offset != expected_id_offset || id_len == 0 || id_len > MAX_MODIFIER_ID_LEN {
            return Err(format!(
                "country_dynamic_modifiers DMI2 row {row} id window [{id_offset},+{id_len}) is \
                 not the canonical next window"
            ));
        }
        let id_end = id_offset
            .checked_add(id_len)
            .ok_or_else(|| format!("country_dynamic_modifiers DMI2 row {row} id window overflow"))?;
        let id_bytes = ids.get(id_offset..id_end).ok_or_else(|| {
            format!("country_dynamic_modifiers DMI2 row {row} id window exceeds the blob")
        })?;
        // `:243`-`:248`: valid UTF-8 and no interior NUL.
        let modifier_id = std::str::from_utf8(id_bytes)
            .ok()
            .filter(|id| !id.as_bytes().contains(&0))
            .ok_or_else(|| format!("country_dynamic_modifiers DMI2 row {row} id is not utf8"))?;
        // `:249`-`:254`: the value windows must be exactly contiguous too.
        if value_offset != expected_value_offset || value_count > MAX_VALUES_PER_INSTANCE {
            return Err(format!(
                "country_dynamic_modifiers DMI2 row {row} value window [{value_offset},+\
                 {value_count}) is not the canonical next window"
            ));
        }
        let value_end = value_offset.checked_add(value_count).ok_or_else(|| {
            format!("country_dynamic_modifiers DMI2 row {row} value window overflow")
        })?;
        if value_end > total_value_count {
            return Err(format!(
                "country_dynamic_modifiers DMI2 row {row} value window exceeds \
                 {total_value_count}"
            ));
        }
        // `:255`-`:266`: the counter is meaningful only under its flag, and must
        // be zero-packed without it. `-1` never reaches the wire — the producer
        // has already turned the permanent sentinel into a clear flag
        // (`timed_instance_exact.rs:96`-`:100`).
        let remaining_days = if flags & FLAG_REMAINING_DAYS != 0 {
            if remaining_raw > i32::MAX as u32 {
                return Err(format!(
                    "country_dynamic_modifiers DMI2 row {row} remaining_days {remaining_raw} out \
                     of range"
                ));
            }
            Some(remaining_raw)
        } else {
            if remaining_raw != 0 {
                return Err(format!(
                    "country_dynamic_modifiers DMI2 row {row} carries remaining_days without its \
                     flag"
                ));
            }
            None
        };
        let mut values_raw = Vec::with_capacity(value_count);
        for value_index in value_offset..value_end {
            let offset = value_index * 8;
            values_raw.push(i64::from_le_bytes(
                values[offset..offset + 8]
                    .try_into()
                    .expect("value window is inside the checked payload"),
            ));
        }
        // `:274`-`:281`: an identity may not repeat within its partition.
        let identities = if is_zero_removal {
            &mut zero_identities
        } else {
            &mut current_identities
        };
        if !identities.insert((owner, modifier_id)) {
            return Err(format!(
                "country_dynamic_modifiers DMI2 repeats identity ({owner}, {modifier_id})"
            ));
        }
        if is_zero_removal {
            // `:283`-`:285`: an event row is a *measured* zero by construction.
            if remaining_days != Some(0) {
                return Err(format!(
                    "country_dynamic_modifiers DMI2 zero-removal row {row} is not a measured zero"
                ));
            }
            zero_removals.push(ZeroRemoval {
                owner_tag_db_id: owner,
                modifier_id: modifier_id.to_string(),
                enabled: flags & FLAG_ENABLED != 0,
                values_raw,
                observed_game_hour_raw: event_hour_raw as i32,
            });
        } else {
            instances.push(Instance {
                owner_tag_db_id: owner,
                modifier_id: modifier_id.to_string(),
                enabled: flags & FLAG_ENABLED != 0,
                values_raw,
                remaining_days,
            });
        }
        expected_id_offset = id_end;
        expected_value_offset = value_end;
    }
    // `:304`-`:306`: the windows must cover the blob and the value block exactly.
    if expected_id_offset != id_blob_bytes || expected_value_offset != total_value_count {
        return Err(format!(
            "country_dynamic_modifiers DMI2 windows cover {expected_id_offset}/{id_blob_bytes} id \
             bytes and {expected_value_offset}/{total_value_count} values"
        ));
    }
    Ok(Frame {
        header,
        instances,
        zero_removals,
    })
}

// --- Key extractors ----------------------------------------------------------

/// The key of one instance: `(owner_tag_db_id, modifier_id)`, the identity the
/// decoder enforces as unique (`dynamic_modifiers.rs:274`-`:281`) and the fold
/// groups by (`world_fold.rs::fold_country_dynamic_modifier_instances`).
pub fn key_from_instance_row(
    instance: &Instance,
    symbols: &Symbols,
) -> core::result::Result<Key, String> {
    Ok(key(&[
        instance.owner_tag_db_id as u64,
        symbol_id(&instance.modifier_id, symbols)?,
    ]))
}

/// The key of one value slot. The child's identity includes its parent's, so it
/// takes the parent's components first — d23's and d83's convention
/// (`d23.rs:576`, `d83.rs:627`) — and a caller that already keyed the instance
/// does not re-resolve the symbol by accident.
pub fn key_from_value_row(
    owner_tag_db_id: u32,
    modifier_id: &str,
    value_ordinal: u32,
    symbols: &Symbols,
) -> core::result::Result<Key, String> {
    Ok(key(&[
        owner_tag_db_id as u64,
        symbol_id(modifier_id, symbols)?,
        value_ordinal as u64,
    ]))
}

fn symbol_id(modifier_id: &str, symbols: &Symbols) -> core::result::Result<u64, String> {
    symbols.id(modifier_id).ok_or_else(|| {
        format!("country_dynamic_modifiers modifier id {modifier_id:?} is not in the symbol table")
    })
}

// --- The boundary mapping ----------------------------------------------------

/// One instance of the `instances` table, as the collector emits it today, onto
/// its stored Record.
///
/// `instance` is one decoded row (the caller has already consumed the payload
/// header via [`decode_payload`]); d67 has exactly one instance-row form, so
/// length selects nothing.
///
/// The typed inputs beyond the fixed three, each because the fold reads
/// something the row does not carry:
///
/// - `source_ordinal` — the instance's position within its owner's group in
///   payload order, which the fold carries as data
///   (`world_fold.rs::fold_country_dynamic_modifier_instances`). Generated at the boundary and counted in
///   exactly one place, so no caller re-derives it (recipe step 5).
/// - `roster` — the current hour's d18 `country_identity` roster. The fold
///   refuses an instance whose owner is not in it (`world_fold.rs::fold_country_dynamic_modifier_instances`).
///
/// `frame_tick` is derived-field contract's derived-field input; d67 declares no derived field, so
/// it is taken and ignored — every domain's mapper has the same shape.
///
/// Fail-closed throughout: an instance the shipping decoder would have rejected
/// is an `Err` here, never a coerced value. Nothing in this function panics.
pub fn record_from_instance_row(
    collection: &Collection,
    instance: &Instance,
    source_ordinal: u32,
    frame_tick: u64,
    roster: &CountryRoster,
) -> core::result::Result<Record, String> {
    // d67 declares no derived field; derived-field contract's transform input is unused. Named
    // rather than `_frame_tick` because the signature is the cross-domain
    // contract.
    let _ = frame_tick;
    if collection.collection_id != INSTANCES {
        return Err(format!(
            "country_dynamic_modifiers instance mapper called with collection {}",
            collection.collection_id
        ));
    }
    check_instance(instance)?;
    // `world_fold.rs::fold_country_dynamic_modifier_instances`.
    if !roster.contains(instance.owner_tag_db_id) {
        return Err(format!(
            "country_dynamic_modifiers DMI2 instance owner {} has no current country_identity \
             mapping",
            instance.owner_tag_db_id
        ));
    }

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::SOURCE_ORDINAL => Value::Unsigned(source_ordinal as u64),
            field::ENABLED => Value::Bool(instance.enabled),
            // Absent, not Null and not Unknown: a permanent modifier has no
            // expiry countdown, so the concept does not apply to this row. See
            // the module docs for the producer evidence.
            field::REMAINING_DAYS => match instance.remaining_days {
                Some(days) => Value::Unsigned(days as u64),
                None => Value::Absent,
            },
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!(
                    "country_dynamic_modifiers instances has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// One value slot of the `values` child table onto its stored Record.
///
/// `value_ordinal` indexes the instance's `values_raw` vector; the slot's raw
/// `i64` is carried unscaled, exactly as the fold emits it
/// (`world_fold.rs::fold_country_dynamic_modifier_instances`).
pub fn record_from_value_row(
    collection: &Collection,
    instance: &Instance,
    value_ordinal: u32,
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    let _ = frame_tick;
    if collection.collection_id != VALUES {
        return Err(format!(
            "country_dynamic_modifiers value mapper called with collection {}",
            collection.collection_id
        ));
    }
    check_instance(instance)?;
    let raw = usize::try_from(value_ordinal)
        .ok()
        .and_then(|index| instance.values_raw.get(index).copied())
        .ok_or_else(|| {
            format!(
                "country_dynamic_modifiers value ordinal {value_ordinal} is outside the {}-slot \
                 vector of ({}, {})",
                instance.values_raw.len(),
                instance.owner_tag_db_id,
                instance.modifier_id
            )
        })?;

    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            field::VALUE_RAW => Value::Signed(raw),
            other => {
                return Err(format!(
                    "country_dynamic_modifiers values has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

/// Every invariant [`decode_payload`] enforces on an instance, re-checked at the
/// mapper because these entry points are public and a hand-built `Instance` must
/// not become a record the shipping decoder would refuse.
fn check_instance(instance: &Instance) -> core::result::Result<(), String> {
    if instance.owner_tag_db_id == 0 || instance.owner_tag_db_id > MAX_OWNER_TAG_DB_ID {
        return Err(format!(
            "country_dynamic_modifiers owner {} out of range",
            instance.owner_tag_db_id
        ));
    }
    if instance.modifier_id.is_empty()
        || instance.modifier_id.len() > MAX_MODIFIER_ID_LEN
        || instance.modifier_id.as_bytes().contains(&0)
    {
        return Err(format!(
            "country_dynamic_modifiers modifier id {:?} is not a legal id",
            instance.modifier_id
        ));
    }
    if instance.values_raw.len() > MAX_VALUES_PER_INSTANCE {
        return Err(format!(
            "country_dynamic_modifiers instance carries {} values, more than \
             {MAX_VALUES_PER_INSTANCE}",
            instance.values_raw.len()
        ));
    }
    if instance
        .remaining_days
        .is_some_and(|days| days > i32::MAX as u32)
    {
        return Err("country_dynamic_modifiers remaining_days out of range".to_string());
    }
    Ok(())
}

// --- The frame-level fold ----------------------------------------------------

/// The state one d67 hour holds: the two collections, in schema order.
///
/// d67 is **full replace** — the fold clears every known country's lists and
/// writes the measured ones (`world_fold.rs::fold_country_dynamic_modifier_instances`) — so one usable
/// record *is* one state and there is no accumulator to carry across hours. The
/// nested walk still lives here rather than in the measurement harness so the
/// tests, the harness and any future caller share one derivation of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub instances: KeyedState,
    pub values: KeyedState,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// The two collections in schema order, for `encode_baseline`/`encode_delta`.
    pub fn as_slice(&self) -> [KeyedState; COLLECTION_COUNT] {
        [self.instances.clone(), self.values.clone()]
    }

    pub fn get(&self, index: usize) -> &KeyedState {
        match index {
            INSTANCES_INDEX => &self.instances,
            _ => &self.values,
        }
    }

    pub fn rows(&self) -> usize {
        self.instances.len() + self.values.len()
    }
}

/// Fold one **clean** decoded d67 frame into its hour's state, exactly as
/// `fold_country_dynamic_modifier_instances` does from `world_fold.rs::fold_country_dynamic_modifier_instances`
/// onward.
///
/// **The caller applies the failure-status exclusion first.**
/// `handle_compact_failure` (`:9853`-`:9861`) retains the accumulator and folds
/// nothing before any of the checks reproduced here, so calling this on a
/// failure-status record would report an error for a record the recorder simply
/// drops. That order — decode, authority, *then* status — is the fold's own and
/// is why [`decode_payload`] is a separate entry point.
///
/// Typed inputs, each mirroring something the fold reads:
///
/// - `frame_hour` — the fold's **own clock**, `hour_index_from_raw(game_hour_raw)`
///   as `apply_domain_data` computes it (`world_fold.rs::apply_domain_data`), not derived-field contract's
///   `frame_tick`. `None` reproduces the fold's own refusal (`:9867`-`:9868`),
///   which is also what the generic `fold_domain_data` dispatcher hands it
///   (`world_fold.rs::fold_domain_data`).
/// - `roster` — the current hour's d18 roster; `None` reproduces "no d18 record
///   folded this hour" (`:9869`-`:9874`).
/// - `symbols` — content-identity contract symbol resolution for the content-defined `modifier_id`.
///
/// **Error contract: all-or-nothing.** The returned state is built in full
/// before anything is published, mirroring the fold, whose own errors all
/// precede its first mutation (`world_fold.rs::fold_country_dynamic_modifier_instances` vs `world_fold.rs::fold_country_dynamic_modifier_instances`).
pub fn fold_payload(
    schema: &Schema,
    frame: &Frame,
    frame_hour: Option<i64>,
    roster: Option<&CountryRoster>,
    symbols: &Symbols,
    frame_tick: u64,
) -> core::result::Result<State, String> {
    if schema.collections.len() != COLLECTION_COUNT {
        return Err(format!(
            "country_dynamic_modifiers schema has {} collections, expected {COLLECTION_COUNT}",
            schema.collections.len()
        ));
    }
    // `world_fold.rs::fold_country_dynamic_modifier_instances`.
    let frame_hour = frame_hour
        .ok_or_else(|| "country_dynamic_modifiers DMI2 has invalid frame hour".to_string())?;
    // `:9869`-`:9874`.
    let roster = roster.ok_or_else(|| {
        "country_dynamic_modifiers DMI2 requires a current-hour country_identity roster".to_string()
    })?;

    // `:9876`-`:9888`: group by owner, refusing an owner the roster does not
    // carry. `BTreeMap` reproduces the fold's own container, so the owner order
    // and the within-owner payload order are the fold's.
    let mut per_owner: BTreeMap<u32, Vec<&Instance>> = BTreeMap::new();
    for instance in &frame.instances {
        if !roster.contains(instance.owner_tag_db_id) {
            return Err(format!(
                "country_dynamic_modifiers DMI2 instance owner {} has no current country_identity \
                 mapping",
                instance.owner_tag_db_id
            ));
        }
        per_owner
            .entry(instance.owner_tag_db_id)
            .or_default()
            .push(instance);
    }

    // `:9889`-`:9905`: the synchronous zero/removal admission. Mirrored first so
    // a malformed event is diagnosed as the fold diagnoses it, and only then
    // stop-reported.
    if !frame.zero_removals.is_empty() {
        let current_identities: BTreeSet<(u32, &str)> = frame
            .instances
            .iter()
            .map(|instance| (instance.owner_tag_db_id, instance.modifier_id.as_str()))
            .collect();
        for event in &frame.zero_removals {
            if !roster.contains(event.owner_tag_db_id)
                || hour_index_from_raw(event.observed_game_hour_raw) != Some(frame_hour)
                || current_identities.contains(&(event.owner_tag_db_id, event.modifier_id.as_str()))
            {
                return Err(
                    "country_dynamic_modifiers synchronous zero-removal event failed \
                     owner/hour/current-set admission"
                        .to_string(),
                );
            }
        }
        // The d67 event-port stop. The generic event-continuity contract grammar exists, but mapping
        // this frame while dropping its still-unported synchronous events would
        // charge this arm's size against a shipping arm that carries them.
        return Err(format!(
            "country_dynamic_modifiers DMI2 frame carries {} synchronous zero-removal event(s); \
             the d67 lifecycle event sub-stream is unported and unevidenced, so this port \
             stop-reports it",
            frame.zero_removals.len()
        ));
    }

    let instances_collection = &schema.collections[INSTANCES_INDEX];
    let values_collection = &schema.collections[VALUES_INDEX];
    let mut state = State::new();
    for (owner, instances) in &per_owner {
        // `:10000`-`:10013`: `source_ordinal` is the enumerate index within this
        // owner's group, in payload order.
        for (source_ordinal, instance) in instances.iter().enumerate() {
            let source_ordinal = u32::try_from(source_ordinal).map_err(|_| {
                format!("country_dynamic_modifiers owner {owner} carries too many instances")
            })?;
            let instance_key = key_from_instance_row(instance, symbols)?;
            let record = record_from_instance_row(
                instances_collection,
                instance,
                source_ordinal,
                frame_tick,
                roster,
            )?;
            // Unreachable while `decode_payload` enforces identity uniqueness,
            // but the insert is where a silent overwrite would happen, so the
            // check belongs here rather than in a comment.
            if state.instances.insert(instance_key, record).is_some() {
                return Err(format!(
                    "country_dynamic_modifiers DMI2 repeats identity ({}, {})",
                    instance.owner_tag_db_id, instance.modifier_id
                ));
            }
            for value_ordinal in 0..instance.values_raw.len() {
                let value_ordinal = u32::try_from(value_ordinal)
                    .map_err(|_| "country_dynamic_modifiers value ordinal overflow".to_string())?;
                let value_key = key_from_value_row(
                    instance.owner_tag_db_id,
                    &instance.modifier_id,
                    value_ordinal,
                    symbols,
                )?;
                let value_record =
                    record_from_value_row(values_collection, instance, value_ordinal, frame_tick)?;
                if state.values.insert(value_key, value_record).is_some() {
                    return Err(format!(
                        "country_dynamic_modifiers DMI2 repeats value slot {value_ordinal} of \
                         ({}, {})",
                        instance.owner_tag_db_id, instance.modifier_id
                    ));
                }
            }
        }
    }
    Ok(state)
}

/// `compact_contract/src/calendar.rs:22`: the engine's stored hour counter minus
/// the epoch, or `None` outside the plausible calendar window (fail closed).
/// Transcribed for the same reason the payload grammar is — d67 is not admitted
/// to the production-codec dependency surface — and asserted equal to it over
/// the whole `i32` boundary neighbourhood in `bench_real_d67`.
pub fn hour_index_from_raw(stored: i32) -> Option<i64> {
    // `calendar.rs:14`-`:18`.
    if !(RAW_DATE_MIN..=RAW_DATE_MAX).contains(&stored) {
        return None;
    }
    Some((stored - DATE_EPOCH_CONST) as i64)
}

/// `compact_contract/src/calendar.rs:9`.
pub const DATE_EPOCH_CONST: i32 = 43_799_999;
/// `calendar.rs:15`-`:16`: the sanity window is years 1850..2200 on the engine's
/// 365-day, no-leap calendar.
pub const RAW_DATE_MIN: i32 = (1850 * 365 * 24) + DATE_EPOCH_CONST;
pub const RAW_DATE_MAX: i32 = (2200 * 365 * 24) + DATE_EPOCH_CONST;

// --- Little-endian readers. Callers reach these only after a length check. ----

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body::{decode_baseline, encode_baseline};
    use crate::schema::Quant;
    use crate::TickGrid;

    fn grid() -> TickGrid {
        TickGrid::new(3).expect("3 is a legal grid")
    }

    fn instances_collection() -> Collection {
        schema().collections[INSTANCES_INDEX].clone()
    }

    fn values_collection() -> Collection {
        schema().collections[VALUES_INDEX].clone()
    }

    fn index_of(c: &Collection, id: u32) -> usize {
        c.value_fields()
            .position(|f| f.field_id == id)
            .expect("field is in the schema")
    }

    fn symbols() -> Symbols {
        Symbols::from_vocabulary(["alpha", "beta", "gamma"])
    }

    /// The capture's own first hour, and the index it folds to.
    const TEST_HOUR_RAW: i32 = 60_832_272;
    const TEST_HOUR: i64 = 17_032_273;

    fn roster() -> CountryRoster {
        CountryRoster::from_ids([1u32, 2, 3, 7, 33])
    }

    fn instance(owner: u32, id: &str, days: Option<u32>, values: Vec<i64>) -> Instance {
        Instance {
            owner_tag_db_id: owner,
            modifier_id: id.to_string(),
            enabled: true,
            values_raw: values,
            remaining_days: days,
        }
    }

    /// Build a raw `DMI2` payload the way the shipping encoder packs one
    /// (`compact_contract/src/dynamic_modifiers.rs:313`-`:437`).
    fn raw_payload(
        status_flags: u32,
        instances: &[Instance],
        zero_removals: &[ZeroRemoval],
    ) -> Vec<u8> {
        raw_payload_with(status_flags, instances, zero_removals, 77, SCHEMA_VERSION)
    }

    fn raw_payload_with(
        status_flags: u32,
        instances: &[Instance],
        zero_removals: &[ZeroRemoval],
        elapsed_wall_ms: u64,
        version: u16,
    ) -> Vec<u8> {
        struct Row<'a> {
            owner: u32,
            modifier_id: &'a str,
            enabled: bool,
            values_raw: &'a [i64],
            remaining_days: Option<u32>,
            event_hour: u32,
        }
        let rows: Vec<Row> = instances
            .iter()
            .map(|i| Row {
                owner: i.owner_tag_db_id,
                modifier_id: &i.modifier_id,
                enabled: i.enabled,
                values_raw: &i.values_raw,
                remaining_days: i.remaining_days,
                event_hour: 0,
            })
            .chain(zero_removals.iter().map(|e| Row {
                owner: e.owner_tag_db_id,
                modifier_id: &e.modifier_id,
                enabled: e.enabled,
                values_raw: &e.values_raw,
                remaining_days: Some(0),
                event_hour: e.observed_game_hour_raw as u32,
            }))
            .collect();
        let id_bytes: usize = rows.iter().map(|r| r.modifier_id.len()).sum();
        let value_count: usize = rows.iter().map(|r| r.values_raw.len()).sum();

        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
        out.extend_from_slice(&(instances.len() as u32).to_le_bytes());
        out.extend_from_slice(&(id_bytes as u32).to_le_bytes());
        out.extend_from_slice(&(value_count as u32).to_le_bytes());
        out.extend_from_slice(&status_flags.to_le_bytes());
        out.extend_from_slice(&(zero_removals.len() as u32).to_le_bytes());
        out.extend_from_slice(&AUTHORITY_KIND_EXACT_A729.to_le_bytes());
        out.extend_from_slice(&A729_EXECUTABLE_SHA256);
        out.extend_from_slice(&STATIC_AUTHORITY_RECEIPT_SHA256);
        let mut id_offset = 0u32;
        let mut value_offset = 0u32;
        for row in &rows {
            let mut flags = if row.enabled { FLAG_ENABLED } else { 0 };
            if row.remaining_days.is_some() {
                flags |= FLAG_REMAINING_DAYS;
            }
            out.extend_from_slice(&row.owner.to_le_bytes());
            out.extend_from_slice(&id_offset.to_le_bytes());
            out.extend_from_slice(&(row.modifier_id.len() as u16).to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&row.remaining_days.unwrap_or_default().to_le_bytes());
            out.extend_from_slice(&value_offset.to_le_bytes());
            out.extend_from_slice(&(row.values_raw.len() as u32).to_le_bytes());
            out.extend_from_slice(&row.event_hour.to_le_bytes());
            id_offset += row.modifier_id.len() as u32;
            value_offset += row.values_raw.len() as u32;
        }
        for row in &rows {
            out.extend_from_slice(row.modifier_id.as_bytes());
        }
        for row in &rows {
            for value in row.values_raw {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn the_schema_is_valid() {
        schema()
            .validate(grid().log2())
            .expect("d67 schema is well-formed");
    }

    #[test]
    fn the_instances_collection_is_keyed_by_owner_and_modifier() {
        let c = instances_collection();
        assert_eq!(c.shape, Shape::KeyedSet, "baseline replaces, deltas upsert");
        let keys = c.key_fields();
        assert_eq!(keys.len(), 2, "the fold's identity is (owner, modifier_id)");
        assert_eq!(keys[0].field_id, field::OWNER_TAG_DB_ID);
        assert_eq!(keys[0].kind, Kind::ContentId, "a country tag is content (content-identity contract)");
        assert_eq!(keys[1].field_id, field::MODIFIER_ID);
        assert_eq!(
            keys[1].kind,
            Kind::ContentId,
            "a modifier definition id is content-defined text carried as a symbol (content-identity contract)"
        );
        assert!(keys.iter().all(|f| f.kind.legal_as_key()));
        assert_eq!(c.fields.len(), 5, "2 key + source_ordinal + enabled + remaining_days");
    }

    #[test]
    fn the_values_collection_carries_the_ordinal_in_its_key() {
        let c = values_collection();
        assert_eq!(c.shape, Shape::KeyedSet);
        let keys = c.key_fields();
        assert_eq!(keys.len(), 3, "the child's identity includes its parent's");
        assert_eq!(keys[0].field_id, field::OWNER_TAG_DB_ID);
        assert_eq!(keys[1].field_id, field::MODIFIER_ID);
        assert_eq!(keys[2].field_id, field::VALUE_ORDINAL);
        assert_eq!(
            keys[2].kind,
            Kind::Unsigned,
            "a value slot index is an array position, not content"
        );
        assert!(keys.iter().all(|f| f.kind.legal_as_key()));
        assert_eq!(c.fields.len(), 4, "3 key + value_raw");
        assert_eq!(
            c.value_fields().next().expect("one value field").kind,
            Kind::Signed,
            "the fold emits the fixed-point word raw (`world_fold.rs:10008`)"
        );
    }

    #[test]
    fn the_schema_declares_exactly_the_states_the_mapper_emits() {
        // `remaining_days` is the ONLY field with an empty state and it is
        // Absent. Nothing here is nullable or unknown-capable: every read
        // failure in d67 fails the whole frame
        // (`attach_log.rs:61344`-`:61352`), so there is no partial row and no
        // per-field "applicable, not captured".
        for c in schema().collections {
            for f in c.value_fields() {
                assert!(
                    !f.nullable,
                    "collection {} field {} must not be nullable",
                    c.collection_id, f.field_id
                );
                assert!(
                    !f.unknown_capable,
                    "collection {} field {} must not be unknown-capable",
                    c.collection_id, f.field_id
                );
                assert_eq!(
                    f.optional,
                    f.field_id == field::REMAINING_DAYS,
                    "collection {} field {} optionality",
                    c.collection_id,
                    f.field_id
                );
            }
        }
        assert_eq!(instances_collection().optional_count(), 1);
        assert_eq!(values_collection().optional_count(), 0);
    }

    #[test]
    fn no_field_is_quantized_or_derived() {
        // Step 4's "quantization saves bytes" test does not apply: d67 has **no
        // continuous field at all**. The only fixed-point quantity, `values_raw`,
        // is emitted raw by the fold (`world_fold.rs::fold_country_dynamic_modifier_instances`, which
        // carries `value_scale` beside it as a constant), so it travels as a
        // `Signed` integer — lossless over the whole i64 range, with no
        // quantization decision to make.
        for c in schema().collections {
            for f in &c.fields {
                assert!(
                    !matches!(f.kind, Kind::Continuous),
                    "field {} is continuous",
                    f.field_id
                );
                assert_eq!(f.quant, Quant::None, "field {} carries a quantizer", f.field_id);
                assert!(f.derived.is_none(), "field {} is derived", f.field_id);
            }
        }
    }

    #[test]
    fn every_content_id_field_is_declared_for_the_retrofit_sweep() {
        // the symbol/manifest-id assignment rule will change every mapper that
        // emits a ContentId, and a ContentId **key** is the heavier class: key
        // components sort and delta-encode by raw value, so a remap reorders keys
        // and changes the byte layout of every baseline. This asserts exactly
        // which fields the sweep must find.
        let mut content_keys = Vec::new();
        let mut content_values = Vec::new();
        for c in schema().collections {
            for f in &c.fields {
                if matches!(f.kind, Kind::ContentId) {
                    if f.is_key() {
                        content_keys.push((c.collection_id, f.field_id));
                    } else {
                        content_values.push((c.collection_id, f.field_id));
                    }
                }
            }
        }
        assert_eq!(
            content_keys,
            vec![
                (INSTANCES, field::OWNER_TAG_DB_ID),
                (INSTANCES, field::MODIFIER_ID),
                (VALUES, field::OWNER_TAG_DB_ID),
                (VALUES, field::MODIFIER_ID),
            ]
        );
        assert!(
            content_values.is_empty(),
            "every d67 ContentId is a key component, the heavier retrofit class"
        );
    }

    // --- Mapper state tests, through the public boundary on raw bytes --------

    #[test]
    fn a_full_frame_maps_every_field_at_its_offset() {
        // The offset test: known words at known offsets, one instance per shape,
        // read back through the public boundary. A transposed row word or a
        // transposed header word fails here.
        let payload = raw_payload(
            0,
            &[
                instance(7, "alpha", Some(716), vec![15_000, -10_000, 0]),
                instance(7, "beta", None, vec![i64::MAX, i64::MIN]),
                instance(33, "gamma", Some(0), vec![]),
            ],
            &[],
        );
        let frame = decode_payload(&payload, true).expect("decodes");
        assert_eq!(
            frame.header,
            PayloadHeader {
                elapsed_wall_ms: 77,
                instance_count: 3,
                id_blob_bytes: 5 + 4 + 5,
                total_value_count: 5,
                status_flags: 0,
                zero_removal_count: 0,
            },
            "every header word, at its own offset"
        );
        assert_eq!(frame.instances.len(), 3);
        assert_eq!(frame.instances[0].owner_tag_db_id, 7);
        assert_eq!(frame.instances[0].modifier_id, "alpha");
        assert!(frame.instances[0].enabled);
        assert_eq!(frame.instances[0].remaining_days, Some(716));
        assert_eq!(frame.instances[0].values_raw, vec![15_000, -10_000, 0]);
        assert_eq!(frame.instances[1].remaining_days, None, "the permanent sentinel");
        assert_eq!(frame.instances[1].values_raw, vec![i64::MAX, i64::MIN]);
        assert_eq!(frame.instances[2].remaining_days, Some(0), "a measured zero");
        assert!(frame.instances[2].values_raw.is_empty());

        let ic = instances_collection();
        let r = record_from_instance_row(&ic, &frame.instances[0], 4, 0, &roster()).expect("maps");
        assert_eq!(
            r.values[index_of(&ic, field::SOURCE_ORDINAL)],
            Value::Unsigned(4)
        );
        assert_eq!(r.values[index_of(&ic, field::ENABLED)], Value::Bool(true));
        assert_eq!(
            r.values[index_of(&ic, field::REMAINING_DAYS)],
            Value::Unsigned(716)
        );

        let vc = values_collection();
        for (ordinal, expected) in [(0u32, 15_000i64), (1, -10_000), (2, 0)] {
            let r = record_from_value_row(&vc, &frame.instances[0], ordinal, 0).expect("maps");
            assert_eq!(
                r.values[index_of(&vc, field::VALUE_RAW)],
                Value::Signed(expected),
                "slot {ordinal} carries the raw word, unscaled"
            );
        }
        // The whole-scale claim: the raw word is never divided by VALUE_SCALE.
        let scaled = record_from_value_row(&vc, &frame.instances[0], 0, 0).expect("maps");
        assert_ne!(
            scaled.values[index_of(&vc, field::VALUE_RAW)],
            Value::Signed(15_000 / VALUE_SCALE)
        );
        // And the i64 extremes survive, which no `Continuous` mapping would.
        for (ordinal, expected) in [(0u32, i64::MAX), (1, i64::MIN)] {
            let r = record_from_value_row(&vc, &frame.instances[1], ordinal, 0).expect("maps");
            assert_eq!(
                r.values[index_of(&vc, field::VALUE_RAW)],
                Value::Signed(expected)
            );
        }
    }

    #[test]
    fn a_permanent_modifier_is_absent_and_a_measured_zero_is_a_value() {
        let ic = instances_collection();
        let permanent = record_from_instance_row(
            &ic,
            &instance(7, "alpha", None, vec![1]),
            0,
            0,
            &roster(),
        )
        .expect("maps");
        assert_eq!(
            permanent.values[index_of(&ic, field::REMAINING_DAYS)],
            Value::Absent,
            "the -1 permanent sentinel means the countdown does not apply to this row"
        );
        let zero =
            record_from_instance_row(&ic, &instance(7, "alpha", Some(0), vec![1]), 0, 0, &roster())
                .expect("maps");
        assert_eq!(
            zero.values[index_of(&ic, field::REMAINING_DAYS)],
            Value::Unsigned(0),
            "a measured zero is a value, never collapsed into an empty state"
        );
        // A zero source_ordinal and a zero value are values too.
        assert_eq!(
            zero.values[index_of(&ic, field::SOURCE_ORDINAL)],
            Value::Unsigned(0)
        );
        let vc = values_collection();
        let r = record_from_value_row(&vc, &instance(7, "alpha", None, vec![0]), 0, 0)
            .expect("maps");
        assert_eq!(r.values[index_of(&vc, field::VALUE_RAW)], Value::Signed(0));
        // `enabled == false` is a value, not an empty state.
        let mut disabled = instance(7, "alpha", None, vec![1]);
        disabled.enabled = false;
        let r = record_from_instance_row(&ic, &disabled, 0, 0, &roster()).expect("maps");
        assert_eq!(r.values[index_of(&ic, field::ENABLED)], Value::Bool(false));
    }

    #[test]
    fn every_record_roundtrips_byte_for_byte() {
        for (c, record) in [
            (
                instances_collection(),
                record_from_instance_row(
                    &instances_collection(),
                    &instance(7, "alpha", Some(716), vec![1]),
                    3,
                    0,
                    &roster(),
                )
                .expect("maps"),
            ),
            // The Absent case gets its own round trip: it is the one state the
            // presence bitmap has to carry.
            (
                instances_collection(),
                record_from_instance_row(
                    &instances_collection(),
                    &instance(7, "alpha", None, vec![1]),
                    0,
                    0,
                    &roster(),
                )
                .expect("maps"),
            ),
            (
                values_collection(),
                record_from_value_row(
                    &values_collection(),
                    &instance(7, "alpha", None, vec![i64::MIN]),
                    0,
                    0,
                )
                .expect("maps"),
            ),
        ] {
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
    fn a_whole_state_roundtrips_through_a_baseline() {
        let s = schema();
        let payload = raw_payload(
            0,
            &[
                instance(7, "beta", Some(1), vec![10, -20]),
                instance(7, "alpha", None, vec![]),
                instance(33, "gamma", Some(0), vec![7]),
            ],
            &[],
        );
        let frame = decode_payload(&payload, true).expect("decodes");
        let state = fold_payload(&s, &frame, Some(TEST_HOUR), Some(&roster()), &symbols(), 0)
            .expect("folds");
        assert_eq!(state.instances.len(), 3);
        assert_eq!(state.values.len(), 3, "2 + 0 + 1 value slots");

        let mut bytes = Vec::new();
        encode_baseline(&s, &state.as_slice(), &mut bytes).expect("encodes");
        let (decoded, consumed) = decode_baseline(&s, &bytes, 0).expect("decodes");
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded[INSTANCES_INDEX], state.instances);
        assert_eq!(decoded[VALUES_INDEX], state.values);
    }

    #[test]
    fn source_ordinal_is_the_payload_order_within_the_owner() {
        // `world_fold.rs::fold_country_dynamic_modifier_instances`. The payload's within-owner order here
        // is NOT the key order, which is the whole reason the field exists.
        let s = schema();
        let payload = raw_payload(
            0,
            &[
                instance(7, "gamma", None, vec![]),
                instance(33, "beta", None, vec![]),
                instance(7, "alpha", None, vec![]),
                instance(7, "beta", None, vec![]),
            ],
            &[],
        );
        let frame = decode_payload(&payload, true).expect("decodes");
        let state = fold_payload(&s, &frame, Some(TEST_HOUR), Some(&roster()), &symbols(), 0)
            .expect("folds");
        let ic = instances_collection();
        let ordinal_of = |owner: u32, id: &str| -> u64 {
            let k = key_from_value_row(owner, id, 0, &symbols()).expect("keys");
            let instance_key = key(&[k.0[0] as u64, k.0[1] as u64]);
            match state.instances[&instance_key].values[index_of(&ic, field::SOURCE_ORDINAL)] {
                Value::Unsigned(v) => v,
                other => panic!("source_ordinal is {other:?}"),
            }
        };
        // Owner 7's payload order is gamma, alpha, beta — not the key order
        // alpha, beta, gamma.
        assert_eq!(ordinal_of(7, "gamma"), 0);
        assert_eq!(ordinal_of(7, "alpha"), 1);
        assert_eq!(ordinal_of(7, "beta"), 2);
        // The ordinal restarts per owner (`:10000` enumerates within the group).
        assert_eq!(ordinal_of(33, "beta"), 0);
    }

    #[test]
    fn a_variable_arity_vector_becomes_exactly_its_slots() {
        let s = schema();
        let payload = raw_payload(
            0,
            &[
                instance(7, "alpha", None, vec![]),
                instance(7, "beta", None, vec![1, 2, 3, 4, 5]),
            ],
            &[],
        );
        let frame = decode_payload(&payload, true).expect("decodes");
        let state =
            fold_payload(&s, &frame, Some(TEST_HOUR), Some(&roster()), &symbols(), 0).expect("folds");
        assert_eq!(state.values.len(), 5, "an empty vector contributes no rows");
        let vc = values_collection();
        for ordinal in 0..5u32 {
            let k = key_from_value_row(7, "beta", ordinal, &symbols()).expect("keys");
            assert_eq!(
                state.values[&k].values[index_of(&vc, field::VALUE_RAW)],
                Value::Signed(ordinal as i64 + 1),
                "slot {ordinal}"
            );
        }
        // An arity change is an ordinary keyed-set difference, never a refusal:
        // the same identity with a shorter vector folds fine.
        let shorter = raw_payload(0, &[instance(7, "beta", None, vec![9])], &[]);
        let frame = decode_payload(&shorter, true).expect("decodes");
        let state =
            fold_payload(&s, &frame, Some(TEST_HOUR), Some(&roster()), &symbols(), 0).expect("folds");
        assert_eq!(state.values.len(), 1);
    }

    // --- Refusals ------------------------------------------------------------

    #[test]
    fn a_legacy_v1_record_is_refused_with_its_reason() {
        // The record-header flag is the discriminator (`world_fold.rs::fold_country_dynamic_modifiers`).
        // A clear flag is the legacy grammar, which this port does not carry, so
        // it is an Err — a stream rejection, never a silently skipped hour.
        let payload = raw_payload(0, &[instance(7, "alpha", None, vec![1])], &[]);
        let err = decode_payload(&payload, false).expect_err("v1 is refused");
        assert!(err.contains("legacy v1"), "{err}");
        // And a v1-shaped payload (no DMI2 magic) is refused under the flag too.
        let v1_like = vec![0u8; 200];
        assert!(decode_payload(&v1_like, false).is_err());
    }

    #[test]
    fn a_frame_carrying_a_zero_removal_event_is_stop_reported() {
        let s = schema();
        let event = ZeroRemoval {
            owner_tag_db_id: 7,
            modifier_id: "beta".into(),
            enabled: true,
            values_raw: vec![5],
            // `hour_index_from_raw(TEST_HOUR_RAW) == Some(TEST_HOUR)`, so this
            // event passes the fold's own admission and is refused only by the
            // event-continuity contract stop.
            observed_game_hour_raw: TEST_HOUR_RAW,
        };
        let payload = raw_payload(0, &[instance(7, "alpha", None, vec![1])], &[event.clone()]);
        let frame = decode_payload(&payload, true).expect("decodes");
        assert_eq!(frame.zero_removals, vec![event.clone()], "decoded, not dropped");
        let err = fold_payload(&s, &frame, Some(TEST_HOUR), Some(&roster()), &symbols(), 0)
            .expect_err("stop-reported");
        assert!(err.contains("zero-removal"), "{err}");

        // The fold's own admission is mirrored first, so a malformed event is
        // diagnosed as the fold diagnoses it (`world_fold.rs::fold_country_dynamic_modifier_instances`).
        for bad in [
            // Owner not in the roster.
            ZeroRemoval { owner_tag_db_id: 9, ..event.clone() },
            // Observed hour is not the frame hour.
            ZeroRemoval { observed_game_hour_raw: TEST_HOUR_RAW + 1, ..event.clone() },
        ] {
            let payload = raw_payload(0, &[instance(7, "alpha", None, vec![1])], &[bad]);
            let frame = decode_payload(&payload, true).expect("decodes");
            let err = fold_payload(&s, &frame, Some(TEST_HOUR), Some(&roster()), &symbols(), 0)
                .expect_err("refused");
            assert!(err.contains("admission"), "{err}");
        }
        // An event whose identity is still in the current set is refused too.
        let clash = ZeroRemoval { modifier_id: "alpha".into(), ..event };
        let payload = raw_payload(0, &[instance(7, "alpha", None, vec![1])], &[clash]);
        let frame = decode_payload(&payload, true).expect("decodes");
        let err = fold_payload(&s, &frame, Some(TEST_HOUR), Some(&roster()), &symbols(), 0)
            .expect_err("refused");
        assert!(err.contains("admission"), "{err}");
    }

    #[test]
    fn the_d18_roster_dependency_is_enforced_both_ways() {
        let s = schema();
        let payload = raw_payload(0, &[instance(7, "alpha", None, vec![1])], &[]);
        let frame = decode_payload(&payload, true).expect("decodes");
        // No roster at all this hour (`world_fold.rs::fold_country_dynamic_modifier_instances`).
        let err = fold_payload(&s, &frame, Some(TEST_HOUR), None, &symbols(), 0).expect_err("refused");
        assert!(err.contains("country_identity roster"), "{err}");
        // A roster that does not carry the owner (`:9878`).
        let empty = CountryRoster::from_ids([]);
        let err = fold_payload(&s, &frame, Some(TEST_HOUR), Some(&empty), &symbols(), 0)
            .expect_err("refused");
        assert!(err.contains("no current country_identity mapping"), "{err}");
        assert!(record_from_instance_row(
            &instances_collection(),
            &frame.instances[0],
            0,
            0,
            &empty
        )
        .is_err());
        // And the fold's own clock (`:9867`), which is not derived-field contract's frame_tick.
        let err = fold_payload(&s, &frame, None, Some(&roster()), &symbols(), 0)
            .expect_err("refused");
        assert!(err.contains("invalid frame hour"), "{err}");
    }

    #[test]
    fn an_unresolvable_symbol_is_an_error_not_an_empty_state() {
        let s = schema();
        let payload = raw_payload(0, &[instance(7, "not_in_the_manifest", None, vec![1])], &[]);
        let frame = decode_payload(&payload, true).expect("decodes");
        assert!(fold_payload(&s, &frame, Some(TEST_HOUR), Some(&roster()), &symbols(), 0).is_err());
        assert!(key_from_instance_row(&frame.instances[0], &Symbols::empty()).is_err());
        assert!(key_from_value_row(7, "alpha", 0, &Symbols::empty()).is_err());
    }

    #[test]
    fn the_mappers_refuse_a_collection_they_cannot_fill() {
        let instance = instance(7, "alpha", None, vec![1]);
        // Another domain's collection, and each other's.
        assert!(record_from_instance_row(&values_collection(), &instance, 0, 0, &roster()).is_err());
        assert!(record_from_value_row(&instances_collection(), &instance, 0, 0).is_err());
        // A field id this mapper does not know.
        let mut c = instances_collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_instance_row(&c, &instance, 0, 0, &roster()).is_err());
        let mut c = values_collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_value_row(&c, &instance, 0, 0).is_err());
        // A value ordinal outside the vector.
        assert!(record_from_value_row(&values_collection(), &instance, 1, 0).is_err());
        assert!(record_from_value_row(&values_collection(), &instance, u32::MAX, 0).is_err());
    }

    #[test]
    fn the_mappers_refuse_an_instance_the_shipping_decoder_would_reject() {
        let ic = instances_collection();
        let r = roster();
        let cases = [
            instance(0, "alpha", None, vec![1]),
            instance(MAX_OWNER_TAG_DB_ID + 1, "alpha", None, vec![1]),
            instance(7, "", None, vec![1]),
            instance(7, &"x".repeat(MAX_MODIFIER_ID_LEN + 1), None, vec![1]),
            instance(7, "with\0nul", None, vec![1]),
            instance(7, "alpha", None, vec![0; MAX_VALUES_PER_INSTANCE + 1]),
            instance(7, "alpha", Some(i32::MAX as u32 + 1), vec![1]),
        ];
        for case in &cases {
            assert!(
                record_from_instance_row(&ic, case, 0, 0, &r).is_err(),
                "instance {case:?} must be refused"
            );
            assert!(record_from_value_row(&values_collection(), case, 0, 0).is_err());
        }
        // The bounds themselves are accepted, so nothing is off by one.
        for ok in [
            instance(MAX_OWNER_TAG_DB_ID, "alpha", None, vec![1]),
            instance(7, &"x".repeat(MAX_MODIFIER_ID_LEN), None, vec![1]),
            instance(7, "alpha", Some(i32::MAX as u32), vec![1]),
            instance(7, "alpha", None, vec![0; MAX_VALUES_PER_INSTANCE]),
        ] {
            let mut extended = symbols().vocabulary.clone();
            extended.push(ok.modifier_id.clone());
            let symbols = Symbols::from_vocabulary(extended);
            assert!(
                record_from_instance_row(&ic, &ok, 0, 0, &CountryRoster::from_ids([
                    ok.owner_tag_db_id
                ]))
                .is_ok(),
                "instance {ok:?} must be accepted"
            );
            assert!(key_from_instance_row(&ok, &symbols).is_ok());
        }
    }

    #[test]
    fn decode_payload_enforces_every_structural_rule() {
        let good = raw_payload(
            0,
            &[
                instance(7, "alpha", Some(1), vec![10, 20]),
                instance(33, "beta", None, vec![30]),
            ],
            &[],
        );
        assert!(decode_payload(&good, true).is_ok());

        // Every truncation, including the empty slice and every prefix of the
        // header.
        for len in 0..good.len() {
            assert!(
                decode_payload(&good[..len], true).is_err(),
                "a {len}-byte prefix must be refused"
            );
        }
        // A trailing byte is a length mismatch, not a tolerated suffix.
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(decode_payload(&trailing, true).is_err());

        // Each header word patched to something the grammar rejects.
        let patches: [(usize, Vec<u8>, &str); 8] = [
            (0, b"DMI3".to_vec(), "magic"),
            (4, 2u16.to_le_bytes().to_vec(), "schema version"),
            (6, 100u16.to_le_bytes().to_vec(), "header length"),
            (16, 3u32.to_le_bytes().to_vec(), "instance_count"),
            (20, 1u32.to_le_bytes().to_vec(), "id_blob_bytes"),
            (24, 4u32.to_le_bytes().to_vec(), "total_value_count"),
            (32, 1u32.to_le_bytes().to_vec(), "zero_removal_count"),
            (36, 2u32.to_le_bytes().to_vec(), "authority_kind"),
        ];
        for (offset, patch, what) in patches {
            let mut bad = good.clone();
            bad[offset..offset + patch.len()].copy_from_slice(&patch);
            assert!(
                decode_payload(&bad, true).is_err(),
                "a patched {what} word must be refused"
            );
        }
        // Both authority hashes, one flipped bit each.
        for offset in [40usize, 72] {
            let mut bad = good.clone();
            bad[offset] ^= 0xff;
            assert!(decode_payload(&bad, true).is_err(), "authority at +{offset}");
        }
        // Row words: a non-canonical id window, a non-canonical value window, an
        // unknown flag bit, a zero owner, an owner past the bound, an event hour
        // on an instance row, and a gated-off counter carrying a value.
        let row0 = HEADER_LEN;
        for (offset, patch, what) in [
            (row0, 0u32.to_le_bytes().to_vec(), "zero owner"),
            (
                row0,
                (MAX_OWNER_TAG_DB_ID + 1).to_le_bytes().to_vec(),
                "owner past the bound",
            ),
            (row0 + 4, 1u32.to_le_bytes().to_vec(), "id window"),
            (row0 + 8, 0u16.to_le_bytes().to_vec(), "empty id"),
            (row0 + 10, 4u16.to_le_bytes().to_vec(), "unknown flag bit"),
            (row0 + 16, 1u32.to_le_bytes().to_vec(), "value window"),
            (row0 + 24, 7u32.to_le_bytes().to_vec(), "event hour on an instance row"),
        ] {
            let mut bad = good.clone();
            bad[offset..offset + patch.len()].copy_from_slice(&patch);
            assert!(
                decode_payload(&bad, true).is_err(),
                "a patched row word ({what}) must be refused"
            );
        }
        // The second row is the permanent one: giving it a counter word without
        // the flag is a malformed row (`dynamic_modifiers.rs:262`-`:265`).
        let row1 = HEADER_LEN + ROW_LEN;
        let mut bad = good.clone();
        bad[row1 + 12..row1 + 16].copy_from_slice(&5u32.to_le_bytes());
        assert!(decode_payload(&bad, true).is_err());
        // A remaining_days word above i32::MAX under its flag.
        let mut bad = good.clone();
        bad[row0 + 12..row0 + 16].copy_from_slice(&(i32::MAX as u32 + 1).to_le_bytes());
        assert!(decode_payload(&bad, true).is_err());
        // A non-utf8 id blob.
        let mut bad = good.clone();
        let blob = HEADER_LEN + 2 * ROW_LEN;
        bad[blob] = 0xff;
        assert!(decode_payload(&bad, true).is_err());
        // An interior NUL in an id.
        let mut bad = good.clone();
        bad[blob] = 0;
        assert!(decode_payload(&bad, true).is_err());
        // A duplicated identity (`:274`-`:281`).
        let duplicate = raw_payload(
            0,
            &[
                instance(7, "alpha", None, vec![]),
                instance(7, "alpha", None, vec![]),
            ],
            &[],
        );
        assert!(decode_payload(&duplicate, true).is_err());
        // A zero-removal row that is not a measured zero (`:283`-`:285`): patch
        // its counter flag off.
        let event = ZeroRemoval {
            owner_tag_db_id: 7,
            modifier_id: "beta".into(),
            enabled: true,
            values_raw: vec![],
            observed_game_hour_raw: TEST_HOUR_RAW,
        };
        let with_event = raw_payload(0, &[instance(7, "alpha", None, vec![])], &[event]);
        assert!(decode_payload(&with_event, true).is_ok());
        let mut bad = with_event.clone();
        bad[row1 + 10..row1 + 12].copy_from_slice(&FLAG_ENABLED.to_le_bytes());
        assert!(decode_payload(&bad, true).is_err());
    }

    #[test]
    fn decode_payload_never_panics_on_arbitrary_bytes() {
        // A payload arrives from a file on disk, so every header word is
        // attacker-shaped. Nothing below may panic, wrap, or allocate on a
        // declared count.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let good = raw_payload(0, &[instance(7, "alpha", Some(3), vec![1, 2])], &[]);
        for case in 0..8_192 {
            let len = (next() as usize) % 300;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                *byte = next() as u8;
            }
            for schema_v2 in [true, false] {
                let _ = decode_payload(&bytes, schema_v2);
            }
            // And the far more interesting shape: a well-formed payload with one
            // word corrupted, which is where an unchecked multiply would land.
            let mut mutated = good.clone();
            let at = (next() as usize) % mutated.len();
            mutated[at] = next() as u8;
            let _ = decode_payload(&mutated, true);
            let _ = case;
        }
        // The counts that would overflow a 32-bit multiply, spelled out.
        for (offset, word) in [
            (16usize, u32::MAX),
            (20, u32::MAX),
            (24, u32::MAX),
            (32, u32::MAX),
        ] {
            let mut bad = good.clone();
            bad[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
            assert!(decode_payload(&bad, true).is_err(), "word at +{offset}");
        }
    }

    #[test]
    fn the_symbol_table_is_a_sorted_stand_in_and_fails_closed() {
        let s = Symbols::from_vocabulary(["gamma", "alpha", "beta", "alpha"]);
        assert_eq!(s.len(), 3, "deduplicated");
        assert_eq!(s.id("alpha"), Some(0));
        assert_eq!(s.id("beta"), Some(1));
        assert_eq!(s.id("gamma"), Some(2));
        assert_eq!(s.id("delta"), None);
        assert_eq!(s.symbol(2), Some("gamma"));
        assert_eq!(s.symbol(3), None);
        // Order-independent: two callers over the same content set mint the same
        // ids.
        assert_eq!(s, Symbols::from_vocabulary(["beta", "gamma", "alpha"]));
        assert!(Symbols::empty().is_empty());
    }

    #[test]
    fn the_transcribed_calendar_matches_its_source() {
        // The exact identity `compact_contract::calendar::hour_index_from_raw`
        // has (`calendar.rs:14`-`:27`), including its sanity window;
        // `bench_real_d67` asserts the two agree across the whole boundary
        // neighbourhood and on the capture's own hours.
        assert_eq!(
            hour_index_from_raw(RAW_DATE_MIN),
            Some((RAW_DATE_MIN - DATE_EPOCH_CONST) as i64)
        );
        assert_eq!(
            hour_index_from_raw(RAW_DATE_MAX),
            Some((RAW_DATE_MAX - DATE_EPOCH_CONST) as i64)
        );
        assert_eq!(hour_index_from_raw(RAW_DATE_MIN - 1), None);
        assert_eq!(hour_index_from_raw(RAW_DATE_MAX + 1), None);
        assert_eq!(hour_index_from_raw(DATE_EPOCH_CONST), None, "before the window");
        assert_eq!(hour_index_from_raw(0), None);
        assert_eq!(hour_index_from_raw(i32::MIN), None);
        assert_eq!(hour_index_from_raw(i32::MAX), None);
        // The capture's own first hour.
        assert_eq!(hour_index_from_raw(60_832_272), Some(17_032_273));
    }
}
