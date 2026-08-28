//! d38 `air_superiority_inputs` — the equipment actually flying each air mission,
//! aggregated into `(region, owner, mission, variant)` buckets.
//!
//! Derived from `world_fold.rs:23184`-`:23319`
//! (`fold_air_superiority_inputs`), its payload constants at `:991`-`:992`,
// @world-fold-authority domain=38 functions=fold_air_superiority_inputs lines=23184-23319 sha256=0d9ec25354f229338546a9c188fb9251b664880f6276727f2f4fa29344bdb840 closure=da71330aad55ec516cccf96aa9966cb30c682b1ef4038350302c26e76ad20af3
//! `:758` and `:156`, the accumulator's own JSON projection (`world_fold.rs::insert_air_superiority_inputs`), and
//! the collector read that builds the buckets and the delta rows
//! (`extractor/native/collector/src/attach_log.rs:55529`-`:55769`,
//! `:71517`-`:71562`). There is no `extractor/documents/domains/d38_*.md` dossier,
//! so the fold and the collector read are the whole basis; where they could be
//! read differently the fold wins.
//!
//! **What the domain is.** Each row answers "how much of variant V is flying
//! mission M over region R for country C, and across how many wings". The
//! collector joins three native samples — air wings (for the region and mission
//! of each active wing), wing loadouts (for the equipment amounts) and air
//! variant defs (for the variant's catalog words) — and sums the loadout entries
//! into one bucket per `(region, owner, mission_mask, variant_id)`
//! (`attach_log.rs:55568`-`:55655`).
//!
//! **Payload grammar** (`world_fold.rs::fold_air_superiority_inputs`, collector
//! `attach_log.rs:71530`-`:71542`): a 52-byte header followed by `row_count`
//! fixed 48-byte rows. The fold requires the length to be exactly
//! `52 + row_count * 48` (`:15447`-`:15454`), so no legacy short row exists and the
//! length selects nothing.
//!
//! Header:
//!
//! | offset | width | field | note |
//! |---|---|---|---|
//! | `+0`  | u64 | `elapsed_wall_ms`         | collector timing, not game state |
//! | `+8`  | u32 | `row_count`              | rows in THIS record, not table size |
//! | `+12` | u32 | `changed_rows`           | `<= row_count` (`:15397`) |
//! | `+16` | u32 | `status_flags`           | the two early-exit gates |
//! | `+20` | u32 | `observed_air_wing_rows` | |
//! | `+24` | u32 | `observed_loadout_rows`  | `<= observed_air_wing_rows` (`:15404`) |
//! | `+28` | u32 | `observed_equipment_rows`| |
//! | `+32` | u32 | `observed_variant_rows`  | **the fold does not read this** |
//! | `+36` | u32 | `invalid_air_wing_rows`  | **the fold does not read this** |
//! | `+40` | u32 | `invalid_variant_rows`   | `<= observed_equipment_rows` (`:15411`) |
//! | `+44` | u32 | `deleted_rows`           | `<= row_count` (`:15418`) |
//! | `+48` | u32 | `scale_raw`              | asserted `== 100_000` (`:15432`) |
//!
//! Neither `+32` nor `+36` is a *reserved* word — the collector writes real
//! counters there (`attach_log.rs:71538`-`:71539`) and the independent authority
//! decoder reads both (`recorder_host/src/domain_data_authority.rs:15265`-`:15266`).
//! They are carried by [`PayloadHeader`] and constrained by nothing, because the
//! fold constrains them by nothing. **d38 has no header reserved word at all**, so
//! unlike d31 there is no zero check on the header; the row has one, at `+36`.
//!
//! Row (`world_fold.rs::fold_air_superiority_inputs`):
//!
//! | offset | width | field | scale | gate |
//! |---|---|---|---|---|
//! | `+0`  | u32 | `region_id`                | raw | none (key component 0) |
//! | `+4`  | u32 | `owner_tag_db_id`          | raw | none (key component 1) |
//! | `+8`  | u32 | `mission_mask`             | raw | none (key component 2) |
//! | `+12` | u32 | `variant_id`               | raw | none (key component 3) |
//! | `+16` | u32 | `flags`                    | —   | only `1<<0` DELETED is legal (`world_fold.rs::fold_air_superiority_inputs`) |
//! | `+20` | u32 | `equipment_token_raw`      | raw | none |
//! | `+24` | u32 | `equipment_type_token_raw` | raw | none |
//! | `+28` | u32 | `interface_type_raw`       | raw | none |
//! | `+32` | u32 | `wing_count`               | raw | none |
//! | `+36` | u32 | `reserved`                 | —   | must be zero (`world_fold.rs::fold_air_superiority_inputs`) |
//! | `+40` | i64 | `amount_raw`               | `/100_000` (`world_fold.rs::insert_air_superiority_inputs`) | none |
//!
//! **One scale, and it is the header's own constant.** `amount_raw` is the only
//! scaled field: the fold's render divides it by `FIXED_SCALE`
//! (`world_fold.rs:9087`) and `ensure_fixed_scale` has already asserted the
//! payload's `scale_raw` word equals that constant (`:15432`). Everything else —
//! the four key words, the three catalog words and `wing_count` — is read raw
//! (`:15462`-`:15494`) and re-emitted raw by the projection (`:9078`-`:9086`).
//!
//! **`scale_raw` is not a field.** The fold asserts it equal to `FIXED_SCALE`
//! *before* folding anything, so it is constant over every payload the grammar
//! accepts; it is then copied into every stored row (`:15496`) and re-emitted per
//! row by the renderer (`:9089`). Per the recipe's rule for a value the fold
//! hard-enforces as a function of carried state, the invariant is enforced in
//! [`peek_header`] and the constant left off the wire — re-encoding it would be a
//! second copy of state the decoder must then keep consistent.
//! [`FIXED_SCALE_RAW`] is that constant and `scale_word_is_invariant_not_data` is
//! the test.
//!
//! ## The three catalog words are FIELDS, not witnesses
//!
//! `equipment_token_raw`, `equipment_type_token_raw` and `interface_type_raw` are
//! the same three words d35 `air_variant_stats` carries per variant — the
//! collector reads all three out of one `read_compact_air_variant_stats_row`
//! (`attach_log.rs:57502`-`:57507`), memoized per `variant_id` within a frame
//! (`:55601`-`:55620`) — and `variant_id` is a key component here. That makes
//! "drop them, they are a function of the key" a tempting call. It is the wrong
//! one, and the deciding authority is the fold:
//!
//! - **The fold enforces no such function.** It reads the three words at fixed
//!   offsets and stores them per row (`world_fold.rs::fold_air_superiority_inputs`) with no
//!   comparison against anything, no cross-row check, and no dependency on d35
//!   (the fold has no cross-domain read here at all). The recipe's
//!   drop-a-derived-value rule applies to a value *the fold hard-enforces* as a
//!   function of carried state — d62's witness word, constant over every payload
//!   its codec accepts. These are not that.
//! - **The function they would be derived from is another domain's capture data,
//!   not a manifest.** d35 is itself a sampled table with its own failure paths,
//!   its own hours and its own `variant_id` key; reconstructing d38's tokens from
//!   it would make this domain undecodable whenever d35's hour is unknown, and
//!   would be a cross-domain dependency the shipping fold does not have.
//! - **Even the collector only holds the identity within one frame.** The memo is
//!   rebuilt every frame, so a variant whose catalog words change between hours
//!   produces a changed d38 row, and the equality check at `:55639`-`:55645`
//!   (which raises `invalid_variant_rows`) is a *within-frame* consistency guard,
//!   not a cross-frame invariant.
//!
//! So all three ride as `Unsigned` value fields. d35 classes the identical words
//! the identical way and states the evidence: a token that can only be resolved by
//! scanning the producing process's heap
//! (`refresh_compact_air_variant_token_dictionary`, `attach_log.rs:53530`) is a
//! process-local handle, not a reference a content manifest resolves, and
//! `interface_type_raw` is an engine discriminator of d20's `div_type` family
//! (`d35.rs:145`-`:155`). `interface_type_raw` is deliberately **not** an `Enum`
//! for the reason d35 gives: the fold validates no cardinality, so any
//! `enum_card(n)` invented here would reject codes at or above `n` (schema-kind contract) — 18
//! distinct values in `0..=22` on this capture, with nothing bounding the next one.
//!
//! ## Four states: none
//!
//! Every value field is unconditionally present in every non-deleted row. There is
//! no presence-flag word — the only bit the fold's flag word admits is DELETED
//! (`:15467`-`:15469`), and that is a row-level *removal* (`:15480`-`:15483`), not a
//! field gate. So no field declares `.optional()`, `.nullable()` or
//! `.unknown_capable()`, and a zero is always a real measured zero. The capture
//! makes that concrete: 6 live rows carry `interface_type_raw == 0`, which is the
//! engine's own code 0 and not an absence.
//!
//! The collector cannot produce a live row with an unread field either, which is
//! what rules Unknown out rather than merely leaving it undeclared:
//! `read_compact_air_variant_stats_row` returns `None` for the whole variant when
//! any word is unreadable or out of range (`attach_log.rs:57493`-`:57538`), the
//! caller then counts `invalid_variant_rows` and **skips the entry entirely**
//! (`:55605`-`:55617`), and any non-zero `invalid_variant_rows` sets
//! `COMPACT_STATUS_PARTIAL_DATA` (`:55657`-`:55659`). d38 carries no expected-partial
//! tolerance (`compact_contract/src/lib.rs:136`-`:158` lists 26, 27, 28 and 54
//! only), so `PARTIAL_DATA` is a plain failure bit and the fold's
//! `handle_compact_failure` (`:15433`) discards the whole record. There is
//! therefore no path on which a d38 row exists and one of its words does not.
//!
//! A deleted row maps to a wire tombstone; [`record_from_row`] refuses one rather
//! than inventing a record of zeros, because the collector packs
//! `equipment_token_raw = equipment_type_token_raw = interface_type_raw =
//! wing_count = amount_raw = 0` into a delete row (`attach_log.rs:55754`-`:55769`)
//! and handing that back would turn a delete into an upsert.
//!
//! ## Sparse-after-first, not baseline-plus-delta
//!
//! `replace_or_sparse_after_first` (`world_fold.rs::replace_or_sparse_after_first`, called at `:15455`)
//! clears the accumulator only for the FIRST record that reaches it without the
//! `COMPACT_STATUS_DELTA_DATA` bit; every record after that — delta-flagged or
//! not — is applied sparsely, upserting the rows it carries and removing the keys
//! it tombstones, without clearing. So one record is not one state, and the
//! frame-level fold lives here as [`Accumulator`] rather than in a harness.
//!
//! **The survey's "146/146 full-replace-shaped" line is false and the capture says
//! so**: `runs/batch2/deepwar_t1` carries 146 d38 DATA records, the first with 201
//! rows and the remaining 145 with 814 rows between them, 47 of which carry no rows
//! at all. Reading records 2..n as snapshots would silently drop ~200 buckets an
//! hour. (The survey document's d38 section already carries the corrected reading
//! from the d32/d36 ports; this port confirms it against d38's own records.)
//!
//! The shape matches the collector, which emits a full snapshot only while its
//! tracker is empty and changed-rows-plus-tombstones after
//! (`attach_log.rs:55709`-`:55751`).
//!
//! ## Two paths return without folding
//!
//! Both are decided before any row is read, and both are the caller's to honour:
//!
//! 1. **failure status** (`handle_compact_failure`, `:15433`-`:15439`) — the
//!    domain-hour is marked unknown and the accumulator is RETAINED. Every bit of
//!    `COMPACT_STATUS_FAILURE_MASK` counts, `COMPACT_STATUS_PARTIAL_DATA` included,
//!    because d38 has no expected-partial tolerance.
//! 2. **delta-before-baseline** (`reject_delta_before_baseline`, `:15440`-`:15446`)
//!    — same treatment. [`Accumulator::fold_payload`] owns this one, because
//!    whether a baseline has been seen is state this type holds.
//!
//! **The exclusions sit between two groups of structural checks**, which is why the
//! header read is split in two. `need`, the four `ensure_count_at_most` calls,
//! `ensure_row_count_with_deletes` and `ensure_fixed_scale` all run BEFORE the
//! exclusions (`:15383`-`:15432`) and are hard errors even on a failure-status
//! record; the exact-length check runs AFTER them (`:15447`), so a failure-status
//! record whose length disagrees with its `row_count` is accepted by the fold and
//! must be accepted here. [`peek_header`] is the first group, [`split_payload`]
//! adds the second.
//!
//! **No derived field (derived-field contract).** Nothing here changes deterministically with the
//! clock: a bucket's contents move when wings re-task, take losses or are
//! re-equipped, on player and AI action. `frame_tick` is taken and ignored so every
//! domain's boundary has one shape.

use crate::body::{key, Key, KeyedState};
use crate::record::Record;
use crate::schema::{Collection, Field, Kind, Quant, Schema, Shape};
use crate::value::Value;

pub const DOMAIN: u32 = 38;
/// The one collection: one row per `(region, owner, mission, variant)` bucket.
pub const SUPERIORITY_INPUTS: u32 = 1;

/// Field ids are stable and append-only; schema-identity contract forbids reuse for a different field.
pub mod field {
    pub const REGION_ID: u32 = 1;
    pub const OWNER_TAG_DB_ID: u32 = 2;
    pub const MISSION_MASK: u32 = 3;
    pub const VARIANT_ID: u32 = 4;
    pub const EQUIPMENT_TOKEN: u32 = 5;
    pub const EQUIPMENT_TYPE_TOKEN: u32 = 6;
    pub const INTERFACE_TYPE: u32 = 7;
    pub const WING_COUNT: u32 = 8;
    pub const AMOUNT: u32 = 9;
}

// --- Payload grammar, from `world_fold.rs:991`-`:992` and `:15383`-`:15498`. ---

/// `AIR_SUPERIORITY_INPUT_HEADER_LEN`, `world_fold.rs::AIR_SUPERIORITY_INPUT_HEADER_LEN`.
pub const HEADER_LEN: usize = 52;
/// `AIR_SUPERIORITY_INPUT_ROW_LEN`, `world_fold.rs::AIR_SUPERIORITY_INPUT_ROW_LEN`. Fixed; no legacy form.
pub const ROW_LEN: usize = 48;

/// `ROW_FLAG_DELETED_LOW_BIT`, `world_fold.rs::ROW_FLAG_DELETED_LOW_BIT`, applied at `:15480`, and the
/// collector's `COMPACT_AIR_SUPERIORITY_INPUT_ROW_FLAG_DELETED`
/// (`attach_log.rs:71545`).
pub const ROW_FLAG_DELETED: u32 = 1;
/// The whole legal flag vocabulary of a d38 row: `world_fold.rs::fold_air_superiority_inputs` rejects the
/// frame outright when any other bit is set, so an unrecognised flag word means
/// the row is not this grammar and nothing in it can be trusted.
pub const ROW_KNOWN_FLAGS: u32 = ROW_FLAG_DELETED;

/// `FIXED_SCALE`, `world_fold.rs::FIXED_SCALE`, and the header word the fold asserts equal
/// to it (`:15432`, via `ensure_fixed_scale` at `world_fold.rs::ensure_fixed_scale`).
pub const FIXED_SCALE: f64 = 100_000.0;
pub const FIXED_SCALE_RAW: u32 = 100_000;
/// `FIXED_SCALE` is `10^5`, so `Quant::Fixed(5)` is exactly the source's own grid
/// — see `schema()` and `fixed_five_reproduces_every_raw_in_the_band`.
pub const SCALE_DIGITS: u8 = 5;

/// The magnitude of `amount_raw` up to which `Kind::Continuous` + `Fixed(5)`
/// reproduces the source integer **exactly**. It is a documented property, **not a
/// gate**: the mapper maps every `i64` the fold maps.
///
/// `amount_raw` is an `i64` (`world_fold.rs::fold_air_superiority_inputs`) and `Value::Continuous` is a
/// binary64, so the pair is exact only while the round trip
/// `round_ties_even((raw / 10^5) * 10^5)` returns `raw`. That holds for every
/// `|raw| <= 2^51`, which is what
/// `fixed_five_reproduces_every_raw_in_the_band` proves by a dense sweep.
///
/// **This is a sufficient band, not the maximal one, and the difference is
/// draft that called it tight). The smallest inexact magnitude found is
/// [`FIRST_KNOWN_INEXACT_AMOUNT_RAW`] — about 52.6% above this constant — so the
/// true contiguous exact band is larger than `2^51`. Nothing gates on either
/// number, so the conservative one is the safe one to publish; both are pinned by
/// tests, and the maximal band is deliberately not claimed, because "no failure
/// below X" over a 3.4e15-wide interval is not something a sweep establishes.
/// Above the band the stored value is still a legal
/// point of the declared `10^-5` grid and within one unit in the last place of the
/// source, and encode/decode remains exactly consistent (derived-field contract's escape rule keeps an
/// unstable integer off the quantized path entirely, `value.rs`), but a consumer
/// recovering the raw integer may see it differ by one. Closing that residue needs
/// an integer-domain `Fixed` in the codec, not a
/// schema-side or mapper-side decision. It is in the deliverable's open questions.
///
/// **Refusing such a row would be worse than carrying it** (the recipe's d36 rule).
/// The fold range-checks `amount_raw` not at all. The collector's own ceiling is
/// `MAX_AIR_WING_EQUIPMENT_AMOUNT_RAW` (`attach_log.rs:55648`), and a bucket that
/// would exceed it is clamped to it while raising `invalid_variant_rows` — which
/// fails the whole record — so today's producer stays far below the band. For
/// scale: the band is ~2.25e10 in game units and the measured maximum over the
/// whole `deepwar_t1` capture is 1344.0 (`amount_raw` 134,400,000).
pub const EXACT_AMOUNT_RAW_BAND: i64 = 1 << 51;

/// The smallest `|amount_raw|` known **not** to survive `Fixed(5)` unchanged:
/// `3_435_973_836_800_005` quantizes to `3_435_973_836_800_006`, while its
/// predecessor `3_435_973_836_800_004` round-trips exactly. Measured, and pinned by
/// `fixed_five_reproduces_every_raw_in_the_band`.
///
/// It exists to keep [`EXACT_AMOUNT_RAW_BAND`] honest: that constant is a
/// *sufficient* band, this is evidence that it is not the maximal one (the two
/// differ by ~52.6%), and neither is a gate — every `i64` the fold accepts is
/// mapped. Both are ~13 orders of magnitude above anything this domain has ever
/// carried; the capture's maximum is `134_400_000`.
pub const FIRST_KNOWN_INEXACT_AMOUNT_RAW: i64 = 3_435_973_836_800_005;

/// `COMPACT_STATUS_DELTA_DATA`, `compact_contract/src/lib.rs`. Named here because
/// d38 is not admitted to the production-codec dependency surface; the
/// measurement bin asserts the local value and shipping contract agree.
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

/// `COMPACT_STATUS_FAILURE_MASK`. d38 has no lane-local status bits and no
/// expected-partial tolerance (`compact_contract/src/lib.rs:136`-`:158` lists
/// domains 26, 27, 28 and 54 only), so the mask is the whole predicate for this
/// domain; the measurement bin calls `compact_contract`'s own
/// `compact_status_failure_flags_after_expected_partial_tolerance` and asserts the
/// two agree, so this constant cannot drift unnoticed.
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
    let fields = vec![
        // The key is the fold's own quadruple, in the fold's own order
        // (`world_fold.rs::fold_air_superiority_inputs`), which is also the row order and the
        // order the collector aggregates and sorts by (`attach_log.rs:55621`,
        // `:55736`).
        //
        // `region_id` is a strategic air region — map content a mod set defines,
        // so a content manifest must resolve it for the id to keep its meaning
        // (content-identity contract's deciding question). ContentId, as d31 classes the same word and
        // d20 classes `location_province`.
        Field::new(f::REGION_ID, Kind::ContentId).key(0),
        // A country tag-db id, classed as ContentId by d20 (`owner`), d31, d62 and
        // d63 for the same reason. The fold's own projection resolves it through
        // the country reference table (`world_fold.rs::insert_air_superiority_inputs`), which is the manifest
        // lookup in its shipping form.
        Field::new(f::OWNER_TAG_DB_ID, Kind::ContentId).key(1),
        // The wing's mission bitmask. `Unsigned`, NOT `ContentId`: air mission
        // types are engine constants — no mod-set manifest resolves bit 9 into a
        // different mission — which is d20's `div_type` case, an engine
        // discriminator that is stable everywhere and still not content. It is also
        // not `Enum`: key-encoding contract admits only `Unsigned`, `Signed` and `ContentId` as key
        // kinds, and a bitmask is not a code in `0..n` anyway — the fold constrains
        // it not at all (`:15464` reads the word and uses it whole), so any of the
        // 2^32 words is a legal bucket. Measured on the capture: eight distinct
        // values, `{1, 2, 4, 8, 512, 1024, 16384, 65536}`, exactly one bit each
        // (the collector skips a wing whose mask is 0, `attach_log.rs:55570`, but
        // the fold does not).
        Field::new(f::MISSION_MASK, Kind::Unsigned).key(2),
        // `Unsigned`, not `ContentId`: `variant_id` is a runtime instance identity
        // minted during play. The collector reads it out of live
        // `CDivisionEquipmentVariant` heap objects and d35 — the domain that owns
        // this catalog — classes the identical word `Unsigned` on exactly that
        // evidence (`d35.rs:136`-`:143`). Measured here: 110 distinct ids in
        // `836..=7249`, in a campaign whose whole country roster is 29 tags.
        Field::new(f::VARIANT_ID, Kind::Unsigned).key(3),
        // The three catalog words, all read raw and stored per row
        // (`world_fold.rs::fold_air_superiority_inputs`). `Unsigned` for the reasons d35 states
        // and the module docs restate: process-local token handles and an engine
        // discriminator, none of them content, and `interface_type_raw` is not an
        // `Enum` because the fold validates no cardinality. See the module docs for
        // why they are fields at all rather than a function of `variant_id`.
        Field::new(f::EQUIPMENT_TOKEN, Kind::Unsigned),
        Field::new(f::EQUIPMENT_TYPE_TOKEN, Kind::Unsigned),
        Field::new(f::INTERFACE_TYPE, Kind::Unsigned),
        // How many wings contribute to this bucket (`world_fold.rs::fold_air_superiority_inputs`, summed
        // at `attach_log.rs:55646`). `Unsigned`, not `Monotone`: schema-kind contract makes monotone
        // a hint that a value usually rises, and this one is recomputed from
        // scratch every frame over the wings currently flying that mission in that
        // region, so it falls whenever a wing re-tasks or dies. It counts a current
        // population, not a lifetime total. Measured: 1..=16, never 0 on a live row.
        Field::new(f::WING_COUNT, Kind::Unsigned),
        // The one scaled field. `Continuous` because the fold divides it by
        // `FIXED_SCALE` to produce the rendered `amount` (`world_fold.rs:9087`) — an
        // equipment pool is a fractional quantity, not a count.
        //
        // `Quant::Fixed(5)` is not a precision *budget*: `amount_raw` is a signed
        // 64-bit fixed-point word whose denominator the fold asserts to be exactly
        // `10^5` before folding anything (`world_fold.rs::fold_air_superiority_inputs`), so `fixed(5)` is the source's
        // own grid and reproduces the source integer bit for bit over the whole
        // accepted band (proved in `fixed_five_reproduces_every_raw_in_the_band`).
        // derived-field contract's "exact unless justified" bar is met by a stronger fact than a
        // precision argument: nothing is rounded at all. It also matters that the
        // raw integer survives — the fold's own renderer emits `amount_raw` beside
        // `amount` (`world_fold.rs::insert_air_superiority_inputs`), so a downstream consumer requires the integer, and
        // `fixed(5)` is precisely the quantization under which `round(value * 1e5)`
        // returns it unchanged, where `exact` would spend 8 bytes per value to
        // carry the same information.
        //
        // The comparison against `Quant::Exact` holds in BOTH directions: the lossy
        // step for a large `amount_raw` is `raw as f64` in the mapper, which both
        // quantizations share, so `exact` recovers the source integer over exactly
        // the same band and spends 8 bytes doing it. Above that band
        // (`EXACT_AMOUNT_RAW_BAND`) neither carries the integer exactly, and derived-field contract's
        // escape rule keeps the wire self-consistent either way.
        Field::new(f::AMOUNT, Kind::Continuous).quant(Quant::Fixed(SCALE_DIGITS)),
    ];

    Schema {
        domain: DOMAIN,
        // Hourly, like every other compact lane: one DATA record per game hour.
        cadence_log2: 0,
        collections: vec![Collection {
            collection_id: SUPERIORITY_INPUTS,
            // The fold upserts and deletes by key against a retained map
            // (`world_fold.rs::fold_air_superiority_inputs`) — a keyed set is what that is, and
            // only a keyed set has a delta grammar (collection-shape contract).
            shape: Shape::KeyedSet,
            fields,
        }],
        event_part: None,
    }
}

// --- The boundary mapping ----------------------------------------------------

/// The 52-byte payload header (`world_fold.rs::fold_air_superiority_inputs`, collector
/// `attach_log.rs:71531`-`:71542`).
///
/// `observed_variant_rows` at `+32` and `invalid_air_wing_rows` at `+36` are read
/// by no authority in the fold — it skips from `+28` to `+40` (`:15393`-`:15394`) —
/// but they are part of the grammar the collector writes and the independent
/// authority decoder reads (`domain_data_authority.rs:15265`-`:15266`), so they are
/// carried rather than silently dropped. Neither is constrained here, because the
/// fold constrains neither.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadHeader {
    pub elapsed_wall_ms: u64,
    pub row_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_air_wing_rows: u32,
    pub observed_loadout_rows: u32,
    pub observed_equipment_rows: u32,
    pub observed_variant_rows: u32,
    pub invalid_air_wing_rows: u32,
    pub invalid_variant_rows: u32,
    pub deleted_rows: u32,
    pub scale_raw: u32,
}

impl PayloadHeader {
    /// `COMPACT_STATUS_DELTA_DATA` (`world_fold.rs::replace_or_sparse_after_first`). Note this is *not* the
    /// same question as "is this record a sparse patch" — see
    /// [`Accumulator::fold_payload`].
    pub fn is_delta(self) -> bool {
        self.status_flags & STATUS_DELTA_DATA != 0
    }

    /// The fold's failure gate (`world_fold.rs::fold_air_superiority_inputs`).
    pub fn is_failure(self) -> bool {
        self.status_flags & FAILURE_STATUS_MASK != 0
    }
}

/// Read the 52-byte payload header and apply exactly the checks
/// `fold_air_superiority_inputs` applies BEFORE its two return-without-folding
/// paths (`world_fold.rs::fold_air_superiority_inputs`).
///
/// Six checks, in the fold's own order: the header-length `need`, four
/// `ensure_count_at_most` invariants, `ensure_row_count_with_deletes`, and
/// `ensure_fixed_scale`.
///
/// The split from [`split_payload`] exists because the fold's order is
/// load-bearing: these checks reject a frame even when its status marks the
/// domain-hour failed, while the exact-length check runs only after both
/// exclusions have passed (`world_fold.rs::fold_air_superiority_inputs`). A caller needs `status_flags` to decide the
/// exclusions, and `status_flags` lives in this header — exposing the header read
/// is what keeps that from becoming a second, hand-rolled copy of the header
/// layout in every caller.
///
/// Total: no input of any length panics.
pub fn peek_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    // `need(payload, AIR_SUPERIORITY_INPUT_HEADER_LEN, ...)`, `world_fold.rs::fold_air_superiority_inputs`.
    if payload.len() < HEADER_LEN {
        return Err(format!(
            "air_superiority_inputs payload {} shorter than the {HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    let header = PayloadHeader {
        elapsed_wall_ms: u64::from_le_bytes(payload[0..8].try_into().expect("8 bytes")),
        row_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        observed_air_wing_rows: u32_at(payload, 20),
        observed_loadout_rows: u32_at(payload, 24),
        observed_equipment_rows: u32_at(payload, 28),
        observed_variant_rows: u32_at(payload, 32),
        invalid_air_wing_rows: u32_at(payload, 36),
        invalid_variant_rows: u32_at(payload, 40),
        deleted_rows: u32_at(payload, 44),
        scale_raw: u32_at(payload, 48),
    };
    // The four `ensure_count_at_most` invariants, `world_fold.rs:15397`-`:15424`,
    // in the fold's order. All four are `<=`, not `==`: d38 is not a full-replace
    // domain, and a record carrying fewer changed rows than rows is ordinary.
    // (The shipping collector happens to set `changed_rows == row_count` — both are
    // `emitted.len()` over one vector, `attach_log.rs:71532`-`:71533` and `:55746` —
    // and all 146 records of the capture agree, but enforcing an equality no
    // authority states would reject healthy frames from any other emitter.)
    ensure_at_most("changed_rows", header.changed_rows, "row_count", header.row_count)?;
    ensure_at_most(
        "observed_loadout_rows",
        header.observed_loadout_rows,
        "observed_air_wing_rows",
        header.observed_air_wing_rows,
    )?;
    ensure_at_most(
        "invalid_variant_rows",
        header.invalid_variant_rows,
        "observed_equipment_rows",
        header.observed_equipment_rows,
    )?;
    ensure_at_most("deleted_rows", header.deleted_rows, "row_count", header.row_count)?;
    // `ensure_row_count_with_deletes`, `world_fold.rs:15425` / `:30228`: the
    // serialized rows are live buckets plus tombstones, so they cannot outnumber the
    // equipment entries observed plus the deletions declared. Widened to `u64`
    // because the fold's `saturating_add` is on `usize` (which cannot overflow on a
    // 64-bit host) while this arithmetic is on `u32` words from a file.
    let allowed = u64::from(header.observed_equipment_rows) + u64::from(header.deleted_rows);
    if u64::from(header.row_count) > allowed {
        return Err(format!(
            "air_superiority_inputs row_count {} > observed_equipment_rows {} + deleted_rows {}",
            header.row_count, header.observed_equipment_rows, header.deleted_rows
        ));
    }
    // `ensure_fixed_scale`, `world_fold.rs:15432`. This is what makes the scale a
    // constant rather than a field.
    if header.scale_raw != FIXED_SCALE_RAW {
        return Err(format!(
            "air_superiority_inputs scale_raw {} != {FIXED_SCALE_RAW}",
            header.scale_raw
        ));
    }
    Ok(header)
}

fn ensure_at_most(
    field: &str,
    value: u32,
    max_field: &str,
    max: u32,
) -> core::result::Result<(), String> {
    if value > max {
        Err(format!(
            "air_superiority_inputs {field} {value} > {max_field} {max}"
        ))
    } else {
        Ok(())
    }
}

/// Split a d38 DATA payload into its header and its rows slice: [`peek_header`]
/// plus the exact-length check the fold makes after its exclusions
/// (`world_fold.rs::fold_air_superiority_inputs`).
///
/// This lives here rather than in each caller so the header grammar has exactly one
/// implementation: a measurement harness that re-derives it is a harness that can
/// drift from the recorder without anything failing. Per-row semantics stay in
/// [`parse_row`] and [`record_from_row`].
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, &[u8]), String> {
    let header = peek_header(payload)?;
    // Checked arithmetic: `row_count` is arbitrary input from a stream on disk, and
    // `row_count * ROW_LEN` overflows `usize` for a large enough word. The fold
    // saturates instead (`:15447`), which rejects the same payloads by way of the
    // length comparison; checked multiplication says so directly.
    let expected = (header.row_count as usize)
        .checked_mul(ROW_LEN)
        .and_then(|len| len.checked_add(HEADER_LEN))
        .ok_or_else(|| {
            format!(
                "air_superiority_inputs row_count {} overflows",
                header.row_count
            )
        })?;
    if payload.len() != expected {
        return Err(format!(
            "air_superiority_inputs len {} != {expected}",
            payload.len()
        ));
    }
    Ok((header, &payload[HEADER_LEN..]))
}

/// One parsed 48-byte row.
///
/// The five value words are carried even on a deleted row, where the fold ignores
/// them (`world_fold.rs::fold_air_superiority_inputs` removes the key and never reads past `+36`).
/// They are `0` in every delete row the shipping collector emits
/// (`attach_log.rs:55754`-`:55769`) but the fold does not require that, so this does
/// not either — rejecting a row the fold accepts would drop real data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Row {
    pub region_id: u32,
    pub owner_tag_db_id: u32,
    pub mission_mask: u32,
    pub variant_id: u32,
    pub deleted: bool,
    pub equipment_token_raw: u32,
    pub equipment_type_token_raw: u32,
    pub interface_type_raw: u32,
    pub wing_count: u32,
    pub amount_raw: i64,
}

/// Parse one 48-byte row, applying every per-row check the fold applies, in the
/// fold's order (`world_fold.rs::fold_air_superiority_inputs`).
///
/// **No key component has a range check**, and that is the fold's decision, not an
/// omission here: `world_fold.rs::fold_air_superiority_inputs` read the four words and use them whole. The
/// collector will not produce a `region_id` outside `1..=MAX_AIR_WING_MISSION_REGION_ID`
/// (`attach_log.rs:55576`), nor a zero `mission_mask` (`:55570`), nor a
/// `variant_id` with no live variant object behind it (`:55605`) — and the capture
/// contains none — but the fold is the authority and it accepts them, so a stricter
/// reading here would refuse frames the recorder folds.
///
/// Total: no input of any length panics.
pub fn parse_row(row: &[u8]) -> core::result::Result<Row, String> {
    check_row_len(row)?;
    let region_id = u32_at(row, 0);
    let owner_tag_db_id = u32_at(row, 4);
    let mission_mask = u32_at(row, 8);
    let variant_id = u32_at(row, 12);
    let flags = u32_at(row, 16);
    // `world_fold.rs::fold_air_superiority_inputs`.
    if flags & !ROW_KNOWN_FLAGS != 0 {
        return Err(format!(
            "air_superiority_inputs row has unknown flags {flags:#x}"
        ));
    }
    // `world_fold.rs::fold_air_superiority_inputs`. Read after the flag check, as the fold does.
    let reserved = u32_at(row, 36);
    if reserved != 0 {
        return Err(format!(
            "air_superiority_inputs row reserved field is nonzero ({reserved})"
        ));
    }
    Ok(Row {
        region_id,
        owner_tag_db_id,
        mission_mask,
        variant_id,
        deleted: flags & ROW_FLAG_DELETED != 0,
        equipment_token_raw: u32_at(row, 20),
        equipment_type_token_raw: u32_at(row, 24),
        interface_type_raw: u32_at(row, 28),
        wing_count: u32_at(row, 32),
        // Every `i64` the fold accepts is mapped; see `EXACT_AMOUNT_RAW_BAND` for
        // where the raw-integer recovery stops being exact and why a cutoff here
        // would be the worse failure.
        amount_raw: i64_at(row, 40),
    })
}

/// The key of one compact row: the fold's `(region_id, owner_tag_db_id,
/// mission_mask, variant_id)` quadruple (`world_fold.rs::fold_air_superiority_inputs`), in the
/// fold's order, which is also the row order.
pub fn key_from_row(row: &[u8]) -> core::result::Result<Key, String> {
    let parsed = parse_row(row)?;
    Ok(row_key(&parsed))
}

/// The key of an already-parsed row.
pub fn row_key(row: &Row) -> Key {
    key(&[
        row.region_id as u64,
        row.owner_tag_db_id as u64,
        row.mission_mask as u64,
        row.variant_id as u64,
    ])
}

/// Whether this row removes its key from the set rather than carrying a state
/// (`world_fold.rs::fold_air_superiority_inputs`).
pub fn is_deleted_row(row: &[u8]) -> core::result::Result<bool, String> {
    Ok(parse_row(row)?.deleted)
}

/// One compact row of the `superiority_inputs` table, as the collector emits it
/// today, onto its stored Record. `row` is the raw 48-byte row slice (the caller
/// has already consumed the payload header via [`split_payload`]); d38 has exactly
/// one row form, so length selects nothing and a wrong length is rejected.
/// `frame_tick` is derived-field contract's derived-field input (stored = v + c * (frame_tick >> u));
/// d38 declares no derived field, so it is taken and ignored — every domain's
/// mapper has the same shape.
///
/// Fail-closed throughout, mirroring `fold_air_superiority_inputs`: a row the fold
/// would reject is an `Err` here, never a coerced value. A **deleted** row is an
/// `Err` too — it is a removal (`world_fold.rs::fold_air_superiority_inputs`) and has no record; a caller
/// maps it to a wire tombstone, and quietly handing back a record of the zeros the
/// collector packs into a delete row would turn a delete into an upsert. Nothing in
/// this function can panic on any input.
pub fn record_from_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    // d38 has no derived field; derived-field contract's transform input is unused. Named rather than
    // `_frame_tick` because the signature is the cross-domain contract.
    let _ = frame_tick;
    let parsed = parse_row(row)?;
    record_from_parsed(collection, &parsed)
}

/// [`record_from_row`] for an already-parsed row, so a caller that needs both the
/// key and the record parses the bytes once.
pub fn record_from_parsed(
    collection: &Collection,
    parsed: &Row,
) -> core::result::Result<Record, String> {
    if parsed.deleted {
        return Err(format!(
            "air_superiority_inputs row region={} owner={} mask={:#x} variant={} is a delete row, \
             not a record",
            parsed.region_id, parsed.owner_tag_db_id, parsed.mission_mask, parsed.variant_id
        ));
    }
    let mut values = Vec::with_capacity(collection.fields.len());
    for f in collection.value_fields() {
        let value = match f.field_id {
            // No gate, no scale: `world_fold.rs::fold_air_superiority_inputs` reads each word and
            // stores it. A zero is a real measured zero — 6 live rows of the capture
            // carry `interface_type_raw == 0`.
            field::EQUIPMENT_TOKEN => Value::Unsigned(parsed.equipment_token_raw as u64),
            field::EQUIPMENT_TYPE_TOKEN => Value::Unsigned(parsed.equipment_type_token_raw as u64),
            field::INTERFACE_TYPE => Value::Unsigned(parsed.interface_type_raw as u64),
            field::WING_COUNT => Value::Unsigned(parsed.wing_count as u64),
            // `world_fold.rs:9087` — the fold's own render divides by `FIXED_SCALE`,
            // and `:15432` has already proved the payload's own scale word equals it.
            field::AMOUNT => Value::Continuous(parsed.amount_raw as f64 / FIXED_SCALE),
            other => {
                // A collection this mapper does not know how to fill. Filling it
                // with a placeholder would publish invented data under a field id
                // that means something else.
                return Err(format!(
                    "air_superiority_inputs has no mapping for field id {other}"
                ));
            }
        };
        values.push(value);
    }
    Ok(Record::new(values))
}

// --- The shipping incremental layer ------------------------------------------

/// What one d38 DATA record did to the accumulator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FoldOutcome {
    /// The FIRST record to reach `replace_or_sparse_after_first` without the delta
    /// bit (`world_fold.rs::replace_or_sparse_after_first`, called at `:15455`): the map was cleared and
    /// refilled, and the domain now has a baseline.
    FullReplace,
    /// Applied sparsely on top of the retained map — rows upserted, tombstoned keys
    /// removed, everything else left alone. This is EVERY record after the first,
    /// whether or not it carries the delta bit.
    Sparse,
    /// A delta-flagged record that arrived before any baseline.
    /// `reject_delta_before_baseline` (`world_fold.rs::reject_delta_before_baseline`, called at `:15440`)
    /// marks the domain-hour unknown, folds **nothing**, and retains the
    /// accumulator. The hour carries no d38 data and belongs in no measurement.
    DeltaBeforeBaseline,
}

/// The running d38 state, replaying `fold_air_superiority_inputs`'s incremental
/// grammar.
///
/// This is what makes d38 different from a full-replace domain, so it lives beside
/// the schema rather than in a harness: the measurement bin, the tests and any
/// future caller share one implementation of clear-on-first, sparse-thereafter,
/// delete-by-flag and delta-before-baseline.
///
/// `fold_air_superiority_inputs` is the accumulator's only writer: nothing else in
/// `world_fold.rs` touches `self.air_superiority_inputs` except the projection at
/// `:9074`, which reads it (verified by grep over `recorder_host` and `collector`).
/// Unlike d26, no other domain's rows prune it, and unlike d32 no end-of-hour sweep
/// removes orphans from it.
#[derive(Clone, Debug, Default)]
pub struct Accumulator {
    /// The `superiority_inputs` collection's current content, keyed by
    /// `(region_id, owner_tag_db_id, mission_mask, variant_id)`.
    pub rows: KeyedState,
    baseline_seen: bool,
}

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a record has already cleared-and-refilled the map — the
    /// `delta_baselines_seen` membership `replace_or_sparse_after_first`
    /// (`world_fold.rs::replace_or_sparse_after_first`) and `reject_delta_before_baseline` (`world_fold.rs::reject_delta_before_baseline`) both
    /// consult. It is set by the first non-delta record that gets past both
    /// exclusions, and by nothing else.
    pub fn baseline_seen(&self) -> bool {
        self.baseline_seen
    }

    /// Fold one d38 DATA payload, exactly as `fold_air_superiority_inputs` folds it
    /// from `:15440` onward.
    ///
    /// `is_delta` is the caller's reading of `COMPACT_STATUS_DELTA_DATA` in
    /// `header.status_flags`. It is a parameter rather than a bit tested here
    /// because that constant belongs to `compact_contract` and d38 is not
    /// admitted to the production-codec dependency surface. Copying the literal
    /// into this module is exactly the drift the shared-contract rule forbids.
    ///
    /// **The caller must already have excluded failure-status records.** The fold
    /// returns at `:15433` before reaching any of this, retaining the accumulator;
    /// the predicate is `compact_contract`'s
    /// `compact_status_failure_flags_after_expected_partial_tolerance`, never a
    /// hand-copied mask. Delta-before-baseline is handled here instead, because
    /// `baseline_seen` is state this type holds.
    ///
    /// `frame_tick` is derived-field contract's derived-field input; d38 declares no derived field.
    ///
    /// **Atomic on `Err`.** Every row is parsed and mapped before anything is
    /// written, so a payload this rejects leaves the accumulator exactly as it found
    /// it. The fold itself is not atomic — it clears and then fails partway through
    /// the row loop — but every one of its `Err`s aborts the stream fold, so no
    /// reader ever observes the difference, and the stricter property is the safer
    /// one for a caller that reports and stops.
    pub fn fold_payload(
        &mut self,
        collection: &Collection,
        payload: &[u8],
        is_delta: bool,
        frame_tick: u64,
    ) -> core::result::Result<FoldOutcome, String> {
        // d38 has no derived field; derived-field contract's transform input is unused. Named rather
        // than `_frame_tick` because the signature is the cross-domain contract.
        let _ = frame_tick;
        // `world_fold.rs::fold_air_superiority_inputs`: the header-structural checks that precede
        // both exclusions.
        peek_header(payload)?;
        // `:15440`-`:15446`. Decided before the exact-length check and before
        // anything is cleared, so the accumulator survives untouched — and a payload
        // whose length disagrees with its `row_count` is NOT an error on this path,
        // because the fold never reaches `:15447`.
        if is_delta && !self.baseline_seen {
            return Ok(FoldOutcome::DeltaBeforeBaseline);
        }
        // `:15447`-`:15454`.
        let (_, rows) = split_payload(payload)?;

        // Parse and map everything first, so a structural error leaves the
        // accumulator as it was.
        let mut parsed: Vec<(Key, bool, Option<Record>)> = Vec::with_capacity(rows.len() / ROW_LEN);
        for row in rows.chunks_exact(ROW_LEN) {
            let fields = parse_row(row)?;
            let record = if fields.deleted {
                None
            } else {
                Some(record_from_parsed(collection, &fields)?)
            };
            parsed.push((row_key(&fields), fields.deleted, record));
        }

        // `:15455`-`:15459`: only the first non-delta record clears.
        let sparse = is_delta || self.baseline_seen;
        if !sparse {
            self.rows.clear();
        }
        // `:15460`-`:15499`. Rows are applied in payload order, so a key repeated
        // inside one frame resolves the way the fold's `BTreeMap` resolves it — last
        // writer wins, and a delete after an insert of the same key in one frame
        // removes it. The fold has no duplicate rejection, so neither does this; the
        // capture contains no intra-frame duplicate.
        for (k, deleted, record) in parsed {
            if deleted {
                // `:15481`: a tombstone for a key that is not there is a no-op, not
                // an error.
                self.rows.remove(&k);
            } else {
                self.rows.insert(k, record.expect("a live row has a record"));
            }
        }
        if !sparse {
            self.baseline_seen = true;
            Ok(FoldOutcome::FullReplace)
        } else {
            Ok(FoldOutcome::Sparse)
        }
    }
}

fn check_row_len(row: &[u8]) -> core::result::Result<(), String> {
    if row.len() != ROW_LEN {
        return Err(format!(
            "air_superiority_inputs row length {} != {ROW_LEN}",
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

    /// A raw 48-byte row, built the way the collector packs one
    /// (`attach_log.rs:71549`-`:71559`).
    #[allow(clippy::too_many_arguments)]
    fn raw_row(
        region_id: u32,
        owner_tag_db_id: u32,
        mission_mask: u32,
        variant_id: u32,
        flags: u32,
        equipment_token_raw: u32,
        equipment_type_token_raw: u32,
        interface_type_raw: u32,
        wing_count: u32,
        reserved: u32,
        amount_raw: i64,
    ) -> Vec<u8> {
        let mut row = Vec::with_capacity(ROW_LEN);
        for word in [
            region_id,
            owner_tag_db_id,
            mission_mask,
            variant_id,
            flags,
            equipment_token_raw,
            equipment_type_token_raw,
            interface_type_raw,
            wing_count,
            reserved,
        ] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        row.extend_from_slice(&amount_raw.to_le_bytes());
        assert_eq!(row.len(), ROW_LEN);
        row
    }

    /// A live row with the capture's own shape.
    fn live_row(
        region_id: u32,
        owner_tag_db_id: u32,
        mission_mask: u32,
        variant_id: u32,
        amount_raw: i64,
    ) -> Vec<u8> {
        raw_row(
            region_id,
            owner_tag_db_id,
            mission_mask,
            variant_id,
            0,
            20_190,
            17,
            5,
            3,
            0,
            amount_raw,
        )
    }

    fn delete_row(
        region_id: u32,
        owner_tag_db_id: u32,
        mission_mask: u32,
        variant_id: u32,
    ) -> Vec<u8> {
        raw_row(
            region_id,
            owner_tag_db_id,
            mission_mask,
            variant_id,
            ROW_FLAG_DELETED,
            0,
            0,
            0,
            0,
            0,
            0,
        )
    }

    /// The header words in payload order, so a test can perturb exactly one.
    #[derive(Clone, Copy)]
    struct Header {
        elapsed_wall_ms: u64,
        row_count: u32,
        changed_rows: u32,
        status_flags: u32,
        observed_air_wing_rows: u32,
        observed_loadout_rows: u32,
        observed_equipment_rows: u32,
        observed_variant_rows: u32,
        invalid_air_wing_rows: u32,
        invalid_variant_rows: u32,
        deleted_rows: u32,
        scale_raw: u32,
    }

    /// A whole payload with the collector's own header arithmetic
    /// (`attach_log.rs:71531`-`:71542`).
    fn payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        payload_with(status_flags, rows, |h| h)
    }

    fn payload_with(
        status_flags: u32,
        rows: &[Vec<u8>],
        perturb: impl FnOnce(Header) -> Header,
    ) -> Vec<u8> {
        let deleted = rows
            .iter()
            .filter(|r| u32_at(r, 16) & ROW_FLAG_DELETED != 0)
            .count() as u32;
        let h = perturb(Header {
            elapsed_wall_ms: 0,
            row_count: rows.len() as u32,
            changed_rows: rows.len() as u32,
            status_flags,
            observed_air_wing_rows: 643,
            observed_loadout_rows: 643,
            observed_equipment_rows: 1999,
            observed_variant_rows: 204,
            invalid_air_wing_rows: 0,
            invalid_variant_rows: 0,
            deleted_rows: deleted,
            scale_raw: FIXED_SCALE_RAW,
        });
        let mut p = Vec::new();
        p.extend_from_slice(&h.elapsed_wall_ms.to_le_bytes());
        for word in [
            h.row_count,
            h.changed_rows,
            h.status_flags,
            h.observed_air_wing_rows,
            h.observed_loadout_rows,
            h.observed_equipment_rows,
            h.observed_variant_rows,
            h.invalid_air_wing_rows,
            h.invalid_variant_rows,
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
            .expect("d38 schema is well-formed");
    }

    #[test]
    fn it_is_a_keyed_set_keyed_by_the_region_owner_mission_variant_quadruple() {
        let c = collection();
        assert_eq!(
            c.shape,
            Shape::KeyedSet,
            "the fold upserts and deletes against a retained map"
        );
        let keys = c.key_fields();
        assert_eq!(
            keys.len(),
            4,
            "AirSuperiorityInputKey is a quadruple (`world_fold.rs:15474`)"
        );
        assert_eq!(keys[0].field_id, field::REGION_ID);
        assert_eq!(keys[1].field_id, field::OWNER_TAG_DB_ID);
        assert_eq!(keys[2].field_id, field::MISSION_MASK);
        assert_eq!(keys[3].field_id, field::VARIANT_ID);
        assert_eq!(
            keys[0].kind,
            Kind::ContentId,
            "an air region is map content a manifest must resolve (content-identity contract)"
        );
        assert_eq!(
            keys[1].kind,
            Kind::ContentId,
            "a country tag-db id is content, as d20/d31/d62/d63 class it"
        );
        assert_eq!(
            keys[2].kind,
            Kind::Unsigned,
            "the mission bitmask is an engine discriminator, not content"
        );
        assert_eq!(
            keys[3].kind,
            Kind::Unsigned,
            "a variant id is a runtime instance identity, as d35 classes it"
        );
        for k in &keys {
            assert!(
                k.kind.legal_as_key(),
                "field {} is a legal key kind",
                k.field_id
            );
            assert!(
                !k.optional && !k.nullable && !k.unknown_capable && k.derived.is_none(),
                "a key component is concrete and present in every row"
            );
        }
        assert_eq!(c.fields.len(), 9, "4 key components + 5 value fields");
        assert_eq!(c.value_fields().count(), 5);
    }

    #[test]
    fn the_content_id_sweep_is_recorded_not_silent() {
        // the ContentId retrofit sweep needs "checked, and these are the ones"
        // rather than silence. Two ContentId fields, BOTH key components — the
        // heavier retrofit class, because a manifest remap reorders keys and changes
        // the byte layout of every baseline and key list, not just one value.
        let c = collection();
        let content: Vec<u32> = c
            .fields
            .iter()
            .filter(|f| f.kind == Kind::ContentId)
            .map(|f| f.field_id)
            .collect();
        assert_eq!(content, vec![field::REGION_ID, field::OWNER_TAG_DB_ID]);
        assert!(c
            .fields
            .iter()
            .filter(|f| f.kind == Kind::ContentId)
            .all(|f| f.is_key()));
    }

    #[test]
    fn no_field_declares_any_empty_state() {
        // Recipe step 8 case 3: the schema declares NO capability, so there is no
        // absent/null/unknown raw-row case to build. The reason is the row grammar
        // itself — the only flag bit the fold accepts is DELETED
        // (`world_fold.rs::fold_air_superiority_inputs`), which removes the key rather than gating a field,
        // so every value field is unconditionally present in every row that becomes
        // a record; and every collector path that cannot read a word drops the whole
        // record through a failure status rather than emitting a gated-off row (see
        // the module docs). `a_real_zero_is_a_value`,
        // `the_mapper_reads_every_field_at_its_own_offset` and
        // `a_delete_row_is_a_tombstone_not_a_record` cover the states the mapper can
        // actually reach.
        let c = collection();
        assert_eq!(c.optional_count(), 0, "no field is ever inapplicable");
        assert!(c.value_fields().all(|f| !f.nullable));
        assert!(c.value_fields().all(|f| !f.unknown_capable));
        assert!(c.key_fields().iter().all(|f| !f.optional));
    }

    #[test]
    fn no_field_is_derived() {
        // derived-field contract derived fields are clock-linear integers; a superiority bucket moves on
        // re-tasking, losses and refits, not on the clock, so `frame_tick` never
        // enters a value.
        assert!(collection().fields.iter().all(|f| f.derived.is_none()));
    }

    #[test]
    fn a_full_row_roundtrips_byte_for_byte() {
        let c = collection();
        let record = record_from_row(&c, &live_row(37, 12, 512, 7249, 13_440_000), 0).expect("maps");
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
    fn a_real_zero_is_a_value() {
        // record-state contract keeps zero out of the three empty states. Measured instance: 6 live
        // rows of the capture carry `interface_type_raw == 0`, the engine's own
        // code 0. `wing_count` and `amount_raw` are zero only in a delete row today,
        // but the fold accepts a live row carrying either, and a mapper that turned
        // one into an empty state would contradict a schema declaring none.
        let c = collection();
        let row = raw_row(37, 12, 512, 7249, 0, 20_190, 17, 0, 0, 0, 0);
        let r = record_from_row(&c, &row, 0).expect("maps");
        for id in [field::INTERFACE_TYPE, field::WING_COUNT] {
            let v = r.values[index_of(&c, id)];
            assert_eq!(v, Value::Unsigned(0), "field {id}");
            assert!(v.carries_value() && v.is_present() && !v.is_null() && !v.is_unknown());
        }
        let amount = r.values[index_of(&c, field::AMOUNT)];
        assert_eq!(amount, Value::Continuous(0.0));
        assert!(amount.carries_value());

        // And it round-trips as a value, not as an empty state.
        let mut bytes = Vec::new();
        r.encode(&c, &mut bytes).expect("encodes");
        let (back, _) = Record::decode(&c, &bytes, 0).expect("decodes");
        assert_eq!(back, r);
    }

    #[test]
    fn the_mapper_never_emits_an_empty_state() {
        // The states test, in the only form d38 admits: over the space of rows the
        // grammar accepts, every mapped value carries a value. A mapper that emitted
        // Absent/Null/Unknown here would contradict a schema that declares none of
        // the three, and the encoder would reject it.
        let c = collection();
        for amount in [
            0i64,
            1,
            -1,
            13_440_000,
            EXACT_AMOUNT_RAW_BAND,
            EXACT_AMOUNT_RAW_BAND + 1,
            i64::MAX,
            i64::MIN,
        ] {
            for (region, owner, mask, variant) in [
                (0u32, 0u32, 0u32, 0u32),
                (1, 1, 1, 836),
                (248, 368, 65_536, 7249),
                (u32::MAX, u32::MAX, u32::MAX, u32::MAX),
            ] {
                for (token, type_token, iface, wings) in
                    [(0u32, 0u32, 0u32, 0u32), (20_190, 17, 5, 3), (u32::MAX, u32::MAX, u32::MAX, u32::MAX)]
                {
                    let row = raw_row(
                        region, owner, mask, variant, 0, token, type_token, iface, wings, 0, amount,
                    );
                    let r = record_from_row(&c, &row, 0)
                        .unwrap_or_else(|e| panic!("row maps: {e}"));
                    for v in &r.values {
                        assert!(v.carries_value(), "{v:?}");
                    }
                    let mut bytes = Vec::new();
                    r.encode(&c, &mut bytes)
                        .expect("a declared-capability-free record encodes");
                }
            }
        }
    }

    #[test]
    fn a_delete_row_is_a_tombstone_not_a_record() {
        // The collector packs zeros into a delete row (`attach_log.rs:55754`), so
        // mapping one would publish a fabricated all-zero bucket for a key that no
        // longer exists — a delete silently turned into an upsert.
        let c = collection();
        let row = delete_row(37, 12, 512, 7249);
        assert!(record_from_row(&c, &row, 0).is_err());
        assert!(record_from_parsed(&c, &parse_row(&row).expect("parses")).is_err());
        assert!(is_deleted_row(&row).expect("flags"));
        // The key is still readable — that is what the tombstone carries.
        assert_eq!(key_from_row(&row).expect("keys"), key(&[37, 12, 512, 7249]));
        // A delete row carrying non-zero payload words is still accepted as a
        // tombstone: the fold never reads past `+36` on that path (`:15480`).
        let noisy = raw_row(37, 12, 512, 7249, ROW_FLAG_DELETED, 9, 9, 9, 9, 0, 77);
        assert!(is_deleted_row(&noisy).expect("flags"));
        assert!(record_from_row(&c, &noisy, 0).is_err());
    }

    #[test]
    fn the_mapper_reads_every_field_at_its_own_offset() {
        // Step 8.6. Every row word gets a distinct known value, so a transposed read
        // lands on a recognisably wrong number — the same failure class as a wrong
        // scale, and the one no round-trip assertion can see.
        let c = collection();
        let row = raw_row(
            0x1111_1111,
            0x2222_2222,
            0x3333_3333,
            0x4444_4444,
            0,
            0x5555_5555,
            0x6666_6666,
            0x7777_7777,
            0x8888_8888,
            0,
            0x0999_9999_9999_9999,
        );
        let parsed = parse_row(&row).expect("parses");
        assert_eq!(parsed.region_id, 0x1111_1111);
        assert_eq!(parsed.owner_tag_db_id, 0x2222_2222);
        assert_eq!(parsed.mission_mask, 0x3333_3333);
        assert_eq!(parsed.variant_id, 0x4444_4444);
        assert_eq!(parsed.equipment_token_raw, 0x5555_5555);
        assert_eq!(parsed.equipment_type_token_raw, 0x6666_6666);
        assert_eq!(parsed.interface_type_raw, 0x7777_7777);
        assert_eq!(parsed.wing_count, 0x8888_8888);
        assert_eq!(parsed.amount_raw, 0x0999_9999_9999_9999);
        assert_eq!(
            key_from_row(&row).expect("keys"),
            key(&[0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444]),
            "the key is the first four words, in row order"
        );
        let r = record_from_row(&c, &row, 0).expect("maps");
        assert_eq!(
            r.values[index_of(&c, field::EQUIPMENT_TOKEN)],
            Value::Unsigned(0x5555_5555)
        );
        assert_eq!(
            r.values[index_of(&c, field::EQUIPMENT_TYPE_TOKEN)],
            Value::Unsigned(0x6666_6666)
        );
        assert_eq!(
            r.values[index_of(&c, field::INTERFACE_TYPE)],
            Value::Unsigned(0x7777_7777)
        );
        assert_eq!(
            r.values[index_of(&c, field::WING_COUNT)],
            Value::Unsigned(0x8888_8888)
        );
        // The flag word reads at +16 and the reserved word at +36, not the other way
        // round: a delete bit in the reserved slot is a rejected row, and a non-zero
        // word in the flag slot is a rejected row.
        assert!(parse_row(&raw_row(1, 2, 3, 4, 0, 0, 0, 0, 0, ROW_FLAG_DELETED, 0)).is_err());
        assert!(
            parse_row(&raw_row(1, 2, 3, 4, ROW_FLAG_DELETED, 0, 0, 0, 0, 0, 0))
                .expect("a delete row parses")
                .deleted
        );
    }

    #[test]
    fn amount_reads_at_offset_40_through_the_hundred_thousand_scale() {
        // The test that catches a 32768-vs-100000 mapping error and a transposed
        // offset in the same breath: known fixed-point words against their exact f64
        // readings.
        let c = collection();
        let cases: [(i64, f64); 9] = [
            (100_000, 1.0),
            (150_000, 1.5),
            (-100_000, -1.0),
            (1, 0.000_01),
            (-1, -0.000_01),
            (13_440_000, 134.4),
            (134_400_000, 1344.0), // the capture's maximum
            (0, 0.0),
            (32_768, 0.327_68), // a /32768 reading would produce 1.0 here
        ];
        for (raw, expected) in cases {
            let r = record_from_row(&c, &live_row(37, 12, 512, 7249, raw), 0).expect("maps");
            assert_eq!(
                r.values[index_of(&c, field::AMOUNT)],
                Value::Continuous(expected),
                "raw {raw} must read {expected} (scale {FIXED_SCALE})"
            );
        }
    }

    #[test]
    fn peek_header_reads_every_word_at_its_own_offset() {
        // The header half of step 8.6: a transposed header word mis-slices every row
        // behind it and no round-trip or replay assertion can see that, because both
        // arms agree on the wrong slice. Distinct values everywhere, and the two
        // words the fold does not read are carried, not dropped.
        let p = payload_with(0x0400, &[], |mut h| {
            h.elapsed_wall_ms = 0x0102_0304_0506_0708;
            h.row_count = 0;
            h.changed_rows = 0;
            h.observed_air_wing_rows = 643;
            h.observed_loadout_rows = 611;
            h.observed_equipment_rows = 1999;
            h.observed_variant_rows = 204;
            h.invalid_air_wing_rows = 7;
            h.invalid_variant_rows = 11;
            h.deleted_rows = 0;
            h
        });
        let h = peek_header(&p).expect("well-formed");
        assert_eq!(h.elapsed_wall_ms, 0x0102_0304_0506_0708);
        assert_eq!(h.row_count, 0);
        assert_eq!(h.changed_rows, 0);
        assert_eq!(h.status_flags, 0x0400);
        assert_eq!(h.observed_air_wing_rows, 643);
        assert_eq!(h.observed_loadout_rows, 611);
        assert_eq!(h.observed_equipment_rows, 1999);
        assert_eq!(h.observed_variant_rows, 204);
        assert_eq!(h.invalid_air_wing_rows, 7);
        assert_eq!(h.invalid_variant_rows, 11);
        assert_eq!(h.deleted_rows, 0);
        assert_eq!(h.scale_raw, FIXED_SCALE_RAW);
        assert!(h.is_delta(), "0x400 is COMPACT_STATUS_DELTA_DATA");
        assert!(!h.is_failure());
    }

    #[test]
    fn scale_word_is_invariant_not_data() {
        // `world_fold.rs:15432` asserts the header's scale word equals `FIXED_SCALE`,
        // so it is constant over every payload the grammar accepts and carries no
        // information. The invariant is enforced in `peek_header`; no field
        // re-encodes it.
        // The whole field vocabulary, so a scale field cannot be added unnoticed:
        // four key components and five value fields, and nothing else.
        let ids: Vec<u32> = collection().fields.iter().map(|f| f.field_id).collect();
        assert_eq!(
            ids,
            vec![
                field::REGION_ID,
                field::OWNER_TAG_DB_ID,
                field::MISSION_MASK,
                field::VARIANT_ID,
                field::EQUIPMENT_TOKEN,
                field::EQUIPMENT_TYPE_TOKEN,
                field::INTERFACE_TYPE,
                field::WING_COUNT,
                field::AMOUNT,
            ],
            "the scale word is an invariant, not a tenth field"
        );
        assert_eq!(collection().value_fields().count(), 5);
        for wrong in [0u32, 1, 32_768, 99_999, 100_001, 1_000_000, u32::MAX] {
            let p = payload_with(0, &[live_row(1, 2, 4, 8, 100_000)], |mut h| {
                h.scale_raw = wrong;
                h
            });
            assert!(
                peek_header(&p).is_err(),
                "scale_raw {wrong} must be refused, not carried"
            );
            assert!(split_payload(&p).is_err(), "scale_raw {wrong}");
        }
        assert!(peek_header(&payload(0, &[live_row(1, 2, 4, 8, 100_000)])).is_ok());
    }

    #[test]
    fn fixed_five_reproduces_every_raw_in_the_band() {
        // The Fixed(5)-vs-Exact defence, as an executable claim: over the whole
        // accepted `amount_raw` band, `quantize(raw / 10^5)` returns `raw` unchanged
        // — nothing is rounded, and the raw integer the fold's own renderer emits
        // beside the scaled value (`world_fold.rs::insert_air_superiority_inputs`) survives the wire. Edges,
        // decades, and a deterministic sweep of the top of the band, where the
        // property fails first.
        let q = Quant::Fixed(SCALE_DIGITS);
        let mut raws: Vec<i64> = vec![0, 1, -1, 5, 99_999, 100_000, 100_001, 134_400_000];
        for decade in 0..16u32 {
            raws.push(10i64.pow(decade));
            raws.push(-10i64.pow(decade));
            raws.push(10i64.pow(decade) + 1);
        }
        for step in 0..2_000i64 {
            raws.push(EXACT_AMOUNT_RAW_BAND - step);
            raws.push(-(EXACT_AMOUNT_RAW_BAND - step));
        }
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

        // The band is SUFFICIENT, not maximal, and that distinction is pinned here
        // called it tight, which the numbers below refute).
        //
        // The smallest known inexact magnitude and its predecessor, both directions:
        // the predecessor round-trips, the value itself does not.
        for sign in [1i64, -1] {
            let last_good = sign * (FIRST_KNOWN_INEXACT_AMOUNT_RAW - 1);
            assert_eq!(
                quantize(last_good as f64 / FIXED_SCALE, q),
                Some(last_good),
                "raw {last_good} is the last known exact magnitude and must round-trip"
            );
            let first_bad = sign * FIRST_KNOWN_INEXACT_AMOUNT_RAW;
            assert_ne!(
                quantize(first_bad as f64 / FIXED_SCALE, q),
                Some(first_bad),
                "raw {first_bad} is the smallest known inexact magnitude; if it now \
                 round-trips, FIRST_KNOWN_INEXACT_AMOUNT_RAW is stale"
            );
        }
        // ...so the exact band genuinely extends past `EXACT_AMOUNT_RAW_BAND`, and
        // the published constant is conservative by roughly half again.
        assert!(
            FIRST_KNOWN_INEXACT_AMOUNT_RAW > EXACT_AMOUNT_RAW_BAND,
            "the published band must not exceed the smallest known failure"
        );
        // And failures really do exist inside `[2^51, 2^52)` — they simply do not
        // start at `2^51`.
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let span = (EXACT_AMOUNT_RAW_BAND as u64) - 1;
        let mut failures = 0usize;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let raw = EXACT_AMOUNT_RAW_BAND + (state % span) as i64;
            if quantize(raw as f64 / FIXED_SCALE, q) != Some(raw) {
                failures += 1;
            }
        }
        assert!(
            failures > 0,
            "some raw in [2^51, 2^52) must fail to reproduce, or the whole band \
             analysis is wrong"
        );
    }

    #[test]
    fn every_i64_amount_survives_encode_and_decode_unchanged() {
        // The property that must hold over the WHOLE `i64` domain, since the mapper
        // gates on magnitude nowhere: whatever the mapper produced, the bytes
        // reproduce it exactly and re-encode identically. Above
        // `EXACT_AMOUNT_RAW_BAND` the recovered *raw integer* may drift by one (see
        // that constant), but the wire never disagrees with itself — derived-field contract's escape rule
        // puts an unstable value on the raw-f64 path instead.
        let c = collection();
        let mut raws: Vec<i64> = vec![
            0,
            1,
            -1,
            134_400_000,
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
            let record = record_from_row(&c, &live_row(1, 2, 4, 8, raw), 0)
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

        let mut fixed_state: KeyedState = KeyedState::new();
        let mut exact_state: KeyedState = KeyedState::new();
        // A realistic table: the capture's own shape, 201 buckets over 18 regions,
        // 29 owners and 110 variants.
        for i in 0..201u32 {
            let row = live_row(
                1 + i % 18,
                1 + i % 29,
                1 << (i % 4),
                836 + i * 37,
                100_000 * (1 + i as i64 % 1344),
            );
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
        for len in [0usize, 1, ROW_LEN - 1, ROW_LEN + 1, 52] {
            let row = vec![0u8; len];
            assert!(
                record_from_row(&c, &row, 0).is_err(),
                "length {len} must be rejected, not read"
            );
            assert!(key_from_row(&row).is_err(), "length {len}");
            assert!(parse_row(&row).is_err(), "length {len}");
            assert!(is_deleted_row(&row).is_err(), "length {len}");
        }
        // An unknown flag bit (`world_fold.rs::fold_air_superiority_inputs`) — every one of them, alone and
        // beside the delete bit.
        for bit in 1..32u32 {
            for base in [0u32, ROW_FLAG_DELETED] {
                let row = raw_row(1, 2, 4, 8, base | (1 << bit), 0, 0, 0, 0, 0, 0);
                assert!(record_from_row(&c, &row, 0).is_err(), "flag bit {bit}");
                assert!(parse_row(&row).is_err(), "flag bit {bit}");
                assert!(is_deleted_row(&row).is_err(), "flag bit {bit}");
            }
        }
        // A non-zero reserved word (`:15470`), including single bits.
        for bit in 0..32u32 {
            let row = raw_row(1, 2, 4, 8, 0, 0, 0, 0, 0, 1 << bit, 5);
            assert!(record_from_row(&c, &row, 0).is_err(), "reserved bit {bit}");
            assert!(parse_row(&row).is_err(), "reserved bit {bit}");
        }
        // ...and the fold rejects NOTHING about `amount_raw` or any other value
        // word, so neither does the mapper.
        for raw in [
            EXACT_AMOUNT_RAW_BAND,
            -EXACT_AMOUNT_RAW_BAND,
            EXACT_AMOUNT_RAW_BAND + 1,
            -(EXACT_AMOUNT_RAW_BAND + 1),
            1 << 52,
            1 << 53,
            i64::MAX,
            i64::MIN,
        ] {
            assert!(
                record_from_row(&c, &live_row(1, 2, 4, 8, raw), 0).is_ok(),
                "amount_raw {raw} is accepted by the fold and must be mapped"
            );
        }
    }

    #[test]
    fn the_mapper_refuses_a_collection_it_cannot_fill() {
        // A caller handing over another domain's collection must not get a record
        // full of placeholders under this domain's field ids.
        let mut c = collection();
        c.fields.push(Field::new(999, Kind::Unsigned));
        assert!(record_from_row(&c, &live_row(1, 2, 4, 8, 5), 0).is_err());
    }

    #[test]
    fn split_payload_enforces_the_folds_structural_rules() {
        let rows = vec![
            live_row(37, 12, 512, 7249, 100_000),
            live_row(37, 12, 512, 836, 200_000),
        ];
        let p = payload(0, &rows);
        let (header, body) = split_payload(&p).expect("well-formed payload");
        assert_eq!(header.row_count, 2);
        assert_eq!(header.scale_raw, FIXED_SCALE_RAW);
        assert_eq!(body.len(), 2 * ROW_LEN);

        // Short of the header.
        for len in 0..HEADER_LEN {
            assert!(split_payload(&p[..len]).is_err(), "len {len}");
            assert!(peek_header(&p[..len]).is_err(), "len {len}");
        }
        // A rowless record is legal and common — 47 of the capture's 146 hours.
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
            h.observed_equipment_rows = u32::MAX;
            h.deleted_rows = 0;
            h
        });
        assert!(split_payload(&huge).is_err());
    }

    #[test]
    fn peek_header_enforces_all_five_count_invariants() {
        // `world_fold.rs::fold_air_superiority_inputs`, one at a time, each just over its bound —
        // the five are separate checks and a copy-paste error between them is exactly
        // the failure this covers. All five run BEFORE the status exits, so each is a
        // hard error on a failure-status record too.
        let rows = vec![live_row(37, 12, 512, 7249, 100_000)];
        type Perturb = fn(Header) -> Header;
        let over: [(&str, Perturb); 5] = [
            ("changed_rows > row_count", |mut h| {
                h.changed_rows = h.row_count + 1;
                h
            }),
            ("observed_loadout > observed_air_wing", |mut h| {
                h.observed_loadout_rows = h.observed_air_wing_rows + 1;
                h
            }),
            ("invalid_variant > observed_equipment", |mut h| {
                h.invalid_variant_rows = h.observed_equipment_rows + 1;
                h
            }),
            ("deleted_rows > row_count", |mut h| {
                h.deleted_rows = h.row_count + 1;
                h
            }),
            ("row_count > observed_equipment + deleted", |mut h| {
                h.observed_equipment_rows = 0;
                h.deleted_rows = 0;
                h
            }),
        ];
        for (label, perturb) in over {
            for status in [0u32, 1 /* STATE_UNAVAILABLE */] {
                let p = payload_with(status, &rows, perturb);
                assert!(
                    peek_header(&p).is_err(),
                    "{label} must be refused (status {status:#x})"
                );
            }
        }
        // The boundary values themselves are legal.
        let at_bound = payload_with(0, &rows, |mut h| {
            h.changed_rows = h.row_count;
            h.observed_loadout_rows = h.observed_air_wing_rows;
            h.invalid_variant_rows = h.observed_equipment_rows;
            h.deleted_rows = h.row_count;
            h
        });
        assert!(peek_header(&at_bound).is_ok());
        // `row_count == observed_equipment_rows + deleted_rows` exactly.
        let exact = payload_with(0, &rows, |mut h| {
            h.observed_equipment_rows = 0;
            h.deleted_rows = 1;
            h
        });
        assert!(peek_header(&exact).is_ok());
    }

    #[test]
    fn the_header_has_no_reserved_word_to_check() {
        // Unlike d31, d38's header carries no reserved word: `+32` and `+36` are
        // `observed_variant_rows` and `invalid_air_wing_rows`, real counters the
        // collector writes (`attach_log.rs:71538`-`:71539`) and the fold does not
        // read. A port that copied d31's zero check would refuse every real record.
        let rows = vec![live_row(37, 12, 512, 7249, 100_000)];
        let p = payload_with(0, &rows, |mut h| {
            h.observed_variant_rows = 204;
            h.invalid_air_wing_rows = 3;
            h
        });
        let header = peek_header(&p).expect("non-zero witness counters are legal");
        assert_eq!(header.observed_variant_rows, 204);
        assert_eq!(header.invalid_air_wing_rows, 3);
    }

    #[test]
    fn the_accumulator_replaces_once_then_patches_sparsely() {
        // The property that separates d38 from a full-replace domain
        // (`world_fold.rs:15455`, `replace_or_sparse_after_first` at `world_fold.rs::replace_or_sparse_after_first`). Only
        // the FIRST record clears; a later record — delta-flagged or not — is a
        // sparse patch, and reading it as a snapshot would drop every bucket it
        // omits.
        let c = collection();
        let mut acc = Accumulator::new();
        assert!(!acc.baseline_seen());

        let first = payload(
            0,
            &[
                live_row(37, 12, 512, 7249, 100_000),
                live_row(9, 3, 1, 836, 200_000),
            ],
        );
        assert_eq!(
            acc.fold_payload(&c, &first, false, 0).expect("folds"),
            FoldOutcome::FullReplace
        );
        assert!(acc.baseline_seen());
        assert_eq!(acc.rows.len(), 2);

        // A NON-delta record after the first is still sparse.
        let second = payload(0, &[live_row(9, 3, 1, 836, 300_000)]);
        assert_eq!(
            acc.fold_payload(&c, &second, false, 0).expect("folds"),
            FoldOutcome::Sparse
        );
        assert_eq!(acc.rows.len(), 2, "a later record does not clear");
        assert_eq!(
            acc.rows[&key(&[9, 3, 1, 836])].values[index_of(&c, field::AMOUNT)],
            Value::Continuous(3.0),
            "the row it carried was updated"
        );

        // An empty record is a no-op — 47 of the capture's 146 hours.
        let before = acc.rows.clone();
        assert_eq!(
            acc.fold_payload(&c, &payload(0, &[]), false, 0).expect("folds"),
            FoldOutcome::Sparse
        );
        assert_eq!(acc.rows, before);

        // A delete row removes its key and only its key, and a tombstone for an
        // absent key is a no-op (`:15481`).
        let third = payload(
            0,
            &[delete_row(37, 12, 512, 7249), delete_row(1, 1, 1, 1)],
        );
        assert_eq!(
            acc.fold_payload(&c, &third, false, 0).expect("folds"),
            FoldOutcome::Sparse
        );
        assert_eq!(acc.rows.len(), 1);
        assert!(acc.rows.contains_key(&key(&[9, 3, 1, 836])));
    }

    #[test]
    fn a_delta_before_any_baseline_folds_nothing() {
        // `reject_delta_before_baseline` (`world_fold.rs::reject_delta_before_baseline`, called at `:15440`)
        // marks the hour unknown and retains the accumulator. Note the shipping
        // collector never sets DELTA_DATA for d38 (`attach_log.rs:29175`-`:29190`
        // passes the frame's base status through), so this path is unreachable from
        // today's producer — but the fold accepts it and so must this.
        let c = collection();
        let mut acc = Accumulator::new();
        let p = payload(STATUS_DELTA_DATA, &[live_row(37, 12, 512, 7249, 100_000)]);
        assert_eq!(
            acc.fold_payload(&c, &p, true, 0).expect("folds"),
            FoldOutcome::DeltaBeforeBaseline
        );
        assert!(acc.rows.is_empty());
        assert!(!acc.baseline_seen());
        // And it is decided before the exact-length check (`:15447`), so a
        // length-inconsistent payload is not an error on this path.
        let mut short = p.clone();
        short.pop();
        let mut acc = Accumulator::new();
        assert_eq!(
            acc.fold_payload(&c, &short, true, 0).expect("folds"),
            FoldOutcome::DeltaBeforeBaseline
        );
        // A delta AFTER a baseline is an ordinary sparse patch.
        let mut acc = Accumulator::new();
        acc.fold_payload(&c, &payload(0, &[live_row(9, 3, 1, 836, 100_000)]), false, 0)
            .expect("baseline");
        assert_eq!(
            acc.fold_payload(&c, &p, true, 0).expect("folds"),
            FoldOutcome::Sparse
        );
        assert_eq!(acc.rows.len(), 2);
    }

    #[test]
    fn a_rejected_payload_leaves_the_accumulator_untouched() {
        let c = collection();
        let mut acc = Accumulator::new();
        acc.fold_payload(
            &c,
            &payload(0, &[live_row(37, 12, 512, 7249, 100_000)]),
            false,
            0,
        )
        .expect("baseline");
        let before = acc.rows.clone();
        // A bad row late in the record: the good row before it must not be applied.
        let bad = payload(
            0,
            &[
                live_row(9, 3, 1, 836, 100_000),
                raw_row(1, 1, 1, 1, 1 << 7, 0, 0, 0, 0, 0, 0),
            ],
        );
        assert!(acc.fold_payload(&c, &bad, false, 0).is_err());
        assert_eq!(acc.rows, before, "an Err leaves the accumulator as it was");
    }

    #[test]
    fn fold_payload_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the boundary reads a stream from disk and must
        // return a diagnosis, never unwind.
        let c = collection();
        let mut state = 0x2545_F491_4F6C_DD1Du64;
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
            let _ = peek_header(&bytes);
            let _ = split_payload(&bytes);
            let _ = parse_row(&bytes);
            let _ = key_from_row(&bytes);
            let _ = record_from_row(&c, &bytes, 0);
            let mut acc = Accumulator::new();
            let _ = acc.fold_payload(&c, &bytes, state & 1 == 0, state);
        }
        // And corruption of a genuinely well-formed payload, byte by byte.
        let good = payload(0, &[live_row(37, 12, 512, 7249, 100_000)]);
        for i in 0..good.len() {
            let mut p = good.clone();
            p[i] ^= 0xFF;
            let mut acc = Accumulator::new();
            let _ = acc.fold_payload(&c, &p, false, 0);
        }
    }
}
